//! Nuxut oturumunun portal ortamı (docs/design/portal.md): Hyprland'in ortamına (üretilen
//! `nuxut.lua`/`nuxut.conf` `env`) giren `XDG_CURRENT_DESKTOP=Nuxut:<önceki>` (varsayılan
//! `Nuxut:Hyprland`). xdg-desktop-portal önce `nuxut-portals.conf`'u arar; Hyprland'e özgü
//! denetimler için `Hyprland` listede kalır.
//!
//! Neden environment.d değil: Hyprland açılışta kendi ortamındaki `XDG_CURRENT_DESKTOP`'u systemd
//! ve D-Bus etkinleştirme ortamına aktarır (`import-environment`,
//! `dbus-update-activation-environment`); environment.d değeri bununla ezilir. Hyprland'in ortamı
//! ise yalnızca bu oturumdur — başka masaüstü oturumları etkilenmez.
//!
//! `XDG_DATA_DIRS` gerekmez: xdg-desktop-portal 1.22 `.portal` dosyalarını `$XDG_DATA_HOME`
//! altında da arar (uçtan uca doğrulandı). Portal kurulu değilse (kayıt dosyası yoksa) hiçbir şey
//! eklenmez. Değer shell'in kendi ortamından türetilir; Hyprland ortamı bir kez kurulunca shell
//! aynı değeri görür, üretim kararlıdır.

use std::path::{Path, PathBuf};

/// Masaüstü adı (`nuxut-portals.conf` → küçük harf `nuxut`).
pub const DESKTOP_NAME: &str = "Nuxut";
/// `XDG_CURRENT_DESKTOP` yoksa Hyprland'in kendi koyduğu değer.
const HYPRLAND_DESKTOP: &str = "Hyprland";
/// `$XDG_DATA_HOME` altındaki kayıt dosyası (install.sh ile aynı).
pub const PORTAL_FILE: &str = "xdg-desktop-portal/portals/nuxut.portal";

/// `list`'in (`:` ayrılmış) başına `item`; zaten varsa olduğu gibi. Boş öğeler atılır.
fn prepend(item: &str, list: Option<&str>, default: &str) -> String {
    let list = list.filter(|l| l.split(':').any(|p| !p.is_empty())).unwrap_or(default);
    let parts: Vec<&str> = list.split(':').filter(|p| !p.is_empty()).collect();
    if parts.contains(&item) {
        return parts.join(":");
    }
    std::iter::once(item).chain(parts).collect::<Vec<_>>().join(":")
}

/// Saf: portal kurulu mu ve shell'in gördüğü `XDG_CURRENT_DESKTOP` → eklenecek çiftler.
pub fn session_env(installed: bool, current_desktop: Option<&str>) -> Vec<(String, String)> {
    if !installed {
        return Vec::new();
    }
    vec![("XDG_CURRENT_DESKTOP".to_owned(), prepend(DESKTOP_NAME, current_desktop, HYPRLAND_DESKTOP))]
}

/// `$XDG_DATA_HOME` (mutlaksa) ya da `$HOME/.local/share`.
fn data_home() -> Option<PathBuf> {
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    var("XDG_DATA_HOME").filter(|p| p.is_absolute()).or_else(|| var("HOME").map(|h| h.join(".local/share")))
}

/// Kayıt dosyası `data_home` altında var mı?
pub fn installed(data_home: &Path) -> bool {
    data_home.join(PORTAL_FILE).is_file()
}

/// Süreç ortamından.
pub fn from_process() -> Vec<(String, String)> {
    let installed = data_home().is_some_and(|d| installed(&d));
    session_env(installed, std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desktop(installed: bool, current: Option<&str>) -> Option<String> {
        let e = session_env(installed, current);
        assert!(e.iter().all(|(k, _)| k == "XDG_CURRENT_DESKTOP"));
        e.into_iter().next().map(|(_, v)| v)
    }

    #[test]
    fn nothing_when_not_installed() {
        assert_eq!(desktop(false, Some("Hyprland")), None);
    }

    #[test]
    fn nuxut_is_prepended() {
        assert_eq!(desktop(true, Some("Hyprland")).unwrap(), "Nuxut:Hyprland");
        // Kullanıcının kendi listesi korunur.
        assert_eq!(desktop(true, Some("Hyprland:wlroots")).unwrap(), "Nuxut:Hyprland:wlroots");
    }

    #[test]
    fn default_when_unset_or_empty() {
        for current in [None, Some(""), Some("::")] {
            assert_eq!(desktop(true, current).unwrap(), "Nuxut:Hyprland");
        }
    }

    /// Hyprland ortamı bir kez kurulunca shell aynı değeri görür: üretim kararlı.
    #[test]
    fn idempotent() {
        let first = desktop(true, Some("Hyprland")).unwrap();
        assert_eq!(desktop(true, Some(&first)).unwrap(), first);
    }

    #[test]
    fn installed_checks_registration_file() {
        let t = tempfile::tempdir().unwrap();
        assert!(!installed(t.path()));
        let f = t.path().join(PORTAL_FILE);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "[portal]\n").unwrap();
        assert!(installed(t.path()));
    }
}
