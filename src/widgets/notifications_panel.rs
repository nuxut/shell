//! Bildirim merkezi (kenar paneli widget'ı): sağ çekmecede oturum kartının altında, kalan
//! alanın tamamı (`height() -> None`).
//!
//! - Başlık: "Bildirimler" · "Tümünü temizle"; altında "Rahatsız etme" anahtarı.
//! - Geçmiş uygulamaya göre gruplu (`history_groups`): grup başlığı + kartlar, kaydırılabilir
//!   (`ScrollView`); kartlar ve başlıklar `NotificationStack` ile yayla yer değiştirir.
//! - Kart tıklaması (varsayılan eylem varsa) `Invoke "default"`, eylem düğmeleri `Invoke`,
//!   X ya da kaydırma `Dismiss`, gövde bağlantısı `OpenLink(BodyLink::url)`.
//! - Boşken sakin bir boş hâl (framework `EmptyState`): maskot Fiyu (`Mood::Empty`, 96) +
//!   "Bildirim yok". Maskot gizliyse (`[appearance] mascot = false`) eski sessiz görünüm:
//!   `NotificationsOff` ikonu + metin (`EmptyState::with_fallback_icon`; 2026-10-14).
//!   Maskot ya da ikon alana sığmıyorsa yer kaplamaz, metin kendi başına ortalı (yerleşim:
//!   `docs/design/mascot-placement.md`).
//! - Panel açılınca (`panel_opened`) okunmamışlar `MarkAllRead`; panel açıkken gelenler panel
//!   üzerindeki ilk işaretçi hareketiyle okunur.
//!
//! Kartlar kimlikle korunur; her `sync`'te yeniden yaratılmaz (bkz. `notification_cards`).

use std::time::{Instant, SystemTime};

use chrono::{DateTime, Local};
use nuxut::ui::{
    Button, ButtonVariant, EmptyState, Icon, IconName, Label, Mood, NotificationStack, ScrollView, TextRole, Toggle, Ui,
    text_style,
};
use nuxut::i18n::t;
use nuxut::{CursorShape, Point, PointerEvent, PointerKind, Rect, Size, TextSystem};

use super::notification_cards::CardSet;
use super::{Action, NotifyCommand, PanelWidget, WidgetCx, WidgetKind};
use crate::model::ShellModel;

/// Yığındaki öğe: grup başlığı ya da kart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelItem {
    Header(String),
    Card(u32),
}

/// Geçmiş grubu (görünüm için sade kopya).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub key: String,
    pub title: String,
    pub ids: Vec<u32>,
}

/// Grup başlığı metni: uygulama adı (boşsa "Diğer") ve birden fazlaysa sayı.
pub fn group_title(app_name: &str, count: usize) -> String {
    let name = if app_name.trim().is_empty() { t!("shell-notifications-group-other") } else { app_name.to_owned() };
    t!("shell-notifications-group-title", name = name, count = count)
}

/// Yığın öğeleri: her grup için başlık ve (ölçülmüş) kartlar. Ölçülmemiş kart atlanır.
pub fn stack_items(groups: &[Group], header_h: f32, card_h: impl Fn(u32) -> Option<f32>) -> Vec<(PanelItem, f32)> {
    let mut items = Vec::new();
    for g in groups {
        let cards: Vec<(PanelItem, f32)> =
            g.ids.iter().filter_map(|&id| card_h(id).map(|h| (PanelItem::Card(id), h))).collect();
        if cards.is_empty() {
            continue;
        }
        items.push((PanelItem::Header(g.key.clone()), header_h));
        items.extend(cards);
    }
    items
}

/// Başlık bölgesinin yerleşimi.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeaderLayout {
    pub title: Point,
    pub clear: Rect,
    pub dnd_label: Point,
    pub dnd: Rect,
    /// Kaydırma görünümü (başlığın altı).
    pub viewport: Rect,
}

/// `rect` içinde: üst satır başlık + temizle düğmesi, ikinci satır DND etiketi + anahtar.
pub fn header_layout(
    rect: Rect,
    pad: f32,
    gap: f32,
    title: Size,
    clear: Size,
    dnd_label: Size,
    dnd: Size,
) -> HeaderLayout {
    let row1 = title.height.max(clear.height);
    let y1 = rect.y + pad;
    let row2 = dnd_label.height.max(dnd.height);
    let y2 = y1 + row1 + gap;
    let top = y2 + row2 + gap;
    HeaderLayout {
        title: Point::new(rect.x + pad, y1 + (row1 - title.height) * 0.5),
        clear: Rect::new(rect.right() - pad - clear.width, y1 + (row1 - clear.height) * 0.5, clear.width, clear.height),
        dnd_label: Point::new(rect.x + pad, y2 + (row2 - dnd_label.height) * 0.5),
        dnd: Rect::new(rect.right() - pad - dnd.width, y2 + (row2 - dnd.height) * 0.5, dnd.width, dnd.height),
        viewport: Rect::new(rect.x, top, rect.width, (rect.bottom() - top).max(0.0)),
    }
}

#[derive(Debug)]
pub struct NotificationsPanel {
    title: Label,
    clear: Button,
    dnd_label: Label,
    dnd: Toggle,
    /// Boş hâl: maskot `Mood::Empty` + "Bildirim yok".
    empty: EmptyState,
    headers: Vec<(String, Label)>,
    groups: Vec<Group>,
    cards: CardSet,
    stack: NotificationStack<PanelItem>,
    scroll: ScrollView,
    unread: u32,
    /// Son çizimdeki yerleşim (işaretçi için).
    layout: Option<HeaderLayout>,
}

impl NotificationsPanel {
    pub fn new(cx: &WidgetCx) -> Self {
        let t = cx.theme;
        Self {
            title: Label::new(t!("shell-notifications-title"), text_style(t, TextRole::Title)),
            clear: Button::new(t, ButtonVariant::Text).with_label(t, t!("shell-notifications-clear-all")),
            dnd_label: Label::new(t!("shell-notifications-dnd-label"), text_style(t, TextRole::Body)),
            dnd: Toggle::new(t, false),
            empty: EmptyState::new(t, t!("shell-notifications-empty")).with_mascot(t, Mood::Empty)
                .with_fallback_icon(t, Icon::named(IconName::NotificationsOff)),
            headers: Vec::new(),
            groups: Vec::new(),
            cards: CardSet::new(t, false),
            stack: NotificationStack::new(t),
            scroll: ScrollView::new(t),
            unread: 0,
            layout: None,
        }
    }

    /// Başlık metni (testler: dil değişiminde yeniden kurulan başlık).
    pub fn title(&self) -> &str {
        self.title.text()
    }

    fn pad(cx: &WidgetCx) -> f32 {
        cx.theme.space(4)
    }

    fn header_style(cx: &WidgetCx) -> nuxut::TextStyle {
        // Grup başlığı sayaç içerir ("uygulama · 3"): rakamlar eş genişlikte.
        text_style(cx.theme, TextRole::Caption).with_max_lines(1).with_tabular_numbers()
    }

    fn compute_layout(&mut self, text: &mut TextSystem, rect: Rect, cx: &WidgetCx) -> HeaderLayout {
        let t = cx.theme;
        let title = self.title.measure(text);
        let clear = self.clear.preferred_size(text, t);
        let dnd_label = self.dnd_label.measure(text);
        let dnd = self.dnd.preferred_size(t);
        let l = header_layout(rect, Self::pad(cx), t.space(2), title, clear, dnd_label, dnd);
        self.clear.set_rect(l.clear);
        self.dnd.set_rect(l.dnd);
        l
    }
}

impl PanelWidget for NotificationsPanel {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Notifications
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let center = &model.notifications;
        let mut changed = self.dnd.set_on(center.dnd(), cx.now);
        let groups: Vec<Group> = center
            .history_groups()
            .iter()
            .map(|g| Group {
                key: g.key.clone(),
                title: group_title(g.app_name, g.notifications.len()),
                ids: g.notifications.iter().map(|n| n.id).collect(),
            })
            .collect();
        if groups != self.groups {
            // Başlık etiketleri anahtarla korunur.
            let style = Self::header_style(cx);
            let mut headers = Vec::with_capacity(groups.len());
            for g in &groups {
                let mut label = match self.headers.iter().position(|(k, _)| *k == g.key) {
                    Some(i) => self.headers.swap_remove(i).1,
                    None => Label::new("", style.clone()),
                };
                label.set_text(&g.title);
                headers.push((g.key.clone(), label));
            }
            self.headers = headers;
            self.groups = groups;
            changed = true;
        }
        changed |= self.cards.sync(center.history(), cx.now);
        changed |= self.clear.set_enabled(!self.groups.is_empty(), cx.now);
        self.unread = model.notifications_unread;
        changed
    }

    fn set_time(&mut self, _now: &DateTime<Local>, _cx: &WidgetCx) -> bool {
        self.cards.refresh_time(SystemTime::now())
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        let t = cx.theme;
        self.title.set_style(text_style(t, TextRole::Title));
        self.clear.set_theme(t, cx.now);
        self.dnd_label.set_style(text_style(t, TextRole::Body));
        self.dnd.set_theme(t, cx.now);
        self.empty.set_theme(t, cx.now);
        let style = Self::header_style(cx);
        for (_, l) in &mut self.headers {
            l.set_style(style.clone());
        }
        self.cards.set_theme(t, cx.now);
        self.stack.set_theme(t, cx.now);
        self.scroll.set_theme(t, cx.now);
    }

    fn height(&mut self, _text: &mut TextSystem, _width: f32, _cx: &WidgetCx) -> Option<f32> {
        None
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        let now = ui.now;
        let t = cx.theme;
        let c = &t.colors;
        let pad = Self::pad(cx);
        let l = self.compute_layout(ui.painter.text_system(), rect, cx);
        self.layout = Some(l);

        self.title.draw(ui.painter, l.title, ui.fade(c.on_surface));
        self.clear.draw(ui);
        self.dnd_label.draw(ui.painter, l.dnd_label, ui.fade(c.on_surface_variant));
        self.dnd.draw(ui);

        let width = (rect.width - 2.0 * pad).max(0.0);
        self.cards.prepare(ui.painter, t, width);
        let header_h = t.space(8);
        let items = stack_items(&self.groups, header_h, |id| {
            self.cards.entries.iter().find(|e| e.id() == id).map(|e| e.height()).filter(|h| *h > 0.0)
        });
        let content_h = self.stack.layout(&items, t.space(2), now) + pad;
        self.scroll.set_geometry(l.viewport, content_h, now);

        if self.groups.is_empty() {
            // Görünüm alanının ortasında maskot + metin (framework `EmptyState`).
            self.empty.set_rect(l.viewport);
            self.empty.draw(ui);
            return;
        }

        let stack = &self.stack;
        let headers = &mut self.headers;
        let cards = &mut self.cards;
        self.scroll.draw(ui, |ui, origin, span| {
            let place = |item: &PanelItem, h: f32| -> Option<(Rect, f32)> {
                let slot = stack.slot(item, now)?;
                // Görünür aralık dışındakiler çizilmez.
                if slot.y + h < span.start || slot.y > span.end {
                    return None;
                }
                Some((Rect::new(origin.x + pad, origin.y + slot.y, width, h), slot.alpha))
            };
            for (key, label) in headers.iter_mut() {
                if let Some((r, alpha)) = place(&PanelItem::Header(key.clone()), header_h) {
                    let s = label.measure_in(ui.painter);
                    let color = ui.fade(c.on_surface_variant.multiply_alpha(alpha));
                    label.draw(ui.painter, Point::new(r.x, r.bottom() - s.height - t.space(1)), color);
                }
            }
            cards.draw(ui, |e| place(&PanelItem::Card(e.id()), e.height()));
        });
    }

    fn pointer(
        &mut self,
        event: &PointerEvent,
        rect: Rect,
        text: &mut TextSystem,
        cx: &WidgetCx,
    ) -> (bool, Option<Action>) {
        let now = cx.now;
        let l = self.compute_layout(text, rect, cx);
        let mut repaint = false;

        let r = self.dnd.pointer(event, now);
        repaint |= r.repaint;
        if let Some(on) = r.event {
            return (true, Some(Action::Notify(NotifyCommand::SetDnd(on))));
        }
        let r = self.clear.pointer(event, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            return (true, Some(Action::Notify(NotifyCommand::ClearAll)));
        }

        repaint |= self.scroll.pointer(cx.theme, event, now).repaint;
        // Kartlar yalnızca görünüm içindeki olayları alır (görünüm dışına kaymış kartın eski
        // dikdörtgeni tık yakalamasın); bırakma/ayrılma her zaman (durum temizliği).
        // Dokunmada kaydırma görünümü basmayı bekletir; kartlar `forward`'ın verdiklerini alır.
        for ev in self.scroll.forward(event) {
            let in_view = l.viewport.contains(ev.position);
            if in_view || matches!(ev.kind, PointerKind::Release(_) | PointerKind::Leave) {
                let (r, command) = self.cards.pointer(cx.theme, &ev, now, false);
                repaint |= r;
                if let Some(cmd) = command {
                    return (true, Some(Action::Notify(cmd)));
                }
            }
        }

        // Panel açıkken gelen okunmamışlar panelde görüldü.
        if self.unread > 0 && event.kind == PointerKind::Motion && rect.contains(event.position) {
            self.unread = 0;
            return (repaint, Some(Action::Notify(NotifyCommand::MarkAllRead)));
        }
        (repaint, None)
    }

    /// Başlık, "Tümünü temizle", Rahatsız etme anahtarı ve görünümdeki kartlar (ya da boş durum).
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        let Some(l) = self.layout else { return };
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            self.clear.access(tree, "clear");
            self.dnd.access(tree, "dnd").label(self.dnd_label.text());
            if self.groups.is_empty() {
                self.empty.access(tree, "empty");
                return;
            }
            let list = tree.push("cards", nuxut::access::Role::List).label(self.title.text()).bounds(l.viewport).id();
            tree.within(list, |tree| self.cards.access(tree, l.viewport, false));
        });
    }

    fn access_action(
        &mut self,
        request: &nuxut::access::AccessRequest,
        parent: nuxut::access::NodeId,
        _rect: Rect,
        cx: &WidgetCx,
    ) -> Option<(bool, Option<Action>)> {
        let list = nuxut::access::child_id(crate::widgets::access_card_id(parent, self.kind()), "cards");
        let cmd = self.cards.access_action(list, request, cx.now, false)?;
        Some((true, Some(Action::Notify(cmd))))
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        let in_view = self.layout.is_some_and(|l| l.viewport.contains(p));
        Some(CursorShape::pick([
            self.dnd.cursor(p),
            self.clear.cursor(p),
            self.scroll.cursor(p),
            in_view.then(|| self.cards.cursor(p)).flatten(),
        ]))
    }

    fn panel_opened(&mut self, _now: Instant) -> Option<Action> {
        // Merkez görüldü: okunmamışlar okundu (rozet söner).
        (std::mem::take(&mut self.unread) > 0).then_some(Action::Notify(NotifyCommand::MarkAllRead))
    }

    fn panel_closed(&mut self, _now: Instant) {
        // Açıkken okunanlar kapanınca okunmuş kalır; yeni gelenler için sayaç sync'ten gelir.
    }

    fn corner_clearance(&self, cx: &WidgetCx) -> Size {
        // Başlık ve liste sütunu `pad` içeride; kaydırılan liste alt kenara kadar akar, kart
        // köşeleri yuvarlak olduğundan alttaki ince bant düğmenin altından akabilir.
        let p = Self::pad(cx);
        Size::new(p, p)
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.stack.is_animating(now) | self.cards.is_animating(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_titles() {
        crate::i18n::use_turkish_for_tests();
        assert_eq!(group_title("Sohbet", 1), "Sohbet");
        assert_eq!(group_title("Sohbet", 3), "Sohbet · 3");
        assert_eq!(group_title("  ", 2), "Diğer · 2");
    }

    #[test]
    fn items_interleave_headers_and_measured_cards() {
        let groups = vec![
            Group { key: "a".into(), title: "A".into(), ids: vec![1, 2] },
            Group { key: "b".into(), title: "B".into(), ids: vec![3] },
            Group { key: "c".into(), title: "C".into(), ids: vec![4] },
        ];
        // 4 henüz ölçülmedi: grubu (başlığıyla) atlanır.
        let items = stack_items(&groups, 30.0, |id| (id != 4).then_some(100.0 + id as f32));
        assert_eq!(
            items,
            vec![
                (PanelItem::Header("a".into()), 30.0),
                (PanelItem::Card(1), 101.0),
                (PanelItem::Card(2), 102.0),
                (PanelItem::Header("b".into()), 30.0),
                (PanelItem::Card(3), 103.0),
            ]
        );
    }

    #[test]
    fn header_layout_rows() {
        let rect = Rect::new(1500.0, 300.0, 380.0, 700.0);
        let l = header_layout(
            rect,
            16.0,
            8.0,
            Size::new(120.0, 24.0),
            Size::new(110.0, 40.0),
            Size::new(100.0, 20.0),
            Size::new(52.0, 32.0),
        );
        assert_eq!(l.title, Point::new(1516.0, 316.0 + 8.0));
        assert_eq!(l.clear, Rect::new(1880.0 - 16.0 - 110.0, 316.0, 110.0, 40.0));
        assert_eq!(l.dnd.right(), 1880.0 - 16.0);
        assert_eq!(l.dnd.y, 316.0 + 40.0 + 8.0);
        assert_eq!(l.viewport.y, 316.0 + 40.0 + 8.0 + 32.0 + 8.0);
        assert_eq!(l.viewport.bottom(), rect.bottom());
        assert_eq!(l.viewport.width, rect.width);
    }

    fn render(panel: &mut NotificationsPanel, cx: &WidgetCx, rect: Rect) -> Vec<nuxut::render::RecordedDraw> {
        let mut text = TextSystem::new();
        let mut rec = nuxut::render::Recorder::new();
        rec.record(&mut text, (800, 1000), 1.0, |p| {
            let mut ui = Ui::new(p, cx.theme, cx.now);
            panel.draw(&mut ui, rect, cx);
        });
        rec.draws()
    }

    fn images(draws: &[nuxut::render::RecordedDraw]) -> Vec<Rect> {
        draws.iter().filter(|d| d.kind == nuxut::render::DrawKind::Image).map(|d| d.rect).collect()
    }

    /// Boş merkez: maskot 96 px, görünüm alanında yatay ortalı. Gizlenince (`[appearance]
    /// mascot = false`, canlı `set_theme`) maskot görseli çizilmez; yerine eski sessiz ikon
    /// (`NotificationsOff`, glif) + metin, sütun görünüm alanının ortasında
    /// (2026-10-14, "gizliyken eski görünüm"); geri açılınca maskot aynı yerde.
    #[test]
    fn empty_center_shows_mascot_and_hides_cleanly() {
        let shown = nuxut::Theme::default();
        let mut hidden = shown.clone();
        hidden.appearance.mascot = false;
        let settings = crate::widgets::WidgetSettings::default();
        let cx = |theme| WidgetCx { theme, settings: &settings, vertical: false, item: 26.0, now: Instant::now(), bounds: Rect::ZERO };
        let rect = Rect::new(100.0, 50.0, 380.0, 700.0);

        let mut panel = NotificationsPanel::new(&cx(&shown));
        let imgs = images(&render(&mut panel, &cx(&shown), rect));
        assert_eq!(imgs.len(), 1, "boş merkezde tek görsel: maskot");
        let viewport = panel.layout.unwrap().viewport;
        assert_eq!(imgs[0].size(), Size::new(96.0, 96.0));
        assert!((imgs[0].center().x - viewport.center().x).abs() <= 1.0);
        assert!(viewport.contains(imgs[0].origin()));
        assert_eq!(panel.empty.rect(), viewport);

        panel.set_theme(&cx(&hidden));
        let draws = render(&mut panel, &cx(&hidden), rect);
        assert!(images(&draws).is_empty(), "gizli maskot çizilmez");
        assert!(draws.iter().any(|d| d.kind == nuxut::render::DrawKind::Glyphs), "metin kalır");
        let l = panel.empty.last_layout().unwrap();
        assert_eq!(panel.empty.fallback_icon(), Some(&Icon::named(IconName::NotificationsOff)));
        let art = l.art.expect("gizliyken yedek ikon yeri");
        assert!(art.height < 96.0, "ikon, maskot boyutunda değil: {art:?}");
        assert!((art.center().x - viewport.center().x).abs() <= 1.0, "ikon ortalı");
        assert!((l.column.center().y - viewport.center().y).abs() <= 1.0, "sütun ortada");

        panel.set_theme(&cx(&shown));
        assert_eq!(images(&render(&mut panel, &cx(&shown), rect)), imgs);
    }

    /// Kısa panelde maskot küçülür (≥ 48); 48'e de yer yoksa çizilmez, yalnız metin kalır.
    #[test]
    fn empty_center_mascot_fits_short_panels() {
        let theme = nuxut::Theme::default();
        let settings = crate::widgets::WidgetSettings::default();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now: Instant::now(), bounds: Rect::ZERO };
        let mut panel = NotificationsPanel::new(&cx);
        render(&mut panel, &cx, Rect::new(0.0, 0.0, 380.0, 700.0));
        let header = panel.layout.unwrap().viewport.y;
        // Görünüm 150 px: maskot küçülür, alana sığar.
        let imgs = images(&render(&mut panel, &cx, Rect::new(0.0, 0.0, 380.0, header + 150.0)));
        assert_eq!(imgs.len(), 1);
        assert!(imgs[0].height >= 48.0 && imgs[0].height < 96.0, "{:?}", imgs[0]);
        assert!(imgs[0].y >= header && imgs[0].bottom() <= header + 150.0);
        // Görünüm 60 px: maskot yok.
        assert!(images(&render(&mut panel, &cx, Rect::new(0.0, 0.0, 380.0, header + 60.0))).is_empty());
        assert_eq!(panel.empty.last_layout().unwrap().art, None);
    }
}
