//! Geçerli kullanıcı: `getpwuid_r(getuid())`. `$USER` ortam değişkenine güvenilmez.

use std::ffi::{CStr, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    pub uid: u32,
    /// Giriş adı (PAM'e verilen).
    pub name: String,
    /// GECOS'un ilk alanı; boşsa `None`.
    pub real_name: Option<String>,
    pub home: PathBuf,
}

impl UserInfo {
    /// Gösterilecek ad: gerçek ad, yoksa giriş adı.
    pub fn display_name(&self) -> &str {
        self.real_name.as_deref().unwrap_or(&self.name)
    }
}

/// Süreç uid'inin passwd kaydı.
pub fn current_user() -> io::Result<UserInfo> {
    // SAFETY: getuid her zaman başarılıdır, yan etkisi yok.
    let uid = unsafe { libc::getuid() };
    lookup_uid(uid)
}

pub fn lookup_uid(uid: u32) -> io::Result<UserInfo> {
    const MAX_BUF: usize = 1 << 20;
    let mut size = 4096;
    loop {
        let mut buf = vec![0 as libc::c_char; size];
        let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: tüm işaretçiler geçerli; buf.len() bayt yazılabilir. Başarıda `result` == pwd
        // ve dizgi alanları buf içini gösterir.
        let rc = unsafe {
            libc::getpwuid_r(
                uid,
                pwd.as_mut_ptr(),
                buf.as_mut_ptr(),
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE && size < MAX_BUF {
            size *= 2;
            continue;
        }
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }
        if result.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "uid için passwd kaydı yok",
            ));
        }
        // SAFETY: rc == 0 ve result null değil → pwd başlatıldı.
        let pwd = unsafe { pwd.assume_init() };
        let field = |p: *const libc::c_char| -> &[u8] {
            if p.is_null() {
                return &[];
            }
            // SAFETY: getpwuid_r'nin doldurduğu, buf içindeki NUL ile biten dizgi; buf canlı.
            unsafe { CStr::from_ptr(p) }.to_bytes()
        };
        let name = std::str::from_utf8(field(pwd.pw_name))
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "kullanıcı adı UTF-8 değil"))?
            .to_owned();
        if name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "boş kullanıcı adı",
            ));
        }
        return Ok(UserInfo {
            uid,
            name,
            real_name: gecos_name(field(pwd.pw_gecos)),
            home: PathBuf::from(OsStr::from_bytes(field(pwd.pw_dir))),
        });
    }
}

/// GECOS'un ilk (virgülle ayrılmış) alanı, temizlenmiş.
fn gecos_name(gecos: &[u8]) -> Option<String> {
    let first = gecos.split(|b| *b == b',').next().unwrap_or_default();
    let s = crate::conv::sanitize(first);
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_matches_uid() {
        let u = current_user().unwrap();
        // SAFETY: test; getuid her zaman başarılı.
        assert_eq!(u.uid, unsafe { libc::getuid() });
        assert!(!u.name.is_empty());
        assert!(!u.display_name().is_empty());
    }

    #[test]
    fn root_exists() {
        assert_eq!(lookup_uid(0).unwrap().name, "root");
    }

    #[test]
    fn gecos_first_field() {
        assert_eq!(gecos_name(b"Hakan G,Oda 1,,"), Some("Hakan G".into()));
        assert_eq!(gecos_name(b""), None);
        assert_eq!(gecos_name(b" ,x"), None);
    }
}
