//! Masaüstü görünüm seçimleri (ADR-0004 §12-3, §17-1) ve sistemde kurulu olana göre çözümleme
//! (yedekler, eksik paketler).
//!
//! `framework.toml [desktop]` şeması, doğrulaması ve `[colors] mode` çözümü framework'tedir
//! (`nuxut::theme::{DesktopConfig, Desktop, QtBackend, valid_theme_name}`,
//! `ColorsConfig::resolve_mode` → `Theme::mode`); burada kopya ayrıştırma yoktur.
//!
//! Önerilen ikon/imleç **kullanıcı onayı bekleyen önerilerdir** (§12-10); değiştirmek için tek
//! nokta [`RECOMMENDED_ICON_THEME`] / [`RECOMMENDED_CURSOR_THEME`].

use std::path::{Path, PathBuf};

pub use nuxut::theme::{ChoiceValue, Desktop, QtBackend, valid_theme_name};
use nuxut::theme::{ColorSource, FrameworkConfig};
use nuxut_palette::Mode;
use serde::Serialize;

/// Önerilen ikon teması (aile adı; kipe göre `-Dark`/`-Light` varyantı seçilir). Öneri.
pub const RECOMMENDED_ICON_THEME: &str = "Papirus";
/// Önerilen imleç teması (kullanıcı kararı 2026-10-11: Vimix; yoksa Adwaita).
pub const RECOMMENDED_CURSOR_THEME: &str = "Vimix-cursors";
/// Seçilen/önerilen tema kurulu değilse zararsız yedek (her Arch kurulumunda `adwaita-*`).
pub const FALLBACK_ICON_THEME: &str = "Adwaita";
pub const FALLBACK_CURSOR_THEME: &str = "Adwaita";

/// `framework.toml`'dan görünüm için okunan her şey (framework doğrulamasından geçmiş).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameworkLook {
    /// Doğrulanmış `[desktop]` (framework `Theme::desktop` ile aynı doğrulama).
    pub desktop: Desktop,
    /// `[colors] source = "wallpaper"`: tohum renk `palette.toml`'dan.
    pub wallpaper: bool,
    /// Yalnızca `[desktop]` ve `[colors] mode` ile ilgili framework uyarıları.
    pub warnings: Vec<String>,
}

impl FrameworkLook {
    /// Ham şemadan: doğrulama framework'ün (`Theme::from_config`); `parse_warnings`
    /// `nuxut_config` ayrıştırma uyarılarıdır (bilinmeyen anahtar).
    pub fn from_config(cfg: &FrameworkConfig, parse_warnings: Vec<String>) -> Self {
        let (theme, warnings) = nuxut::Theme::from_config(cfg);
        let warnings = parse_warnings.into_iter().chain(warnings).filter(|w| is_look_warning(w)).collect();
        Self { desktop: theme.desktop, wallpaper: cfg.colors.source == ColorSource::Wallpaper, warnings }
    }
}

/// Uyarı birleşik görünümü mü ilgilendiriyor (`desktop.*`, `colors.mode`)?
fn is_look_warning(w: &str) -> bool {
    let key = w.strip_prefix("bilinmeyen anahtar yok sayıldı: ").unwrap_or(w);
    key.starts_with("desktop") || key.starts_with("colors.mode")
}

/// `framework.toml` metninden seçimler (framework ayrıştırıcısı ve doğrulaması). Bozuk TOML'da
/// varsayılanlar (birleşik görünüm kapalı — hiçbir şey yazılmaz) + uyarı.
pub fn parse_framework(text: &str) -> FrameworkLook {
    match nuxut_config::parse_str::<FrameworkConfig>(text) {
        Ok((cfg, warnings)) => FrameworkLook::from_config(&cfg, warnings),
        Err(_) => broken(),
    }
}

/// Okunamayan/bozuk `framework.toml`: birleşik görünüm kapalı sayılır.
pub fn broken() -> FrameworkLook {
    FrameworkLook {
        warnings: vec!["framework.toml okunamadı; birleşik görünüm kapalı sayıldı".into()], // i18n: tanı framework okunamadı uyarısı (log + msg JSON)
        // Varsayılan açık (framework 604a790); bozuk dosya kullanıcının "kapalı"sını da bozmuş
        // olabilir: yazım yapılmaz.
        desktop: Desktop { unified: false, ..Default::default() },
        ..Default::default()
    }
}

/// Kurulu ikon teması (framework tarayıcısından; `msg look status` `icons.installed`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IconEntry {
    /// GTK/Qt ayarlarına yazılan dizin adı (`Papirus-Dark`).
    pub id: String,
    /// Görünen ad (`index.theme` `Name`).
    pub name: String,
    /// `Hidden=true`: seçicide gösterilmemeli (yedek olarak kullanılabilir).
    pub hidden: bool,
}

/// Kurulu imleç teması.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CursorEntry {
    pub id: String,
    pub name: String,
    pub xcursor: bool,
    pub hyprcursor: bool,
}

/// Sistemde ne kurulu (gerçeği [`FsProbe`], testlerde sahte).
pub trait SystemProbe {
    fn icon_themes(&self) -> Vec<IconEntry>;
    fn cursor_themes(&self) -> Vec<CursorEntry>;
    fn icon_theme(&self, name: &str) -> bool {
        self.icon_themes().iter().any(|t| t.id == name)
    }
    fn cursor_theme(&self, name: &str) -> bool {
        self.cursor_themes().iter().any(|t| t.id == name)
    }
    /// GTK3 teması (`<tema>/gtk-3.0`).
    fn gtk_theme(&self, name: &str) -> bool;
    /// Kurulu GTK teması dizini (Nuxut temasının tabanı); bilinmiyorsa `None`.
    fn gtk_theme_dir(&self, _name: &str) -> Option<PathBuf> {
        None
    }
    /// Nuxut GTK temasının tabanını okur (gerçek sistemde IO, sınırlı; sahtede yok).
    fn gtk_base(&self, _name: &str, _dir: &Path) -> Option<super::gtk_theme::GtkBase> {
        None
    }
    /// `path` başka bir temaya/araca bağ (symlink) ve Nuxut onu devralabilir mi (`nuxut-apply`
    /// `external_reason` + `can_take_over`; ör. root'un tema dosyasına bağ)? Kullanıcının kendi
    /// yazılabilir dosyasına bağ değildir (oraya yazılır).
    fn foreign_link(&self, _path: &Path) -> bool {
        false
    }
    /// Qt6 platform teması eklentisi (`kde` → plasma-integration, `qt6ct`).
    fn qt_platform_theme(&self, backend: QtBackend) -> bool;
    /// Qt6 stil eklentisi (`Breeze`).
    fn qt_style(&self, name: &str) -> bool;
    /// `xdg-desktop-portal-gtk` (libadwaita kip değişimini portal üzerinden alır).
    fn portal_gtk(&self) -> bool;
    /// Palete göre klasör ikonları için seçili Papirus temasının mavi klasörleri (gerçek sistemde
    /// IO, önbellekli; sahtede yok). `name`: kurulu ikon teması.
    fn folder_icons(&self, _name: &str) -> Option<super::folders::FolderSet> {
        None
    }
    /// Nuxut Qt stil eklentisi kurulu `lib/nuxut` dizini (`qt6/styles/libnuxut-style.so`).
    fn nuxut_qt_style(&self) -> Option<PathBuf> {
        None
    }
    /// `PATH`'te çalıştırılabilir `name` (ör. `kitty`: Nuxut teması yalnızca kuruluysa yazılır).
    fn program(&self, _name: &str) -> bool {
        false
    }
}

/// Kurulu olana göre çözülmüş, yazılacak değerler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub mode: Mode,
    /// Seçilen (ya da önerilen) ikon teması adı.
    pub icon_requested: String,
    /// Yazılacak ad (kip varyantı ya da yedek).
    pub icon_theme: String,
    pub icon_fallback: bool,
    pub cursor_requested: String,
    pub cursor_theme: String,
    pub cursor_fallback: bool,
    pub cursor_size: u32,
    /// GTK3 tema adı (`adw-gtk3[-dark]`, yoksa yerleşik Adwaita).
    pub gtk_theme: String,
    /// `gtk_theme` adw-gtk3 ise kurulu dizini (Nuxut GTK temasının tabanı; docs/design/look-gtk.md).
    pub gtk_base_dir: Option<PathBuf>,
    /// `None`: hiçbir Qt platform teması kurulu değil (Qt hedefleri yine yazılır, ortam yazılmaz).
    pub qt_backend: Option<QtBackend>,
    /// Kullanılabilir yollar (`kde`, `qt6ct`).
    pub qt_available: Vec<QtBackend>,
    /// Qt stili (`Breeze`, yoksa yerleşik `Fusion`).
    pub qt_style: String,
    /// Nuxut Qt stili kuruluysa `lib/nuxut` dizini: stil `Nuxut`, `QT_PLUGIN_PATH`, token dosyası.
    pub qt_plugin_dir: Option<PathBuf>,
    /// kitty kurulu: Nuxut renkleri + imleç izi (`kitty/nuxut.conf`).
    pub kitty: bool,
    /// Eksik paketler (Arch adı); Nuxut paket kurmaz, Ayarlar kurulum komutunu gösterir.
    pub missing_packages: Vec<&'static str>,
}

/// Kipe göre varyantı olan ikon aileleri: (aile, koyu, açık).
const MODE_VARIANTS: [(&str, &str, &str); 1] = [("Papirus", "Papirus-Dark", "Papirus-Light")];

/// `mode`: çözülmüş etkin kip (framework `Theme::mode`; `auto` framework'te çözülür).
pub fn resolve(sel: &Desktop, mode: Mode, probe: &dyn SystemProbe) -> Resolved {
    let mut missing = Vec::new();
    let dark = mode == Mode::Dark;

    let icon_requested = sel.icon_theme.clone().unwrap_or_else(|| RECOMMENDED_ICON_THEME.to_owned());
    let variant = MODE_VARIANTS
        .iter()
        .find(|(family, ..)| *family == icon_requested)
        .map(|(_, d, l)| if dark { *d } else { *l })
        .filter(|v| probe.icon_theme(v));
    let (icon_theme, icon_fallback) = match variant {
        Some(v) => (v.to_owned(), false),
        None if probe.icon_theme(&icon_requested) => (icon_requested.clone(), false),
        None => {
            if icon_requested == RECOMMENDED_ICON_THEME {
                missing.push("papirus-icon-theme");
            }
            (FALLBACK_ICON_THEME.to_owned(), true)
        }
    };

    let cursor_requested = sel.cursor_theme.clone().unwrap_or_else(|| RECOMMENDED_CURSOR_THEME.to_owned());
    let (cursor_theme, cursor_fallback) = if probe.cursor_theme(&cursor_requested) {
        (cursor_requested.clone(), false)
    } else {
        if cursor_requested == RECOMMENDED_CURSOR_THEME {
            missing.push("vimix-cursors");
        }
        (FALLBACK_CURSOR_THEME.to_owned(), true)
    };

    let adw = if dark { "adw-gtk3-dark" } else { "adw-gtk3" };
    let gtk_theme = if probe.gtk_theme(adw) {
        adw.to_owned()
    } else {
        missing.push("adw-gtk-theme");
        if dark { "Adwaita-dark" } else { "Adwaita" }.to_owned()
    };
    let gtk_base_dir = if gtk_theme == adw { probe.gtk_theme_dir(adw) } else { None };
    if !probe.portal_gtk() {
        missing.push("xdg-desktop-portal-gtk");
    }

    let qt_available: Vec<QtBackend> =
        [QtBackend::Kde, QtBackend::Qt6ct].into_iter().filter(|b| probe.qt_platform_theme(*b)).collect();
    let qt_backend = match sel.qt_backend {
        QtBackend::Auto => qt_available.first().copied(),
        b => Some(b),
    };
    // Kullanıcı açıkça qt6ct seçmediyse birincil (canlı) yol önerilir.
    if !qt_available.contains(&QtBackend::Kde) && sel.qt_backend != QtBackend::Qt6ct {
        missing.push("plasma-integration");
    }
    if qt_backend == Some(QtBackend::Qt6ct) && !qt_available.contains(&QtBackend::Qt6ct) {
        missing.push("qt6ct");
    }
    let qt_plugin_dir = probe.nuxut_qt_style();
    let breeze = probe.qt_style("Breeze");
    if !breeze {
        missing.push("breeze");
    }
    // Nuxut stili Breeze'in (yoksa Fusion'ın) üstüne çizer (ADR-0009).
    let qt_style = if qt_plugin_dir.is_some() {
        super::qt::NUXUT_STYLE.to_owned()
    } else if breeze {
        "Breeze".to_owned()
    } else {
        "Fusion".to_owned()
    };

    Resolved {
        mode,
        icon_requested,
        icon_theme,
        icon_fallback,
        cursor_requested,
        cursor_theme,
        cursor_fallback,
        cursor_size: sel.cursor_size,
        gtk_theme,
        gtk_base_dir,
        qt_backend,
        qt_available,
        qt_style,
        kitty: probe.program("kitty"),
        qt_plugin_dir,
        missing_packages: missing,
    }
}

/// Gerçek sistem yoklaması: ikon/imleç temaları framework tarayıcısından
/// (`nuxut_image::icon::installed_*`; sınırlı), GTK temaları ve Qt eklentileri var/yok denetimi.
#[derive(Debug, Clone)]
pub struct FsProbe {
    pub icons: Vec<IconEntry>,
    pub cursors: Vec<CursorEntry>,
    pub theme_dirs: Vec<PathBuf>,
    /// İkon teması arama dizinleri (`~/.icons`, `$XDG_DATA_HOME/icons`, `$XDG_DATA_DIRS/*/icons`).
    pub icon_dirs: Vec<PathBuf>,
    pub qt_plugin_dir: PathBuf,
    pub portal_dir: PathBuf,
    /// Nuxut Qt stil eklentisinin kurulu olduğu `lib/nuxut` dizini (ADR-0009).
    pub nuxut_lib: Option<PathBuf>,
}

/// Nuxut Qt stil eklentisinin kurulu olduğu `lib/nuxut` (install.sh `$LIBEXEC`; yardımcı
/// ikililerle aynı arama sırası): çalışan ikilinin `../lib/nuxut`'u, `~/.local/lib/nuxut`,
/// `/usr/lib/nuxut`. Qt6 eklentisi şart (Qt5'inki isteğe bağlı).
fn nuxut_lib_with_qt_style(home: Option<&Path>) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.join("lib/nuxut")));
    let local = home.map(|h| h.join(".local/lib/nuxut"));
    [exe, local, Some(PathBuf::from("/usr/lib/nuxut"))]
        .into_iter()
        .flatten()
        .find(|d| d.join("qt6/styles/libnuxut-style.so").is_file())
}

impl FsProbe {
    pub fn from_env(home: Option<&Path>, data_home: Option<&Path>) -> Self {
        let mut data: Vec<PathBuf> = Vec::new();
        if let Some(d) = data_home {
            data.push(d.to_path_buf());
        }
        let sys = std::env::var("XDG_DATA_DIRS").ok().filter(|s| !s.is_empty());
        let sys = sys.as_deref().unwrap_or("/usr/local/share:/usr/share");
        data.extend(sys.split(':').map(PathBuf::from).filter(|p| p.is_absolute()).take(16));
        let mut theme_dirs: Vec<PathBuf> = home.map(|h| h.join(".themes")).into_iter().collect();
        theme_dirs.extend(data.iter().map(|d| d.join("themes")));
        let mut icon_dirs: Vec<PathBuf> = home.map(|h| h.join(".icons")).into_iter().collect();
        icon_dirs.extend(data.iter().map(|d| d.join("icons")));
        let icons = nuxut_image::icon::installed_icon_themes()
            .into_iter()
            .map(|t| IconEntry { id: t.id, name: t.name, hidden: t.hidden })
            .collect();
        let cursors = nuxut_image::icon::installed_cursor_themes()
            .into_iter()
            .map(|t| CursorEntry { id: t.id, name: t.name, xcursor: t.xcursor, hyprcursor: t.hyprcursor })
            .collect();
        Self {
            icons,
            cursors,
            theme_dirs,
            icon_dirs,
            qt_plugin_dir: PathBuf::from("/usr/lib/qt6/plugins"),
            portal_dir: PathBuf::from("/usr/share/xdg-desktop-portal/portals"),
            nuxut_lib: nuxut_lib_with_qt_style(home),
        }
    }
}

impl SystemProbe for FsProbe {
    fn icon_themes(&self) -> Vec<IconEntry> {
        self.icons.clone()
    }

    fn cursor_themes(&self) -> Vec<CursorEntry> {
        self.cursors.clone()
    }

    fn gtk_theme(&self, name: &str) -> bool {
        valid_theme_name(name) && self.theme_dirs.iter().any(|d| d.join(name).join("gtk-3.0").exists())
    }

    fn gtk_theme_dir(&self, name: &str) -> Option<PathBuf> {
        if !valid_theme_name(name) {
            return None;
        }
        self.theme_dirs.iter().map(|d| d.join(name)).find(|d| d.join("gtk-3.0").is_dir())
    }

    fn gtk_base(&self, name: &str, dir: &Path) -> Option<super::gtk_theme::GtkBase> {
        super::gtk_theme::load_cached(name, dir)
    }

    fn foreign_link(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
            && nuxut_apply::external_reason(path).is_some_and(|r| r.can_take_over(path))
    }

    fn qt_platform_theme(&self, backend: QtBackend) -> bool {
        let file = match backend {
            QtBackend::Kde => "KDEPlasmaPlatformTheme6.so",
            QtBackend::Qt6ct => "libqt6ct.so",
            QtBackend::Auto => return false,
        };
        self.qt_plugin_dir.join("platformthemes").join(file).exists()
    }

    fn qt_style(&self, name: &str) -> bool {
        name == "Breeze" && self.qt_plugin_dir.join("styles/breeze6.so").exists()
    }

    fn folder_icons(&self, name: &str) -> Option<super::folders::FolderSet> {
        if !valid_theme_name(name) || !super::folders::supports(name) {
            return None;
        }
        let dir = self.icon_dirs.iter().map(|d| d.join(name)).find(|d| d.join("index.theme").is_file())?;
        super::folders::load_cached(name, &dir)
    }

    fn nuxut_qt_style(&self) -> Option<PathBuf> {
        self.nuxut_lib.clone()
    }

    fn portal_gtk(&self) -> bool {
        self.portal_dir.join("gtk.portal").exists()
    }

    fn program(&self, name: &str) -> bool {
        use std::os::unix::fs::PermissionsExt;
        let Some(path) = std::env::var_os("PATH") else { return false };
        std::env::split_paths(&path)
            .take(64)
            .filter(|d| d.is_absolute())
            .any(|d| std::fs::metadata(d.join(name)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Sahte sistem: verilen adlar kurulu.
    #[derive(Debug, Default)]
    pub(crate) struct FakeProbe {
        pub icons: Vec<&'static str>,
        pub cursors: Vec<&'static str>,
        pub gtk: Vec<&'static str>,
        pub qt: Vec<QtBackend>,
        pub breeze: bool,
        pub portal: bool,
        pub base: Option<crate::services::look::gtk_theme::GtkBase>,
        pub links: Vec<PathBuf>,
        pub programs: Vec<&'static str>,
        pub qt_lib: Option<PathBuf>,
        pub folders: Option<crate::services::look::folders::FolderSet>,
    }

    impl FakeProbe {
        pub(crate) fn full() -> Self {
            Self {
                icons: vec!["Papirus", "Papirus-Dark", "Papirus-Light", "Adwaita"],
                cursors: vec!["Vimix-cursors", "Bibata-Modern-Classic", "Adwaita"],
                gtk: vec!["adw-gtk3", "adw-gtk3-dark"],
                qt: vec![QtBackend::Kde, QtBackend::Qt6ct],
                breeze: true,
                portal: true,
                base: None,
                links: Vec::new(),
                programs: Vec::new(),
                qt_lib: None,
                folders: None,
            }
        }
    }

    impl SystemProbe for FakeProbe {
        fn icon_themes(&self) -> Vec<IconEntry> {
            self.icons.iter().map(|n| IconEntry { id: (*n).into(), name: (*n).into(), hidden: *n == "Adwaita" }).collect()
        }
        fn cursor_themes(&self) -> Vec<CursorEntry> {
            self.cursors
                .iter()
                .map(|n| CursorEntry { id: (*n).into(), name: (*n).into(), xcursor: true, hyprcursor: false })
                .collect()
        }
        fn gtk_theme(&self, name: &str) -> bool {
            self.gtk.contains(&name)
        }
        fn gtk_theme_dir(&self, name: &str) -> Option<PathBuf> {
            self.gtk.contains(&name).then(|| PathBuf::from("/usr/share/themes").join(name))
        }
        fn gtk_base(&self, name: &str, _dir: &Path) -> Option<crate::services::look::gtk_theme::GtkBase> {
            self.base.clone().filter(|b| b.name == name)
        }
        fn foreign_link(&self, path: &Path) -> bool {
            self.links.iter().any(|l| l == path)
        }
        fn qt_platform_theme(&self, backend: QtBackend) -> bool {
            self.qt.contains(&backend)
        }
        fn qt_style(&self, name: &str) -> bool {
            self.breeze && name == "Breeze"
        }
        fn portal_gtk(&self) -> bool {
            self.portal
        }
        fn folder_icons(&self, name: &str) -> Option<crate::services::look::folders::FolderSet> {
            self.folders.clone().filter(|s| s.base == name)
        }
        fn nuxut_qt_style(&self) -> Option<PathBuf> {
            self.qt_lib.clone()
        }
        fn program(&self, name: &str) -> bool {
            self.programs.contains(&name)
        }
    }

    #[test]
    fn parses_desktop_through_framework_schema() {
        let f = parse_framework(
            "[colors]\nsource = \"wallpaper\"\nmode = \"light\"\n[desktop]\nunified = true\nicon_theme = \"breeze\"\n\
             cursor_theme = \"\"\ncursor_size = 32\nqt_backend = \"qt6ct\"\n",
        );
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        assert!(f.wallpaper);
        let s = f.desktop;
        assert!(s.unified);
        assert_eq!(s.icon_theme.as_deref(), Some("breeze"));
        assert_eq!(s.cursor_theme, None);
        assert_eq!(s.cursor_size, 32);
        assert_eq!(s.qt_backend, QtBackend::Qt6ct);
        assert_eq!(s.qt_backend.name(), "qt6ct");
        // Eski ad framework'te kabul edilir.
        assert_eq!(parse_framework("[desktop]\nqt_backend = \"qtct\"\n").desktop.qt_backend, QtBackend::Qt6ct);
    }

    #[test]
    fn invalid_values_warn_and_default() {
        let f = parse_framework(
            "[colors]\nmode = \"dim\"\nsurface = \"#101010\"\n[desktop]\nicon_theme = \"../etc\"\ncursor_size = 4000\n\
             qt_backend = \"kvantum\"\nextra = 1\n[shape]\nradius_small = 9999.0\n",
        );
        // colors.mode, icon_theme, cursor_size, qt_backend, desktop.extra; [shape] uyarısı süzülür.
        assert_eq!(f.warnings.len(), 5, "{:?}", f.warnings);
        assert_eq!(f.desktop.icon_theme, None);
        assert_eq!(f.desktop.cursor_size, nuxut::theme::CURSOR_SIZE_MAX, "framework sınırlar");
        assert_eq!(f.desktop.qt_backend, QtBackend::Auto);
        // Yanlış tür ve bozuk dosya (ikisi de ayrıştırılamaz): varsayılan açık olsa da
        // (framework 604a790) kapalı + uyarı.
        for text in ["[desktop]\nunified = \"yes\"\n", "[desktop\nunified = false"] {
            let f = parse_framework(text);
            assert!(!f.desktop.unified, "{text}");
            assert_eq!(f.warnings.len(), 1);
        }
        // Dosya var ama [desktop] yok: açık.
        assert!(parse_framework("[colors]\nmode = \"dark\"\n").desktop.unified);
    }

    #[test]
    fn resolves_recommended_with_mode_variant() {
        let mut sel = Desktop::default();
        let r = resolve(&sel, Mode::Dark, &FakeProbe::full());
        assert_eq!(r.icon_theme, "Papirus-Dark");
        assert_eq!(r.cursor_theme, RECOMMENDED_CURSOR_THEME);
        assert_eq!(r.gtk_theme, "adw-gtk3-dark");
        assert_eq!(r.qt_backend, Some(QtBackend::Kde));
        assert_eq!(r.qt_style, "Breeze");
        assert!(r.missing_packages.is_empty());
        let r = resolve(&sel, Mode::Light, &FakeProbe::full());
        assert_eq!((r.icon_theme.as_str(), r.gtk_theme.as_str()), ("Papirus-Light", "adw-gtk3"));
        // Açıkça seçilmiş varyant kipi izlemez.
        sel.icon_theme = Some("Papirus-Dark".into());
        assert_eq!(resolve(&sel, Mode::Light, &FakeProbe::full()).icon_theme, "Papirus-Dark");
    }

    #[test]
    fn missing_themes_fall_back_harmlessly() {
        let probe = FakeProbe { icons: vec!["Adwaita"], cursors: vec!["Adwaita"], qt: vec![QtBackend::Qt6ct], ..Default::default() };
        let r = resolve(&Desktop::default(), Mode::Dark, &probe);
        assert_eq!((r.icon_theme.as_str(), r.icon_fallback), (FALLBACK_ICON_THEME, true));
        assert_eq!((r.cursor_theme.as_str(), r.cursor_fallback), (FALLBACK_CURSOR_THEME, true));
        assert_eq!(r.gtk_theme, "Adwaita-dark");
        assert_eq!(r.qt_backend, Some(QtBackend::Qt6ct), "auto: kde yoksa qt6ct");
        assert_eq!(r.qt_style, "Fusion");
        assert_eq!(
            r.missing_packages,
            ["papirus-icon-theme", "vimix-cursors", "adw-gtk-theme", "xdg-desktop-portal-gtk", "plasma-integration", "breeze"]
        );
        // Önerilebilen her paketin deposu ve hedefi katalogda.
        for name in &r.missing_packages {
            assert!(super::super::catalog::package(name).is_some(), "{name}");
        }
        // Kullanıcının seçtiği kurulu olmayan tema için paket önerilmez.
        let sel = Desktop { icon_theme: Some("Tela".into()), ..Default::default() };
        assert!(!resolve(&sel, Mode::Dark, &probe).missing_packages.contains(&"papirus-icon-theme"));
        // Hiç Qt yolu yok.
        let none = FakeProbe::default();
        assert_eq!(resolve(&Desktop::default(), Mode::Dark, &none).qt_backend, None);
    }
}
