//! Birleşik görünüm planı (ADR-0004 §5, §6, §7; saf): tema + çözülmüş seçimler → ayar başına
//! yazılacak parçalar ve ardından çalışacak canlı eylemler.
//!
//! Parça modeli `nuxut-apply`'ınkiyle (ADR-0001) birebir: `ManagedFile`, `Line` (dosya
//! sonu), `IniKey`, `Dconf`. Plan yalnızca **sonraki** değerleri taşır; önceki değerler, snapshot
//! ve yazım `nuxut-apply`'ın işidir (bkz. [`super::backend`]).

use std::path::{Path, PathBuf};

use nuxut::i18n::t;
use nuxut::theme::{ContrastMode, Palette as ThemePalette, PaletteMeta, Typography};
use serde::Serialize;

use super::env;
use super::folders::{self, FolderSet};
use super::fonts;
use super::gtk;
use super::kitty;
use super::gtk_theme::{self, GtkBase};

use super::palette::{self, Seed};
use super::qt;
use super::selection::{Desktop, QtBackend, Resolved};

/// Ayar kimlikleri (`nuxut-apply` `SettingId`; §6). Geri alma bu düzeyde.
pub mod setting {
    /// Birleşik görünümün bütünü (ortam dosyası; kapatma = ilk snapshot'a dönüş).
    pub const UNIFIED: &str = "look.unified";
    /// Koyu/açık + renkler + GTK/Qt teması/stili.
    pub const MODE: &str = "look.mode";
    pub const ICONS: &str = "look.icons";
    pub const CURSOR: &str = "look.cursor";
    /// Yazı tipleri ve metin ölçeği (`text-scaling-factor`, `gtk-xft-dpi`; GTK adı referans boyda).
    pub const FONTS: &str = "look.fonts";
    /// GTK animasyonları (`enable-animations`, `gtk-enable-animations` = `!Theme::reduced_motion()`).
    pub const MOTION: &str = "look.motion";
    pub const QT_BACKEND: &str = "look.qt_backend";
    /// Uygulama sırası (§7: dconf/renkler önce, ortam en son).
    pub const ALL: [&str; 7] = [MODE, ICONS, CURSOR, FONTS, MOTION, QT_BACKEND, UNIFIED];
}

/// Hedef dizinler (mutlak).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME` (yoksa `~/.config`).
    pub config: PathBuf,
    /// `$XDG_DATA_HOME` (yoksa `~/.local/share`).
    pub data: PathBuf,
}

impl Dirs {
    /// Ortamdan. Göreli XDG değerleri yok sayılır (XDG kuralı); `HOME` yoksa `None`.
    pub fn from_env() -> Option<Self> {
        let abs = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_absolute());
        let home = abs("HOME")?;
        Some(Self {
            config: abs("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
            data: abs("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share")),
            home,
        })
    }

    /// Kullanıcıya/loglara gösterilen biçim: ev dizini `~`.
    pub fn display(&self, p: &Path) -> String {
        match p.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => p.display().to_string(),
        }
    }
}

/// Üreticilerin verdiği INI anahtarı. `section` köşeli parantezsiz, KDE alt bölümleri dahil
/// (`Colors:Header][Inactive`). Kullanıcı dosyasına birleştirme `nuxut-apply` INI adaptörünün işi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IniKey {
    pub section: String,
    pub key: String,
    pub value: String,
}

impl IniKey {
    pub fn new(section: &str, key: &str, value: impl Into<String>) -> Self {
        Self { section: section.into(), key: key.into(), value: value.into() }
    }
}

/// Yazımın dokunduğu en küçük birim (`nuxut_apply::Fragment` karşılığı).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fragment {
    /// Tümü Nuxut'un dosyası (ilk satırdaki `Nuxut — ` başlığıyla tanınır).
    ManagedFile { file: PathBuf },
    /// Dosya sonuna tek satır; satır (ya da `nuxut.css` anan yorum) varsa eklenmez.
    Line { file: PathBuf, marker: String },
    /// INI `[bölüm] anahtar` (KDE `[$e]` ekleri korunur).
    IniKey { file: PathBuf, section: String, key: String },
    /// dconf anahtarı; değer GVariant metni.
    Dconf { key: String },
    /// Önceki sürümün yazdığı dosya parçasının (`ManagedFile`, `Line`) kaldırılması
    /// (`nuxut-apply` `Absent`; değer boş).
    Remove(Box<Fragment>),
}

impl Fragment {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ManagedFile { .. } => "managed_file",
            Self::Line { .. } => "line",
            Self::IniKey { .. } => "ini_key",
            Self::Dconf { .. } => "dconf",
            Self::Remove(_) => "remove",
        }
    }
}

/// Bir ayarın yazımları (`nuxut_apply::Change` karşılığı).
#[derive(Debug, Clone, PartialEq)]
pub struct LookChange {
    pub setting: &'static str,
    /// Kullanıcıya gösterilen kısa açıklama (değer içermez).
    pub label: String,
    /// (parça, sonraki değer). Dosya için bütün içerik, satır için satırın kendisi.
    pub writes: Vec<(Fragment, String)>,
}

/// Dosya dışı, snapshot'sız canlı eylemler (§7). Yalnızca bağlı oldukları ayarın yazımı
/// gerçekten bir şey değiştirdiyse çalışır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveAction {
    /// `/KGlobalSettings org.kde.KGlobalSettings.notifyChange(type, 0)`: 0 palet, 1 yazı tipi, 2 stil.
    KdeNotifyChange(i32),
    /// `/KIconLoader org.kde.KIconLoader.iconChanged(0)`.
    KdeIconChanged,
    /// GTK temasını canlı yeniden yükletir: dconf `gtk-theme` önce eşdeğer ikize, sonra asıl ada
    /// (docs/design/look-gtk.md). Son değer `nuxut-apply`'ın yazdığıyla aynı.
    GtkReload,
    /// Açık kitty pencerelerine `SIGUSR1` (yapılandırmayı yeniden okur; yalnızca bu kullanıcının,
    /// açılışı tamamlanmış süreçleri).
    KittyReload,
    /// GTK ikon önbelleğini yeniletir: dconf `icon-theme` önce içeriksiz ikize, sonra asıl ada
    /// (klasör ikonları aynı adla yeniden yazıldığında; docs/design/look-gtk.md).
    IconReload { twin: String, name: String },
    /// Hyprland `setcursor <tema> <boy>` (`hypr_config` `HyprControl` üzerinden).
    HyprSetCursor { theme: String, size: u32 },
    /// systemd kullanıcı yöneticisi `SetEnvironment` + D-Bus `UpdateActivationEnvironment`.
    SessionEnvironment(Vec<(&'static str, String)>),
    /// Birleşik görünüm kapatıldı (§8-3): systemd `UnsetEnvironment`. D-Bus etkinleştirme
    /// ortamında silme yok (`UpdateActivationEnvironment` yalnızca atar); dbus-broker systemd'ninkini
    /// kullanır, klasik dbus-daemon'da değerler oturum sonuna kadar kalır.
    SessionUnset(Vec<&'static str>),
}

impl LiveAction {
    /// Eylemi tetikleyen ayarlar.
    pub fn settings(&self) -> &'static [&'static str] {
        match self {
            Self::KdeNotifyChange(1) => &[setting::FONTS],
            Self::KdeNotifyChange(_) => &[setting::MODE],
            Self::KdeIconChanged => &[setting::ICONS],
            Self::GtkReload => &[setting::MODE],
            Self::KittyReload => &[setting::MODE, setting::MOTION],
            Self::IconReload { .. } => &[setting::ICONS],
            Self::HyprSetCursor { .. } => &[setting::CURSOR],
            Self::SessionEnvironment(_) => &[setting::UNIFIED, setting::CURSOR, setting::QT_BACKEND],
            Self::SessionUnset(_) => &[setting::UNIFIED],
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::KdeNotifyChange(0) => "kde_palette",
            Self::KdeNotifyChange(1) => "kde_fonts",
            Self::KdeNotifyChange(_) => "kde_style",
            Self::KdeIconChanged => "kde_icons",
            Self::GtkReload => "gtk_reload",
            Self::KittyReload => "kitty_reload",
            Self::IconReload { .. } => "icon_reload",
            Self::HyprSetCursor { .. } => "hypr_setcursor",
            Self::SessionEnvironment(_) => "session_env",
            Self::SessionUnset(_) => "session_unset",
        }
    }
}

/// Plan girdisi.
#[derive(Debug, Clone, Copy)]
pub struct LookInput<'a> {
    pub colors: &'a ThemePalette,
    /// `[colors] source = "wallpaper"` iken `palette.toml`'un bilgisi (tohum, şema, kontrast).
    pub generated: Option<&'a PaletteMeta>,
    /// Statik kaynakta seçili Nuxut renk teması (`Theme::color_theme`): dış palet temanın
    /// tanımından kurulur (`palette::Seed::with_color_theme`).
    pub color_theme: Option<nuxut::theme::ColorThemeId>,
    pub typography: &'a Typography,
    /// Köşe yarıçapları (Nuxut Qt stilinin token dosyası).
    pub shape: &'a nuxut::theme::Shape,
    /// `Theme::contrast`: yüksek karşıtlıkta dışa aktarılan palet de eşiğe itilir.
    pub contrast: ContrastMode,
    /// `Theme::reduced_motion()`: GTK animasyonları kapatılır.
    pub reduced_motion: bool,
    pub selection: &'a Desktop,
    pub resolved: &'a Resolved,
    /// Nuxut GTK temasının tabanı (okunmuş, öneklenmiş adw-gtk3); yoksa eski yol (kullanıcı
    /// `gtk.css` satırı). docs/design/look-gtk.md.
    pub gtk_base: Option<&'a GtkBase>,
    /// Başka bir temaya bağ olan, devralınacak kullanıcı GTK CSS'leri (`gtk-4.0/gtk.css`,
    /// `gtk-4.0/gtk-dark.css`, `gtk-3.0/gtk.css`; `SystemProbe::foreign_link`).
    pub css_links: &'a [PathBuf],
    /// Palete göre klasör ikonları (seçili tema Papirus ailesindense; `look::folders`).
    pub folders: Option<&'a FolderSet>,
}

/// Açık uygulamaların yeniden açılınca alacağı parçalar (§7; Ayarlar dürüstçe gösterir).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Restart {
    /// GTK3 renkleri yalnızca açılışta okunur (B3).
    Gtk3Colors,
    /// GTK4 renkleri yalnızca açılışta okunur (B3); koyu/açık geçişi canlıdır (B4).
    Gtk4Colors,
    /// qt6ct yolunda Qt paleti/ikon/yazı tipi (B5).
    QtColors,
    /// İmleç: XWayland/X11 ve ortamı açılışta okuyanlar.
    XCursor,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LookPlan {
    pub changes: Vec<LookChange>,
    pub live: Vec<LiveAction>,
    /// Oturum ortamı (environment.d ile aynı); imleç alt kümesi `hypr_config`'in `env` girdisi.
    pub env: Vec<(&'static str, String)>,
    pub restart_needed: Vec<Restart>,
}

impl LookPlan {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Toplayıcı: (ayar, parça, değer) sırayla; sonra ayar başına gruplanır.
struct Builder(Vec<(&'static str, Fragment, String)>);

impl Builder {
    fn file(&mut self, setting: &'static str, file: PathBuf, text: String) {
        self.0.push((setting, Fragment::ManagedFile { file }, text));
    }

    fn line(&mut self, setting: &'static str, file: PathBuf, line: &str) {
        self.0.push((setting, Fragment::Line { file, marker: line.to_owned() }, line.to_owned()));
    }

    fn remove(&mut self, setting: &'static str, fragment: Fragment) {
        self.0.push((setting, Fragment::Remove(Box::new(fragment)), String::new()));
    }

    /// Kullanıcının GTK CSS'i (`gtk-<v>/gtk.css`): `import` ise `nuxut.css` satırı, değilse önceki
    /// sürümün satırı kaldırılır. Dosya başka bir temaya bağsa (`links`; ör. `gtk-4.0/gtk.css` →
    /// `/usr/share/themes/Dracula/…`) içerik temanınkidir, kullanıcınınki değil: devralınır ve
    /// Nuxut'un dosyasıyla değiştirilir (tema CSS'i Nuxut renklerini ezerdi). Birleşik görünümü
    /// kapatmak bağı geri koyar (`nuxut-apply` devralma). docs/design/look-gtk.md.
    fn user_css(&mut self, links: &[PathBuf], file: PathBuf, import: bool) {
        if links.contains(&file) {
            let body = if import { format!("{}\n", gtk::GTK_IMPORT) } else { String::new() };
            let text = format!("{}{body}", gtk_theme::css_header("kullanıcı GTK CSS'i (devralınan tema bağının yerine)")); // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
            self.file(setting::MODE, file, text);
        } else if import {
            self.line(setting::MODE, file, gtk::GTK_IMPORT);
        } else {
            self.remove(setting::MODE, Fragment::Line { file, marker: gtk::GTK_IMPORT.to_owned() });
        }
    }

    /// INI anahtarları; ayarı anahtardan sınıflandırır.
    fn ini(&mut self, file: &Path, keys: Vec<IniKey>) {
        for k in keys {
            let setting = classify_ini(&k.section, &k.key);
            self.0.push((setting, Fragment::IniKey { file: file.to_path_buf(), section: k.section, key: k.key }, k.value));
        }
    }

    fn dconf(&mut self, keys: Vec<(String, String)>) {
        for (key, value) in keys {
            let setting = classify_dconf(&key);
            self.0.push((setting, Fragment::Dconf { key }, value));
        }
    }

    fn finish(self) -> Vec<LookChange> {
        setting::ALL
            .iter()
            .filter_map(|s| {
                let writes: Vec<(Fragment, String)> =
                    self.0.iter().filter(|(x, ..)| x == s).map(|(_, f, v)| (f.clone(), v.clone())).collect();
                (!writes.is_empty()).then(|| LookChange { setting: s, label: label(s), writes })
            })
            .collect()
    }
}

fn label(setting: &str) -> String {
    match setting {
        setting::MODE => t!("shell-look-mode-label"),
        setting::ICONS => t!("shell-look-icons-label"),
        setting::CURSOR => t!("shell-look-cursor-label"),
        setting::FONTS => t!("shell-look-fonts-label"),
        setting::MOTION => t!("shell-look-motion-label"),
        setting::QT_BACKEND => t!("shell-look-qt-backend-label"),
        _ => t!("shell-look-unified-label"),
    }
}

fn classify_ini(section: &str, key: &str) -> &'static str {
    match (section, key) {
        (_, "gtk-icon-theme-name" | "icon_theme") | ("Icons", _) => setting::ICONS,
        (_, "gtk-cursor-theme-name" | "gtk-cursor-theme-size" | "cursorTheme" | "cursorSize") | ("Icon Theme", _) => setting::CURSOR,
        (_, "gtk-font-name" | "gtk-xft-dpi" | "font" | "menuFont" | "toolBarFont" | "smallestReadableFont" | "fixed") => setting::FONTS,
        (_, "gtk-enable-animations") => setting::MOTION,
        ("Appearance", "custom_palette" | "color_scheme_path" | "style") => setting::QT_BACKEND,
        _ => setting::MODE,
    }
}

fn classify_dconf(key: &str) -> &'static str {
    let name = key.rsplit('/').next().unwrap_or(key);
    match name {
        "icon-theme" => setting::ICONS,
        "cursor-theme" | "cursor-size" => setting::CURSOR,
        "font-name" | "document-font-name" | "monospace-font-name" | "text-scaling-factor" => setting::FONTS,
        "enable-animations" => setting::MOTION,
        _ => setting::MODE,
    }
}

/// Birleşik görünüm kapalıysa boş plan (§8-1: hiçbir dış hedef yazılmaz).
pub fn build(input: &LookInput<'_>, dirs: &Dirs) -> LookPlan {
    if !input.selection.unified {
        return LookPlan::default();
    }
    let r = input.resolved;
    let mode = r.mode;
    let seed = Seed::new(input.colors, input.generated).with_color_theme(input.color_theme, input.generated);
    let colors = palette::build(input.colors, seed, mode, input.contrast);
    let active = colors.get(mode);
    let f = fonts::fonts(input.typography);
    let cfg = &dirs.config;
    let mut b = Builder(Vec::new());

    // Palete göre klasör ikonları (kullanıcı isteği 2026-10-16; look::folders): seçili Papirus'u
    // miras alan gizli tema; yazılan ikon teması adı onun adı.
    let icon_theme = match input.folders {
        Some(set) => {
            let icons = dirs.data.join("icons");
            let name = folders::theme_name(&set.base);
            let shades = folders::shades(colors.light.primary);
            for (dir, file, svg) in &set.icons {
                b.file(setting::ICONS, icons.join(&name).join(dir).join(file), folders::recolor(svg, &shades));
            }
            b.file(setting::ICONS, icons.join(&name).join("index.theme"), folders::index_theme(&name, &set.base));
            let twin = folders::reload_name(&set.base);
            b.file(setting::ICONS, icons.join(&twin).join("index.theme"), folders::reload_index(&twin, &name));
            name
        }
        None => r.icon_theme.clone(),
    };

    // GTK (§5.2).
    b.file(setting::MODE, cfg.join("gtk-4.0/nuxut.css"), gtk::gtk4_css(&colors, mode));
    b.user_css(input.css_links, cfg.join("gtk-4.0/gtk.css"), true);
    if input.css_links.contains(&cfg.join("gtk-4.0/gtk-dark.css")) {
        b.user_css(input.css_links, cfg.join("gtk-4.0/gtk-dark.css"), true);
    }
    let gtk3 = gtk::gtk3_css(&colors, mode);
    let gtk_theme_name = match input.gtk_base {
        // Canlı GTK3: renkler Nuxut temasında; kullanıcı gtk.css'indeki önceki satır ve dosya
        // kaldırılır (açılışta yüklenip temayı gölgelerdi). docs/design/look-gtk.md.
        Some(base) => {
            let themes = dirs.data.join("themes");
            let theme = themes.join(gtk_theme::THEME);
            let twin = themes.join(gtk_theme::RELOAD_THEME);
            let css = |what: &str| gtk_theme::css_header(what);
            b.file(setting::MODE, theme.join("gtk-3.0/base.css"), format!("{}{}", css(&format!("GTK 3 taban ({})", base.name)), base.css3));
            b.file(setting::MODE, theme.join("gtk-3.0/gtk.css"), gtk_theme::gtk3_theme_css(&css("GTK 3 teması"), &gtk_theme::prefixed_colors(&gtk3))); // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
            if let Some(gtk4) = gtk_theme::gtk4_theme_css(&css("GTK 4 teması"), &base.dir) { // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
                b.file(setting::MODE, theme.join("gtk-4.0/gtk.css"), gtk4);
            }
            b.file(setting::MODE, theme.join("index.theme"), gtk_theme::index_theme(&gtk_theme::ini_header("GTK teması"), gtk_theme::THEME)); // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
            for v in ["3.0", "4.0"] {
                b.file(setting::MODE, twin.join(format!("gtk-{v}/gtk.css")), gtk_theme::reload_css(&css("GTK teması ikizi"), v)); // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
            }
            b.file(setting::MODE, twin.join("index.theme"), gtk_theme::index_theme(&gtk_theme::ini_header("GTK teması ikizi"), gtk_theme::RELOAD_THEME)); // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
            b.user_css(input.css_links, cfg.join("gtk-3.0/gtk.css"), false);
            b.remove(setting::MODE, Fragment::ManagedFile { file: cfg.join("gtk-3.0/nuxut.css") });
            gtk_theme::THEME.to_owned()
        }
        None => {
            b.file(setting::MODE, cfg.join("gtk-3.0/nuxut.css"), gtk3);
            b.user_css(input.css_links, cfg.join("gtk-3.0/gtk.css"), true);
            r.gtk_theme.clone()
        }
    };
    let animations = !input.reduced_motion;
    let settings = gtk::settings_ini(
        &gtk_theme_name,
        &icon_theme,
        &r.cursor_theme,
        r.cursor_size,
        f.ui_pango.as_deref(),
        &f.xft_dpi,
        animations,
        mode,
    );
    b.ini(&cfg.join("gtk-3.0/settings.ini"), settings.clone());
    let mut gtk4_settings = settings;
    gtk4_settings.push(gtk::gtk4_color_scheme(mode));
    b.ini(&cfg.join("gtk-4.0/settings.ini"), gtk4_settings);
    b.dconf(gtk::dconf(&gtk::DconfInput {
        mode,
        gtk_theme: &gtk_theme_name,
        icon: &icon_theme,
        cursor: &r.cursor_theme,
        cursor_size: r.cursor_size,
        ui_font: f.ui_pango.as_deref(),
        document_font: f.document_pango.as_deref(),
        mono_font: f.mono_pango.as_deref(),
        text_scaling_factor: &f.text_scaling_factor,
        animations,
        accent: colors.light.primary,
    }));

    // Qt/KDE (§5.3): kdeglobals her iki yolda da (KDE uygulamaları oradan okur).
    b.ini(&cfg.join("kdeglobals"), qt::kdeglobals(active, &f, &icon_theme, &r.qt_style));
    b.file(setting::MODE, dirs.data.join("color-schemes/Nuxut.colors"), qt::colors_file(active));
    if r.qt_backend == Some(QtBackend::Qt6ct) {
        for (dir, qt6) in [("qt6ct", true), ("qt5ct", false)] {
            let scheme = cfg.join(dir).join("colors/nuxut.conf");
            b.file(setting::MODE, scheme.clone(), qt::qtct_scheme(active, qt6));
            b.ini(&cfg.join(dir).join(format!("{dir}.conf")), qt::qtct_conf(&scheme.to_string_lossy(), &icon_theme, &r.qt_style));
        }
    }

    // Nuxut Qt stili (ADR-0009): kuruluysa token dosyası (eklenti izler, açık uygulamalar canlı).
    if r.qt_plugin_dir.is_some() {
        b.file(setting::MODE, cfg.join("nuxut/qt-style.conf"), qt::style_tokens(input.shape.radius_small, input.shape.radius_medium, active.primary));
    }

    // İmleç (§5.4): X istemcileri ve temasız yedek; Qt5/wl_cursor.
    b.ini(&dirs.home.join(".icons/default/index.theme"), vec![IniKey::new("Icon Theme", "Inherits", r.cursor_theme.clone())]);
    b.ini(
        &cfg.join("kcminputrc"),
        vec![IniKey::new("Mouse", "cursorTheme", r.cursor_theme.clone()), IniKey::new("Mouse", "cursorSize", r.cursor_size.to_string())],
    );

    // kitty (kullanıcı isteği 2026-10-16): Nuxut renkleri + imleç izi; yalnızca kuruluysa.
    if r.kitty {
        let dir = cfg.join("kitty");
        if let Some(text) = kitty::colors_conf(&palette::full(seed, mode, input.contrast)) {
            b.file(setting::MODE, dir.join("nuxut.conf"), text);
            b.line(setting::MODE, dir.join("kitty.conf"), kitty::INCLUDE_COLORS);
        }
        b.file(setting::MOTION, dir.join("nuxut-motion.conf"), kitty::motion_conf(input.reduced_motion));
        b.line(setting::MOTION, dir.join("kitty.conf"), kitty::INCLUDE_MOTION);
    }

    // Ortam (§5.6).
    let env_vars = env::variables(r, fonts::qt_font_dpi(input.typography, r.qt_backend == Some(QtBackend::Qt6ct)));
    b.file(setting::UNIFIED, cfg.join("environment.d").join(env::ENV_FILE), env::environment_d(&env_vars));

    // Canlı eylemler, §7 sırasıyla (dconf yazımın kendisi).
    let mut live = vec![LiveAction::KdeNotifyChange(0), LiveAction::KdeNotifyChange(2), LiveAction::KdeIconChanged];
    if f.ui_qt.is_some() {
        live.insert(1, LiveAction::KdeNotifyChange(1));
    }
    if input.gtk_base.is_some() {
        live.push(LiveAction::GtkReload);
    }
    if r.kitty {
        live.push(LiveAction::KittyReload);
    }
    if let Some(set) = input.folders {
        live.push(LiveAction::IconReload { twin: folders::reload_name(&set.base), name: folders::theme_name(&set.base) });
    }
    live.push(LiveAction::HyprSetCursor { theme: r.cursor_theme.clone(), size: r.cursor_size });
    live.push(LiveAction::SessionEnvironment(env_vars.clone()));
    // Metin ölçeği qt6ct yolundan çıktıysa (ölçek 1 ya da kdeglobals yolu) Nuxut'un önceki
    // QT_FONT_DPI'ı oturumdan kalkar; dosyadan zaten düştü.
    if !env_vars.iter().any(|(k, _)| *k == "QT_FONT_DPI") {
        live.push(LiveAction::SessionUnset(vec!["QT_FONT_DPI"]));
    }

    // Nuxut temasıyla GTK3 renkleri canlı (gtk_reload).
    let mut restart_needed = if input.gtk_base.is_some() { vec![Restart::Gtk4Colors, Restart::XCursor] } else { vec![Restart::Gtk3Colors, Restart::Gtk4Colors, Restart::XCursor] };
    if r.qt_backend == Some(QtBackend::Qt6ct) {
        restart_needed.push(Restart::QtColors);
    }

    LookPlan { changes: b.finish(), live, env: env_vars, restart_needed }
}

/// Planın kullanıcıya/Ayarlar'a gösterilen özeti (`msg look plan`). Dosya içerikleri yerine
/// boyları verilir (yanıt sınırlı kalır); değerler tema adı/renk/yazı tipi (kullanıcı içeriği değil).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanEntry {
    pub setting: &'static str,
    pub kind: &'static str,
    /// `~/…` yolu ya da dconf anahtarı.
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Satır/anahtar değeri; yönetilen dosyada `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Yönetilen dosyanın boyu (bayt).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<usize>,
}

pub fn summarize(plan: &LookPlan, dirs: &Dirs) -> Vec<PlanEntry> {
    let mut out = Vec::new();
    for c in &plan.changes {
        for (frag, value) in &c.writes {
            let (target, section, key, value, bytes) = match frag {
                Fragment::ManagedFile { file } => (dirs.display(file), None, None, None, Some(value.len())),
                Fragment::Line { file, .. } => (dirs.display(file), None, None, Some(value.clone()), None),
                Fragment::IniKey { file, section, key } => {
                    (dirs.display(file), Some(section.clone()), Some(key.clone()), Some(value.clone()), None)
                }
                Fragment::Dconf { key } => (key.clone(), None, None, Some(value.clone()), None),
                Fragment::Remove(inner) => match inner.as_ref() {
                    Fragment::Line { file, .. } | Fragment::ManagedFile { file } | Fragment::IniKey { file, .. } => (dirs.display(file), None, None, None, None),
                    Fragment::Dconf { key } => (key.clone(), None, None, None, None),
                    Fragment::Remove(_) => continue,
                },
            };
            out.push(PlanEntry { setting: c.setting, kind: frag.kind(), target, section, key, value, bytes });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use nuxut_palette::Mode;

    use crate::services::look::selection::{SystemProbe as _, resolve, tests::FakeProbe};

    fn dirs() -> Dirs {
        Dirs { home: "/home/u".into(), config: "/home/u/.config".into(), data: "/home/u/.local/share".into() }
    }

    fn plan_for(sel: &Desktop, probe: &FakeProbe) -> LookPlan {
        plan_in(sel, Mode::Dark, probe)
    }

    fn plan_in(sel: &Desktop, mode: Mode, probe: &FakeProbe) -> LookPlan {
        let theme = nuxut::Theme::default();
        let r = resolve(sel, mode, probe);
        let folders = probe.folder_icons(&r.icon_theme);
        build(
            &LookInput {
                colors: &theme.colors,
                generated: None,
                color_theme: None,
                typography: &theme.typography,
                shape: &theme.shape,
                contrast: theme.contrast,
                reduced_motion: theme.reduced_motion(),
                selection: sel,
                resolved: &r,
                gtk_base: None,
                css_links: &[],
                folders: folders.as_ref(),
            },
            &dirs(),
        )
    }

    fn unified() -> Desktop {
        Desktop { unified: true, ..Default::default() }
    }

    fn targets(p: &LookPlan) -> Vec<String> {
        summarize(p, &dirs()).into_iter().map(|e| e.target).collect()
    }

    /// Nuxut GTK teması (docs/design/look-gtk.md): renkler temada, ikiz, kullanıcı gtk.css
    /// satırı kaldırılır, gtk-theme = Nuxut, canlı gtk_reload, GTK3 yeniden başlatma gerekmez.
    #[test]
    fn gtk_base_builds_live_nuxut_theme() {
        let theme = nuxut::Theme::default();
        let r = resolve(&unified(), Mode::Dark, &FakeProbe::full());
        let base = GtkBase {
            name: "adw-gtk3-dark".into(),
            dir: "/usr/share/themes/adw-gtk3-dark".into(),
            css3: "@define-color nuxut_window_bg_color #000;\n".into(),
        };
        let p = build(
            &LookInput {
                colors: &theme.colors,
                generated: None,
                color_theme: None,
                typography: &theme.typography,
                shape: &theme.shape,
                contrast: theme.contrast,
                reduced_motion: false,
                selection: &unified(),
                resolved: &r,
                gtk_base: Some(&base),
                css_links: &[],
                folders: None,
            },
            &dirs(),
        );
        let mode = p.changes.iter().find(|c| c.setting == setting::MODE).unwrap();
        let file = |path: &str| {
            mode.writes
                .iter()
                .find(|(f, _)| matches!(f, Fragment::ManagedFile { file } if file.ends_with(path)))
                .map(|(_, v)| v.as_str())
                .unwrap_or_else(|| panic!("{path} yok"))
        };
        let base_css = file("themes/Nuxut/gtk-3.0/base.css");
        assert!(base_css.starts_with("/* Nuxut — GTK 3 taban (adw-gtk3-dark)") && base_css.ends_with(&base.css3));
        let gtk3 = file("themes/Nuxut/gtk-3.0/gtk.css");
        assert!(gtk3.contains("@import url(\"base.css\");") && gtk3.contains("@define-color nuxut_window_bg_color #"), "{gtk3}");
        assert!(file("themes/Nuxut/gtk-4.0/gtk.css").contains("file:///usr/share/themes/adw-gtk3-dark/gtk-4.0/gtk.css"));
        assert!(file("themes/Nuxut-reload/gtk-3.0/gtk.css").contains("../../Nuxut/gtk-3.0/gtk.css"));
        assert!(file("themes/Nuxut/index.theme").contains("GtkTheme=Nuxut"));
        for (f, _) in &mode.writes {
            if let Fragment::ManagedFile { file } = f {
                let text = file.to_string_lossy();
                assert!(!text.ends_with("gtk-3.0/nuxut.css"), "kullanıcı nuxut.css yazılmaz");
            }
        }
        let removed: Vec<&str> = mode.writes.iter().filter_map(|(f, _)| match f { Fragment::Remove(i) => Some(i.kind()), _ => None }).collect();
        assert_eq!(removed, ["line", "managed_file"]);
        assert!(mode.writes.iter().any(|(f, v)| matches!(f, Fragment::Dconf { key } if key.ends_with("/gtk-theme")) && v == "'Nuxut'"));
        assert!(mode.writes.iter().any(|(f, v)| matches!(f, Fragment::IniKey { key, .. } if key == "gtk-theme-name") && v == "Nuxut"));
        assert!(p.live.contains(&LiveAction::GtkReload));
        assert!(!p.restart_needed.contains(&Restart::Gtk3Colors) && p.restart_needed.contains(&Restart::Gtk4Colors));
        // Özet: kaldırma parçaları değersiz görünür.
        assert!(summarize(&p, &dirs()).iter().any(|e| e.kind == "remove" && e.target == "~/.config/gtk-3.0/gtk.css" && e.value.is_none()));
        // Taban yoksa eski yol (canlı yeniden yükleme yok).
        assert!(!plan_for(&unified(), &FakeProbe::full()).live.contains(&LiveAction::GtkReload));
    }

    /// kitty kuruluysa: renkler look.mode'da, imleç izi look.motion'da, iki `include` satırı ve
    /// canlı yeniden yükleme; kurulu değilse kitty'ye hiç dokunulmaz.
    #[test]
    fn kitty_theme_only_when_installed() {
        let kitty_targets = |p: &LookPlan| -> Vec<(&'static str, String)> {
            p.changes
                .iter()
                .flat_map(|c| c.writes.iter().map(move |(f, _)| (c.setting, f)))
                .filter_map(|(s, f)| match f {
                    Fragment::ManagedFile { file } | Fragment::Line { file, .. } if file.to_string_lossy().contains("/kitty/") => {
                        Some((s, file.file_name().unwrap().to_string_lossy().into_owned()))
                    }
                    _ => None,
                })
                .collect()
        };
        let none = plan_for(&unified(), &FakeProbe::full());
        assert!(kitty_targets(&none).is_empty() && !none.live.contains(&LiveAction::KittyReload));
        let p = plan_for(&unified(), &FakeProbe { programs: vec!["kitty"], ..FakeProbe::full() });
        let mut got = kitty_targets(&p);
        got.sort();
        let want = [
            (setting::MODE, "kitty.conf"),
            (setting::MODE, "nuxut.conf"),
            (setting::MOTION, "kitty.conf"),
            (setting::MOTION, "nuxut-motion.conf"),
        ];
        assert_eq!(got, want.map(|(s, f)| (s, f.to_owned())));
        assert!(p.live.contains(&LiveAction::KittyReload));
        let motion = p.changes.iter().find(|c| c.setting == setting::MOTION).unwrap();
        assert!(motion.writes.iter().any(|(f, v)| matches!(f, Fragment::ManagedFile { .. }) && v.contains("cursor_trail 3")));
    }

    /// Papirus seçiliyken klasör ikonları paletle (look::folders): gizli tema look.icons'ta,
    /// yazılan ikon teması onun adı, canlı ikon yenileme; başka ikon temasında hiçbiri.
    #[test]
    fn folder_icons_follow_palette() {
        use crate::services::look::folders::FolderSet;
        let set = FolderSet { base: "Papirus-Dark".into(), icons: vec![("48/places", "folder.svg".into(), "<svg style=\"fill:#5294e2\"/>".into())] };
        let p = plan_for(&unified(), &FakeProbe { folders: Some(set), ..FakeProbe::full() });
        let icons = p.changes.iter().find(|c| c.setting == setting::ICONS).unwrap();
        let file = |end: &str| icons.writes.iter().find_map(|(f, v)| matches!(f, Fragment::ManagedFile { file } if file.ends_with(end)).then(|| v.clone()));
        let svg = file("icons/Papirus-Dark-Nuxut/48/places/folder.svg").expect("klasör ikonu");
        assert!(svg.starts_with("<!-- Nuxut — ") && !svg.contains("#5294e2"), "{svg}");
        assert!(file("icons/Papirus-Dark-Nuxut/index.theme").unwrap().contains("Inherits=Papirus-Dark,hicolor"));
        assert!(file("icons/Papirus-Dark-Nuxut-reload/index.theme").unwrap().contains("Inherits=Papirus-Dark-Nuxut\n"));
        assert!(icons.writes.iter().any(|(f, v)| matches!(f, Fragment::IniKey { key, .. } if key == "gtk-icon-theme-name") && v == "Papirus-Dark-Nuxut"));
        assert!(icons.writes.iter().any(|(f, v)| matches!(f, Fragment::Dconf { key } if key.ends_with("/icon-theme")) && v == "'Papirus-Dark-Nuxut'"));
        assert!(p.live.contains(&LiveAction::IconReload { twin: "Papirus-Dark-Nuxut-reload".into(), name: "Papirus-Dark-Nuxut".into() }));
        let plain = plan_for(&unified(), &FakeProbe::full());
        assert!(!plain.live.iter().any(|a| matches!(a, LiveAction::IconReload { .. })));
    }

    /// Nuxut Qt stili kuruluysa (ADR-0009): kdeglobals `widgetStyle=Nuxut`, token dosyası,
    /// `QT_PLUGIN_PATH`; değilse Breeze ve hiçbiri.
    #[test]
    fn nuxut_qt_style_when_installed() {
        let style = |p: &LookPlan| {
            p.changes.iter().flat_map(|c| &c.writes).find_map(|(f, v)| match f {
                Fragment::IniKey { key, .. } if key == "widgetStyle" => Some(v.clone()),
                _ => None,
            })
        };
        let tokens = |p: &LookPlan| p.changes.iter().flat_map(|c| &c.writes).any(|(f, _)| matches!(f, Fragment::ManagedFile { file } if file.ends_with("nuxut/qt-style.conf")));
        let plain = plan_for(&unified(), &FakeProbe::full());
        assert_eq!(style(&plain).as_deref(), Some("Breeze"));
        assert!(!tokens(&plain) && !plain.env.iter().any(|(k, _)| *k == "QT_PLUGIN_PATH"));
        let p = plan_for(&unified(), &FakeProbe { qt_lib: Some("/opt/n/lib/nuxut".into()), ..FakeProbe::full() });
        assert_eq!(style(&p).as_deref(), Some("Nuxut"));
        assert!(tokens(&p));
        assert!(p.env.contains(&("QT_PLUGIN_PATH", "/opt/n/lib/nuxut/qt6:/opt/n/lib/nuxut/qt5".to_owned())));
    }

    /// Kullanıcının makinesi (2026-10-16): `gtk-4.0/gtk.css` ve `gtk-dark.css` bir temaya bağ →
    /// Nuxut'un yönetilen dosyası (yalnızca `nuxut.css` içe aktarımı); bağ olmayan `gtk-3.0`'da
    /// olağan satır.
    #[test]
    fn linked_theme_css_is_replaced() {
        let theme = nuxut::Theme::default();
        let r = resolve(&unified(), Mode::Dark, &FakeProbe::full());
        let d = dirs();
        let links = [d.config.join("gtk-4.0/gtk.css"), d.config.join("gtk-4.0/gtk-dark.css")];
        let p = build(
            &LookInput {
                colors: &theme.colors,
                generated: None,
                color_theme: None,
                typography: &theme.typography,
                shape: &theme.shape,
                contrast: theme.contrast,
                reduced_motion: false,
                selection: &unified(),
                resolved: &r,
                gtk_base: None,
                css_links: &links,
                folders: None,
            },
            &d,
        );
        let mode = p.changes.iter().find(|c| c.setting == setting::MODE).unwrap();
        for l in &links {
            let (_, v) = mode.writes.iter().find(|(f, _)| matches!(f, Fragment::ManagedFile { file } if file == l)).expect("yönetilen dosya");
            assert!(v.starts_with("/* Nuxut — ") && v.ends_with(&format!("{}\n", gtk::GTK_IMPORT)), "{v}");
            assert!(!mode.writes.iter().any(|(f, _)| matches!(f, Fragment::Line { file, .. } if file == l)), "aynı dosyaya satır yok");
        }
        assert!(mode.writes.iter().any(|(f, _)| matches!(f, Fragment::Line { file, .. } if file.ends_with("gtk-3.0/gtk.css"))));
    }

    #[test]
    fn disabled_writes_nothing() {
        let p = plan_for(&Desktop { unified: false, ..Default::default() }, &FakeProbe::full());
        assert!(p.is_empty());
        assert!(p.live.is_empty());
    }

    #[test]
    fn kde_path_targets() {
        let p = plan_for(&unified(), &FakeProbe::full());
        let t: HashSet<String> = targets(&p).into_iter().collect();
        for want in [
            "~/.config/gtk-4.0/nuxut.css",
            "~/.config/gtk-4.0/gtk.css",
            "~/.config/gtk-3.0/nuxut.css",
            "~/.config/gtk-3.0/gtk.css",
            "~/.config/gtk-3.0/settings.ini",
            "~/.config/gtk-4.0/settings.ini",
            "~/.config/kdeglobals",
            "~/.local/share/color-schemes/Nuxut.colors",
            "~/.icons/default/index.theme",
            "~/.config/kcminputrc",
            "~/.config/environment.d/99-nuxut-look.conf",
            "/org/gnome/desktop/interface/color-scheme",
            "/org/gnome/desktop/interface/icon-theme",
        ] {
            assert!(t.contains(want), "{want} yok: {t:?}");
        }
        assert!(!t.iter().any(|x| x.contains("qt6ct")), "kde yolunda qt6ct yazılmaz");
        let settings: Vec<&str> = p.changes.iter().map(|c| c.setting).collect();
        assert_eq!(settings, [setting::MODE, setting::ICONS, setting::CURSOR, setting::FONTS, setting::MOTION, setting::UNIFIED]);
        assert_eq!(p.restart_needed, [Restart::Gtk3Colors, Restart::Gtk4Colors, Restart::XCursor]);
        let live: Vec<&str> = p.live.iter().map(LiveAction::name).collect();
        assert_eq!(live, ["kde_palette", "kde_fonts", "kde_style", "kde_icons", "hypr_setcursor", "session_env", "session_unset"]);
    }

    #[test]
    fn settings_are_classified() {
        let p = plan_for(&unified(), &FakeProbe::full());
        let entries = summarize(&p, &dirs());
        let find = |key: &str| entries.iter().find(|e| e.key.as_deref() == Some(key) || e.target.ends_with(key)).unwrap().setting;
        assert_eq!(find("gtk-icon-theme-name"), setting::ICONS);
        assert_eq!(find("Theme"), setting::ICONS);
        assert_eq!(find("cursorSize"), setting::CURSOR);
        assert_eq!(find("Inherits"), setting::CURSOR);
        assert_eq!(find("/cursor-size"), setting::CURSOR);
        assert_eq!(find("fixed"), setting::FONTS);
        assert_eq!(find("/monospace-font-name"), setting::FONTS);
        assert_eq!(find("/color-scheme"), setting::MODE);
        assert_eq!(find("widgetStyle"), setting::MODE);
        assert_eq!(find("BackgroundNormal"), setting::MODE);
        // gtk-interface-color-scheme yalnızca GTK 4'e, kipe bağlı (look.mode).
        let ics: Vec<_> = entries.iter().filter(|e| e.key.as_deref() == Some("gtk-interface-color-scheme")).collect();
        assert_eq!(ics.len(), 1, "{ics:?}");
        assert!(ics[0].target.ends_with("gtk-4.0/settings.ini"));
        assert_eq!((ics[0].setting, ics[0].value.as_deref()), (setting::MODE, Some("dark")));
        // nuxut-apply: bir Change içinde parça yinelenmez.
        for c in &p.changes {
            let mut seen = HashSet::new();
            for (f, _) in &c.writes {
                assert!(seen.insert(format!("{f:?}")), "yinelenen parça: {f:?}");
            }
        }
    }

    #[test]
    fn qtct_fallback_targets() {
        let sel = Desktop { qt_backend: QtBackend::Qt6ct, ..unified() };
        let p = plan_for(&sel, &FakeProbe::full());
        let entries = summarize(&p, &dirs());
        let path = entries
            .iter()
            .find(|e| e.target == "~/.config/qt6ct/qt6ct.conf" && e.key.as_deref() == Some("color_scheme_path"))
            .unwrap();
        assert_eq!(path.value.as_deref(), Some("/home/u/.config/qt6ct/colors/nuxut.conf"));
        assert_eq!(path.setting, setting::QT_BACKEND);
        assert!(entries.iter().any(|e| e.target == "~/.config/qt5ct/colors/nuxut.conf"));
        assert!(p.restart_needed.contains(&Restart::QtColors));
        let env: Vec<_> = p.env.iter().filter(|(k, _)| *k == "QT_QPA_PLATFORMTHEME").collect();
        assert_eq!(env[0].1, "qt6ct");
    }

    #[test]
    fn managed_files_carry_header() {
        let p = plan_for(&Desktop { qt_backend: QtBackend::Qt6ct, ..unified() }, &FakeProbe::full());
        for c in &p.changes {
            for (f, v) in &c.writes {
                if let Fragment::ManagedFile { .. } = f {
                    let first = v.lines().next().unwrap();
                    assert!(first.starts_with("/* Nuxut — ") || first.starts_with("# Nuxut — "), "{f:?}: {first}");
                }
            }
        }
    }

    #[test]
    fn values_are_single_line_and_deterministic() {
        let a = plan_for(&unified(), &FakeProbe::full());
        assert_eq!(a, plan_for(&unified(), &FakeProbe::full()));
        for c in &a.changes {
            for (f, v) in &c.writes {
                if !matches!(f, Fragment::ManagedFile { .. }) {
                    assert!(!v.chars().any(char::is_control), "{f:?}");
                }
            }
        }
    }

    #[test]
    fn mode_switch_changes_gtk3_and_scheme_only_in_mode_setting() {
        let dark = plan_for(&unified(), &FakeProbe::full());
        let light = plan_in(&unified(), Mode::Light, &FakeProbe::full());
        let by = |p: &LookPlan, s: &str| p.changes.iter().find(|c| c.setting == s).cloned().unwrap();
        assert_ne!(by(&dark, setting::MODE), by(&light, setting::MODE));
        // İkon ailesi Papirus: varyant kipe göre değişir.
        assert_ne!(by(&dark, setting::ICONS), by(&light, setting::ICONS));
        assert_eq!(by(&dark, setting::CURSOR), by(&light, setting::CURSOR));
        assert_eq!(by(&dark, setting::FONTS), by(&light, setting::FONTS));
    }

    // -----------------------------------------------------------------------------------------
    // Erişilebilirlik (framework docs/design/accessibility.md §1–3, t124).

    /// `framework.toml` metninden tema; seçim ve kip ondan bağımsız (birleşik, koyu).
    fn plan_from(text: &str, sel: &Desktop) -> LookPlan {
        let theme = nuxut::Theme::parse(text).unwrap().0;
        let r = resolve(sel, Mode::Dark, &FakeProbe::full());
        build(
            &LookInput {
                colors: &theme.colors,
                generated: None,
                color_theme: None,
                typography: &theme.typography,
                shape: &theme.shape,
                contrast: theme.contrast,
                reduced_motion: theme.reduced_motion(),
                selection: sel,
                resolved: &r,
                gtk_base: None,
                css_links: &[],
                folders: None,
            },
            &dirs(),
        )
    }

    fn entry<'a>(e: &'a [PlanEntry], target_end: &str, key: Option<&str>) -> &'a PlanEntry {
        e.iter()
            .find(|x| x.target.ends_with(target_end) && (key.is_none() || x.key.as_deref() == key))
            .unwrap_or_else(|| panic!("{target_end} {key:?} yok"))
    }

    /// Azaltılmış hareket → dconf `enable-animations` ve iki `settings.ini`'de
    /// `gtk-enable-animations`, hepsi `look.motion` ayarında.
    #[test]
    fn reduced_motion_writes_both_keys() {
        for (text, dconf, ini) in [("", "true", "1"), ("[motion]\npreset = \"instant\"\n", "false", "0"), ("[motion]\nenabled = false\n", "false", "0")] {
            let p = plan_from(text, &unified());
            let e = summarize(&p, &dirs());
            let d = entry(&e, "/enable-animations", None);
            assert_eq!((d.setting, d.value.as_deref()), (setting::MOTION, Some(dconf)), "{text}");
            for file in ["gtk-3.0/settings.ini", "gtk-4.0/settings.ini"] {
                let k = entry(&e, file, Some("gtk-enable-animations"));
                assert_eq!((k.setting, k.value.as_deref()), (setting::MOTION, Some(ini)), "{text} {file}");
            }
            let motion = p.changes.iter().find(|c| c.setting == setting::MOTION).unwrap();
            assert_eq!(motion.writes.len(), 3, "{text}");
        }
        // Hareket diğer ayarları etkilemez.
        let on = plan_from("", &unified());
        let off = plan_from("[motion]\nenabled = false\n", &unified());
        for s in [setting::MODE, setting::FONTS, setting::UNIFIED] {
            let by = |p: &LookPlan| p.changes.iter().find(|c| c.setting == s).cloned();
            assert_eq!(by(&on), by(&off), "{s}");
        }
    }

    /// Metin ölçeği her hedefe TAM BİR kez: GTK adı referans boyda + `text-scaling-factor` /
    /// `gtk-xft-dpi`; kdeglobals ölçekli pt; `QT_FONT_DPI` yalnız qt6ct yolunda; `QT_SCALE_FACTOR` asla.
    #[test]
    fn text_scale_enters_each_target_once() {
        let text = "[typography]\nsize = 16\n";
        let p = plan_from(text, &unified());
        let e = summarize(&p, &dirs());
        let font = entry(&e, "/font-name", None).value.clone().unwrap();
        assert!(font.ends_with(" 10'"), "GTK adı referans boyda: {font}");
        assert_eq!(entry(&e, "/text-scaling-factor", None).value.as_deref(), Some("1.23"));
        assert_eq!(entry(&e, "/text-scaling-factor", None).setting, setting::FONTS);
        for file in ["gtk-3.0/settings.ini", "gtk-4.0/settings.ini"] {
            assert!(entry(&e, file, Some("gtk-font-name")).value.as_deref().unwrap().ends_with(" 10"), "{file}");
            let dpi = entry(&e, file, Some("gtk-xft-dpi"));
            assert_eq!((dpi.setting, dpi.value.as_deref()), (setting::FONTS, Some("120914")), "{file}");
        }
        assert!(entry(&e, "kdeglobals", Some("font")).value.as_deref().unwrap().contains(",12,"), "kdeglobals ölçekli");
        // kde yolu: QT_FONT_DPI yok, Nuxut'un önceki değeri oturumdan kalkar.
        assert!(p.env.iter().all(|(k, _)| *k != "QT_FONT_DPI" && *k != "QT_SCALE_FACTOR"), "{:?}", p.env);
        assert!(p.live.contains(&LiveAction::SessionUnset(vec!["QT_FONT_DPI"])));
        // qt6ct yolu: QT_FONT_DPI = round(96·1.23) ortamda ve dosyada; kaldırma yok.
        let q = plan_from(text, &Desktop { qt_backend: QtBackend::Qt6ct, ..unified() });
        assert!(q.env.contains(&("QT_FONT_DPI", "118".into())), "{:?}", q.env);
        assert!(!q.live.iter().any(|a| matches!(a, LiveAction::SessionUnset(_))));
        let envd = q.changes.iter().find(|c| c.setting == setting::UNIFIED).unwrap();
        assert!(envd.writes[0].1.contains("\nQT_FONT_DPI=118\n"));
        assert!(!envd.writes[0].1.contains("QT_SCALE_FACTOR"));
        // Ölçek 1 (varsayılan): qt6ct yolunda da QT_FONT_DPI yok, GTK çarpanı 1.00.
        let d = plan_from("", &Desktop { qt_backend: QtBackend::Qt6ct, ..unified() });
        assert!(d.env.iter().all(|(k, _)| *k != "QT_FONT_DPI"));
        assert_eq!(entry(&summarize(&d, &dirs()), "/text-scaling-factor", None).value.as_deref(), Some("1.0"));
    }

    /// Yüksek karşıtlık: yalnızca look.mode (renkler) değişir.
    #[test]
    fn high_contrast_changes_only_colors() {
        let std = plan_from("", &unified());
        let high = plan_from("[colors]\ncontrast = \"high\"\n", &unified());
        let by = |p: &LookPlan, s: &str| p.changes.iter().find(|c| c.setting == s).cloned();
        assert_ne!(by(&std, setting::MODE), by(&high, setting::MODE));
        for s in [setting::ICONS, setting::CURSOR, setting::FONTS, setting::MOTION, setting::UNIFIED] {
            assert_eq!(by(&std, s), by(&high, s), "{s}");
        }
    }
}
