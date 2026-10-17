//! Görünüm paket kataloğu: eksik paketlerin deposu ve hedefi, Ayarlar'ın ikon/imleç seçicisinde
//! "kurulabilir" olarak gösterdiği tema aileleri (ADR-0004 §9). Nuxut paket kurmaz; Ayarlar bu
//! bilgiden kurulum komutunu gösterir.
//!
//! Aile tanıma kurulu tema kimliğinin (dizin adı) küçük harfli önekiyle yapılır: paketler birden
//! çok varyant kurar (`Qogir-cursors`, `Qogir-white-cursors`), dizin adları paketten pakete değişir.

use serde::Serialize;

use super::selection::{CursorEntry, IconEntry};

/// Paketin deposu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Resmî depo (`pacman -S`).
    Repo,
    /// AUR (paru/yay).
    Aur,
}

/// Eksik paketin hangi görünüm hedefine ait olduğu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Icons,
    Cursors,
    Gtk,
    Portal,
    Qt,
}

/// Önerilebilen eksik paketler: (Arch adı, depo, hedef). `selection::resolve`'un önerdiği her ad
/// burada olmalı (testli).
const PACKAGES: [(&str, Source, Target); 7] = [
    ("papirus-icon-theme", Source::Repo, Target::Icons),
    ("vimix-cursors", Source::Repo, Target::Cursors),
    ("adw-gtk-theme", Source::Repo, Target::Gtk),
    ("xdg-desktop-portal-gtk", Source::Repo, Target::Portal),
    ("plasma-integration", Source::Repo, Target::Qt),
    ("qt6ct", Source::Repo, Target::Qt),
    ("breeze", Source::Repo, Target::Qt),
];

/// Bir paketin deposu ve hedefi.
pub fn package(name: &str) -> Option<(Source, Target)> {
    PACKAGES.iter().find(|(n, ..)| *n == name).map(|&(_, s, t)| (s, t))
}

/// Kurulabilir bir tema ailesi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Family {
    /// Kararlı kimlik (`vimix`).
    pub family: &'static str,
    /// Görünen ad (özel ad; çevrilmez).
    pub name: &'static str,
    pub package: &'static str,
    pub source: Source,
    /// Kurulu tema kimliğinin küçük harfli öneki.
    prefix: &'static str,
}

const fn fam(family: &'static str, name: &'static str, package: &'static str, source: Source, prefix: &'static str) -> Family {
    Family { family, name, package, source, prefix }
}

/// İmleç aileleri (önerilen Vimix başta; install.sh `--with-cursors` ile aynı set + Bibata).
pub const CURSOR_FAMILIES: [Family; 8] = [
    fam("vimix", "Vimix", "vimix-cursors", Source::Repo, "vimix"),
    fam("capitaine", "Capitaine", "capitaine-cursors", Source::Repo, "capitaine"),
    fam("material", "Material", "material-cursors-git", Source::Aur, "material_"),
    fam("oreo", "Oreo", "oreo-cursors-bin", Source::Aur, "oreo_"),
    fam("volantes", "Volantes", "volantes-cursors", Source::Aur, "volantes"),
    fam("qogir", "Qogir", "qogir-cursor-theme", Source::Aur, "qogir"),
    fam("sweet", "Sweet", "sweet-cursors-git", Source::Aur, "sweet"),
    fam("bibata", "Bibata", "bibata-cursor-theme-bin", Source::Aur, "bibata"),
];

/// İkon aileleri (önerilen Papirus başta).
pub const ICON_FAMILIES: [Family; 8] = [
    fam("papirus", "Papirus", "papirus-icon-theme", Source::Repo, "papirus"),
    fam("breeze", "Breeze", "breeze-icons", Source::Repo, "breeze"),
    fam("tela-circle", "Tela Circle", "tela-circle-icon-theme-git", Source::Aur, "tela-circle"),
    fam("colloid", "Colloid", "colloid-icon-theme-git", Source::Aur, "colloid"),
    fam("qogir", "Qogir", "qogir-icon-theme", Source::Aur, "qogir"),
    fam("kora", "Kora", "kora-icon-theme", Source::Aur, "kora"),
    fam("fluent", "Fluent", "fluent-icon-theme-git", Source::Aur, "fluent"),
    fam("whitesur", "WhiteSur", "whitesur-icon-theme", Source::Aur, "whitesur"),
];

/// Aile başına kurulu varyant üst sınırı (Oreo ~40 varyant kurar).
pub const MAX_VARIANTS: usize = 64;

/// `msg look status` içindeki katalog girdisi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogEntry {
    pub family: &'static str,
    pub name: &'static str,
    pub package: &'static str,
    pub source: Source,
    /// Bu aileye ait kurulu tema kimlikleri (boş: kurulu değil).
    pub installed: Vec<String>,
}

impl Family {
    fn matches(&self, id: &str) -> bool {
        id.get(..self.prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(self.prefix))
    }
}

fn entries<'a>(families: &[Family], ids: impl Iterator<Item = &'a str> + Clone) -> Vec<CatalogEntry> {
    families
        .iter()
        .map(|f| CatalogEntry {
            family: f.family,
            name: f.name,
            package: f.package,
            source: f.source,
            installed: ids.clone().filter(|id| f.matches(id)).take(MAX_VARIANTS).map(str::to_owned).collect(),
        })
        .collect()
}

pub fn cursor_catalog(installed: &[CursorEntry]) -> Vec<CatalogEntry> {
    entries(&CURSOR_FAMILIES, installed.iter().map(|e| e.id.as_str()))
}

pub fn icon_catalog(installed: &[IconEntry]) -> Vec<CatalogEntry> {
    entries(&ICON_FAMILIES, installed.iter().map(|e| e.id.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::look::selection::{RECOMMENDED_CURSOR_THEME, RECOMMENDED_ICON_THEME};

    fn cursor(id: &str) -> CursorEntry {
        CursorEntry { id: id.into(), name: id.into(), xcursor: true, hyprcursor: false }
    }

    #[test]
    fn families_match_by_prefix_case_insensitive() {
        let installed = [cursor("Vimix-cursors"), cursor("Vimix-white-cursors"), cursor("Qogir-white-cursors"), cursor("Adwaita"), cursor("v")];
        let c = cursor_catalog(&installed);
        assert_eq!(c[0].family, "vimix");
        assert_eq!(c[0].installed, ["Vimix-cursors", "Vimix-white-cursors"]);
        assert_eq!(c.iter().find(|e| e.family == "qogir").unwrap().installed, ["Qogir-white-cursors"]);
        assert!(c.iter().find(|e| e.family == "oreo").unwrap().installed.is_empty());
        // Çok baytlı ad önek sınırında panik yok.
        assert!(cursor_catalog(&[cursor("çğüşöı-cursors")]).iter().all(|e| e.installed.is_empty()));
    }

    #[test]
    fn recommended_families_first_and_known() {
        assert!(CURSOR_FAMILIES[0].matches(RECOMMENDED_CURSOR_THEME));
        assert!(ICON_FAMILIES[0].matches(RECOMMENDED_ICON_THEME));
        for f in CURSOR_FAMILIES.iter().chain(&ICON_FAMILIES) {
            assert!(!f.prefix.is_empty() && f.prefix == f.prefix.to_ascii_lowercase(), "{}", f.family);
        }
    }

    #[test]
    fn json_shape() {
        let v = serde_json::to_value(&cursor_catalog(&[cursor("Vimix-cursors")])[0]).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"family": "vimix", "name": "Vimix", "package": "vimix-cursors", "source": "repo", "installed": ["Vimix-cursors"]})
        );
        assert_eq!(package("qt6ct"), Some((Source::Repo, Target::Qt)));
        assert_eq!(package("nope"), None);
    }
}
