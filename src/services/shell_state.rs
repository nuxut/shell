//! Kabuğun kendi sürüm durumu ve "güncelleme bitti" kararı (2026-10-13;
//! `docs/design/mascot-placement.md` "Güncelleme bitti").
//!
//! - Dosya: `$XDG_STATE_HOME/nuxut/shell-state.toml` (yoksa `~/.local/state/nuxut/…`). Tek
//!   yazan shell'dir; install.sh bu dosyaya dokunmaz ve sinyal göndermez (test dönemi taze
//!   kurulum listesinde de YOKTUR — orada olsaydı her güncelleme ilk kurulum sayılırdı).
//! - İçerik (donuk v1):
//!   ```toml
//!   last_version = "0.2.0"
//!   ```
//!   Bilinmeyen anahtarlar yok sayılır (ileri uyum). Değer SemVer 2.0 (`semver` crate).
//! - Okuma [`MAX_STATE_BYTES`] ile sınırlı; bozuk dosya panik değil, uyarı + doğru değerle
//!   üzerine yazım ([`StateRead::Corrupt`]).
//! - Yazım `nuxut-apply` ile (`services::apply::write_own`: atomik, ortam kapısı); dizin yoksa
//!   `0700`. Snapshot yok (Nuxut'un kendi durum dosyası).
//!
//! Karar ([`decide`]): dosya yok → ilk kurulum, gösterilmez (Hoş geldin var), yalnızca yazılır;
//! kayıtlı sürümden SemVer önceliğiyle büyük → bir kez gösterilir; aynı → hiçbir şey; küçük →
//! gösterilmez, yazılır; bozuk → gösterilmez, onarılır. Gösterim yazım başarısına bağlıdır
//! ([`check`]): yazılamayan örnek (kilitsiz, kullanıcının evine yazamayan test örneği) her
//! açılışta yeniden göstermesin diye hiç göstermez.
//!
//! UI'dan bağımsızdır; IO yalnızca küçük bir dosya (açılışta bir kez, ana thread'de).

use std::cmp::Ordering;
use std::io::Read;
use std::path::{Path, PathBuf};

use nuxut_apply::{Env, TargetKind};
use semver::Version;
use serde::Deserialize;

use crate::services::apply;

/// shell-state.toml okuma üst sınırı (geçerli dosya ~80 bayt).
pub const MAX_STATE_BYTES: u64 = 4096;
/// Sürüm metninin en uzun hâli (bayt); daha uzunu bozuk sayılır.
pub const MAX_VERSION_BYTES: usize = 64;
/// Dosya adı (`<state>/nuxut/` altında).
pub const STATE_FILE: &str = "shell-state.toml";

/// Kabuğun sürüm kimliği: paket sürümü (`Cargo.toml`). Git/derleme kimliği yok; sıralanabilir
/// tek kimlik budur.
pub const SHELL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Diskteki kayıt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateRecord {
    pub last_version: Version,
}

/// shell-state.toml'un okunmuş hâli.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateRead {
    /// Dosya yok: ilk kurulum.
    Missing,
    Present(StateRecord),
    /// Okunamadı/ayrıştırılamadı (neden log içindir; dosya içeriği girmez).
    Corrupt(String),
}

/// `<state_home>/nuxut/shell-state.toml`.
pub fn state_path(env: &Env) -> Option<PathBuf> {
    Some(env.nuxut_state_dir()?.join(STATE_FILE))
}

/// Sürüm metni: boşluksuz, sınırlı, geçerli SemVer.
pub fn parse_version(text: &str) -> Result<Version, String> {
    if text.is_empty() || text.len() > MAX_VERSION_BYTES {
        return Err(format!("last_version 1..={MAX_VERSION_BYTES} bayt olmalı"));
    }
    Version::parse(text).map_err(|e| format!("last_version SemVer değil: {e}"))
}

/// Metni ayrıştırır (saf).
pub fn parse(text: &str) -> Result<StateRecord, String> {
    #[derive(Deserialize)]
    struct Raw {
        last_version: String,
    }
    let raw: Raw = toml::from_str(text).map_err(|e| format!("ayrıştırılamadı: {}", e.message()))?;
    Ok(StateRecord { last_version: parse_version(&raw.last_version)? })
}

/// Kaydı dosya metnine çevirir (saf). Değer `Version`'ın kendi biçimi: TOML dizgesinde kaçış
/// gerektiren karakter içermez (SemVer yalnızca `[0-9A-Za-z.+-]`).
pub fn render(record: &StateRecord) -> String {
    // i18n: tanı makine dosyasının başlık yorumu
    format!("# Nuxut kabuk durumu; shell yazar (elle düzenlemeyin).\nlast_version = \"{}\"\n", record.last_version)
}

/// Dosyayı sınırla okur.
pub fn read(path: &Path) -> StateRead {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return StateRead::Missing,
        Err(e) => return StateRead::Corrupt(format!("açılamadı ({:?})", e.kind())),
    };
    match file.metadata() {
        Ok(m) if !m.is_file() => return StateRead::Corrupt("normal dosya değil".into()),
        Ok(m) if m.len() > MAX_STATE_BYTES => return StateRead::Corrupt(format!("{MAX_STATE_BYTES} bayttan büyük")),
        Ok(_) => {}
        Err(e) => return StateRead::Corrupt(format!("okunamadı ({:?})", e.kind())),
    }
    let mut bytes = Vec::new();
    // Ölçümle okuma arasında dosya büyüyebilir: okuma da sınırlı.
    if let Err(e) = file.take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes) {
        return StateRead::Corrupt(format!("okunamadı ({:?})", e.kind()));
    }
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return StateRead::Corrupt(format!("{MAX_STATE_BYTES} bayttan büyük"));
    }
    let Ok(text) = String::from_utf8(bytes) else { return StateRead::Corrupt("UTF-8 değil".into()) };
    match parse(&text) {
        Ok(r) => StateRead::Present(r),
        Err(e) => StateRead::Corrupt(e),
    }
}

/// Açılıştaki sürüm karşılaştırmasının sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionChange {
    /// Durum dosyası yok: ilk kurulum (Hoş geldin var, bildirim yok).
    FirstRun,
    /// Kayıtlı sürümden büyük: "güncelleme bitti" bildirimi.
    Upgraded { from: Version },
    /// Aynı sürüm (SemVer önceliğiyle; derleme meta verisi `+…` yok sayılır).
    Same,
    /// Kayıtlı sürümden küçük (geri dönüş): bildirim yok.
    Downgraded { from: Version },
    /// Bozuk dosya: bildirim yok, onarılır.
    Repaired,
}

impl VersionChange {
    /// Bildirim gösterilmeli mi.
    pub fn notify(&self) -> bool {
        matches!(self, Self::Upgraded { .. })
    }
}

/// Karar (saf): ne oldu ve dosyaya `current` yazılmalı mı. Aynı sürümde metin farklıysa
/// (yalnızca derleme meta verisi) yine yazılır ki dosya çalışan ikiliyi yansıtsın.
pub fn decide(read: &StateRead, current: &Version) -> (VersionChange, bool) {
    match read {
        StateRead::Missing => (VersionChange::FirstRun, true),
        StateRead::Corrupt(_) => (VersionChange::Repaired, true),
        StateRead::Present(r) => match current.cmp_precedence(&r.last_version) {
            Ordering::Greater => (VersionChange::Upgraded { from: r.last_version.clone() }, true),
            Ordering::Less => (VersionChange::Downgraded { from: r.last_version.clone() }, true),
            Ordering::Equal => (VersionChange::Same, r.last_version != *current),
        },
    }
}

/// [`check`] sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    pub change: VersionChange,
    /// Dosya bu çağrıda yazıldı.
    pub written: bool,
}

impl UpdateCheck {
    /// Bildirim gösterilsin mi: yükseltme VE yeni sürüm diske yazıldı (yazım başarısızsa
    /// [`check`] zaten `Err` döner). Böylece bildirim sürüm başına en çok bir kez çıkar.
    pub fn should_notify(&self) -> bool {
        self.written && self.change.notify()
    }
}

/// Açılışta bir kez: oku, karşılaştır, gerekiyorsa `current`'ı atomik yaz. `Err`: yazım izni
/// yok ya da yazım başarısız (ADR-0001 §10.3 adı ya da hata metni); bu durumda bildirim yok.
/// Kapı okumadan önce: reddedilecek örnek dosyayı da açmaz.
pub fn check(env: &Env, current: &str) -> Result<UpdateCheck, String> {
    let current = parse_version(current)?;
    env.guard(TargetKind::File).map_err(|s| apply::error_text(&s.into()))?;
    let path = state_path(env).ok_or("skipped(no_session)")?;
    let read = read(&path);
    if let StateRead::Corrupt(reason) = &read {
        tracing::warn!(reason = %reason, "shell-state.toml bozuk; doğru değerle yeniden yazılıyor");
    }
    let (change, write) = decide(&read, &current);
    if write {
        if let Some(dir) = path.parent() {
            apply::ensure_private_dir(env, dir).map_err(|e| apply::error_text(&e))?;
        }
        let text = render(&StateRecord { last_version: current });
        apply::write_own(env, &path, text.as_bytes()).map_err(|e| apply::error_text(&e))?;
    }
    Ok(UpdateCheck { change, written: write })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn present(s: &str) -> StateRead {
        StateRead::Present(StateRecord { last_version: v(s) })
    }

    #[test]
    fn shell_version_is_semver() {
        assert!(parse_version(SHELL_VERSION).is_ok(), "{SHELL_VERSION}");
    }

    #[test]
    fn parse_render_roundtrip_and_rejects() {
        let r = StateRecord { last_version: v("1.2.3-rc.1+abc") };
        assert_eq!(parse(&render(&r)), Ok(r));
        assert_eq!(parse("last_version = \"0.1.0\"\nfuture = 1\n").map(|r| r.last_version), Ok(v("0.1.0")));
        for bad in [
            "",
            "last_version = 1",
            "last_version = \"\"",
            "last_version = \"1.2\"",
            "last_version = \"v1.2.3\"",
            "last_version = \" 1.2.3\"",
            &format!("last_version = \"1.0.0-{}\"", "a".repeat(MAX_VERSION_BYTES)),
            "not toml [",
        ] {
            assert!(parse(bad).is_err(), "kabul edilmemeli: {bad:?}");
        }
    }

    #[test]
    fn decide_rules() {
        let cur = v("0.2.0");
        assert_eq!(decide(&StateRead::Missing, &cur), (VersionChange::FirstRun, true));
        assert_eq!(decide(&StateRead::Corrupt("x".into()), &cur), (VersionChange::Repaired, true));
        assert_eq!(decide(&present("0.1.9"), &cur), (VersionChange::Upgraded { from: v("0.1.9") }, true));
        assert_eq!(decide(&present("0.2.0"), &cur), (VersionChange::Same, false));
        assert_eq!(decide(&present("0.3.0"), &cur), (VersionChange::Downgraded { from: v("0.3.0") }, true));
        // Ön sürüm: 0.2.0-rc.1 < 0.2.0 (yükseltme); derleme meta verisi öncelikte yok sayılır.
        assert!(decide(&present("0.2.0-rc.1"), &cur).0.notify());
        assert_eq!(decide(&present("0.2.0+git.1"), &cur), (VersionChange::Same, true));
        assert!(!VersionChange::FirstRun.notify() && !VersionChange::Repaired.notify());
    }

    #[test]
    fn read_missing_present_corrupt_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nuxut").join(STATE_FILE);
        assert_eq!(read(&path), StateRead::Missing);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "last_version = \"0.1.0\"\n").unwrap();
        assert_eq!(read(&path), present("0.1.0"));
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(matches!(read(&path), StateRead::Corrupt(_)));
        let base = "last_version = \"0.1.0\"\n#";
        let fill = |n: usize| format!("{base}{}", "x".repeat(n - base.len()));
        std::fs::write(&path, fill(MAX_STATE_BYTES as usize)).unwrap();
        assert!(matches!(read(&path), StateRead::Present(_)));
        std::fs::write(&path, fill(MAX_STATE_BYTES as usize + 1)).unwrap();
        assert!(matches!(read(&path), StateRead::Corrupt(_)));
        let d = dir.path().join("asdir");
        std::fs::create_dir(&d).unwrap();
        assert!(matches!(read(&d), StateRead::Corrupt(_)));
    }

    /// Uçtan uca dosya akışı: ilk kurulum gösterilmez; artış bir kez gösterir (ikinci açılış
    /// göstermez); aynı/düşüş gösterilmez; bozuk dosya panik yok, onarılır; geçici dosya kalmaz.
    #[test]
    fn check_flow_on_disk() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::for_test(t.path());
        let path = state_path(&env).unwrap();
        let notify = |cur: &str| check(&env, cur).unwrap().should_notify();

        assert!(!notify("0.1.0"), "ilk kurulum: bildirim yok");
        assert_eq!(read(&path), present("0.1.0"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);

        assert!(!notify("0.1.0"), "aynı sürüm");
        assert!(notify("0.2.0"), "artış");
        assert!(!notify("0.2.0"), "ikinci açılış göstermez");
        assert_eq!(read(&path), present("0.2.0"));

        assert!(!notify("0.1.5"), "düşüş");
        assert_eq!(read(&path), present("0.1.5"), "düşüşte last_version güncellenir");

        std::fs::write(&path, "bozuk [").unwrap();
        let c = check(&env, "0.3.0").unwrap();
        assert_eq!((c.change.clone(), c.should_notify()), (VersionChange::Repaired, false));
        assert_eq!(read(&path), present("0.3.0"));
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from(STATE_FILE)]);

        assert!(check(&env, "bozuk").is_err(), "geçersiz kendi sürümü: hiçbir şey yazılmaz");
        assert_eq!(read(&path), present("0.3.0"));
    }

    /// Yazamayan örnek (kilitsiz) dosya açmaz, yazmaz, bildirim göstermez.
    #[test]
    fn check_without_write_permission_never_notifies() {
        let t = tempfile::tempdir().unwrap();
        let env = Env { disabled: true, ..Env::for_test(t.path()) };
        assert_eq!(check(&env, "9.0.0"), Err("skipped(disabled)".into()));
        assert!(!t.path().join("state").exists());
    }
}
