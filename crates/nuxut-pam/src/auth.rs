//! `pam_start` → `pam_authenticate` → (başarıda `pam_setcred(REFRESH)`) → `pam_end`.
//!
//! Bloklar (pam_unix hata gecikmesi ~2 sn, ağ modülleri): UI thread'inden çağrılmaz.
//! `pam_acct_mgmt` bilerek çağrılmaz (hesap politikası oturum açılışına aittir; bkz.
//! shell `docs/ARCHITECTURE.md` "Kilit ekranı").

use std::ffi::CString;
use std::path::Path;

use libc::c_int;

use crate::conv::{ConvState, PamMessage, conversation};
use crate::ffi::*;
use crate::secret::SecretBuf;

/// Doğrulama sonucu. **Yalnızca** [`Outcome::Success`] kilidi açar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// Yanlış şifre, izin yok, kullanıcı bilinmiyor.
    Denied,
    /// Modül deneme sınırına ulaştı (`PAM_MAXTRIES`).
    MaxTries,
    /// Kimlik bilgisi servisine ulaşılamadı (`PAM_AUTHINFO_UNAVAIL`).
    Unavailable,
    /// Desteklenmeyen istem (ikinci şifre istemi, ikili istem).
    UnsupportedPrompt,
    /// Diğer her PAM kodu (servis/sistem/bellek hatası, `pam_start` hatası…).
    Error(i32),
}

impl Outcome {
    pub fn is_success(self) -> bool {
        self == Self::Success
    }

    /// PAM dönüş kodunu sınıflandırır. `PAM_SUCCESS` dışındaki her kod başarısızdır.
    pub fn from_code(rc: c_int, unsupported_prompt: bool) -> Self {
        match rc {
            PAM_SUCCESS => Self::Success,
            PAM_AUTH_ERR | PAM_PERM_DENIED | PAM_CRED_INSUFFICIENT | PAM_USER_UNKNOWN => {
                Self::Denied
            }
            PAM_MAXTRIES => Self::MaxTries,
            PAM_AUTHINFO_UNAVAIL => Self::Unavailable,
            PAM_CONV_ERR if unsupported_prompt => Self::UnsupportedPrompt,
            other => Self::Error(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthResult {
    pub outcome: Outcome,
    /// Modüllerin kullanıcıya mesajları (faillock vb.), sınırlı ve temiz.
    pub messages: Vec<PamMessage>,
}

impl AuthResult {
    fn error(rc: c_int) -> Self {
        Self {
            outcome: Outcome::Error(rc),
            messages: Vec::new(),
        }
    }
}

/// Geçersiz PAM servis adı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidService;

/// PAM servisi (`/etc/pam.d/<ad>`).
#[derive(Debug, Clone)]
pub struct Service {
    name: CString,
    #[cfg(feature = "testing")]
    confdir: Option<CString>,
}

impl Service {
    /// Kilidin kendi servisi.
    pub const LOCK: &'static str = "nuxut-lock";
    /// `/etc/pam.d/nuxut-lock` yoksa (içeriği zaten `auth include login` olurdu).
    pub const FALLBACK: &'static str = "login";

    /// `[a-z0-9_.-]{1,64}`, nokta ile başlamaz (yol bileşeni olamaz).
    pub fn named(name: &str) -> Result<Self, InvalidService> {
        let ok = (1..=64).contains(&name.len())
            && !name.starts_with('.')
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
        if !ok {
            return Err(InvalidService);
        }
        Ok(Self {
            name: CString::new(name).map_err(|_| InvalidService)?,
            #[cfg(feature = "testing")]
            confdir: None,
        })
    }

    /// Sistem servisi: `/etc/pam.d/nuxut-lock` dosyası varsa o, yoksa `login`.
    ///
    /// Dosya yokken `nuxut-lock` ile başlamak PAM'ı `other`'a (Arch'ta `pam_deny`) düşürür ve
    /// kullanıcı kalıcı olarak kilitli kalır; bu yüzden kilitlemeden **önce** çağrılır.
    pub fn system() -> Self {
        Self::select_in(Path::new("/etc/pam.d"))
    }

    /// [`Service::system`]'in dizin parametreli hâli (test).
    pub fn select_in(pam_d: &Path) -> Self {
        let name = if pam_d.join(Self::LOCK).is_file() {
            Self::LOCK
        } else {
            Self::FALLBACK
        };
        Self::named(name).expect("sabit servis adları geçerli")
    }

    /// Yalnızca testler: servis dosyaları `dir` içinden okunur (`pam_start_confdir`).
    #[cfg(feature = "testing")]
    pub fn with_confdir(name: &str, dir: &Path) -> Result<Self, InvalidService> {
        use std::os::unix::ffi::OsStrExt;
        let mut s = Self::named(name)?;
        s.confdir = Some(CString::new(dir.as_os_str().as_bytes()).map_err(|_| InvalidService)?);
        Ok(s)
    }

    pub fn name(&self) -> &str {
        self.name.to_str().unwrap_or_default()
    }

    pub fn is_fallback(&self) -> bool {
        self.name() == Self::FALLBACK
    }
}

/// `user`'ı `secret` ile doğrular. Bloklar.
pub fn authenticate(service: &Service, user: &str, secret: &SecretBuf) -> AuthResult {
    let Ok(user) = CString::new(user) else {
        return AuthResult::error(PAM_USER_UNKNOWN);
    };
    let mut state = ConvState::new(secret.as_bytes());
    let conv = pam_conv {
        conv: Some(conversation),
        appdata_ptr: (&raw mut state).cast(),
    };
    let mut handle: *mut pam_handle_t = std::ptr::null_mut();

    let rc = start(service, &user, &conv, &mut handle);
    if rc != PAM_SUCCESS || handle.is_null() {
        if !handle.is_null() {
            // SAFETY: pam_start'ın verdiği tutamaç, bir kez sonlandırılır.
            unsafe { pam_end(handle, rc) };
        }
        return AuthResult::error(if rc == PAM_SUCCESS {
            PAM_SYSTEM_ERR
        } else {
            rc
        });
    }

    // SAFETY: geçerli tutamaç; conv ve state pam_end'e kadar bu yığın çerçevesinde canlı.
    let rc = unsafe { pam_authenticate(handle, 0) };
    if rc == PAM_SUCCESS {
        // Kerberos vb. bilet yenileme; sonucu doğrulamayı etkilemez.
        // SAFETY: geçerli tutamaç.
        unsafe { pam_setcred(handle, PAM_REFRESH_CRED) };
    }
    // SAFETY: geçerli tutamaç, bir kez sonlandırılır; sonrasında conversation çağrılmaz.
    unsafe { pam_end(handle, rc) };

    let ConvState {
        messages,
        unsupported,
        ..
    } = state;
    AuthResult {
        outcome: Outcome::from_code(rc, unsupported),
        messages,
    }
}

fn start(
    service: &Service,
    user: &CString,
    conv: &pam_conv,
    handle: &mut *mut pam_handle_t,
) -> c_int {
    #[cfg(feature = "testing")]
    if let Some(dir) = &service.confdir {
        // SAFETY: tüm dizgiler NUL ile biten canlı CString; conv ve handle geçerli işaretçiler.
        return unsafe {
            pam_start_confdir(
                service.name.as_ptr(),
                user.as_ptr(),
                conv,
                dir.as_ptr(),
                handle,
            )
        };
    }
    // SAFETY: tüm dizgiler NUL ile biten canlı CString; conv ve handle geçerli işaretçiler.
    unsafe { pam_start(service.name.as_ptr(), user.as_ptr(), conv, handle) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pam_success_is_success() {
        for rc in 0..64 {
            for unsupported in [false, true] {
                assert_eq!(
                    Outcome::from_code(rc, unsupported).is_success(),
                    rc == PAM_SUCCESS,
                    "rc={rc}"
                );
            }
        }
        for rc in [-1, i32::MIN, i32::MAX] {
            assert!(!Outcome::from_code(rc, false).is_success());
        }
        assert_eq!(Outcome::from_code(PAM_AUTH_ERR, false), Outcome::Denied);
        assert_eq!(Outcome::from_code(PAM_MAXTRIES, false), Outcome::MaxTries);
        assert_eq!(
            Outcome::from_code(PAM_AUTHINFO_UNAVAIL, false),
            Outcome::Unavailable
        );
        assert_eq!(
            Outcome::from_code(PAM_CONV_ERR, true),
            Outcome::UnsupportedPrompt
        );
        assert_eq!(
            Outcome::from_code(PAM_CONV_ERR, false),
            Outcome::Error(PAM_CONV_ERR)
        );
    }

    #[test]
    fn service_names_are_validated() {
        assert!(Service::named("nuxut-lock").is_ok());
        assert!(Service::named("system-local-login").is_ok());
        for bad in [
            "",
            ".",
            "..",
            "../etc",
            "a/b",
            "Login",
            "a b",
            "x\0y",
            &"a".repeat(65),
        ] {
            assert_eq!(Service::named(bad).err(), Some(InvalidService), "{bad:?}");
        }
    }

    #[test]
    fn system_service_falls_back_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let s = Service::select_in(dir.path());
        assert_eq!(s.name(), "login");
        assert!(s.is_fallback());
        std::fs::write(dir.path().join("nuxut-lock"), "auth include login\n").unwrap();
        let s = Service::select_in(dir.path());
        assert_eq!(s.name(), "nuxut-lock");
        assert!(!s.is_fallback());
        // Dizin olan "dosya" sayılmaz.
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir2.path().join("nuxut-lock")).unwrap();
        assert!(Service::select_in(dir2.path()).is_fallback());
    }

    #[test]
    fn nul_in_user_fails_without_pam() {
        let s = Service::named("login").unwrap();
        let r = authenticate(&s, "a\0b", &SecretBuf::new());
        assert!(!r.outcome.is_success());
    }
}
