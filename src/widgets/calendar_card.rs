//! "Takvim" kartı (kenar paneli): saat popout'unun takvimi ([`CalendarView`], kart kipi) —
//! ay ızgarası, ay değiştirme, gün seçimi, klavye ve seçili günün etkinlikleri. Veri aynı
//! `model.calendar`'dan; kart ay değiştirince ya da göründüğünde `App` görünür tüm takvimlerin
//! aralıklarını birleştirip sorgular ([`crate::calendar_view::query_range`]). Yüzey ve sütun
//! `quick::Card` çerçevesinden (`widgets::create_panel`).

use std::time::Instant;

use chrono::{DateTime, Local};
use nuxut::ui::Ui;
use nuxut::{CursorShape, Key, KeyEvent, Point, PointerEvent, Rect, TextSystem};

use super::{Action, PanelWidget, WidgetCx, WidgetKind};
use crate::calendar_view::{CalendarMode, CalendarView};
use crate::model::ShellModel;

#[derive(Debug)]
pub struct CalendarCard {
    view: CalendarView,
}

impl CalendarCard {
    pub fn new(cx: &WidgetCx) -> Self {
        Self {
            view: CalendarView::new(cx.theme, CalendarMode::Card),
        }
    }
}

fn range_action(range: Option<(i64, i64)>) -> Option<Action> {
    range.map(|(from, to)| Action::CalendarRange { from, to })
}

impl PanelWidget for CalendarCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Calendar
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        self.view
            .set_entries(cx.theme, &model.calendar, model.calendar_available)
    }

    fn set_time(&mut self, now: &DateTime<Local>, _cx: &WidgetCx) -> bool {
        self.view.set_time(now)
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.view.set_theme(cx.theme, cx.now);
    }

    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        Some(self.view.preferred_height(text, cx.theme, width))
    }

    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        crate::widgets::access_card(tree, self.kind(), rect, |tree| self.view.access(tree, rect));
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        self.view.draw(ui, rect, cx.theme);
    }

    fn pointer(
        &mut self,
        event: &PointerEvent,
        rect: Rect,
        text: &mut TextSystem,
        cx: &WidgetCx,
    ) -> (bool, Option<Action>) {
        let (redraw, range) = self.view.pointer(text, cx.theme, rect, event, cx.now);
        let action = range_action(range);
        (redraw || action.is_some(), action)
    }

    /// Popout'taki gibi: oklar gün/hafta, PageUp/PageDown ay, Home bugün, Enter/Space seçim.
    /// Escape panele kalır (kapatma).
    fn key(&mut self, event: &KeyEvent, cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        if event.key == Key::Escape {
            return None;
        }
        let (redraw, range) = self.view.key(cx.theme, event, cx.now);
        (redraw || range.is_some()).then(|| (true, range_action(range)))
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        self.view.cursor(p)
    }

    fn calendar_range(&self) -> Option<(i64, i64)> {
        Some(self.view.requested_range())
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        // Ay geçişi animasyonu ızgaranın kendi çiziminde kare ister (`Ui::request_frame`).
        false
    }
}

#[cfg(test)]
mod tests {
    use nuxut::{Button, KeyState, Modifiers, PointerKind};

    use super::*;
    use crate::widgets::WidgetSettings;

    fn key(k: Key) -> KeyEvent {
        KeyEvent::new(k, KeyState::Pressed, None, Modifiers::default())
    }

    #[test]
    fn keyboard_and_month_navigation_request_ranges() {
        let (theme, settings) = (nuxut::Theme::default(), WidgetSettings::default());
        let cx = WidgetCx {
            theme: &theme,
            settings: &settings,
            vertical: false,
            item: 26.0,
            now: Instant::now(),
            bounds: Rect::ZERO,
        };
        let mut card =
            crate::widgets::create_panel(WidgetKind::Calendar, &cx).expect("panel kartı");
        assert_eq!(card.kind(), WidgetKind::Calendar);
        let first = card.calendar_range().expect("kart aralık bildirir");
        // PageDown: sonraki ay → yeni aralık eylemi ve kartın bildirdiği aralık o ay.
        let (redraw, action) = card.key(&key(Key::PageDown), &cx).expect("tüketildi");
        assert!(redraw);
        let next = card.calendar_range().unwrap();
        assert!(next.0 > first.0);
        assert_eq!(
            action,
            Some(Action::CalendarRange {
                from: next.0,
                to: next.1
            })
        );
        // Escape panele kalır; bilinmeyen tuş tüketilmez.
        assert!(card.key(&key(Key::Escape), &cx).is_none());
        assert!(card.key(&key(Key::Tab), &cx).is_none());
        // İleri düğmesine tıklama da ay değiştirir (ızgara dikdörtgeni kartın içinde).
        let mut text = TextSystem::new();
        let rect = Rect::new(0.0, 0.0, 380.0, card.height(&mut text, 380.0, &cx).unwrap());
        // quick::Card sütunu + kart iç boşluğu: ileri düğmesi içerik sütununun sağ üstünde.
        let mut hit = None;
        for x in (0..380).rev().step_by(4) {
            let p = Point::new(x as f32, theme.space(3) + theme.space(12) * 0.5);
            let press = PointerEvent::new(p, PointerKind::Press(Button::Left));
            let release = PointerEvent::new(p, PointerKind::Release(Button::Left));
            card.pointer(
                &PointerEvent::new(p, PointerKind::Motion),
                rect,
                &mut text,
                &cx,
            );
            card.pointer(&press, rect, &mut text, &cx);
            if let (_, Some(a)) = card.pointer(&release, rect, &mut text, &cx) {
                hit = Some(a);
                break;
            }
        }
        let after = card.calendar_range().unwrap();
        assert_eq!(
            hit,
            Some(Action::CalendarRange {
                from: after.0,
                to: after.1
            })
        );
        assert!(after.0 > next.0);
    }
}
