//! Bar ve kenar paneli widget'ları. Kayıt (adlar, yerleşim, ayarlar): [`registry`];
//! çalışma zamanı nesneleri: [`BarWidget`] ve [`create`].
//!
//! Widget'lar yalnızca kendi dikdörtgenlerini bilir; yerleşimi (bar ekseni, bölümler)
//! `Screen` yapar. Bar ekseni: yatay bar'da x, dikey bar'da y; `measure` bu eksen boyunca
//! uzunluk döndürür (0 = gizli).

pub mod registry;

mod active_window;
mod calendar_card;
mod clock;
mod icon_button;
pub(crate) mod logo;
mod now_playing;
mod caffeine;
mod privacy;
mod notification_cards;
pub mod notification_popups;
pub mod notifications_panel;
mod quick;
mod keyboard_layout;
mod magnifier_pill;
mod next_event;
pub(crate) use quick::magnifier as quick_magnifier;
pub(crate) use next_event::near_range as next_event_range;
mod pill;
mod session_card;
mod status;
mod sysmon;
mod tray;
pub(crate) mod workspaces;

use std::fmt;
use std::time::Instant;

use chrono::{DateTime, Local};
use nuxut::ui::Ui;
use nuxut::{BlobShape, CursorShape, KeyEvent, Painter, Point, PointerEvent, Rect, TextSystem, Theme};

pub use registry::{PanelTarget, Placement, WidgetKind, WidgetSettings};
pub use notification_cards::relative_time;

use crate::layout::Edge;
use crate::model::ShellModel;

/// Açılıp kapanan yapraklar. Açık/sabit durumu: [`crate::panels::PanelState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    /// Saatin altındaki tarih popout'u.
    Clock,
    /// Kenar çekmecesi.
    Edge(Edge),
    /// Uygulama başlatıcısı (bar kenarının ortasından büyür).
    Launcher,
    /// Sistem izleyicisi ayrıntı popout'u (bar widget'ının hizasında).
    Sysmon,
}

impl From<PanelTarget> for PanelId {
    fn from(t: PanelTarget) -> Self {
        PanelId::Edge(match t {
            PanelTarget::Left => Edge::Left,
            PanelTarget::Right => Edge::Right,
            PanelTarget::Top => Edge::Top,
            PanelTarget::Bottom => Edge::Bottom,
        })
    }
}

/// Widget'tan çıkan kullanıcı eylemleri; yan etkileri (IPC, D-Bus, yüzey durumu) `App`
/// uygular. Widget'lar servis tutamağı tutmaz, yalnızca eylem döndürür.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    SwitchWorkspace(i64),
    TogglePanel(PanelId),
    ClosePanel(PanelId),
    /// Kenar panelini sabitle / sabitlemeyi kaldır (pin düğmesi, `msg panel pin`).
    PinPanel(Edge, bool),
    /// Overlay (menü, popover) açıldı/kapandı ya da boyutu değişti: input bölgesi ve klavye
    /// modu yeniden hesaplansın.
    RefreshInput,
    /// Takvim bu aralığı gösteriyor (ay değişti / popout açıldı): servis sorgulasın.
    CalendarRange { from: i64, to: i64 },
    Notify(NotifyCommand),
    Tray(TrayCommand),
    Session(crate::services::session::SessionAction),
    /// Hızlı ayar kartları ve OSD komutları (`App` → `SystemServices`).
    System(crate::system::SystemCommand),
    /// Gece ışığı kartı (`App` → `services::night_light`).
    NightLight(crate::services::night_light::Command),
    /// Büyüteç kartı (`App` → `services::magnifier`).
    Magnifier(crate::services::magnifier::Command),
    /// Uyanık tut aç/kapa (hızlı anahtar, bar ikonu; `App` → boşta kalma makinesi, `msg idle` ile aynı yol).
    IdleInhibit(bool),
    /// Klavye düzeni göstergesi: ana klavyede sıradaki düzen (Hyprland `switchxkblayout`).
    KeyboardLayoutNext,
    /// Koyu/açık kip (hızlı anahtar): `framework.toml [colors] mode` yazılır, tema dosya izleyicisinden gelir.
    SetThemeMode(nuxut::theme::ThemeMode),
    /// Ekran klavyesi kartı (`App` → `app/osk.rs`).
    Osk(crate::services::osk::Command),
    /// Bar uç eylemi (`[bar.edge_actions]`): ses/mikrofon/parlaklık/workspace.
    BarEdge(crate::bar_edge::EdgeCommand),
    /// Odaklı metin alanı Ctrl+V istedi: `App` panoyu okur, sonucu [`PanelWidget::paste`] ile verir.
    RequestPaste,
    /// Başlatıcıdan: uygulamayı (ya da masaüstü eylemini) başlat.
    Launch { entry: std::sync::Arc<crate::services::launcher::DesktopEntry>, action: Option<String> },
    /// Metin girdisinde kopyala/kes: `App` panoya yazar (`cx.set_clipboard`).
    Copy(CopyText),
    /// Pano geçmişi (başlatıcının pano kipi): öğeyi panoya koy, sil, geçmişi temizle.
    Clipboard(crate::services::clipboard::ClipboardCommand),
    /// Başlatıcının emoji kipi: emojiyi panoya koy, sık kullanılanlara kaydet, başlatıcıyı kapat.
    PickEmoji(&'static str),
}

/// Bildirim eylemleri (`App` → `NotificationCenter` + `NotifyHandle`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyCommand {
    /// X düğmesi: merkezden kaldır, istemciye `Dismissed`.
    Dismiss(u32),
    /// Kaydırarak kapatma: yalnızca popup gizlenir, merkezde kalır.
    HidePopup(u32),
    /// Eylem düğmesi ya da kart tıklaması (`"default"`).
    Invoke { id: u32, key: String },
    ClearAll,
    MarkAllRead,
    SetDnd(bool),
    /// Popup üzerinde: süre durur.
    Hover { id: u32, hovered: bool },
    /// Gövdedeki doğrulanmış bağlantı (`BodyLink::url`); portal ile açılır.
    OpenLink(String),
}

/// Panoya yazılacak metin (kopyala/kes). Bırakılınca bellekten silinir (`Zeroizing`); Debug
/// içeriği göstermez (yalnızca uzunluk).
#[derive(Clone, PartialEq, Eq)]
pub struct CopyText(zeroize::Zeroizing<String>);

impl CopyText {
    pub fn new(text: String) -> Self {
        Self(zeroize::Zeroizing::new(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for CopyText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CopyText(<{} bayt>)", self.0.len())
    }
}

/// Tepsi eylemleri (`App` → `TrayHandle`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCommand {
    Activate { item: crate::services::tray::ItemId, x: i32, y: i32 },
    SecondaryActivate { item: crate::services::tray::ItemId, x: i32, y: i32 },
    Scroll { item: crate::services::tray::ItemId, delta: i32, horizontal: bool },
    /// Menüyü `request` kimliğiyle aç (`services::tray::next_menu_request`).
    OpenMenu { item: crate::services::tray::ItemId, request: crate::services::tray::MenuRequest },
    CloseMenu(crate::services::tray::ItemId),
    MenuAboutToShow { item: crate::services::tray::ItemId, ids: Vec<i32> },
    MenuClicked { item: crate::services::tray::ItemId, id: i32 },
    /// dbusmenu sunmayan öğede sağ tık: uygulamanın kendi bağlam menüsü (`ContextMenu`).
    ContextMenu { item: crate::services::tray::ItemId, x: i32, y: i32 },
}

impl TrayCommand {
    /// Bar'dan bir menü açıyor mu (shell'in çizdiği dbusmenu ya da uygulamanın kendi bağlam
    /// menüsü). "Tek geçici yüzey" kuralı: açılırken geçici yaprak kapanır
    /// (`docs/design/panels.md` §5).
    pub fn opens_menu(&self) -> bool {
        matches!(self, Self::OpenMenu { .. } | Self::ContextMenu { .. })
    }
}

/// Widget'ların ortak bağlamı (tema, ayarlar, bar yönü, öğe boyu).
#[derive(Debug, Clone, Copy)]
pub struct WidgetCx<'a> {
    pub theme: &'a Theme,
    pub settings: &'a WidgetSettings,
    /// Bar dikey mi (sol/sağ kenar)?
    pub vertical: bool,
    /// Kare öğe kenarı (workspace yuvası, ikon düğmesi): bar kalınlığından türetilir.
    pub item: f32,
    pub now: Instant,
    /// Bar overlay'lerinin (tepsi menüsü, ipucu) sığacağı alan (yüzey koordinatı): çerçevenin iç
    /// dikdörtgeni eksi görünen sabit paneller (`panels::free_area`); menü sabit panelle
    /// çakışmaz, onun dışına kaydırılır/yönü çevrilir.
    pub bounds: Rect,
}

impl WidgetCx<'_> {
    /// Bar kalınlığından öğe boyu: her iki yanda 8 px nefes payı.
    pub fn item_for(theme: &Theme, bar_size: f32) -> f32 {
        (bar_size - theme.space(2) * 2.0).clamp(12.0, bar_size)
    }
}

/// İpucu balonunun içeriği: metin ve isteğe bağlı çapa dikdörtgeni. Çapa, balonun hangi
/// alanın altında/yanında duracağını belirler; `None` ise widget'ın tüm dikdörtgeni kullanılır.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarTooltip<'a> {
    pub text: &'a str,
    pub anchor: Option<Rect>,
}

pub trait BarWidget: fmt::Debug {
    fn kind(&self) -> WidgetKind;

    /// Shell modeli değişti (Hyprland, bildirimler…). Görünüm değiştiyse `true`.
    fn sync(&mut self, _model: &ShellModel, _monitor: &str, _cx: &WidgetCx) -> bool {
        false
    }

    /// Saat ilerledi. Görünüm değiştiyse `true`.
    fn set_time(&mut self, _now: &DateTime<Local>, _cx: &WidgetCx) -> bool {
        false
    }

    /// Tema veya widget ayarları değişti.
    fn set_theme(&mut self, cx: &WidgetCx);

    /// Bar ekseni boyunca uzunluk; 0 = gizli.
    fn measure(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> f32;

    /// `highlighted`: widget'ın açtığı yaprak açık.
    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, highlighted: bool);

    /// İşaretçi konumu/basılı durumu (hover state layer). Yeniden çizim gerekiyorsa `true`.
    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool;

    /// Sol tık bırakıldı (`at` widget dikdörtgeninde).
    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action>;

    /// Widget'ın açtığı yaprak (vurgulama için).
    fn panel(&self, _cx: &WidgetCx) -> Option<PanelId> {
        None
    }

    fn is_animating(&mut self, now: Instant) -> bool;

    /// İmleç (DESIGN "İmleç"): tıklanabilir yerde `Pointer`, değilse `None` (alttaki karar verir).
    fn cursor(&self, _rect: Rect, _p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        None
    }

    /// İpucu balonu (`p` widget dikdörtgeninde): ör. kırpılmış başlığın tamamı. Balonu
    /// framework'ün genel ipucu bileşeni çizer; bağlantı noktası `Screen::pointer`'daki bar
    /// hover adımıdır (bileşen gelince orada `hover` ile birlikte sorulur). `None` = ipucu yok.
    /// `anchor` yoksa balon widget'ın tüm dikdörtgenine göre konumlanır; yuva gibi bir alt
    /// bölge varsa o bölgenin rect'ini vermek balonu yalnızca o yuvanın altına koyar.
    fn tooltip(&self, _rect: Rect, _p: Point, _cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        None
    }

    /// Overlay üzerindeyken imleç (açık menü).
    fn overlay_cursor(&self, _p: Point) -> Option<CursorShape> {
        None
    }

    // --- Overlay: bar dışına taşan içerik (tepsi menüsü, popover). Varsayılan: yok.

    /// Overlay'in bu karedeki blob şekilleri (shell kendi alanına ekler; karede bir kez).
    fn overlay_shapes(&mut self, _cx: &WidgetCx) -> Vec<BlobShape> {
        Vec::new()
    }

    /// Overlay içeriği (şekiller çizildikten sonra, bar içeriğinin üstüne).
    fn draw_overlay(&mut self, _ui: &mut Ui, _cx: &WidgetCx) {}

    /// Overlay'in tıklama alacağı dikdörtgenler (input bölgesine eklenir).
    fn overlay_input(&self) -> Vec<Rect> {
        Vec::new()
    }

    /// Overlay işaretçi olayını tüketir mi? `Some((yeniden çiz, eylem))` = tüketildi; bar
    /// isabet testine düşmez. `text`: menü ölçümü için.
    fn overlay_pointer(
        &mut self,
        _event: &PointerEvent,
        _text: &mut TextSystem,
        _cx: &WidgetCx,
    ) -> Option<(bool, Option<Action>)> {
        None
    }

    /// Overlay klavye istiyor mu (açık menü: ok tuşları, Enter, Escape).
    fn wants_keyboard(&self) -> bool {
        false
    }

    /// Klavye olayı; tüketildiyse `Some`.
    fn key(&mut self, _event: &KeyEvent, _text: &mut TextSystem, _cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        None
    }

    /// Dışarıdan kapatma (Escape, odak kaybı, başka yaprak açıldı). Bir şey kapandıysa `true`.
    fn close_overlay(&mut self, _now: Instant) -> bool {
        false
    }

    /// Ekran okuyucu düğümleri (ADR-0011): widget'ın tıklanabilir bölgeleri. Varsayılan tek
    /// düğüm: ad = tür adı (katalog), bölge = tüm widget. AT tıklaması bölgenin ortasına
    /// işaretçi tıklaması olarak gelir ([`BarWidget::click`] yolu). Birden çok bölge (çalışma
    /// alanları) ya da görünen değer (saat) olan widget bunu ezer.
    fn access_items(&self, rect: Rect, cx: &WidgetCx) -> Vec<AccessItem> {
        // Değer: widget'ın kendi ipucu (gösterge metni: "Uyanık tut açık…", "Ekip toplantısı
        // 14:00–14:45"); satırlar virgülle. İpucu ekranda görünen bilginin özetidir.
        let value = self.tooltip(rect, rect.center(), cx).map(|t| t.text.lines().collect::<Vec<_>>().join(", ")).unwrap_or_default();
        vec![AccessItem::new("widget", self.kind().title(), rect).with_value(value)]
    }
}

/// Panel kartının ekran okuyucu grubu (ADR-0011): `Role::Group`, ad kartın türü ("Ses"),
/// anahtar tür adı (panelde her tür bir kez). Kart içeriğini `f` içinde ekler.
pub fn access_card(tree: &mut nuxut::access::AccessTree, kind: WidgetKind, rect: Rect, f: impl FnOnce(&mut nuxut::access::AccessTree)) {
    let id = tree.push(kind.name(), nuxut::access::Role::Group).label(&kind.title()).bounds(rect).id();
    tree.within(id, f);
}

/// [`access_card`] grubunun kimliği (`parent`: panel düğümü).
pub fn access_card_id(parent: nuxut::access::NodeId, kind: WidgetKind) -> nuxut::access::NodeId {
    nuxut::access::child_id(parent, kind.name())
}

/// Bar widget'ının erişilebilir bir bölgesi (ADR-0011). `Debug` redakte: ad/değer kullanıcı içeriği
/// (pencere başlığı) taşıyabilir (değişmez 9).
pub struct AccessItem {
    /// Widget içinde kararlı anahtar (ör. çalışma alanı kimliği); indis değil.
    pub key: String,
    /// Erişilebilir ad (katalogdan).
    pub label: String,
    /// Görünen değer (saat metni, pencere başlığı); boş = yok.
    pub value: String,
    pub rect: Rect,
    /// Seçili durum (etkin çalışma alanı); `None` = seçilebilir değil.
    pub selected: Option<bool>,
}

// Değişmez 9: ad/değer içerik taşıyabilir; yalnız uzunluk.
impl fmt::Debug for AccessItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessItem")
            .field("key", &self.key.len())
            .field("label", &self.label.len())
            .field("value", &self.value.len())
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl AccessItem {
    pub fn new(key: impl Into<String>, label: impl Into<String>, rect: Rect) -> Self {
        Self { key: key.into(), label: label.into(), value: String::new(), rect, selected: None }
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }
}

/// Kenar paneli (çekmece) içeriği. Panel widget'ları dikey olarak üst üste dizilir; yüksekliği
/// `None` olan widget kalan alanın tamamını alır (bildirim merkezi). Çizim `nuxut_ui::Ui` ile.
pub trait PanelWidget: fmt::Debug + AsAny {
    fn kind(&self) -> WidgetKind;

    /// Shell modeli değişti. Görünüm değiştiyse `true`.
    fn sync(&mut self, _model: &ShellModel, _cx: &WidgetCx) -> bool {
        false
    }

    /// Saat ilerledi (göreli zaman damgaları, uptime). Değiştiyse `true`.
    fn set_time(&mut self, _now: &DateTime<Local>, _cx: &WidgetCx) -> bool {
        false
    }

    fn set_theme(&mut self, cx: &WidgetCx);

    /// Verilen genişlikte istenen yükseklik; `None` = esnek (kalan alan).
    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32>;

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx);

    /// İşaretçi olayı (dikdörtgen içindeyse ya da sürükleme sürüyorsa). Döner: (yeniden çiz, eylem).
    fn pointer(&mut self, event: &PointerEvent, rect: Rect, text: &mut TextSystem, cx: &WidgetCx)
    -> (bool, Option<Action>);

    fn key(&mut self, _event: &KeyEvent, _cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        None
    }

    /// [`Action::RequestPaste`] sonucu: pano metni (kullanıcı verisi; loglanmaz, saklanmaz).
    /// Tüketildiyse `Some`.
    fn paste(&mut self, _text: &str, _cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        None
    }

    /// Panel açıldı (çekmece büyümeye başladı). Dönen eylemi App uygular (ör. okundu).
    fn panel_opened(&mut self, _now: Instant) -> Option<Action> {
        None
    }

    /// Panel kapandı: geçici durumları (onay bekleyen düğme, açık menü) sıfırla.
    fn panel_closed(&mut self, _now: Instant) {}

    /// İmleç (DESIGN "İmleç"); `None` = panel zemini (`Default`).
    fn cursor(&self, _rect: Rect, _p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        None
    }

    /// Dikdörtgenin köşelerinden içeriğe (metin, ikon, etkileşimli öğe) yatay/dikey boşluk.
    /// Panelin sabitleme düğmesi köşede içeriğin üstünde yüzer; bu boşluğa sığmazsa panel
    /// içeriği o köşede itilir (`panels::pin_clearance_shift`). Varsayılan sıfır: bilinmeyen
    /// içerik düğmenin altına hiç girmez.
    fn corner_clearance(&self, _cx: &WidgetCx) -> nuxut::Size {
        nuxut::Size::ZERO
    }

    /// Takvim gösteren widget'ın şu an gösterdiği sorgu aralığı `[from, to)` (Unix sn). `App`
    /// görünür tüm takvimlerin aralıklarını birleştirip servise tek sorgu verir
    /// ([`crate::calendar_view::query_range`]). Varsayılan: takvim yok.
    fn calendar_range(&self) -> Option<(i64, i64)> {
        None
    }

    fn is_animating(&mut self, now: Instant) -> bool;

    /// Ekran okuyucu düğümleri (framework ADR-0011): kart kendi alt ağacını framework
    /// bileşenlerinin `access` yöntemleriyle `tree`'nin şu anki ebeveyni altına ekler (`rect`: son
    /// çizimin kutusu). AT tıklaması düğümün ortasına işaretçi tıklaması olarak gelir; ayrı eylem
    /// kodu gerekmez. Varsayılan: kart ağaçta yok (henüz benimsenmedi).
    fn access(&self, _tree: &mut nuxut::access::AccessTree, _rect: Rect, _cx: &WidgetCx) {}

    /// Tıklama dışındaki AT eylemi (kaydırıcı adımı ya da değeri, bildirim düğmeleri):
    /// `parent` kartın [`PanelWidget::access`] çağrısındaki ebeveyn düğüm. Kartın düğümüyse
    /// işaretçi/klavye ile aynı sonuç: (yeniden çiz, eylem). Tıklama işaretçi yolundan gelir.
    fn access_action(
        &mut self,
        _request: &nuxut::access::AccessRequest,
        _parent: nuxut::access::NodeId,
        _rect: Rect,
        _cx: &WidgetCx,
    ) -> Option<(bool, Option<Action>)> {
        None
    }
}

/// Bar widget'ı üretir; yalnızca kayıtta `placement().bar` olan türler. Yeni widget: buraya bir kol.
pub fn create(kind: WidgetKind, cx: &WidgetCx) -> Option<Box<dyn BarWidget>> {
    Some(match kind {
        WidgetKind::Workspaces => Box::new(workspaces::Workspaces::new(cx)),
        WidgetKind::Clock => Box::new(clock::Clock::new(cx)),
        WidgetKind::ActiveWindow => Box::new(active_window::ActiveWindow::new(cx)),
        WidgetKind::Tray => Box::new(tray::Tray::new(cx)),
        WidgetKind::Sysmon => Box::new(sysmon::Sysmon::new(cx)),
        WidgetKind::Status => Box::new(status::Status::new(cx)),
        WidgetKind::NowPlaying => Box::new(now_playing::NowPlaying::new(cx)),
        WidgetKind::Privacy => Box::new(privacy::Privacy::new(cx)),
        WidgetKind::Caffeine => Box::new(caffeine::Caffeine::new(cx)),
        WidgetKind::Magnifier => Box::new(magnifier_pill::MagnifierPill::new(cx)),
        WidgetKind::KeyboardLayout => Box::new(keyboard_layout::KeyboardLayout::new(cx)),
        WidgetKind::NextEvent => Box::new(next_event::NextEvent::new(cx)),
        WidgetKind::Logo => Box::new(logo::Logo::new(cx)),
        WidgetKind::Session | WidgetKind::Notifications | WidgetKind::PanelToggle | WidgetKind::Launcher => {
            Box::new(icon_button::IconButton::new(kind, cx))
        }
        _ => return None,
    })
}

/// Kenar paneli widget'ı üretir; yalnızca kayıtta `placement().panel` olan türler.
pub fn create_panel(kind: WidgetKind, cx: &WidgetCx) -> Option<Box<dyn PanelWidget>> {
    Some(match kind {
        WidgetKind::Session => Box::new(session_card::SessionCard::new(cx)),
        WidgetKind::Notifications => Box::new(notifications_panel::NotificationsPanel::new(cx)),
        WidgetKind::Toggles => Box::new(quick::TogglesCard::new(cx)),
        WidgetKind::Volume => Box::new(quick::Card(quick::VolumeCard::new(cx))),
        WidgetKind::Mixer => Box::new(quick::Card(quick::MixerCard::new(cx))),
        WidgetKind::Brightness => Box::new(quick::BrightnessCard::new(cx)),
        WidgetKind::NightLight => Box::new(quick::Card(quick::NightLightCard::new(cx))),
        WidgetKind::Magnifier => Box::new(quick::Card(quick::MagnifierCard::new(cx))),
        WidgetKind::Osk => Box::new(quick::Card(quick::OskCard::new(cx))),
        WidgetKind::Power => Box::new(quick::Card(quick::PowerCard::new(cx))),
        WidgetKind::Network => Box::new(quick::Card(quick::NetworkCard::new(cx))),
        WidgetKind::Bluetooth => Box::new(quick::Card(quick::BluetoothCard::new(cx))),
        WidgetKind::Media => Box::new(quick::MediaCard::new(cx)),
        WidgetKind::Calendar => Box::new(quick::Card(calendar_card::CalendarCard::new(cx))),
        _ => return None,
    })
}

/// Kutulu panel widget'ının somut türüne erişim (testler: ör. yeniden kurulan widget'ın metni).
/// Her `'static` tür için hazır; kutuda `w.as_ref().as_any()` ile çağrılır (`w.as_any()`
/// kutunun kendisini verir).
pub trait AsAny {
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<T: std::any::Any> AsAny for T {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Dil değişti (ADR-0010): bar widget'larını türlerinden yeniden kurar; metinler yeni
/// katalogdan gelir. Sıra ve bölümler korunur. Yeni widget'lar boştur: çağıran modelin ve
/// saatin son durumunu `sync`/`set_time` ile verir (servis olaylarından biçimlenen metinler de
/// böylece yeniden biçimlenir). Widget'ın kendi geçici durumu (hover, açık menü) sıfırlanır.
pub fn recreate_bar(bar: &mut [(crate::config::BarSection, Box<dyn BarWidget>)], cx: &WidgetCx) {
    for (_, w) in bar.iter_mut() {
        if let Some(fresh) = create(w.kind(), cx) {
            *w = fresh;
        }
    }
}

/// [`recreate_bar`]'ın kenar paneli karşılığı.
pub fn recreate_panel(widgets: &mut [Box<dyn PanelWidget>], cx: &WidgetCx) {
    for w in widgets.iter_mut() {
        if let Some(fresh) = create_panel(w.kind(), cx) {
            *w = fresh;
        }
    }
}

pub(crate) fn center_in(rect: Rect, w: f32, h: f32) -> Point {
    Point::new(rect.center().x - w * 0.5, rect.center().y - h * 0.5)
}

/// Bar öğesinin hover/basılı katmanı: `on_surface` tonu, `container` (açık popout'un
/// `surface_bright` hapı; yoksa `TRANSPARENT` = doğrudan bar yüzeyi) üstünde. `contents`:
/// katmanın üstüne çizilen içerik renkleri. Framework `Interaction::draw_state_layer_under`
/// (t124): standart kipte eski `draw_layer` ile aynı piksel; yüksek karşıtlıkta katman,
/// içerik ≥ 4.5 kalacak kadarla sınırlanır. Shell durum katmanını yalnız bununla çizer.
pub(crate) fn draw_hover(
    painter: &mut Painter,
    hover: &nuxut::ui::Interaction,
    rect: Rect,
    radii: nuxut::Corners,
    cx: &WidgetCx,
    container: nuxut::Color,
    contents: &[nuxut::Color],
) {
    let mut ui = Ui::new(painter, cx.theme, cx.now);
    hover.draw_state_layer_under(&mut ui, rect, radii, cx.theme.colors.on_surface, container, contents);
}

/// Açık popout hapı varsa onun dolgusu, yoksa saydam (bar yüzeyi): [`draw_hover`]'ın `container`'ı.
pub(crate) fn chip_fill(theme: &Theme, highlighted: bool) -> nuxut::Color {
    if highlighted { theme.colors.surface_bright } else { nuxut::Color::TRANSPARENT }
}

#[cfg(test)]
mod state_layer_tests {
    use std::time::Duration;

    use nuxut::theme::contrast::TEXT_MIN;
    use nuxut::ui::{Interaction, style};
    use nuxut::{Color, Corners, Rect, TextSystem, Theme};

    use super::*;

    /// Basılı (tam opak katman) bir etkileşim ve oturmuş zaman.
    fn pressed(theme: &Theme, rect: Rect) -> (Interaction, Instant) {
        let t0 = Instant::now();
        let mut i = Interaction::new(theme);
        i.pointer(rect, Some(rect.center()), true, t0);
        (i, t0 + Duration::from_secs(2))
    }

    /// t124: standart kipte bar öğesinin katmanı eski `Interaction::draw_layer` rengiyle aynı
    /// (hap dolgusu ve içerik ne olursa olsun); kayıtta tek dikdörtgen, boşta hiç çizim yok.
    #[test]
    fn hover_layer_standard_mode_is_unchanged() {
        let theme = Theme::default();
        let c = &theme.colors;
        let rect = Rect::new(0.0, 0.0, 30.0, 26.0);
        let (i, now) = pressed(&theme, rect);
        let old = i.layer_color(&theme, now);
        for highlighted in [false, true] {
            for content in [c.on_surface, c.on_surface_variant, c.primary, c.error] {
                let new = style::state_layer_under(&theme, c.on_surface, i.opacity(now), &[chip_fill(&theme, highlighted)], &[content]);
                assert_eq!(new, old);
            }
        }

        let settings = WidgetSettings::default();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now, bounds: Rect::ZERO };
        let mut text = TextSystem::new();
        let mut rec = nuxut::render::Recorder::new();
        rec.record(&mut text, (64, 64), 1.0, |p| draw_hover(p, &i, rect, Corners::ZERO, &cx, Color::TRANSPARENT, &[c.on_surface]));
        assert_eq!(rec.quads().len(), 1);
        let idle = Interaction::new(&theme);
        rec.record(&mut text, (64, 64), 1.0, |p| draw_hover(p, &idle, rect, Corners::ZERO, &cx, Color::TRANSPARENT, &[c.on_surface]));
        assert!(rec.is_empty());
    }

    /// Yüksek karşıtlıkta katmanın altındaki içerik (açık popout hapı dahil) ≥ 4.5 kalır.
    #[test]
    fn hover_layer_keeps_content_legible_in_high_contrast() {
        for mode in ["dark", "light"] {
            let theme = Theme::parse(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"high\"\n")).unwrap().0;
            let c = &theme.colors;
            let rect = Rect::new(0.0, 0.0, 30.0, 26.0);
            let (i, now) = pressed(&theme, rect);
            for highlighted in [false, true] {
                let fill = chip_fill(&theme, highlighted);
                for content in [c.on_surface, c.on_surface_variant, c.primary] {
                    let layer = style::state_layer_under(&theme, c.on_surface, i.opacity(now), &[fill], &[content]);
                    let ratio = style::worst_over(content, &style::backdrops(&theme, &[fill, layer]));
                    assert!(ratio >= TEXT_MIN, "{mode} highlighted={highlighted}: {ratio}");
                }
            }
        }
    }
}
