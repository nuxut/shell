//! Büyüteç bar hapı: büyütme açıkken (çarpan > 1) `primary` dolgulu "🔍 2,0×" hapı, kapalıyken
//! gizli (uzunluk 0). Kısayolla yakınlaştıran kişiye "büyütme açık" hatırlatıcısı ve tek tıkla
//! kapatma (`Command::Off`: son çarpan hatırlanır, `toggle` geri getirir). Tasarım:
//! `docs/design/magnifier.md` "Bar hapı ve OSD".

use std::time::Instant;

use nuxut::i18n::t;
use nuxut::ui::IconName;
use nuxut::{CursorShape, Painter, Point, Rect, TextSystem};

use super::pill::Pill;
use super::quick::magnifier::factor_text;
use super::{Action, BarTooltip, BarWidget, WidgetCx, WidgetKind};
use crate::model::ShellModel;
use crate::services::magnifier::Command;

#[derive(Debug)]
pub struct MagnifierPill {
    pill: Pill,
    /// Gösterilen çarpan; büyütme kapalıysa `None` (gizli).
    factor: Option<f64>,
    tooltip: String,
}

impl MagnifierPill {
    pub fn new(cx: &WidgetCx) -> Self {
        Self { pill: Pill::new(cx, IconName::Search), factor: None, tooltip: String::new() }
    }

    fn visible(&self) -> bool {
        self.factor.is_some()
    }
}

impl BarWidget for MagnifierPill {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Magnifier
    }

    fn sync(&mut self, model: &ShellModel, _monitor: &str, _cx: &WidgetCx) -> bool {
        let v = &model.magnifier;
        let factor = (v.available && v.active()).then_some(v.factor);
        if factor == self.factor {
            return false;
        }
        self.factor = factor;
        if let Some(f) = factor {
            let text = factor_text(f);
            self.tooltip = t!("shell-bar-magnifier-tooltip", factor = text.clone());
            self.pill.set_text(&text);
        }
        true
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.pill.set_theme(cx);
    }

    fn measure(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        if self.visible() { self.pill.measure(text, cx) } else { 0.0 }
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, _highlighted: bool) {
        if self.visible() {
            let c = &cx.theme.colors;
            self.pill.draw(painter, rect, cx, Some(c.primary), c.on_primary);
        }
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        self.visible() && self.pill.hover(rect, pointer, down, cx)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        (self.visible() && Pill::chip(rect, cx).contains(at)).then_some(Action::Magnifier(Command::Off))
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.pill.is_animating(now)
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        (self.visible() && Pill::chip(rect, cx).contains(p)).then_some(CursorShape::Pointer)
    }

    fn tooltip(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        (self.visible() && Pill::chip(rect, cx).contains(p)).then_some(BarTooltip { text: &self.tooltip, anchor: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::magnifier::MagnifierView;
    use crate::widgets::WidgetSettings;
    use nuxut::Theme;

    #[test]
    fn visible_only_while_zoomed_and_click_turns_off() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let settings = WidgetSettings::default();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now: Instant::now(), bounds: Rect::ZERO };
        let mut w = MagnifierPill::new(&cx);
        let mut text = TextSystem::new();
        let mut m = ShellModel { magnifier: MagnifierView { available: true, factor: 1.0, max: 4.0 }, ..Default::default() };
        assert!(!w.sync(&m, "", &cx));
        assert_eq!(w.measure(&mut text, &cx), 0.0, "büyütme yokken gizli");
        m.magnifier.factor = 2.0;
        assert!(w.sync(&m, "", &cx));
        assert!(w.measure(&mut text, &cx) > 26.0, "ikon + çarpan");
        assert_eq!(w.tooltip, "Büyüteç açık: 2,0× · tıklayınca kapanır");
        let rect = Rect::new(0.0, 0.0, w.measure(&mut text, &cx), 34.0);
        assert_eq!(w.click(rect, rect.center(), &cx), Some(Action::Magnifier(Command::Off)));
        assert!(!w.sync(&m, "", &cx), "aynı çarpan çizim istemez");
        // Hyprland'e ulaşılamıyorsa (available=false) hap gösterilmez.
        m.magnifier.available = false;
        assert!(w.sync(&m, "", &cx));
        assert_eq!(w.measure(&mut text, &cx), 0.0);
        assert_eq!(w.click(rect, rect.center(), &cx), None);
    }
}
