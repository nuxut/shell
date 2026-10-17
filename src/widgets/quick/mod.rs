//! Hızlı ayar kartları (kenar paneli widget'ları): hızlı anahtarlar, ses, ses karıştırıcı, parlaklık, gece ışığı, güç, ağ, Bluetooth,
//! medya.
//!
//! Kartlar `ShellModel::system`'i okur, yalnızca `Action::System` döndürür; servis tutamağı
//! tutmaz. İkon seçimi (seviye → `IconName`) burada: servisler UI sözlüğü bilmez.

mod bluetooth;
mod brightness;
pub(crate) mod magnifier;
mod osk;
mod media;
mod mixer;
mod network;
mod night_light;
mod power;
mod slider_row;
mod toggles;
mod volume;

pub use bluetooth::BluetoothCard;
pub use brightness::BrightnessCard;
pub use magnifier::MagnifierCard;
pub use osk::OskCard;
pub use media::MediaCard;
pub use mixer::MixerCard;
pub use network::NetworkCard;
pub use night_light::NightLightCard;
pub use power::PowerCard;
pub use toggles::TogglesCard;
pub use volume::VolumeCard;

use std::time::Instant;

use chrono::{DateTime, Local};
use nuxut::i18n::t;
use nuxut::ui::{IconName, Label, Ui};
use nuxut::{Corners, CursorShape, KeyEvent, Point, PointerEvent, Rect, RectStyle, TextSystem, Theme};

use crate::model::ShellModel;
use crate::services::power::{Battery, ChargeState, PowerProfile};
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

/// Kart iç boşluğu.
fn padding(theme: &Theme) -> f32 {
    theme.space(2)
}

/// Algısal seviye (1.0 = %100) → ikon.
pub fn volume_icon(volume: f32, muted: bool) -> IconName {
    match volume {
        _ if muted || volume <= 0.0 => IconName::VolumeMute,
        v if v < 0.34 => IconName::VolumeLow,
        v if v < 0.67 => IconName::VolumeMedium,
        _ => IconName::VolumeHigh,
    }
}

pub fn mic_icon(muted: bool) -> IconName {
    if muted { IconName::MicOff } else { IconName::Mic }
}

/// Wi-Fi sinyal gücü (0..=100) → ikon.
pub fn wifi_icon(strength: u8) -> IconName {
    match strength {
        0..=24 => IconName::Wifi1,
        25..=49 => IconName::Wifi2,
        50..=74 => IconName::Wifi3,
        _ => IconName::Wifi4,
    }
}

pub fn profile_icon(p: PowerProfile) -> IconName {
    match p {
        PowerProfile::PowerSaver => IconName::PowerSaver,
        PowerProfile::Balanced => IconName::Balanced,
        PowerProfile::Performance => IconName::Performance,
    }
}

pub fn battery_icon(b: &Battery) -> IconName {
    if b.state.is_charging() {
        return IconName::BatteryCharging;
    }
    if b.state == ChargeState::Unknown {
        return IconName::BatteryUnknown;
    }
    const LEVELS: [IconName; 11] = [
        IconName::Battery0,
        IconName::Battery10,
        IconName::Battery20,
        IconName::Battery30,
        IconName::Battery40,
        IconName::Battery50,
        IconName::Battery60,
        IconName::Battery70,
        IconName::Battery80,
        IconName::Battery90,
        IconName::Battery100,
    ];
    // En yakın onluk (yarım yukarı); 0..=100 dışı değerler model tarafından kırpılır.
    LEVELS[((b.percentage + 5.0) / 10.0).clamp(0.0, 10.0) as usize]
}

/// BlueZ `Icon` özelliği (freedesktop adı: `audio-headphones`, `input-mouse`…) → ikon.
pub fn bluetooth_device_icon(icon: Option<&str>) -> IconName {
    match icon.unwrap_or_default() {
        "audio-headphones" | "audio-headset" => IconName::Headphones,
        "audio-card" | "audio-speakers" => IconName::Speaker,
        "input-keyboard" => IconName::Keyboard,
        "input-mouse" | "input-tablet" => IconName::Mouse,
        "computer" => IconName::Desktop,
        _ => IconName::Bluetooth,
    }
}

/// Süre → "2 sa 5 dk" / "35 dk" / "1 dk".
pub fn format_duration(d: std::time::Duration) -> String {
    let mins = d.as_secs().div_ceil(60);
    match (mins / 60, mins % 60) {
        (0, m) => t!("shell-quick-duration-minutes", minutes = m.max(1)),
        (h, 0) => t!("shell-quick-duration-hours", hours = h),
        (h, m) => t!("shell-quick-duration-hours-minutes", hours = h, minutes = m),
    }
}

/// Parça konumu → "3:07" / "1:02:03".
pub fn format_position(d: std::time::Duration) -> String {
    let s = d.as_secs();
    match s / 3600 {
        0 => format!("{}:{:02}", s / 60, s % 60),
        h => format!("{h}:{:02}:{:02}", (s / 60) % 60, s % 60),
    }
}

/// Bayrağı günceller; değiştiyse `true`.
fn set_flag(slot: &mut bool, value: bool) -> bool {
    std::mem::replace(slot, value) != value
}

/// Kartın altındaki hata satırı (varsa): yüksekliği.
fn error_height(label: &mut Label, text: &mut nuxut::TextSystem, theme: &Theme, shown: bool) -> f32 {
    if shown { theme.space(1) + label.measure(text).height } else { 0.0 }
}

/// Kart çerçevesi: içteki kartı panelin içerik sütununa (diğer panel widget'larının yatay
/// iç boşluğu, [`column_inset`]) oturtur ve altına tonal yüzey çizer — panel zemininden
/// (`surface`) bir ton yukarı (DESIGN "Kenarlar ve ayrım": kenar çizgisi yok). Kartın kendi
/// iç boşluğu ([`padding`]) yüzeyin içinde kalır; kartlar arası yalnızca yığın aralığı.
#[derive(Debug)]
pub struct Card<W>(pub W);

/// Panel widget'larının ortak yatay iç boşluğu (bildirim merkezi başlığıyla aynı sütun).
fn column_inset(theme: &Theme) -> f32 {
    theme.space(4)
}

fn column(rect: Rect, theme: &Theme) -> Rect {
    let m = column_inset(theme).min(rect.width * 0.5);
    Rect::new(rect.x + m, rect.y, rect.width - 2.0 * m, rect.height)
}

impl<W: PanelWidget + 'static> PanelWidget for Card<W> {
    fn kind(&self) -> WidgetKind {
        self.0.kind()
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        self.0.sync(model, cx)
    }

    fn set_time(&mut self, now: &DateTime<Local>, cx: &WidgetCx) -> bool {
        self.0.set_time(now, cx)
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.0.set_theme(cx);
    }

    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        let w = (width - 2.0 * column_inset(cx.theme)).max(0.0);
        self.0.height(text, w, cx)
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        let r = column(rect, cx.theme);
        let t = cx.theme;
        ui.painter.rect(r, &RectStyle::filled(ui.fade(t.colors.surface_container), Corners::uniform(t.shape.radius_medium)));
        self.0.draw(ui, r, cx);
    }

    fn pointer(
        &mut self,
        event: &PointerEvent,
        rect: Rect,
        text: &mut TextSystem,
        cx: &WidgetCx,
    ) -> (bool, Option<Action>) {
        self.0.pointer(event, column(rect, cx.theme), text, cx)
    }

    fn key(&mut self, event: &KeyEvent, cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        self.0.key(event, cx)
    }

    fn paste(&mut self, text: &str, cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        self.0.paste(text, cx)
    }

    fn panel_opened(&mut self, now: Instant) -> Option<Action> {
        self.0.panel_opened(now)
    }

    fn corner_clearance(&self, cx: &WidgetCx) -> nuxut::Size {
        // Sütun oluğu + kartın iç boşluğu (yatay); dikeyde yalnızca iç boşluk.
        let p = padding(cx.theme);
        nuxut::Size::new(column_inset(cx.theme) + p, p)
    }

    fn panel_closed(&mut self, now: Instant) {
        self.0.panel_closed(now);
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        self.0.cursor(column(rect, cx.theme), p, cx)
    }

    fn calendar_range(&self) -> Option<(i64, i64)> {
        self.0.calendar_range()
    }

    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, cx: &WidgetCx) {
        self.0.access(tree, column(rect, cx.theme), cx);
    }

    fn access_action(
        &mut self,
        request: &nuxut::access::AccessRequest,
        parent: nuxut::access::NodeId,
        rect: Rect,
        cx: &WidgetCx,
    ) -> Option<(bool, Option<Action>)> {
        self.0.access_action(request, parent, column(rect, cx.theme), cx)
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.0.is_animating(now)
    }
}

fn draw_error(ui: &mut Ui, label: &mut Label, at: Point, theme: &Theme) {
    let c = ui.fade(theme.colors.error);
    label.draw(ui.painter, at, c);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn volume_icons_follow_level() {
        assert_eq!(volume_icon(0.5, true), IconName::VolumeMute);
        assert_eq!(volume_icon(0.0, false), IconName::VolumeMute);
        assert_eq!(volume_icon(0.2, false), IconName::VolumeLow);
        assert_eq!(volume_icon(0.5, false), IconName::VolumeMedium);
        assert_eq!(volume_icon(1.4, false), IconName::VolumeHigh);
    }

    #[test]
    fn wifi_icons_cover_range() {
        assert_eq!(wifi_icon(0), IconName::Wifi1);
        assert_eq!(wifi_icon(49), IconName::Wifi2);
        assert_eq!(wifi_icon(74), IconName::Wifi3);
        assert_eq!(wifi_icon(100), IconName::Wifi4);
    }

    fn battery(percentage: f64, state: ChargeState) -> Battery {
        Battery {
            percentage,
            state,
            time_to_empty: None,
            time_to_full: None,
            icon_name: String::new(),
            warning: Default::default(),
        }
    }

    #[test]
    fn battery_icons_round_to_tens() {
        assert_eq!(battery_icon(&battery(0.0, ChargeState::Discharging)), IconName::Battery0);
        assert_eq!(battery_icon(&battery(44.0, ChargeState::Discharging)), IconName::Battery40);
        assert_eq!(battery_icon(&battery(45.0, ChargeState::Discharging)), IconName::Battery50);
        assert_eq!(battery_icon(&battery(100.0, ChargeState::FullyCharged)), IconName::Battery100);
        assert_eq!(battery_icon(&battery(30.0, ChargeState::Charging)), IconName::BatteryCharging);
        assert_eq!(battery_icon(&battery(30.0, ChargeState::Unknown)), IconName::BatteryUnknown);
    }

    #[test]
    fn durations_are_humane() {
        crate::i18n::use_turkish_for_tests();
        assert_eq!(format_duration(Duration::from_secs(20)), "1 dk");
        assert_eq!(format_duration(Duration::from_secs(35 * 60)), "35 dk");
        assert_eq!(format_duration(Duration::from_secs(2 * 3600)), "2 sa");
        assert_eq!(format_duration(Duration::from_secs(2 * 3600 + 5 * 60)), "2 sa 5 dk");
        assert_eq!(format_position(Duration::from_secs(187)), "3:07");
        assert_eq!(format_position(Duration::from_secs(3723)), "1:02:03");
    }

    #[test]
    fn bluetooth_icons_map_known_kinds() {
        assert_eq!(bluetooth_device_icon(Some("audio-headset")), IconName::Headphones);
        assert_eq!(bluetooth_device_icon(Some("input-mouse")), IconName::Mouse);
        assert_eq!(bluetooth_device_icon(Some("phone")), IconName::Bluetooth);
        assert_eq!(bluetooth_device_icon(None), IconName::Bluetooth);
    }
}
