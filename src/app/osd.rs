//! Ses/parlaklık komutları (`nuxut-shell msg volume|mic|brightness …`) ve OSD.
//!
//! Komut `app/system.rs`'in `apply_*` yardımcılarıyla servise gider; OSD komuttan değil servis
//! olayından gelir (değer gerçekten değişince — tuş, `msg` ya da başka bir uygulama). Ne zaman
//! görüneceğine [`OsdTracker`] karar verir (saf durum): bir kaynağın (varsayılan çıkış,
//! varsayılan giriş, ekran) önceki değeri biliniyor ve yeni değer farklıysa. Başlangıç okuması,
//! PipeWire'a yeniden bağlanma (`[audio] max_volume` değişince servis yeniden başlar), varsayılan
//! cihaz değişimi, ekran listesinin yeniden taranması ve aynı değerin tekrarı OSD değildir.
//! Monitör [`osd_monitor`] ile seçilir.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nuxut::i18n::t;
use nuxut::ui::{Icon, IconName};
use nuxut::{Context, TimerId};

use super::Shell;
use crate::ipc::{self, Command, Toggle};
use crate::services::audio::AudioState;
use crate::services::brightness::{Display, DisplayKind};
use crate::services::magnifier::MagnifierView;
use crate::widgets::quick_magnifier::{factor_text, slider_value};
use crate::system::{Level, Mute};

/// `--output`'la istenen OSD monitörü bu kadar süre geçerli (servis olayı bundan önce gelir).
const REQUEST_WINDOW: Duration = Duration::from_secs(2);

impl Shell {
    /// `Command::Volume|VolumeMute|MicMute|Brightness` kolları: servise iletir; `--output`
    /// verildiyse OSD'nin monitörünü hatırlar.
    pub(super) fn osd_command(&mut self, cx: &mut Context<Self>, command: Command) -> Result<String, String> {
        let (kind, output) = match &command {
            Command::Volume { output, .. } | Command::VolumeMute { output, .. } => (OsdKind::Volume, output),
            Command::MicMute { output, .. } => (OsdKind::Mic, output),
            Command::Brightness { output, .. } => (OsdKind::Brightness, output),
            _ => return Err("ses/parlaklık komutu değil".into()),
        };
        let output = output.clone();
        // Parlaklık hedefi: `--output`, yoksa odaklı monitör (o monitörde DDC/CI ekranı yoksa
        // dahili panel — `SystemModel::brightness_display`).
        let idx = self.target_screen(output.as_deref())?;
        let monitor = self.screens[idx].name.clone();
        match command {
            Command::Volume { level, .. } => self.apply_volume(cx, system_level(level))?,
            Command::VolumeMute { action, .. } => self.apply_volume_mute(cx, system_mute(action))?,
            Command::MicMute { action, .. } => self.apply_mic_mute(cx, system_mute(action))?,
            Command::Brightness { level, .. } => self.apply_brightness(cx, Some(&monitor), system_level(level))?,
            _ => unreachable!("yukarıda süzüldü"),
        }
        if output.is_some() {
            self.osd_request = Some((kind, monitor, Instant::now()));
        }
        Ok("ok".into())
    }

    /// `AudioEvent::State` (model güncellenmeden önce; izleyici kendi önceki değerini tutar).
    pub(super) fn osd_audio(&mut self, cx: &mut Context<Self>, state: &AudioState) {
        for show in self.osd.audio(state) {
            self.show_osd(cx, show);
        }
    }

    /// Büyüteç görünümü geldi (model güncellenmeden önce; izleyici önceki çarpanı tutar).
    pub(super) fn osd_magnifier(&mut self, cx: &mut Context<Self>, view: &MagnifierView) {
        if let Some(show) = self.osd.magnifier(view) {
            self.show_osd(cx, show);
        }
    }

    /// `BrightnessEvent::Displays`: yalnızca kaydedilir.
    pub(super) fn osd_displays(&mut self, displays: &[Display]) {
        self.osd.displays(displays);
    }

    /// `BrightnessEvent::Changed`.
    pub(super) fn osd_brightness(&mut self, cx: &mut Context<Self>, display: &Display) {
        if let Some(show) = self.osd.brightness(display) {
            self.show_osd(cx, show);
        }
    }

    /// OSD'yi tek monitörde gösterir (diğerlerinde kapatır); gizlenme timer'ı her değişimde
    /// yeniden kurulur.
    fn show_osd(&mut self, cx: &mut Context<Self>, show: OsdShow) {
        let now = Instant::now();
        let names: Vec<&str> = self.screens.iter().map(|s| s.name.as_str()).collect();
        let requested = self
            .osd_request
            .as_ref()
            .filter(|(kind, _, at)| *kind == show.kind && now.saturating_duration_since(*at) < REQUEST_WINDOW)
            .map(|(_, name, _)| name.as_str());
        let focused = self.model.hypr.focused_monitor().map(|m| m.name.as_str());
        let Some(idx) = osd_monitor(&names, requested, show.device.as_ref(), focused).or((!names.is_empty()).then_some(0))
        else {
            return;
        };
        // Ses `[audio] max_volume`'e kadar (%100 üstü taşma); mikrofon ve parlaklık %100.
        let max = if show.kind == OsdKind::Volume { self.model.system.max_volume } else { 1.0 };
        let theme = cx.theme().clone();
        let announce = show.announcement();
        for (i, s) in self.screens.iter_mut().enumerate() {
            if i == idx {
                s.osd_access = announce.clone();
                cx.a11y_invalidate(s.main);
                s.show_osd(Icon::named(show.icon()), show.value, max, show.dimmed(), show.text.clone(), &self.model.config, &theme, now);
                cx.request_frame(s.main);
            } else if s.hide_osd(now) {
                cx.request_frame(s.main);
            }
        }
        if let Some(t) = self.osd_timer.take() {
            cx.cancel_timer(t);
        }
        self.osd_timer = Some(cx.set_timer(Duration::from_millis(u64::from(self.model.config.osd.timeout_ms))));
    }

    /// OSD gizlenme timer'ıysa her ekranda kapatır ve `true` döner.
    pub(super) fn osd_timer_fired(&mut self, cx: &mut Context<Self>, id: TimerId) -> bool {
        if self.osd_timer != Some(id) {
            return false;
        }
        self.osd_timer = None;
        let now = Instant::now();
        for s in &mut self.screens {
            if s.hide_osd(now) {
                cx.request_frame(s.main);
            }
        }
        true
    }
}

fn system_level(level: ipc::Level) -> Level {
    match level {
        ipc::Level::Up(n) => Level::Step(i16::from(n)),
        ipc::Level::Down(n) => Level::Step(-i16::from(n)),
        ipc::Level::Set(n) => Level::Set(u16::from(n)),
    }
}

fn system_mute(action: Toggle) -> Mute {
    match action {
        Toggle::Toggle => Mute::Toggle,
        Toggle::On => Mute::On,
        Toggle::Off => Mute::Off,
    }
}

/// OSD türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsdKind {
    Volume,
    Mic,
    Brightness,
    /// Büyüteç çarpanı: ray 1..=`[magnifier] max`, etiket "2,0×" (yüzde değil).
    Magnifier,
}

/// Gösterilecek OSD.
#[derive(Debug, Clone, PartialEq)]
pub struct OsdShow {
    pub kind: OsdKind,
    /// Ses: algısal (1.0 = %100, üstü yükseltme); parlaklık ve büyüteç: 0..=1.
    pub value: f32,
    pub muted: bool,
    /// Değişen cihazın bağlı olduğu monitör (parlaklık); ses için `None`.
    pub device: Option<DeviceMonitor>,
    /// Yüzde yerine gösterilecek değer metni (büyüteç "2,0×"; framework `Osd::set_value_text`).
    pub text: Option<String>,
}

impl OsdShow {
    pub fn icon(&self) -> IconName {
        let percent = (self.value.clamp(0.0, 1.0) * 100.0).round() as u8;
        match self.kind {
            OsdKind::Volume => IconName::volume(percent, self.muted),
            OsdKind::Mic if self.muted => IconName::MicOff,
            OsdKind::Mic => IconName::Mic,
            OsdKind::Brightness => IconName::brightness(percent),
            OsdKind::Magnifier => IconName::Search,
        }
    }

    /// Ekran okuyucunun duyuracağı metin (canlı bölge): "Ses %64", "Mikrofon kapalı",
    /// "Büyüteç 2,0×".
    pub fn announcement(&self) -> String {
        let value = self.text.clone().unwrap_or_else(|| crate::percent::text(f64::from(self.value) * 100.0));
        match self.kind {
            OsdKind::Volume if self.muted => t!("shell-a11y-osd-volume-muted"),
            OsdKind::Mic if self.muted => t!("shell-a11y-osd-mic-muted"),
            OsdKind::Volume => t!("shell-a11y-osd-volume", value = value),
            OsdKind::Mic => t!("shell-a11y-osd-mic", value = value),
            OsdKind::Brightness => t!("shell-a11y-osd-brightness", value = value),
            OsdKind::Magnifier => t!("shell-a11y-osd-magnifier", value = value),
        }
    }

    /// İçerik (ikon, ray, yüzde) soluk mu: sessizdeki ses/mikrofon. Seviye yine gösterilir
    /// (sessizi kaldırınca dönülecek değer).
    pub fn dimmed(&self) -> bool {
        self.muted && matches!(self.kind, OsdKind::Volume | OsdKind::Mic)
    }
}

/// Parlaklığı değişen ekranın monitörü.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceMonitor {
    /// Harici ekranın Wayland çıktı adı.
    Output(String),
    /// Dahili panel: adı `eDP`/`LVDS`/`DSI` ile başlayan monitör.
    Internal,
}

/// Dahili panel bağlantısı mı (`eDP-1`, `LVDS-1`, `DSI-1`).
fn is_internal_output(name: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| name.starts_with(p))
}

/// OSD'nin monitörü (`names` içindeki sıra): (1) komutun `--output`'u (açık istek), (2) değişen
/// cihazın bağlı olduğu monitör, (3) odaklı monitör. Eşleşmeyen ad atlanır.
pub fn osd_monitor(
    names: &[&str],
    requested: Option<&str>,
    device: Option<&DeviceMonitor>,
    focused: Option<&str>,
) -> Option<usize> {
    let by_name = |name: &str| names.iter().position(|n| *n == name);
    let device = || match device? {
        DeviceMonitor::Output(o) => by_name(o),
        DeviceMonitor::Internal => names.iter().position(|n| is_internal_output(n)),
    };
    requested.and_then(by_name).or_else(device).or_else(|| focused.and_then(by_name))
}

/// Seviye farkı bunun altındaysa aynı değer sayılır (PipeWire kübik dönüşüm yuvarlaması).
const EPSILON: f32 = 0.0005;

#[derive(Debug, Clone, Copy, PartialEq)]
struct AudioLevel {
    /// Cihaz değişti mi diye (varsayılan cihaz değişimi OSD değildir).
    device: u32,
    volume: f32,
    muted: bool,
}

/// Kaynak başına son bilinen değer.
#[derive(Debug, Default)]
pub struct OsdTracker {
    output: Option<AudioLevel>,
    input: Option<AudioLevel>,
    /// Son bilinen büyüteç çarpanı (Hyprland erişilemiyorsa `None`).
    magnifier: Option<f64>,
    /// `Display::id` → yüzde.
    displays: HashMap<String, f64>,
}

impl OsdTracker {
    /// Ses durumu anlık görüntüsü (her `AudioEvent::State`). Değişen varsayılan cihazlar için OSD.
    pub fn audio(&mut self, state: &AudioState) -> Vec<OsdShow> {
        if !state.available {
            // Yeniden bağlanınca ilk görüntü yeni başlangıç okumasıdır.
            self.output = None;
            self.input = None;
            return Vec::new();
        }
        let mut shows = Vec::new();
        let sides = [
            (OsdKind::Volume, &mut self.output, state.default_sink()),
            (OsdKind::Mic, &mut self.input, state.default_source()),
        ];
        for (kind, slot, device) in sides {
            let new = device.map(|d| AudioLevel { device: d.id, volume: d.volume, muted: d.muted });
            if let (Some(old), Some(new)) = (*slot, new)
                && old.device == new.device
                && ((old.volume - new.volume).abs() > EPSILON || old.muted != new.muted)
            {
                shows.push(OsdShow { kind, value: new.volume, muted: new.muted, device: None, text: None });
            }
            *slot = new;
        }
        shows
    }

    /// Ekran listesi (başlangıç / yeniden tarama): yalnızca kaydedilir, OSD yok. Listede
    /// olmayan ekranlar unutulur.
    pub fn displays(&mut self, displays: &[Display]) {
        self.displays = displays.iter().map(|d| (d.id.clone(), d.percent)).collect();
    }

    /// Tek ekranın parlaklığı değişti. Önceki değeri biliniyor ve farklıysa OSD.
    pub fn brightness(&mut self, display: &Display) -> Option<OsdShow> {
        let old = self.displays.insert(display.id.clone(), display.percent)?;
        ((old - display.percent).abs() > f64::from(EPSILON) * 100.0).then(|| OsdShow {
            kind: OsdKind::Brightness,
            value: (display.percent / 100.0).clamp(0.0, 1.0) as f32,
            muted: false,
            device: match display.kind {
                DisplayKind::External => display.output.clone().map(DeviceMonitor::Output),
                DisplayKind::Internal => Some(DeviceMonitor::Internal),
            },
            text: None,
        })
    }

    /// Büyüteç görünümü (her `on_magnifier`). Önceki çarpan biliniyor (Hyprland erişilebilir)
    /// ve farklıysa OSD: kısayol, `msg`, kart ya da kullanıcının kendi bind'i. Erişim kaybı
    /// ve ilk okuma OSD değildir.
    pub fn magnifier(&mut self, view: &MagnifierView) -> Option<OsdShow> {
        let new = view.available.then_some(view.factor);
        let old = std::mem::replace(&mut self.magnifier, new);
        let (old, new) = (old?, new?);
        ((old - new).abs() > 1e-3).then(|| OsdShow {
            kind: OsdKind::Magnifier,
            value: slider_value(new, view.max),
            muted: false,
            device: None,
            text: Some(factor_text(new)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::audio::AudioDevice;

    fn device(id: u32, volume: f32, muted: bool) -> AudioDevice {
        AudioDevice { id, name: String::new(), description: String::new(), volume, muted, is_default: true, class: Default::default() }
    }

    fn state(sink: Option<AudioDevice>, source: Option<AudioDevice>) -> AudioState {
        AudioState { available: true, sinks: sink.into_iter().collect(), sources: source.into_iter().collect(), ..AudioState::default() }
    }

    fn display(id: &str, kind: DisplayKind, output: Option<&str>, percent: f64) -> Display {
        Display { id: id.into(), kind, output: output.map(Into::into), percent }
    }

    #[test]
    fn audio_shows_only_changes_of_known_values() {
        let mut t = OsdTracker::default();
        // Başlangıç okuması.
        assert!(t.audio(&state(Some(device(1, 0.5, false)), Some(device(2, 0.8, false)))).is_empty());
        // Aynı değer tekrarı (yeniden yükleme, ilgisiz akış değişimi).
        assert!(t.audio(&state(Some(device(1, 0.5, false)), Some(device(2, 0.8, false)))).is_empty());
        let shows = t.audio(&state(Some(device(1, 0.55, false)), Some(device(2, 0.8, false))));
        assert_eq!(shows, vec![OsdShow { kind: OsdKind::Volume, value: 0.55, muted: false, device: None, text: None }]);
        let shows = t.audio(&state(Some(device(1, 0.55, false)), Some(device(2, 0.8, true))));
        assert_eq!(shows, vec![OsdShow { kind: OsdKind::Mic, value: 0.8, muted: true, device: None, text: None }]);
        // Yuvarlama gürültüsü değişim değil.
        assert!(t.audio(&state(Some(device(1, 0.5501, false)), Some(device(2, 0.8, true)))).is_empty());
    }

    #[test]
    fn audio_device_switch_and_reconnect_are_silent() {
        let mut t = OsdTracker::default();
        t.audio(&state(Some(device(1, 0.5, false)), None));
        // Varsayılan cihaz değişti (farklı seviye): OSD yok.
        assert!(t.audio(&state(Some(device(7, 0.9, false)), None)).is_empty());
        assert_eq!(t.audio(&state(Some(device(7, 0.8, false)), None)).len(), 1);
        // Varsayılan yok olup geri gelince ilk görüntü sessiz.
        assert!(t.audio(&state(None, None)).is_empty());
        assert!(t.audio(&state(Some(device(7, 0.3, false)), None)).is_empty());
        // PipeWire düştü ve geri geldi (ya da max_volume değişti, servis yeniden başladı).
        assert!(t.audio(&AudioState::default()).is_empty());
        assert!(t.audio(&state(Some(device(7, 0.6, true)), None)).is_empty());
        assert_eq!(t.audio(&state(Some(device(7, 0.6, false)), None)).len(), 1);
    }

    #[test]
    fn brightness_needs_a_known_previous_value() {
        let mut t = OsdTracker::default();
        let panel = |p| display("backlight:intel", DisplayKind::Internal, None, p);
        let monitor = |p| display("ddc:DP-1", DisplayKind::External, Some("DP-1"), p);
        // Liste gelmeden değişim: önceki değer yok.
        assert_eq!(t.brightness(&panel(40.0)), None);
        t.displays(&[panel(40.0), monitor(70.0)]);
        assert_eq!(t.brightness(&panel(40.0)), None, "aynı değer");
        let s = t.brightness(&panel(45.0)).expect("değişim");
        assert_eq!((s.kind, s.value, s.device), (OsdKind::Brightness, 0.45, Some(DeviceMonitor::Internal)));
        let s = t.brightness(&monitor(60.0)).expect("değişim");
        assert_eq!(s.device, Some(DeviceMonitor::Output("DP-1".into())));
        // Yeniden tarama: sessiz, listede olmayan unutulur.
        t.displays(&[panel(80.0)]);
        assert_eq!(t.brightness(&monitor(50.0)), None);
        assert!(t.brightness(&panel(85.0)).is_some());
    }

    #[test]
    fn icons_and_command_mapping() {
        let show = |kind, value, muted| OsdShow { kind, value, muted, device: None, text: None };
        assert_eq!(show(OsdKind::Volume, 1.3, false).icon(), IconName::VolumeHigh);
        assert_eq!(show(OsdKind::Volume, 0.5, true).icon(), IconName::VolumeMute);
        assert_eq!(show(OsdKind::Mic, 0.8, true).icon(), IconName::MicOff);
        assert_eq!(show(OsdKind::Mic, 0.8, false).icon(), IconName::Mic);
        assert_eq!(show(OsdKind::Brightness, 1.0, false).icon(), IconName::BrightnessHigh);
        assert!(show(OsdKind::Volume, 0.5, true).dimmed() && show(OsdKind::Mic, 0.5, true).dimmed());
        assert!(!show(OsdKind::Volume, 0.5, false).dimmed() && !show(OsdKind::Brightness, 0.5, true).dimmed());
        assert_eq!(system_level(ipc::Level::Up(5)), Level::Step(5));
        assert_eq!(system_level(ipc::Level::Down(100)), Level::Step(-100));
        assert_eq!(system_level(ipc::Level::Set(150)), Level::Set(150));
        assert_eq!(system_mute(Toggle::Toggle), Mute::Toggle);
        assert_eq!(system_mute(Toggle::On), Mute::On);
        assert_eq!(system_mute(Toggle::Off), Mute::Off);
    }

    #[test]
    fn monitor_order_request_then_device_then_focused() {
        let names = ["eDP-1", "DP-1", "HDMI-A-1"];
        let dp = DeviceMonitor::Output("DP-1".into());
        let internal = DeviceMonitor::Internal;
        // Açık istek her şeyi ezer.
        assert_eq!(osd_monitor(&names, Some("HDMI-A-1"), Some(&dp), Some("eDP-1")), Some(2));
        assert_eq!(osd_monitor(&names, Some("HDMI-A-1"), Some(&internal), None), Some(2));
        // İstek yok: cihazın monitörü.
        assert_eq!(osd_monitor(&names, None, Some(&dp), Some("HDMI-A-1")), Some(1));
        assert_eq!(osd_monitor(&names, None, Some(&internal), Some("HDMI-A-1")), Some(0));
        // Ses (cihaz monitörü yok): odaklı.
        assert_eq!(osd_monitor(&names, None, None, Some("HDMI-A-1")), Some(2));
        // Eşleşmeyen adlar atlanır.
        assert_eq!(osd_monitor(&names, Some("DP-9"), Some(&DeviceMonitor::Output("DP-8".into())), Some("DP-1")), Some(1));
        assert_eq!(osd_monitor(&["DP-1"], None, Some(&internal), None), None);
        assert_eq!(osd_monitor(&[], Some("DP-1"), None, Some("DP-1")), None);
        assert!(is_internal_output("eDP-1") && is_internal_output("LVDS-1") && !is_internal_output("DP-1"));
    }
}

#[cfg(test)]
mod magnifier_tests {
    use super::*;

    fn view(available: bool, factor: f64) -> MagnifierView {
        MagnifierView { available, factor, max: 5.0 }
    }

    /// Büyüteç OSD'si: ilk okuma ve erişim kaybı sessiz; çarpan değişimi rayı 1..max'a göre
    /// doldurur, etiket yüzde değil "2,0×".
    #[test]
    fn magnifier_osd_on_factor_change_with_factor_text() {
        crate::i18n::use_turkish_for_tests();
        let mut t = OsdTracker::default();
        assert_eq!(t.magnifier(&view(true, 1.0)), None, "ilk okuma");
        assert_eq!(t.magnifier(&view(true, 1.0)), None, "aynı değer");
        let s = t.magnifier(&view(true, 2.0)).expect("yakınlaştı");
        assert_eq!((s.kind, s.value, s.text.as_deref()), (OsdKind::Magnifier, 0.25, Some("2,0×")));
        assert_eq!(s.icon(), IconName::Search);
        assert!(!s.dimmed());
        assert_eq!(t.magnifier(&view(false, 1.0)), None, "Hyprland erişilemez");
        assert_eq!(t.magnifier(&view(true, 3.0)), None, "yeniden bağlanınca ilk okuma");
        assert_eq!(t.magnifier(&view(true, 1.0)).map(|s| s.value), Some(0.0), "kapandı: ray boş");
    }
}
