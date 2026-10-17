//! Widget kaydı: bar'a ve kenar panellerine konabilen her widget'ın adı, yerleşebileceği
//! yerler ve ayar tipi **tek yerde** tanımlanır. Yeni widget eklemek = buraya bir varyant,
//! ayar yapısı ve `match` kolları + `widgets/<ad>.rs` içinde çizimi.
//!
//! Ayarlar `[bar.widget.<ad>]` tablolarından okunur. Tablo yazılmamışsa varsayılanlar
//! geçerlidir; bilinmeyen anahtar ve geçersiz değer uyarıyla düzeltilir (çalışma durmaz).

use nuxut::i18n::t;
use nuxut::ui::IconName;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::schema::SettingSchema;
use crate::services::clock::ClockFormat;

/// Widget türleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WidgetKind {
    Workspaces,
    Clock,
    ActiveWindow,
    /// Şimdi çalıyor: etkin MPRIS oynatıcısının kapağı, "Başlık · Sanatçı" ve durum (yalnızca
    /// bar; çalan/duraklatılmış oynatıcı yoksa gizli). `docs/design/now-playing.md`.
    NowPlaying,
    /// Gizlilik göstergesi: mikrofon, kamera ya da ekran yakalama (paylaşım/kayıt) kullanımdayken
    /// görünür (yalnız bar). `docs/design/privacy-indicator.md`.
    Privacy,
    /// Klavye düzeni göstergesi: etkin düzen, tıkla sıradaki (yalnız 2+ düzende görünür; yalnız bar).
    /// `docs/design/keyboard-layout.md`.
    KeyboardLayout,
    /// Sıradaki etkinlik: eşik içinde başlayacak ya da süren etkinlik (yalnız bar; yoksa gizli).
    /// `docs/design/next-event.md`.
    NextEvent,
    /// Uyanık tut: elle boşta kalma engeli düğmesi (açıkken dolgulu; yalnız bar).
    Caffeine,
    Tray,
    Session,
    Notifications,
    PanelToggle,
    // Hızlı ayar kartları (yalnızca panel).
    /// Hızlı anahtarlar: Wi-Fi, Bluetooth, Rahatsız etme, Uyanık tut, Gece ışığı, Koyu tema
    /// kutucuk ızgarası (`docs/design/quick-toggles.md`).
    Toggles,
    Volume,
    /// Ses karıştırıcı: ses çalan uygulamaların seviyesi/sessizi; cihaz seçimi ses kartında
    /// (`docs/design/mixer.md`).
    Mixer,
    Brightness,
    /// Gece ışığı: anahtar + sıcaklık (t113); compositor CTM sunmuyorsa gizli.
    NightLight,
    /// Büyüteç: anahtar + çarpan (t124); Hyprland'e komut gönderilemiyorsa gizli.
    Magnifier,
    /// Ekran klavyesi: göster/gizle anahtarı (t125); sanal klavye protokolü yoksa gizli.
    Osk,
    Power,
    Network,
    Bluetooth,
    Media,
    /// Uygulama başlatıcısını açan düğme.
    Launcher,
    /// Sistem izleyicisi: seçilen ölçümler; tıklayınca ayrıntı popout'u (t099).
    Sysmon,
    /// Durum göstergeleri: ağ, Bluetooth, ses, pil (t062); tıklayınca hızlı ayarlar paneli.
    Status,
    /// Nuxut marka işareti; tıklayınca uygulama başlatıcısı (`launcher` ile aynı yaprak).
    Logo,
    /// Takvim kartı (yalnızca panel): saat popout'unun ay takvimi ve seçili günün etkinlikleri
    /// (`CalendarView` kart kipi; `docs/design/calendar-card.md`).
    Calendar,
}

/// Bir widget'ın konabileceği yerler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub bar: bool,
    pub panel: bool,
}

impl WidgetKind {
    /// Sıra bilinçlidir: ayarlar uygulamasının "widget ekle" menüsü bu sırayı kullanır
    /// (`nuxut-shell schema`); en çok kullanılanlar önce.
    pub const ALL: [WidgetKind; 28] = [
        WidgetKind::Workspaces,
        WidgetKind::Clock,
        WidgetKind::ActiveWindow,
        WidgetKind::NowPlaying,
        WidgetKind::Privacy,
        WidgetKind::Caffeine,
        WidgetKind::KeyboardLayout,
        WidgetKind::NextEvent,
        WidgetKind::Tray,
        WidgetKind::Session,
        WidgetKind::Notifications,
        WidgetKind::PanelToggle,
        WidgetKind::Toggles,
        WidgetKind::Volume,
        WidgetKind::Mixer,
        WidgetKind::Brightness,
        WidgetKind::NightLight,
        WidgetKind::Magnifier,
        WidgetKind::Osk,
        WidgetKind::Power,
        WidgetKind::Network,
        WidgetKind::Bluetooth,
        WidgetKind::Media,
        WidgetKind::Launcher,
        WidgetKind::Sysmon,
        WidgetKind::Status,
        WidgetKind::Logo,
        WidgetKind::Calendar,
    ];

    pub fn name(self) -> &'static str {
        match self {
            WidgetKind::Workspaces => "workspaces",
            WidgetKind::Clock => "clock",
            WidgetKind::ActiveWindow => "active_window",
            WidgetKind::NowPlaying => "now_playing",
            WidgetKind::Privacy => "privacy",
            WidgetKind::Caffeine => "caffeine",
            WidgetKind::KeyboardLayout => "keyboard_layout",
            WidgetKind::NextEvent => "next_event",
            WidgetKind::Tray => "tray",
            WidgetKind::Session => "session",
            WidgetKind::Notifications => "notifications",
            WidgetKind::PanelToggle => "panel-toggle",
            WidgetKind::Toggles => "toggles",
            WidgetKind::Volume => "volume",
            WidgetKind::Mixer => "mixer",
            WidgetKind::Brightness => "brightness",
            WidgetKind::NightLight => "night_light",
            WidgetKind::Magnifier => "magnifier",
            WidgetKind::Osk => "osk",
            WidgetKind::Power => "power",
            WidgetKind::Network => "network",
            WidgetKind::Bluetooth => "bluetooth",
            WidgetKind::Media => "media",
            WidgetKind::Launcher => "launcher",
            WidgetKind::Sysmon => "sysmon",
            WidgetKind::Status => "status",
            WidgetKind::Logo => "logo",
            WidgetKind::Calendar => "calendar",
        }
    }

    /// Hızlı ayar kartı mı (yalnızca panelde; bar karşılığı yok).
    pub fn is_quick_setting(self) -> bool {
        matches!(
            self,
            WidgetKind::Toggles
                | WidgetKind::Volume
                | WidgetKind::Mixer
                | WidgetKind::Brightness
                | WidgetKind::NightLight
                | WidgetKind::Magnifier
                | WidgetKind::Osk
                | WidgetKind::Power
                | WidgetKind::Network
                | WidgetKind::Bluetooth
                | WidgetKind::Media
        )
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.name() == name)
    }

    /// Ayarlarda görünen ad.
    pub fn title(self) -> String {
        match self {
            WidgetKind::Workspaces => t!("shell-bar-widget-workspaces-title"),
            WidgetKind::Clock => t!("shell-bar-widget-clock-title"),
            WidgetKind::ActiveWindow => t!("shell-bar-widget-active-window-title"),
            WidgetKind::NowPlaying => t!("shell-bar-widget-now-playing-title"),
            WidgetKind::Privacy => t!("shell-bar-widget-privacy-title"),
            WidgetKind::Caffeine => t!("shell-bar-widget-caffeine-title"),
            WidgetKind::KeyboardLayout => t!("shell-bar-widget-keyboard-layout-title"),
            WidgetKind::NextEvent => t!("shell-bar-widget-next-event-title"),
            WidgetKind::Tray => t!("shell-bar-widget-tray-title"),
            WidgetKind::Session => t!("shell-bar-widget-session-title"),
            WidgetKind::Notifications => t!("shell-bar-widget-notifications-title"),
            WidgetKind::PanelToggle => t!("shell-bar-widget-panel-toggle-title"),
            WidgetKind::Toggles => t!("shell-bar-widget-toggles-title"),
            WidgetKind::Volume => t!("shell-bar-widget-volume-title"),
            WidgetKind::Mixer => t!("shell-bar-widget-mixer-title"),
            WidgetKind::Brightness => t!("shell-bar-widget-brightness-title"),
            WidgetKind::NightLight => t!("shell-bar-widget-night-light-title"),
            WidgetKind::Magnifier => t!("shell-bar-widget-magnifier-title"),
            WidgetKind::Osk => t!("shell-bar-widget-osk-title"),
            WidgetKind::Power => t!("shell-bar-widget-power-title"),
            WidgetKind::Network => t!("shell-bar-widget-network-title"),
            WidgetKind::Bluetooth => t!("shell-bar-widget-bluetooth-title"),
            WidgetKind::Media => t!("shell-bar-widget-media-title"),
            WidgetKind::Launcher => t!("shell-bar-widget-launcher-title"),
            WidgetKind::Sysmon => t!("shell-bar-widget-sysmon-title"),
            WidgetKind::Status => t!("shell-bar-widget-status-title"),
            WidgetKind::Logo => t!("shell-bar-widget-logo-title"),
            WidgetKind::Calendar => t!("shell-bar-widget-calendar-title"),
        }
    }

    /// Ayarlarda widget'ı temsil eden ikon.
    pub fn icon(self) -> IconName {
        match self {
            WidgetKind::Workspaces => IconName::Grid,
            WidgetKind::Clock => IconName::Schedule,
            WidgetKind::ActiveWindow => IconName::Window,
            WidgetKind::NowPlaying => IconName::Music,
            WidgetKind::Privacy => IconName::Visibility,
            WidgetKind::Caffeine => IconName::Coffee,
            WidgetKind::KeyboardLayout => IconName::Keyboard,
            WidgetKind::NextEvent => IconName::Event,
            WidgetKind::Tray => IconName::MoreHoriz,
            WidgetKind::Session => IconName::Power,
            WidgetKind::Notifications => IconName::Notifications,
            WidgetKind::PanelToggle => IconName::Panel,
            WidgetKind::Toggles => IconName::Tune,
            WidgetKind::Volume => IconName::VolumeHigh,
            WidgetKind::Mixer => IconName::Tune,
            WidgetKind::Brightness => IconName::Brightness,
            WidgetKind::NightLight => IconName::NightLight,
            WidgetKind::Magnifier => IconName::Search,
            WidgetKind::Osk => IconName::Keyboard,
            WidgetKind::Power => IconName::Balanced,
            WidgetKind::Network => IconName::Wifi4,
            WidgetKind::Bluetooth => IconName::Bluetooth,
            WidgetKind::Media => IconName::Music,
            WidgetKind::Launcher => IconName::Apps,
            WidgetKind::Sysmon => IconName::Performance,
            WidgetKind::Status => IconName::Wifi4,
            WidgetKind::Logo => IconName::Apps,
            WidgetKind::Calendar => IconName::CalendarMonth,
        }
    }

    /// `[bar.widget.<ad>]` anahtarlarının şeması: varsayılanlar ayar yapısının `Default`'undan,
    /// sınırlar [`WidgetSettings::validate`]'in sabitlerinden.
    pub fn settings_schema(self, out: &mut Vec<SettingSchema>) {
        let d = WidgetSettings::default();
        let panel = |default: PanelTarget| {
            SettingSchema::choice("panel", t!("shell-bar-widget-opens-panel-label"), default.name(), PanelTarget::names())
                .labels(PanelTarget::ALL.map(PanelTarget::label).to_vec())
        };
        match self {
            WidgetKind::Workspaces => {
                out.push(SettingSchema::int(
                    "count",
                    t!("shell-bar-workspaces-count-label"),
                    d.workspaces.count.into(),
                    1,
                    MAX_WORKSPACE_COUNT.into(),
                ));
                let styles = WorkspaceStyle::ALL.map(WorkspaceStyle::name).to_vec();
                out.push(
                    SettingSchema::choice("style", t!("shell-bar-workspaces-style-label"), d.workspaces.style.name(), styles)
                        .labels(WorkspaceStyle::ALL.map(WorkspaceStyle::label).to_vec()),
                );
                let scrolls = WorkspaceScroll::ALL.map(WorkspaceScroll::name).to_vec();
                out.push(
                    SettingSchema::choice("scroll", t!("shell-bar-workspaces-scroll-label"), d.workspaces.scroll.name(), scrolls)
                        .labels(WorkspaceScroll::ALL.map(WorkspaceScroll::label).to_vec()),
                );
            }
            WidgetKind::Clock => {
                // Varsayılan "" = otomatik (`[locale] hour_cycle`); Ayarlar alanı boşaltınca
                // anahtarı siler ya da "" yazar, ikisi aynıdır.
                out.push(
                    SettingSchema::text("format", t!("shell-bar-clock-format-label"), d.clock.format.unwrap_or_default())
                        .describe(t!("shell-bar-clock-format-describe")),
                );
                out.push(
                    SettingSchema::text(
                        "vertical_format",
                        t!("shell-bar-clock-vertical-format-label"),
                        d.clock.vertical_format.unwrap_or_default(),
                    )
                    .describe(t!("shell-bar-clock-format-describe")),
                );
            }
            WidgetKind::ActiveWindow => {
                out.push(SettingSchema::int(
                    "max_chars",
                    t!("shell-bar-active-window-max-chars-label"),
                    d.active_window.max_chars.into(),
                    MIN_ACTIVE_WINDOW_CHARS.into(),
                    MAX_ACTIVE_WINDOW_CHARS.into(),
                ));
                out.push(
                    SettingSchema::float("max_width", t!("shell-bar-active-window-max-width-label"), d.active_window.max_width, MIN_ACTIVE_WINDOW_WIDTH, MAX_ACTIVE_WINDOW_WIDTH)
                        .step(10.0)
                        .unit("px"),
                );
            }
            WidgetKind::Tray => out.push(
                SettingSchema::float("icon_size", t!("shell-bar-tray-icon-size-label"), d.tray.icon_size, MIN_TRAY_ICON_SIZE, MAX_TRAY_ICON_SIZE).unit("px"),
            ),
            WidgetKind::Session => out.push(panel(d.session.panel)),
            WidgetKind::Notifications => {
                out.push(SettingSchema::bool("show_zero", t!("shell-bar-notifications-show-zero-label"), d.notifications.show_zero));
                out.push(panel(d.notifications.panel));
            }
            WidgetKind::PanelToggle => {
                out.push(panel(d.panel_toggle.panel));
                out.push(SettingSchema::text("icon", t!("shell-bar-panel-toggle-icon-label"), d.panel_toggle.icon));
            }
            // Hızlı ayar kartlarının ayarı yok.
            WidgetKind::Toggles
            | WidgetKind::Volume
            | WidgetKind::Mixer
            | WidgetKind::Brightness
            | WidgetKind::NightLight
            | WidgetKind::Magnifier
            | WidgetKind::Osk
            | WidgetKind::Power
            | WidgetKind::Network
            | WidgetKind::Bluetooth
            | WidgetKind::Media => {}
            // Takvim kartının ayarı yok (ilk gün ve adlar zaman locale'inden).
            WidgetKind::Calendar => {}
            // Başlatıcının ayarları `[launcher]` tablosunda.
            WidgetKind::Launcher | WidgetKind::Logo => {}
            // Bar'da hangi ölçümler görünsün; örnekleme ayarları `[sysmon]` tablosunda.
            WidgetKind::Sysmon => {
                let s = &d.sysmon;
                out.push(SettingSchema::bool("cpu", t!("shell-bar-sysmon-cpu-label"), s.cpu));
                out.push(SettingSchema::bool("memory", t!("shell-bar-sysmon-memory-label"), s.memory));
                out.push(SettingSchema::bool("gpu", t!("shell-bar-sysmon-gpu-label"), s.gpu));
                out.push(SettingSchema::bool("temperature", t!("shell-bar-sysmon-temperature-label"), s.temperature));
                out.push(SettingSchema::bool("network", t!("shell-bar-sysmon-network-label"), s.network));
                out.push(SettingSchema::bool("disk", t!("shell-bar-sysmon-disk-label"), s.disk));
            }
            WidgetKind::Status => {
                let s = &d.status;
                out.push(SettingSchema::bool("network", t!("shell-bar-status-network-label"), s.network));
                out.push(SettingSchema::bool("bluetooth", t!("shell-bar-status-bluetooth-label"), s.bluetooth));
                out.push(SettingSchema::bool("volume", t!("shell-bar-status-volume-label"), s.volume));
                out.push(SettingSchema::bool("battery", t!("shell-bar-status-battery-label"), s.battery));
                out.push(SettingSchema::bool("volume_percent", t!("shell-bar-status-volume-percent-label"), s.volume_percent));
                out.push(SettingSchema::bool("battery_percent", t!("shell-bar-status-battery-percent-label"), s.battery_percent));
                out.push(panel(s.panel));
            }
            WidgetKind::NowPlaying => {
                let s = &d.now_playing;
                out.push(
                    SettingSchema::float("width", t!("shell-bar-now-playing-width-label"), s.width, MIN_NOW_PLAYING_WIDTH, MAX_NOW_PLAYING_WIDTH)
                        .step(10.0)
                        .unit("px")
                        .describe(t!("shell-bar-now-playing-width-describe")),
                );
                let scrolls = MediaScroll::ALL.map(MediaScroll::name).to_vec();
                out.push(
                    SettingSchema::choice("scroll", t!("shell-bar-now-playing-scroll-label"), s.scroll.name(), scrolls)
                        .labels(MediaScroll::ALL.map(MediaScroll::label).to_vec()),
                );
                out.push(panel(s.panel));
            }
            WidgetKind::Caffeine => {
                out.push(SettingSchema::bool("show_when_off", t!("shell-bar-caffeine-show-when-off-label"), d.caffeine.show_when_off));
            }
            // Düzen listesi `[input] keyboard.layouts`'ta; göstergenin kendi ayarı yok.
            WidgetKind::KeyboardLayout => {}
            WidgetKind::NextEvent => out.push(
                SettingSchema::int(
                    "lead_minutes",
                    t!("shell-bar-next-event-lead-label"),
                    d.next_event.lead_minutes.into(),
                    MIN_NEXT_EVENT_LEAD.into(),
                    MAX_NEXT_EVENT_LEAD.into(),
                )
                .unit("min"),
            ),
            WidgetKind::Privacy => {
                let s = &d.privacy;
                out.push(SettingSchema::bool("microphone", t!("shell-bar-privacy-microphone-label"), s.microphone));
                out.push(SettingSchema::bool("camera", t!("shell-bar-privacy-camera-label"), s.camera));
                out.push(SettingSchema::bool("screen", t!("shell-bar-privacy-screen-label"), s.screen));
                out.push(panel(s.panel));
            }
        }
    }

    /// Bar'da: hızlı ayar kartları dışında hepsi. Panelde: yalnızca tam genişlik içerik
    /// sunanlar (oturum kartı, bildirim merkezi, hızlı ayar kartları, takvim kartı). Bar'daki
    /// `session` panel açan bir düğme, `notifications` sayaçtır; bar'daki takvim `clock`'un
    /// popout'udur.
    pub fn placement(self) -> Placement {
        match self {
            // Büyüteç: panelde kart, bar'da yalnız büyütme açıkken görünen hap.
            WidgetKind::Session | WidgetKind::Notifications | WidgetKind::Magnifier => Placement { bar: true, panel: true },
            k if k.is_quick_setting() => Placement { bar: false, panel: true },
            WidgetKind::Calendar => Placement { bar: false, panel: true },
            _ => Placement { bar: true, panel: false },
        }
    }

    pub fn names_for(place: fn(Placement) -> bool) -> String {
        Self::ALL.into_iter().filter(|k| place(k.placement())).map(Self::name).collect::<Vec<_>>().join(", ")
    }
}

// ---------------------------------------------------------------------------------------
// Ayar yapıları

pub const MAX_WORKSPACE_COUNT: u32 = 30;
pub const MAX_CLOCK_FORMAT_LEN: usize = 64;
pub const MIN_ACTIVE_WINDOW_WIDTH: f32 = 40.0;
pub const MAX_ACTIVE_WINDOW_WIDTH: f32 = 1200.0;
pub const MIN_ACTIVE_WINDOW_CHARS: u32 = 8;
pub const MAX_ACTIVE_WINDOW_CHARS: u32 = 200;
pub const MIN_TRAY_ICON_SIZE: f32 = 10.0;
pub const MAX_TRAY_ICON_SIZE: f32 = 64.0;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct WorkspacesSettings {
    /// Sayfa başına nokta (1..=[`MAX_WORKSPACE_COUNT`]); her zaman bu kadar yuva görünür.
    pub count: u32,
    /// Yuvada ne çizilir.
    pub style: WorkspaceStyle,
    /// Fare tekerleği / dokunmatik yüzey kaydırmasıyla geçiş.
    pub scroll: WorkspaceScroll,
}

impl Default for WorkspacesSettings {
    fn default() -> Self {
        Self { count: 10, style: WorkspaceStyle::default(), scroll: WorkspaceScroll::default() }
    }
}

/// Workspace yuvasının içeriği.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceStyle {
    /// Dolu workspace'te uygulama glifi (Nerd Font); bulunamazsa numara; boşta nokta.
    #[default]
    Icons,
    /// Her yuvada numara.
    Numbers,
    /// Yalnızca nokta (dolu büyük, boş küçük).
    Dots,
}

impl WorkspaceStyle {
    pub const ALL: [WorkspaceStyle; 3] = [WorkspaceStyle::Icons, WorkspaceStyle::Numbers, WorkspaceStyle::Dots];

    /// `shell.toml`'daki adı (serde adıyla aynı; şema testi doğrular).
    pub fn name(self) -> &'static str {
        match self {
            WorkspaceStyle::Icons => "icons",
            WorkspaceStyle::Numbers => "numbers",
            WorkspaceStyle::Dots => "dots",
        }
    }

    /// Ayarlarda seçeneğin görünen adı.
    pub fn label(self) -> String {
        match self {
            WorkspaceStyle::Icons => t!("shell-schema-workspace-style-icons"),
            WorkspaceStyle::Numbers => t!("shell-schema-workspace-style-numbers"),
            WorkspaceStyle::Dots => t!("shell-schema-workspace-style-dots"),
        }
    }
}

/// Kaydırmanın hedefi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceScroll {
    /// Bir sonraki/önceki id (boş dahil).
    #[default]
    All,
    /// Yalnızca dolu ya da bir monitörde görünen workspace'ler arasında.
    Occupied,
    Off,
}

impl WorkspaceScroll {
    pub const ALL: [WorkspaceScroll; 3] = [WorkspaceScroll::All, WorkspaceScroll::Occupied, WorkspaceScroll::Off];

    /// `shell.toml`'daki adı (serde adıyla aynı; şema testi doğrular).
    pub fn name(self) -> &'static str {
        match self {
            WorkspaceScroll::All => "all",
            WorkspaceScroll::Occupied => "occupied",
            WorkspaceScroll::Off => "off",
        }
    }

    /// Ayarlarda seçeneğin görünen adı.
    pub fn label(self) -> String {
        match self {
            WorkspaceScroll::All => t!("shell-schema-workspace-scroll-all"),
            WorkspaceScroll::Occupied => t!("shell-schema-workspace-scroll-occupied"),
            WorkspaceScroll::Off => t!("shell-schema-workspace-scroll-off"),
        }
    }
}

/// Saat biçimleri. `None` (anahtar yazılmamış ya da `""`) = otomatik: saat `framework.toml
/// [locale] hour_cycle`'a uyar ([`ClockFormat::auto`]); yazılmış strftime kalıbı her zaman
/// kazanır (t119, `docs/design/clock-locale.md`). "Yazılmış mı" ayrımı serde varsayılanında
/// kaybolmasın diye `Option`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct ClockSettings {
    /// Yatay bar'da strftime biçimi.
    pub format: Option<String>,
    /// Dikey bar'da (sol/sağ) biçim; `\n` satır böler.
    pub vertical_format: Option<String>,
}

impl ClockSettings {
    /// Bar yönüne göre kullanıcının yazdığı kalıp (`None` = otomatik).
    pub fn pattern(&self, vertical: bool) -> Option<&str> {
        if vertical { self.vertical_format.as_deref() } else { self.format.as_deref() }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ActiveWindowSettings {
    /// Başlıkta en fazla bu kadar grafem; uzunsa sonu "…" (tamamı ipucunda).
    pub max_chars: u32,
    /// Başlığın en geniş hâli (mantıksal piksel). Dikey bar'da gösterilmez.
    pub max_width: f32,
}

impl Default for ActiveWindowSettings {
    fn default() -> Self {
        Self { max_chars: 40, max_width: 360.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct TraySettings {
    /// İkon boyu (mantıksal piksel).
    pub icon_size: f32,
}

impl Default for TraySettings {
    fn default() -> Self {
        Self { icon_size: 18.0 }
    }
}

/// Hangi kenar paneli (bar düğmeleri için hedef).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelTarget {
    Left,
    #[default]
    Right,
    Top,
    Bottom,
}

impl PanelTarget {
    pub const ALL: [PanelTarget; 4] = [PanelTarget::Left, PanelTarget::Right, PanelTarget::Top, PanelTarget::Bottom];

    /// `shell.toml`'daki adı (serde adıyla aynı; şema testi doğrular).
    pub fn name(self) -> &'static str {
        match self {
            PanelTarget::Left => "left",
            PanelTarget::Right => "right",
            PanelTarget::Top => "top",
            PanelTarget::Bottom => "bottom",
        }
    }

    pub fn names() -> Vec<&'static str> {
        Self::ALL.map(Self::name).to_vec()
    }

    /// Ayarlarda seçeneğin görünen adı.
    pub fn label(self) -> String {
        match self {
            PanelTarget::Left => t!("shell-schema-panel-left"),
            PanelTarget::Right => t!("shell-schema-panel-right"),
            PanelTarget::Top => t!("shell-schema-panel-top"),
            PanelTarget::Bottom => t!("shell-schema-panel-bottom"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct SessionSettings {
    /// Bar düğmesinin açtığı panel.
    pub panel: PanelTarget,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct NotificationsSettings {
    /// Bar sayacı okunmamış yokken de görünsün mü.
    pub show_zero: bool,
    /// Bar sayacının açtığı panel.
    pub panel: PanelTarget,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct PanelToggleSettings {
    pub panel: PanelTarget,
    /// Semantik ikon adı (`nuxut::ui::IconName`, ör. `menu`); eski Material adları da çözülür.
    pub icon: String,
}

impl Default for PanelToggleSettings {
    fn default() -> Self {
        Self { panel: PanelTarget::Right, icon: "menu".into() }
    }
}

/// `[bar.widget.sysmon]`: bar'da görünen ölçümler (ipucu ve panel hepsini gösterir). Hiçbiri
/// seçili değilse yalnızca ikon görünür (panel yine açılır).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SysmonSettings {
    pub cpu: bool,
    pub memory: bool,
    pub gpu: bool,
    pub temperature: bool,
    pub network: bool,
    pub disk: bool,
}

impl Default for SysmonSettings {
    fn default() -> Self {
        Self { cpu: true, memory: true, gpu: false, temperature: false, network: false, disk: false }
    }
}

/// `[bar.widget.status]`: hangi göstergeler görünsün ve tıklayınca hangi panel açılsın. Veri
/// olmayan gösterge (pil yok, Bluetooth kapalı, servis yok) seçili olsa da gizlidir.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct StatusSettings {
    /// Wi-Fi gücü / kablolu / bağlantı yok.
    pub network: bool,
    /// Açık / bağlı (kapalıyken gizli).
    pub bluetooth: bool,
    /// Seviye / sessiz; üzerinde tekerlek sesi değiştirir.
    pub volume: bool,
    /// Yüzde / şarj (pil yoksa gizli).
    pub battery: bool,
    /// Ses ikonunun yanında yüzde.
    pub volume_percent: bool,
    /// Pil ikonunun yanında yüzde.
    pub battery_percent: bool,
    /// Tıklayınca açılan panel (hızlı ayar kartları varsayılan olarak sağda).
    pub panel: PanelTarget,
}

impl Default for StatusSettings {
    fn default() -> Self {
        Self {
            network: true,
            bluetooth: true,
            volume: true,
            battery: true,
            volume_percent: false,
            battery_percent: true,
            panel: PanelTarget::Right,
        }
    }
}

pub const MIN_NOW_PLAYING_WIDTH: f32 = 120.0;
pub const MAX_NOW_PLAYING_WIDTH: f32 = 600.0;

/// `[bar.widget.now_playing]`. Genişlik sabittir (yatay bar): başlık ya da duraklatma
/// değişince yerleşim kaymaz, yalnızca widget'ın dikdörtgeni yeniden çizilir (bar hasarı).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct NowPlayingSettings {
    /// Yatay bar'da widget genişliği (mantıksal piksel); metin "…" ile bu alana sığar.
    pub width: f32,
    /// Tekerleğin etkisi.
    pub scroll: MediaScroll,
    /// Tıklayınca açılan panel (medya kartı varsayılan olarak sağda).
    pub panel: PanelTarget,
}

impl Default for NowPlayingSettings {
    fn default() -> Self {
        Self { width: 240.0, scroll: MediaScroll::default(), panel: PanelTarget::Right }
    }
}

/// `[bar.widget.caffeine]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CaffeineSettings {
    /// Kapalıyken de görünsün (düğme); `false`: yalnız açıkken hatırlatıcı.
    pub show_when_off: bool,
}

impl Default for CaffeineSettings {
    fn default() -> Self {
        Self { show_when_off: true }
    }
}

/// `[bar.widget.next_event]`.
pub const MIN_NEXT_EVENT_LEAD: u32 = 5;
pub const MAX_NEXT_EVENT_LEAD: u32 = 720;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct NextEventSettings {
    /// Etkinlik başlamadan kaç dakika önce görünsün (süren etkinlik bitene dek görünür).
    pub lead_minutes: u32,
}

impl Default for NextEventSettings {
    fn default() -> Self {
        Self { lead_minutes: 60 }
    }
}

/// `[bar.widget.privacy]`: hangi kullanımlar gösterilsin. Kapatılan kaynak hiç izlenmez (kamera
/// kapalıysa kamera izleyicisi çalışmaz).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PrivacySettings {
    /// Bir uygulama mikrofondan kayıt alıyor (PipeWire kayıt akışı).
    pub microphone: bool,
    /// Bir süreç `/dev/video*` aygıtını açık tutuyor.
    pub camera: bool,
    /// Ekran sürekli yakalanıyor: paylaşım ya da kayıt (Hyprland `screencast`).
    pub screen: bool,
    /// Tıklayınca açılan panel (ses kartı varsayılan olarak sağda).
    pub panel: PanelTarget,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self { microphone: true, camera: true, screen: true, panel: PanelTarget::Right }
    }
}

/// "Şimdi çalıyor" üzerinde tekerlek.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaScroll {
    /// Sistem sesi ± `edge_actions.scroll_step` (durum göstergesinin ses ikonuyla aynı yol, OSD).
    /// Varsayılan: yanlışlıkla kaydırma geri alınabilir; parça atlamak alınamaz.
    #[default]
    Volume,
    /// Yukarı = sonraki, aşağı = önceki parça; hareket başına en fazla bir atlama.
    Track,
    Off,
}

impl MediaScroll {
    pub const ALL: [MediaScroll; 3] = [MediaScroll::Volume, MediaScroll::Track, MediaScroll::Off];

    /// `shell.toml`'daki adı (serde adıyla aynı; şema testi doğrular).
    pub fn name(self) -> &'static str {
        match self {
            MediaScroll::Volume => "volume",
            MediaScroll::Track => "track",
            MediaScroll::Off => "off",
        }
    }

    /// Ayarlarda seçeneğin görünen adı.
    pub fn label(self) -> String {
        match self {
            MediaScroll::Volume => t!("shell-schema-media-scroll-volume"),
            MediaScroll::Track => t!("shell-schema-media-scroll-track"),
            MediaScroll::Off => t!("shell-schema-media-scroll-off"),
        }
    }
}

/// Tüm widget'ların çözülmüş ayarları.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WidgetSettings {
    pub workspaces: WorkspacesSettings,
    pub clock: ClockSettings,
    pub active_window: ActiveWindowSettings,
    pub tray: TraySettings,
    pub session: SessionSettings,
    pub notifications: NotificationsSettings,
    pub panel_toggle: PanelToggleSettings,
    pub sysmon: SysmonSettings,
    pub status: StatusSettings,
    pub now_playing: NowPlayingSettings,
    pub privacy: PrivacySettings,
    pub caffeine: CaffeineSettings,
    pub next_event: NextEventSettings,
}

impl WidgetSettings {
    /// `[bar.widget.<ad>]` tablosunu ilgili ayara uygular. Uyarılar `key` ön ekiyle döner.
    pub fn apply(&mut self, kind: WidgetKind, table: &toml::Table, warnings: &mut Vec<String>) {
        let key = format!("bar.widget.{}", kind.name());
        match kind {
            WidgetKind::Workspaces => self.workspaces = parse_table(table, &key, warnings),
            WidgetKind::Clock => self.clock = parse_table(table, &key, warnings),
            WidgetKind::ActiveWindow => self.active_window = parse_table(table, &key, warnings),
            WidgetKind::Tray => self.tray = parse_table(table, &key, warnings),
            WidgetKind::Session => self.session = parse_table(table, &key, warnings),
            WidgetKind::Notifications => self.notifications = parse_table(table, &key, warnings),
            WidgetKind::PanelToggle => self.panel_toggle = parse_table(table, &key, warnings),
            WidgetKind::Sysmon => self.sysmon = parse_table(table, &key, warnings),
            WidgetKind::Status => self.status = parse_table(table, &key, warnings),
            WidgetKind::NowPlaying => self.now_playing = parse_table(table, &key, warnings),
            WidgetKind::Privacy => self.privacy = parse_table(table, &key, warnings),
            WidgetKind::Caffeine => self.caffeine = parse_table(table, &key, warnings),
            WidgetKind::NextEvent => self.next_event = parse_table(table, &key, warnings),
            WidgetKind::Toggles
            | WidgetKind::Volume
            | WidgetKind::Mixer
            | WidgetKind::Brightness
            | WidgetKind::NightLight
            | WidgetKind::Magnifier
            | WidgetKind::Osk
            | WidgetKind::Power
            | WidgetKind::Network
            | WidgetKind::Bluetooth
            | WidgetKind::Media
            | WidgetKind::KeyboardLayout
            | WidgetKind::Calendar => warnings.push(format!("[{key}]: bu widget'ın ayarı yok, yok sayıldı")),
            // Ayarı yok (başlatıcının kendi ayarları `[launcher]`): boş tablo da uyarı vermez.
            WidgetKind::Launcher | WidgetKind::Logo => {
                if !table.is_empty() {
                    warnings.push(format!("{key}: ayar yok ([launcher] tablosunu kullan), yok sayıldı"));
                }
            }
        }
    }

    /// Değerleri sınırlar; her düzeltme için uyarı.
    pub fn validate(&mut self, warnings: &mut Vec<String>) {
        let d = WidgetSettings::default();

        let ws = &mut self.workspaces;
        if !(1..=MAX_WORKSPACE_COUNT).contains(&ws.count) {
            let fixed = ws.count.clamp(1, MAX_WORKSPACE_COUNT);
            warnings.push(format!(
                "bar.widget.workspaces.count = {} geçersiz, {fixed} kullanılıyor (1..={MAX_WORKSPACE_COUNT})",
                ws.count
            ));
            ws.count = fixed;
        }

        validate_clock_format(&mut self.clock.format, "bar.widget.clock.format", warnings);
        validate_clock_format(&mut self.clock.vertical_format, "bar.widget.clock.vertical_format", warnings);

        let aw = &mut self.active_window;
        let (lo, hi) = (MIN_ACTIVE_WINDOW_CHARS, MAX_ACTIVE_WINDOW_CHARS);
        if !(lo..=hi).contains(&aw.max_chars) {
            let fixed = aw.max_chars.clamp(lo, hi);
            warnings.push(format!("bar.widget.active_window.max_chars = {} geçersiz, {fixed} kullanılıyor ({lo}..={hi})", aw.max_chars));
            aw.max_chars = fixed;
        }
        let (lo, hi) = (MIN_ACTIVE_WINDOW_WIDTH, MAX_ACTIVE_WINDOW_WIDTH);
        if !aw.max_width.is_finite() || !(lo..=hi).contains(&aw.max_width) {
            let fixed = if aw.max_width.is_finite() { aw.max_width.clamp(lo, hi) } else { d.active_window.max_width };
            warnings.push(format!("bar.widget.active_window.max_width geçersiz, {fixed} kullanılıyor ({lo}..={hi})"));
            aw.max_width = fixed;
        }

        let tray = &mut self.tray;
        let (lo, hi) = (MIN_TRAY_ICON_SIZE, MAX_TRAY_ICON_SIZE);
        if !tray.icon_size.is_finite() || !(lo..=hi).contains(&tray.icon_size) {
            let fixed = if tray.icon_size.is_finite() { tray.icon_size.clamp(lo, hi) } else { d.tray.icon_size };
            warnings.push(format!("bar.widget.tray.icon_size geçersiz, {fixed} kullanılıyor ({lo}..={hi})"));
            tray.icon_size = fixed;
        }

        let np = &mut self.now_playing;
        let (lo, hi) = (MIN_NOW_PLAYING_WIDTH, MAX_NOW_PLAYING_WIDTH);
        if !np.width.is_finite() || !(lo..=hi).contains(&np.width) {
            let fixed = if np.width.is_finite() { np.width.clamp(lo, hi) } else { d.now_playing.width };
            warnings.push(format!("bar.widget.now_playing.width geçersiz, {fixed} kullanılıyor ({lo}..={hi})"));
            np.width = fixed;
        }

        let ne = &mut self.next_event;
        let (lo, hi) = (MIN_NEXT_EVENT_LEAD, MAX_NEXT_EVENT_LEAD);
        if !(lo..=hi).contains(&ne.lead_minutes) {
            let fixed = ne.lead_minutes.clamp(lo, hi);
            warnings.push(format!("bar.widget.next_event.lead_minutes = {} geçersiz, {fixed} kullanılıyor ({lo}..={hi})", ne.lead_minutes));
            ne.lead_minutes = fixed;
        }

        let icon = &mut self.panel_toggle.icon;
        if icon.len() > 64 || icon.parse::<nuxut::ui::IconName>().is_err() {
            warnings.push(format!(
                "bar.widget.panel-toggle.icon bilinmeyen ikon adı (semantik ad, ör. \"menu\", \"settings\"), \"{}\" kullanılıyor",
                d.panel_toggle.icon
            ));
            *icon = d.panel_toggle.icon;
        }
    }
}

/// Yazılmış saat kalıbını doğrular. Boş (ya da yalnız boşluk) kalıp = otomatik (`None`,
/// uyarısız: Ayarlar alanı boşaltınca `""` yazabilir); geçersiz kalıp uyarı + otomatik.
fn validate_clock_format(slot: &mut Option<String>, key: &str, warnings: &mut Vec<String>) {
    let Some(format) = slot.as_deref() else { return };
    if format.trim().is_empty() {
        *slot = None;
        return;
    }
    let reason = if format.chars().count() > MAX_CLOCK_FORMAT_LEN {
        Some("çok uzun")
    } else if format.chars().any(|c| c.is_control() && c != '\n') {
        Some("kontrol karakteri içeriyor")
    } else if ClockFormat::parse(format).is_err() {
        Some("geçersiz strftime belirteci")
    } else {
        None
    };
    if let Some(reason) = reason {
        warnings.push(format!(
            "{key} \"{}\" geçersiz ({reason}, en fazla {MAX_CLOCK_FORMAT_LEN} karakter), otomatik biçim ([locale] hour_cycle) kullanılıyor",
            crate::config::sanitize(format),
        ));
        *slot = None;
    }
}

/// Tabloyu ayar yapısına çevirir. Bilinmeyen anahtar uyarı; tip hatası tabloyu yok sayar
/// (varsayılanlar) ve uyarır.
fn parse_table<T: DeserializeOwned + Default>(table: &toml::Table, key: &str, warnings: &mut Vec<String>) -> T {
    let text = match toml::to_string(table) {
        Ok(t) => t,
        Err(e) => {
            warnings.push(format!("{key}: okunamadı ({e}), varsayılanlar kullanılıyor"));
            return T::default();
        }
    };
    match nuxut_config::parse_str::<T>(&text) {
        Ok((value, unknown)) => {
            warnings.extend(unknown.into_iter().map(|w| format!("{key}: {w}")));
            value
        }
        Err(e) => {
            warnings.push(format!("{key}: geçersiz ({}), varsayılanlar kullanılıyor", e.lines().next().unwrap_or("")));
            T::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        text.parse().unwrap()
    }

    #[test]
    fn names_round_trip_and_are_unique() {
        for k in WidgetKind::ALL {
            assert_eq!(WidgetKind::from_name(k.name()), Some(k));
        }
        let mut names: Vec<_> = WidgetKind::ALL.iter().map(|k| k.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), WidgetKind::ALL.len());
    }

    #[test]
    fn panel_placement_is_limited() {
        assert_eq!(
            WidgetKind::names_for(|p| p.panel),
            "session, notifications, toggles, volume, mixer, brightness, night_light, magnifier, osk, power, network, bluetooth, media, calendar"
        );
        assert_eq!(
            WidgetKind::names_for(|p| p.panel && !p.bar),
            "toggles, volume, mixer, brightness, night_light, osk, power, network, bluetooth, media, calendar"
        );
        assert!(
            WidgetKind::ALL
                .iter()
                // Takvim yalnız panel ama hızlı ayar değil; büyüteç hızlı ayar ama bar hapı da var.
                .filter(|k| !matches!(k, WidgetKind::Calendar | WidgetKind::Magnifier))
                .all(|k| k.placement().bar != k.is_quick_setting())
        );
        assert!(!WidgetKind::Calendar.is_quick_setting());
    }

    #[test]
    fn applies_and_validates_settings() {
        let mut s = WidgetSettings::default();
        let mut w = Vec::new();
        s.apply(WidgetKind::Clock, &table("format = \"%H:%M:%S\"\nvertical_format = \"%H\\n%M\\n%S\""), &mut w);
        s.apply(WidgetKind::Workspaces, &table("count = 99\nstyle = \"dots\"\nscroll = \"occupied\"\nfoo = 1"), &mut w);
        s.apply(WidgetKind::PanelToggle, &table("panel = \"left\"\nicon = \"bad icon!\""), &mut w);
        s.validate(&mut w);
        assert_eq!(s.clock.format.as_deref(), Some("%H:%M:%S"));
        assert_eq!(s.clock.vertical_format.as_deref(), Some("%H\n%M\n%S"));
        assert_eq!(s.workspaces.count, MAX_WORKSPACE_COUNT);
        assert_eq!((s.workspaces.style, s.workspaces.scroll), (WorkspaceStyle::Dots, WorkspaceScroll::Occupied));
        assert_eq!(s.panel_toggle.panel, PanelTarget::Left);
        assert_eq!(s.panel_toggle.icon, "menu");
        let mut ok = WidgetSettings::default();
        ok.apply(WidgetKind::PanelToggle, &table("icon = \"settings\""), &mut w);
        let before = w.len();
        ok.validate(&mut w);
        assert_eq!((ok.panel_toggle.icon.as_str(), w.len()), ("settings", before), "semantik ad geçerli");
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w[0].contains("bar.widget.workspaces") && w[0].contains("foo"), "{w:?}");
    }

    #[test]
    fn active_window_max_chars_is_bounded() {
        for (written, expected) in [(3, MIN_ACTIVE_WINDOW_CHARS), (40, 40), (5000, MAX_ACTIVE_WINDOW_CHARS)] {
            let mut s = WidgetSettings::default();
            let mut w = Vec::new();
            s.apply(WidgetKind::ActiveWindow, &table(&format!("max_chars = {written}")), &mut w);
            s.validate(&mut w);
            assert_eq!(s.active_window.max_chars, expected);
            assert_eq!(w.len(), usize::from(written != expected), "{w:?}");
        }
        // Negatif değer tip hatası: tablo varsayılana döner.
        let mut s = WidgetSettings::default();
        let mut w = Vec::new();
        s.apply(WidgetKind::ActiveWindow, &table("max_chars = -1"), &mut w);
        assert_eq!(s.active_window, ActiveWindowSettings::default());
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn status_settings_parse() {
        let mut s = WidgetSettings::default();
        let mut w = Vec::new();
        s.apply(WidgetKind::Status, &table("bluetooth = false\nvolume_percent = true\npanel = \"left\"\nwifi = true"), &mut w);
        s.validate(&mut w);
        let st = &s.status;
        assert!(!st.bluetooth && st.volume_percent && st.network && st.battery);
        assert_eq!(st.panel, PanelTarget::Left);
        assert_eq!(w.len(), 1, "bilinmeyen anahtar uyarısı: {w:?}");
        assert!(w[0].contains("bar.widget.status") && w[0].contains("wifi"), "{w:?}");
        assert!(WidgetKind::Status.placement().bar && !WidgetKind::Status.placement().panel);
        let mut schema = Vec::new();
        WidgetKind::Status.settings_schema(&mut schema);
        assert_eq!(schema.len(), 7);
    }

    #[test]
    fn now_playing_settings_parse_and_clamp() {
        let mut s = WidgetSettings::default();
        let mut w = Vec::new();
        s.apply(WidgetKind::NowPlaying, &table("width = 5000.0\nscroll = \"track\"\npanel = \"left\""), &mut w);
        s.validate(&mut w);
        let np = &s.now_playing;
        assert_eq!((np.width, np.scroll, np.panel), (MAX_NOW_PLAYING_WIDTH, MediaScroll::Track, PanelTarget::Left));
        assert_eq!(w.len(), 1, "{w:?}");
        let p = WidgetKind::NowPlaying.placement();
        assert!(p.bar && !p.panel);
        let mut schema = Vec::new();
        WidgetKind::NowPlaying.settings_schema(&mut schema);
        assert_eq!(schema.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), ["width", "scroll", "panel"]);
    }

    #[test]
    fn type_error_falls_back_to_defaults() {
        let mut s = WidgetSettings::default();
        let mut w = Vec::new();
        s.apply(WidgetKind::Workspaces, &table("count = \"ten\""), &mut w);
        assert_eq!(s.workspaces, WorkspacesSettings::default());
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn bad_clock_format_falls_back() {
        let mut s = WidgetSettings::default();
        let mut w = Vec::new();
        s.apply(WidgetKind::Clock, &table("format = \"%Q\""), &mut w);
        s.validate(&mut w);
        assert_eq!(s.clock.format, None, "geçersiz kalıp → otomatik");
        assert!(w[0].contains("strftime"), "{w:?}");
        // Boş kalıp otomatiktir, uyarı yok; yazılmamış da otomatik.
        let mut s = WidgetSettings::default();
        let mut w = Vec::new();
        s.apply(WidgetKind::Clock, &table("format = \"\"\nvertical_format = \" \""), &mut w);
        s.validate(&mut w);
        assert_eq!(s.clock, ClockSettings::default());
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(ClockSettings::default().pattern(false), None);
    }
}
