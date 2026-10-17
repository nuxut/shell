//! Kurulu ikilinin dizini ve oturum PATH'i.
//!
//! Kabuk kendi ikililerini (bind'ler, oturum kancası, kilit, Ayarlar) `current_exe()` mutlak yoluyla
//! çağırır; PATH'e güvenmez. Terminalden `nuxut-shell msg …` ve adla arayan betikler için kurulum
//! dizinini oturumun PATH'ine Nuxut'un Hyprland katmanı koyar (`nuxut.conf`/`nuxut.lua`,
//! [`crate::services::hypr_config::generate::session_path_dir`]; kullanıcı isteği 2026-10-16: kabuk
//! başlangıç dosyalarına yazılmaz, elle adım yok). Dizin bu sürecin PATH'inde yoksa bir kez
//! bilgi log'u yazılır.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Çalışan ikilinin dizini (sembolik bağlar çözülmüş).
pub fn exe_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    Some(dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()))
}

/// `dir` PATH değerinde var mı? Girdiler sembolik bağ ya da `..` içerebilir: hem yalın hem
/// çözülmüş biçim karşılaştırılır. Boş girdi (geçerli dizin) sayılmaz.
pub fn dir_in_path(dir: &Path, path: Option<&OsStr>) -> bool {
    let Some(path) = path else { return false };
    let canon = dir.canonicalize().ok();
    std::env::split_paths(path).filter(|p| !p.as_os_str().is_empty()).any(|p| {
        p == dir || canon.as_deref().is_some_and(|c| p.canonicalize().is_ok_and(|q| q == c))
    })
}

/// Kurulum dizini PATH'te değilse bir kez bilgi log'u yazar. Çözüm uygulanmıştır (Hyprland katmanı);
/// eksikse nedeni ilk kurulumdan sonraki ilk oturum ya da katmanın yüklenmemesidir.
pub fn warn_if_not_in_path() {
    let Some(dir) = exe_dir() else { return };
    if !dir_in_path(&dir, std::env::var_os("PATH").as_deref()) {
        tracing::info!(
            dir = %dir.display(),
            "kabuğun dizini bu oturumun PATH'inde değil; Nuxut'un Hyprland katmanı onu bir sonraki \
             girişte ekler. Sonra da yoksa katman yüklenmiyor: nuxut-shell msg hypr status (source: present)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn join(parts: &[&Path]) -> OsString {
        std::env::join_paths(parts).unwrap()
    }

    #[test]
    fn plain_match_and_miss() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let path = join(&[Path::new("/usr/bin"), a.path()]);
        assert!(dir_in_path(a.path(), Some(&path)));
        assert!(!dir_in_path(b.path(), Some(&path)));
        assert!(!dir_in_path(a.path(), None), "PATH yoksa yok sayılır");
        assert!(!dir_in_path(a.path(), Some(OsStr::new(""))));
    }

    #[test]
    fn symlinked_and_dotted_entries_match() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // PATH'te bağ, ikili gerçek dizinde (current_exe çözülmüş yol verir).
        assert!(dir_in_path(&real, Some(&join(&[&link]))));
        // `..` içeren girdi.
        let dotted = root.path().join("real/../real");
        assert!(dir_in_path(&real, Some(&join(&[&dotted]))));
        // Boş girdi geçerli dizin demektir; sayılmaz.
        assert!(!dir_in_path(&real, Some(OsStr::new(":"))));
    }

    #[test]
    fn exe_dir_is_absolute() {
        let dir = exe_dir().expect("test ikilisinin dizini");
        assert!(dir.is_absolute());
    }
}
