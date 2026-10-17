//! Gece ışığı kartı (t113): anahtar satırı (alt metinde durum: "Kapalı · açılış 20:00",
//! "Açık · gün doğumunda kapanır", "Başka bir uygulama yönetiyor") ve açıkken gece sıcaklığı
//! kaydırıcısı. Compositor renk dönüşümü sunmuyorsa kart gizlidir.

use std::time::Instant;

use chrono::{DateTime, Local, Utc};
use nuxut::i18n::t;
use nuxut::ui::{Icon, IconName, List, ListEvent, ListItem, ListTrailing, Ui};
use nuxut::{CursorShape, Point, PointerEvent, Rect, TextSystem, Theme};

use super::padding;
use super::slider_row::{RowEvent, SliderRow};
use crate::model::ShellModel;
use crate::services::night_light::config::TEMPERATURE;
use crate::services::night_light::{Command, NightLightView, Status};
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

/// Kaydırıcı adımı (K).
const STEP_KELVIN: u32 = 100;

/// Durum satırı; `time` UTC anını yerel saat metnine çevirir.
pub(super) fn status_text(status: Status, time: impl Fn(DateTime<Utc>) -> String) -> String {
    match status {
        Status::Off => t!("shell-quick-night-light-off"),
        Status::On => t!("shell-quick-night-light-on"),
        Status::OnUntil { sun: true, .. } => t!("shell-quick-night-light-on-until-sunrise"),
        Status::OnUntil { at, sun: false } => t!("shell-quick-night-light-on-until", time = time(at)),
        Status::OffUntil { sun: true, .. } => t!("shell-quick-night-light-off-until-sunset"),
        Status::OffUntil { at, sun: false } => t!("shell-quick-night-light-off-until", time = time(at)),
        Status::Blocked => t!("shell-quick-night-light-blocked"),
    }
}

pub(super) fn local_time(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}

/// Sıcaklık ↔ kaydırıcı (0 = en sıcak, 1 = en soğuk).
fn slider_value(kelvin: u32) -> f32 {
    let (lo, hi) = (*TEMPERATURE.start(), *TEMPERATURE.end());
    (kelvin.clamp(lo, hi) - lo) as f32 / (hi - lo) as f32
}

fn kelvin_from_slider(v: f32) -> u32 {
    let (lo, hi) = (*TEMPERATURE.start(), *TEMPERATURE.end());
    let k = lo as f32 + v.clamp(0.0, 1.0) * (hi - lo) as f32;
    ((k / STEP_KELVIN as f32).round() as u32 * STEP_KELVIN).clamp(lo, hi)
}

#[derive(Debug)]
pub struct NightLightCard {
    view: NightLightView,
    list: List,
    slider: SliderRow,
}

fn build_item(theme: &Theme, view: &NightLightView) -> ListItem {
    ListItem::new(theme, t!("shell-quick-night-light-title"))
        .with_leading(theme, Icon::named(IconName::NightLight))
        .with_subtitle(theme, status_text(view.status, local_time))
        .with_trailing(theme, ListTrailing::Toggle(view.active))
}

impl NightLightCard {
    pub fn new(cx: &WidgetCx) -> Self {
        let view = NightLightView::default();
        let mut list = List::new(cx.theme);
        list.items_mut().push(build_item(cx.theme, &view));
        Self { view, list, slider: SliderRow::new(cx.theme, IconName::LightMode).inert() }
    }

    fn shows_slider(&self) -> bool {
        self.view.active
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

impl PanelWidget for NightLightCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::NightLight
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let view = model.night_light;
        if view == self.view {
            return false;
        }
        self.view = view;
        if let Some(item) = self.list.items_mut().first_mut() {
            item.set_subtitle(cx.theme, Some(&status_text(view.status, local_time)));
            item.set_toggle(view.active, cx.now);
        }
        // Metin olarak: sayı biçimi binlik ayırıcı eklemesin ("3.000 K").
        let text = t!("shell-quick-night-light-kelvin", kelvin = view.temperature.to_string());
        self.slider.set_with_text(IconName::LightMode, slider_value(view.temperature), &text, true, cx.now);
        true
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.list.set_theme(cx.theme, cx.now);
        self.slider.set_theme(cx.theme, cx.now);
    }

    fn height(&mut self, _text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        if !self.view.supported {
            return Some(0.0);
        }
        let (_, slider) = self.layout(Rect::new(0.0, 0.0, width, 0.0), cx.theme);
        Some(slider.bottom() + padding(cx.theme))
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        if !self.view.supported {
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
        if !self.view.supported {
            return (false, None);
        }
        let (list, slider) = self.layout(rect, cx.theme);
        if self.list.rect() != list {
            self.list.set_rect(cx.theme, list, cx.now);
        }
        let r = self.list.pointer(cx.theme, event, cx.now);
        let mut repaint = r.repaint;
        let mut cmd = r.event.map(|(_, e)| match e {
            ListEvent::Toggled(on) => Command::Set(on),
            _ => Command::Toggle,
        });
        if self.shows_slider() {
            let (rp, ev) = self.slider.pointer(event, slider, cx.theme, cx.now);
            repaint |= rp;
            if let Some(RowEvent::Value(v)) = ev {
                cmd = Some(Command::Temperature(kelvin_from_slider(v)));
            }
        }
        (repaint || cmd.is_some(), cmd.map(Action::NightLight))
    }

    /// Anahtar satırı ve açıkken sıcaklık kaydırıcısı.
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        if !self.view.supported {
            return;
        }
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            self.list.access(tree, "switch", |i| i).label(&self.kind().title());
            if self.shows_slider() {
                self.slider.access(tree, "temperature", &t!("shell-a11y-night-light-temperature"), "");
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
        if !self.view.supported || !self.shows_slider() {
            return None;
        }
        let card = crate::widgets::access_card_id(parent, self.kind());
        match self.slider.access_action(card, "temperature", request, cx.now)? {
            RowEvent::Value(v) => Some((true, Some(Action::NightLight(Command::Temperature(kelvin_from_slider(v)))))),
            RowEvent::Icon => None,
        }
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        if !self.view.supported {
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
    use chrono::TimeZone;

    #[test]
    fn status_lines() {
        crate::i18n::use_turkish_for_tests();
        let at = Utc.with_ymd_and_hms(2026, 1, 10, 19, 0, 0).unwrap();
        let fixed = |_| "20:00".to_owned();
        assert_eq!(status_text(Status::Off, fixed), "Kapalı");
        assert_eq!(status_text(Status::OffUntil { at, sun: false }, fixed), "Kapalı · açılış 20:00");
        assert_eq!(status_text(Status::OffUntil { at, sun: true }, fixed), "Kapalı · gün batımında açılır");
        assert_eq!(status_text(Status::OnUntil { at, sun: true }, fixed), "Açık · gün doğumunda kapanır");
        assert_eq!(status_text(Status::OnUntil { at, sun: false }, fixed), "Açık · kapanış 20:00");
        assert_eq!(status_text(Status::Blocked, fixed), "Başka bir uygulama yönetiyor");
    }

    #[test]
    fn slider_maps_temperature_range_in_100k_steps() {
        assert_eq!(slider_value(1500), 0.0);
        assert_eq!(slider_value(6000), 1.0);
        assert_eq!(kelvin_from_slider(0.0), 1500);
        assert_eq!(kelvin_from_slider(1.0), 6000);
        assert_eq!(kelvin_from_slider(0.5), 3800);
        assert_eq!(kelvin_from_slider(slider_value(4000)), 4000);
        assert_eq!(kelvin_from_slider(-3.0), 1500);
    }
}
