//! Palete göre klasör ikonları (kullanıcı isteği 2026-10-16; docs/design/look-gtk.md
//! "Klasör ikonları").
//!
//! Papirus her klasör ikonunu renk çeşitleriyle gelir (`folder-blue-documents.svg`, …) ve genel
//! adları (`folder-documents.svg`) bağla maviye verir; `papirus-folders` aracı bu bağları root
//! olarak değiştirir. Nuxut sistem dosyalarına dokunmaz: kullanıcı ikon dizininde seçili Papirus
//! temasını miras alan küçük bir tema (`<tema>-Nuxut`, gizli) kurar; içinde yalnızca mavi klasör
//! ikonlarının, üç mavi tonu paletin birincil rengine taşınmış kopyaları vardır. İki tasarım boyu
//! ölçeklenebilir dizin olarak (22 px tasarımı 20–31, 48 px tasarımı 32–512); 16 px Papirus'un
//! kendi (renk çeşidi yok). Canlı yeniden yükleme için içeriksiz ikiz (`<ad>-reload`).
//!
//! Yükleme bloklayan IO'dur (görünüm thread'i; sonuç dizin değişim zamanına göre önbellekli),
//! renk taşıma saftır.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use material_colors::color::Argb;
use material_colors::hct::Hct;
use nuxut_core::Color;

/// Nuxut temasının ad eki ve ikizin eki.
pub const SUFFIX: &str = "-Nuxut";
pub const RELOAD_SUFFIX: &str = "-reload";

/// (kaynak boy dizini, yazılan dizin, en küçük, en büyük boy).
pub const SIZES: [(&str, &str, u32, u32); 2] = [("22x22", "22/places", 20, 31), ("48x48", "48/places", 32, 512)];

/// Papirus mavisi: ön yüz, arka yüz, simge (küçük harf; `folder-blue*.svg`).
const BLUE: [&str; 3] = ["#5294e2", "#4877b1", "#1d344f"]; // renk-verisi: Papirus'un kendi mavisi (tema dosyalarında aranan değerler)
/// Bir boyda okunan en çok ikon ve ikon başına bayt (Papirus 48 px ≈ 85 ikon, ≤ 8 KiB). İki boy ve
/// look.icons'un diğer parçaları tek `nuxut-apply` işlemine (≤ 256 parça) sığmalı.
const MAX_ICONS: usize = 110;
const MAX_ICON_BYTES: u64 = 64 * 1024;

/// Okunmuş mavi klasör ikonları: (yazılan dizin, genel ad, SVG).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSet {
    /// Miras alınan tema (`Papirus-Dark`).
    pub base: String,
    pub icons: Vec<(&'static str, String, String)>,
}

/// Papirus ailesinden mi (renk çeşitli klasörleri var)?
pub fn supports(icon_theme: &str) -> bool {
    icon_theme == "Papirus" || icon_theme.starts_with("Papirus-")
}

pub fn theme_name(base: &str) -> String {
    format!("{base}{SUFFIX}")
}

pub fn reload_name(base: &str) -> String {
    format!("{base}{SUFFIX}{RELOAD_SUFFIX}")
}

/// Genel ad mavi bir çeşide mi bağlı (`folder-blue…`, `user-blue…`; `bluegrey` değil)?
fn blue_target(target: &str) -> bool {
    ["folder-blue", "user-blue"].iter().any(|p| target.strip_prefix(p).is_some_and(|rest| rest.starts_with(['-', '.'])))
}

impl FolderSet {
    /// `dir`: seçili Papirus temasının kök dizini. Bir boyun `places/` dizininde mavi çeşide bağ
    /// olan her genel ad okunur (bağ zinciri izlenir). Hiç ikon yoksa `None`.
    pub fn load(base: &str, dir: &Path) -> Option<Self> {
        let mut icons = Vec::new();
        for (src, out, _, _) in SIZES {
            let places = dir.join(src).join("places");
            let Ok(rd) = std::fs::read_dir(&places) else { continue };
            let mut names: Vec<(String, PathBuf)> = rd
                .take(4096)
                .filter_map(Result::ok)
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    let target = std::fs::read_link(e.path()).ok()?;
                    let target = target.file_name()?.to_str()?.to_owned();
                    (name.ends_with(".svg") && blue_target(&target) && !blue_target(&name)).then(|| (name, e.path()))
                })
                .collect();
            names.sort();
            for (name, path) in names.into_iter().take(MAX_ICONS) {
                let ok = std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= MAX_ICON_BYTES);
                if let Some(svg) = ok.then(|| std::fs::read_to_string(&path).ok()).flatten() {
                    icons.push((out, name, svg));
                }
            }
        }
        (!icons.is_empty()).then(|| Self { base: base.to_owned(), icons })
    }
}

/// Önbellek anahtarı: tema dizini + iki kaynak dizinin değişim zamanı.
type Key = (PathBuf, Vec<Option<SystemTime>>);
static CACHE: Mutex<Option<(Key, Option<FolderSet>)>> = Mutex::new(None);

/// [`FolderSet::load`], dizinler değişmedikçe önbellekten (durum isteği her seferinde ikonları
/// yeniden okumasın).
pub fn load_cached(base: &str, dir: &Path) -> Option<FolderSet> {
    let key: Key = (dir.to_path_buf(), SIZES.iter().map(|(s, ..)| std::fs::metadata(dir.join(s).join("places")).and_then(|m| m.modified()).ok()).collect());
    let mut slot = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((k, v)) = slot.as_ref()
        && *k == key
    {
        return v.clone();
    }
    let v = FolderSet::load(base, dir);
    *slot = Some((key, v.clone()));
    v
}

fn to_argb(c: Color) -> Argb {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Argb::new(255, b(c.r), b(c.g), b(c.b))
}

fn hex(a: Argb) -> String {
    format!("#{:02x}{:02x}{:02x}", a.red, a.green, a.blue)
}

/// Üç Papirus mavisinin `accent` rengine taşınmışı: ton aynen (klasörün ışık/gölge düzeni),
/// ton açısı vurgunun, renklilik mavininkiyle orantılı ama vurgudan fazla değil (nötr temada
/// gri klasör). Açı 5°, oran 0,1 adımla yuvarlanır: küçük duvar kâğıdı farkları ikonları yeniden
/// yazdırmaz.
pub fn shades(accent: Color) -> [String; 3] {
    let a = Hct::new(to_argb(accent));
    let front = Hct::new(Argb::from_u32(0xff52_94e2));
    let hue = ((a.get_hue() / 5.0).round() * 5.0).rem_euclid(360.0);
    let ratio = ((a.get_chroma() / front.get_chroma()).min(1.0) * 10.0).round() / 10.0;
    BLUE.map(|b| {
        let src = Hct::new(Argb::from_u32(u32::from_str_radix(&b[1..], 16).unwrap_or(0) | 0xff00_0000));
        hex(Hct::from(hue, src.get_chroma() * ratio, src.get_tone()).into())
    })
}

/// SVG'deki üç maviyi değiştirir (büyük/küçük harf duyarsız) ve Nuxut başlığını ekler.
pub fn recolor(svg: &str, shades: &[String; 3]) -> String {
    let mut out = svg.to_owned();
    for (from, to) in BLUE.iter().zip(shades) {
        out = replace_ci(&out, from, to);
    }
    format!("<!-- Nuxut — klasör ikonu · elle düzenleme bir sonraki uygulamada ezilir -->\n{out}") // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
}

fn replace_ci(text: &str, from: &str, to: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (i, _) in lower.match_indices(from) {
        out.push_str(&text[last..i]);
        out.push_str(to);
        last = i + from.len();
    }
    out.push_str(&text[last..]);
    out
}

/// Temanın `index.theme`'i: gizli (tema seçicilerde görünmez), seçili Papirus'u miras alır.
pub fn index_theme(name: &str, inherits: &str) -> String {
    let dirs: Vec<&str> = SIZES.iter().map(|(_, d, ..)| *d).collect();
    let mut out = format!(
        "# Nuxut — klasör ikonları · elle düzenleme bir sonraki uygulamada ezilir\n[Icon Theme]\nName={name}\nComment=Nuxut\nInherits={inherits},hicolor\nHidden=true\nDirectories={}\n", // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
        dirs.join(",")
    );
    for (_, d, min, max) in SIZES {
        let size = d.split('/').next().unwrap_or("48");
        out.push_str(&format!("\n[{d}]\nContext=Places\nSize={size}\nMinSize={min}\nMaxSize={max}\nType=Scalable\n"));
    }
    out
}

/// İkiz: içeriksiz, asıl Nuxut temasını miras alır (ad değişimi ikon önbelleklerini yeniletir,
/// görüntü aynı).
pub fn reload_index(name: &str, inherits: &str) -> String {
    format!(
        "# Nuxut — klasör ikonları ikizi · elle düzenleme bir sonraki uygulamada ezilir\n[Icon Theme]\nName={name}\nComment=Nuxut\nInherits={inherits}\nHidden=true\nDirectories=\n" // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blue_targets() {
        assert!(blue_target("folder-blue.svg") && blue_target("folder-blue-documents.svg") && blue_target("user-blue-home.svg"));
        assert!(!blue_target("folder-bluegrey.svg") && !blue_target("folder-red.svg") && !blue_target("folder-blues.svg"));
    }

    #[test]
    fn shades_keep_tone_and_follow_accent() {
        // Papirus mavisine yakın vurgu: renkler neredeyse aynı kalır.
        let same = shades(Color::from_hex("#5294e2").unwrap());
        assert_eq!(same[0].len(), 7);
        let tone = |h: &str| Hct::new(Argb::from_u32(u32::from_str_radix(&h[1..], 16).unwrap() | 0xff00_0000)).get_tone();
        for (got, orig) in same.iter().zip(BLUE) {
            assert!((tone(got) - tone(orig)).abs() < 1.5, "{got} {orig}");
        }
        // Kırmızı vurgu: ön yüz kırmızımsı, ton korunur.
        let red = shades(Color::from_hex("#c62828").unwrap());
        let h = Hct::new(Argb::from_u32(u32::from_str_radix(&red[0][1..], 16).unwrap() | 0xff00_0000));
        assert!(h.get_hue() < 40.0 || h.get_hue() > 340.0, "{}", h.get_hue());
        assert!((h.get_tone() - tone(BLUE[0])).abs() < 1.5);
        // Nötr vurgu: gri klasör.
        let grey = shades(Color::from_hex("#8a8a8a").unwrap());
        let g = Hct::new(Argb::from_u32(u32::from_str_radix(&grey[0][1..], 16).unwrap() | 0xff00_0000));
        assert!(g.get_chroma() < 8.0, "{}", g.get_chroma());
        // Küçük vurgu farkı aynı çıktı (yeniden yazım yok).
        assert_eq!(shades(Color::from_hex("#c62828").unwrap()), shades(Color::from_hex("#c72929").unwrap()));
    }

    #[test]
    fn recolor_replaces_all_three_case_insensitively() {
        let svg = "<svg><path style=\"fill:#4877B1\"/><rect style=\"fill:#5294e2\"/><path style=\"fill:#1d344f\"/><rect style=\"fill:#e4e4e4\"/></svg>";
        let s = ["#111111".to_owned(), "#222222".to_owned(), "#333333".to_owned()];
        let out = recolor(svg, &s);
        assert!(out.starts_with("<!-- Nuxut — "));
        assert!(out.contains("fill:#222222") && out.contains("fill:#111111") && out.contains("fill:#333333") && out.contains("#e4e4e4"), "{out}");
    }

    #[test]
    fn index_is_hidden_and_inherits() {
        let t = index_theme("Papirus-Dark-Nuxut", "Papirus-Dark");
        assert!(t.contains("Inherits=Papirus-Dark,hicolor\n") && t.contains("Hidden=true\n"));
        assert!(t.contains("Directories=22/places,48/places\n") && t.contains("[48/places]\nContext=Places\nSize=48\nMinSize=32\nMaxSize=512\nType=Scalable\n"));
        assert!(reload_index("X-reload", "X").contains("Inherits=X\n"));
    }

    /// Sahte Papirus: mavi bağlar okunur, bluegrey ve düz dosya okunmaz; önbellek dizin
    /// değişmedikçe aynı sonucu verir.
    #[test]
    fn loads_blue_links_only() {
        let t = tempfile::tempdir().unwrap();
        let places = t.path().join("Papirus-Dark/48x48/places");
        std::fs::create_dir_all(&places).unwrap();
        std::fs::write(places.join("folder-blue.svg"), "<svg fill=\"#5294e2\"/>").unwrap();
        std::fs::write(places.join("folder-bluegrey.svg"), "<svg/>").unwrap();
        std::os::unix::fs::symlink("folder-blue.svg", places.join("folder.svg")).unwrap();
        std::os::unix::fs::symlink("folder-bluegrey.svg", places.join("folder-grey-like.svg")).unwrap();
        std::fs::write(places.join("bookmark-missing.svg"), "<svg/>").unwrap();
        let dir = t.path().join("Papirus-Dark");
        let set = FolderSet::load("Papirus-Dark", &dir).unwrap();
        assert_eq!(set.icons, [("48/places", "folder.svg".to_owned(), "<svg fill=\"#5294e2\"/>".to_owned())]);
        assert_eq!(load_cached("Papirus-Dark", &dir), Some(set));
        assert!(FolderSet::load("Papirus-Dark", &t.path().join("yok")).is_none());
    }
}
