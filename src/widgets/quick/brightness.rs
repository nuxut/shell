//! Parlaklık kartı: ayarlanabilen her ekran için bir kaydırıcı (dahili panel ve DDC/CI
//! destekli harici monitörler). Birden çok ekranda satırın üstünde ekran adı yazılır.
//! Ekran yoksa kart gizlidir.

use std::time::Instant;

use nuxut::ui::{IconName, Label, TextRole, Ui, text_style};
use nuxut::{CursorShape, Point, PointerEvent, Rect, TextSystem};

use super::slider_row::{RowEvent, SliderRow};
use super::{draw_error, error_height, padding, set_flag};
use crate::model::ShellModel;
use crate::services::brightness::{Display, DisplayKind};
use crate::system::{ErrorSource, SystemCommand};
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

/// Kartta gösterilen en fazla ekran.
const MAX_ROWS: usize = 4;

#[derive(Debug)]
struct Row {
    id: String,
    caption: Label,
    slider: SliderRow,
}

#[derive(Debug)]
pub struct BrightnessCard {
    rows: Vec<Row>,
    error: Label,
    has_error: bool,
}

fn caption_text(d: &Display) -> String {
    match (d.kind, &d.output) {
        (DisplayKind::Internal, _) => "Dahili ekran".to_owned(),
        (DisplayKind::External, Some(o)) => o.clone(),
        (DisplayKind::External, None) => "Harici ekran".to_owned(),
    }
}

impl BrightnessCard {
    pub fn new(cx: &WidgetCx) -> Self {
        Self { rows: Vec::new(), error: Label::new("", text_style(cx.theme, TextRole::Caption)), has_error: false }
    }

    fn captions(&self) -> bool {
        self.rows.len() > 1
    }

    /// Satır dikdörtgenleri: (başlık konumu, kaydırıcı satırı).
    fn layout(&mut self, text: &mut TextSystem, rect: Rect, cx: &WidgetCx) -> Vec<(Point, Rect)> {
        let p = padding(cx.theme);
        let captions = self.captions();
        let mut y = rect.y + p;
        let mut out = Vec::with_capacity(self.rows.len());
        for r in &mut self.rows {
            let cap = Point::new(rect.x + p, y);
            if captions {
                y += r.caption.measure(text).height;
            }
            let h = r.slider.height(cx.theme);
            out.push((cap, Rect::new(rect.x + p, y, rect.width - 2.0 * p, h)));
            y += h + cx.theme.space(1);
        }
        out
    }
}

impl PanelWidget for BrightnessCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Brightness
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let displays = &model.system.displays[..model.system.displays.len().min(MAX_ROWS)];
        let mut changed = false;
        if self.rows.len() != displays.len() || self.rows.iter().zip(displays).any(|(r, d)| r.id != d.id) {
            self.rows = displays
                .iter()
                .map(|d| Row {
                    id: d.id.clone(),
                    caption: Label::new(caption_text(d), text_style(cx.theme, TextRole::Caption)),
                    slider: SliderRow::new(cx.theme, IconName::Brightness).inert(),
                })
                .collect();
            changed = true;
        }
        for (r, d) in self.rows.iter_mut().zip(displays) {
            let pct = d.percent.clamp(0.0, 100.0);
            changed |= r.slider.set(IconName::Brightness, (pct / 100.0) as f32, pct.round() as u32, true, cx.now);
        }
        let err = model.system.error_for(ErrorSource::Brightness).unwrap_or_default();
        changed |= set_flag(&mut self.has_error, !err.is_empty());
        changed |= self.error.set_text(err);
        changed
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        for r in &mut self.rows {
            r.caption.set_style(text_style(cx.theme, TextRole::Caption));
            r.slider.set_theme(cx.theme, cx.now);
        }
        self.error.set_style(text_style(cx.theme, TextRole::Caption));
    }

    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        if self.rows.is_empty() {
            return Some(0.0);
        }
        let p = padding(cx.theme);
        let rows = self.layout(text, Rect::new(0.0, 0.0, width, 0.0), cx);
        let bottom = rows.last().map_or(0.0, |(_, r)| r.bottom());
        self.error.set_max_width(Some((width - 2.0 * p).max(0.0)));
        Some(bottom + error_height(&mut self.error, text, cx.theme, self.has_error) + p)
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        let layout = self.layout(ui.painter.text_system(), rect, cx);
        let captions = self.captions();
        let muted = ui.fade(cx.theme.colors.on_surface_variant);
        for (r, (cap, row)) in self.rows.iter_mut().zip(&layout) {
            if captions {
                r.caption.draw(ui.painter, *cap, muted);
            }
            r.slider.draw(ui, *row);
        }
        if self.has_error
            && let Some((_, last)) = layout.last()
        {
            draw_error(ui, &mut self.error, Point::new(last.x, last.bottom() + cx.theme.space(1)), cx.theme);
        }
    }

    fn pointer(&mut self, event: &PointerEvent, rect: Rect, text: &mut TextSystem, cx: &WidgetCx) -> (bool, Option<Action>) {
        let layout = self.layout(text, rect, cx);
        let mut repaint = false;
        let mut action = None;
        for (r, (_, row)) in self.rows.iter_mut().zip(&layout) {
            let (rp, ev) = r.slider.pointer(event, *row, cx.theme, cx.now);
            repaint |= rp;
            if let Some(RowEvent::Value(v)) = ev {
                let percent = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
                action = Some(Action::System(SystemCommand::SetBrightness { id: r.id.clone(), percent }));
            }
        }
        (repaint || action.is_some(), action)
    }

    /// Ekran başına kaydırıcı; ad ekranın başlığı (tek ekranda kart adı). Anahtar ekran kimliği.
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        if self.rows.is_empty() {
            return;
        }
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            for r in &self.rows {
                let name = if r.caption.text().is_empty() { self.kind().title() } else { r.caption.text().to_owned() };
                r.slider.access(tree, &r.id, &name, "");
            }
        });
    }

    fn access_action(
        &mut self,
        request: &nuxut::access::AccessRequest,
        parent: nuxut::access::NodeId,
        _rect: Rect,
        cx: &WidgetCx,
    ) -> Option<(bool, Option<Action>)> {
        let card = crate::widgets::access_card_id(parent, self.kind());
        self.rows.iter_mut().find_map(|r| match r.slider.access_action(card, &r.id, request, cx.now)? {
            RowEvent::Value(v) => {
                let percent = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
                Some((true, Some(Action::System(SystemCommand::SetBrightness { id: r.id.clone(), percent }))))
            }
            RowEvent::Icon => None,
        })
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        self.rows.iter().find_map(|r| r.slider.cursor(p))
    }

    fn corner_clearance(&self, cx: &WidgetCx) -> nuxut::Size {
        let p = padding(cx.theme);
        nuxut::Size::new(p, p)
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captions_name_displays() {
        let d = |kind, output: Option<&str>| Display { id: "x".into(), kind, output: output.map(Into::into), percent: 0.0 };
        assert_eq!(caption_text(&d(DisplayKind::Internal, None)), "Dahili ekran");
        assert_eq!(caption_text(&d(DisplayKind::External, Some("DP-1"))), "DP-1");
        assert_eq!(caption_text(&d(DisplayKind::External, None)), "Harici ekran");
    }
}
