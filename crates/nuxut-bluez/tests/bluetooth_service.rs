//! Bluetooth servisi: sahte BlueZ (`tests/fake_bluez`, `examples/fake_bluez` ile ortak) ile
//! özel bus'ta uçtan uca. Bu VM'de gerçek adaptör yok.
//!
//! Kapsam: ilk okuma, aç/kapa, zaman aşımlı tarama ve yeni cihaz, eşleştirme ajanı (onay →
//! güven + bağlan), ajan zaman aşımı, BlueZ dışından ajan çağrısının reddi, unut, BlueZ'in
//! yeniden başlaması (ajan yeniden kaydı). Aşama 2: ajansız istemci, adaptör adı / görünürlük
//! (süreli) / eşleştirilebilirlik, cihaz engelle / ad, `Pair` hata türleri.

// Ortak yardımcılar (özel dbus-daemon, olay bekleme) kabuğun tests/support'undan paylaşılır.
#[path = "../../../tests/support/mod.rs"]
mod support;
mod fake_bluez;

use std::time::Duration;

use nuxut_bluez::{
    AGENT_PATH, BluetoothEvent, BluetoothOptions, BluetoothService, BluetoothState, BluezError, PairingKind, PowerState,
    AgentStatus, Rfkill, Unavailable,
    PairingReply,
};
use crate::fake_bluez::*;
use crate::support::{Events, PrivateBus, channel, wait_until};
use zbus::zvariant::ObjectPath;

fn start(bus: &PrivateBus) -> (BluetoothService, Events<BluetoothEvent>) {
    let (sink, events) = channel();
    let options = BluetoothOptions {
        bus_address: Some(bus.address.clone()),
        call_timeout: Duration::from_millis(500),
        action_timeout: Duration::from_secs(3),
        discovery_timeout: Duration::from_millis(300),
        agent_timeout: Duration::from_millis(400),
        throttle: Duration::from_millis(20),
        max_backoff: Duration::from_millis(200),
        ..BluetoothOptions::with_agent()
    };
    (BluetoothService::spawn(options, sink).unwrap(), events)
}

fn state(events: &Events<BluetoothEvent>, what: &str, pred: impl Fn(&BluetoothState) -> bool) -> BluetoothState {
    match events.wait_for(what, |e| matches!(e, BluetoothEvent::State(s) if pred(s))) {
        BluetoothEvent::State(s) => s,
        _ => unreachable!(),
    }
}

#[test]
fn adapter_discovery_pairing_and_restart() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeBluez::start(bus.connect(), &log, false);
    let (svc, events) = start(&bus);

    let s = state(&events, "ilk okuma", |s| s.available && s.devices.len() == 1);
    assert_eq!(s.adapter().unwrap().name, "nuxut-vm");
    assert!(!s.powered());
    let d = &s.devices[0];
    assert_eq!((d.name.as_str(), d.icon.as_deref(), d.paired), ("Kulaklık", Some("audio-headset"), true));
    wait_until("ajan kaydı", || log.has("RegisterAgent KeyboardDisplay") && log.has("RequestDefaultAgent"));

    svc.set_powered(true);
    state(&events, "açıldı", |s| s.powered());

    // Tarama: yeni cihaz görünür, süre dolunca kendiliğinden durur.
    svc.start_discovery();
    wait_until("tarama başladı", || log.has("StartDiscovery"));
    let new = fake.add_device("dev_2", "Fare", false);
    state(&events, "yeni cihaz", |s| s.devices.iter().any(|d| d.path == new));
    wait_until("tarama durdu", || log.has("StopDiscovery"));

    // Eşleştirme: ajan onay sorar → kabul → güven + bağlan.
    svc.pair(&new);
    let req = match events.wait_for("onay isteği", |e| matches!(e, BluetoothEvent::Pairing(_))) {
        BluetoothEvent::Pairing(r) => r,
        _ => unreachable!(),
    };
    assert_eq!(req.kind, PairingKind::Confirm { passkey: 123456 });
    assert_eq!(req.name, "Fare");
    svc.reply(req.id, PairingReply::Accept);
    wait_until("eşleşti, güvenildi, bağlandı", || {
        log.has(&format!("Paired {new}")) && log.has(&format!("Trusted {new} true")) && log.has(&format!("Connect {new}"))
    });

    // Cevapsız istek zaman aşımına uğrar: diyalog kapanır, eşleştirme başarısız.
    svc.pair(&new);
    let req = match events.wait_for("ikinci onay", |e| matches!(e, BluetoothEvent::Pairing(_))) {
        BluetoothEvent::Pairing(r) => r,
        _ => unreachable!(),
    };
    events.wait_for("zaman aşımı", |e| matches!(e, BluetoothEvent::PairingDone { id } if *id == req.id));
    let e = events.wait_for("eşleştirme hatası", |e| matches!(e, BluetoothEvent::Failed { action: "pair", .. }));
    assert!(matches!(e, BluetoothEvent::Failed { error: BluezError::AuthenticationCanceled, .. }), "{e:?}");

    svc.forget(&new);
    wait_until("unut", || log.has(&format!("RemoveDevice {new}")));
    svc.connect("/org/bluez/hci0/dev_yok");
    events.wait_for("bilinmeyen cihaz", |e| matches!(e, BluetoothEvent::Failed { action: "device", .. }));

    // BlueZ yeniden başlar: durum temizlenir, yeniden okunur, ajan yeniden kaydedilir.
    drop(fake);
    state(&events, "BlueZ yok", |s| !s.available && s.devices.is_empty());
    let _fake = FakeBluez::start(bus.connect(), &log, false);
    state(&events, "BlueZ geri", |s| s.available && s.devices.len() == 1);
    wait_until("yeniden kayıt", || log.count("RegisterAgent KeyboardDisplay") == 2);
    drop(svc);
}

#[test]
fn agent_rejects_foreign_callers() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeBluez::start(bus.connect(), &log, false);
    let (_svc, events) = start(&bus);
    state(&events, "ilk okuma", |s| s.available);
    wait_until("ajan kaydı", || fake.agent.lock().unwrap().is_some());
    let (service_name, _) = fake.agent.lock().unwrap().clone().unwrap();

    let attacker = bus.connect();
    let device = ObjectPath::try_from("/org/bluez/hci0/dev_1").unwrap();
    let r = attacker.call_method(
        Some(service_name.as_str()),
        AGENT_PATH,
        Some("org.bluez.Agent1"),
        "RequestConfirmation",
        &(device, 1u32),
    );
    let err = r.expect_err("yabancı çağrı kabul edildi").to_string();
    assert!(err.contains("Rejected"), "{err}");
    let leaked = events.collect(Duration::from_millis(200)).into_iter().any(|e| matches!(e, BluetoothEvent::Pairing(_)));
    assert!(!leaked, "yabancı istek UI'a ulaştı");
}

/// Ajansız istemci (Ayarlar): ajan nesnesi yok, `RegisterAgent`/`RequestDefaultAgent` yok;
/// eylemler çalışır.
#[test]
fn no_agent_client() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeBluez::start(bus.connect(), &log, false);
    let (sink, events) = channel();
    let options = BluetoothOptions {
        bus_address: Some(bus.address.clone()),
        throttle: Duration::from_millis(20),
        ..BluetoothOptions::default()
    };
    let svc = BluetoothService::spawn(options, sink).unwrap();
    state(&events, "ilk okuma", |s| s.available && s.devices.len() == 1);
    svc.set_powered(true);
    state(&events, "açıldı", |s| s.powered());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(log.count("RegisterAgent KeyboardDisplay"), 0);
    assert_eq!(log.count("RequestDefaultAgent"), 0);
    assert!(fake.agent.lock().unwrap().is_none());
    drop(svc);
}

fn start_client(bus: &PrivateBus) -> (BluetoothService, Events<BluetoothEvent>) {
    let (sink, events) = channel();
    let options = BluetoothOptions {
        bus_address: Some(bus.address.clone()),
        call_timeout: Duration::from_millis(500),
        action_timeout: Duration::from_secs(3),
        throttle: Duration::from_millis(20),
        max_backoff: Duration::from_millis(200),
        ..BluetoothOptions::default()
    };
    (BluetoothService::spawn(options, sink).unwrap(), events)
}

fn failed(events: &Events<BluetoothEvent>, action: &'static str) -> BluezError {
    match events.wait_for(action, |e| matches!(e, BluetoothEvent::Failed { action: a, .. } if *a == action)) {
        BluetoothEvent::Failed { error, .. } => error,
        _ => unreachable!(),
    }
}

#[test]
fn adapter_and_device_settings() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let _fake = FakeBluez::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    let s = state(&events, "ilk okuma", |s| s.available && s.devices.len() == 5);
    let a = s.adapter().unwrap();
    assert_eq!((a.name.as_str(), a.pairable, a.discoverable, a.discoverable_timeout), ("nuxut-vm", true, false, 180));

    svc.set_adapter_alias(HCI0, "Masaüstü");
    state(&events, "ad", |s| s.adapter().is_some_and(|a| a.name == "Masaüstü"));
    assert!(log.has(&format!("Alias {HCI0} Masaüstü")));

    // Görünürlük: süre önce, sonra aç; kapatırken süre yazılmaz.
    svc.set_discoverable(HCI0, true, 120);
    state(&events, "görünür", |s| s.adapter().is_some_and(|a| a.discoverable && a.discoverable_timeout == 120));
    svc.set_discoverable(HCI0, false, 0);
    state(&events, "görünmez", |s| s.adapter().is_some_and(|a| !a.discoverable));
    assert_eq!(log.count("DiscoverableTimeout 120"), 1);
    assert_eq!(log.count("DiscoverableTimeout 0"), 0);
    assert!(log.index("Discoverable true").is_some());
    assert!(log.index("DiscoverableTimeout 120") < log.index("Discoverable true"), "süre önce yazılır");
    svc.set_pairable(HCI0, false);
    state(&events, "eşleştirilemez", |s| s.adapter().is_some_and(|a| !a.pairable));

    svc.set_adapter_alias("/org/bluez/hci9", "x");
    assert_eq!(failed(&events, "adapter_alias"), BluezError::NoAdapter);
    svc.set_adapter_alias(HCI0, "kötü\nad");
    assert_eq!(failed(&events, "adapter_alias"), BluezError::InvalidValue);

    // Cihaz: engelle, ad.
    let klavye = format!("{HCI0}/dev_2");
    svc.set_blocked(&klavye, true);
    state(&events, "engelli", |s| s.devices.iter().any(|d| d.path == klavye && d.blocked));
    svc.set_device_alias(&klavye, "Ofis klavyesi");
    state(&events, "cihaz adı", |s| s.devices.iter().any(|d| d.path == klavye && d.name == "Ofis klavyesi"));
    svc.set_blocked(&format!("{HCI0}/dev_yok"), true);
    assert_eq!(failed(&events, "device"), BluezError::UnknownDevice);
    assert_eq!(log.count("RegisterAgent KeyboardDisplay"), 0);
    drop(svc);
}

#[test]
fn pair_error_types() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeBluez::start(bus.connect(), &log, true);
    let cases = [
        (fake.add_failing_device("dev_r", "R", PairError::Rejected), BluezError::AuthenticationRejected),
        (fake.add_failing_device("dev_c", "C", PairError::Canceled), BluezError::AuthenticationCanceled),
        (fake.add_failing_device("dev_t", "T", PairError::Timeout), BluezError::AuthenticationTimeout),
        (
            fake.add_failing_device("dev_x", "X", PairError::ConnectionAttemptFailed),
            BluezError::ConnectionAttemptFailed,
        ),
        (format!("{HCI0}/dev_3"), BluezError::AuthenticationFailed),
    ];
    let (svc, events) = start_client(&bus);
    state(&events, "ilk okuma", |s| s.devices.len() == 5 + 4);
    for (device, expected) in cases {
        svc.pair(&device);
        let got = match events.wait_for("pair hatası", |e| matches!(e, BluetoothEvent::Failed { action: "pair", .. })) {
            BluetoothEvent::Failed { device: d, error, .. } => {
                assert_eq!(d.as_deref(), Some(device.as_str()));
                error
            }
            _ => unreachable!(),
        };
        assert_eq!(got, expected, "{device}");
    }
    // Ajan yok: "just works" eşleşme → güven + bağlan.
    let klavye = format!("{HCI0}/dev_2");
    svc.pair(&klavye);
    wait_until("eşleşti", || log.has(&format!("Paired {klavye}")) && log.has(&format!("Connect {klavye}")));
    drop(svc);
}

/// t115: ham alanlar (ad/takma ad, sınıf, görünüm, ikon), pil değişimi, bağlan →
/// `Connected` + `ServicesResolved`.
#[test]
fn device_raw_fields_battery_and_connect() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeBluez::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    let s = state(&events, "ilk okuma", |s| s.devices.len() == 5 && s.devices.iter().any(|d| d.battery.is_some()));
    let dev = |s: &BluetoothState, id: &str| s.devices.iter().find(|d| d.path.ends_with(id)).cloned().unwrap();
    let headset = dev(&s, "dev_1");
    assert_eq!((headset.battery, headset.remote_name.as_deref()), (Some(80), Some("Kulaklık")));
    let kb = dev(&s, "dev_2");
    assert_eq!((kb.icon.as_deref(), kb.class, kb.appearance), (Some("input-keyboard"), Some(0x002540), Some(0)));
    let watch = dev(&s, "dev_4");
    assert_eq!((watch.icon, watch.appearance), (None, Some(0x00c2)));
    let anon = dev(&s, "dev_5");
    assert_eq!(anon.remote_name, None, "adsız cihaz gizlenebilir");
    assert_eq!(anon.alias, "12-34-56-78-9A-BC");

    fake.set_battery(&headset.path, 55);
    state(&events, "pil", |s| s.devices.iter().any(|d| d.path == headset.path && d.battery == Some(55)));

    svc.connect(&kb.path);
    state(&events, "bağlı + servisler", |s| s.devices.iter().any(|d| d.path == kb.path && d.connected && d.services_resolved));
    svc.disconnect(&kb.path);
    state(&events, "koptu", |s| s.devices.iter().any(|d| d.path == kb.path && !d.connected && !d.services_resolved));
    drop(svc);
}

/// t115: tarama filtresi her başlatmada; kendi tarama ayrımı.
#[test]
fn discovery_filter_and_ownership() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeBluez::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    state(&events, "ilk okuma", |s| s.available && !s.own_discovery);

    // Başka istemcinin taraması: discovering açık, own_discovery kapalı.
    fake.set_external_discovering(true);
    state(&events, "yabancı tarama", |s| s.adapter().is_some_and(|a| a.discovering) && !s.own_discovery);
    fake.set_external_discovering(false);
    state(&events, "yabancı bitti", |s| s.adapter().is_some_and(|a| !a.discovering));

    svc.start_discovery();
    state(&events, "kendi taramamız", |s| s.own_discovery && s.adapter().is_some_and(|a| a.discovering));
    assert!(log.index("SetDiscoveryFilter auto false") < log.index("StartDiscovery"), "filtre önce");
    assert!(log.index("SetDiscoveryFilter auto false").is_some());
    svc.stop_discovery();
    state(&events, "durdu", |s| !s.own_discovery && s.adapter().is_some_and(|a| !a.discovering));
    drop(svc);
}

/// t115: rfkill — `PowerState=off-blocked`, sysfs'ten yumuşak/sert, `Powered=true` →
/// `RfkillBlocked { hard }`.
#[test]
fn rfkill_blocked_power() {
    let Some(bus) = PrivateBus::start() else { return };
    let dir = std::env::temp_dir().join(format!("nuxut-bt-rfkill-{}", std::process::id()));
    let entry = dir.join("rfkill3");
    std::fs::create_dir_all(&entry).unwrap();
    std::fs::write(entry.join("type"), "bluetooth\n").unwrap();
    std::fs::write(entry.join("soft"), "1\n").unwrap();
    std::fs::write(entry.join("hard"), "0\n").unwrap();
    let log = Log::default();
    let fake = FakeBluez::start(bus.connect(), &log, true);
    let (sink, events) = channel();
    let options = BluetoothOptions {
        bus_address: Some(bus.address.clone()),
        throttle: Duration::from_millis(20),
        rfkill_dir: Some(dir.clone()),
        ..BluetoothOptions::default()
    };
    let svc = BluetoothService::spawn(options, sink).unwrap();
    state(&events, "ilk okuma", |s| s.powered() && s.rfkill.is_none());

    fake.set_rfkill(true);
    let s = state(&events, "engelli", |s| s.adapter().is_some_and(|a| a.power_state == PowerState::Blocked));
    assert_eq!(s.rfkill, Some(Rfkill { soft: true, hard: false }));
    assert!(!s.powered());
    svc.set_powered(true);
    assert_eq!(failed(&events, "power"), BluezError::RfkillBlocked { hard: Some(false) });

    fake.set_rfkill(false);
    let s = state(&events, "engel kalktı", |s| s.adapter().is_some_and(|a| a.power_state == PowerState::Off));
    assert_eq!(s.rfkill, None);
    svc.set_powered(true);
    state(&events, "açıldı", |s| s.powered());
    drop(svc);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// t115: `CancelPairing` süren eşleştirmeyi `AuthenticationCanceled` ile bitirir.
#[test]
fn cancel_pairing() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let _fake = FakeBluez::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    state(&events, "ilk okuma", |s| s.devices.len() == 5);
    let watch = format!("{HCI0}/dev_4");
    svc.pair(&watch);
    wait_until("eşleştirme sürüyor", || log.has(&format!("PairWaiting {watch}")));
    svc.cancel_pairing(&watch);
    assert_eq!(failed(&events, "pair"), BluezError::AuthenticationCanceled);
    assert!(log.has(&format!("CancelPairing {watch}")));
    svc.cancel_pairing(&format!("{HCI0}/dev_yok"));
    assert_eq!(failed(&events, "device"), BluezError::UnknownDevice);
    drop(svc);
}

/// Kullanılamama nedeni (NM ile aynı): bus'a ulaşılamıyor (bir kez) / BlueZ çalışmıyor / gelince yok.
#[test]
fn unavailable_reasons() {
    let (sink, events) = channel();
    let options = BluetoothOptions {
        bus_address: Some("unix:path=/nonexistent/nuxut-test-bus".into()),
        max_backoff: Duration::from_millis(50),
        ..BluetoothOptions::default()
    };
    let svc = BluetoothService::spawn(options, sink).unwrap();
    let states: Vec<BluetoothState> = events
        .collect(Duration::from_millis(600))
        .into_iter()
        .filter_map(|e| match e {
            BluetoothEvent::State(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(states.len(), 1, "yeniden denemelerde aynı durum tekrar yayınlanmaz");
    assert_eq!((states[0].available, states[0].unavailable), (false, Some(Unavailable::BusUnreachable)));
    drop(svc);

    let Some(bus) = PrivateBus::start() else { return };
    let (svc, events) = start_client(&bus);
    state(&events, "BlueZ yok", |s| !s.available && s.unavailable == Some(Unavailable::ServiceNotRunning));
    let log = Log::default();
    let _fake = FakeBluez::start(bus.connect(), &log, false);
    state(&events, "BlueZ geldi", |s| s.available && s.unavailable.is_none());
    drop(svc);
}

/// Review notu (d): eski bir tarama hatası, sonra başlatılan yeni taramanın `own_discovery`
/// bayrağını indirmez (kuşak sayacı).
#[test]
fn stale_discovery_failure_keeps_new_scan() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeBluez::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    state(&events, "ilk okuma", |s| s.available);
    fake.fail_next_start(Duration::from_millis(400));
    svc.start_discovery();
    wait_until("ilk başlatma askıda", || log.has("StartDiscoveryFailing"));
    svc.start_discovery();
    state(&events, "yeni tarama", |s| s.own_discovery && s.adapter().is_some_and(|a| a.discovering));
    assert_eq!(failed(&events, "discovery"), BluezError::InProgress);
    assert!(log.has("StartDiscoveryFailed"));
    // Hatadan sonra durum hâlâ "bizim taramamız sürüyor".
    let late: Vec<BluetoothState> = events
        .collect(Duration::from_millis(200))
        .into_iter()
        .filter_map(|e| match e {
            BluetoothEvent::State(s) => Some(s),
            _ => None,
        })
        .collect();
    assert!(late.iter().all(|s| s.own_discovery), "eski hata bayrağı indirdi: {late:?}");
    svc.stop_discovery();
    state(&events, "durdu", |s| !s.own_discovery);
    drop(svc);
}

/// e2e (6) + (8): eşleştirme başarı olayı istek kimlikli; `Paired=true` durumu sonuçtan önce;
/// RSSI; "Saat" giyilebilir sınıf; `RemoveDevice` cihazı siler; hata da istek kimlikli.
#[test]
fn pair_success_event_and_fake_details() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let _fake = FakeBluez::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    let s = state(&events, "ilk okuma", |s| s.devices.len() == 5);
    let dev = |s: &BluetoothState, id: &str| s.devices.iter().find(|d| d.path.ends_with(id)).cloned().unwrap();
    assert_eq!(dev(&s, "dev_2").rssi, Some(-48));
    assert_eq!(dev(&s, "dev_4").class, Some(0x000704), "kol saati");
    let kb = dev(&s, "dev_2").path;

    let r = svc.pair(&kb).unwrap();
    let mut seen = Vec::new();
    let done = events.wait_for("pair Done", |e| {
        let hit = matches!(e, BluetoothEvent::Done { request, .. } if *request == r);
        if !hit {
            seen.push(e.clone());
        }
        hit
    });
    assert!(matches!(&done, BluetoothEvent::Done { action: "pair", device: Some(d), .. } if *d == kb), "{done:?}");
    assert!(
        seen.iter().any(|e| matches!(e, BluetoothEvent::State(s) if s.devices.iter().any(|d| d.path == kb && d.paired))),
        "Paired durumu sonuçtan önce"
    );
    wait_until("güven + bağlan", || log.has(&format!("Trusted {kb} true")) && log.has(&format!("Connect {kb}")));

    let bad = dev(&s, "dev_3").path;
    let r = svc.pair(&bad).unwrap();
    match events.wait_for("pair hatası", |e| matches!(e, BluetoothEvent::Failed { request: Some(x), .. } if *x == r)) {
        BluetoothEvent::Failed { error, .. } => assert_eq!(error, BluezError::AuthenticationFailed),
        _ => unreachable!(),
    }

    let anon = dev(&s, "dev_5").path;
    let r = svc.forget(&anon).unwrap();
    events.wait_for("unut Done", |e| matches!(e, BluetoothEvent::Done { request, action: "forget", .. } if *request == r));
    state(&events, "silindi", |s| s.devices.iter().all(|d| d.path != anon));
    drop(svc);
}

/// e2e (8): sahte BlueZ sahneleri.
#[test]
fn fake_scenes() {
    for (scene, check) in [
        (Scene::NoAdapter, (|s: &BluetoothState| s.adapters.is_empty() && s.devices.is_empty()) as fn(&BluetoothState) -> bool),
        (Scene::Off, |s| !s.powered() && s.devices.len() == 5),
        (Scene::Rfkill, |s| s.adapter().is_some_and(|a| a.power_state == PowerState::Blocked)),
        (Scene::Empty, |s| s.powered() && s.devices.is_empty()),
        (Scene::Demo, |s| s.powered() && s.devices.len() == 5),
    ] {
        let Some(bus) = PrivateBus::start() else { return };
        let log = Log::default();
        let _fake = FakeBluez::start_scene(bus.connect(), &log, scene);
        let (_svc, events) = start_client(&bus);
        state(&events, &format!("{scene:?}"), |s| s.available && check(s));
    }
    assert_eq!(Scene::parse("rfkill"), Some(Scene::Rfkill));
    assert_eq!(Scene::parse("bilinmeyen"), None);
}

/// Ajan durumu (`BluetoothState::agent`; kabuğun `msg bluetooth agent-status`'u): ajanlı
/// istemcide kayıtlı + varsayılan, ajansızda ikisi de `false`; BlueZ yeniden başlayınca yeniden.
#[test]
fn agent_status_in_state() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeBluez::start(bus.connect(), &log, false);
    let (with, with_events) = start(&bus);
    state(&with_events, "ajan kayıtlı", |s| s.agent == AgentStatus { registered: true, default: true });
    let (without, without_events) = start_client(&bus);
    let s = state(&without_events, "ajansız", |s| s.available);
    assert_eq!(s.agent, AgentStatus::default());

    drop(fake);
    state(&with_events, "BlueZ yok", |s| !s.available && s.agent == AgentStatus::default());
    let _fake = FakeBluez::start(bus.connect(), &log, false);
    state(&with_events, "yeniden kayıt", |s| s.available && s.agent.registered && s.agent.default);
    drop((with, without));
}
