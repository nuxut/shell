//! Klavye düzeni göstergesi (`keyboard_layout`): etkin düzenin kısa adı ("TR", "US") ve tıkla
//! sıradaki düzen. Yalnız 2+ düzen varken görünür (tek düzenli makinede uzunluk 0). Kaynak
//! Hyprland (`ShellModel::keyboard`, `services::keyboard_layouts`). Tasarım:
//! `docs/design/keyboard-layout.md`.

use std::time::Instant;

use nuxut::i18n::t;
use nuxut::ui::IconName;
use nuxut::{CursorShape, Painter, Point, Rect, TextSystem};

use super::pill::Pill;
use super::{Action, BarTooltip, BarWidget, WidgetCx, WidgetKind};
use crate::model::ShellModel;

#[derive(Debug)]
pub struct KeyboardLayout {
    pill: Pill,
    /// Gösterilen kısa ad; gizliyse `None`.
    label: Option<String>,
    tooltip: String,
}

impl KeyboardLayout {
    pub fn new(cx: &WidgetCx) -> Self {
        Self { pill: Pill::new(cx, IconName::Keyboard).with_text_in_vertical(), label: None, tooltip: String::new() }
    }

    fn visible(&self) -> bool {
        self.label.is_some()
    }
}

impl BarWidget for KeyboardLayout {
    fn kind(&self) -> WidgetKind {
        WidgetKind::KeyboardLayout
    }

    fn sync(&mut self, model: &ShellModel, _monitor: &str, _cx: &WidgetCx) -> bool {
        let k = model.keyboard.as_ref().filter(|k| k.switchable());
        let label = k.and_then(|k| k.active_label());
        let tooltip = k.map(|k| t!("shell-bar-keyboard-layout-tooltip", name = k.active_name.clone())).unwrap_or_default();
        if label == self.label && tooltip == self.tooltip {
            return false;
        }
        if let Some(l) = &label {
            self.pill.set_text(l);
        }
        self.label = label;
        self.tooltip = tooltip;
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
            self.pill.draw(painter, rect, cx, None, cx.theme.colors.on_surface);
        }
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        self.visible() && self.pill.hover(rect, pointer, down, cx)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        (self.visible() && Pill::chip(rect, cx).contains(at)).then_some(Action::KeyboardLayoutNext)
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
    use crate::services::keyboard_layouts::KeyboardLayouts;
    use crate::widgets::WidgetSettings;
    use nuxut::Theme;

    #[test]
    fn shows_active_layout_only_with_two_or_more() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let settings = WidgetSettings::default();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now: Instant::now(), bounds: Rect::ZERO };
        let mut w = KeyboardLayout::new(&cx);
        let mut text = TextSystem::new();
        let mut m = ShellModel::default();
        assert!(!w.sync(&m, "", &cx));
        assert_eq!(w.measure(&mut text, &cx), 0.0, "bilinmiyor: gizli");
        m.keyboard = Some(KeyboardLayouts { layouts: vec![("tr".into(), String::new())], active: 0, active_name: "Turkish".into() });
        assert!(!w.sync(&m, "", &cx));
        assert_eq!(w.measure(&mut text, &cx), 0.0, "tek düzen: gizli");
        m.keyboard = Some(KeyboardLayouts {
            layouts: vec![("tr".into(), String::new()), ("us".into(), String::new())],
            active: 1,
            active_name: "English (US)".into(),
        });
        assert!(w.sync(&m, "", &cx));
        assert_eq!(w.label.as_deref(), Some("US"));
        assert_eq!(w.tooltip, "Klavye düzeni: English (US) · tıklayınca sıradaki");
        let len = w.measure(&mut text, &cx);
        assert!(len > 26.0);
        let rect = Rect::new(0.0, 0.0, len, 34.0);
        assert_eq!(w.click(rect, rect.center(), &cx), Some(Action::KeyboardLayoutNext));
        // Dikey bar: yalnız kısa ad, öğe boyunda.
        let vcx = WidgetCx { vertical: true, ..cx };
        assert_eq!(w.measure(&mut text, &vcx), 26.0);
    }
}
