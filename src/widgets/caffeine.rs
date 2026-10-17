//! Uyanık tut: elle boşta kalma engeli (`ShellModel::idle_inhibit`) için bar düğmesi. Açıkken
//! ekran kararmaz, kilitlenmez, uykuya geçmez (boşta kalma makinesinin `Inhibitor::Manual`'ı;
//! hızlı anahtarlar kartı ve `msg idle inhibit` ile aynı durum). Tık: aç/kapa
//! ([`Action::IdleInhibit`]).
//!
//! Açıkken `primary` dolgulu hap (bir bakışta "unutma: açık"); kapalıyken soluk ikon, ya da
//! `show_when_off = false` ise gizli (yalnız açıkken hatırlatıcı olarak görünür).

use std::time::Instant;

use nuxut::i18n::t;
use nuxut::render::RectStyle;
use nuxut::ui::{Icon, IconName, IconView, Interaction, TextRole};
use nuxut::{Color, Corners, CursorShape, Painter, Point, Rect, TextSystem};

use super::{Action, BarTooltip, BarWidget, WidgetCx, WidgetKind, chip_fill, draw_hover};
use crate::model::ShellModel;

#[derive(Debug)]
pub struct Caffeine {
    on: bool,
    show_when_off: bool,
    icon: IconView,
    tooltip: String,
    hover: Interaction,
}

impl Caffeine {
    pub fn new(cx: &WidgetCx) -> Self {
        Self {
            on: false,
            show_when_off: cx.settings.caffeine.show_when_off,
            icon: IconView::with_role(cx.theme, Icon::named(IconName::Coffee), TextRole::IconSmall),
            tooltip: t!("shell-caffeine-off"),
            hover: Interaction::new(cx.theme),
        }
    }

    fn visible(&self) -> bool {
        self.on || self.show_when_off
    }
}

/// Kare hap: bar'a dik eksende öğe boyunda, ortada.
fn chip_rect(rect: Rect, cx: &WidgetCx) -> Rect {
    let c = rect.center();
    Rect::new(c.x - cx.item * 0.5, c.y - cx.item * 0.5, cx.item, cx.item)
}

impl BarWidget for Caffeine {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Caffeine
    }

    fn sync(&mut self, model: &ShellModel, _monitor: &str, _cx: &WidgetCx) -> bool {
        if self.on == model.idle_inhibit {
            return false;
        }
        self.on = model.idle_inhibit;
        self.tooltip = if self.on { t!("shell-caffeine-on") } else { t!("shell-caffeine-off") };
        true
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.hover.set_theme(cx.theme, cx.now);
        self.icon.set_theme(cx.theme, cx.now);
        self.show_when_off = cx.settings.caffeine.show_when_off;
    }

    fn measure(&mut self, _text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        if self.visible() { cx.item } else { 0.0 }
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, _highlighted: bool) {
        if !self.visible() {
            return;
        }
        let colors = &cx.theme.colors;
        let chip = chip_rect(rect, cx);
        let radii = Corners::uniform(chip.height * 0.5);
        let (fill, fg) = if self.on { (colors.primary, colors.on_primary) } else { (Color::TRANSPARENT, colors.on_surface_variant) };
        if self.on {
            painter.rect(chip, &RectStyle::filled(fill, radii));
        }
        let container = if self.on { fill } else { chip_fill(cx.theme, false) };
        draw_hover(painter, &self.hover, chip, radii, cx, container, &[fg]);
        self.icon.draw_centered(painter, chip.center(), fg);
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        self.visible() && self.hover.pointer(chip_rect(rect, cx), pointer, down, cx.now)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        (self.visible() && chip_rect(rect, cx).contains(at)).then_some(Action::IdleInhibit(!self.on))
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.hover.is_animating(now)
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        (self.visible() && chip_rect(rect, cx).contains(p)).then_some(CursorShape::Pointer)
    }

    fn tooltip(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        (self.visible() && chip_rect(rect, cx).contains(p)).then_some(BarTooltip { text: &self.tooltip, anchor: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::WidgetSettings;
    use nuxut::Theme;

    #[test]
    fn toggles_and_follows_model() {
        let theme = Theme::default();
        let mut settings = WidgetSettings::default();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now: Instant::now(), bounds: Rect::ZERO };
        let mut w = Caffeine::new(&cx);
        let mut text = TextSystem::new();
        let rect = Rect::new(0.0, 0.0, 26.0, 34.0);
        assert_eq!(w.measure(&mut text, &cx), 26.0, "varsayılan: kapalıyken de görünür");
        assert_eq!(w.click(rect, rect.center(), &cx), Some(Action::IdleInhibit(true)));
        let mut m = ShellModel::default();
        assert!(!w.sync(&m, "", &cx));
        m.idle_inhibit = true;
        assert!(w.sync(&m, "", &cx));
        assert!(!w.sync(&m, "", &cx), "değişmeyen durum çizim istemez");
        assert_eq!(w.click(rect, rect.center(), &cx), Some(Action::IdleInhibit(false)));

        settings.caffeine.show_when_off = false;
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now: Instant::now(), bounds: Rect::ZERO };
        w.set_theme(&cx);
        assert_eq!(w.measure(&mut text, &cx), 26.0, "açıkken görünür");
        m.idle_inhibit = false;
        w.sync(&m, "", &cx);
        assert_eq!(w.measure(&mut text, &cx), 0.0, "kapalıyken gizli");
        assert_eq!(w.click(rect, rect.center(), &cx), None);
    }
}
