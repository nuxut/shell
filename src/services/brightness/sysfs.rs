//! `/sys` okumaları: arka ışık aygıtları ve DRM bağlayıcılarının DDC i2c hatları.
//! Tüm dosya okumaları boyut sınırlıdır; kök dizin testler için değiştirilebilir.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Okunan sysfs dosyalarının sınırı.
const MAX_FILE: u64 = 4096;
/// En fazla bu kadar aygıt/bağlayıcı taranır.
const MAX_ENTRIES: usize = 64;

pub(crate) fn read_small(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut s = String::new();
    f.take(MAX_FILE).read_to_string(&mut s).ok()?;
    Some(s.trim().to_owned())
}

pub(crate) fn read_u32(path: &Path) -> Option<u32> {
    read_small(path)?.parse().ok()
}

/// sysfs/aygıt adı güvenli mi (logind'e ve yollara giden ad)?
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
        && name != "."
        && name != ".."
}

/// Arka ışık türü (öncelik: firmware > platform > raw; systemd ve GNOME ile aynı).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum BacklightType {
    Firmware,
    Platform,
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Backlight {
    pub name: String,
    pub dir: PathBuf,
    pub kind: BacklightType,
    pub max: u32,
}

/// `<root>/class/backlight` altındaki aygıtlar, tercih sırasına göre.
pub(crate) fn backlights(root: &Path) -> Vec<Backlight> {
    let Ok(entries) = std::fs::read_dir(root.join("class/backlight")) else {
        return Vec::new();
    };
    let mut out: Vec<Backlight> = entries
        .flatten()
        .take(MAX_ENTRIES)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok().filter(|n| valid_name(n))?;
            let dir = e.path();
            let kind = match read_small(&dir.join("type")).as_deref() {
                Some("firmware") => BacklightType::Firmware,
                Some("platform") => BacklightType::Platform,
                _ => BacklightType::Raw,
            };
            let max = read_u32(&dir.join("max_brightness")).filter(|&m| m > 0)?;
            Some(Backlight { name, dir, kind, max })
        })
        .collect();
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Güncel değer: `actual_brightness` (donanımın bildirdiği) yoksa `brightness`.
pub(crate) fn current(b: &Backlight) -> Option<u32> {
    read_u32(&b.dir.join("actual_brightness")).or_else(|| read_u32(&b.dir.join("brightness"))).map(|v| v.min(b.max))
}

/// DDC'si olabilecek bağlı harici bağlayıcı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Connector {
    /// Hyprland/Wayland çıktı adı (`DP-1`, `HDMI-A-1`).
    pub name: String,
    /// i2c aygıt numarası (`/dev/i2c-N`).
    pub i2c: u32,
}

/// `<root>/class/drm/card*-<ad>` bağlayıcılarından bağlı olan harici olanlar ve DDC hatları.
/// HDMI/DVI/VGA'da `ddc` sembolik bağı, DP'de bağlayıcı altındaki `i2c-N` (DP AUX) kullanılır.
/// Dahili paneller (eDP, LVDS, DSI) atlanır: DDC'leri yoktur, arka ışıkları vardır.
pub(crate) fn connectors(root: &Path) -> Vec<Connector> {
    let Ok(entries) = std::fs::read_dir(root.join("class/drm")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten().take(MAX_ENTRIES * 2) {
        let Ok(dir_name) = e.file_name().into_string() else {
            continue;
        };
        let Some((card, name)) = dir_name.split_once('-') else {
            continue;
        };
        if !card.starts_with("card") || !valid_name(name) {
            continue;
        }
        if ["eDP", "LVDS", "DSI"].iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let dir = e.path();
        if read_small(&dir.join("status")).as_deref() != Some("connected") {
            continue;
        }
        if let Some(i2c) = ddc_link(&dir).or_else(|| aux_i2c(&dir)) {
            out.push(Connector { name: name.to_owned(), i2c });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

fn i2c_number(name: &str) -> Option<u32> {
    name.strip_prefix("i2c-")?.parse().ok()
}

fn ddc_link(dir: &Path) -> Option<u32> {
    let target = std::fs::read_link(dir.join("ddc")).ok()?;
    i2c_number(target.file_name()?.to_str()?)
}

fn aux_i2c(dir: &Path) -> Option<u32> {
    std::fs::read_dir(dir).ok()?.flatten().take(MAX_ENTRIES).find_map(|e| i2c_number(e.file_name().to_str()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, s: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, s).unwrap();
    }

    #[test]
    fn backlights_ordered_and_validated() {
        let root = tempfile::tempdir().unwrap();
        let bl = root.path().join("class/backlight");
        write(&bl.join("intel_backlight/type"), "raw\n");
        write(&bl.join("intel_backlight/max_brightness"), "96000\n");
        write(&bl.join("intel_backlight/actual_brightness"), "48000\n");
        write(&bl.join("acpi_video0/type"), "firmware\n");
        write(&bl.join("acpi_video0/max_brightness"), "15\n");
        write(&bl.join("acpi_video0/brightness"), "99\n");
        write(&bl.join("broken/type"), "raw\n");
        write(&bl.join("broken/max_brightness"), "0\n");
        let list = backlights(root.path());
        assert_eq!(list.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["acpi_video0", "intel_backlight"]);
        assert_eq!(current(&list[0]), Some(15), "max ile sınırlanır");
        assert_eq!(current(&list[1]), Some(48000));
    }

    #[test]
    fn connectors_find_ddc_and_aux() {
        let root = tempfile::tempdir().unwrap();
        let drm = root.path().join("class/drm");
        write(&drm.join("card1-HDMI-A-1/status"), "connected\n");
        std::os::unix::fs::symlink("../../i2c-4", drm.join("card1-HDMI-A-1/ddc")).unwrap();
        write(&drm.join("card1-DP-2/status"), "connected\n");
        std::fs::create_dir_all(drm.join("card1-DP-2/i2c-7")).unwrap();
        write(&drm.join("card1-DP-3/status"), "disconnected\n");
        write(&drm.join("card1-eDP-1/status"), "connected\n");
        std::fs::create_dir_all(drm.join("card1-eDP-1/i2c-9")).unwrap();
        let list = connectors(root.path());
        assert_eq!(list, [Connector { name: "DP-2".into(), i2c: 7 }, Connector { name: "HDMI-A-1".into(), i2c: 4 }]);
    }

    #[test]
    fn names() {
        assert!(valid_name("intel_backlight"));
        assert!(!valid_name("../x"));
        assert!(!valid_name("a b"));
        assert!(!valid_name(".."));
    }
}
