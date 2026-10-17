//! `nuxut-shell msg look status` ve `msg look plan` yanıtları (ADR-0004 §9; Ayarlar'ın Hoş geldin
//! ve Görünüm sayfası okur). Tek satır JSON; tüm listeler sınırlı.

use serde::Serialize;

use super::backend::ApplyReport;
use super::catalog::{self, CatalogEntry, Source, Target};
use super::env::{Conflict, MAX_CONFLICTS};
use super::plan::{LookPlan, PlanEntry, Restart};
use super::selection::{
    ChoiceValue, CursorEntry, Desktop, FrameworkLook, IconEntry, QtBackend, RECOMMENDED_CURSOR_THEME, RECOMMENDED_ICON_THEME,
    Resolved,
};

/// `msg look status` sürümü (alan eklemek uyumlu; alan silmek/anlam değiştirmek sürüm artırır).
/// 2: `missing_packages` nesne listesi, `installed` dolu nesne listeleri.
/// 3: `missing_packages[].target`, `icons.catalog` / `cursors.catalog` (kurulabilir aileler).
/// 4: `themes_rev` (kurulu ikon/imleç/GTK temaları değişince artan sayaç; `look::watch`).
/// 5: `last_apply.external` (başka aracın yönettiği, dokunulmayan dosyalar: ayar, `~/…` yol, neden)
///    ve `last_apply.paused` (yeniden yazım döngüsü bekçisinin durdurduğu ayarlar).
/// 6: `last_apply.taken_over` (Nuxut'un devraldığı, önceden başka aracın yönettiği dosyalar: ayar,
///    `~/…` yol; birleşik görünümü kapatmak özgün bağı/izni geri koyar).
pub const STATUS_VERSION: u32 = 6;

/// Kurulu tema listesi üst sınırı (framework tarayıcısı zaten ≤ 512 verir).
pub const MAX_INSTALLED: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemeStatus {
    /// Seçilen ya da (seçim yoksa) önerilen ad.
    pub requested: String,
    /// Yazılan/yazılacak ad (kip varyantı ya da yedek).
    pub current: String,
    /// İstenen kurulu değil, yedek kullanılıyor.
    pub fallback: bool,
    /// Önerilen (kullanıcı onayı bekleyen öneri; §12-10).
    pub recommended: &'static str,
    /// `framework.toml` açıkça bir ad seçmiş mi (yoksa önerilen izlenir).
    pub explicit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IconStatus {
    #[serde(flatten)]
    pub theme: ThemeStatus,
    /// Kurulu ikon temaları, görünen ada göre (`hidden` olanları seçici gizler).
    pub installed: Vec<IconEntry>,
    /// Kurulabilir aileler (paket, depo, kurulu varyantlar).
    pub catalog: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CursorStatus {
    #[serde(flatten)]
    pub theme: ThemeStatus,
    pub size: u32,
    pub installed: Vec<CursorEntry>,
    /// Kurulabilir aileler (paket, depo, kurulu varyantlar).
    pub catalog: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QtStatus {
    /// `framework.toml` seçimi: `auto` | `kde` | `qt6ct`.
    pub selected: &'static str,
    /// Etkin yol; hiçbir Qt platform teması kurulu değilse `null`.
    pub active: Option<&'static str>,
    pub available: Vec<&'static str>,
    /// Qt stili (`Breeze`, yoksa `Fusion`).
    pub style: String,
}

/// Eksik paket (Arch adı), deposu ve hedefi; Ayarlar kurulum komutunu buna göre gösterir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MissingPackage {
    pub name: &'static str,
    /// `repo` (pacman) | `aur`.
    pub source: Source,
    /// `icons` | `cursors` | `gtk` | `portal` | `qt`.
    pub target: Target,
}

/// Katalogda olmayan ad olmamalı (testli); olursa resmî depo + `gtk` sayılır.
pub fn missing_package(name: &'static str) -> MissingPackage {
    let (source, target) = catalog::package(name).unwrap_or((Source::Repo, Target::Gtk));
    MissingPackage { name, source, target }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LookStatus {
    pub version: u32,
    pub unified: bool,
    /// `dark` | `light`.
    pub mode: &'static str,
    /// `[colors] source = "wallpaper"`.
    pub wallpaper_colors: bool,
    pub icons: IconStatus,
    pub cursors: CursorStatus,
    /// GTK3 tema adı (`adw-gtk3[-dark]`, yoksa `Adwaita[-dark]`).
    pub gtk_theme: String,
    pub qt_backend: QtStatus,
    /// Eksik paketler; Nuxut paket kurmaz.
    pub missing_packages: Vec<MissingPackage>,
    pub conflicts: Vec<Conflict>,
    /// Açık uygulamaların yeniden açılınca alacakları (birleşik görünüm kapalıyken boş).
    pub restart_needed: Vec<Restart>,
    /// Dış yazım arka ucu: `disabled` (nuxut-apply gelene kadar hiçbir şey yazılmaz).
    pub writer: &'static str,
    /// Son uygulamanın raporu (henüz uygulama yoksa `null`).
    pub last_apply: Option<ApplyReport>,
    /// `framework.toml` `[desktop]`/`[colors] mode` uyarıları.
    pub warnings: Vec<String>,
    /// Kurulu tema listesinin sürümü: ikon/imleç/GTK tema dizinlerinde tema eklenince, silinince
    /// ya da `index.theme` değişince artar (`look::watch`). Shell süreci başına 0'dan başlar;
    /// izleme kapalıysa (inotify limiti) 0'da kalır. Ayarlar listeyi değer **farklıysa** yeniden
    /// okur. Listeler her `status` çağrısında taze taranır (önbellek yok).
    pub themes_rev: u64,
}

fn theme_status(requested: &str, current: &str, fallback: bool, recommended: &'static str, explicit: bool) -> ThemeStatus {
    ThemeStatus { requested: requested.into(), current: current.into(), fallback, recommended, explicit }
}

#[derive(Debug)]
pub struct StatusInput<'a> {
    pub framework: &'a FrameworkLook,
    pub resolved: &'a Resolved,
    pub icons: &'a [IconEntry],
    pub cursors: &'a [CursorEntry],
    pub conflicts: Vec<Conflict>,
    pub plan: &'a LookPlan,
    pub writer: &'static str,
    pub last_apply: Option<ApplyReport>,
    pub themes_rev: u64,
}

pub fn status(i: StatusInput<'_>) -> LookStatus {
    let sel: &Desktop = &i.framework.desktop;
    let r = i.resolved;
    let mut conflicts = i.conflicts;
    conflicts.truncate(MAX_CONFLICTS);
    LookStatus {
        version: STATUS_VERSION,
        unified: sel.unified,
        mode: r.mode.name(),
        wallpaper_colors: i.framework.wallpaper,
        icons: IconStatus {
            theme: theme_status(&r.icon_requested, &r.icon_theme, r.icon_fallback, RECOMMENDED_ICON_THEME, sel.icon_theme.is_some()),
            installed: i.icons.iter().take(MAX_INSTALLED).cloned().collect(),
            catalog: catalog::icon_catalog(i.icons),
        },
        cursors: CursorStatus {
            theme: theme_status(&r.cursor_requested, &r.cursor_theme, r.cursor_fallback, RECOMMENDED_CURSOR_THEME, sel.cursor_theme.is_some()),
            size: r.cursor_size,
            installed: i.cursors.iter().take(MAX_INSTALLED).cloned().collect(),
            catalog: catalog::cursor_catalog(i.cursors),
        },
        gtk_theme: r.gtk_theme.clone(),
        qt_backend: QtStatus {
            selected: sel.qt_backend.name(),
            active: r.qt_backend.map(QtBackend::name),
            available: r.qt_available.iter().map(|b| b.name()).collect(),
            style: r.qt_style.clone(),
        },
        missing_packages: r.missing_packages.iter().map(|n| missing_package(n)).collect(),
        conflicts,
        restart_needed: i.plan.restart_needed.clone(),
        writer: i.writer,
        last_apply: i.last_apply,
        themes_rev: i.themes_rev,
        warnings: i.framework.warnings.iter().take(32).cloned().collect(),
    }
}

/// `msg look plan` yanıtı.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanReply {
    pub version: u32,
    pub unified: bool,
    pub entries: Vec<PlanEntry>,
    /// Oturum ortamı (environment.d ile aynı).
    pub env: Vec<(String, String)>,
    /// Değişimden sonra çalışacak canlı eylemler (adları).
    pub live: Vec<&'static str>,
    pub writer: &'static str,
}

pub fn plan_reply(unified: bool, plan: &LookPlan, entries: Vec<PlanEntry>, writer: &'static str) -> PlanReply {
    PlanReply {
        version: STATUS_VERSION,
        unified,
        entries,
        env: plan.env.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect(),
        live: plan.live.iter().map(|a| a.name()).collect(),
        writer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_palette::Mode;

    use crate::services::look::selection::{SystemProbe, parse_framework, resolve, tests::FakeProbe};

    #[test]
    fn status_json_contract() {
        let fw = parse_framework("[colors]\nmode = \"light\"\n[desktop]\nunified = true\ncursor_size = 32\nbogus = 1\n");
        let probe = FakeProbe { cursors: vec!["Adwaita"], ..FakeProbe::full() };
        let r = resolve(&fw.desktop, Mode::Light, &probe);
        let conflict = Conflict { kind: "env", name: Some("QT_STYLE_OVERRIDE".into()), value: Some("kvantum".into()), source: "session_env".into() };
        let s = status(StatusInput {
            framework: &fw,
            resolved: &r,
            icons: &probe.icon_themes(),
            cursors: &probe.cursor_themes(),
            conflicts: vec![conflict],
            plan: &LookPlan { restart_needed: vec![Restart::Gtk3Colors], ..Default::default() },
            writer: "disabled",
            last_apply: None,
            themes_rev: 7,
        });
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(v["version"], 6);
        assert_eq!(v["themes_rev"], 7);
        assert_eq!(v["unified"], true);
        assert_eq!(v["mode"], "light");
        assert_eq!(v["icons"]["current"], "Papirus-Light");
        assert_eq!(v["icons"]["recommended"], "Papirus");
        assert_eq!(v["icons"]["explicit"], false);
        assert_eq!(v["icons"]["installed"][0], serde_json::json!({"id": "Papirus", "name": "Papirus", "hidden": false}));
        assert_eq!(v["icons"]["installed"].as_array().unwrap().len(), 4);
        assert_eq!(v["cursors"]["current"], "Adwaita");
        assert_eq!(v["cursors"]["fallback"], true);
        assert_eq!(v["cursors"]["size"], 32);
        assert_eq!(
            v["cursors"]["installed"],
            serde_json::json!([{"id": "Adwaita", "name": "Adwaita", "xcursor": true, "hyprcursor": false}])
        );
        assert_eq!(v["missing_packages"], serde_json::json!([{"name": "vimix-cursors", "source": "repo", "target": "cursors"}]));
        assert_eq!(v["cursors"]["recommended"], "Vimix-cursors");
        assert_eq!(v["cursors"]["catalog"][0]["family"], "vimix");
        assert_eq!(v["cursors"]["catalog"][0]["installed"], serde_json::json!([]));
        assert_eq!(v["icons"]["catalog"][0]["family"], "papirus");
        assert_eq!(v["icons"]["catalog"][0]["installed"][0], "Papirus");
        assert_eq!(v["qt_backend"]["selected"], "auto");
        assert_eq!(v["qt_backend"]["active"], "kde");
        assert_eq!(v["qt_backend"]["available"], serde_json::json!(["kde", "qt6ct"]));
        assert_eq!(v["conflicts"][0]["name"], "QT_STYLE_OVERRIDE");
        assert!(v["conflicts"][0].get("value").is_some());
        assert_eq!(v["restart_needed"], serde_json::json!(["gtk3_colors"]));
        assert_eq!(v["writer"], "disabled");
        assert!(v["last_apply"].is_null());
        assert_eq!(v["warnings"].as_array().unwrap().len(), 1);
        // Tek satır.
        assert!(!serde_json::to_string(&s).unwrap().contains('\n'));
    }


    /// v5: `last_apply.external` (dokunulmayan dosya: ayar, `~/…` yol, neden) ve `last_apply.paused`;
    /// v6: `last_apply.taken_over` (devralınan dosya: ayar, `~/…` yol).
    #[test]
    fn last_apply_carries_external_paused_and_taken_over() {
        use crate::services::look::backend::{ExternalTarget, TakenOver};
        let report = ApplyReport {
            external: vec![ExternalTarget { setting: "look.mode", path: "~/.config/gtk-3.0/gtk.css".into(), reason: "read_only" }],
            paused: vec!["look.icons"],
            taken_over: vec![TakenOver { setting: "look.mode", path: "~/.config/gtk-4.0/gtk.css".into() }],
            ..Default::default()
        };
        let v = serde_json::to_value(&report).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["external"], serde_json::json!([{"setting": "look.mode", "path": "~/.config/gtk-3.0/gtk.css", "reason": "read_only"}]));
        assert_eq!(v["paused"], serde_json::json!(["look.icons"]));
        assert_eq!(v["taken_over"], serde_json::json!([{"setting": "look.mode", "path": "~/.config/gtk-4.0/gtk.css"}]));
    }
    #[test]
    fn package_sources() {
        assert_eq!(missing_package("adw-gtk-theme"), MissingPackage { name: "adw-gtk-theme", source: Source::Repo, target: Target::Gtk });
        assert_eq!(missing_package("vimix-cursors").target, Target::Cursors);
        assert_eq!(missing_package("breeze").target, Target::Qt);
    }
}
