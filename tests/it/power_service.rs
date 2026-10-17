//! Güç servisi: sahte UPower + power-profiles-daemon ile özel bus'ta uçtan uca.
//!
//! Kapsam: ilk okuma, özellik değişimi, düşük pil olayı (seviye başına bir kez), sahte
//! (unicast) sinyalin reddi, UPower'ın düşüp geri gelmesi, profil yokken/varken, profil
//! değiştirme, temiz kapanış. Şarj sınırı: destek yok / UPower yok, aç/kapa (yanıttan önce yeni
//! durum), polkit reddi ve `NotSupported`, pil eklenip çıkınca, dışarıdan değişim, UPower'ın
//! yeniden başlaması, çok pilde kısmi hata (karışık durum), bus yokken isteklerin hemen yanıtlanması.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nuxut_shell::services::power::{
    ChargeLimit, PowerError, ChargeState, PowerEvent, PowerOptions, PowerProfile, PowerService, PowerState,
    WarningLevel,
};
use crate::support::{Events, PrivateBus, channel};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const UPOWER: &str = "org.freedesktop.UPower";
const UPOWER_PATH: &str = "/org/freedesktop/UPower";
const DISPLAY: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const BAT0: &str = "/org/freedesktop/UPower/devices/battery_BAT0";
const BAT1: &str = "/org/freedesktop/UPower/devices/battery_BAT1";
const MOUSE: &str = "/org/freedesktop/UPower/devices/mouse_hidpp_battery_0";
const PPD: &str = "org.freedesktop.UPower.PowerProfiles";
const PPD_PATH: &str = "/org/freedesktop/UPower/PowerProfiles";

struct FakeManager {
    on_battery: bool,
    devices: Vec<OwnedObjectPath>,
}

#[zbus::interface(name = "org.freedesktop.UPower")]
impl FakeManager {
    #[zbus(property)]
    fn on_battery(&self) -> bool {
        self.on_battery
    }
    #[zbus(property)]
    fn lid_is_present(&self) -> bool {
        true
    }
    fn enumerate_devices(&self) -> Vec<OwnedObjectPath> {
        self.devices.clone()
    }
    #[zbus(signal)]
    async fn device_added(e: &SignalEmitter<'_>, device: OwnedObjectPath) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn device_removed(e: &SignalEmitter<'_>, device: OwnedObjectPath) -> zbus::Result<()>;
}

/// upowerd 1.91.5'in döndürdüğü hata adları.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.UPower")]
enum UpError {
    #[zbus(error)]
    ZBus(zbus::Error),
    GeneralError(String),
    NotSupported(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Answer {
    Accept,
    /// polkit reddi: upowerd `GeneralError` + "Operation is not allowed.".
    Deny,
    NotSupported,
}

/// Gerçek pil cihazı (şarj eşiği alanlarıyla). `EnableChargeThreshold` kendi
/// `PropertiesChanged`'ini göndermez: servis başarıdan sonra yeniden okumalı.
#[derive(Clone)]
struct FakeBattery {
    supply: bool,
    supported: bool,
    enabled: bool,
    start: u32,
    end: u32,
    answer: Answer,
    calls: Arc<Mutex<Vec<bool>>>,
}

impl FakeBattery {
    fn new(supported: bool, enabled: bool) -> Self {
        Self { supply: true, supported, enabled, start: 75, end: 80, answer: Answer::Accept, calls: Arc::default() }
    }
}

#[zbus::interface(name = "org.freedesktop.UPower.Device")]
impl FakeBattery {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> u32 {
        2
    }
    #[zbus(property)]
    fn power_supply(&self) -> bool {
        self.supply
    }
    #[zbus(property)]
    fn charge_threshold_supported(&self) -> bool {
        self.supported
    }
    #[zbus(property)]
    fn charge_threshold_enabled(&self) -> bool {
        self.enabled
    }
    #[zbus(property)]
    fn charge_start_threshold(&self) -> u32 {
        self.start
    }
    #[zbus(property)]
    fn charge_end_threshold(&self) -> u32 {
        self.end
    }
    fn enable_charge_threshold(&mut self, on: bool) -> Result<(), UpError> {
        self.calls.lock().unwrap().push(on);
        match self.answer {
            Answer::Accept => {
                self.enabled = on;
                Ok(())
            }
            Answer::Deny => Err(UpError::GeneralError("Operation is not allowed.".into())),
            Answer::NotSupported => Err(UpError::NotSupported("setting battery charge thresholds is unsupported".into())),
        }
    }
}

struct FakeDevice {
    percentage: f64,
    state: u32,
    warning: u32,
}

#[zbus::interface(name = "org.freedesktop.UPower.Device")]
impl FakeDevice {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> u32 {
        2
    }
    #[zbus(property)]
    fn is_present(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn percentage(&self) -> f64 {
        self.percentage
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.state
    }
    #[zbus(property)]
    fn time_to_empty(&self) -> i64 {
        3600
    }
    #[zbus(property)]
    fn time_to_full(&self) -> i64 {
        0
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        "battery-level-50-symbolic".into()
    }
    #[zbus(property)]
    fn warning_level(&self) -> u32 {
        self.warning
    }
}

struct FakePpd {
    active: String,
    sets: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.freedesktop.UPower.PowerProfiles")]
impl FakePpd {
    #[zbus(property)]
    fn active_profile(&self) -> String {
        self.active.clone()
    }
    #[zbus(property)]
    fn set_active_profile(&mut self, p: String) -> zbus::fdo::Result<()> {
        if p == "performance" {
            return Err(zbus::fdo::Error::AccessDenied("yok".into()));
        }
        self.sets.lock().unwrap().push(p.clone());
        self.active = p;
        Ok(())
    }
    #[zbus(property)]
    fn profiles(&self) -> Vec<HashMap<String, OwnedValue>> {
        ["power-saver", "balanced", "performance"]
            .iter()
            .map(|p| HashMap::from([("Profile".to_owned(), OwnedValue::try_from(Value::from(*p)).unwrap())]))
            .collect()
    }
    #[zbus(property)]
    fn performance_degraded(&self) -> String {
        String::new()
    }
}

fn serve_upower(bus: &PrivateBus, percentage: f64) -> zbus::blocking::Connection {
    serve_upower_with(bus, percentage, &[])
}

fn object_path(p: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p.to_owned()).unwrap()
}

/// Sahte UPower; `batteries` `EnumerateDevices`'ta listelenir.
fn serve_upower_with(bus: &PrivateBus, percentage: f64, batteries: &[(&str, FakeBattery)]) -> zbus::blocking::Connection {
    let conn = bus.connect();
    let devices = batteries.iter().map(|(p, _)| object_path(p)).collect();
    conn.object_server().at(UPOWER_PATH, FakeManager { on_battery: true, devices }).unwrap();
    conn.object_server().at(DISPLAY, FakeDevice { percentage, state: 2, warning: 1 }).unwrap();
    for (path, b) in batteries {
        conn.object_server().at(*path, b.clone()).unwrap();
    }
    conn.request_name(UPOWER).unwrap();
    conn
}

/// Pili ekler ve `DeviceAdded` yayınlar (UPower'ın sırası: nesne önce, sinyal sonra).
fn add_battery(conn: &zbus::blocking::Connection, path: &str, b: FakeBattery) {
    conn.object_server().at(path, b).unwrap();
    let iface = conn.object_server().interface::<_, FakeManager>(UPOWER_PATH).unwrap();
    iface.get_mut().devices.push(object_path(path));
    zbus::block_on(FakeManager::device_added(iface.signal_emitter(), object_path(path))).unwrap();
}

fn remove_battery(conn: &zbus::blocking::Connection, path: &str) {
    let iface = conn.object_server().interface::<_, FakeManager>(UPOWER_PATH).unwrap();
    iface.get_mut().devices.retain(|p| p.as_str() != path);
    conn.object_server().remove::<FakeBattery, _>(path).unwrap();
    zbus::block_on(FakeManager::device_removed(iface.signal_emitter(), object_path(path))).unwrap();
}

fn battery_mut(conn: &zbus::blocking::Connection, path: &str, f: impl FnOnce(&mut FakeBattery)) {
    let iface = conn.object_server().interface::<_, FakeBattery>(path).unwrap();
    f(&mut iface.get_mut());
}

fn limit(events: &Events<PowerEvent>, what: &str, pred: impl Fn(ChargeLimit) -> bool) -> ChargeLimit {
    state(events, what, |s| pred(s.charge_limit())).charge_limit()
}

/// `set_charge_limit` sonucunu bekler; arada gelen olaylar da döner (sıra denetimi için).
fn charge_result(events: &Events<PowerEvent>, id: u64) -> (Result<(), PowerError>, Vec<PowerEvent>) {
    let mut before = Vec::new();
    loop {
        let e = events.wait_for("şarj sınırı sonucu", |_| true);
        match e {
            PowerEvent::ChargeLimitDone { id: got, result } if got == id => return (result, before),
            other => before.push(other),
        }
    }
}

/// `set_profile` sonucunu bekler; arada gelen olaylar da döner.
fn profile_result(events: &Events<PowerEvent>, id: u64) -> (Result<(), PowerError>, Vec<PowerEvent>) {
    let mut before = Vec::new();
    loop {
        match events.wait_for("profil sonucu", |_| true) {
            PowerEvent::ProfileDone { id: got, result } if got == id => return (result, before),
            other => before.push(other),
        }
    }
}

fn set_device(conn: &zbus::blocking::Connection, percentage: f64, warning: u32) {
    let iface = conn.object_server().interface::<_, FakeDevice>(DISPLAY).unwrap();
    let emitter = iface.signal_emitter().clone();
    let mut d = iface.get_mut();
    d.percentage = percentage;
    d.warning = warning;
    zbus::block_on(async {
        d.percentage_changed(&emitter).await.unwrap();
        d.warning_level_changed(&emitter).await.unwrap();
    });
}

fn start(bus: &PrivateBus) -> (PowerService, Events<PowerEvent>) {
    let (sink, events) = channel();
    let options = PowerOptions {
        bus_address: Some(bus.address.clone()),
        call_timeout: Duration::from_millis(500),
        max_backoff: Duration::from_millis(200),
        ..PowerOptions::default()
    };
    (PowerService::spawn(options, sink).unwrap(), events)
}

fn state(events: &Events<PowerEvent>, what: &str, pred: impl Fn(&PowerState) -> bool) -> PowerState {
    match events.wait_for(what, |e| matches!(e, PowerEvent::Changed(s) if pred(s))) {
        PowerEvent::Changed(s) => s,
        _ => unreachable!(),
    }
}

#[test]
fn battery_changes_and_low_warning() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    let up = serve_upower(&bus, 50.0);
    let (_svc, events) = start(&bus);

    let s = state(&events, "ilk okuma", |s| s.upower && s.battery().is_some());
    let b = s.battery().unwrap();
    assert_eq!(b.percentage, 50.0);
    assert_eq!(b.state, ChargeState::Discharging);
    assert_eq!(b.time_to_empty, Some(Duration::from_secs(3600)));
    assert!(s.on_battery && s.lid_present);
    assert_eq!(s.profiles, None, "ppd yok → kullanılamıyor");

    set_device(&up, 15.0, 3);
    events.wait_for("düşük pil", |e| matches!(e, PowerEvent::BatteryWarning(WarningLevel::Low)));
    set_device(&up, 14.0, 3);
    state(&events, "%14", |s| s.battery().is_some_and(|b| b.percentage == 14.0));
    set_device(&up, 4.0, 4);
    events.wait_for("kritik pil", |e| matches!(e, PowerEvent::BatteryWarning(WarningLevel::Critical)));
    let warnings = events
        .collect(Duration::from_millis(200))
        .into_iter()
        .filter(|e| matches!(e, PowerEvent::BatteryWarning(_)))
        .count();
    assert_eq!(warnings, 0, "aynı seviye tekrar uyarmamalı");
}

#[test]
fn spoofed_unicast_signal_is_ignored() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    let _up = serve_upower(&bus, 80.0);
    let (_svc, events) = start(&bus);
    state(&events, "ilk okuma", |s| s.battery().is_some());

    // Saldırgan: bus'taki her benzersiz ada doğrudan sahte PropertiesChanged gönderir.
    let attacker = bus.connect();
    let names: Vec<String> = attacker
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "ListNames",
            &(),
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    let changed = HashMap::from([("Percentage", Value::F64(1.0))]);
    for name in names.iter().filter(|n| n.starts_with(':')) {
        let _ = attacker.emit_signal(
            Some(name.as_str()),
            DISPLAY,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &("org.freedesktop.UPower.Device", &changed, Vec::<String>::new()),
        );
    }
    let spoofed = events
        .collect(Duration::from_millis(300))
        .into_iter()
        .any(|e| matches!(e, PowerEvent::Changed(s) if s.battery().is_some_and(|b| b.percentage == 1.0)));
    assert!(!spoofed, "sahte sinyal kabul edildi");
}

#[test]
fn upower_restart_and_profiles() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    let up = serve_upower(&bus, 60.0);
    let (svc, events) = start(&bus);
    state(&events, "ilk okuma", |s| s.battery().is_some());

    // UPower düşer → kullanılamaz; başka bir süreç olarak geri gelir → yeniden okunur.
    drop(up);
    state(&events, "UPower yok", |s| !s.upower && s.battery().is_none());
    let _up = serve_upower(&bus, 33.0);
    state(&events, "UPower geri", |s| s.battery().is_some_and(|b| b.percentage == 33.0));

    // Profil servisi yokken istek reddedilir.
    let id = svc.set_profile(PowerProfile::PowerSaver).unwrap();
    assert_eq!(profile_result(&events, id).0, Err(PowerError::Unavailable));

    let sets = Arc::new(Mutex::new(Vec::new()));
    let ppd = bus.connect();
    ppd.object_server().at(PPD_PATH, FakePpd { active: "balanced".into(), sets: sets.clone() }).unwrap();
    ppd.request_name(PPD).unwrap();
    let s = state(&events, "profiller", |s| s.profiles.is_some());
    let p = s.profiles.unwrap();
    assert_eq!(p.active, Some(PowerProfile::Balanced));
    assert_eq!(p.available.len(), 3);

    // Yeni profil sonuçtan ÖNCE yayınlanır (sahte servis PropertiesChanged göndermiyor).
    let id = svc.set_profile(PowerProfile::PowerSaver).unwrap();
    let (result, before) = profile_result(&events, id);
    assert_eq!(result, Ok(()));
    assert_eq!(sets.lock().unwrap().as_slice(), ["power-saver"]);
    assert!(
        before.iter().any(|e| matches!(e, PowerEvent::Changed(s)
            if s.profiles.as_ref().is_some_and(|p| p.active == Some(PowerProfile::PowerSaver)))),
        "{before:?}"
    );

    // polkit reddi türüyle.
    let id = svc.set_profile(PowerProfile::Performance).unwrap();
    assert_eq!(profile_result(&events, id).0, Err(PowerError::Denied));
    drop(svc);
}

#[test]
fn charge_limit_unsupported_without_battery_or_upower() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    let (svc, events) = start(&bus);
    // UPower hiç yok.
    let id = svc.set_charge_limit(true).unwrap();
    assert_eq!(charge_result(&events, id).0, Err(PowerError::Unavailable));

    // UPower var ama sistem pili yok (masaüstü, VM) ya da yalnız fare pili var.
    let mut mouse = FakeBattery::new(true, false);
    mouse.supply = false;
    let up = serve_upower_with(&bus, 0.0, &[(MOUSE, mouse)]);
    let s = state(&events, "UPower", |s| s.upower);
    assert_eq!(s.charge_limit(), ChargeLimit::Unsupported);
    let id = svc.set_charge_limit(true).unwrap();
    assert_eq!(charge_result(&events, id).0, Err(PowerError::Unsupported));

    // Desteklemeyen pil.
    add_battery(&up, BAT0, FakeBattery::new(false, false));
    let id = svc.set_charge_limit(false).unwrap();
    assert_eq!(charge_result(&events, id).0, Err(PowerError::Unsupported));
}

#[test]
fn charge_limit_toggle() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    let bat = FakeBattery::new(true, false);
    let calls = bat.calls.clone();
    let up = serve_upower_with(&bus, 70.0, &[(BAT0, bat)]);
    let (svc, events) = start(&bus);
    let l = limit(&events, "pil okundu", ChargeLimit::is_supported);
    assert_eq!(l, ChargeLimit::Supported { enabled: false, mixed: false, start: Some(75), end: Some(80) });

    // Aç: yeni durum yanıttan ÖNCE yayınlanır (sahte servis PropertiesChanged göndermiyor).
    let id = svc.set_charge_limit(true).unwrap();
    let (result, before) = charge_result(&events, id);
    assert_eq!(result, Ok(()));
    assert!(
        before.iter().any(|e| matches!(e, PowerEvent::Changed(s) if s.charge_limit().is_enabled())),
        "yanıttan önce açık durum yayınlanmalı: {before:?}"
    );
    assert_eq!(calls.lock().unwrap().as_slice(), [true]);

    // Zaten açık: UPower'a çağrı gitmez.
    let id = svc.set_charge_limit(true).unwrap();
    assert_eq!(charge_result(&events, id).0, Ok(()));
    assert_eq!(calls.lock().unwrap().len(), 1);

    // Dışarıdan (ör. `upower` aracıyla) kapatıldı: PropertiesChanged ile izlenir.
    let iface = up.object_server().interface::<_, FakeBattery>(BAT0).unwrap();
    iface.get_mut().enabled = false;
    zbus::block_on(iface.get().charge_threshold_enabled_changed(iface.signal_emitter())).unwrap();
    limit(&events, "dışarıdan kapandı", |l| l.is_supported() && !l.is_enabled());

    let id = svc.set_charge_limit(false).unwrap();
    assert_eq!(charge_result(&events, id).0, Ok(()));
    assert_eq!(calls.lock().unwrap().len(), 1, "zaten kapalı");
    drop(svc);
}

#[test]
fn charge_limit_denied_and_not_supported() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    let mut bat = FakeBattery::new(true, false);
    bat.answer = Answer::Deny;
    let up = serve_upower_with(&bus, 70.0, &[(BAT0, bat)]);
    let (svc, events) = start(&bus);
    limit(&events, "pil okundu", ChargeLimit::is_supported);

    let id = svc.set_charge_limit(true).unwrap();
    let (result, before) = charge_result(&events, id);
    assert_eq!(result, Err(PowerError::Denied));
    assert!(!before.iter().any(|e| matches!(e, PowerEvent::Changed(s) if s.charge_limit().is_enabled())));

    battery_mut(&up, BAT0, |b| b.answer = Answer::NotSupported);
    let id = svc.set_charge_limit(true).unwrap();
    assert_eq!(charge_result(&events, id).0, Err(PowerError::Unsupported));

    // İki istek: sonuçlar kendi kimlikleriyle.
    battery_mut(&up, BAT0, |b| b.answer = Answer::Accept);
    let a = svc.set_charge_limit(true).unwrap();
    let b = svc.set_charge_limit(false).unwrap();
    assert_ne!(a, b);
    assert_eq!(charge_result(&events, a).0, Ok(()));
    // Sırayla: "kapat", "aç"ın sonucundan sonra değerlendirilir; son istek kazanır.
    let (result, before) = charge_result(&events, b);
    assert_eq!(result, Ok(()));
    let last = before.iter().rev().find_map(|e| match e {
        PowerEvent::Changed(s) => Some(s.charge_limit()),
        _ => None,
    });
    assert_eq!(last.map(ChargeLimit::is_enabled), Some(false), "{before:?}");
    let calls = up.object_server().interface::<_, FakeBattery>(BAT0).unwrap().get().calls.lock().unwrap().clone();
    assert_eq!(calls, [true, true, true, false], "ret, desteklenmiyor, aç, kapat");
}

#[test]
fn batteries_added_removed_and_upower_restart() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    let up = serve_upower_with(&bus, 70.0, &[]);
    let (svc, events) = start(&bus);
    state(&events, "UPower", |s| s.upower);

    // Pil takıldı (ör. ikinci, değiştirilebilir pil).
    add_battery(&up, BAT0, FakeBattery::new(true, true));
    limit(&events, "BAT0 eklendi", |l| l.is_enabled());
    // İkinci pil kapalı: karışık → anahtar kapalı görünür; açmak yalnız onu çağırır.
    let bat1 = FakeBattery::new(true, false);
    let calls1 = bat1.calls.clone();
    add_battery(&up, BAT1, bat1);
    let l = limit(&events, "BAT1 eklendi", |l| matches!(l, ChargeLimit::Supported { mixed: true, .. }));
    assert!(!l.is_enabled());
    let id = svc.set_charge_limit(true).unwrap();
    let (result, before) = charge_result(&events, id);
    assert_eq!(result, Ok(()));
    assert_eq!(calls1.lock().unwrap().as_slice(), [true]);
    assert!(
        before.iter().any(|e| matches!(e, PowerEvent::Changed(s)
            if s.charge_limit() == ChargeLimit::Supported { enabled: true, mixed: false, start: Some(75), end: Some(80) })),
        "{before:?}"
    );

    remove_battery(&up, BAT0);
    remove_battery(&up, BAT1);
    limit(&events, "piller çıktı", |l| l == ChargeLimit::Unsupported);

    // UPower yeniden başlar; açık eşikli pil yeniden okunur.
    drop(up);
    state(&events, "UPower yok", |s| !s.upower);
    let _up = serve_upower_with(&bus, 50.0, &[(BAT0, FakeBattery::new(true, true))]);
    limit(&events, "UPower geri", |l| l.is_enabled());
}

#[test]
fn requests_answered_while_bus_unavailable() {
    // Bus hiç yok: servis yeniden bağlanmak için uyurken gelen istekler hemen yanıtlanır
    // (`msg power charge-limit on` asılı kalmaz). Uzun geri çekilme: yanıt beklemeye bağlı olmasın.
    let (sink, events) = channel();
    let options = PowerOptions {
        bus_address: Some("unix:path=/nonexistent/nuxut-test-bus".into()),
        call_timeout: Duration::from_millis(500),
        max_backoff: Duration::from_secs(60),
        ..PowerOptions::default()
    };
    let svc = PowerService::spawn(options, sink).unwrap();
    // Birkaç bağlantı denemesi geçsin; geri çekilme 1 s'yi aşsın.
    std::thread::sleep(Duration::from_millis(1_000));

    let t = std::time::Instant::now();
    let id = svc.set_charge_limit(true).unwrap();
    assert_eq!(charge_result(&events, id).0, Err(PowerError::Unavailable));
    assert!(svc.set_profile(PowerProfile::PowerSaver).is_some());
    events.wait_for("profil hatası", |e| matches!(e, PowerEvent::ProfileDone { result: Err(_), .. }));
    assert!(t.elapsed() < Duration::from_millis(500), "yanıt uykuyu beklememeli: {:?}", t.elapsed());
    drop(svc);
}

#[test]
fn charge_limit_partial_failure_shows_mixed() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    // `charge_targets` yol sırasında: BAT0 kabul eder (açılır), BAT1 reddeder.
    let bat0 = FakeBattery::new(true, false);
    let calls0 = bat0.calls.clone();
    let mut bat1 = FakeBattery::new(true, false);
    bat1.answer = Answer::Deny;
    let calls1 = bat1.calls.clone();
    let _up = serve_upower_with(&bus, 70.0, &[(BAT0, bat0), (BAT1, bat1)]);
    let (svc, events) = start(&bus);
    limit(&events, "iki pil okundu", |l| l == ChargeLimit::Supported { enabled: false, mixed: false, start: Some(75), end: Some(80) });

    let id = svc.set_charge_limit(true).unwrap();
    let (result, before) = charge_result(&events, id);
    assert_eq!(result, Err(PowerError::Denied));
    assert_eq!(calls0.lock().unwrap().as_slice(), [true]);
    assert_eq!(calls1.lock().unwrap().as_slice(), [true]);
    // Yarım değişiklik sonuçtan önce yayınlanır: BAT0 açık, BAT1 kapalı → karışık.
    let last = before.iter().rev().find_map(|e| match e {
        PowerEvent::Changed(s) => Some(s.charge_limit()),
        _ => None,
    });
    assert!(matches!(last, Some(ChargeLimit::Supported { mixed: true, .. })), "{before:?}");
    drop(svc);
}
