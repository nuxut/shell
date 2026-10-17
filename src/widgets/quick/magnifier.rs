//! Büyüteç kartı (t124): anahtar satırı (alt metinde durum: "Kapalı" / "Açık · 2,5×") ve açıkken
//! çarpan kaydırıcısı (1 … `[magnifier] max`, 0,25 adım). Hyprland'e komut gönderilemiyorsa
//! (Hyprland yok, test örneği) kart gizlidir.

use std::time::Instant;

use nuxut::i18n::t;
use nuxut::ui::{Icon, IconName, List, ListEvent, ListItem, ListTrailing, Ui};
use nuxut::{CursorShape, Point, PointerEvent, Rect, TextSystem, Theme};

use super::padding;
use super::slider_row::{RowEvent, SliderRow};
use crate::model::ShellModel;
use crate::services::magnifier::{Command, Factor, MIN_FACTOR, MagnifierView};
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

/// Kaydırıcı adımı (çarpan).
const STEP: f64 = 0.25;

/// Çarpan metni ("2,5×"; ondalık ayırıcı dile göre).
pub(crate) fn factor_text(factor: f64) -> String {
    t!("shell-quick-magnifier-factor", factor = factor)
}

fn status_text(view: &MagnifierView) -> String {
    if view.active() {
        t!("shell-quick-magnifier-on", factor = factor_text(view.factor))
    } else {
        t!("shell-quick-magnifier-off")
    }
}

/// Çarpan ↔ kaydırıcı (0 = 1×, 1 = tavan).
pub(crate) fn slider_value(factor: f64, max: f64) -> f32 {
    if max <= MIN_FACTOR {
        return 0.0;
    }
    ((factor.clamp(MIN_FACTOR, max) - MIN_FACTOR) / (max - MIN_FACTOR)) as f32
}

fn factor_from_slider(v: f32, max: f64) -> f64 {
    let f = MIN_FACTOR + f64::from(v.clamp(0.0, 1.0)) * (max - MIN_FACTOR);
    ((f / STEP).round() * STEP).clamp(MIN_FACTOR, max)
}

#[derive(Debug)]
pub struct MagnifierCard {
    view: MagnifierView,
    list: List,
    slider: SliderRow,
}

fn build_item(theme: &Theme, view: &MagnifierView) -> ListItem {
    ListItem::new(theme, t!("shell-quick-magnifier-title"))
        .with_leading(theme, Icon::named(IconName::Search))
        .with_subtitle(theme, status_text(view))
        .with_trailing(theme, ListTrailing::Toggle(view.active()))
}

impl MagnifierCard {
    pub fn new(cx: &WidgetCx) -> Self {
        let view = MagnifierView::default();
        let mut list = List::new(cx.theme);
        list.items_mut().push(build_item(cx.theme, &view));
        Self { view, list, slider: SliderRow::new(cx.theme, IconName::Search).inert() }
    }

    fn shows_slider(&self) -> bool {
        self.view.active()
    }

    /// (anahtar satırı, kaydırıcı satırı).
    fn layout(&self, rect: Rect, theme: &Theme) -> (Rect, Rect) {
        let p = padding(theme);
        let w = rect.width - 2.0 * p;
        let h = self.list.items().first().map_or(0.0, |i| i.height(theme));
        let list = Rect::new(rect.x + p, rect.y + p, w, h);
        let slider = if self.shows_slider() {
            Rect::new(rect.x + p, list.bottom() + theme.space(1), w, self.slider.height(theme))
        } else {
            Rect::new(rect.x + p, list.bottom(), w, 0.0)
        };
        (list, slider)
    }
}

impl PanelWidget for MagnifierCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Magnifier
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let view = model.magnifier;
        if view == self.view {
            return false;
        }
        self.view = view;
        if let Some(item) = self.list.items_mut().first_mut() {
            item.set_subtitle(cx.theme, Some(&status_text(&view)));
            item.set_toggle(view.active(), cx.now);
        }
        self.slider
            .set_with_text(IconName::Search, slider_value(view.factor, view.max), &factor_text(view.factor), true, cx.now);
        true
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.list.set_theme(cx.theme, cx.now);
        self.slider.set_theme(cx.theme, cx.now);
    }

    fn height(&mut self, _text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        if !self.view.available {
            return Some(0.0);
        }
        let (_, slider) = self.layout(Rect::new(0.0, 0.0, width, 0.0), cx.theme);
        Some(slider.bottom() + padding(cx.theme))
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        if !self.view.available {
            return;
        }
        let (list, slider) = self.layout(rect, cx.theme);
        if self.list.rect() != list {
            self.list.set_rect(cx.theme, list, cx.now);
        }
        self.list.draw(ui);
        if self.shows_slider() {
            self.slider.draw(ui, slider);
        }
    }

    fn pointer(&mut self, event: &PointerEvent, rect: Rect, _text: &mut TextSystem, cx: &WidgetCx) -> (bool, Option<Action>) {
        if !self.view.available {
            return (false, None);
        }
        let (list, slider) = self.layout(rect, cx.theme);
        if self.list.rect() != list {
            self.list.set_rect(cx.theme, list, cx.now);
        }
        let r = self.list.pointer(cx.theme, event, cx.now);
        let mut repaint = r.repaint;
        let mut cmd = r.event.map(|(_, e)| match e {
            ListEvent::Toggled(true) => Command::On,
            ListEvent::Toggled(false) => Command::Off,
            _ => Command::Toggle,
        });
        if self.shows_slider() {
            let (rp, ev) = self.slider.pointer(event, slider, cx.theme, cx.now);
            repaint |= rp;
            if let Some(RowEvent::Value(v)) = ev {
                cmd = Some(Command::Set(Factor::new(factor_from_slider(v, self.view.max))));
            }
        }
        (repaint || cmd.is_some(), cmd.map(Action::Magnifier))
    }

    /// Anahtar satırı ve açıkken çarpan kaydırıcısı (değer metni "2,0×").
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        if !self.view.available {
            return;
        }
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            self.list.access(tree, "switch", |i| i).label(&self.kind().title());
            if self.shows_slider() {
                self.slider.access(tree, "factor", &t!("shell-a11y-magnifier-factor"), "");
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
        if !self.view.available || !self.shows_slider() {
            return None;
        }
        let card = crate::widgets::access_card_id(parent, self.kind());
        match self.slider.access_action(card, "factor", request, cx.now)? {
            RowEvent::Value(v) => Some((true, Some(Action::Magnifier(Command::Set(Factor::new(factor_from_slider(v, self.view.max))))))),
            RowEvent::Icon => None,
        }
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        if !self.view.available {
            return None;
        }
        let slider = if self.shows_slider() { self.slider.cursor(p) } else { None };
        slider.or_else(|| self.list.cursor(p))
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines() {
        crate::i18n::use_turkish_for_tests();
        let off = MagnifierView { available: true, factor: 1.0, max: 8.0 };
        let on = MagnifierView { available: true, factor: 2.5, max: 8.0 };
        assert_eq!(status_text(&off), "Kapalı");
        assert_eq!(status_text(&on), "Açık · 2,5×");
        assert_eq!(factor_text(2.0), "2,0×");
        assert_eq!(factor_text(1.953), "1,95×");
    }

    #[test]
    fn slider_maps_factor_in_quarter_steps() {
        assert_eq!(slider_value(1.0, 8.0), 0.0);
        assert_eq!(slider_value(8.0, 8.0), 1.0);
        assert_eq!(slider_value(20.0, 8.0), 1.0);
        assert_eq!(factor_from_slider(0.0, 8.0), 1.0);
        assert_eq!(factor_from_slider(1.0, 8.0), 8.0);
        assert_eq!(factor_from_slider(0.5, 8.0), 4.5);
        assert_eq!(factor_from_slider(0.51, 8.0), 4.5);
        assert_eq!(factor_from_slider(-1.0, 8.0), 1.0);
        assert_eq!(factor_from_slider(slider_value(2.5, 8.0), 8.0), 2.5);
        assert_eq!(slider_value(3.0, 1.0), 0.0, "bozuk tavan");
    }
}
