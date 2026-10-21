//! `shell.toml` şeması, varsayılanlar ve doğrulama.
//!
//! Yükleme: kısmi dosya geçerli, bilinmeyen anahtar uyarı. Dosya yalnızca **sürüm göçünde**
//! yazılır (ADR-0002, [`crate::config_migrate`]): eski biçim (`bar.left`, `[clock]`, …) çalışma
//! anında değil dosyada, bir kez ve yorumlar korunarak taşınır. Ardından
//! [`ShellConfig::validate`] değerleri güvenli aralıklara sınırlar ve widget ayarlarını kayıttan
//! ([`crate::widgets::registry`]) çözer; bozuk değer çalışmayı durdurmaz, uyarıyla düzeltilir.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use nuxut::i18n::t;
use nuxut_core::Insets;
use serde::Deserialize;

use crate::config_heal::{self, HealReport, HealWritten};
use crate::config_migrate::{
    self, ApplyConfigWriter, CURRENT_CONFIG_VERSION, ConfigWriter, Migrated, Migration, MigrationRecord, WriteLabel,
    WritePolicy,
};
use crate::layout::Edge;
use crate::schema::SettingSchema;
use crate::services::audio::AudioBackend;
use crate::services::hypr_config::input::InputConfig;
use crate::services::hypr_config::settings::{CompositorConfig, KeybindOwnerConfig, KeybindsConfig, WindowsConfig};
use crate::services::notifications::NotificationsConfig;
use crate::widgets::{WidgetKind, WidgetSettings};

/// Config dosyasının adı (`<config_dir>/shell.toml`).
pub const CONFIG_NAME: &str = "shell";

/// Çerçeve kenar kalınlığı üst sınırı (mantıksal piksel).
pub const MAX_FRAME_THICKNESS: f32 = 64.0;
pub const MIN_BAR_SIZE: f32 = 16.0;
pub const MAX_BAR_SIZE: f32 = 128.0;
pub const MAX_BAR_PADDING: f32 = 64.0;
pub const MAX_BAR_SPACING: f32 = 64.0;
pub const MIN_PANEL_SIZE: f32 = 160.0;
pub const MAX_PANEL_SIZE: f32 = 900.0;
/// Bilinmeyen widget uyarısı en fazla bu kadar ad listeler (dev listeler uyarı yağdırmaz).
const MAX_REPORTED_UNKNOWN: usize = 4;

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct ShellConfig {
    pub frame: FrameConfig,
    pub bar: BarConfig,
    pub panels: PanelsConfig,
    /// `[notifications]`: şema ve doğrulama bildirim modülünde.
    pub notifications: NotificationsConfig,
    pub wallpaper: WallpaperConfig,
    pub audio: AudioConfig,
    pub media: MediaConfig,
    pub osd: OsdConfig,
    /// `[launcher]`: şema ve doğrulama başlatıcı servisinde.
    pub launcher: crate::services::launcher::LauncherConfig,
    /// `[compositor]`, `[windows]`, `[keybinds]`: masaüstü (Hyprland) ortaklığı (`services::hypr_config`).
    pub compositor: CompositorConfig,
    pub windows: WindowsConfig,
    pub keybinds: KeybindsConfig,
    /// `[keybind_owner]`: çakışan tuşta kimin bind'i çalışsın (Ayarlar › Kısayollar).
    pub keybind_owner: KeybindOwnerConfig,
    /// `[input]`: klavye, fare, dokunmatik yüzey (yalnızca yazılı anahtarlar Hyprland'e gider).
    pub input: InputConfig,
    /// Dosya sürümü (ADR-0002). Göç yüklemeden önce okur ve yazar; burada yalnızca bilinmeyen
    /// anahtar uyarısı vermesin diye kabul edilir, doğrulamada atılır.
    #[serde(rename = "config_version")]
    file_version: Option<toml::Value>,
    /// `[displays]`: Ekranlar davranışı (ADR-0005); monitör yerleşimleri ayrı `displays.toml`'da.
    pub displays: crate::services::displays::DisplaysConfig,
    /// `[setup]`: ilk açılış / Hoş geldin (ADR-0001 §10, `services::setup`).
    pub setup: crate::services::setup::SetupConfig,
    /// `[sysmon]`: sistem izleyici örneklemesi (`services::sysmon`).
    pub sysmon: crate::services::sysmon::SysmonConfig,
    /// `[idle]`: boşta kalma (karart, ekranı kapat, kilitle, uyu; `services::idle`).
    pub idle: crate::services::idle::IdleConfig,
    /// `[lock]`: kilit ekranı tetikleyicisi (`services::lock_launcher`).
    pub lock: crate::services::lock_launcher::LockConfig,
    /// `[screenshot]`: ekran görüntüsü (`services::screenshot`).
    pub screenshot: crate::services::screenshot::ScreenshotConfig,
    /// `[clipboard]`: pano geçmişi (`services::clipboard`).
    pub clipboard: crate::services::clipboard::ClipboardConfig,
    /// `[night_light]`: gece ışığı (`services::night_light`, t113).
    pub night_light: crate::services::night_light::NightLightConfig,
    /// `[removable_media]`: çıkarılabilir medya bildirimi (`services::removable_media`).
    pub removable_media: crate::services::removable_media::RemovableMediaConfig,
    /// `[magnifier]`: büyüteç (`services::magnifier`, t124).
    pub magnifier: crate::services::magnifier::MagnifierConfig,
    /// `[osk]`: ekran klavyesi (`services::osk`, t125).
    pub osk: crate::services::osk::OskConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct FrameConfig {
    pub enabled: bool,
    /// Tüm kenarların kalınlığı.
    pub thickness: f32,
    /// Kenar başına geçersiz kılma. Bar kenarında yok sayılır (o kenar = `bar.size`).
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self { enabled: true, thickness: 6.0, top: None, right: None, bottom: None, left: None }
    }
}

impl FrameConfig {
    fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(SettingSchema::bool("frame.enabled", t!("shell-config-frame-enabled-label"), d.enabled));
        out.push(SettingSchema::float("frame.thickness", t!("shell-config-frame-thickness-label"), d.thickness, 0.0, MAX_FRAME_THICKNESS).unit("px"));
    }
}

/// Bar bölümleri. Yatay bar'da start = sol, end = sağ; dikey bar'da start = üst, end = alt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarSection {
    Start,
    Center,
    End,
}

impl BarSection {
    pub const ALL: [BarSection; 3] = [BarSection::Start, BarSection::Center, BarSection::End];
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct BarConfig {
    pub edge: Edge,
    /// Bar kalınlığı = bar kenarındaki çerçeve kalınlığı.
    pub size: f32,
    /// Bar içeriğinin uçlardaki iç boşluğu (bar boyunca).
    pub padding: f32,
    /// Widget'lar arası boşluk.
    pub spacing: f32,
    pub start: Vec<String>,
    pub center: Vec<String>,
    pub end: Vec<String>,
    /// Uç widget eylemleri (`[bar.edge_actions]`, `docs/design/bar-edge-actions.md`).
    pub edge_actions: crate::bar_edge::EdgeActionsConfig,
    /// `[bar.widget.<ad>]` ham tabloları; doğrulamada `widgets`'a çözülür.
    #[serde(rename = "widget")]
    widget_tables: BTreeMap<String, toml::Table>,
    /// Çözülmüş widget ayarları.
    #[serde(skip)]
    pub widgets: WidgetSettings,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            edge: Edge::Top,
            size: 34.0,
            padding: 10.0,
            spacing: 12.0,
            start: vec!["logo".into(), "workspaces".into()],
            center: vec!["clock".into(), "next_event".into(), "now_playing".into()],
            end: vec![
                "privacy".into(),
                "magnifier".into(),
                "keyboard_layout".into(),
                "sysmon".into(),
                "tray".into(),
                "status".into(),
                "notifications".into(),
                "session".into(),
            ],
            edge_actions: crate::bar_edge::EdgeActionsConfig::default(),
            widget_tables: BTreeMap::new(),
            widgets: WidgetSettings::default(),
        }
    }
}

/// `bar.edge` seçeneklerinin görünen adları (`Edge` `src/layout.rs`'te tanımlı, görünen ad burada).
fn edge_label(e: Edge) -> String {
    match e {
        Edge::Top => t!("shell-schema-bar-edge-top"),
        Edge::Right => t!("shell-schema-bar-edge-right"),
        Edge::Bottom => t!("shell-schema-bar-edge-bottom"),
        Edge::Left => t!("shell-schema-bar-edge-left"),
    }
}

impl BarConfig {
    fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        let edges = Edge::ALL.map(Edge::name).to_vec();
        out.push(SettingSchema::choice("bar.edge", t!("shell-config-bar-edge-label"), d.edge.name(), edges).labels(Edge::ALL.map(edge_label).to_vec()));
        out.push(SettingSchema::float("bar.size", t!("shell-config-bar-size-label"), d.size, MIN_BAR_SIZE, MAX_BAR_SIZE).unit("px"));
        out.push(SettingSchema::float("bar.padding", t!("shell-config-bar-padding-label"), d.padding, 0.0, MAX_BAR_PADDING).unit("px"));
        out.push(SettingSchema::float("bar.spacing", t!("shell-config-bar-spacing-label"), d.spacing, 0.0, MAX_BAR_SPACING).unit("px"));
        crate::bar_edge::EdgeActionsConfig::schema(out);
    }

    pub fn section(&self, section: BarSection) -> &[String] {
        match section {
            BarSection::Start => &self.start,
            BarSection::Center => &self.center,
            BarSection::End => &self.end,
        }
    }

    /// Bir bölümün widget'ları. Doğrulanmış config'de bilinmeyen ad kalmaz.
    pub fn widgets_in(&self, section: BarSection) -> Vec<WidgetKind> {
        self.section(section).iter().filter_map(|n| WidgetKind::from_name(n)).collect()
    }

    /// Bar dikey mi (sol/sağ kenarda)?
    pub fn is_vertical(&self) -> bool {
        !self.edge.is_horizontal()
    }
}

/// Bir kenar paneli (çekmece): çerçeve kenarından tam boy büyür.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct EdgePanelConfig {
    pub enabled: bool,
    /// Kenardan içeri derinlik (sol/sağ: genişlik, üst/alt: yükseklik).
    pub size: f32,
    pub widgets: Vec<String>,
}

impl EdgePanelConfig {
    fn new(enabled: bool, size: f32, widgets: &[&str]) -> Self {
        Self { enabled, size, widgets: widgets.iter().map(|&w| w.to_owned()).collect() }
    }

    pub fn widget_kinds(&self) -> Vec<WidgetKind> {
        self.widgets.iter().filter_map(|n| WidgetKind::from_name(n)).collect()
    }
}

impl Default for EdgePanelConfig {
    fn default() -> Self {
        Self::new(false, 340.0, &[])
    }
}

/// Kenar panelleri. Kısmi tablo o kenarın kendi varsayılanının üzerine yazılır
/// (`[panels.right] size = 400` sağ panelin `enabled`/`widgets` varsayılanını korur).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "PanelsPatch")]
pub struct PanelsConfig {
    pub left: EdgePanelConfig,
    pub right: EdgePanelConfig,
    pub top: EdgePanelConfig,
    pub bottom: EdgePanelConfig,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct EdgePanelPatch {
    enabled: Option<bool>,
    size: Option<f32>,
    widgets: Option<Vec<String>>,
}

impl EdgePanelPatch {
    fn apply(self, base: &mut EdgePanelConfig) {
        base.enabled = self.enabled.unwrap_or(base.enabled);
        base.size = self.size.unwrap_or(base.size);
        if let Some(w) = self.widgets {
            base.widgets = w;
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PanelsPatch {
    left: EdgePanelPatch,
    right: EdgePanelPatch,
    top: EdgePanelPatch,
    bottom: EdgePanelPatch,
}

impl From<PanelsPatch> for PanelsConfig {
    fn from(p: PanelsPatch) -> Self {
        let mut c = Self::default();
        p.left.apply(&mut c.left);
        p.right.apply(&mut c.right);
        p.top.apply(&mut c.top);
        p.bottom.apply(&mut c.bottom);
        c
    }
}

impl Default for PanelsConfig {
    fn default() -> Self {
        Self {
            left: EdgePanelConfig::default(),
            right: EdgePanelConfig::new(
                true,
                380.0,
                &["session", "toggles", "volume", "mixer", "brightness", "network", "bluetooth", "power", "media", "notifications"],
            ),
            top: EdgePanelConfig::new(false, 320.0, &[]),
            bottom: EdgePanelConfig::new(false, 320.0, &[]),
        }
    }
}

impl PanelsConfig {
    fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        for edge in Edge::ALL {
            let (p, name) = (d.get(edge), edge.name());
            out.push(SettingSchema::bool(format!("panels.{name}.enabled"), t!("shell-config-panels-enabled-label"), p.enabled));
            out.push(
                SettingSchema::float(format!("panels.{name}.size"), t!("shell-config-panels-size-label"), p.size, MIN_PANEL_SIZE, MAX_PANEL_SIZE)
                    .step(10.0)
                    .unit("px"),
            );
        }
    }

    pub fn get(&self, edge: Edge) -> &EdgePanelConfig {
        match edge {
            Edge::Left => &self.left,
            Edge::Right => &self.right,
            Edge::Top => &self.top,
            Edge::Bottom => &self.bottom,
        }
    }

    /// Oturum menüsü (`msg session menu`, CTRL+ALT+Delete): oturum kartının bulunduğu ilk açık kenar
    /// paneli (sağ, sol, üst, alt sırasıyla); hiçbirinde yoksa sağ panel.
    pub fn session_edge(&self) -> Edge {
        [Edge::Right, Edge::Left, Edge::Top, Edge::Bottom]
            .into_iter()
            .find(|&e| self.get(e).enabled && self.get(e).widget_kinds().contains(&WidgetKind::Session))
            .unwrap_or(Edge::Right)
    }

    fn get_mut(&mut self, edge: Edge) -> &mut EdgePanelConfig {
        match edge {
            Edge::Left => &mut self.left,
            Edge::Right => &mut self.right,
            Edge::Top => &mut self.top,
            Edge::Bottom => &mut self.bottom,
        }
    }
}

/// `[media]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct MediaConfig {
    /// Oynatıcının verdiği ağ kapağını (`https://`) indir. Gizlilik: indirme, çalınan parçayı
    /// kapağın sunucusuna bildirir; `false` iken hiç istek yapılmaz, yer tutucu gösterilir.
    pub remote_art: bool,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self { remote_art: true }
    }
}

impl MediaConfig {
    fn schema(out: &mut Vec<SettingSchema>) {
        out.push(SettingSchema::bool("media.remote_art", t!("shell-config-media-remote-art-label"), Self::default().remote_art));
    }
}

/// `[audio]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Ses seviyesi üst sınırı, yüzde (100..=150; 100'ün üstü yükseltme).
    pub max_volume: u32,
    /// Seviye/sessiz yazım yolu (geçici; yerli yol doğrulanınca kalkacak): `wpctl` alt süreci
    /// ya da süreç içi `native` (WirePlumber mixer-api ile aynı anlam).
    pub backend: AudioBackend,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self { max_volume: 100, backend: AudioBackend::default() }
    }
}

/// `audio.backend` seçeneklerinin görünen adları (`AudioBackend` `services/audio/mod.rs`'te tanımlı).
fn audio_backend_label(b: AudioBackend) -> String {
    match b {
        AudioBackend::Wpctl => t!("shell-schema-audio-backend-wpctl"),
        AudioBackend::Native => t!("shell-schema-audio-backend-native"),
    }
}

impl AudioConfig {
    fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        let max = i64::from(crate::system::MAX_VOLUME_PERCENT);
        out.push(SettingSchema::int("audio.max_volume", t!("shell-config-audio-max-volume-label"), d.max_volume.into(), 100, max).unit("%"));
        out.push(
            SettingSchema::choice(
                "audio.backend",
                t!("shell-config-audio-backend-label"),
                d.backend.name(),
                AudioBackend::ALL.map(AudioBackend::name).to_vec(),
            )
            .labels(AudioBackend::ALL.map(audio_backend_label).to_vec()),
        );
    }

    fn validate(&mut self, w: &mut Vec<String>) {
        let max = u32::from(crate::system::MAX_VOLUME_PERCENT);
        if !(100..=max).contains(&self.max_volume) {
            let fixed = self.max_volume.clamp(100, max);
            w.push(format!("audio.max_volume = {} aralık dışı, {fixed} kullanılıyor (100..={max})", self.max_volume));
            self.max_volume = fixed;
        }
    }
}

/// Duvar kağıdı yerleşimi (`nuxut-image` `Wallpaper` bileşeninin desteklediği kipler).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperFit {
    /// Ekranı doldurur, taşan kırpılır.
    #[default]
    Cover,
    /// Tamamı görünür, kenarlarda boşluk kalabilir.
    Contain,
    /// Oranı bozarak doldurur.
    Fill,
}

/// Monitör başına geçersiz kılma (`[wallpaper.outputs."<bağlayıcı>"]`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct WallpaperOutput {
    pub path: Option<String>,
    pub fit: Option<WallpaperFit>,
}

/// Letterbox zemini: görselin kaplamadığı alan (`contain` kenarları; görsel çözülmeden önce
/// ve saydam piksellerin altı da).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperBackdropKind {
    /// Aynı görselin ekranı kaplayan bulanık kopyası. Yalnızca `contain`'de üretilir; diğer
    /// kiplerde, kopya hazır olana dek ve görsel yokken `backdrop_color` boyanır.
    #[default]
    Blur,
    /// Düz tema rengi (`backdrop_color`).
    Color,
}

/// Zeminin tema rengi (sabit renk yok: tema değişince izler; her zaman opak boyanır).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperBackdropColor {
    #[default]
    Surface,
    SurfaceContainer,
    SurfaceBright,
}

/// Letterbox zemini ayarları (tüm monitörlerde ortak).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallpaperBackdropSpec {
    pub kind: WallpaperBackdropKind,
    /// Bulanıklık yarıçapı (mantıksal px, [`WALLPAPER_BACKDROP_BLUR`] içinde).
    pub blur: u32,
    pub color: WallpaperBackdropColor,
}

/// Duvar kağıdının kaynağı: kullanıcı dosyası ya da yerleşik Nuxut duvar kağıdı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WallpaperSource {
    /// Görsel dosyası (yalıtılmış işçide çözülür).
    File(std::path::PathBuf),
    /// `nuxut:<id>[/dark|light]` (gömülü SVG; `docs/design/wallpapers.md`).
    Builtin(nuxut_wallpapers::Selection),
}

impl WallpaperSource {
    /// `[wallpaper] path` dizgesi → kaynak. Boş → `None`. Bilinmeyen yerleşik kimlik burada
    /// varsayılana düşer (uyarı [`WallpaperConfig::validate`]'te bir kez verilir).
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }
        Some(match nuxut_wallpapers::parse_source(s) {
            nuxut_wallpapers::Parsed::NotBuiltin => Self::File(expand_home(s)),
            nuxut_wallpapers::Parsed::Builtin(sel) | nuxut_wallpapers::Parsed::Invalid { fallback: sel, .. } => Self::Builtin(sel),
        })
    }
}

/// Bir monitörün çözülmüş duvar kağıdı isteği ([`WallpaperConfig::for_output`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallpaperWanted {
    pub source: WallpaperSource,
    pub fit: WallpaperFit,
    pub backdrop: WallpaperBackdropSpec,
}

/// `[wallpaper]`. `path` boşsa shell duvar kağıdı çizmez (başka bir araç — hyprpaper vb. —
/// kullanılıyor olabilir); yüzey de yaratılmaz. Varsayılan (anahtar yazılmamış, taze kurulum)
/// yerleşik [`nuxut_wallpapers::DEFAULT_SOURCE`] (kullanıcı kararı, 2026-10-13).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct WallpaperConfig {
    /// Görsel dosyası (yalıtılmış işçide çözülür; framework "Duvar kağıdı yalıtımı"); animasyonlu kaynakta
    /// ilk kare durağan gösterilir. `~` ev dizinine açılır. `nuxut:<id>` (tema kipini izler) ya da
    /// `nuxut:<id>/dark|light` yerleşik duvar kağıdıdır.
    pub path: String,
    pub fit: WallpaperFit,
    /// Letterbox zemini (varsayılan: bulanık kopya — siyah/düz bant yerine).
    pub backdrop: WallpaperBackdropKind,
    /// Zemin bulanıklığının yarıçapı (mantıksal px).
    pub backdrop_blur: u32,
    /// Düz zeminin ve bulanık kopya hazır olana dek boyanan yedeğin tema rengi.
    pub backdrop_color: WallpaperBackdropColor,
    pub outputs: BTreeMap<String, WallpaperOutput>,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            path: nuxut_wallpapers::DEFAULT_SOURCE.to_owned(),
            fit: WallpaperFit::default(),
            backdrop: WallpaperBackdropKind::default(),
            backdrop_blur: 48,
            backdrop_color: WallpaperBackdropColor::default(),
            outputs: BTreeMap::new(),
        }
    }
}

/// `[osd]`: ses/parlaklık göstergesi.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct OsdConfig {
    /// Son değişimden sonra görünme süresi (ms).
    pub timeout_ms: u32,
}

impl Default for OsdConfig {
    fn default() -> Self {
        Self { timeout_ms: 1200 }
    }
}

const OSD_TIMEOUT_MS: std::ops::RangeInclusive<u32> = 500..=10_000;

impl OsdConfig {
    fn schema(out: &mut Vec<SettingSchema>) {
        let (min, max) = (i64::from(*OSD_TIMEOUT_MS.start()), i64::from(*OSD_TIMEOUT_MS.end()));
        out.push(SettingSchema::int("osd.timeout_ms", t!("shell-config-osd-timeout-label"), Self::default().timeout_ms.into(), min, max).unit("ms"));
    }

    fn validate(&mut self, w: &mut Vec<String>) {
        if !OSD_TIMEOUT_MS.contains(&self.timeout_ms) {
            let fixed = self.timeout_ms.clamp(*OSD_TIMEOUT_MS.start(), *OSD_TIMEOUT_MS.end());
            w.push(format!("osd.timeout_ms = {} aralık dışı, {fixed} kullanılıyor (500..=10000)", self.timeout_ms));
            self.timeout_ms = fixed;
        }
    }
}

/// Duvar kağıdı yolu sınırı (bayt).
const MAX_WALLPAPER_PATH: usize = 4096;
const MAX_WALLPAPER_OUTPUTS: usize = 32;
/// Letterbox zemini bulanıklık yarıçapı sınırı (mantıksal px).
pub const WALLPAPER_BACKDROP_BLUR: std::ops::RangeInclusive<u32> = 4..=128;

impl WallpaperBackdropKind {
    pub const ALL: [WallpaperBackdropKind; 2] = [WallpaperBackdropKind::Blur, WallpaperBackdropKind::Color];

    /// `shell.toml`'daki adı (serde adıyla aynı; şema testi doğrular).
    pub fn name(self) -> &'static str {
        match self {
            WallpaperBackdropKind::Blur => "blur",
            WallpaperBackdropKind::Color => "color",
        }
    }

    /// Ayarlar'daki görünen adı.
    pub fn label(self) -> String {
        match self {
            WallpaperBackdropKind::Blur => t!("shell-schema-wallpaper-backdrop-blur"),
            WallpaperBackdropKind::Color => t!("shell-schema-wallpaper-backdrop-color"),
        }
    }
}

impl WallpaperBackdropColor {
    pub const ALL: [WallpaperBackdropColor; 3] =
        [WallpaperBackdropColor::Surface, WallpaperBackdropColor::SurfaceContainer, WallpaperBackdropColor::SurfaceBright];

    /// `shell.toml`'daki adı (tema rol adı; serde adıyla aynı).
    pub fn name(self) -> &'static str {
        match self {
            WallpaperBackdropColor::Surface => "surface",
            WallpaperBackdropColor::SurfaceContainer => "surface_container",
            WallpaperBackdropColor::SurfaceBright => "surface_bright",
        }
    }

    /// Ayarlar'daki görünen adı.
    pub fn label(self) -> String {
        match self {
            WallpaperBackdropColor::Surface => t!("shell-schema-wallpaper-backdrop-color-surface"),
            WallpaperBackdropColor::SurfaceContainer => t!("shell-schema-wallpaper-backdrop-color-surface-container"),
            WallpaperBackdropColor::SurfaceBright => t!("shell-schema-wallpaper-backdrop-color-surface-bright"),
        }
    }
}

impl WallpaperFit {
    pub const ALL: [WallpaperFit; 3] = [WallpaperFit::Cover, WallpaperFit::Contain, WallpaperFit::Fill];

    /// `shell.toml`'daki adı (serde adıyla aynı; şema testi doğrular).
    pub fn name(self) -> &'static str {
        match self {
            WallpaperFit::Cover => "cover",
            WallpaperFit::Contain => "contain",
            WallpaperFit::Fill => "fill",
        }
    }

    /// Ayarlar'daki görünen adı.
    pub fn label(self) -> String {
        match self {
            WallpaperFit::Cover => t!("shell-schema-wallpaper-fit-cover"),
            WallpaperFit::Contain => t!("shell-schema-wallpaper-fit-contain"),
            WallpaperFit::Fill => t!("shell-schema-wallpaper-fit-fill"),
        }
    }
}

impl WallpaperConfig {
    fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(
            SettingSchema::text("wallpaper.path", t!("shell-config-wallpaper-path-label"), d.path)
                .describe(t!("shell-config-wallpaper-path-describe")),
        );
        let fits = WallpaperFit::ALL.map(WallpaperFit::name).to_vec();
        out.push(SettingSchema::choice("wallpaper.fit", t!("shell-config-wallpaper-fit-label"), d.fit.name(), fits).labels(WallpaperFit::ALL.map(WallpaperFit::label).to_vec()));
        out.push(SettingSchema::choice(
            "wallpaper.backdrop",
            t!("shell-config-wallpaper-backdrop-label"),
            d.backdrop.name(),
            WallpaperBackdropKind::ALL.map(WallpaperBackdropKind::name).to_vec(),
        )
        .labels(WallpaperBackdropKind::ALL.map(WallpaperBackdropKind::label).to_vec()));
        let (min, max) = (i64::from(*WALLPAPER_BACKDROP_BLUR.start()), i64::from(*WALLPAPER_BACKDROP_BLUR.end()));
        out.push(SettingSchema::int("wallpaper.backdrop_blur", t!("shell-config-wallpaper-backdrop-blur-label"), d.backdrop_blur.into(), min, max).unit("px"));
        out.push(SettingSchema::choice(
            "wallpaper.backdrop_color",
            t!("shell-config-wallpaper-backdrop-color-label"),
            d.backdrop_color.name(),
            WallpaperBackdropColor::ALL.map(WallpaperBackdropColor::name).to_vec(),
        )
        .labels(WallpaperBackdropColor::ALL.map(WallpaperBackdropColor::label).to_vec()));
    }

    /// Bu monitörün duvar kağıdı (yol, yerleşim, zemin); yoksa `None`.
    pub fn for_output(&self, output: &str) -> Option<WallpaperWanted> {
        let o = self.outputs.get(output);
        let path = o.and_then(|o| o.path.as_deref()).unwrap_or(&self.path);
        let source = WallpaperSource::parse(path)?;
        let fit = o.and_then(|o| o.fit).unwrap_or(self.fit);
        Some(WallpaperWanted { source, fit, backdrop: self.backdrop_spec() })
    }

    /// Letterbox zemini ayarları.
    pub fn backdrop_spec(&self) -> WallpaperBackdropSpec {
        WallpaperBackdropSpec { kind: self.backdrop, blur: self.backdrop_blur, color: self.backdrop_color }
    }

    /// Shell en az bir monitörde duvar kağıdı çiziyor mu (genel ya da monitöre özel yol
    /// dolu). Öyleyse Hyprland'in kendi varsayılan duvar kağıdı/logosu kapatılır.
    pub fn draws_any(&self) -> bool {
        !self.path.is_empty() || self.outputs.values().any(|o| o.path.as_deref().is_some_and(|p| !p.is_empty()))
    }

    fn validate(&mut self, w: &mut Vec<String>) {
        let bad = |p: &str| p.len() > MAX_WALLPAPER_PATH || p.contains('\0');
        if bad(&self.path) {
            w.push("wallpaper.path çok uzun ya da geçersiz, yok sayıldı".into());
            self.path.clear();
        }
        if !WALLPAPER_BACKDROP_BLUR.contains(&self.backdrop_blur) {
            let fixed = self.backdrop_blur.clamp(*WALLPAPER_BACKDROP_BLUR.start(), *WALLPAPER_BACKDROP_BLUR.end());
            w.push(format!(
                "wallpaper.backdrop_blur = {} aralık dışı, {fixed} kullanılıyor ({}..={})",
                self.backdrop_blur,
                WALLPAPER_BACKDROP_BLUR.start(),
                WALLPAPER_BACKDROP_BLUR.end()
            ));
            self.backdrop_blur = fixed;
        }
        if self.outputs.len() > MAX_WALLPAPER_OUTPUTS {
            w.push(format!("wallpaper.outputs: en fazla {MAX_WALLPAPER_OUTPUTS} monitör, fazlası atıldı"));
            while self.outputs.len() > MAX_WALLPAPER_OUTPUTS {
                self.outputs.pop_last();
            }
        }
        for (name, o) in &mut self.outputs {
            if o.path.as_deref().is_some_and(bad) {
                w.push(format!("wallpaper.outputs.\"{}\".path geçersiz, yok sayıldı", sanitize(name)));
                o.path = None;
            }
        }
        // Bilinmeyen yerleşik kimlik/varyant: uyarı + varsayılan (dosyaya yazılmaz).
        let builtin = |key: &str, p: &mut String, w: &mut Vec<String>| {
            if let nuxut_wallpapers::Parsed::Invalid { warning, .. } = nuxut_wallpapers::parse_source(p) {
                w.push(format!("{key}: {warning}"));
                *p = nuxut_wallpapers::DEFAULT_SOURCE.to_owned();
            }
        };
        builtin("wallpaper.path", &mut self.path, w);
        for (name, o) in &mut self.outputs {
            if let Some(p) = o.path.as_mut() {
                builtin(&format!("wallpaper.outputs.\"{}\".path", sanitize(name)), p, w);
            }
        }
    }
}

/// `~` ve `~/…` → ev dizini. Diğer yollar olduğu gibi.
fn expand_home(path: &str) -> std::path::PathBuf {
    let home = || std::env::var_os("HOME").map(std::path::PathBuf::from);
    match path.strip_prefix('~') {
        Some("") => home().unwrap_or_else(|| path.into()),
        Some(rest) if rest.starts_with('/') => {
            home().map_or_else(|| path.into(), |h| h.join(rest.trim_start_matches('/')))
        }
        _ => path.into(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("config dizini bulunamadı (XDG_CONFIG_HOME / HOME yok)")]
    NoConfigDir,
    #[error(transparent)]
    Config(#[from] nuxut_config::ConfigError),
}

/// Yüklenmiş ve doğrulanmış config.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: ShellConfig,
    pub warnings: Vec<String>,
    pub from_file: bool,
    /// Bu yüklemede yapılan sürüm göçü (ADR-0002); bildirim ve geri al için.
    pub migration: Option<MigrationRecord>,
    /// Dosya sürümsüz kaldı (damga yazılamadı ya da yalnızca bellekte çalışılıyor) ama güncel
    /// sürüm sayıldı. Sonraki yüklemeye [`LoadState::unversioned_is_current`] olarak geçirilir.
    pub unversioned_is_current: bool,
    /// Kendini onarma (`config_heal`): yapılanlar ve diske yazıldıysa geri al bilgisi.
    pub heal: Option<HealReport>,
}

/// Süreç boyunca yüklemeler arasında taşınan göç durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoadState {
    pub policy: WritePolicy,
    /// Sürümsüz dosya bu oturumda zaten güncel sayıldı: göç adımları yeniden çalışmaz (yoksa
    /// damgası yazılamayan dosyaya sonradan v2 anlamıyla yazılan anahtar v1 sayılırdı).
    pub unversioned_is_current: bool,
    /// Bu yükleme onarımı diske yazar (açılış ve `msg config heal`). `false`: dosya değişince
    /// yeniden yükleme — onarım yalnızca bellekte (açık bir düzenleyicinin altından satır silinmez).
    pub heal_write: bool,
}

/// Onarımın bu yüklemedeki seçenekleri.
#[derive(Debug, Default)]
pub struct HealOptions<'a> {
    /// Kullanıcının geri aldığı/yok saydığı, dokunulmayacak anahtarlar.
    pub keep: BTreeSet<String>,
    /// Yazılacaksa ortam (yedek ve yazım kapısı); `None`: yalnızca bellekte.
    pub write: Option<&'a nuxut_apply::Env>,
    /// Yazım hatası önerilerinin uygulanması (`config heal --fix-typos`).
    pub renames: Vec<(String, String)>,
}

/// Göç sonucu ayrıştırılacak metin ve uyarı. Yazım çağıranın işidir.
fn migrated_text(text: &str, warnings: &mut Vec<String>) -> Option<Migrated> {
    match config_migrate::migrate(text) {
        Migration::Migrated(m) => Some(m),
        Migration::Newer(v) => {
            warnings.push(format!(
                "shell.toml daha yeni bir Nuxut sürümüne ait (config_version = {v}, bu sürüm {CURRENT_CONFIG_VERSION}); \
                 göç yapılmadı, tanınmayan anahtarlar yok sayılır"
            ));
            None
        }
        Migration::Current | Migration::Unchanged { .. } | Migration::Unparsable => None,
    }
}

impl ShellConfig {
    /// `~/.config/nuxut/shell.toml` dosyasını yükler (gerekirse sürüm göçü ve onarımla) ve
    /// doğrular. Yalnızca `state.policy` [`WritePolicy::Disk`] ise dosyaya yazılır (`nuxut-apply`
    /// ile, `env` kapısından); onarım ayrıca yalnızca `state.heal_write` iken.
    pub fn load(state: LoadState, env: &nuxut_apply::Env) -> Result<LoadedConfig, LoadError> {
        Self::load_healing(state, env, Vec::new())
    }

    /// [`load`](Self::load), yazım hatası önerileri taşınarak (`config heal --fix-typos`).
    pub fn load_healing(
        state: LoadState,
        env: &nuxut_apply::Env,
        renames: Vec<(String, String)>,
    ) -> Result<LoadedConfig, LoadError> {
        let path = nuxut_config::config_path(CONFIG_NAME).ok_or(LoadError::NoConfigDir)?;
        let writer = ApplyConfigWriter::new(env.clone());
        let disk = state.policy == WritePolicy::Disk;
        let writer: Option<&dyn ConfigWriter> = disk.then_some(&writer as &dyn ConfigWriter);
        let heal = HealOptions {
            keep: config_heal::keep_set(env, config_heal::SHELL_FILE),
            write: (disk && state.heal_write).then_some(env),
            renames,
        };
        Self::load_full(&path, writer, state.unversioned_is_current, &heal)
    }

    /// Yükler; göç, damga ve onarım `env` kapısından diske yazılır (testler ve araçlar için).
    pub fn load_from(path: &Path, env: &nuxut_apply::Env) -> Result<LoadedConfig, LoadError> {
        let heal = HealOptions { write: Some(env), ..HealOptions::default() };
        Self::load_full(path, Some(&ApplyConfigWriter::new(env.clone())), false, &heal)
    }

    /// Yükler; onarım yalnızca bellekte (bkz. [`load_full`](Self::load_full)).
    pub fn load_with(
        path: &Path,
        writer: Option<&dyn ConfigWriter>,
        unversioned_is_current: bool,
    ) -> Result<LoadedConfig, LoadError> {
        Self::load_full(path, writer, unversioned_is_current, &HealOptions::default())
    }

    /// Yükler; eski sürümlü dosya bellekte göç eder ve `writer` ile yazılır (ADR-0002 §3.4).
    /// Yazım başarısızsa göçmüş metinle çalışılır, dosya eski kalır, sonraki yükleme yeniden dener.
    /// Adımların değiştirmediği sürümsüz dosyaya yalnızca sürüm damgası yazılır; yazılamazsa
    /// (ya da `writer` `None` ise) dosya bellekte güncel sayılır. `writer = None`: hiçbir şey
    /// yazılmaz (ikincil/test örneği).
    ///
    /// Onarım (`config_heal`): önce sözdizimi (bozuk TOML), sonra göç, sonra tam onarım. Çalışan
    /// ayar her zaman onarılmış metinden okunur. Onarım metni değiştirdiyse ve `heal.write` varsa
    /// önce yedek alınır, göç + onarım + damga **tek yazımda** gider; yoksa dosyaya yalnızca
    /// eskisi gibi göç/damga yazılır (dosya okunabiliyorsa) ve onarım bekler.
    pub fn load_full(
        path: &Path,
        writer: Option<&dyn ConfigWriter>,
        unversioned_is_current: bool,
        heal: &HealOptions<'_>,
    ) -> Result<LoadedConfig, LoadError> {
        let Some((raw, lossy)) = config_heal::read_lossy(path)? else {
            return Ok(LoadedConfig {
                config: Self::validated_default(),
                warnings: Vec::new(),
                from_file: false,
                migration: None,
                unversioned_is_current: false,
                heal: None,
            });
        };
        let labels = config_heal::labels();
        let mut warnings = Vec::new();
        // 1. Sözdizimi: göç ve tam onarım okunabilir metin ister.
        let syntax = nuxut_config_heal::repair_syntax(&raw, &labels);
        let mut actions = syntax.actions;
        if lossy {
            actions.insert(0, nuxut_config_heal::Action { key: String::new(), kind: nuxut_config_heal::ActionKind::Broken, line: 0 });
        }
        let base = syntax.text;
        // Dosyanın kendisi okunabiliyor mu (göç/damga onarımsız da yazılabilir)?
        let raw_ok = !lossy && base == raw;
        // 2. Sürüm göçü / damga (bellekte).
        let mut stamp = false;
        let mut newer = false;
        let migrated = if unversioned_is_current && config_migrate::is_unversioned(&base) {
            stamp = true;
            None
        } else {
            let m = migrated_text(&base, &mut warnings);
            newer = matches!(config_migrate::migrate(&base), Migration::Newer(_));
            if m.is_none() {
                stamp = config_migrate::is_unversioned(&base);
            }
            m
        };
        let mtext = migrated.as_ref().map_or(base.as_str(), |m| m.text.as_str()).to_owned();
        // 3. Tam onarım (daha yeni sürümün dosyasında anahtarları tanımayız: yalnızca sözdizimi).
        let full = if newer { None } else { Some(config_heal::heal_shell(&mtext, &heal.keep, &labels, &heal.renames)) };
        let healed_text = full.as_ref().map_or(mtext.clone(), |h| h.text.clone());
        actions.extend(full.map(|h| h.actions).unwrap_or_default());
        let edited = actions.iter().any(|a| a.kind.edits());

        let mut migration = None;
        let mut written = None;
        let mut still_unversioned = false;
        if edited && let (Some(env), Some(w)) = (heal.write, writer) {
            // Tek yazım: göç (varsa, snapshot'ıyla) + onarım + damga; önce yedek.
            let final_text = if stamp { config_migrate::stamp_version(&healed_text).unwrap_or(healed_text.clone()) } else { healed_text.clone() };
            if let Some(dir) = config_heal::backup_dir(env) {
                match config_heal::write_backup(env, &dir, config_heal::SHELL_FILE, raw.as_bytes()) {
                    Ok(backup) => {
                        let (label, edits) = match &migrated {
                            Some(m) => (WriteLabel::Migration, m.edits.as_slice()),
                            None => (WriteLabel::Heal, &[][..]),
                        };
                        match w.replace(path, &raw, &final_text, label, edits) {
                            Ok(snapshot) => {
                                tracing::info!(changes = actions.len(), backup = %backup.display(), "shell.toml onarıldı");
                                if let Some(m) = &migrated {
                                    migration = Some(MigrationRecord { from: m.from, to: m.to, edits: m.edits.clone(), written: true, snapshot });
                                }
                                let restore = if migrated.is_some() { mtext.clone() } else { raw.clone() };
                                written = Some(HealWritten { path: path.to_owned(), backup, restore, written: final_text });
                                stamp = false;
                            }
                            Err(e) => warnings.push(format!("onarım dosyaya yazılamadı ({e}); bellekte uygulandı")),
                        }
                    }
                    Err(e) => warnings.push(format!("onarım yazılmadı: yedek alınamadı ({e}); bellekte uygulandı")),
                }
            }
        }
        if written.is_none() {
            // Onarım yazılmadı: dosya okunabiliyorsa göç ve damga eskisi gibi.
            if let Some(m) = &migrated {
                let (ok, snapshot) = match writer.filter(|_| raw_ok).map(|w| w.replace(path, &raw, &m.text, WriteLabel::Migration, &m.edits)) {
                    Some(Ok(snapshot)) => {
                        tracing::info!(from = m.from, to = m.to, edits = m.edits.len(), "shell.toml yeni sürüme taşındı");
                        (true, snapshot)
                    }
                    Some(Err(e)) => {
                        warnings.push(format!(
                            "sürüm göçü dosyaya yazılamadı ({e}); bellekte uygulandı, sonraki açılışta yeniden denenecek"
                        ));
                        (false, None)
                    }
                    None => {
                        tracing::info!(from = m.from, to = m.to, "shell.toml göçü yalnızca bellekte");
                        (false, None)
                    }
                };
                migration = Some(MigrationRecord { from: m.from, to: m.to, edits: m.edits.clone(), written: ok, snapshot });
            }
            if stamp && let Some(stamped) = config_migrate::stamp_version(&raw).filter(|_| raw_ok) {
                match writer.map(|w| w.replace(path, &raw, &stamped, WriteLabel::VersionStamp, &[])) {
                    Some(Ok(_)) => tracing::info!("shell.toml'a config_version = {CURRENT_CONFIG_VERSION} damgası yazıldı"),
                    Some(Err(e)) => {
                        warnings.push(format!(
                            "config_version damgası yazılamadı ({e}); dosya bu oturumda {CURRENT_CONFIG_VERSION}. sürüm sayılıyor"
                        ));
                        still_unversioned = true;
                    }
                    None => still_unversioned = true,
                }
            } else if stamp {
                still_unversioned = true;
            }
        }
        for a in &actions {
            tracing::info!(key = %a.key, line = a.line, kind = ?a.kind, "shell.toml onarımı");
        }
        let heal_report = (!actions.is_empty()).then_some(HealReport { file: config_heal::SHELL_FILE, actions, written });
        let (mut config, parse_warnings): (ShellConfig, _) = nuxut_config::parse_str(&healed_text)
            .map_err(|message| nuxut_config::ConfigError::Parse { path: path.to_owned(), message })?;
        warnings.extend(parse_warnings);
        warnings.extend(config.validate());
        Ok(LoadedConfig {
            config,
            warnings,
            from_file: true,
            migration,
            unversioned_is_current: still_unversioned,
            heal: heal_report,
        })
    }

    /// Metinden ayrıştırır ve doğrular (testler için). Eski sürümlü metin yalnızca bellekte
    /// göç eder; hiçbir şey yazılmaz.
    pub fn parse(text: &str) -> Result<(ShellConfig, Vec<String>), String> {
        let mut warnings = Vec::new();
        let migrated = migrated_text(text, &mut warnings);
        let text = migrated.as_ref().map_or(text, |m| m.text.as_str());
        let (mut config, parse_warnings): (ShellConfig, _) = nuxut_config::parse_str(text)?;
        warnings.extend(parse_warnings);
        warnings.extend(config.validate());
        Ok((config, warnings))
    }

    /// Doğrulanmış varsayılan config (widget ayarları çözülmüş).
    pub fn validated_default() -> Self {
        let mut c = Self::default();
        c.validate();
        c
    }

    /// Değerleri güvenli aralıklara çeker, eski anahtarları taşır, widget ayarlarını çözer;
    /// yapılan her düzeltme için bir uyarı döndürür. İdempotent.
    pub fn validate(&mut self) -> Vec<String> {
        let mut w = Vec::new();
        let d = ShellConfig::default();

        // --- çerçeve
        let f = &mut self.frame;
        clamp_f32(&mut f.thickness, "frame.thickness", 0.0, MAX_FRAME_THICKNESS, d.frame.thickness, &mut w);
        for (value, key) in
            [(&mut f.top, "frame.top"), (&mut f.right, "frame.right"), (&mut f.bottom, "frame.bottom"), (&mut f.left, "frame.left")]
        {
            if let Some(v) = value {
                clamp_f32(v, key, 0.0, MAX_FRAME_THICKNESS, d.frame.thickness, &mut w);
            }
        }
        // Sürüm göç katmanının; çözülmüş config'te tutulmaz (aynı içerik = aynı config).
        self.file_version = None;
        // Bar kenarındaki `frame.<kenar>` bar görünürken etkisizdir (kalınlık = bar.size);
        // bar gizlenince (`bar hide`) o kenarın kalınlığı olur. Bu yüzden uyarı verilmez.

        // --- bar
        let b = &mut self.bar;
        clamp_f32(&mut b.size, "bar.size", MIN_BAR_SIZE, MAX_BAR_SIZE, d.bar.size, &mut w);
        clamp_f32(&mut b.padding, "bar.padding", 0.0, MAX_BAR_PADDING, d.bar.padding, &mut w);
        clamp_f32(&mut b.spacing, "bar.spacing", 0.0, MAX_BAR_SPACING, d.bar.spacing, &mut w);
        // Her widget bar'da en fazla bir kez bulunur; bu, listeleri de sınırlar.
        let mut seen: Vec<WidgetKind> = Vec::new();
        for (list, key) in [(&mut b.start, "bar.start"), (&mut b.center, "bar.center"), (&mut b.end, "bar.end")] {
            filter_widgets(list, key, |p| p.bar, &mut seen, &mut w);
        }

        b.edge_actions.validate(&mut w);

        // --- widget ayarları: kayıttan çöz
        let mut settings = WidgetSettings::default();
        let mut unknown_tables = Vec::new();
        // Boş tablo etkisizdir: göçün boşalttığı `[bar.widget.workspaces]` başlığı onarımda gidebilsin
        // (eşitlik kâhini onu ayar farkı saymasın).
        b.widget_tables.retain(|_, t| !t.is_empty());
        for (name, table) in &b.widget_tables {
            match WidgetKind::from_name(name) {
                Some(kind) => settings.apply(kind, table, &mut w),
                None => unknown_tables.push(format!("\"{}\"", sanitize(name))),
            }
        }
        if !unknown_tables.is_empty() {
            unknown_tables.truncate(MAX_REPORTED_UNKNOWN);
            w.push(format!(
                "bar.widget: bilinmeyen widget tablosu {} yok sayıldı (geçerli: {})",
                unknown_tables.join(", "),
                WidgetKind::names_for(|p| p.bar)
            ));
        }
        settings.validate(&mut w);
        b.widgets = settings;

        // --- bildirimler
        w.extend(self.notifications.validate());

        // --- duvar kağıdı
        self.wallpaper.validate(&mut w);

        // --- ses
        self.audio.validate(&mut w);
        // --- başlatıcı
        w.extend(self.launcher.validate());

        // --- OSD
        self.osd.validate(&mut w);
        self.sysmon.validate(&mut w);
        self.idle.validate(&mut w);
        self.screenshot.validate(&mut w);
        self.clipboard.validate(&mut w);
        self.night_light.validate(&mut w);
        self.magnifier.validate(&mut w);
        self.osk.validate(&mut w);

        // --- masaüstü ortaklığı
        self.windows.validate(&mut w);
        self.keybinds.validate(&mut w);
        self.keybind_owner.validate(&mut w);
        self.input.validate(&mut w);
        self.displays.validate(&mut w);

        // --- kenar panelleri
        for edge in Edge::ALL {
            let p = self.panels.get_mut(edge);
            let key = format!("panels.{}", edge.name());
            let default_size = PanelsConfig::default().get(edge).size;
            clamp_f32(&mut p.size, &format!("{key}.size"), MIN_PANEL_SIZE, MAX_PANEL_SIZE, default_size, &mut w);
            let mut seen = Vec::new();
            filter_widgets(&mut p.widgets, &format!("{key}.widgets"), |pl| pl.panel, &mut seen, &mut w);
        }

        w
    }

    /// Genel anahtarların şeması (`nuxut-shell schema`). Her bölüm kendi anahtarlarını ekler;
    /// yeni bir config anahtarı o bölümün `schema`'sına da yazılır.
    pub fn schema(out: &mut Vec<SettingSchema>) {
        FrameConfig::schema(out);
        BarConfig::schema(out);
        PanelsConfig::schema(out);
        NotificationsConfig::schema(out);
        WallpaperConfig::schema(out);
        AudioConfig::schema(out);
        MediaConfig::schema(out);
        OsdConfig::schema(out);
        crate::services::launcher::LauncherConfig::schema(out);
        CompositorConfig::schema(out);
        WindowsConfig::schema(out);
        KeybindsConfig::schema(out);
        InputConfig::schema(out);
        crate::services::displays::DisplaysConfig::schema(out);
        crate::services::setup::SetupConfig::schema(out);
        crate::services::sysmon::SysmonConfig::schema(out);
        crate::services::idle::IdleConfig::schema(out);
        crate::services::lock_launcher::LockConfig::schema(out);
        crate::services::screenshot::ScreenshotConfig::schema(out);
        crate::services::clipboard::ClipboardConfig::schema(out);
        crate::services::night_light::NightLightConfig::schema(out);
        crate::services::removable_media::RemovableMediaConfig::schema(out);
        crate::services::magnifier::MagnifierConfig::schema(out);
        crate::services::osk::OskConfig::schema(out);
    }

    /// Kenar başına çerçeve kalınlıkları. Bar kenarı = `bar.size`; `frame.enabled = false`
    /// ise yalnızca bar kenarı kalır (bar her zaman çerçevenin kalın kenarıdır).
    pub fn frame_insets(&self) -> Insets {
        self.frame_insets_with_bar(true)
    }

    /// `bar = false`: bar gizli; bar kenarı da diğer kenarlar gibi çerçeve kalınlığındadır
    /// (`frame.<kenar>` geçersiz kılması burada uygulanır).
    pub fn frame_insets_with_bar(&self, bar: bool) -> Insets {
        let f = &self.frame;
        let edge = |o: Option<f32>| if f.enabled { o.unwrap_or(f.thickness) } else { 0.0 };
        let mut insets = Insets::new(edge(f.top), edge(f.right), edge(f.bottom), edge(f.left));
        if !bar {
            return insets;
        }
        match self.bar.edge {
            Edge::Top => insets.top = self.bar.size,
            Edge::Right => insets.right = self.bar.size,
            Edge::Bottom => insets.bottom = self.bar.size,
            Edge::Left => insets.left = self.bar.size,
        }
        insets
    }
}

/// Listeden bilinmeyen, bu yere konamayan ve tekrarlanan widget'ları atar.
fn filter_widgets(
    list: &mut Vec<String>,
    key: &str,
    allowed: fn(crate::widgets::Placement) -> bool,
    seen: &mut Vec<WidgetKind>,
    w: &mut Vec<String>,
) {
    let mut unknown: Vec<String> = Vec::new();
    let mut unknown_count = 0usize;
    list.retain(|name| match WidgetKind::from_name(name) {
        None => {
            unknown_count += 1;
            if unknown.len() < MAX_REPORTED_UNKNOWN {
                unknown.push(format!("\"{}\"", sanitize(name)));
            }
            false
        }
        Some(kind) if !allowed(kind.placement()) => {
            w.push(format!("{key}: \"{name}\" burada kullanılamaz, yok sayıldı"));
            false
        }
        Some(kind) if seen.contains(&kind) => {
            w.push(format!("{key}: \"{name}\" birden fazla kez yazılmış, tekrarı yok sayıldı"));
            false
        }
        Some(kind) => {
            seen.push(kind);
            true
        }
    });
    if unknown_count > 0 {
        let more = unknown_count - unknown.len();
        let more = if more > 0 { format!(" (+{more} daha)") } else { String::new() };
        w.push(format!(
            "{key}: bilinmeyen widget {}{more} yok sayıldı (geçerli: {})",
            unknown.join(", "),
            WidgetKind::names_for(allowed)
        ));
    }
}

fn clamp_f32(value: &mut f32, key: &str, min: f32, max: f32, default: f32, warnings: &mut Vec<String>) {
    if !value.is_finite() {
        warnings.push(format!("{key} sayı değil, varsayılan {default} kullanılıyor"));
        *value = default;
    } else if *value < min || *value > max {
        let fixed = value.clamp(min, max);
        warnings.push(format!("{key} = {} aralık dışı, {fixed} kullanılıyor ({min}..={max})", *value));
        *value = fixed;
    }
}

/// Uyarı metnine giren kullanıcı girdisini kısaltır ve kontrol karakterlerinden arındırır.
pub(crate) fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::PanelTarget;

    fn parse(text: &str) -> (ShellConfig, Vec<String>) {
        ShellConfig::parse(text).expect("ayrıştırılabilir olmalı")
    }

    #[test]
    fn empty_file_is_validated_default_without_warnings() {
        let (c, w) = parse("");
        assert_eq!(c, ShellConfig::validated_default());
        assert!(w.is_empty(), "{w:?}");
    }

    /// ADR-0002 §3.6: yeni widget (status) yalnızca listeye dokunmamış kullanıcıda görünür;
    /// dosyada yazılı `bar.end` olduğu gibi kalır (göç listeye widget eklemez).
    #[test]
    fn new_widget_only_in_untouched_list() {
        let (c, _) = parse("config_version = 2\n[bar]\nspacing = 8.0\n");
        assert!(c.bar.widgets_in(BarSection::End).contains(&WidgetKind::Status));
        let (c, w) = parse("config_version = 2\n[bar]\nend = [\"tray\", \"session\"]\n");
        assert_eq!(c.bar.widgets_in(BarSection::End), [WidgetKind::Tray, WidgetKind::Session]);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn defaults() {
        let c = ShellConfig::validated_default();
        assert!(c.frame.enabled);
        assert_eq!(c.frame.thickness, 6.0);
        assert_eq!(c.bar.edge, Edge::Top);
        assert_eq!((c.bar.size, c.bar.padding, c.bar.spacing), (34.0, 10.0, 12.0));
        assert_eq!(c.bar.widgets_in(BarSection::Start), [WidgetKind::Logo, WidgetKind::Workspaces]);
        assert_eq!(c.bar.widgets_in(BarSection::Center), [WidgetKind::Clock, WidgetKind::NextEvent, WidgetKind::NowPlaying]);
        assert_eq!(
            c.bar.widgets_in(BarSection::End),
            [
                WidgetKind::Privacy,
                WidgetKind::Magnifier,
                WidgetKind::KeyboardLayout,
                WidgetKind::Sysmon,
                WidgetKind::Tray,
                WidgetKind::Status,
                WidgetKind::Notifications,
                WidgetKind::Session
            ]
        );
        assert_eq!(c.bar.widgets.workspaces.count, 10);
        assert_eq!(c.bar.widgets.clock.format, None, "varsayılan: otomatik ([locale] hour_cycle)");
        assert!(c.panels.right.enabled && !c.panels.left.enabled);
        assert_eq!(
            c.panels.right.widget_kinds(),
            [
                WidgetKind::Session,
                WidgetKind::Toggles,
                WidgetKind::Volume,
                WidgetKind::Mixer,
                WidgetKind::Brightness,
                WidgetKind::Network,
                WidgetKind::Bluetooth,
                WidgetKind::Power,
                WidgetKind::Media,
                WidgetKind::Notifications
            ]
        );
    }

    #[test]
    fn audio_max_volume_is_bounded() {
        assert_eq!(ShellConfig::validated_default().audio.max_volume, 100);
        let (c, w) = parse("[audio]\nmax_volume = 150\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.audio.max_volume, 150);
        let (c, w) = parse("[audio]\nmax_volume = 400\n");
        assert_eq!(c.audio.max_volume, 150);
        assert!(w[0].contains("audio.max_volume"), "{w:?}");
        let (c, _) = parse("[audio]\nmax_volume = 20\n");
        assert_eq!(c.audio.max_volume, 100);
        assert_eq!(c.audio.backend, AudioBackend::Wpctl, "varsayılan wpctl");
        let (c, w) = parse("[audio]\nbackend = \"native\"\n");
        assert_eq!((c.audio.backend, w.len()), (AudioBackend::Native, 0));
    }

    #[test]
    fn media_remote_art_defaults_on() {
        assert!(ShellConfig::validated_default().media.remote_art);
        let (c, w) = parse("[media]\nremote_art = false\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(!c.media.remote_art);
        assert!(!crate::system::system_options(&c).remote_art);
    }

    #[test]
    fn quick_settings_cards_are_panel_only() {
        let (c, w) = parse(
            "[bar]\nend = [\"volume\", \"session\"]\n[panels.right]\nwidgets = [\"volume\", \"brightness\", \"power\", \"network\", \"bluetooth\", \"media\"]\n",
        );
        assert_eq!(c.bar.widgets_in(BarSection::End), [WidgetKind::Session]);
        assert!(w.len() == 1 && w[0].contains("bar.end") && w[0].contains("volume"), "{w:?}");
        assert_eq!(c.panels.right.widget_kinds().len(), 6);
    }

    #[test]
    fn default_insets_put_bar_on_top() {
        assert_eq!(ShellConfig::validated_default().frame_insets(), Insets::new(34.0, 6.0, 6.0, 6.0));
    }

    #[test]
    fn bar_on_every_edge_takes_that_edge() {
        for (edge, expected) in [
            ("top", Insets::new(40.0, 6.0, 6.0, 6.0)),
            ("right", Insets::new(6.0, 40.0, 6.0, 6.0)),
            ("bottom", Insets::new(6.0, 6.0, 40.0, 6.0)),
            ("left", Insets::new(6.0, 6.0, 6.0, 40.0)),
        ] {
            let (c, w) = parse(&format!("[bar]\nedge = \"{edge}\"\nsize = 40\n"));
            assert!(w.is_empty(), "{w:?}");
            assert_eq!(c.frame_insets(), expected, "{edge}");
            assert_eq!(c.bar.is_vertical(), edge == "left" || edge == "right");
        }
    }

    #[test]
    fn hidden_bar_edge_falls_back_to_frame_thickness() {
        let c = ShellConfig::validated_default();
        assert_eq!(c.frame_insets_with_bar(false), Insets::uniform(6.0));
        let (c, _) = parse("[bar]\nedge = \"left\"\n[frame]\nleft = 2.0\n");
        assert_eq!(c.frame_insets_with_bar(false).left, 2.0);
        assert_eq!(c.frame_insets().left, 34.0);
    }

    #[test]
    fn per_edge_overrides_resolve() {
        let (c, w) = parse("[frame]\nthickness = 4.0\nleft = 10.0\nright = 0.0\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.frame_insets(), Insets::new(34.0, 0.0, 4.0, 10.0));
    }

    #[test]
    fn bar_edge_override_only_applies_when_bar_hidden() {
        let (c, w) = parse("[bar]\nedge = \"left\"\n[frame]\nleft = 12.0\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.frame_insets().left, 34.0);
        assert_eq!(c.frame_insets_with_bar(false).left, 12.0);
    }

    #[test]
    fn disabled_frame_keeps_only_bar() {
        let (c, _) = parse("[frame]\nenabled = false\nleft = 9.0\n");
        assert_eq!(c.frame_insets(), Insets::new(34.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn invalid_edge_is_parse_error() {
        assert!(ShellConfig::parse("[bar]\nedge = \"middle\"\n").is_err());
    }

    #[test]
    fn logo_leads_default_start_only() {
        // Listeyi yazmamış kullanıcı: işaret başta (serde varsayılanı; göç adımı gerekmez).
        let (c, w) = parse("[bar]\nsize = 40\n");
        assert_eq!(c.bar.widgets_in(BarSection::Start), [WidgetKind::Logo, WidgetKind::Workspaces]);
        assert!(w.is_empty(), "{w:?}");
        // Açıkça yazılmış `bar.start`'a dokunulmaz (boş liste dahil).
        let (c, _) = parse("[bar]\nstart = [\"workspaces\", \"clock\"]\n");
        assert_eq!(c.bar.widgets_in(BarSection::Start), [WidgetKind::Workspaces, WidgetKind::Clock]);
        let (c, _) = parse("[bar]\nstart = []\n");
        assert!(c.bar.start.is_empty());
        // İşaret yalnızca başta: diğer bölümlerde yok, başka yere de konabilir.
        let d = ShellConfig::default();
        assert!(!d.bar.center.iter().chain(&d.bar.end).any(|n| n == "logo"));
        let (c, w) = parse("[bar]\nstart = [\"workspaces\"]\nend = [\"logo\"]\n");
        assert_eq!(c.bar.widgets_in(BarSection::End), [WidgetKind::Logo]);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn legacy_left_right_migrate_to_start_end() {
        let (c, w) = parse("[bar]\nleft = [\"clock\"]\ncenter = []\nright = [\"workspaces\"]\n");
        assert_eq!(c.bar.widgets_in(BarSection::Start), [WidgetKind::Clock]);
        assert_eq!(c.bar.widgets_in(BarSection::End), [WidgetKind::Workspaces]);
        assert!(w.is_empty(), "göç sessiz: {w:?}");
        // Güncel sürümde eski anahtar artık bilinmeyen anahtardır.
        let (c, w) = parse("config_version = 2\n[bar]\nleft = [\"clock\"]\n");
        assert_eq!(c.bar.start, ShellConfig::default().bar.start);
        assert!(w.len() == 1 && w[0].contains("bar.left"), "{w:?}");
    }

    #[test]
    fn legacy_top_level_tables_migrate_to_widget_settings() {
        let (c, w) = parse("[workspaces]\ncount = 5\nshow_empty = true\n[clock]\nformat = \"%H:%M:%S\"\n");
        assert_eq!(c.bar.widgets.workspaces.count, 5);
        assert_eq!(c.bar.widgets.clock.format.as_deref(), Some("%H:%M:%S"));
        assert!(w.is_empty(), "{w:?}");
        // Yeni tablo varsa eskisi (zaten etkisizdi) silinir.
        let (c, w) = parse("[clock]\nformat = \"%R\"\n[bar.widget.clock]\nformat = \"%T\"\n");
        assert_eq!(c.bar.widgets.clock.format.as_deref(), Some("%T"));
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn widget_tables_resolve_through_registry() {
        let (c, w) = parse(
            "[bar.widget.clock]\nvertical_format = \"%H\\n%M\\n%S\"\n\
             [bar.widget.panel-toggle]\npanel = \"left\"\n\
             [bar.widget.weather]\ncity = \"x\"\n",
        );
        assert_eq!(c.bar.widgets.clock.vertical_format.as_deref(), Some("%H\n%M\n%S"));
        assert_eq!(c.bar.widgets.panel_toggle.panel, PanelTarget::Left);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("weather"), "{w:?}");
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let (c, w) = parse(
            "config_version = 2\n[frame]\nthickness = -3.0\nleft = 500.0\n\
             [bar]\nsize = 2.0\npadding = -1.0\n[bar.widget.workspaces]\ncount = 0\n\
             [panels.right]\nsize = 5000\n",
        );
        assert_eq!(c.frame.thickness, 0.0);
        assert_eq!(c.frame.left, Some(MAX_FRAME_THICKNESS));
        assert_eq!(c.bar.size, MIN_BAR_SIZE);
        assert_eq!(c.bar.padding, 0.0);
        assert_eq!(c.bar.widgets.workspaces.count, 1);
        assert_eq!(c.panels.right.size, MAX_PANEL_SIZE);
        assert_eq!(w.len(), 6, "{w:?}");
    }

    #[test]
    fn non_finite_values_fall_back_to_default() {
        let (c, w) = parse("[frame]\nthickness = nan\ninner_radius = inf\n");
        assert_eq!(c.frame.thickness, 6.0);
        assert_eq!(w.len(), 1, "inner_radius göçte sessizce silinir: {w:?}");
    }

    /// Kalkan `frame.inner_radius` (ADR-0008) eski config'i bozmaz: her tür değer göçte silinir.
    #[test]
    fn legacy_inner_radius_is_migrated_away() {
        for value in ["20", "1e9", "\"x\"", "[1, 2]"] {
            let (c, w) = parse(&format!("[frame]\ninner_radius = {value}\n"));
            assert_eq!(c.frame, ShellConfig::validated_default().frame, "{value}");
            assert!(w.is_empty(), "{value}: {w:?}");
        }
    }

    /// Sürüm alanı çözülmüş config'e girmez: aynı ayarlar = aynı config.
    #[test]
    fn config_version_is_accepted_and_not_part_of_config() {
        let (c, w) = parse("config_version = 2\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c, ShellConfig::validated_default());
    }

    #[test]
    fn unknown_duplicate_and_misplaced_widgets_are_dropped() {
        crate::i18n::use_turkish_for_tests();
        let (c, w) = parse(
            "[bar]\nstart = [\"workspaces\", \"wifi\"]\ncenter = [\"clock\"]\nend = [\"clock\", \"active_window\"]\n\
             [panels.left]\nwidgets = [\"clock\", \"session\"]\n",
        );
        assert_eq!(c.bar.widgets_in(BarSection::Start), [WidgetKind::Workspaces]);
        assert_eq!(c.bar.widgets_in(BarSection::End), [WidgetKind::ActiveWindow]);
        assert_eq!(c.panels.left.widget_kinds(), [WidgetKind::Session]);
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w[0].contains("wifi"), "{w:?}");
        assert!(w[1].contains("clock") && w[1].contains("birden fazla"), "{w:?}");
        assert!(w[2].contains("panels.left.widgets") && w[2].contains("kullanılamaz"), "{w:?}");
    }

    #[test]
    fn huge_unknown_widget_list_gives_one_short_warning() {
        crate::i18n::use_turkish_for_tests();
        let names = (0..1000).map(|i| format!("\"w{i}\"")).collect::<Vec<_>>().join(",");
        let (c, w) = parse(&format!("[bar]\nend = [{names}]\n"));
        assert!(c.bar.end.is_empty());
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("+996 daha"), "{w:?}");
        assert!(w[0].len() < 320, "{w:?}");
    }

    #[test]
    fn widget_warning_sanitizes_input() {
        let (_, w) = parse("[bar]\nstart = [\"a\\u001b[31mb\"]\n");
        assert_eq!(w.len(), 1);
        assert!(!w[0].contains('\u{1b}'), "{w:?}");
    }

    #[test]
    fn unknown_keys_warn_but_load() {
        let (c, w) = parse("[frame]\nthicknes = 3.0\n[barr]\n");
        assert_eq!(c, ShellConfig::validated_default());
        assert_eq!(w.len(), 2, "{w:?}");
    }

    #[test]
    fn load_from_missing_file_is_default() {
        let t = tempfile::tempdir().unwrap();
        let loaded = ShellConfig::load_from(Path::new("/nonexistent/nuxut/shell.toml"), &nuxut_apply::Env::for_test(t.path())).unwrap();
        assert!(!loaded.from_file);
        assert_eq!(loaded.config, ShellConfig::validated_default());
    }

    #[test]
    fn wallpaper_resolution_per_output() {
        let (c, w) = parse(
            "[wallpaper]\npath = \"~/Resimler/dag.png\"\nfit = \"contain\"\n\
             [wallpaper.outputs.\"DP-1\"]\npath = \"/srv/a.gif\"\n\
             [wallpaper.outputs.\"HDMI-A-1\"]\nfit = \"fill\"\n",
        );
        assert!(w.is_empty(), "{w:?}");
        let home = std::env::var("HOME").unwrap_or_default();
        let e = c.wallpaper.for_output("eDP-1").unwrap();
        assert_eq!(e.source, WallpaperSource::File(format!("{home}/Resimler/dag.png").into()));
        assert_eq!(e.fit, WallpaperFit::Contain);
        let dp = c.wallpaper.for_output("DP-1").unwrap();
        assert_eq!((dp.source, dp.fit), (WallpaperSource::File("/srv/a.gif".into()), WallpaperFit::Contain));
        assert_eq!(c.wallpaper.for_output("HDMI-A-1").unwrap().fit, WallpaperFit::Fill);
        // Açıkça boş yol: duvar kağıdı yok (başka bir araç kullanılıyor olabilir).
        let (none, _) = parse("[wallpaper]\npath = \"\"\n");
        assert!(none.wallpaper.for_output("DP-1").is_none());
        assert!(!none.wallpaper.draws_any());
        assert!(ShellConfig::parse("[wallpaper]\nfit = \"tile\"\n").is_err(), "desteklenmeyen kip");
        // Hyprland varsayılanlarını kapatma kararı: herhangi bir monitörde yol varsa.
        assert!(c.wallpaper.draws_any());
        assert!(ShellConfig::validated_default().wallpaper.draws_any(), "varsayılan yerleşik çizilir");
        let (only_output, _) = parse("[wallpaper]\npath = \"\"\n[wallpaper.outputs.\"DP-1\"]\npath = \"/srv/a.png\"\n");
        assert!(only_output.wallpaper.draws_any());
        let (empty_output, _) = parse("[wallpaper]\npath = \"\"\n[wallpaper.outputs.\"DP-1\"]\nfit = \"fill\"\n");
        assert!(!empty_output.wallpaper.draws_any());
    }

    fn builtin(id: &str, variant: Option<nuxut_wallpapers::Variant>) -> WallpaperSource {
        WallpaperSource::Builtin(nuxut_wallpapers::Selection { builtin: nuxut_wallpapers::find(id).unwrap(), variant })
    }

    #[test]
    fn wallpaper_builtin_default_and_sources() {
        // Taze kurulum / anahtar yazılmamış: yerleşik eş uçuş, tema kipini izler.
        let d = ShellConfig::validated_default();
        assert_eq!(d.wallpaper.path, "nuxut:pair-flight");
        assert_eq!(d.wallpaper.for_output("DP-1").unwrap().source, builtin("pair-flight", None));
        let (c, w) = parse("[wallpaper]\nfit = \"contain\"\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.wallpaper.for_output("eDP-1").unwrap().source, builtin("pair-flight", None));
        // Açık yerleşik, sabit varyant, monitöre özel.
        let (c, w) = parse(
            "[wallpaper]\npath = \"nuxut:aurora\"\n[wallpaper.outputs.\"DP-1\"]\npath = \"nuxut:migration/light\"\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.wallpaper.for_output("eDP-1").unwrap().source, builtin("aurora", None));
        assert_eq!(
            c.wallpaper.for_output("DP-1").unwrap().source,
            builtin("migration", Some(nuxut_wallpapers::Variant::Light))
        );
        // Bilinmeyen kimlik ya da eski `:dark` varyant biçimi (yayımlanmadı, kabul edilmez): uyarı + varsayılan.
        let (c, w) = parse("[wallpaper]\npath = \"nuxut:yok\"\n[wallpaper.outputs.\"DP-1\"]\npath = \"nuxut:aurora:dark\"\n");
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].contains("wallpaper.path") && w[1].contains("DP-1"), "{w:?}");
        assert_eq!(c.wallpaper.path, "nuxut:pair-flight");
        assert_eq!(c.wallpaper.for_output("DP-1").unwrap().source, builtin("pair-flight", None));
        // Önek dışı: dosya yolu (göreli değil; "./nuxut:x" dosyadır).
        assert_eq!(WallpaperSource::parse("./nuxut:x"), Some(WallpaperSource::File("./nuxut:x".into())));
        assert_eq!(WallpaperSource::parse(""), None);
    }

    #[test]
    fn wallpaper_backdrop_defaults_parse_and_bounds() {
        // Varsayılan: letterbox'ta bulanık kopya (siyah/düz bant değil), tema yüzeyi yedek.
        let d = ShellConfig::validated_default().wallpaper.backdrop_spec();
        assert_eq!(d, WallpaperBackdropSpec { kind: WallpaperBackdropKind::Blur, blur: 48, color: WallpaperBackdropColor::Surface });
        assert!(WALLPAPER_BACKDROP_BLUR.contains(&d.blur));

        let (c, w) = parse(
            "[wallpaper]\npath = \"/srv/a.png\"\nbackdrop = \"color\"\nbackdrop_blur = 16\n\
             backdrop_color = \"surface_container\"\n",
        );
        assert!(w.is_empty(), "{w:?}");
        let b = c.wallpaper.for_output("DP-1").unwrap().backdrop;
        assert_eq!(b, WallpaperBackdropSpec { kind: WallpaperBackdropKind::Color, blur: 16, color: WallpaperBackdropColor::SurfaceContainer });

        // Sınır dışı yarıçap uyarıyla kırpılır.
        for (raw, fixed) in [(0, 4), (100_000, 128)] {
            let (c, w) = parse(&format!("[wallpaper]\nbackdrop_blur = {raw}\n"));
            assert_eq!(c.wallpaper.backdrop_blur, fixed);
            assert_eq!(w.len(), 1, "{w:?}");
        }
        assert!(ShellConfig::parse("[wallpaper]\nbackdrop = \"none\"\n").is_err(), "bilinmeyen zemin");
        assert!(ShellConfig::parse("[wallpaper]\nbackdrop_color = \"#000000\"\n").is_err(), "sabit renk yok, yalnızca tema rolü");
        assert!(ShellConfig::parse("[wallpaper]\nbackdrop_blur = -3\n").is_err(), "negatif yarıçap");
    }

    /// Kısmi panel tablosu o kenarın varsayılanını korur (genel `EdgePanelConfig` varsayılanına
    /// düşmez).
    #[test]
    fn partial_panel_table_keeps_edge_defaults() {
        let (c, w) = ShellConfig::parse("[panels.right]\nsize = 400\n[panels.top]\nenabled = true\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        let d = PanelsConfig::default();
        assert_eq!(c.panels.right, EdgePanelConfig { size: 400.0, ..d.right.clone() });
        assert_eq!(c.panels.top, EdgePanelConfig { enabled: true, ..d.top.clone() });
        assert_eq!(c.panels.left, d.left);
        let (_, w) = ShellConfig::parse("[panels.right]\nsizee = 400\n").unwrap();
        assert_eq!(w.len(), 1, "bilinmeyen anahtar uyarı verir: {w:?}");
    }

    #[test]
    fn osd_timeout_is_clamped() {
        assert_eq!(ShellConfig::validated_default().osd.timeout_ms, 1200);
        let (c, w) = parse("[osd]\ntimeout_ms = 3000\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.osd.timeout_ms, 3000);
        let (c, w) = parse("[osd]\ntimeout_ms = 10\n");
        assert_eq!((c.osd.timeout_ms, w.len()), (500, 1), "{w:?}");
        let (c, _) = parse("[osd]\ntimeout_ms = 99999\n");
        assert_eq!(c.osd.timeout_ms, 10_000);
    }

    #[test]
    fn validate_is_idempotent() {
        let (mut c, _) = parse("[frame]\nthickness = -1\n[bar]\nleft = [\"x\"]\n[clock]\nformat = \"%T\"\n");
        let before = c.clone();
        assert!(c.validate().is_empty());
        assert_eq!(c, before);
    }

    /// CTRL+ALT+Delete (`msg session menu`): oturum kartının olduğu açık kenar paneli.
    #[test]
    fn session_menu_edge_follows_session_card() {
        assert_eq!(ShellConfig::default().panels.session_edge(), Edge::Right);
        let (c, w) = parse("[panels.right]\nwidgets = [\"volume\"]\n[panels.left]\nenabled = true\nwidgets = [\"volume\", \"session\"]\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.panels.session_edge(), Edge::Left);
        // Kapalı panel sayılmaz; hiçbir yerde yoksa sağ.
        let (c, _) = parse("[panels.right]\nwidgets = [\"volume\"]\n[panels.left]\nenabled = false\nwidgets = [\"session\"]\n");
        assert_eq!(c.panels.session_edge(), Edge::Right);
    }
}
