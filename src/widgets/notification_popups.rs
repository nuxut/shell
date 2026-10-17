//! Bildirim popup yığını: bar kenarının "end" köşesinden (üst bar'da sağ üst, bar'ın altı)
//! blob olarak büyüyen tek bir yaprak (`Sheet`), içinde yayla dizilen kartlar.
//!
//! - Yaprak, kartların toplam yüksekliğine yayla uyar; yeni kart gelince büyür, kart gidince
//!   (kaldırılma kararıyla, kartın uçarak çıkışıyla aynı anda) küçülür, son kart gidince bar
//!   şeridine geri çöker. Açık yığın büyürken içerik hiçbir karede gizlenmez
//!   (`Sheet::is_revealed`, `docs/design/panels.md` §4.1).
//! - Kartlar `CardSet` ile kimlikle korunur. Kart kurma, ölçme ve doku yükleme `shapes`'te,
//!   yaprak hedeflenmeden önce yapılır (Painter orada): yeni kartın ölçüsü ve yaprağın yeni
//!   hedefi aynı karede; `draw` yalnız çizer.
//! - İşaretçi kartın üzerindeyse `Hover{id, true}` (model aynı anda tek popup'ı "üzerinde"
//!   sayar), yığından çıkınca `Hover{id, false}`. Kaydırma → `HidePopup`, X → `Dismiss`.

use std::time::Instant;

use nuxut::ui::Ui;
use nuxut::{BlobShape, CursorShape, Painter, Point, PointerEvent, PointerKind, Rect, TextSystem};

use super::notification_cards::CardSet;
use super::{Action, NotifyCommand, WidgetCx};
use crate::layout::Edge;
use crate::model::ShellModel;
use crate::sheet::{Sheet, seed_for};

/// Yaprak gövdesini çevreleyen iç boşluk (`theme.space` birimi).
const PAD: u32 = 2;
/// Kartlar arası boşluk (`theme.space` birimi).
const GAP: u32 = 2;

/// Popup kabının açık hâldeki dikdörtgeni. `area`: popup sütunu (bar kenarındaki iç alanın
/// end köşesi); yatay bar'da kap bar'a yapışık uçtan, dikey bar'da alttan büyür. Kap alana
/// sığacak kadar kısalır.
pub fn container_rect(area: Rect, edge: Edge, height: f32) -> Rect {
    let h = height.clamp(0.0, area.height.max(0.0));
    match edge {
        Edge::Top => Rect::new(area.x, area.y, area.width, h),
        Edge::Bottom | Edge::Left | Edge::Right => Rect::new(area.x, area.bottom() - h, area.width, h),
    }
}

/// Kabın bar tarafındaki kapalı hâli (derinlik 0).
pub fn closed_rect(container: Rect, edge: Edge) -> Rect {
    match edge {
        Edge::Top => Rect::new(container.x, container.y, container.width, 0.0),
        Edge::Bottom => Rect::new(container.x, container.bottom(), container.width, 0.0),
        Edge::Left => Rect::new(container.x, container.y, 0.0, container.height),
        Edge::Right => Rect::new(container.right(), container.y, 0.0, container.height),
    }
}

/// Kartların yerleşim dikdörtgeni (hedef boyutta). Yaprak hedefinden kısayken (ilk açılış)
/// bar'dan uzak kenara yapışır, yaprakla birlikte iner; hedefinden uzunken (kart kalktı, yaprak
/// küçülüyor) bar tarafına yapışır: kalan kartlar yerinde durur, boşluğu yığın yayı kapatır.
/// (`Sheet::content_rect` hep uzak kenara yapışır; küçülmede kalan kart önce aşağı sıçrardı.)
///
/// `revealed` (içerik bu açılışta bir kez tam göründü, `Sheet::is_revealed`): üst bar'da
/// büyüme de bar tarafına yapışır. Yeni kart gelince yığın kökeni yerinde kalır, yeni kart
/// kendi girişini oynatır, eskiler yığın yayıyla kayar; uzak kenara yapışsaydı eski kartlar
/// yeni kartın boyu kadar yukarı sıçrayıp yaprakla geri inerdi.
fn content_rect(visual: Rect, target: Rect, edge: Edge, revealed: bool) -> Rect {
    let (w, h) = (target.width, target.height);
    match edge {
        Edge::Top if revealed => Rect::new(visual.x, visual.y, visual.width, h),
        Edge::Top => Rect::new(visual.x, visual.y.min(visual.bottom() - h), visual.width, h),
        Edge::Bottom => Rect::new(visual.x, visual.y.max(visual.bottom() - h), visual.width, h),
        Edge::Left => Rect::new(visual.x.min(visual.right() - w), visual.y, w, visual.height),
        Edge::Right => Rect::new(visual.x.max(visual.right() - w), visual.y, w, visual.height),
    }
}

#[derive(Debug)]
pub struct NotificationPopups {
    cards: CardSet,
    sheet: Sheet,
    /// Son yerleşimde kartların toplam (hedef) yüksekliği.
    stack_height: f32,
    /// Son karedeki görünen yaprak dikdörtgeni (çizim ve isabet için).
    visual: Option<Rect>,
    /// Yerleşim yenilenmeli (sync sonrası, tema, alan değişimi).
    dirty: bool,
    /// İşaretçinin üzerinde olduğu popup (Hover komutu için).
    hovered: Option<u32>,
    area: Rect,
    /// Gizli (kenar çekmecesi açık: merkez zaten gösteriyor): yaprak çöker, kartlar korunur.
    hidden: bool,
    edge: Edge,
}

impl NotificationPopups {
    pub fn new(cx: &WidgetCx) -> Self {
        Self {
            cards: CardSet::new(cx.theme, true),
            sheet: Sheet::new(cx.theme, Edge::Top),
            stack_height: 0.0,
            visual: None,
            dirty: false,
            hovered: None,
            area: Rect::ZERO,
            hidden: false,
            edge: Edge::Top,
        }
    }

    /// Model değişti (popup eklendi/kalktı/güncellendi). Görünüm değiştiyse `true`: bir kare
    /// çizilmeli (kartlar o karenin `shapes`'inde kurulup ölçülür, yaprak aynı karede uyar).
    pub fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let changed = self.cards.sync(model.notifications.popups(), cx.now);
        if changed {
            self.dirty = true;
            if self.hovered.is_some_and(|id| !model.notifications.is_popup(id)) {
                self.hovered = None;
            }
        }
        changed
    }

    pub fn set_theme(&mut self, cx: &WidgetCx) {
        self.cards.set_theme(cx.theme, cx.now);
        self.sheet.set_theme(cx.theme, cx.now);
        self.dirty = true;
    }

    /// Dil değişti: kartlar yeni katalogla yeniden kurulur, yerleşim yenilenir.
    pub fn relocalize(&mut self) {
        self.cards.relocalize();
        self.dirty = true;
    }

    /// Dakikalık zaman metni ("şimdi" → "1 dk"). Değiştiyse `true`.
    pub fn set_time(&mut self) -> bool {
        self.cards.refresh_time(std::time::SystemTime::now())
    }

    fn pad(cx: &WidgetCx) -> f32 {
        cx.theme.space(PAD)
    }

    /// Kartların genişliği (popup sütunu eksi iç boşluk).
    fn card_width(&self, cx: &WidgetCx) -> f32 {
        (self.area.width - 2.0 * Self::pad(cx)).max(0.0)
    }

    /// Bu karedeki blob şekli (shell kendi alanına ekler). `area`: popup sütunu. Eksik kartlar
    /// burada kurulup ölçülür ve yığın dizilir; yaprak yeni yüksekliğe aynı karede yönelir
    /// (eskiden ölçüm `draw`'daydı, yaprak bir kare geriden geliyordu).
    pub fn shapes(&mut self, painter: &mut Painter, area: Rect, edge: Edge, cx: &WidgetCx) -> Vec<BlobShape> {
        let now = cx.now;
        if edge != self.edge {
            self.sheet.set_edge(edge);
            self.edge = edge;
        }
        if area != self.area {
            self.area = area;
            self.dirty = true;
        }
        let width = self.card_width(cx);
        self.cards.prepare(painter, cx.theme, width);
        self.stack_height = self.cards.layout(cx.theme.space(GAP), now);
        self.cards.reap(now);
        self.dirty = false;
        let has_cards = !self.hidden && !self.cards.entries.is_empty() && self.stack_height > 0.0;
        if has_cards {
            let target = container_rect(area, edge, self.stack_height + 2.0 * Self::pad(cx));
            let seed = seed_for(edge, closed_rect(target, edge), 1.0);
            if self.sheet.is_open() {
                self.sheet.retarget(target, seed, now);
            } else {
                self.sheet.open(target, seed, now);
            }
        } else {
            self.sheet.close(now);
        }
        let shape = self.sheet.shape(now);
        self.visual = shape.as_ref().map(BlobShape::visual_rect);
        shape.into_iter().collect()
    }

    /// Yığını gizler/gösterir. Değiştiyse `true`.
    pub fn set_hidden(&mut self, hidden: bool) -> bool {
        std::mem::replace(&mut self.hidden, hidden) != hidden
    }

    /// Kartları çizer (şekiller çizildikten sonra; yerleşim `shapes`'te yapıldı).
    pub fn draw(&mut self, ui: &mut Ui, cx: &WidgetCx) {
        let now = ui.now;
        let pad = Self::pad(cx);
        let width = self.card_width(cx);
        let Some(visual) = self.visual else { return };
        let alpha = self.sheet.content_alpha(visual, now);
        if alpha <= 0.0 {
            return;
        }
        let content = content_rect(visual, self.sheet.target(), self.edge, self.sheet.is_revealed());
        let origin = Point::new(content.x + pad, content.y + pad);
        let places: Vec<(u32, Rect, f32)> = self
            .cards
            .entries
            .iter()
            .filter_map(|e| {
                let slot = self.cards.slot(e, now)?;
                Some((e.id(), Rect::new(origin.x, origin.y + slot.y, width, e.height()), slot.alpha))
            })
            .collect();
        ui.painter.push_clip(visual);
        ui.with_alpha(alpha, |ui| {
            self.cards.draw(ui, |e| places.iter().find(|p| p.0 == e.id()).map(|p| (p.1, p.2)));
        });
        ui.painter.pop_clip();
    }

    /// Tıklama alan dikdörtgenler (input bölgesi). Yaprak açıkken hedef dikdörtgeni.
    pub fn input_rects(&self) -> Vec<Rect> {
        if self.sheet.is_open() { vec![self.sheet.target()] } else { Vec::new() }
    }

    fn contains(&self, p: Point) -> bool {
        self.sheet.is_open() && self.sheet.target().contains(p)
    }

    /// İşaretçi. Yığının üzerindeyse ya da üzerinden yeni çıkıyorsa tüketir:
    /// `Some((yeniden çiz, eylem))`; ilgisizse `None`.
    pub fn pointer(
        &mut self,
        event: &PointerEvent,
        _text: &mut TextSystem,
        cx: &WidgetCx,
    ) -> Option<(bool, Option<Action>)> {
        let inside = event.kind != PointerKind::Leave && self.contains(event.position);
        if !inside && self.hovered.is_none() {
            return None;
        }
        let (repaint, command) = self.cards.pointer(cx.theme, event, cx.now, true);
        let now_hovered = if inside { self.cards.card_at(event.position) } else { None };
        let hover = match (self.hovered, now_hovered) {
            (old, new) if old == new => None,
            (_, Some(id)) => Some(NotifyCommand::Hover { id, hovered: true }),
            (Some(id), None) => Some(NotifyCommand::Hover { id, hovered: false }),
            (None, None) => None,
        };
        self.hovered = now_hovered;
        // Komut (tık, kapatma) hover'dan önce gelir; hover bir sonraki harekette yeniden
        // değerlendirilir.
        let action = command.or(hover).map(Action::Notify);
        Some((repaint, action))
    }


    /// Ekran okuyucu (ADR-0011): görünen açılır kartlar canlı bölge (`Live::Polite`), kabuğun
    /// odağını almadan Orca'ya duyurulur. Gizliyken (kenar çekmecesi açık) ağaçta yok.
    pub fn access(&self, tree: &mut nuxut::access::AccessTree, label: &str) {
        let Some(visual) = self.visual.filter(|_| !self.hidden) else { return };
        let id = tree.push("popups", nuxut::access::Role::Group).label(label).bounds(visual).id();
        tree.within(id, |tree| self.cards.access(tree, visual, true));
    }

    /// AT tıklaması açılır karta mı: işaretçiyle aynı komut.
    pub fn access_action(&mut self, request: &nuxut::access::AccessRequest, now: Instant) -> Option<Action> {
        let parent = nuxut::access::root_child("popups");
        self.cards.access_action(parent, request, now, true).map(Action::Notify)
    }
    /// İmleç: kart (gövde Pointer, düğmeler, kaydırırken Grabbing); yığın zemini Default.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.contains(p) {
            return self.cards.cursor(p);
        }
        Some(self.cards.cursor(p).unwrap_or(CursorShape::Default))
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        self.dirty | self.sheet.is_animating(now) | self.cards.is_animating(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(1500.0, 40.0, 380.0, 1000.0);

    #[test]
    fn container_hugs_bar_side_end_corner() {
        assert_eq!(container_rect(AREA, Edge::Top, 200.0), Rect::new(1500.0, 40.0, 380.0, 200.0));
        assert_eq!(container_rect(AREA, Edge::Bottom, 200.0), Rect::new(1500.0, 840.0, 380.0, 200.0));
        // Dikey bar: end köşesi alt.
        assert_eq!(container_rect(AREA, Edge::Right, 200.0), Rect::new(1500.0, 840.0, 380.0, 200.0));
        assert_eq!(container_rect(AREA, Edge::Left, 200.0).bottom(), AREA.bottom());
        // Alandan uzun yığın kısalır.
        assert_eq!(container_rect(AREA, Edge::Top, 5000.0).height, 1000.0);
        assert_eq!(container_rect(AREA, Edge::Top, -5.0).height, 0.0);
    }

    #[test]
    fn content_rides_far_edge_when_growing_and_bar_side_when_shrinking() {
        let target = Rect::new(1500.0, 40.0, 380.0, 100.0);
        // Açılış: yaprak hedefinden kısa, içerik uzak kenarla iner.
        let growing = Rect::new(1500.0, 40.0, 380.0, 60.0);
        assert_eq!(content_rect(growing, target, Edge::Top, false).bottom(), growing.bottom());
        // Küçülme: yaprak hâlâ uzun, kalan kartlar bar tarafında yerinde.
        let shrinking = Rect::new(1500.0, 40.0, 380.0, 210.0);
        assert_eq!(content_rect(shrinking, target, Edge::Top, false).y, 40.0);
        // Alt bar: bar tarafı alt.
        let t = Rect::new(1500.0, 940.0, 380.0, 100.0);
        assert_eq!(content_rect(Rect::new(1500.0, 830.0, 380.0, 210.0), t, Edge::Bottom, false).bottom(), 1040.0);
        assert_eq!(content_rect(Rect::new(1500.0, 980.0, 380.0, 60.0), t, Edge::Bottom, false).y, 980.0);
    }

    /// Boş kare regresyonu (2026-10-17): açık yığına kart eklenince yaprak hedefinin yarısı
    /// kadarken içerik opaklığı sıfıra iniyordu (boş koyu kutu, görünen kartlar kayboluyordu).
    /// Büyümenin HER karesinde içerik tam opak kalır ve üst bar'da yığın kökeni bar tarafında
    /// yerinde durur. Açılış/yeniden açılış: `sheet::tests::sheet_never_shows_empty_once_grown`.
    #[test]
    fn growing_open_stack_never_draws_a_blank_frame() {
        use std::time::Duration;

        use nuxut::Theme;
        use nuxut::theme::{FrameworkConfig, MotionPreset};

        use crate::sheet::Sheet;

        let frame = |t0: Instant, i: u64| t0 + Duration::from_micros(16_667 * i);
        for preset in [MotionPreset::Calm, MotionPreset::Lively] {
            let mut cfg = FrameworkConfig::default();
            cfg.motion.preset = preset;
            let theme = Theme::from_config(&cfg).0;
            for edge in Edge::ALL {
                let mut sheet = Sheet::new(&theme, edge);
                let small = container_rect(AREA, edge, 120.0);
                let t0 = Instant::now();
                sheet.open(small, seed_for(edge, closed_rect(small, edge), 1.0), t0);
                let mut shown = false;
                for i in 0..120 {
                    let now = frame(t0, i);
                    let Some(shape) = sheet.shape(now) else { continue };
                    shown |= sheet.content_alpha(shape.visual_rect(), now) >= 0.999;
                }
                assert!(shown && sheet.is_revealed(), "{preset:?} {edge:?}: ilk açılış içeriği göstermedi");

                // Yeni kart: hedef iki katına çıkar.
                let t1 = frame(t0, 120);
                let big = container_rect(AREA, edge, 240.0);
                sheet.retarget(big, seed_for(edge, closed_rect(big, edge), 1.0), t1);
                for i in 0..90 {
                    let now = frame(t1, i);
                    let visual = sheet.shape(now).expect("açık yaprak alanda").visual_rect();
                    let alpha = sheet.content_alpha(visual, now);
                    let ctx = format!("{preset:?} {edge:?} kare {i}: görünen {visual:?}");
                    assert!(alpha >= 0.999, "{ctx}: içerik opaklığı {alpha}");
                    if edge == Edge::Top {
                        let c = content_rect(visual, sheet.target(), edge, sheet.is_revealed());
                        assert!((c.y - AREA.y).abs() < 0.5, "{ctx}: yığın kökeni kaydı {c:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn closed_rect_sits_on_bar_side() {
        let c = Rect::new(1500.0, 40.0, 380.0, 200.0);
        assert_eq!(closed_rect(c, Edge::Top), Rect::new(1500.0, 40.0, 380.0, 0.0));
        assert_eq!(closed_rect(c, Edge::Bottom), Rect::new(1500.0, 240.0, 380.0, 0.0));
        assert_eq!(closed_rect(c, Edge::Left), Rect::new(1500.0, 40.0, 0.0, 200.0));
        assert_eq!(closed_rect(c, Edge::Right), Rect::new(1880.0, 40.0, 0.0, 200.0));
    }
}
