//! Kare ikon düğmeleri: `session` (güç), `notifications` (zil + okunmamış durumu),
//! `panel-toggle`, `launcher`. Hepsi bir yaprağı (kenar paneli ya da başlatıcı) açar/kapatır;
//! yaprak açıkken vurgulanır.
//!
//! İkon semantik addır (`IconName`); okunmamış bildirim varken zil dolar (`IconView` dolgu
//! geçişi) ve birincil renge döner.

use std::time::Instant;

use nuxut::ui::{Icon, IconName, IconView, Interaction};
use nuxut::{Corners, CursorShape, Painter, Point, Rect, RectStyle, TextSystem};

use super::{Action, BarWidget, PanelId, WidgetCx, WidgetKind, chip_fill, draw_hover};
use crate::model::ShellModel;

#[derive(Debug)]
pub struct IconButton {
    kind: WidgetKind,
    icon: IconView,
    hover: Interaction,
    unread: u32,
}

impl IconButton {
    pub fn new(kind: WidgetKind, cx: &WidgetCx) -> Self {
        let mut s = Self { kind, icon: IconView::new(cx.theme, Icon::named(IconName::Menu)), hover: Interaction::new(cx.theme), unread: 0 };
        s.icon.set_icon(Icon::named(s.icon_name(cx)));
        s
    }

    fn icon_name(&self, cx: &WidgetCx) -> IconName {
        match self.kind {
            WidgetKind::Session => IconName::Power,
            WidgetKind::Notifications => IconName::Notifications,
            WidgetKind::Launcher => IconName::Apps,
            // Ayar doğrulamada çözülebilir ada indirilir (`WidgetSettings::validate`).
            _ => cx.settings.panel_toggle.icon.parse().unwrap_or(IconName::Menu),
        }
    }

    fn target(&self, cx: &WidgetCx) -> PanelId {
        match self.kind {
            WidgetKind::Session => cx.settings.session.panel.into(),
            WidgetKind::Notifications => cx.settings.notifications.panel.into(),
            WidgetKind::Launcher => PanelId::Launcher,
            _ => cx.settings.panel_toggle.panel.into(),
        }
    }

    fn button_rect(rect: Rect, cx: &WidgetCx) -> Rect {
        let s = cx.item;
        Rect::new(rect.center().x - s * 0.5, rect.center().y - s * 0.5, s, s)
    }
}

impl BarWidget for IconButton {
    fn kind(&self) -> WidgetKind {
        self.kind
    }

    fn sync(&mut self, model: &ShellModel, _monitor: &str, cx: &WidgetCx) -> bool {
        if self.kind != WidgetKind::Notifications || self.unread == model.notifications_unread {
            return false;
        }
        self.unread = model.notifications_unread;
        self.icon.set_filled(self.unread > 0, cx.now);
        true
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.hover.set_theme(cx.theme, cx.now);
        self.icon.set_theme(cx.theme, cx.now);
        self.icon.set_icon(Icon::named(self.icon_name(cx)));
    }

    fn measure(&mut self, _text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        cx.item
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, highlighted: bool) {
        let colors = &cx.theme.colors;
        let b = Self::button_rect(rect, cx);
        let radii = Corners::uniform(b.width * 0.5);
        if highlighted {
            painter.rect(b, &RectStyle::filled(colors.surface_bright, radii));
        }
        let color = if highlighted || (self.kind == WidgetKind::Notifications && self.unread > 0) {
            colors.primary
        } else {
            colors.on_surface
        };
        draw_hover(painter, &self.hover, b, radii, cx, chip_fill(cx.theme, highlighted), &[color]);
        self.icon.animate(cx.now);
        self.icon.draw_centered(painter, b.center(), color);
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        self.hover.pointer(Self::button_rect(rect, cx), pointer, down, cx.now)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        Self::button_rect(rect, cx).contains(at).then(|| Action::TogglePanel(self.target(cx)))
    }

    fn panel(&self, cx: &WidgetCx) -> Option<PanelId> {
        Some(self.target(cx))
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.hover.is_animating(now) | self.icon.animate(now)
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        Self::button_rect(rect, cx).contains(p).then_some(CursorShape::Pointer)
    }
}
