//! Bir output'un shell durumu: yüzey kimlikleri, animasyonlu çerçeve, kayıttan kurulan bar
//! widget'ları, saat popout'u ve kenar çekmeceleri ([`Sheet`]).
//!
//! Geometri her zaman `layout.rs`'in saf fonksiyonlarından, yaydaki ara değerlerle kurulur;
//! çizim ve isabet testi aynı [`BarGeometry`]'yi kullanır, böylece tıklanan yer çizilen yerdir.
//! Aynı anda en fazla bir yaprak (popout ya da çekmece) açıktır.

use std::time::Instant;

mod access;
pub use access::{NavOutcome, NavTarget};

use chrono::{DateTime, Local};
use nuxut::ui::{Icon, IconButton, IconButtonVariant, IconName, MenuAlign, MenuAnchor, MenuSide, Osd, ScreenEdge, Tooltip, Ui};
use nuxut::{
    Animated, BlobField, Button, CursorShape, FocusGrabId, Insets, KeyEvent, KeyboardMode, OutputId, Painter, Point, PointerEvent, PointerKind,
    Rect, ShadowStyle, Size, SurfaceId, TextSystem, Theme,
};

use crate::bar_damage::{BarDamage, BarFrame};
use crate::bar_edge::{EdgeCommand, EdgeHint, EdgeRoute, EdgeSide, ScrollAccumulator, ScrollAction};
use crate::calendar_view::{CalendarMode, CalendarView};
use crate::launcher_view::{LauncherOutcome, LauncherView};
use crate::config::{BarSection, ShellConfig};
use crate::layout::{Edge, PanelAlign, PanelSpec, ShellLayout};
use crate::model::ShellModel;
use crate::panels::{self, PanelState, Transition};
use crate::sheet::{Sheet, seed_for};
use crate::sysmon_view::{SYSMON_WIDTH, SysmonView};
use crate::widgets::notification_popups::NotificationPopups;
use crate::widgets::{self, Action, BarWidget, PanelId, PanelWidget, WidgetCx, WidgetKind};

/// Saat/takvim popout'unun genişliği (yüksekliği içerikten hesaplanır; dikey bar'da yana açılır).
pub const POPOUT_WIDTH: f32 = 360.0;
/// Bildirim popup sütununun genişliği.
pub const POPUP_WIDTH: f32 = 380.0;
/// Sabitleme düğmesinin kenarı (`theme.space` birimi: 8 × 4 = 32 px).
const PIN_SIZE_GRID: u32 = 8;
/// Bu kadarın altındaki kenar/kavis "yok" sayılır (yay kuyruğu).
const EPSILON: f32 = 0.01;

/// Bir karedeki bar geometrisi.
#[derive(Debug, Clone)]
struct BarGeometry {
    layout: ShellLayout,
    /// `Screen::bar` sırasıyla widget dikdörtgenleri (gizli widget: boş dikdörtgen).
    items: Vec<Rect>,
    /// `EdgeSide::ALL` sırasıyla uç eylem bölgeleri (o bölümde görünür widget yoksa `None`).
    edges: [Option<EdgeZone>; 2],
}

/// Bir bar ucunun eylem bölgesi.
#[derive(Debug, Clone, Copy, PartialEq)]
struct EdgeZone {
    /// Uç widget ∪ ipucu yuvası: tık ve kaydırma burada uç eylemine gider.
    zone: Rect,
    /// Ayrılmış ipucu ikonu yuvası (kaydırma eylemi ikonluysa).
    hint: Option<Rect>,
    /// Yuvadan widget'a birim yön (ikon oradan büyüyerek çıkar).
    toward: Point,
}

/// İki isteğe bağlı uyanma anından erken olanı (`Ui::wake_deadline` birleştirmesi).
fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Uçta ipucu yuvası ayrılır mı: kaydırma eylemi ikonlu ve (parlaklıksa) bu monitörde
/// parlaklık kontrolü var. Yuva yalnızca yetenek değişince eklenir/kalkar, hover'da değil.
fn hint_wanted(scroll: ScrollAction, brightness_available: bool) -> bool {
    scroll.has_hint() && (scroll != ScrollAction::Brightness || brightness_available)
}

/// Bar kenarı → ipucunun kaçınacağı ekran kenarı (bire bir).
fn screen_edge(e: Edge) -> ScreenEdge {
    match e {
        Edge::Top => ScreenEdge::Top,
        Edge::Right => ScreenEdge::Right,
        Edge::Bottom => ScreenEdge::Bottom,
        Edge::Left => ScreenEdge::Left,
    }
}

/// Bölümlerin bar ekseni boyunca yerleşimi: her widget'ın dikdörtgeni ve uç bölgeleri. `lengths`
/// widget sırasıyla ölçülen uzunluklar (0 = gizli); `hints` ucun ipucu yuvası isteyip istemediği;
/// `hint_len` yuvanın uzunluğu (öğe boyu). Yuva bölüm toplamına eklenir, böylece `bar_slots`
/// onu sıradan bir öğe gibi yerleştirir ve ikon belirince hiçbir widget kaymaz.
fn arrange_bar(
    layout: &ShellLayout,
    sections: &[BarSection],
    lengths: &[f32],
    gap: f32,
    hints: [bool; 2],
    hint_len: f32,
) -> (Vec<Rect>, [Option<EdgeZone>; 2]) {
    let vertical = !layout.bar_edge.is_horizontal();
    let visible = |sec: BarSection| sections.iter().zip(lengths).enumerate().filter(move |(_, (s, l))| **s == sec && **l > 0.0);
    let first = visible(BarSection::Start).map(|(i, _)| i).next();
    let last = visible(BarSection::End).map(|(i, _)| i).next_back();
    let reserve = [hints[0] && first.is_some(), hints[1] && last.is_some()].map(|on| if on { hint_len + gap } else { 0.0 });

    let totals = BarSection::ALL.map(|sec| {
        let ls: Vec<f32> = visible(sec).map(|(_, (_, l))| *l).collect();
        ls.iter().sum::<f32>() + gap * ls.len().saturating_sub(1) as f32
    });
    let slots = layout.bar_slots(totals[0] + reserve[0], totals[1], totals[2] + reserve[1], gap);

    let mut items = vec![Rect::ZERO; lengths.len()];
    for (sec, region) in BarSection::ALL.into_iter().zip([slots.start, slots.center, slots.end]) {
        let (mut pos, end) = if vertical { (region.y, region.bottom()) } else { (region.x, region.right()) };
        if sec == BarSection::Start {
            pos += reserve[0];
        }
        for (i, (_, &len)) in visible(sec) {
            let len = len.min((end - pos).max(0.0));
            if len <= 0.0 {
                continue;
            }
            items[i] = if vertical { Rect::new(region.x, pos, region.width, len) } else { Rect::new(pos, region.y, len, region.height) };
            pos += len + gap;
        }
    }

    // Uç bölgeleri: yuva bölümün dış ucunda, widget'ın hemen yanında.
    let along = |r: Rect, p: f32, l: f32| if vertical { Rect::new(r.x, p, r.width, l) } else { Rect::new(p, r.y, l, r.height) };
    let unit = |s: f32| if vertical { Point::new(0.0, s) } else { Point::new(s, 0.0) };
    let zone = |widget: Option<usize>, side: usize| -> Option<EdgeZone> {
        let w = items[widget?];
        if w.is_empty() {
            return None;
        }
        let region = if side == 0 { slots.start } else { slots.end };
        let hint = (reserve[side] > 0.0).then(|| {
            let (r0, r1) = if vertical { (region.y, region.bottom()) } else { (region.x, region.right()) };
            along(region, if side == 0 { r0 } else { r1 - hint_len }, hint_len)
        });
        let zone = hint.map_or(w, |h| w.union(&h));
        Some(EdgeZone { zone, hint, toward: unit(if side == 0 { 1.0 } else { -1.0 }) })
    };
    let edges = [zone(first, 0), zone(last, 1)];
    (items, edges)
}

#[derive(Debug)]
pub struct Screen {
    pub output: OutputId,
    pub name: String,
    pub main: SurfaceId,
    /// `Edge::ALL` sırasıyla exclusion yüzeyleri ve son ayarlanan zone.
    pub exclusions: [Option<(SurfaceId, i32)>; 4],
    /// Duvar kağıdı yüzeyi (`[wallpaper]` ayarlıysa).
    pub wallpaper: Option<crate::wallpaper::WallpaperSurface>,
    /// Geçici yaprak için tutulan odak kapma (Hyprland; protokol yoksa hep `None`).
    pub focus_grab: Option<FocusGrabId>,
    /// Odak kapmaya ana yüzeyle birlikte eklenen yüzey: açık ekran klavyesi (`app/osk.rs`);
    /// ona dokunmak geçici yaprağı kapatmaz.
    pub focus_grab_extra: Option<SurfaceId>,
    /// Çıkışın compositor düzenindeki sol üstü (mantıksal; xdg-output). Ekran klavyesi
    /// monitörler arası sürüklemede işaretçinin hangi çıkışta bırakıldığını bununla bulur.
    pub position: Point,
    size: Size,
    fullscreen: bool,
    /// `bar hide` komutuyla gizlendi mi (bar kenarı çerçeve kalınlığına iner).
    bar_hidden: bool,
    insets: Animated<Insets>,
    /// Birleşim yarıçapı R (`BlobField::smoothing`, çerçeve iç köşeleri dahil): temanın
    /// `blob.smoothing`i; tam ekranda kenarlarla birlikte 0'a iner (ADR-0008).
    smoothing: Animated<f32>,

    bar: Vec<(BarSection, Box<dyn BarWidget>)>,
    /// Bar'ın kurulduğu (bölüm, tür) listesi ve yön; config değişince karşılaştırılır.
    bar_spec: (Vec<(BarSection, WidgetKind)>, bool),

    popout: Sheet,
    calendar: CalendarView,
    /// Uygulama başlatıcısı: bar kenarının ortasından büyüyen yaprak ve içeriği.
    launcher_sheet: Sheet,
    launcher: LauncherView,
    /// Sistem izleyicisi ayrıntı popout'u: bar widget'ının hizasından büyür.
    sysmon_sheet: Sheet,
    sysmon: SysmonView,
    /// `Edge::ALL` sırasıyla kenar çekmeceleri.
    drawers: [Sheet; 4],
    /// `Edge::ALL` sırasıyla çekmece içerikleri (`[panels.<kenar>].widgets`).
    panels: [Vec<Box<dyn PanelWidget>>; 4],
    /// Son çizimdeki panel widget dikdörtgenleri (isabet testi için).
    panel_rects: [Vec<Rect>; 4],
    /// Hangi yapraklar açık/sabit (`docs/design/panels.md`). Yaprak animasyonları bundan
    /// türetilir ([`Screen::sync_drawers`]).
    state: PanelState,
    /// `Edge::ALL` sırasıyla çekmecelerin sabitleme düğmeleri; dikdörtgenleri çizimde konur
    /// (görünmüyorsa boş).
    pins: [IconButton; 4],
    /// Bildirim popup yığını: bar kenarının end köşesinden büyür (overlay gibi en üstte).
    popups: NotificationPopups,
    /// Ses/parlaklık göstergesi: bar'ın karşı kenarının ortasından büyür (overlay gibi).
    osd: Osd,
    /// `EdgeSide::ALL` sırasıyla uç eylem ipucu ikonları ve kaydırma birikimleri.
    edge_hints: [EdgeHint; 2],
    edge_scroll: [ScrollAccumulator; 2],
    /// Bu monitörde parlaklığı ayarlanabilen ekran var mı (yoksa `brightness` uç eylemi yok
    /// sayılır, ipucu yuvası ayrılmaz).
    brightness_available: bool,
    /// Bar widget'larının ortak ipucu balonu ([`BarWidget::tooltip`]).
    tooltip: Tooltip,
    /// Bekleyen uyanma anı (ipucu gecikmesi): App tek timer kurar, timer'da kare ister.
    wake: Option<Instant>,
    /// Overlay şekillerinin son karedeki etki alanları (hasar: eski ∪ yeni).
    last_overlay_damage: Vec<Rect>,

    pointer: Option<Point>,
    button_down: bool,
    /// İşaretçinin altındaki imleç (DESIGN "İmleç"); App `cx.set_cursor` ile uygular.
    cursor: CursorShape,
    /// Yaprak açıldıktan sonra yüzey klavye odağını aldı mı (bkz. [`Screen::keyboard_mode`]).
    focus_grabbed: bool,
    /// Yaprak komutla açıldı: kapanana kadar klavye `Exclusive` kalır (bkz. `keyboard_mode`).
    hold_exclusive: bool,
    /// Son uygulanan input bölgesi ve klavye modu (yalnızca değişince yeniden gönderilir).
    applied_input: Option<(Vec<Rect>, KeyboardMode)>,
    /// Boşluğa açılan köşe yarıçapı r (`theme.shape.radius_large`; girdi alanı için).
    corner_radius: f32,
    /// Girdi bölgesinin blob kısmı ve hesaplandığı alan (hedef şekil): yalnızca alan değişince
    /// yeniden hesaplanır (`BlobField::input_region` ≈ 160 µs; her karede değil).
    blob_input: Option<(BlobField, Vec<Rect>)>,

    /// Hasar takibi: sonraki karede tüm yüzey; bar şeridinde değişen widget/uç bölgeleri
    /// ([`BarDamage`]: yerleşim değişmediyse yalnızca kirli dikdörtgenler).
    damage_all: bool,
    bar_damage: BarDamage,
    /// Son çizilen vurgu (açık yaprağı açan bar widget'ı): yaprak geçişinde yalnızca vurgusu
    /// değişen widget'ın dikdörtgeni hasar alır, tüm şerit değil (yaprağın kendi hasarıyla
    /// birleşip tam ekrana büyümesin).
    bar_highlight: Vec<bool>,
    /// Ekran okuyucu: tıklanabilir düğüm → işaretçi noktası (son ağaçtan; `screen/access.rs`).
    access_points: std::collections::HashMap<nuxut::access::NodeId, Point>,
    /// Saat popout'unun son çizildiği alan (ekran okuyucu düğümlerinin sınırı).
    calendar_area: Rect,
    /// Klavye gezinmesi (screen/access.rs): son ağaçtan hedefler, odak ve bar gezinmesi kipi.
    nav_targets: Vec<access::NavTarget>,
    nav_focus: Option<nuxut::access::NodeId>,
    bar_nav: bool,
    /// OSD'nin ekran okuyucu metni ("Ses %64"); `App::show_osd` yazar.
    pub osd_access: String,
}

impl Screen {
    pub fn new(theme: &Theme, config: &ShellConfig, output: OutputId, name: String, main: SurfaceId) -> Self {
        let mut s = Self {
            output,
            name,
            main,
            exclusions: [None; 4],
            wallpaper: None,
            focus_grab: None,
            focus_grab_extra: None,
            position: Point::ZERO,
            size: Size::ZERO,
            fullscreen: false,
            bar_hidden: false,
            insets: Animated::new(config.frame_insets(), theme.motion.spatial),
            smoothing: Animated::new(theme.blob.smoothing, theme.motion.spatial),
            bar: Vec::new(),
            bar_spec: (Vec::new(), false),
            popout: Sheet::new(theme, config.bar.edge),
            calendar: CalendarView::new(theme, CalendarMode::Popout),
            launcher_sheet: Sheet::new(theme, config.bar.edge),
            launcher: LauncherView::new(theme, &config.launcher),
            sysmon_sheet: Sheet::new(theme, config.bar.edge),
            sysmon: SysmonView::new(theme),
            drawers: Edge::ALL.map(|e| Sheet::new(theme, e)),
            panels: Default::default(),
            panel_rects: Default::default(),
            state: PanelState::default(),
            pins: Edge::ALL.map(|_| IconButton::new(theme, Icon::named(IconName::Pin), IconButtonVariant::Standard).toggleable(false)),
            popups: NotificationPopups::new(&WidgetCx {
                theme,
                settings: &config.bar.widgets,
                vertical: config.bar.is_vertical(),
                item: WidgetCx::item_for(theme, config.bar.size),
                now: Instant::now(),
                bounds: Rect::ZERO,
            }),
            osd: Osd::new(theme),
            edge_hints: [EdgeHint::new(theme), EdgeHint::new(theme)],
            edge_scroll: Default::default(),
            brightness_available: false,
            tooltip: Tooltip::new(theme),
            wake: None,
            last_overlay_damage: Vec::new(),
            pointer: None,
            button_down: false,
            cursor: CursorShape::Default,
            focus_grabbed: false,
            hold_exclusive: false,
            applied_input: None,
            corner_radius: theme.shape.radius_large,
            blob_input: None,
            damage_all: true,
            bar_damage: BarDamage::default(),
            bar_highlight: Vec::new(),
            access_points: Default::default(),
            calendar_area: Rect::ZERO,
            nav_targets: Vec::new(),
            nav_focus: None,
            bar_nav: false,
            osd_access: String::new(),
        };
        s.rebuild_bar(config, theme, Instant::now());
        s.rebuild_panels(config, theme, Instant::now());
        s
    }

    /// Çekmece içeriklerini config'e göre kurar; aynı türden mevcut widget'lar korunur.
    fn rebuild_panels(&mut self, config: &ShellConfig, theme: &Theme, now: Instant) {
        let cx = self.cx(config, theme, now);
        for (i, edge) in Edge::ALL.into_iter().enumerate() {
            let kinds = config.panels.get(edge).widget_kinds();
            let current: Vec<WidgetKind> = self.panels[i].iter().map(|w| w.kind()).collect();
            if current == kinds {
                for w in &mut self.panels[i] {
                    w.set_theme(&cx);
                }
                continue;
            }
            let mut old = std::mem::take(&mut self.panels[i]);
            self.panels[i] = kinds
                .into_iter()
                .filter_map(|k| match old.iter().position(|w| w.kind() == k) {
                    Some(p) => {
                        let mut w = old.swap_remove(p);
                        w.set_theme(&cx);
                        Some(w)
                    }
                    None => widgets::create_panel(k, &cx),
                })
                .collect();
            self.panel_rects[i].clear();
        }
    }

    /// Klavye isteyen bir şey açık mı: geçici yaprak ya da overlay (açık tepsi menüsü). Sabit
    /// panel klavyeyi istemez (`OnDemand`, bkz. [`Screen::keyboard_mode`]).
    fn wants_keyboard(&self) -> bool {
        self.state.transient().is_some() || self.bar_nav || self.bar.iter().any(|(_, w)| w.wants_keyboard())
    }

    /// Sabit panel şu an görünüyor mu: tam ekranda sabitler geçici olarak gizlenir.
    fn pinned_shown(&self, edge: Edge) -> bool {
        self.state.is_pinned(edge) && !self.fullscreen
    }

    fn cx<'a>(&self, config: &'a ShellConfig, theme: &'a Theme, now: Instant) -> WidgetCx<'a> {
        WidgetCx {
            theme,
            settings: &config.bar.widgets,
            vertical: config.bar.is_vertical(),
            item: WidgetCx::item_for(theme, config.bar.size),
            now,
            bounds: panels::free_area(self.inner_rect(config), &self.shown_pinned(config)),
        }
    }

    /// Çerçevenin iç dikdörtgeni (yüzey koordinatı): overlay'lerin (tepsi menüsü) sığacağı
    /// alan. Hedef kalınlıklarla hesaplanır; çerçeve animasyonu sürerken titremez.
    pub(crate) fn inner_rect(&self, config: &ShellConfig) -> Rect {
        let ins = ShellLayout::compute(config, self.size).insets;
        Rect::new(
            ins.left,
            ins.top,
            (self.size.width - ins.left - ins.right).max(0.0),
            (self.size.height - ins.top - ins.bottom).max(0.0),
        )
    }

    /// Bar widget'larını config'e göre kurar; aynı türden mevcut widget'lar korunur (hover ve
    /// hap animasyonları kesilmez), ayarları yenilenir.
    fn rebuild_bar(&mut self, config: &ShellConfig, theme: &Theme, now: Instant) {
        let cx = self.cx(config, theme, now);
        let spec: Vec<(BarSection, WidgetKind)> = BarSection::ALL
            .into_iter()
            .flat_map(|sec| config.bar.widgets_in(sec).into_iter().map(move |k| (sec, k)))
            .collect();
        let vertical = config.bar.is_vertical();
        if spec == self.bar_spec.0 && vertical == self.bar_spec.1 {
            for (_, w) in &mut self.bar {
                w.set_theme(&cx);
            }
            return;
        }
        let mut old = std::mem::take(&mut self.bar);
        self.bar = spec
            .iter()
            .filter_map(|&(sec, kind)| match old.iter().position(|(_, w)| w.kind() == kind) {
                Some(i) => {
                    let (_, mut w) = old.swap_remove(i);
                    w.set_theme(&cx);
                    Some((sec, w))
                }
                None => widgets::create(kind, &cx).map(|w| (sec, w)),
            })
            .collect();
        self.bar_spec = (spec, vertical);
        self.damage_all = true;
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn set_size(&mut self, size: Size) {
        self.size = size;
        self.damage_all = true;
    }

    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub fn is_bar_visible(&self) -> bool {
        !self.bar_hidden
    }

    /// Bar'ı gösterir/gizler: bar kenarı yayla çerçeve kalınlığına iner, içerik kenardan
    /// dışarı kayar. Değiştiyse `true` (çağıran exclusion ve input bölgesini eşler).
    pub fn set_bar_visible(&mut self, visible: bool, config: &ShellConfig, theme: &Theme, now: Instant) -> bool {
        if self.bar_hidden != visible {
            return false;
        }
        self.bar_hidden = !visible;
        if self.bar_hidden {
            // Bar gizlendi: sabitler dahil hepsi kapanır (açık kapatma).
            let t = self.state.close_all();
            self.apply_closed(&t, now);
        }
        self.apply_config(config, theme, now);
        true
    }

    /// Geçici yaprak (örtük kapatmanın hedefi: Escape, dışarı tıklama, odak kaybı).
    pub fn transient_panel(&self) -> Option<PanelId> {
        self.state.transient()
    }

    /// Bar'dan menü açılırken kapanacak yaprak ([`panels::PanelState::closed_by_bar_menu`]).
    pub fn closed_by_bar_menu(&self) -> Option<PanelId> {
        self.state.closed_by_bar_menu()
    }

    /// Açık yapraklar üstten alta (geçici, sonra sabitler).
    pub fn open_panels(&self) -> Vec<PanelId> {
        self.state.open_panels().collect()
    }

    pub fn is_panel_open(&self, panel: PanelId) -> bool {
        self.state.is_open(panel)
    }

    pub fn is_pinned(&self, edge: Edge) -> bool {
        self.state.is_pinned(edge)
    }

    /// Klavye modu. Hyprland `OnDemand`'e geçişte odak vermez; yaprak açılınca önce
    /// `Exclusive` ile odak alınır, odak gelince `OnDemand`'e inilir ki başka yere tıklama
    /// odağı alıp yaprağı kapatabilsin (`keyboard_focus(false)`).
    ///
    /// Komutla (Hyprland bind'i) açılan yaprakta işaretçi bir pencerenin üstündedir:
    /// `OnDemand`'e inince Hyprland odağı o pencereye geri verir ve yaprak hemen kapanırdı. Bu
    /// yüzden komutla açılan yaprak açık kaldığı sürece `Exclusive` kalır; Escape, aynı komut ya
    /// da bar ile kapanır.
    ///
    /// Yalnızca sabit panel(ler) açıkken `OnDemand`: kullanıcı başka pencerelerde yazar, panele
    /// tıklarsa odak gelir; odak kaybı sabit paneli kapatmaz.
    pub fn keyboard_mode(&self) -> KeyboardMode {
        match (self.wants_keyboard(), self.focus_grabbed && !self.hold_exclusive) {
            (false, _) if self.state.pinned().iter().any(|&e| self.pinned_shown(e)) => KeyboardMode::OnDemand,
            (false, _) => KeyboardMode::None,
            (true, false) => KeyboardMode::Exclusive,
            (true, true) => KeyboardMode::OnDemand,
        }
    }

    /// Geçici yaprak komutla açıldı: kapanana kadar klavye `Exclusive` kalır.
    pub fn hold_keyboard(&mut self) {
        if self.state.transient().is_some() {
            self.hold_exclusive = true;
        }
    }

    /// Odak kapma gerekli mi (geçici yaprak açık; `PanelState::wants_focus_grab`).
    pub fn wants_focus_grab(&self) -> bool {
        self.state.wants_focus_grab()
    }

    /// Son uygulanan klavye modu (henüz uygulanmadıysa `None`).
    pub fn applied_keyboard_mode(&self) -> Option<KeyboardMode> {
        self.applied_input.as_ref().map(|(_, m)| *m)
    }

    /// Geçici yaprak var mı (odak bekleme süresi bunun için yeniden başlar).
    pub fn has_transient(&self) -> bool {
        self.state.transient().is_some()
    }

    /// Odak beklenen sürede gelmedi: `Exclusive`'te kalıp klavyeyi rehin tutmamak için
    /// `OnDemand`'e in. Mod değiştiyse `true`. Komutla açılan yaprakta inilmez (Escape çalışsın).
    pub fn give_up_focus_grab(&mut self) -> bool {
        if self.hold_exclusive {
            return false;
        }
        self.focus_changed(true)
    }

    /// Klavye odağı değişti. Klavye modu değişmesi gerekiyorsa `true`.
    pub fn focus_changed(&mut self, focused: bool) -> bool {
        if focused && self.wants_keyboard() && !self.focus_grabbed {
            self.focus_grabbed = true;
            return true;
        }
        false
    }

    /// Çerçevenin hedef kenarları: tam ekranda 0.
    fn targets(&self, config: &ShellConfig) -> Insets {
        if self.fullscreen { Insets::ZERO } else { config.frame_insets_with_bar(!self.bar_hidden) }
    }

    /// Hedef birleşim yarıçapı R: tam ekranda 0 (kenarlarla birlikte söner, köşe zıplamaz).
    fn target_smoothing(&self, theme: &Theme) -> f32 {
        if self.fullscreen { 0.0 } else { theme.blob.smoothing }
    }

    /// Config veya tam ekran durumu değişti: çerçeve ve açık yapraklar yeni hedefe yayla akar.
    pub fn apply_config(&mut self, config: &ShellConfig, theme: &Theme, now: Instant) {
        let insets = self.targets(config);
        self.insets.set_target(insets, now);
        self.smoothing.set_target(self.target_smoothing(theme), now);
        self.rebuild_bar(config, theme, now);
        // Yerleşim değişti: ipucu bir sonraki işaretçi olayında yeniden değerlendirilir.
        for h in &mut self.edge_hints {
            h.set_hovered(false, now);
        }
        self.rebuild_panels(config, theme, now);
        if self.popout.edge() != config.bar.edge && self.state.is_open(PanelId::Clock) {
            // Popout kenarı değişti: anında kapanır (`Sheet::set_edge`), durum da.
            self.state.close(PanelId::Clock);
        }
        self.popout.set_edge(config.bar.edge);
        if (self.launcher_sheet.edge() != config.bar.edge || !config.launcher.enabled)
            && self.state.is_open(PanelId::Launcher)
        {
            // Kenar değişti (`Sheet::set_edge` anında kapatır) ya da başlatıcı kapatıldı.
            let t = self.state.close(PanelId::Launcher);
            self.apply_closed(&t, now);
        }
        if self.sysmon_sheet.edge() != config.bar.edge && self.state.is_open(PanelId::Sysmon) {
            self.state.close(PanelId::Sysmon);
        }
        self.sysmon_sheet.set_edge(config.bar.edge);
        self.launcher_sheet.set_edge(config.bar.edge);
        self.launcher.set_config(&config.launcher);
        for edge in Edge::ALL {
            if !config.panels.get(edge).enabled {
                let t = self.state.close(PanelId::Edge(edge));
                self.apply_closed(&t, now);
            }
        }
        self.sync_drawers(config, now);
        self.retarget_launcher(config, theme, now);
        self.damage_all = true;
    }

    /// Dil değişti (ADR-0010 `LocaleChanged`; süreç geneli katalog zaten yeni): metin kuran
    /// her şey zorla yeniden kurulur — [`Screen::apply_config`]'in aynı config ile zorlanmış
    /// türevi. Bar ve çekmece widget'ları türlerinden yeniden yaratılıp modelin ve saatin son
    /// durumundan eşlenir (servis olaylarından biçimlenen metinler de yeniden biçimlenir);
    /// başlatıcı, takvim, sistem izleyici ve popup kartları kendi durumlarını koruyarak
    /// metinlerini yeniler. Yaprak durumu, sabitler ve odak (klavye modu, odak kapma) korunur;
    /// açık yapraklar yeni içerik boyuna akar, tam hasar verilir. Widget'ın kendi geçici durumu
    /// (açık tepsi menüsü, onay bekleyen düğme, kaydırma) sıfırlanır.
    ///
    /// Döner: açık çekmecelerdeki yeni widget'ların açılış kancalarının eylemleri (App uygular;
    /// ör. açık bildirim merkezinde okundu işareti).
    pub fn relocalize(&mut self, model: &ShellModel, text: &mut TextSystem, theme: &Theme, now: Instant) -> Vec<Action> {
        let config = &model.config;
        let cx = self.cx(config, theme, now);
        widgets::recreate_bar(&mut self.bar, &cx);
        for (ws, rects) in self.panels.iter_mut().zip(&mut self.panel_rects) {
            widgets::recreate_panel(ws, &cx);
            rects.clear();
        }
        self.popups.relocalize();
        self.sysmon.relocalize(theme, now);
        self.calendar.relocalize(theme);
        self.launcher.relocalize(theme, now);
        // Görünen ipucu eski metni taşır: kapanır, sonraki işaretçi olayında yenisi sorulur.
        self.tooltip.hide(now);
        // Yeni widget'lar boş: modelin ve saatin son durumu.
        self.sync_model(model, theme, now);
        self.set_time(&Local::now(), config, theme);
        let mut actions = Vec::new();
        for edge in Edge::ALL {
            if self.drawers[edge_index(edge)].is_open() && self.state.is_open(PanelId::Edge(edge)) {
                actions.extend(self.panel_opened(PanelId::Edge(edge), now));
            }
        }
        // Metin genişlikleri değişti: çerçeve, çekmeceler ve başlatıcı yeni hedefe; içeriğe göre
        // boyutlanan açık popout'lar (saat, sistem izleyici) yeni yükseklik ve bar hizasıyla.
        self.apply_config(config, theme, now);
        if self.popout.is_open() {
            self.open_popout(text, config, theme, now);
        }
        if self.sysmon_sheet.is_open() {
            self.open_sysmon(text, config, theme, now);
        }
        self.damage_all = true;
        self.bar_damage.mark_all();
        actions
    }

    /// Görünen sabit paneller, sabitleme sırasıyla (kenar, derinlik): yer paylaşımının girdisi.
    fn shown_pinned(&self, config: &ShellConfig) -> Vec<(Edge, f32)> {
        self.state.pinned().iter().filter(|&&e| self.pinned_shown(e)).map(|&e| (e, config.panels.get(e).size)).collect()
    }

    /// Sabit panellerin ve geçici yaprağın yerleşeceği alanlar (hedef geometri): `Edge::ALL`
    /// sırasıyla görünen sabit panellerin alanı ve geçici yaprağın alanı (`panels::share_area`).
    fn sheet_areas(&self, config: &ShellConfig) -> ([Option<Rect>; 4], Rect) {
        let inner = self.target_layout(config).frame.inner;
        let shown = self.shown_pinned(config);
        let (areas, rest) = panels::share_area(inner, &shown);
        let mut out = [None; 4];
        for ((e, _), a) in shown.iter().zip(areas) {
            out[edge_index(*e)] = Some(a);
        }
        (out, rest)
    }

    /// Hedef geometri, iç alanı `area` ile değiştirilmiş (yaprak yerleşimi için).
    fn layout_in(&self, config: &ShellConfig, area: Rect) -> ShellLayout {
        let mut l = self.target_layout(config);
        l.frame.inner = area;
        l
    }

    /// Çekmece yapraklarını duruma eşler: görünmesi gereken açılır ya da yeni hedefine akar,
    /// gerekmeyen kapanır (kapanış kancaları [`Screen::apply_closed`]'da). Sabitleme düğmeleri
    /// de durumu gösterir. Bar'ın vurgu değişimi çizimde widget başına hasar alır.
    fn sync_drawers(&mut self, config: &ShellConfig, now: Instant) {
        let (pinned_areas, free) = self.sheet_areas(config);
        for (i, edge) in Edge::ALL.into_iter().enumerate() {
            let id = PanelId::Edge(edge);
            let area = match pinned_areas[i] {
                Some(a) => Some(a),
                None if self.state.transient() == Some(id) => Some(free),
                None => None,
            };
            let area = area.filter(|_| config.panels.get(edge).enabled);
            match area {
                Some(a) => {
                    let (target, seed) = drawer_geometry(&self.layout_in(config, a), edge, config);
                    if self.drawers[i].is_open() {
                        self.drawers[i].retarget(target, seed, now);
                    } else {
                        self.drawers[i].open(target, seed, now);
                        log_sheet_target(edge.name(), target);
                    }
                }
                None => self.drawers[i].close(now),
            }
            self.pins[i].set_selected(self.state.is_pinned(edge), now);
        }
        if self.state.transient() != Some(PanelId::Clock) {
            self.popout.close(now);
        }
        if self.state.transient() != Some(PanelId::Launcher) {
            self.launcher_sheet.close(now);
        }
        if self.state.transient() != Some(PanelId::Sysmon) {
            self.sysmon_sheet.close(now);
        }
        if self.state.transient().is_none() {
            self.focus_grabbed = false;
            self.hold_exclusive = false;
        }
    }

    /// Kapanan yaprakların kapanış kancaları (onay bekleyen düğme, açık menü sıfırlanır).
    fn apply_closed(&mut self, t: &Transition, now: Instant) {
        for p in &t.closed {
            match p {
                PanelId::Edge(e) => {
                    for w in &mut self.panels[edge_index(*e)] {
                        w.panel_closed(now);
                    }
                }
                PanelId::Launcher => self.launcher.closed(now),
                PanelId::Clock | PanelId::Sysmon => {}
            }
        }
    }

    /// Hedef (animasyon sonu) geometrisi: input bölgesi ve yaprak konumları için.
    fn target_layout(&self, config: &ShellConfig) -> ShellLayout {
        let insets = self.targets(config);
        ShellLayout::from_parts(self.size, insets, config.bar.edge, config.bar.padding)
    }

    /// Ana yüzeyin input bölgesi: blob'un gerçek (oturmuş, hedef) şekli — çerçeve, açık
    /// yapraklar, birleşim filetoları; boşluğa açılan `r` köşelerinin dışı değil — + overlay ve
    /// popup dikdörtgenleri. Yaprak dışına tıklama alttaki pencereye geçer; o pencere klavye
    /// odağını alınca yaprak kapanır (`keyboard_focus`). Blob kısmı hedef alan değişince bir kez
    /// hesaplanır (`blob_input` önbelleği); açılışta hedef şekil hemen geçerlidir (eski
    /// dikdörtgen kuralıyla aynı zamanlama), animasyon karelerinde yeniden hesap yok.
    pub fn input_rects(&mut self, config: &ShellConfig) -> Vec<Rect> {
        let sheets: Vec<Rect> = std::iter::once(&self.popout)
            .chain(&self.drawers)
            .chain(std::iter::once(&self.launcher_sheet))
            .chain(std::iter::once(&self.sysmon_sheet))
            .filter(|s| s.is_open())
            .map(Sheet::target)
            .collect();
        let extra: Vec<Rect> =
            self.bar.iter().flat_map(|(_, w)| w.overlay_input()).chain(self.popups.input_rects()).collect();
        if self.fullscreen && sheets.is_empty() && extra.is_empty() {
            return Vec::new();
        }
        let layout = self.target_layout(config);
        let field = layout.input_field(&sheets, self.smoothing.target(), self.corner_radius);
        if !matches!(&self.blob_input, Some((f, _)) if *f == field) {
            let started = Instant::now();
            let rects = field.input_region();
            let micros = started.elapsed().as_micros();
            tracing::debug!(output = %self.name, rects = rects.len(), micros, "girdi bölgesi blob şeklinden yeniden hesaplandı");
            self.blob_input = Some((field, rects));
        }
        let blob = self.blob_input.as_ref().map_or(&[][..], |(_, r)| r.as_slice());
        layout.input_region(blob, &extra)
    }

    /// Input bölgesi ya da klavye modu son uygulanandan farklı mı? (Çizimde ucuz kontrol:
    /// overlay'ler ve popup'lar boyutlarını çizim sırasında öğrenir; blob kısmı önbellekten.)
    pub fn input_stale(&mut self, config: &ShellConfig) -> bool {
        if self.size == Size::ZERO {
            return false;
        }
        let Some((_, mode)) = &self.applied_input else { return true };
        if *mode != self.keyboard_mode() {
            return true;
        }
        let rects = self.input_rects(config);
        self.applied_input.as_ref().is_none_or(|(applied, _)| *applied != rects)
    }

    /// Değiştiyse uygulanacak yeni (input bölgesi, klavye modu); değişmediyse `None`. Klavye
    /// isteyen bir şey kalmadıysa odak bayrağı sıfırlanır: sonraki açılış yeniden `Exclusive`
    /// ile odak alır (menü kendi kendine kapandığında da).
    pub fn take_input_change(&mut self, config: &ShellConfig) -> Option<(Vec<Rect>, KeyboardMode)> {
        if self.size == Size::ZERO {
            return None;
        }
        if !self.wants_keyboard() {
            self.focus_grabbed = false;
            self.hold_exclusive = false;
        }
        let next = (self.input_rects(config), self.keyboard_mode());
        if self.applied_input.as_ref() == Some(&next) {
            return None;
        }
        self.applied_input = Some(next.clone());
        Some(next)
    }

    /// Odak bekleniyor mu (klavye `Exclusive` istendi, odak henüz gelmedi; komutla açılan
    /// yaprak hariç): çağıran yedek timer'ı kurar.
    pub fn awaiting_focus(&self) -> bool {
        self.wants_keyboard() && !self.focus_grabbed && !self.hold_exclusive
    }

    /// Exclusion yüzeyleri için hedef zone'lar. Tam ekranda hepsi 0: çerçeve 0'a iner,
    /// yüzeyler kaldırılır (0 kalınlıklı yüzey runtime'da 1 px itme olurdu).
    pub fn desired_zones(&self, config: &ShellConfig) -> [i32; 4] {
        if self.fullscreen {
            return [0; 4];
        }
        let size = Size::new(self.size.width.max(1.0), self.size.height.max(1.0));
        let insets = self.targets(config);
        let z = ShellLayout::from_parts(size, insets, config.bar.edge, config.bar.padding).exclusive_zones();
        Edge::ALL.map(|e| z.get(e))
    }

    /// Model değişikliğini uygular. Yeniden çizim gerekiyorsa `true`.
    pub fn sync_model(&mut self, model: &ShellModel, theme: &Theme, now: Instant) -> bool {
        let cx = self.cx(&model.config, theme, now);
        let mut changed = false;
        for (i, (_, w)) in self.bar.iter_mut().enumerate() {
            if w.sync(model, &self.name, &cx) {
                self.bar_damage.mark(i);
                changed = true;
            }
        }
        // Uç ipucu ikonları canlı durumu gösterir (ses seviyesi/sessiz, parlaklık).
        for (i, (side, hint)) in EdgeSide::ALL.into_iter().zip(&mut self.edge_hints).enumerate() {
            let scroll = model.config.bar.edge_actions.side(side).binding.scroll;
            if hint.set_icon(scroll.hint_icon(model, &self.name)) {
                self.bar_damage.mark_hint(i);
                changed = true;
            }
        }
        // Parlaklık yeteneği değişirse (ekran takıldı/çıktı) ipucu yuvası eklenir/kalkar.
        let brightness = model.system.brightness_display(Some(&self.name)).is_some();
        if brightness != self.brightness_available {
            self.brightness_available = brightness;
            self.damage_all = true;
            changed = true;
        }
        for (i, ws) in self.panels.iter_mut().enumerate() {
            let mut panel_changed = false;
            for w in ws.iter_mut() {
                panel_changed |= w.sync(model, &cx);
            }
            // Açık çekmecenin içeriği değiştiyse çizilsin (hasar: çekmece şekli her karede
            // bildiriliyor; kapalıysa iş yok).
            changed |= panel_changed && self.drawers[i].is_open();
        }
        // Popup yığını: kartlar bir sonraki karede kurulup ölçülür.
        changed |= self.popups.sync(model, &cx);
        let cal = self.calendar.set_entries(theme, &model.calendar, model.calendar_available);
        changed |= cal && self.popout.is_open();
        changed |= self.launcher.set_data(theme, &model.launcher_catalog, &model.launcher_usage, now);
        changed |= self.launcher.set_clipboard(theme, &model.clipboard, now) && self.launcher_sheet.is_open();
        let fullscreen = model.is_fullscreen(&self.name);
        if fullscreen != self.fullscreen {
            self.fullscreen = fullscreen;
            self.apply_config(&model.config, theme, now);
            changed = true;
        }
        changed
    }

    /// Saat ilerledi. Değiştiyse `true`.
    pub fn set_time(&mut self, now: &DateTime<Local>, config: &ShellConfig, theme: &Theme) -> bool {
        let cx = self.cx(config, theme, Instant::now());
        let mut changed = false;
        for (i, (_, w)) in self.bar.iter_mut().enumerate() {
            if w.set_time(now, &cx) {
                self.bar_damage.mark(i);
                changed = true;
            }
        }
        changed |= self.calendar.set_time(now);
        changed |= self.popups.set_time();
        for (i, ws) in self.panels.iter_mut().enumerate() {
            let open = self.drawers[i].is_open();
            for w in ws.iter_mut() {
                // Kapalı çekmecenin widget'ı da güncellenir (açılınca doğru görünsün); yalnızca
                // açıksa yeniden çizim istenir.
                let c = w.set_time(now, &cx);
                changed |= c && open;
            }
        }
        changed
    }

    /// OSD'yi gösterir ya da açıksa değerini günceller. `value` 0..=`max` (`max` > 1: %100 üstü
    /// taşma, ör. ses yükseltmesi). `dimmed`: içerik soluk çizilir (sessiz).
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn show_osd(
        &mut self,
        icon: Icon,
        value: f32,
        max: f32,
        dimmed: bool,
        value_text: Option<String>,
        config: &ShellConfig,
        theme: &Theme,
        now: Instant,
    ) {
        let inner = self.inner_rect(config);
        // Sessizlik ve değer metni `show`'dan önce: açılış ilk karesinden doğru, değişim aynı
        // karede çizilir. Metin `None` ise yüzde (ses/parlaklık).
        self.osd.set_muted(dimmed, now);
        self.osd.set_value_text(value_text);
        self.osd.set_max(max, now);
        self.osd.show(theme, icon, value, osd_anchor(inner, config.bar.edge), inner, now);
    }

    /// Açıksa kapatır; kare gerekiyorsa `true`.
    pub fn hide_osd(&mut self, now: Instant) -> bool {
        let open = self.osd.is_open();
        self.osd.hide(now);
        open
    }

    pub fn set_theme(&mut self, config: &ShellConfig, theme: &Theme, now: Instant) {
        self.insets.set_spring(theme.motion.spatial, now);
        self.smoothing.set_spring(theme.motion.spatial, now);
        self.smoothing.set_target(self.target_smoothing(theme), now);
        self.corner_radius = theme.shape.radius_large;
        self.popout.set_theme(theme, now);
        self.launcher_sheet.set_theme(theme, now);
        self.sysmon_sheet.set_theme(theme, now);
        self.sysmon.set_theme(theme, now);
        self.launcher.set_theme(theme, now);
        for d in &mut self.drawers {
            d.set_theme(theme, now);
        }
        for pin in &mut self.pins {
            pin.set_theme(theme, now);
        }
        self.calendar.set_theme(theme, now);
        let cx = self.cx(config, theme, now);
        for (_, w) in &mut self.bar {
            w.set_theme(&cx);
        }
        self.popups.set_theme(&cx);
        self.osd.set_theme(theme, now);
        self.tooltip.set_theme(theme, now);
        for h in &mut self.edge_hints {
            h.set_theme(theme, now);
        }
        for w in self.panels.iter_mut().flatten() {
            w.set_theme(&cx);
        }
        self.damage_all = true;
    }

    fn geometry(&mut self, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) -> BarGeometry {
        let edge = config.bar.edge;
        let layout = ShellLayout::from_parts(self.size, self.insets.value(now), edge, config.bar.padding);
        let cx = self.cx(config, theme, now);

        // Bar içeriği tam boyutlu şeritte durur; bar daralırken kenardan dışarı kayar.
        let full = config.bar.size;
        let t = edge.of(&layout.insets);
        let c = layout.bar_content;
        let content = match edge {
            Edge::Top => Rect::new(c.x, t - full, c.width, full),
            Edge::Bottom => Rect::new(c.x, self.size.height - t, c.width, full),
            Edge::Left => Rect::new(t - full, c.y, full, c.height),
            Edge::Right => Rect::new(self.size.width - t, c.y, full, c.height),
        };

        let lengths: Vec<f32> = self.bar.iter_mut().map(|(_, w)| w.measure(text, &cx)).collect();
        let sections: Vec<BarSection> = self.bar.iter().map(|(s, _)| *s).collect();
        let mut arranging = layout;
        arranging.bar_content = content;
        let actions = &config.bar.edge_actions;
        let hints = EdgeSide::ALL.map(|side| hint_wanted(actions.side(side).binding.scroll, self.brightness_available));
        let (items, edges) = arrange_bar(&arranging, &sections, &lengths, config.bar.spacing, hints, cx.item);
        BarGeometry { layout, items, edges }
    }

    /// Yaprağı açar/kapatır. Açılan geçici olur; önceki geçici yaprak kapanır, sabitler kalır.
    /// Açık (sabit de olsa) yaprağa toggle onu kapatır.
    pub fn toggle_panel(&mut self, panel: PanelId, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) -> Transition {
        if !self.state.is_open(panel)
            && let PanelId::Edge(e) = panel
            && !config.panels.get(e).enabled
        {
            tracing::info!(edge = e.name(), "panel kapalı (panels.{}.enabled = false)", e.name());
            return Transition::default();
        }
        if panel == PanelId::Launcher && !self.state.is_open(panel) && !config.launcher.enabled {
            tracing::info!("başlatıcı kapalı (launcher.enabled = false)");
            return Transition::default();
        }
        let t = self.state.toggle(panel);
        self.apply_transition(&t, text, config, theme, now);
        t
    }

    /// Açık kapatma (komut, aynı düğme; örtük kapatmalar `transient_panel`'i verir).
    pub fn close_panel(&mut self, panel: PanelId, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) -> Transition {
        let t = self.state.close(panel);
        self.apply_transition(&t, text, config, theme, now);
        t
    }

    /// Sabitler / sabitlemeyi kaldırır (`true`: kapalıysa açılır).
    pub fn set_pinned(&mut self, edge: Edge, pinned: bool, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) -> Transition {
        if pinned && !self.state.is_open(PanelId::Edge(edge)) && !config.panels.get(edge).enabled {
            tracing::info!(edge = edge.name(), "panel kapalı (panels.{}.enabled = false)", edge.name());
            return Transition::default();
        }
        let t = self.state.set_pinned(edge, pinned);
        self.apply_transition(&t, text, config, theme, now);
        // Durum değişmese de (ör. düğme ile aynı anda komut) düğme durumu eşlenir.
        self.sync_drawers(config, now);
        t
    }

    /// Durum değişikliğini yapraklara uygular: kapanış kancaları, popout'un açılışı (geometrisi
    /// saatin konumundan), çekmecelerin eşlenmesi.
    fn apply_transition(&mut self, t: &Transition, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) {
        self.apply_closed(t, now);
        if t.opened.contains(&PanelId::Clock) {
            self.open_popout(text, config, theme, now);
        }
        if t.opened.contains(&PanelId::Sysmon) {
            self.open_sysmon(text, config, theme, now);
        }
        self.sync_drawers(config, now);
        if t.opened.contains(&PanelId::Launcher) {
            let (target, seed) = self.launcher_geometry(config, theme);
            self.launcher_sheet.set_edge(config.bar.edge);
            self.launcher_sheet.open(target, seed, now);
            self.launcher.opened(theme, now);
            log_sheet_target("launcher", target);
        } else {
            // Sabitler değişince serbest alan da değişir.
            self.retarget_launcher(config, theme, now);
        }
    }

    /// Açık başlatıcıyı yeni hedefine akıtır (config, sabitler, içerik boyu).
    fn retarget_launcher(&mut self, config: &ShellConfig, theme: &Theme, now: Instant) {
        if self.launcher_sheet.is_open() {
            let (target, seed) = self.launcher_geometry(config, theme);
            self.launcher_sheet.retarget(target, seed, now);
        }
    }

    /// Saat popout'unu saatin hizasında, sabit panellerin dışındaki alanda açar.
    fn open_popout(&mut self, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) {
        let (_, free) = self.sheet_areas(config);
        let target_layout = self.layout_in(config, free);
        let g = self.geometry(text, config, theme, now);
        let edge = config.bar.edge;
        let anchor = self
            .bar
            .iter()
            .zip(&g.items)
            .find(|((_, w), r)| w.kind() == WidgetKind::Clock && !r.is_empty())
            .map(|(_, r)| if edge.is_horizontal() { r.center().x } else { r.center().y })
            .unwrap_or_else(|| {
                let c = target_layout.bar_content.center();
                if edge.is_horizontal() { c.x } else { c.y }
            });
        let height = self.calendar.preferred_height(text, theme, POPOUT_WIDTH);
        let (length, depth) = if edge.is_horizontal() { (POPOUT_WIDTH, height) } else { (height, POPOUT_WIDTH) };
        let p = target_layout.panel(&PanelSpec { edge, align: PanelAlign::At(anchor), length, depth });
        self.popout.set_edge(edge);
        self.popout.open(p.open, seed_for(edge, p.closed, 0.5), now);
        log_sheet_target("clock", p.open);
    }

    /// Sistem izleyicisi popout'unun (hedef, tohum) dikdörtgeni: bar widget'ının hizasında (yoksa
    /// bar ortasında), sabit panellerin dışındaki alanda; yükseklik içerikten.
    fn sysmon_geometry(&mut self, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) -> (Rect, Rect) {
        let (_, free) = self.sheet_areas(config);
        let target_layout = self.layout_in(config, free);
        let g = self.geometry(text, config, theme, now);
        let edge = config.bar.edge;
        let anchor = self
            .bar
            .iter()
            .zip(&g.items)
            .find(|((_, w), r)| w.kind() == WidgetKind::Sysmon && !r.is_empty())
            .map(|(_, r)| if edge.is_horizontal() { r.center().x } else { r.center().y })
            .unwrap_or_else(|| {
                let c = target_layout.bar_content.center();
                if edge.is_horizontal() { c.x } else { c.y }
            });
        let height = self.sysmon.preferred_height(text, theme);
        let (length, depth) = if edge.is_horizontal() { (SYSMON_WIDTH, height) } else { (height, SYSMON_WIDTH) };
        let p = target_layout.panel(&PanelSpec { edge, align: PanelAlign::At(anchor), length, depth });
        (p.open, seed_for(edge, p.closed, 0.5))
    }

    fn open_sysmon(&mut self, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) {
        let (target, seed) = self.sysmon_geometry(text, config, theme, now);
        self.sysmon_sheet.set_edge(config.bar.edge);
        self.sysmon_sheet.open(target, seed, now);
        log_sheet_target("sysmon", target);
    }

    /// Yeni sistem izleyici ölçümü: yalnızca bar'daki izleyici widget'ı ve (açıksa) popout
    /// güncellenir — modelin geri kalanına dokunulmaz. Popout içeriğinin boyu değiştiyse yaprak
    /// yeni hedefe akar; görünür ipucu yeni metni alır. Yeniden çizim gerekiyorsa `true`.
    /// Açık ipucu varsa metnini ve yerini yeniden sorar (widget verisi değişti, işaretçi
    /// kımıldamadı: ör. durum göstergesinin ses yüzdesi). Kare gerekiyorsa `true`.
    pub fn refresh_tooltip(&mut self, config: &ShellConfig, text: &mut TextSystem, theme: &Theme, now: Instant) -> bool {
        if !self.tooltip.is_visible() {
            return false;
        }
        let g = self.geometry(text, config, theme, now);
        let cx = self.cx(config, theme, now);
        self.bar_hover(&g, &cx, self.pointer, self.button_down, now)
    }

    pub fn sync_sysmon(&mut self, model: &ShellModel, text: &mut TextSystem, theme: &Theme, now: Instant) -> bool {
        let config = &model.config;
        let cx = self.cx(config, theme, now);
        let mut bar = false;
        for (i, (_, w)) in self.bar.iter_mut().enumerate().filter(|(_, (_, w))| w.kind() == WidgetKind::Sysmon) {
            if w.sync(model, &self.name, &cx) {
                self.bar_damage.mark(i);
                bar = true;
            }
        }
        if bar {
            self.refresh_tooltip(config, text, theme, now);
        }
        let open = self.sysmon_sheet.is_open();
        let view = self.sysmon.set_snapshot(theme, model.sysmon.as_ref(), now);
        if view && open {
            let (target, seed) = self.sysmon_geometry(text, config, theme, now);
            if target != self.sysmon_sheet.target() {
                self.sysmon_sheet.retarget(target, seed, now);
            }
        }
        bar || (view && open)
    }

    /// Kenar çekmecesi yeni açıldı: içerik widget'larının açılış kancaları (bildirim merkezi →
    /// okundu). Dönen eylemleri App uygular.
    pub fn panel_opened(&mut self, panel: PanelId, now: Instant) -> Vec<Action> {
        let PanelId::Edge(e) = panel else { return Vec::new() };
        self.panels[edge_index(e)].iter_mut().filter_map(|w| w.panel_opened(now)).collect()
    }

    /// Başlatıcının açık dikdörtgeni ve tohumu: bar kenarının ortasından, içerik + iç boşluk;
    /// sabit panellerin dışında kalan alanda.
    fn launcher_geometry(&self, config: &ShellConfig, theme: &Theme) -> (Rect, Rect) {
        let edge = config.bar.edge;
        let (_, free) = self.sheet_areas(config);
        let layout = self.layout_in(config, free);
        let content = self.launcher.content_size(theme);
        let pad = 2.0 * launcher_inset(theme);
        let (w, h) = (content.width + pad, content.height + pad);
        let (length, depth) = if edge.is_horizontal() { (w, h) } else { (h, w) };
        let c = layout.bar_content.center();
        let anchor = if edge.is_horizontal() { c.x } else { c.y };
        let p = layout.panel(&PanelSpec { edge, align: PanelAlign::At(anchor), length, depth });
        (p.open, seed_for(edge, p.closed, 0.5))
    }

    /// Başlatıcının istediği ikon adları (App yükleyiciye verir).
    pub fn take_launcher_icon_requests(&mut self) -> Vec<String> {
        self.launcher.take_icon_requests()
    }

    /// Başlatıcının istediği pano küçük resimleri (App yalıtılmış çözücüye verir).
    pub fn take_launcher_thumb_requests(&mut self) -> Vec<crate::launcher_view::ThumbRequest> {
        self.launcher.take_thumb_requests()
    }

    /// Açık başlatıcının kipi (kapalıysa `None`).
    pub fn launcher_mode(&self) -> Option<crate::launcher_view::LauncherMode> {
        self.launcher.mode().filter(|_| self.launcher_sheet.is_open())
    }

    /// Başlatıcı kipini ister: açıksa hemen geçer, kapalıysa bir sonraki açılışta. Yeniden çizim
    /// gerekiyorsa `true`.
    pub fn set_launcher_mode(&mut self, mode: crate::launcher_view::LauncherMode, theme: &Theme, now: Instant) -> bool {
        self.launcher.request_mode(theme, mode, now)
    }

    /// Başlatıcı ikonu çözüldü. Yeniden çizim gerekiyorsa `true`.
    pub fn launcher_icon_loaded(&mut self, name: &str, image: Option<nuxut_image::Image>) -> bool {
        self.launcher.icon_loaded(name, image) && self.launcher_sheet.is_open()
    }

    /// Açık overlay'leri (tepsi menüsü vb.) kapatır. Bir şey kapandıysa `true`.
    pub fn close_overlays(&mut self, now: Instant) -> bool {
        let mut closed = false;
        for (i, (_, w)) in self.bar.iter_mut().enumerate() {
            if w.close_overlay(now) {
                self.bar_damage.mark(i);
                closed = true;
            }
        }
        if closed {
            self.focus_grabbed = false;
        }
        closed
    }

    /// Klavye olayı: önce overlay'ler, sonra açık çekmecenin widget'ları. Tüketilmezse `None`
    /// (çağıran Escape ile yaprağı kapatır).
    pub fn key(
        &mut self,
        text: &mut TextSystem,
        config: &ShellConfig,
        theme: &Theme,
        event: &KeyEvent,
        now: Instant,
    ) -> Option<(bool, Option<Action>)> {
        let cx = self.cx(config, theme, now);
        for (_, w) in &mut self.bar {
            if w.wants_keyboard()
                && let Some(r) = w.key(event, text, &cx)
            {
                return Some(r);
            }
        }
        // Açık yapraklar üstten alta (geçici, sonra sabitler): ilk tüketen kazanır.
        for panel in self.open_panels() {
            match panel {
                PanelId::Clock if event.key != nuxut::Key::Escape => {
                    let (redraw, range) = self.calendar.key(theme, event, now);
                    if redraw || range.is_some() {
                        return Some((redraw, range.map(|(from, to)| Action::CalendarRange { from, to })));
                    }
                }
                // Başlatıcı tüm tuşları alır: Esc önce metni temizler / alt sayfadan döner.
                PanelId::Launcher if self.launcher_sheet.is_open() => {
                    let (redraw, outcome) = self.launcher.key(theme, event, now);
                    return Some((redraw, outcome.map(launcher_action)));
                }
                PanelId::Edge(e) if self.drawers[edge_index(e)].is_open() => {
                    for w in &mut self.panels[edge_index(e)] {
                        if let Some(r) = w.key(event, &cx) {
                            return Some(r);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Pano metni açık kenar panellerinde onu isteyen widget'a (ilk tüketen).
    pub fn paste(&mut self, config: &ShellConfig, theme: &Theme, text: &str, now: Instant) -> Option<(bool, Option<Action>)> {
        // Başlatıcı geçici yaprak: açıksa sabit panellerin üstündedir, metin arama alanına.
        if self.state.transient() == Some(PanelId::Launcher) && self.launcher_sheet.is_open() {
            return Some((self.launcher.paste(theme, text, now), None));
        }
        let cx = self.cx(config, theme, now);
        for panel in self.open_panels() {
            if let PanelId::Edge(e) = panel
                && self.drawers[edge_index(e)].is_open()
                && let Some(r) = self.panels[edge_index(e)].iter_mut().find_map(|w| w.paste(text, &cx))
            {
                return Some(r);
            }
        }
        None
    }

    /// İşaretçi olayı. Döner: (yeniden çizim gerekli mi, eylem).
    pub fn pointer(
        &mut self,
        text: &mut TextSystem,
        config: &ShellConfig,
        theme: &Theme,
        event: PointerEvent,
        now: Instant,
    ) -> (bool, Option<Action>) {
        // Sabitleme düğmeleri her olayı görür (hover'dan çıkış dahil); açık menü varken değil.
        let overlay_open = self.bar.iter().any(|(_, w)| w.wants_keyboard());
        let mut pin_redraw = false;
        if !overlay_open {
            for (i, edge) in Edge::ALL.into_iter().enumerate() {
                let r = self.pins[i].pointer(&event, now);
                pin_redraw |= r.repaint;
                if let Some(selected) = r.event {
                    return (true, Some(Action::PinPanel(edge, selected)));
                }
                if self.pins[i].rect().contains(event.position) && !matches!(event.kind, PointerKind::Leave) {
                    self.cursor = self.pins[i].cursor(event.position).unwrap_or(CursorShape::Default);
                    if let PointerKind::Release(Button::Left) = event.kind {
                        self.button_down = false;
                    }
                    return (pin_redraw, None);
                }
            }
        }
        let (redraw, action) = self.pointer_inner(text, config, theme, event, now);
        (redraw || pin_redraw, action)
    }

    fn pointer_inner(
        &mut self,
        text: &mut TextSystem,
        config: &ShellConfig,
        theme: &Theme,
        event: PointerEvent,
        now: Instant,
    ) -> (bool, Option<Action>) {
        let g = self.geometry(text, config, theme, now);
        let cx = self.cx(config, theme, now);
        self.cursor = self.cursor_at(event.position, &g, &cx, config);

        // 0) Bar uç eylemleri: atanmış girdi uç widget'tan önce gelir. Açık overlay (tepsi
        // menüsü) varken önce o (menü dışına tık menüyü kapatır).
        let overlay_open = self.bar.iter().any(|(_, w)| w.wants_keyboard());
        if !overlay_open && let Some(action) = self.edge_event(&event, &g, config, now) {
            if let PointerKind::Release(Button::Left) = event.kind {
                self.button_down = false;
            }
            self.pointer = Some(event.position);
            let redraw = self.bar_hover(&g, &cx, self.pointer, self.button_down, now);
            return (redraw || action.is_some(), action);
        }

        // 1) Overlay'ler (açık menü) en üstte: olayı tüketirlerse bar'a düşmez.
        for (i, (_, w)) in self.bar.iter_mut().enumerate() {
            if let Some((redraw, action)) = w.overlay_pointer(&event, text, &cx) {
                if redraw {
                    self.bar_damage.mark(i);
                }
                return (redraw || action.is_some(), action);
            }
        }

        // 1b) Bildirim popup'ları (yaprakların üstünde). İşaretçi bar'da değil: bar hover'ları söner.
        if let Some((redraw, action)) = self.popups.pointer(&event, text, &cx) {
            let bar = self.bar_hover(&g, &cx, None, false, now);
            return (redraw || bar || action.is_some(), action);
        }

        // 2a) Açık takvim popout'u: ızgara (gün seçimi, ay değiştirme).
        if self.popout.is_open() && self.popout.target().contains(event.position) {
            let area = self.popout.target();
            let (redraw, range) = self.calendar.pointer(text, theme, area, &event, now);
            if let PointerKind::Release(Button::Left) = event.kind {
                self.button_down = false;
            }
            let action = range.map(|(from, to)| Action::CalendarRange { from, to });
            return (redraw || action.is_some(), action);
        }

        // 2a') Açık sistem izleyici popout'u: etkileşimsiz içerik; olay bar'a/arkaya düşmez.
        if self.sysmon_sheet.is_open() && self.sysmon_sheet.target().contains(event.position) {
            if let PointerKind::Release(Button::Left) = event.kind {
                self.button_down = false;
            }
            let redraw = self.bar_hover(&g, &cx, None, false, now);
            return (redraw, None);
        }

        // 2b) Açık başlatıcı (geçici yaprak; sabit çekmecelerin üstünde).
        if self.launcher_sheet.is_open() && self.launcher_sheet.target().contains(event.position) {
            let (redraw, outcome) = self.launcher.pointer(text, theme, &event, now);
            if let PointerKind::Release(Button::Left) = event.kind {
                self.button_down = false;
            }
            let action = outcome.map(launcher_action);
            return (redraw || action.is_some(), action);
        }

        // 2) Açık çekmecelerin içeriği (üstten alta): işaretçi hangisinin içindeyse ona gider.
        for panel in self.open_panels() {
            let PanelId::Edge(e) = panel else { continue };
            let i = edge_index(e);
            let p = event.position;
            if self.drawers[i].is_open() && self.drawers[i].target().contains(p) {
                let mut redraw = false;
                let mut action = None;
                for (w, r) in self.panels[i].iter_mut().zip(&self.panel_rects[i]) {
                    // Hover/çıkış için her widget olayı görür; tıklama yalnızca isabet edene.
                    let (rd, a) = w.pointer(&event, *r, text, &cx);
                    redraw |= rd;
                    if action.is_none() {
                        action = a;
                    }
                }
                if let PointerKind::Release(Button::Left) = event.kind {
                    self.button_down = false;
                    // Sabit panele tıklama geçici yaprağın dışına tıklamadır: widget eylemi
                    // yoksa geçici yaprak kapanır.
                    if action.is_none()
                        && let Some(t) = self.state.transient()
                        && t != panel
                    {
                        action = Some(Action::ClosePanel(t));
                    }
                }
                return (redraw || action.is_some(), action);
            }
        }

        let mut action = None;
        match event.kind {
            PointerKind::Leave => {
                self.pointer = None;
                self.button_down = false;
            }
            PointerKind::Press(Button::Left) => self.button_down = true,
            PointerKind::Release(Button::Left) => {
                self.button_down = false;
                let p = event.position;
                let hit = self.bar.iter_mut().zip(&g.items).find(|(_, r)| r.contains(p));
                tracing::debug!(x = p.x, y = p.y, hit = ?hit.as_ref().map(|((_, w), r)| (w.kind(), **r)), "tıklama");
                if let Some(((_, w), r)) = hit {
                    action = w.click(*r, p, &cx);
                } else if let Some(open) = self.state.transient() {
                    // Dışarı tıklama yalnızca geçici yaprağı kapatır; sabit panel kalır.
                    let inside = match open {
                        PanelId::Clock => self.popout.target().contains(p),
                        PanelId::Launcher => self.launcher_sheet.target().contains(p),
                        PanelId::Sysmon => self.sysmon_sheet.target().contains(p),
                        PanelId::Edge(e) => self.drawers[edge_index(e)].target().contains(p),
                    };
                    if !inside {
                        action = Some(Action::ClosePanel(open));
                    }
                }
            }
            _ => {}
        }
        if event.kind != PointerKind::Leave {
            self.pointer = Some(event.position);
        }
        let redraw = self.bar_hover(&g, &cx, self.pointer, self.button_down, now);
        (redraw || action.is_some(), action)
    }

    /// Bar widget'larının hover durumu ve uç ipucu ikonlarının görünürlüğü. Yeniden çizim
    /// gerekiyorsa `true` (bar hasarı işaretlenir).
    fn bar_hover(&mut self, g: &BarGeometry, cx: &WidgetCx, pointer: Option<Point>, down: bool, now: Instant) -> bool {
        let mut redraw = false;
        for (i, ((_, w), r)) in self.bar.iter_mut().zip(&g.items).enumerate() {
            if w.hover(*r, pointer, down, cx) {
                self.bar_damage.mark(i);
                redraw = true;
            }
        }
        for (i, (hint, zone)) in self.edge_hints.iter_mut().zip(&g.edges).enumerate() {
            let hovered = zone.is_some_and(|z| z.hint.is_some() && pointer.is_some_and(|p| z.zone.contains(p)));
            if hint.set_hovered(hovered, now) {
                self.bar_damage.mark_hint(i);
                redraw = true;
            }
        }

        // Ortak ipucu: işaretçinin altındaki widget metin veriyorsa (ör. kırpılmış başlık).
        let tip = pointer.and_then(|p| {
            self.bar.iter().zip(&g.items).find(|(_, r)| r.contains(p)).and_then(|((_, w), r)| {
                w.tooltip(*r, p, cx).map(|t| (t.text, t.anchor.unwrap_or(*r)))
            })
        });
        let was_visible = self.tooltip.is_visible();
        match tip {
            Some((text, anchor)) => {
                let side = MenuSide::away_from(screen_edge(g.layout.bar_edge));
                if let Some(at) = self.tooltip.hover(text, anchor, side, cx.bounds, now) {
                    self.wake = earliest(self.wake, Some(at));
                }
            }
            None => self.tooltip.unhover(now),
        }
        // Görünürken yer/metin değişebilir: kare (hasarı bileşen sınırlar).
        redraw | was_visible | self.tooltip.is_visible()
    }

    /// Bekleyen uyanma anı (ipucu gecikmesi ya da son karedeki `Ui::wake_at`): App tüm
    /// ekranların en erkenine tek bir timer kurar ([`crate::app`] `wake.rs`).
    pub fn wake(&self) -> Option<Instant> {
        self.wake
    }

    /// Uyanma anı geldiyse (`<= now`) temizler ve `true` döner: çağıran bu yüzeyde kare ister;
    /// kare yeni anı yeniden hesaplar.
    pub fn take_due_wake(&mut self, now: Instant) -> bool {
        let due = self.wake.is_some_and(|at| at <= now);
        if due {
            self.wake = None;
        }
        due
    }

    /// Uç eylem bölgesindeki işaretçi olayı. `None`: uç eylemi yok (olay normal akışa);
    /// `Some(eylem)`: tüketildi (`Some(None)` = hiçbir şey yapma).
    fn edge_event(&mut self, event: &PointerEvent, g: &BarGeometry, config: &ShellConfig, now: Instant) -> Option<Option<Action>> {
        let i = g.edges.iter().position(|z| z.is_some_and(|z| z.zone.contains(event.position)))?;
        let mut binding = config.bar.edge_actions.side(EdgeSide::ALL[i]).binding;
        // Parlaklık kontrolü yok: `brightness` kaydırması yok sayılır, girdi uç widget'ına kalır.
        if binding.scroll == ScrollAction::Brightness && !self.brightness_available {
            binding.scroll = ScrollAction::Widget;
        }
        if event.kind == PointerKind::ScrollEnd {
            self.edge_scroll[i].end();
        }
        // Öncelik (`EdgeBinding::route`): atanmış eylem uç widget'tan önce; `widget` → widget'a.
        match binding.route(&event.kind)? {
            EdgeRoute::Done(action) => Some(action),
            EdgeRoute::Scroll(action) => {
                let PointerKind::Scroll { delta, steps } = event.kind else { return Some(None) };
                let steps = self.edge_scroll[i].push(delta, steps, now);
                Some((steps != 0).then_some(Action::BarEdge(EdgeCommand::Scroll { action, steps })))
            }
        }
    }

    /// Görünür takvimlerin sorgu aralıkları: açık saat popout'u ve açık/sabit kenar
    /// panellerindeki takvim kartları. `App` bunları tüm ekranlardan toplayıp birleştirir
    /// ([`crate::calendar_view::query_range`]); kapalı yapraktaki takvim sorgu genişletmez.
    pub fn calendar_ranges(&self) -> Vec<(i64, i64)> {
        let mut out = Vec::new();
        if self.state.is_open(PanelId::Clock) {
            out.push(self.calendar.requested_range());
        }
        for (i, edge) in Edge::ALL.into_iter().enumerate() {
            if self.state.is_open(PanelId::Edge(edge)) {
                out.extend(self.panels[i].iter().filter_map(|w| w.calendar_range()));
            }
        }
        out
    }

    /// Sonraki karede tüm yüzey yeniden çizilsin (renk geçişi).
    pub fn invalidate(&mut self) {
        self.damage_all = true;
    }

    /// Son işaretçi olayındaki imleç.
    pub fn cursor(&self) -> CursorShape {
        self.cursor
    }

    /// `p` noktasındaki imleç: en üstteki isabetten başlayarak (overlay → popup → açık çekmece → bar),
    /// ilk karar veren kazanır; hiçbiri değilse `Default` (zemin, çerçeve).
    fn cursor_at(&self, p: Point, g: &BarGeometry, cx: &WidgetCx, config: &ShellConfig) -> CursorShape {
        let overlay = self.bar.iter().map(|(_, w)| w.overlay_cursor(p)).chain([self.popups.cursor(p)]);
        let drawer = self.state.open_panels().find_map(|panel| match panel {
            PanelId::Clock if self.popout.is_open() && self.popout.target().contains(p) => {
                Some(self.calendar.cursor(p).unwrap_or(CursorShape::Default))
            }
            PanelId::Launcher if self.launcher_sheet.is_open() && self.launcher_sheet.target().contains(p) => {
                Some(self.launcher.cursor(p).unwrap_or(CursorShape::Default))
            }
            PanelId::Sysmon if self.sysmon_sheet.is_open() && self.sysmon_sheet.target().contains(p) => Some(CursorShape::Default),
            PanelId::Edge(e) if self.drawers[edge_index(e)].is_open() && self.drawers[edge_index(e)].target().contains(p) => {
                let i = edge_index(e);
                self.panels[i]
                    .iter()
                    .zip(&self.panel_rects[i])
                    .find(|(_, r)| r.contains(p))
                    .and_then(|(w, r)| w.cursor(*r, p, cx))
                    // Çekmece zemini: alttaki bar'a düşmesin.
                    .or(Some(CursorShape::Default))
            }
            _ => None,
        });
        // Uç eylem bölgesi: atanmış sol tık varsa `Pointer`; `widget` ise widget karar verir.
        let edge = g.edges.iter().zip(EdgeSide::ALL).find(|(z, _)| z.is_some_and(|z| z.zone.contains(p))).and_then(|(_, side)| {
            let click = config.bar.edge_actions.side(side).binding.left_click;
            (click != crate::bar_edge::ClickAction::Widget).then(|| if click.is_action() { CursorShape::Pointer } else { CursorShape::Default })
        });
        let bar = edge.or_else(|| self.bar.iter().zip(&g.items).find(|(_, r)| r.contains(p)).and_then(|((_, w), r)| w.cursor(*r, p, cx)));
        CursorShape::pick(overlay.chain([drawer, bar]))
    }

    /// Kareyi çizer. Animasyon sürüyorsa `true` döner (çağıran yeni kare ister).
    pub fn draw(&mut self, painter: &mut Painter, config: &ShellConfig, theme: &Theme, now: Instant) -> bool {
        let g = self.geometry(painter.text_system(), config, theme, now);
        let cx = self.cx(config, theme, now);
        let colors = &theme.colors;

        let frame_moving = !self.insets.is_settled(now) | !self.smoothing.is_settled(now);
        // Animasyondaki bar widget'ları/uç ipuçları (hasar yalnızca onların dikdörtgeni).
        let widgets_animating: Vec<bool> = self.bar.iter_mut().map(|(_, w)| w.is_animating(now)).collect();
        let mut hints_animating = [false; 2];
        for (a, h) in hints_animating.iter_mut().zip(&mut self.edge_hints) {
            *a = h.is_animating(now);
        }
        let bar_moving = widgets_animating.iter().chain(&hints_animating).any(|a| *a);
        // Vurgusu değişen widget kirli (yaprak açıldı/kapandı); sayı değiştiyse yerleşim de
        // değişmiştir, BarDamage tüm şeridi zaten verir.
        let highlight: Vec<bool> = self.bar.iter().map(|(_, w)| w.panel(&cx).is_some_and(|p| self.state.is_open(p))).collect();
        if highlight.len() == self.bar_highlight.len() {
            for (i, (new, old)) in highlight.iter().zip(&self.bar_highlight).enumerate() {
                if new != old {
                    self.bar_damage.mark(i);
                }
            }
        }
        self.bar_highlight.clone_from(&highlight);

        // --- Blob alanı: çerçeve + açık/kapanan yapraklar
        let mut shapes = Vec::new();
        let mut owners = Vec::new();
        if let Some(s) = self.popout.shape(now) {
            shapes.push(s);
            owners.push(None);
        }
        for (i, d) in self.drawers.iter_mut().enumerate() {
            if let Some(s) = d.shape(now) {
                shapes.push(s);
                owners.push(Some(i));
            }
        }
        if let Some(s) = self.launcher_sheet.shape(now) {
            shapes.push(s);
            owners.push(Some(LAUNCHER_OWNER));
        }
        if let Some(s) = self.sysmon_sheet.shape(now) {
            shapes.push(s);
            owners.push(Some(SYSMON_OWNER));
        }
        // Overlay şekilleri (tepsi menüsü…) aynı alana: bar'dan blob olarak büyür, fileto ile birleşir.
        let sheet_shapes = shapes.len();
        for (_, w) in &mut self.bar {
            shapes.extend(w.overlay_shapes(&cx));
        }
        // Bildirim popup'ları: geçici çekmece ya da bildirim merkezli bir çekmece açıkken
        // (merkez orada) çöker; sabit panellerin dışındaki alana yerleşir.
        let hide_popups = Edge::ALL.into_iter().enumerate().any(|(i, e)| {
            self.drawers[i].is_open()
                && (self.state.transient() == Some(PanelId::Edge(e))
                    || self.panels[i].iter().any(|w| w.kind() == WidgetKind::Notifications))
        });
        self.popups.set_hidden(hide_popups);
        let popup_area = popup_area(self.sheet_areas(config).1, config.bar.edge);
        shapes.extend(self.popups.shapes(painter, popup_area, config.bar.edge, &cx));
        shapes.extend(self.osd.shape(now, theme));
        let ins = g.layout.insets;
        let has_frame = ins.top.max(ins.right).max(ins.bottom).max(ins.left) > EPSILON
            || self.smoothing.value(now) > EPSILON;
        let shadow = theme.shadow.enabled.then(|| ShadowStyle {
            color: theme.shadow.color,
            blur: theme.shadow.blur,
            offset: Point::new(0.0, theme.shadow.offset_y),
            spread: 0.0,
        });
        let field = BlobField {
            frame: has_frame.then(|| g.layout.frame.shape()),
            shapes,
            smoothing: self.smoothing.value(now),
            corner_radius: theme.shape.radius_large,
            fill: colors.surface,
            shadow,
        };

        // --- Hasar: çizimden önce. Çerçeve hareket ediyorsa tüm yüzey; aksi halde her
        // yaprağın eski ∪ yeni etki alanı ve değişen bar şeridi.
        if std::mem::take(&mut self.damage_all) || frame_moving {
            painter.damage_all();
        }
        // Sıra: popout, 4 çekmece, başlatıcı, sistem izleyici (`owner + 1`).
        let mut current: [Option<Rect>; 7] = [None; 7];
        for (shape, owner) in field.shapes.iter().zip(&owners) {
            current[owner.map_or(0, |i| i + 1)] = Some(field.damage_rect(shape));
        }
        let sheets = std::iter::once(&mut self.popout)
            .chain(&mut self.drawers)
            .chain(std::iter::once(&mut self.launcher_sheet))
            .chain(std::iter::once(&mut self.sysmon_sheet));
        for (i, sheet) in sheets.enumerate() {
            for r in [sheet.last_damage, current[i]].into_iter().flatten() {
                painter.damage(r);
            }
            sheet.last_damage = current[i];
        }
        let overlay_damage: Vec<Rect> = field.shapes[sheet_shapes..].iter().map(|s| field.damage_rect(s)).collect();
        for r in self.last_overlay_damage.iter().chain(&overlay_damage) {
            painter.damage(*r);
        }
        self.last_overlay_damage = overlay_damage;
        // Bar: yerleşim (şerit, widget/uç dikdörtgenleri) değiştiyse tüm şerit; değilse yalnızca
        // değişen ya da animasyondaki widget'ların dikdörtgeni (saat tıkı ≠ tüm bar).
        let bar_frame = BarFrame {
            strip: g.layout.bar,
            items: g.items.clone(),
            zones: g.edges.map(|z| z.map(|z| z.zone)),
            gap: config.bar.spacing,
            vertical: config.bar.is_vertical(),
        };
        for r in self.bar_damage.take(bar_frame, &widgets_animating, hints_animating) {
            painter.damage(r);
        }

        if field.frame.is_some() || !field.shapes.is_empty() {
            painter.blob(&field);
        }

        // --- Bar içeriği: bar şeridine kırpılır.
        let strip = g.layout.bar;
        if strip.height > EPSILON && strip.width > EPSILON {
            painter.push_clip(strip);
            for (((_, w), r), highlighted) in self.bar.iter_mut().zip(&g.items).zip(&highlight) {
                if !r.is_empty() {
                    w.draw(painter, *r, &cx, *highlighted);
                }
            }
            for (hint, zone) in self.edge_hints.iter_mut().zip(&g.edges) {
                if let Some(z) = zone
                    && let Some(slot) = z.hint
                {
                    hint.draw(painter, slot, z.toward, theme, now);
                }
            }
            painter.pop_clip();
        }

        // --- Yaprak içerikleri. Sabitleme düğmeleri yalnızca içeriği görünen çekmecede konur.
        let mut content_wants_frame = false;
        // Bu karedeki her `Ui`'nin uyanma anı (ipucu gecikmesi, Slider balonunun sönmesi,
        // CurveEditor beklemesi): en erkeni App'in tek timer'ına. Tüketilmezse zamanlı bileşen
        // bir sonraki olaya kadar donar (framework sözleşmesi).
        let mut wake = None;
        for pin in &mut self.pins {
            pin.set_rect(Rect::ZERO);
        }
        for (shape, owner) in field.shapes.iter().zip(&owners) {
            let visual = shape.visual_rect();
            match owner {
                None => {
                    let alpha = self.popout.content_alpha(visual, now);
                    if alpha > 0.003 {
                        let area = self.popout.content_rect(visual);
                        self.calendar_area = area;
                        let lift = self.popout.lift_direction() * ((1.0 - alpha) * theme.space(2));
                        painter.push_clip(visual);
                        {
                            let mut ui = Ui::new(painter, theme, now).with_damage_tracking(true);
                            ui.with_alpha(alpha, |ui| self.calendar.draw(ui, area.translate(lift), theme));
                            content_wants_frame |= ui.wants_frame();
                            wake = earliest(wake, ui.wake_deadline());
                        }
                        painter.pop_clip();
                    }
                }
                Some(LAUNCHER_OWNER) => {
                    let sheet = &self.launcher_sheet;
                    let alpha = sheet.content_alpha(visual, now);
                    if alpha > 0.003 {
                        let lift = sheet.lift_direction() * ((1.0 - alpha) * theme.space(2));
                        let area = sheet.content_rect(visual).inset(Insets::uniform(launcher_inset(theme))).translate(lift);
                        painter.push_clip(visual);
                        {
                            let mut ui = Ui::new(painter, theme, now).with_damage_tracking(true);
                            ui.with_alpha(alpha, |ui| self.launcher.draw(ui, area));
                            content_wants_frame |= ui.wants_frame();
                        }
                        painter.pop_clip();
                    }
                }
                Some(SYSMON_OWNER) => {
                    let sheet = &self.sysmon_sheet;
                    let alpha = sheet.content_alpha(visual, now);
                    if alpha > 0.003 {
                        let lift = sheet.lift_direction() * ((1.0 - alpha) * theme.space(2));
                        let area = sheet.content_rect(visual).translate(lift);
                        painter.push_clip(visual);
                        {
                            let mut ui = Ui::new(painter, theme, now).with_damage_tracking(true);
                            ui.with_alpha(alpha, |ui| self.sysmon.draw(ui, area, theme));
                            content_wants_frame |= ui.wants_frame();
                            wake = earliest(wake, ui.wake_deadline());
                        }
                        painter.pop_clip();
                    }
                }
                Some(i) => {
                    let i = *i;
                    let sheet = &self.drawers[i];
                    let alpha = sheet.content_alpha(visual, now);
                    if alpha <= 0.003 {
                        self.panel_rects[i].clear();
                        continue;
                    }
                    let lift = sheet.lift_direction() * ((1.0 - alpha) * theme.space(2));
                    let content = sheet.content_rect(visual).translate(lift);
                    let area = content.inset(Insets::uniform(theme.space(2)));
                    // Sabitleme düğmesi içeriğin üstünde yüzer (yer ayırmaz); içerikten sonra çizilir.
                    let corner = panels::pin_corner(sheet.edge(), config.bar.edge);
                    let pin_rect = panels::pin_rect(content, corner, theme.space(PIN_SIZE_GRID));
                    self.pins[i].set_rect(pin_rect);
                    let mut rects = stack_panel_widgets(&mut self.panels[i], painter.text_system(), area, theme.space(2), &cx);
                    // Düğmenin altındaki widget'ın içeriği düğmeye değecekse yalnızca o köşenin
                    // tarafında küçük bir boşluk: yığın o kadar itilir.
                    let shift = self.panels[i]
                        .iter()
                        .zip(&rects)
                        .map(|(w, r)| panels::pin_clearance_shift(pin_rect, corner, *r, w.corner_clearance(&cx)))
                        .fold(0.0_f32, f32::max);
                    if shift > 0.0 {
                        let top = if corner.bottom { 0.0 } else { shift };
                        let pushed = Rect::new(area.x, area.y + top, area.width, (area.height - shift).max(0.0));
                        rects = stack_panel_widgets(&mut self.panels[i], painter.text_system(), pushed, theme.space(2), &cx);
                    }
                    painter.push_clip(visual);
                    {
                        let mut ui = Ui::new(painter, theme, now).with_damage_tracking(true);
                        let pin = &mut self.pins[i];
                        ui.with_alpha(alpha, |ui| {
                            for (w, rect) in self.panels[i].iter_mut().zip(&rects) {
                                if !rect.is_empty() {
                                    w.draw(ui, *rect, &cx);
                                }
                            }
                            pin.draw(ui);
                        });
                        content_wants_frame |= ui.wants_frame();
                        wake = earliest(wake, ui.wake_deadline());
                    }
                    painter.pop_clip();
                    self.panel_rects[i] = rects;
                }
            }
        }

        // --- Popup kartları, sonra overlay içerikleri en üstte (menü, popover).
        {
            let mut ui = Ui::new(painter, theme, now).with_damage_tracking(true);
            self.popups.draw(&mut ui, &cx);
            for (_, w) in &mut self.bar {
                w.draw_overlay(&mut ui, &cx);
            }
            self.osd.draw_content(&mut ui);
            // Klavye gezinmesinin odak halkası (screen/access.rs).
            self.draw_nav_focus(&mut ui);
            // İpucu en üstte; girdi bölgesine eklenmez.
            self.tooltip.draw(&mut ui);
            content_wants_frame |= ui.wants_frame();
            wake = earliest(wake, ui.wake_deadline());
        }
        // Kare uyanma anını baştan hesaplar (bekleyenler çizimde yeniden bildirir).
        self.wake = wake;

        // `bar_moving` çizimden önce (hasar için) soruldu; çizim sırasında başlayan bir bar
        // animasyonu da sonraki kareyi istemeli, yoksa ilk karesinde donar.
        frame_moving
            | bar_moving
            | self.bar.iter_mut().any(|(_, w)| w.is_animating(now))
            | self.edge_hints.iter_mut().any(|h| h.is_animating(now))
            | content_wants_frame
            | self.panels.iter_mut().flatten().fold(false, |acc, w| acc | w.is_animating(now))
            | self.popout.is_animating(now)
            | self.popups.is_animating(now)
            | self.launcher_sheet.is_animating(now)
            | self.sysmon_sheet.is_animating(now)
            | self.launcher.is_animating(now)
            | self.drawers.iter_mut().fold(false, |acc, d| acc | d.is_animating(now))
    }
}

/// Panel widget'larını dikey yığar: sabit yükseklikliler sırayla, esnek (`None`) olan(lar)
/// kalan alanı eşit paylaşır. Taşan widget kırpılır (boş dikdörtgen).
fn stack_panel_widgets(
    widgets: &mut [Box<dyn PanelWidget>],
    text: &mut TextSystem,
    area: Rect,
    gap: f32,
    cx: &WidgetCx,
) -> Vec<Rect> {
    let heights: Vec<Option<f32>> = widgets.iter_mut().map(|w| w.height(text, area.width, cx)).collect();
    let fixed: f32 = heights.iter().flatten().sum();
    let visible = heights.iter().filter(|h| h.is_none_or(|v| v > 0.0)).count();
    let gaps = gap * visible.saturating_sub(1) as f32;
    let flex_count = heights.iter().filter(|h| h.is_none()).count().max(1) as f32;
    let flex = ((area.height - fixed - gaps) / flex_count).max(0.0);
    let mut y = area.y;
    heights
        .iter()
        .map(|h| {
            let hh = h.unwrap_or(flex);
            if hh <= 0.0 {
                return Rect::ZERO;
            }
            let hh = hh.min((area.bottom() - y).max(0.0));
            let r = Rect::new(area.x, y, area.width, hh);
            y += hh + gap;
            r
        })
        .collect()
}

/// Yaprağın hedef dikdörtgeni (tam sayı): canlı testte yalnızca kendi yüzeyini kırpmak için.
fn log_sheet_target(name: &str, r: Rect) {
    tracing::debug!(
        "yaprak hedefi {name} geom={},{} {}x{}",
        r.x.round() as i32,
        r.y.round() as i32,
        r.width.round() as i32,
        r.height.round() as i32
    );
}

/// Kenar çekmecesinin açık dikdörtgeni ve tohumu: kenar boyunca iç alanın tamamı, derinlik
/// `panels.<kenar>.size`.
fn drawer_geometry(layout: &ShellLayout, edge: Edge, config: &ShellConfig) -> (Rect, Rect) {
    let spec = PanelSpec { edge, align: PanelAlign::Start, length: f32::MAX, depth: config.panels.get(edge).size };
    let p = layout.panel(&spec);
    (p.open, seed_for(edge, p.closed, 1.0))
}

/// Bildirim popup sütunu: bar kenarındaki iç alanın (`WidgetCx::bounds`) end köşesi (yatay
/// bar'da sağ, dikey bar'da alt), genişlik `POPUP_WIDTH`.
/// OSD bar'ın karşı kenarının ortasından, çerçeveye yapışık büyür (bar'daki yaprak ve
/// popup'larla çakışmaz).
fn osd_anchor(inner: Rect, bar: Edge) -> MenuAnchor {
    let c = inner.center();
    let (point, side) = match bar {
        Edge::Top => (Point::new(c.x, inner.bottom()), MenuSide::Above),
        Edge::Bottom => (Point::new(c.x, inner.y), MenuSide::Below),
        Edge::Left => (Point::new(inner.right(), c.y), MenuSide::Left),
        Edge::Right => (Point::new(inner.x, c.y), MenuSide::Right),
    };
    MenuAnchor { rect: Rect::new(point.x, point.y, 0.0, 0.0), side, align: MenuAlign::Center, gap: 0.0 }
}

fn popup_area(inner: Rect, edge: Edge) -> Rect {
    let w = POPUP_WIDTH.min(inner.width).max(0.0);
    let x = match edge {
        Edge::Left => inner.x,
        _ => inner.right() - w,
    };
    Rect::new(x, inner.y, w, inner.height)
}

/// Çizimde şekil sahibi: çekmeceler `0..4`, başlatıcı bu değer (popout `None`).
const LAUNCHER_OWNER: usize = 4;
/// Sistem izleyici popout'unun şekil sahibi.
const SYSMON_OWNER: usize = 5;

/// Başlatıcı yaprağının iç boşluğu.
fn launcher_inset(theme: &Theme) -> f32 {
    theme.space(4)
}

fn launcher_action(outcome: LauncherOutcome) -> Action {
    match outcome {
        LauncherOutcome::Launch { entry, action } => Action::Launch { entry, action },
        LauncherOutcome::Close => Action::ClosePanel(PanelId::Launcher),
        LauncherOutcome::Copy(t) => Action::Copy(t),
        LauncherOutcome::RequestPaste => Action::RequestPaste,
        LauncherOutcome::Clipboard(c) => Action::Clipboard(c),
        LauncherOutcome::Emoji(e) => Action::PickEmoji(e),
    }
}

fn edge_index(e: Edge) -> usize {
    match e {
        Edge::Top => 0,
        Edge::Right => 1,
        Edge::Bottom => 2,
        Edge::Left => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_index_matches_edge_all_order() {
        for (i, e) in Edge::ALL.into_iter().enumerate() {
            assert_eq!(edge_index(e), i);
        }
    }

    use BarSection::{Center, End, Start};

    #[test]
    fn edge_hint_slots_are_reserved_outside_edge_widgets() {
        let c = ShellConfig::validated_default();
        let l = ShellLayout::compute(&c, Size::new(1920.0, 1080.0));
        let secs = [Start, Start, Center, End, End];
        let lens = [100.0, 50.0, 80.0, 30.0, 60.0];
        let (gap, item) = (12.0, 18.0);

        let (plain, edges) = arrange_bar(&l, &secs, &lens, gap, [false, false], item);
        assert_eq!(plain[0].x, 16.0);
        assert_eq!(plain[4].right(), 1904.0);
        // Uç bölgesi = uç widget (yuva yok).
        assert_eq!(edges[0].unwrap().zone, plain[0]);
        assert_eq!(edges[1].unwrap().zone, plain[4]);
        assert!(edges[0].unwrap().hint.is_none());

        let (items, edges) = arrange_bar(&l, &secs, &lens, gap, [true, true], item);
        let (s, e) = (edges[0].unwrap(), edges[1].unwrap());
        // Yuva dış uçta, widget'lar içeri kayar ama orta bölüm yerinde.
        assert_eq!(s.hint.unwrap(), Rect::new(16.0, l.bar_content.y, item, l.bar_content.height));
        assert_eq!(items[0].x, 16.0 + item + gap);
        assert_eq!(e.hint.unwrap().right(), 1904.0);
        assert_eq!(items[4].right(), 1904.0 - item - gap);
        assert_eq!(items[2], plain[2]);
        assert_eq!(s.zone, items[0].union(&s.hint.unwrap()));
        assert_eq!(s.toward, Point::new(1.0, 0.0));
        assert_eq!(e.toward, Point::new(-1.0, 0.0));
        // Yuva baştan ayrıldığı için yerleşim hover'dan bağımsızdır (ikon widget'ı itmez).
        assert_eq!(arrange_bar(&l, &secs, &lens, gap, [true, true], item).0, items);
    }

    #[test]
    fn wake_deadlines_merge_to_earliest() {
        let t = Instant::now();
        let later = t + std::time::Duration::from_millis(500);
        assert_eq!(earliest(None, None), None);
        assert_eq!(earliest(Some(later), None), Some(later));
        assert_eq!(earliest(None, Some(t)), Some(t));
        assert_eq!(earliest(Some(later), Some(t)), Some(t), "kaç Ui olursa olsun en erken an");
    }

    #[test]
    fn brightness_hint_needs_capability() {
        assert!(!hint_wanted(ScrollAction::Brightness, false), "parlaklık kontrolü yoksa yer ayrılmaz");
        assert!(hint_wanted(ScrollAction::Brightness, true));
        assert!(hint_wanted(ScrollAction::Volume, false));
        assert!(!hint_wanted(ScrollAction::Widget, true) && !hint_wanted(ScrollAction::None, true));
        for (e, s) in [(Edge::Top, ScreenEdge::Top), (Edge::Right, ScreenEdge::Right), (Edge::Bottom, ScreenEdge::Bottom), (Edge::Left, ScreenEdge::Left)] {
            assert_eq!(screen_edge(e), s);
        }
    }

    #[test]
    fn edge_zones_follow_visibility_and_vertical_axis() {
        let c = ShellConfig::parse("[bar]\nedge = \"left\"\nsize = 40\n").unwrap().0;
        let l = ShellLayout::compute(&c, Size::new(1920.0, 1080.0));
        // Start'taki ilk widget gizli (0): ilk görünür olan uçtur. End bölümü boş: bölge yok.
        let secs = [Start, Start, Center];
        let (items, edges) = arrange_bar(&l, &secs, &[0.0, 40.0, 40.0], 8.0, [true, true], 24.0);
        let s = edges[0].unwrap();
        assert!(items[0].is_empty());
        assert_eq!(s.hint.unwrap().y, l.bar_content.y);
        assert_eq!(s.hint.unwrap().height, 24.0);
        assert_eq!(items[1].y, l.bar_content.y + 24.0 + 8.0);
        assert_eq!(s.toward, Point::new(0.0, 1.0));
        assert!(edges[1].is_none());
    }

    #[test]
    fn drawer_spans_inner_length() {
        let c = ShellConfig::validated_default();
        let l = ShellLayout::compute(&c, Size::new(1920.0, 1080.0));
        let (target, seed) = drawer_geometry(&l, Edge::Right, &c);
        assert_eq!(target, Rect::new(1914.0 - 380.0, 34.0, 380.0, 1040.0));
        assert_eq!(seed, Rect::new(1915.0, 34.0, 0.0, 1040.0));
    }
}
