//! Kilit ret bildirimi Ayarlar'daki yolu ve etiketi birebir söyler (2026-10-16): "Ayarlar › Güç ›
//! Kilit › "Nuxut kilit ekranını kullan"". Kabuğun şema etiketi (`lock.listen_logind`) aynı metin;
//! Ayarlar kaynağı yanda duruyorsa (`../settings`, worktree'de `../../settings`) onun sayfa, kart ve
//! satır etiketleriyle de karşılaştırılır.

use std::path::{Path, PathBuf};

use crate::support::repo::{manifest_dir, repo_path};

/// `anahtar = değer` satırının değeri (tek satırlık iletiler).
fn value(ftl: &str, key: &str) -> String {
    let prefix = format!("{key} = ");
    ftl.lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("{key} katalogda yok"))
        .trim()
        .to_owned()
}

fn settings_locales() -> Option<PathBuf> {
    let root = manifest_dir();
    [root.join("../settings/locales"), root.join("../../settings/locales")].into_iter().find(|p| p.is_dir())
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn refused_notice_names_settings_path_and_label() {
    let settings = settings_locales();
    if settings.is_none() {
        eprintln!("UYARI: Ayarlar kaynağı yanda yok; yalnız kabuğun kendi etiketleri karşılaştırıldı");
    }
    for (lang, page, card, label) in
        [("tr", "Güç", "Kilit", "Nuxut kilit ekranını kullan"), ("en", "Power", "Lock", "Use the Nuxut lock screen")]
    {
        let shell = read(&repo_path(&format!("locales/{lang}.ftl")));
        assert_eq!(value(&shell, "shell-config-lock-listen-logind-label"), label, "{lang}: şema etiketi");
        let body = value(&shell, "shell-lock-refused-body");
        let path = format!("{page} › {card} › \"{label}\"");
        assert!(body.contains(&path), "{lang}: gövde Ayarlar yolunu söylemiyor: {body}\nbeklenen: {path}");
        if let Some(dir) = &settings {
            let s = read(&dir.join(format!("{lang}.ftl")));
            assert_eq!(value(&s, "settings-page-power"), page, "{lang}: Ayarlar sayfa adı değişti");
            assert_eq!(value(&s, "settings-section-lock"), card, "{lang}: Ayarlar kart adı değişti");
            assert_eq!(value(&s, "settings-lock-listen-logind-label"), label, "{lang}: Ayarlar satır etiketi değişti");
        }
    }
}
