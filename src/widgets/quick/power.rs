//! Güç kartı: pil durumu (varsa), güç profili seçimi (power-profiles-daemon varsa) ve pil şarj
//! sınırı anahtarı ("Pil ömrünü koru"; yalnızca UPower destekleyen pil bildirirse).
//! Pil de profil de yoksa (masaüstü, ppd kurulu değil) kart gizlidir.

use std::time::Instant;

use nuxut::i18n::t;
use nuxut::ui::{
    Icon, IconName, IconView, Label, List, ListEvent, ListItem, ListTrailing, Segment, SegmentedButton, TextRole, Ui,
    text_style,
};
use nuxut::{CursorShape, Point, PointerEvent, Rect, TextSystem};

use super::{battery_icon, draw_error, error_height, format_duration, padding, profile_icon, set_flag};
use crate::model::ShellModel;
use crate::services::power::{Battery, ChargeLimit, ChargeState, PowerProfile, PowerState};
use crate::system::{ErrorSource, SystemCommand};
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

#[derive(Debug)]
pub struct PowerCard {
    battery_icon: IconView,
    battery: Label,
    has_battery: bool,
    /// Seçici ve sıradaki profiller (`available` sırası).
    profiles: Option<(SegmentedButton, Vec<PowerProfile>)>,
    /// Performans kısıtı (ppd `PerformanceDegraded`).
    degraded: Label,
    has_degraded: bool,
    /// Şarj sınırı anahtar satırı (tek öğeli liste) ve gösterilen alt metin; desteklenmiyorsa `None`.
    charge: Option<(List, String)>,
    /// Modeldeki şarj sınırı durumu (anahtar buna döner; ret sonrası geri çevrilir).
    charge_on: bool,
    error: Label,
    has_error: bool,
}

/// Şarj sınırı alt metni: karışık durum önce; eşik biliniyorsa gerçek değer, yoksa genel.
fn charge_subtitle(limit: ChargeLimit) -> Option<String> {
    let ChargeLimit::Supported { mixed, end, .. } = limit else { return None };
    Some(if mixed {
        t!("shell-quick-power-charge-limit-mixed")
    } else {
        match end {
            Some(n) => t!("shell-quick-power-charge-limit-at", end = f64::from(n) / 100.0),
            None => t!("shell-quick-power-charge-limit-firmware"),
        }
    })
}

/// Seçicide kısa ad (dar panelde kesilmesin).
fn short_label(p: PowerProfile) -> String {
    match p {
        PowerProfile::PowerSaver => t!("shell-quick-power-profile-saver"),
        PowerProfile::Balanced => t!("shell-quick-power-profile-balanced"),
        PowerProfile::Performance => t!("shell-quick-power-profile-performance"),
    }
}

/// Pil satırı: "%64 · 2 sa 10 dk kaldı", "%40 · şarj oluyor · 35 dk içinde dolar", "%100 · dolu".
fn battery_text(b: &Battery) -> String {
    let pct = crate::percent::ratio(b.percentage);
    match b.state {
        ChargeState::FullyCharged => t!("shell-quick-battery-full", percent = pct),
        s if s.is_charging() => match b.time_to_full {
            Some(d) => t!("shell-quick-battery-charging-time", percent = pct, time = format_duration(d)),
            None => t!("shell-quick-battery-charging", percent = pct),
        },
        ChargeState::Discharging => match b.time_to_empty {
            Some(d) => t!("shell-quick-battery-discharging-time", percent = pct, time = format_duration(d)),
            None => t!("shell-quick-percent-value", percent = pct),
        },
        _ => t!("shell-quick-percent-value", percent = pct),
    }
}

/// ppd kısıt nedeni → metin.
fn degraded_text(reason: &str) -> String {
    match reason {
        "lap-detected" => t!("shell-quick-power-degraded-lap"),
        "high-operating-temperature" => t!("shell-quick-power-degraded-temperature"),
        _ => t!("shell-quick-power-degraded-fallback"),
    }
}

impl PowerCard {
    pub fn new(cx: &WidgetCx) -> Self {
        let t = cx.theme;
        Self {
            battery_icon: IconView::new(t, Icon::named(IconName::BatteryUnknown)),
            battery: Label::new("", text_style(t, TextRole::Body)),
            has_battery: false,
            profiles: None,
            degraded: Label::new("", text_style(t, TextRole::Caption)),
            has_degraded: false,
            charge: None,
            charge_on: false,
            error: Label::new("", text_style(t, TextRole::Caption)),
            has_error: false,
        }
    }

    fn sync_profiles(&mut self, power: &PowerState, cx: &WidgetCx) -> bool {
        let Some(p) = power.profiles.as_ref().filter(|p| !p.available.is_empty()) else {
            return self.profiles.take().is_some();
        };
        let selected = p.active.and_then(|a| p.available.iter().position(|x| *x == a)).unwrap_or(0);
        match &mut self.profiles {
            Some((seg, list)) if *list == p.available => {
                seg.set_selected(selected, cx.now);
                false
            }
            _ => {
                let segments =
                    p.available.iter().map(|x| Segment::new(short_label(*x)).with_icon(Icon::named(profile_icon(*x)))).collect();
                self.profiles = Some((SegmentedButton::new(cx.theme, segments, selected), p.available.clone()));
                true
            }
        }
    }

    /// Şarj sınırı satırını modele eşitler. Anahtar her eşitlemede modelin durumuna döner:
    /// istek reddedilirse (polkit) tıklamayla çevrilmiş anahtar geri gelir.
    fn sync_charge(&mut self, limit: ChargeLimit, cx: &WidgetCx) -> bool {
        let Some(subtitle) = charge_subtitle(limit) else {
            return self.charge.take().is_some();
        };
        let on = limit.is_enabled();
        let t = cx.theme;
        let changed = match &mut self.charge {
            Some((list, shown)) => {
                let differs = *shown != subtitle;
                if differs {
                    if let Some(item) = list.items_mut().first_mut() {
                        item.set_subtitle(t, Some(&subtitle));
                    }
                    *shown = subtitle;
                }
                differs || self.charge_on != on
            }
            None => {
                let item = ListItem::new(t, t!("shell-quick-power-charge-limit"))
                    .with_leading(t, Icon::named(IconName::Battery80))
                    .with_subtitle(t, subtitle.clone())
                    .with_trailing(t, ListTrailing::Toggle(on));
                self.charge = Some((List::new(t).with_items(vec![item]), subtitle));
                true
            }
        };
        self.charge_on = on;
        if let Some(item) = self.charge.as_mut().and_then(|(l, _)| l.items_mut().first_mut()) {
            item.set_toggle(on, cx.now);
        }
        changed
    }

    /// (pil satırı, seçici, kısıt satırı, şarj sınırı satırı, alt) dikey konumları.
    fn layout(&mut self, text: &mut TextSystem, rect: Rect, cx: &WidgetCx) -> (Rect, Rect, Point, Rect, f32) {
        let t = cx.theme;
        let p = padding(t);
        let w = rect.width - 2.0 * p;
        let mut y = rect.y + p;
        let battery = if self.has_battery {
            let h = self.battery_icon.measure(text).height.max(self.battery.measure(text).height);
            let r = Rect::new(rect.x + p, y, w, h);
            y += h + t.space(2);
            r
        } else {
            Rect::ZERO
        };
        let seg = match &mut self.profiles {
            Some((s, _)) => {
                let h = s.preferred_size(text, t).height;
                let r = Rect::new(rect.x + p, y, w, h);
                y += h + t.space(1);
                r
            }
            None => Rect::ZERO,
        };
        let degraded = Point::new(rect.x + p, y);
        if self.has_degraded {
            y += self.degraded.measure(text).height + t.space(1);
        }
        let charge = match &self.charge {
            Some((list, _)) => {
                let h = list.items().first().map_or(0.0, |i| i.height(t));
                let r = Rect::new(rect.x + p, y, w, h);
                y += h + t.space(1);
                r
            }
            None => Rect::ZERO,
        };
        (battery, seg, degraded, charge, y)
    }
}

impl PanelWidget for PowerCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Power
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let power = &model.system.power;
        let battery = power.battery();
        let mut changed = set_flag(&mut self.has_battery, battery.is_some());
        if let Some(b) = &battery {
            changed |= self.battery_icon.set_icon(Icon::named(battery_icon(b)));
            changed |= self.battery.set_text(&battery_text(b));
        }
        changed |= self.sync_profiles(power, cx);
        changed |= self.sync_charge(power.charge_limit(), cx);
        let degraded = power.profiles.as_ref().and_then(|p| p.degraded.as_deref()).map(degraded_text).unwrap_or_default();
        changed |= set_flag(&mut self.has_degraded, !degraded.is_empty());
        changed |= self.degraded.set_text(&degraded);
        let err = model.system.error_for(ErrorSource::Power).unwrap_or_default();
        changed |= set_flag(&mut self.has_error, !err.is_empty());
        changed |= self.error.set_text(err);
        changed
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        let t = cx.theme;
        self.battery_icon.set_theme(t, cx.now);
        self.battery.set_style(text_style(t, TextRole::Body));
        if let Some((s, _)) = &mut self.profiles {
            s.set_theme(t, cx.now);
        }
        self.degraded.set_style(text_style(t, TextRole::Caption));
        if let Some((list, _)) = &mut self.charge {
            list.set_theme(t, cx.now);
        }
        self.error.set_style(text_style(t, TextRole::Caption));
    }

    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        if !self.has_battery && self.profiles.is_none() && self.charge.is_none() {
            return Some(0.0);
        }
        let p = padding(cx.theme);
        let (_, _, _, _, bottom) = self.layout(text, Rect::new(0.0, 0.0, width, 0.0), cx);
        self.error.set_max_width(Some((width - 2.0 * p).max(0.0)));
        self.degraded.set_max_width(Some((width - 2.0 * p).max(0.0)));
        Some(bottom + error_height(&mut self.error, text, cx.theme, self.has_error) + p)
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        let t = cx.theme;
        let (battery, seg, degraded, charge, bottom) = self.layout(ui.painter.text_system(), rect, cx);
        if self.has_battery {
            let s = self.battery_icon.size();
            let on_surface = ui.fade(t.colors.on_surface);
            self.battery_icon.draw_centered(ui.painter, Point::new(battery.x + s.width * 0.5, battery.center().y), on_surface);
            let m = self.battery.measure_in(ui.painter);
            let at = Point::new(battery.x + s.width + t.space(2), battery.center().y - m.height * 0.5);
            self.battery.draw(ui.painter, at, on_surface);
        }
        if let Some((s, _)) = &mut self.profiles {
            s.set_rect(t, seg);
            s.draw(ui);
        }
        if self.has_degraded {
            let c = ui.fade(t.colors.on_surface_variant);
            self.degraded.draw(ui.painter, degraded, c);
        }
        if let Some((list, _)) = &mut self.charge {
            if list.rect() != charge {
                list.set_rect(t, charge, cx.now);
            }
            list.draw(ui);
        }
        if self.has_error {
            draw_error(ui, &mut self.error, Point::new(rect.x + padding(t), bottom), t);
        }
    }

    fn pointer(&mut self, event: &PointerEvent, rect: Rect, text: &mut TextSystem, cx: &WidgetCx) -> (bool, Option<Action>) {
        let (_, seg, _, charge, _) = self.layout(text, rect, cx);
        let mut repaint = false;
        if let Some((s, list)) = &mut self.profiles {
            s.set_rect(cx.theme, seg);
            let r = s.pointer(event, cx.now);
            if let Some(p) = r.event.and_then(|i| list.get(i)) {
                return (true, Some(Action::System(SystemCommand::SetPowerProfile(*p))));
            }
            repaint |= r.repaint;
        }
        if let Some((list, _)) = &mut self.charge {
            if list.rect() != charge {
                list.set_rect(cx.theme, charge, cx.now);
            }
            let r = list.pointer(cx.theme, event, cx.now);
            repaint |= r.repaint;
            let on = match r.event {
                Some((_, ListEvent::Toggled(on))) => Some(on),
                Some((_, ListEvent::Clicked)) => Some(!self.charge_on),
                _ => None,
            };
            if let Some(on) = on {
                return (true, Some(Action::System(SystemCommand::SetChargeLimit(on))));
            }
        }
        (repaint, None)
    }

    /// Pil durumu (etiket), güç profili (radyo grubu) ve şarj sınırı anahtarı.
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        if !self.has_battery && self.profiles.is_none() && self.charge.is_none() {
            return;
        }
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            if self.has_battery {
                self.battery.access(tree, "battery", rect);
            }
            if self.has_degraded {
                self.degraded.access(tree, "degraded", rect);
            }
            if let Some((s, _)) = &self.profiles {
                s.access(tree, "profile", false).label(&t!("shell-a11y-power-profile"));
            }
            if let Some((list, _)) = &self.charge {
                list.access(tree, "charge", |i| i).label(&t!("shell-a11y-power-charge"));
            }
            if self.has_error {
                self.error.access(tree, "error", rect);
            }
        });
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        self.profiles.as_ref().and_then(|(s, _)| s.cursor(p)).or_else(|| self.charge.as_ref().and_then(|(l, _)| l.cursor(p)))
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.battery_icon.animate(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use nuxut::{Button, PointerKind};

    fn battery(percentage: f64, state: ChargeState, empty: Option<u64>, full: Option<u64>) -> Battery {
        Battery {
            percentage,
            state,
            time_to_empty: empty.map(Duration::from_secs),
            time_to_full: full.map(Duration::from_secs),
            icon_name: String::new(),
            warning: Default::default(),
        }
    }

    #[test]
    fn battery_line_describes_state() {
        crate::i18n::use_turkish_for_tests();
        assert_eq!(battery_text(&battery(64.4, ChargeState::Discharging, Some(7800), None)), "%64 · 2 sa 10 dk kaldı");
        assert_eq!(battery_text(&battery(64.0, ChargeState::Discharging, None, None)), "%64");
        assert_eq!(
            battery_text(&battery(40.0, ChargeState::Charging, None, Some(35 * 60))),
            "%40 · şarj oluyor · 35 dk içinde dolar"
        );
        assert_eq!(battery_text(&battery(100.0, ChargeState::FullyCharged, None, None)), "%100 · dolu");
    }

    #[test]
    fn degraded_reasons_are_translated() {
        crate::i18n::use_turkish_for_tests();
        assert_eq!(degraded_text("lap-detected"), "Performans kısıtlı: kucakta kullanım algılandı");
        assert_eq!(degraded_text("other"), "Performans kısıtlı: sistem kısıtlıyor");
    }

    #[test]
    fn charge_subtitle_uses_real_threshold() {
        crate::i18n::use_turkish_for_tests();
        let s = |mixed, end| charge_subtitle(ChargeLimit::Supported { enabled: true, mixed, start: Some(75), end });
        assert_eq!(s(false, Some(80)).as_deref(), Some("Şarj %80 olunca durur"));
        assert_eq!(s(false, None).as_deref(), Some("Şarj tam dolmadan durur"));
        assert_eq!(s(true, Some(80)).as_deref(), Some("Pillerin bir kısmında açık"));
        assert_eq!(charge_subtitle(ChargeLimit::Unsupported), None);
    }

    /// Şarj sınırını destekleyen tek pilli durum.
    fn power_with_charge(enabled: bool) -> PowerState {
        use zbus::zvariant::{OwnedValue, Value};
        let props: nuxut_dbus::PropMap = [
            ("Type", Value::U32(2)),
            ("PowerSupply", Value::Bool(true)),
            ("ChargeThresholdSupported", Value::Bool(true)),
            ("ChargeThresholdEnabled", Value::Bool(enabled)),
            ("ChargeStartThreshold", Value::U32(75)),
            ("ChargeEndThreshold", Value::U32(80)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), OwnedValue::try_from(v).unwrap()))
        .collect();
        let mut s = PowerState::default();
        s.upower = true;
        s.track_battery("/org/freedesktop/UPower/devices/battery_BAT0", &props, 1);
        s
    }

    #[test]
    fn charge_switch_sends_command_and_follows_model() {
        crate::i18n::use_turkish_for_tests();
        let theme = nuxut::Theme::default();
        let settings = crate::widgets::WidgetSettings::default();
        let cx = WidgetCx {
            theme: &theme,
            settings: &settings,
            vertical: false,
            item: 26.0,
            now: Instant::now(),
            bounds: Rect::new(0.0, 0.0, 1920.0, 1080.0),
        };
        let mut text = TextSystem::new();
        let mut card = PowerCard::new(&cx);
        let mut model = ShellModel::default();
        // Desteklenmiyor: satır yok, pil/profil de yoksa kart gizli.
        card.sync(&model, &cx);
        assert!(card.charge.is_none());
        assert_eq!(card.height(&mut text, 360.0, &cx), Some(0.0));

        model.system.power = power_with_charge(false);
        assert!(card.sync(&model, &cx));
        assert!(card.charge.is_some() && !card.charge_on);
        assert!(card.height(&mut text, 360.0, &cx).is_some_and(|h| h > 0.0));

        // Satıra tıklama → aç komutu.
        let rect = Rect::new(0.0, 0.0, 360.0, card.height(&mut text, 360.0, &cx).unwrap());
        let (_, _, _, row, _) = card.layout(&mut text, rect, &cx);
        let at = row.center();
        let mut action = None;
        for kind in [PointerKind::Enter, PointerKind::Press(Button::Left), PointerKind::Release(Button::Left)] {
            let (_, a) = card.pointer(&PointerEvent::new(at, kind), rect, &mut text, &cx);
            action = action.or(a);
        }
        assert_eq!(action, Some(Action::System(SystemCommand::SetChargeLimit(true))));

        // Ret: model değişmedi, anahtar modele döner; açılınca modeli izler.
        card.sync(&model, &cx);
        assert!(!card.charge_on);
        model.system.power = power_with_charge(true);
        assert!(card.sync(&model, &cx));
        assert!(card.charge_on);

        // Pil çıktı / UPower gitti: satır kalkar.
        model.system.power = PowerState::default();
        assert!(card.sync(&model, &cx));
        assert!(card.charge.is_none());
    }
}
