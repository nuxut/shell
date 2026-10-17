use std::time::Duration;

use zbus::zvariant::{OwnedValue, Value};

use super::*;
use nuxut::i18n::{Catalog, LanguagePrefs};
use crate::services::audio::{AudioDevice, AudioState};
use crate::services::bluetooth::{Adapter, BluetoothState, Device};
use nuxut_dbus::PropMap;
use crate::services::network::{AccessPoint, ActiveConnection, NetworkState, Security, WifiDevice};
use crate::services::power::PowerState;
use crate::widgets::WidgetSettings;

fn props(entries: &[(&str, Value<'static>)]) -> PropMap {
    entries.iter().map(|(k, v)| (k.to_string(), OwnedValue::try_from(v.try_clone().unwrap()).unwrap())).collect()
}

/// UPower `State`: 1 şarj, 2 deşarj, 4 dolu.
fn laptop(percentage: f64, state: u32, warning: u32) -> PowerState {
    let mut s = PowerState::default();
    s.upower = true;
    s.apply_device(&props(&[
        ("Type", Value::U32(2)),
        ("IsPresent", Value::Bool(true)),
        ("Percentage", Value::F64(percentage)),
        ("State", Value::U32(state)),
        ("TimeToEmpty", Value::I64(5400)),
        ("TimeToFull", Value::I64(600)),
        ("WarningLevel", Value::U32(warning)),
    ]));
    s
}

fn sink(volume: f32, muted: bool) -> AudioState {
    AudioState {
        available: true,
        sinks: vec![AudioDevice {
            id: 40,
            name: "alsa_output.pci".into(),
            description: "Dahili ses".into(),
            volume,
            muted,
            is_default: true,
            class: Default::default(),
        }],
        ..AudioState::default()
    }
}

fn wifi(strength: u8) -> NetworkState {
    NetworkState {
        available: true,
        state: NmState::ConnectedGlobal,
        connectivity: Connectivity::Full,
        primary_kind: Some(ConnectionKind::Wifi),
        primary_name: Some("Ev".into()),
        wifi_enabled: true,
        wifi_hardware_enabled: true,
        wifi_devices: vec![WifiDevice { path: "/d/1".into(), interface: "wlan0".into(), active_ap: Some("/ap/1".into()) }],
        access_points: vec![AccessPoint {
            path: "/ap/1".into(),
            ssid: "Ev".into(),
            ssid_bytes: b"Ev".to_vec(),
            strength,
            security: Security::default(),
            frequency: 5180,
            active: true,
            saved: true,
        }],
        ..NetworkState::default()
    }
}

fn system() -> SystemModel {
    SystemModel { audio: sink(0.45, false), network: wifi(80), ..SystemModel::default() }
}

fn look(slot: Slot, sys: &SystemModel) -> Option<(IconName, Option<String>, Tone)> {
    view(slot, sys, &StatusSettings::default()).map(|v| (v.look.icon, v.look.value, v.look.tone))
}

#[test]
fn volume_level_and_mute() {
    tr();
    let mut sys = system();
    assert_eq!(look(Slot::Volume, &sys), Some((IconName::VolumeMedium, None, Tone::Normal)));
    let v = view(Slot::Volume, &sys, &StatusSettings { volume_percent: true, ..StatusSettings::default() }).unwrap();
    assert_eq!(v.look.value.as_deref(), Some("%45"));
    assert_eq!(v.tooltip, "Ses %45\nDahili ses");
    sys.audio = sink(0.45, true);
    assert_eq!(look(Slot::Volume, &sys), Some((IconName::VolumeMute, None, Tone::Dim)));
    assert!(view(Slot::Volume, &sys, &StatusSettings::default()).unwrap().tooltip.starts_with("Ses kapalı (%45)"));
    sys.audio.available = false;
    assert_eq!(look(Slot::Volume, &sys), None, "PipeWire yok: gizli");
}

#[test]
fn network_states() {
    tr();
    let mut sys = system();
    assert_eq!(look(Slot::Network, &sys), Some((IconName::Wifi4, None, Tone::Normal)));
    assert!(view(Slot::Network, &sys, &StatusSettings::default()).unwrap().tooltip.starts_with("Wi-Fi: Ev · %80"));
    sys.network = wifi(30);
    assert_eq!(look(Slot::Network, &sys).unwrap().0, IconName::Wifi2);

    // Kablolu birincil.
    sys.network.primary_kind = Some(ConnectionKind::Ethernet);
    sys.network.primary_name = Some("Kablolu bağlantı 1".into());
    sys.network.access_points.clear();
    assert_eq!(look(Slot::Network, &sys), Some((IconName::Ethernet, None, Tone::Normal)));

    // İnternet yok: soluk, ipucunda neden.
    sys.network.connectivity = Connectivity::Limited;
    let v = view(Slot::Network, &sys, &StatusSettings::default()).unwrap();
    assert_eq!(v.look.tone, Tone::Dim);
    assert!(v.tooltip.contains("İnternet erişimi yok"), "{}", v.tooltip);

    // Bağlantı yok, Wi-Fi kapalı.
    sys.network = NetworkState { state: NmState::Disconnected, wifi_enabled: false, ..wifi(0) };
    assert_eq!(look(Slot::Network, &sys), Some((IconName::WifiOff, None, Tone::Dim)));
    // Wi-Fi kartı olmayan masaüstü, kablo takılı değil.
    sys.network = NetworkState { available: true, state: NmState::Disconnected, ..NetworkState::default() };
    assert_eq!(look(Slot::Network, &sys), Some((IconName::Ethernet, None, Tone::Dim)));
    sys.network.available = false;
    assert_eq!(look(Slot::Network, &sys), None);
}

#[test]
fn vpn_primary_shows_underlying_link() {
    tr();
    let mut sys = system();
    sys.network.primary_kind = Some(ConnectionKind::Vpn);
    sys.network.active.push(ActiveConnection {
        path: "/a/2".into(),
        id: "İş".into(),
        kind: ConnectionKind::Vpn,
        state: ActiveState::Activated,
        connection: "/c/2".into(),
    });
    let v = view(Slot::Network, &sys, &StatusSettings::default()).unwrap();
    assert_eq!(v.look.icon, IconName::Wifi4);
    assert!(v.tooltip.ends_with("VPN: İş"), "{}", v.tooltip);
}

#[test]
fn bluetooth_hidden_when_off() {
    tr();
    let mut sys = system();
    assert_eq!(look(Slot::Bluetooth, &sys), None, "BlueZ yok");
    let adapter = Adapter { path: "/org/bluez/hci0".into(), powered: false, ..Adapter::default() };
    sys.bluetooth = BluetoothState { available: true, adapters: vec![adapter], ..BluetoothState::default() };
    assert_eq!(look(Slot::Bluetooth, &sys), None, "kapalı");
    sys.bluetooth.adapters[0].powered = true;
    assert_eq!(look(Slot::Bluetooth, &sys), Some((IconName::Bluetooth, None, Tone::Normal)));
    sys.bluetooth.devices = (0..5)
        .map(|i| Device { name: format!("Kulaklık {i}"), connected: true, battery: (i == 0).then_some(80), ..Device::default() })
        .collect();
    let v = view(Slot::Bluetooth, &sys, &StatusSettings::default()).unwrap();
    assert_eq!(v.look.icon, IconName::BluetoothConnected);
    assert_eq!(v.tooltip, "Bluetooth: 5 cihaz bağlı\nKulaklık 0 · pil %80\nKulaklık 1\nKulaklık 2\nve 2 cihaz daha");
}

#[test]
fn battery_alert_and_charge() {
    tr();
    let mut sys = system();
    assert_eq!(look(Slot::Battery, &sys), None, "pil yok (masaüstü/VM): gizli");
    sys.power = laptop(44.0, 2, 0);
    assert_eq!(look(Slot::Battery, &sys), Some((IconName::Battery40, Some("%44".into()), Tone::Normal)));
    assert_eq!(view(Slot::Battery, &sys, &StatusSettings::default()).unwrap().tooltip, "Pil %44 · 1 sa 30 dk kaldı");
    sys.power = laptop(15.0, 2, 0);
    assert_eq!(look(Slot::Battery, &sys).unwrap().2, Tone::Alert, "≤ %15");
    sys.power = laptop(16.0, 2, 0);
    assert_eq!(look(Slot::Battery, &sys).unwrap().2, Tone::Normal);
    sys.power = laptop(30.0, 2, 4);
    assert_eq!(look(Slot::Battery, &sys).unwrap().2, Tone::Alert, "UPower kritik");
    sys.power = laptop(10.0, 1, 4);
    let v = view(Slot::Battery, &sys, &StatusSettings::default()).unwrap();
    assert_eq!((v.look.icon, v.look.tone), (IconName::BatteryCharging, Tone::Normal), "şarjda uyarı yok");
    assert_eq!(v.tooltip, "Pil %10 · Şarj oluyor · dolmasına 10 dk");
    let off = StatusSettings { battery_percent: false, ..StatusSettings::default() };
    assert_eq!(view(Slot::Battery, &sys, &off).unwrap().look.value, None);
    assert_eq!(format_duration(Duration::from_secs(5400)), "1 sa 30 dk");
}

#[test]
fn settings_hide_indicators() {
    let sys = system();
    let s = StatusSettings { network: false, volume: false, ..StatusSettings::default() };
    assert!(Slot::ALL.iter().all(|&slot| view(slot, &sys, &s).is_none()));
}

#[test]
fn short_names_are_bounded() {
    tr();
    let long = "a".repeat(200);
    assert_eq!(short(&long).chars().count(), MAX_TIP_NAME);
    assert_eq!(short("a\nb\u{7}c"), "abc");
}

fn with_cx<R>(vertical: bool, settings: &WidgetSettings, f: impl FnOnce(&WidgetCx) -> R) -> R {
    let theme = nuxut::Theme::default();
    let cx = WidgetCx { theme: &theme, settings, vertical, item: 18.0, now: Instant::now(), bounds: Rect::ZERO };
    f(&cx)
}

#[test]
fn sync_redraws_only_on_visible_change() {
    tr();
    let settings = WidgetSettings::default();
    with_cx(false, &settings, |cx| {
        let mut w = Status::new(cx);
        let mut model = ShellModel { system: system(), ..ShellModel::default() };
        assert!(w.sync(&model, "M", cx), "ilk veri");
        assert!(!w.sync(&model, "M", cx), "değişmedi");
        // Aynı ikon kademesinde güç değişimi: kare yok, ipucu güncel.
        model.system.network.access_points[0].strength = 78;
        assert!(!w.sync(&model, "M", cx));
        assert!(w.indicators[Slot::Network.index()].tooltip.contains("%78"));
        // Başka servisin olayı (ör. medya): hesap bile yapılmaz.
        model.notifications_unread = 3;
        assert!(!w.sync(&model, "M", cx));
        model.system.audio = sink(0.45, true);
        assert!(w.sync(&model, "M", cx), "sessiz: ikon değişti");
        // Gizli gösterge değeri değişince kare yok.
        model.system.audio = sink(0.47, true);
        assert!(!w.sync(&model, "M", cx), "yüzde gösterilmiyor, ikon aynı");
    });
}

#[test]
fn layout_tooltip_scroll_and_click() {
    tr();
    let settings = WidgetSettings::default();
    for vertical in [false, true] {
        with_cx(vertical, &settings, |cx| {
            let mut w = Status::new(cx);
            let mut model = ShellModel { system: system(), ..ShellModel::default() };
            model.system.power = laptop(50.0, 2, 0);
            w.sync(&model, "M", cx);
            let mut text = TextSystem::new();
            let len = w.measure(&mut text, cx);
            assert!(len > 0.0);
            let rect = if vertical { Rect::new(0.0, 0.0, 34.0, len) } else { Rect::new(0.0, 0.0, len, 34.0) };
            let hits = w.hit_rects(rect, cx);
            assert_eq!(hits.iter().map(|(s, _)| *s).collect::<Vec<_>>(), [Slot::Network, Slot::Volume, Slot::Battery]);
            // Göstergeler aralıksız, sırayla ve hap boyunca.
            for pair in hits.windows(2) {
                let (a, b) = (pair[0].1, pair[1].1);
                if vertical {
                    assert!((a.y + a.height - b.y).abs() < 0.01);
                } else {
                    assert!((a.x + a.width - b.x).abs() < 0.01);
                }
            }
            let at = |r: Rect| r.center();
            let tip = w.tooltip(rect, at(hits[2].1), cx).unwrap();
            assert_eq!((tip.text, tip.anchor), ("Pil %50 · 1 sa 30 dk kaldı", Some(hits[2].1)));
            assert_eq!(w.click(rect, at(hits[0].1), cx), Some(Action::TogglePanel(PanelId::Edge(crate::layout::Edge::Right))));

            // Tekerlek: yalnızca ses göstergesinde.
            w.last_rect = Some(rect);
            let wheel = |p: Point| PointerEvent::new(p, PointerKind::Scroll { delta: Point::new(0.0, -15.0), steps: Point::new(0.0, -1.0) });
            let up = w.overlay_pointer(&wheel(at(hits[1].1)), &mut text, cx);
            assert_eq!(up, Some((false, Some(Action::BarEdge(EdgeCommand::Scroll { action: ScrollAction::Volume, steps: 1 })))));
            assert_eq!(w.overlay_pointer(&wheel(at(hits[2].1)), &mut text, cx), None, "pilde kaydırma bar'a düşer");
            assert_eq!(w.overlay_pointer(&wheel(Point::new(-50.0, -50.0)), &mut text, cx), None);
        });
    }
}

#[test]
fn hidden_when_nothing_to_show() {
    let settings = WidgetSettings::default();
    with_cx(false, &settings, |cx| {
        let mut w = Status::new(cx);
        let mut text = TextSystem::new();
        assert_eq!(w.measure(&mut text, cx), 0.0, "veri yok");
        let model = ShellModel::default();
        w.sync(&model, "M", cx);
        assert_eq!(w.measure(&mut text, cx), 0.0, "servisler yok");
    });
}

/// Kaynak dil (Türkçe) katalogu, sayı biçimi tr_TR ("%45"), süreç geneli kurulur
/// ([`crate::percent::tr_catalog`]; yüzde metni üreten tüm modüller aynısını kurar).
fn tr() {
    crate::percent::tr_catalog();
}

/// İngilizce: yüzde sonda, cihaz sayısında tekil/çoğul (süreç geneli dile dokunmadan).
/// Yüzde değişkenleri oran (`NUMBER(style: "percent")`).
#[test]
fn english_catalog() {
    let en = Catalog::load(&[nuxut::i18n::embed!("shell")], &LanguagePrefs::fixed(&["en"]));
    let fmt = |id: &str, pairs: &[(&str, f64)]| {
        let mut args = nuxut::i18n::Args::new();
        for (k, v) in pairs {
            args.set(*k, *v);
        }
        en.format(id, &args).into_owned()
    };
    assert_eq!(fmt("shell-bar-status-percent", &[("percent", 0.45)]), "45%");
    assert_eq!(fmt("shell-bar-status-bluetooth-connected", &[("count", 1.0)]), "Bluetooth: 1 device connected");
    assert_eq!(fmt("shell-bar-status-bluetooth-connected", &[("count", 3.0)]), "Bluetooth: 3 devices connected");
    assert_eq!(fmt("shell-bar-status-volume-muted", &[("percent", 0.2)]), "Muted (20%)");
}
