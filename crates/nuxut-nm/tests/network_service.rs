//! Ağ servisi: sahte NetworkManager (`tests/fake_nm`, `examples/fake_nm` ile ortak) ile özel
//! bus'ta uçtan uca.
//!
//! Kapsam: ilk okuma ve türetilmiş durum, tarama isteği, Wi-Fi kapatma, yeni ağa şifreyle
//! bağlanma (AddAndActivate), kayıtlı ağa bağlanırken gizli bilgi ajanı (UI'a sorma ve önceden
//! verilmiş şifre), NM dışından ajan çağrısının reddi, yeni kayıtlı bağlantının görünmesi,
//! unutma, NM'nin yeniden başlaması. Aşama 2: gizli ağ, `Update` yamaları (yalnız ilgili alan,
//! gizli bilgiye dokunmadan), WireGuard içe aktarma + silme, cihaz ayrıntısı, ajansız
//! istemcide etkinleşme hatası (NO_SECRETS).

// Ortak yardımcılar (özel dbus-daemon, olay bekleme) kabuğun tests/support'undan paylaşılır.
#[path = "../../../tests/support/mod.rs"]
mod support;
mod fake_nm;

use std::collections::HashMap;
use std::time::Duration;

use nuxut_nm::{
    AGENT_ID, ActivationFailure, ConnectionKind, DeviceKind, Metered, NetworkEvent, NetworkOptions, NetworkService, RequestId,
    NetworkState, NmError, SECRET_AGENT_PATH, Secret, SecretKind, Security, Unavailable, WireGuardErrorKind,
};
use crate::fake_nm::*;
use crate::support::{Events, PrivateBus, channel, wait_until};
use zbus::zvariant::{ObjectPath, Value};

fn start(bus: &PrivateBus) -> (NetworkService, Events<NetworkEvent>) {
    let (sink, events) = channel();
    let options = NetworkOptions {
        bus_address: Some(bus.address.clone()),
        call_timeout: Duration::from_secs(2),
        secret_timeout: Duration::from_secs(2),
        throttle: Duration::from_millis(20),
        max_backoff: Duration::from_millis(200),
        ..NetworkOptions::with_agent(AGENT_ID)
    };
    (NetworkService::spawn(options, sink).unwrap(), events)
}

fn state(events: &Events<NetworkEvent>, what: &str, pred: impl Fn(&NetworkState) -> bool) -> NetworkState {
    match events.wait_for(what, |e| matches!(e, NetworkEvent::State(s) if pred(s))) {
        NetworkEvent::State(s) => s,
        _ => unreachable!(),
    }
}

#[test]
fn wifi_flow_and_secret_agent() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeNm::start(bus.connect(), &log, false);
    let (svc, events) = start(&bus);

    let s = state(&events, "ilk okuma", |s| s.available && s.access_points.len() == 2 && !s.saved.is_empty());
    assert!(s.state.is_connected() && s.wifi_enabled);
    assert_eq!(s.primary_kind, Some(ConnectionKind::Wifi));
    assert_eq!(s.primary_name.as_deref(), Some("Ev"));
    let home = &s.access_points[0];
    assert_eq!((home.ssid.as_str(), home.active, home.saved, home.security), ("Ev", true, true, Security::WpaPsk));
    assert!(!s.access_points[1].saved);
    assert_eq!(s.saved[0].active.as_deref(), Some(ACTIVE));
    wait_until("ajan kaydı", || log.has("Register org.nuxut.shell"));

    svc.scan();
    wait_until("tarama", || log.has("RequestScan"));
    svc.set_wifi_enabled(false);
    wait_until("wifi kapat", || log.has("WirelessEnabled false"));
    state(&events, "wifi kapalı", |s| !s.wifi_enabled);

    // Yeni ağ, şifreyle.
    svc.connect_access_point(AP_CAFE, Some(Secret::new("12345678")));
    wait_until("AddAndActivate", || log.has(&format!("AddAndActivate Kafe wpa-psk 12345678 {DEV} {AP_CAFE}")));
    // Geçersiz şifre servis içinde reddedilir.
    svc.connect_access_point(AP_CAFE, Some(Secret::new("kısa")));
    events.wait_for("geçersiz şifre", |e| matches!(e, NetworkEvent::Failed { action: "connect", .. }));

    // Kayıtlı ağ: NM şifre sorar → UI.
    svc.connect_access_point(AP_HOME, None);
    let req = match events.wait_for("şifre isteği", |e| matches!(e, NetworkEvent::SecretRequest(_))) {
        NetworkEvent::SecretRequest(r) => r,
        _ => unreachable!(),
    };
    assert_eq!((req.connection.as_str(), req.ssid.as_deref(), req.kind), ("Ev", Some("Ev"), SecretKind::WifiPassword));
    svc.provide_secret(req.id, Some(Secret::new("parola123")));
    wait_until("ajan cevabı", || log.has("secret parola123"));
    events.wait_for("istek kapandı", |e| matches!(e, NetworkEvent::SecretDone { id } if *id == req.id));

    // Önceden verilen şifre: UI'a sorulmadan kullanılır.
    svc.connect_access_point(AP_HOME, Some(Secret::new("onceden12")));
    wait_until("önceden şifre", || log.has("secret onceden12"));
    let asked = events.collect(Duration::from_millis(100)).into_iter().any(|e| matches!(e, NetworkEvent::SecretRequest(_)));
    assert!(!asked);

    // İptal: istek kapanır, NM hata alır.
    svc.connect_access_point(AP_HOME, None);
    let req = match events.wait_for("şifre isteği 2", |e| matches!(e, NetworkEvent::SecretRequest(_))) {
        NetworkEvent::SecretRequest(r) => r,
        _ => unreachable!(),
    };
    svc.provide_secret(req.id, None);
    events.wait_for("bağlanma hatası", |e| matches!(e, NetworkEvent::Failed { action: "connect", .. }));

    // Yeni kayıtlı VPN görünür.
    fake.add_saved("/org/freedesktop/NetworkManager/Settings/5", plain_settings("İş", "wireguard"));
    let s = state(&events, "vpn", |s| s.vpns().count() == 1);
    assert_eq!(s.vpns().next().unwrap().id, "İş");

    svc.forget(SAVED_HOME);
    wait_until("unut", || log.has(&format!("Delete {SAVED_HOME}")));
    svc.deactivate(ACTIVE);
    wait_until("kopar", || log.has(&format!("Deactivate {ACTIVE}")));
    svc.deactivate("/org/freedesktop/NetworkManager/ActiveConnection/77");
    events.wait_for("bilinmeyen etkin", |e| matches!(e, NetworkEvent::Failed { action: "deactivate", .. }));

    // NM yeniden başlar: ajan yeniden kaydedilir.
    drop(fake);
    state(&events, "NM yok", |s| !s.available);
    let _fake = FakeNm::start(bus.connect(), &log, false);
    state(&events, "NM geri", |s| s.available && s.access_points.len() == 2);
    wait_until("yeniden kayıt", || log.count("Register ") == 2);
    drop(svc);
}

#[test]
fn secret_agent_rejects_foreign_callers() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeNm::start(bus.connect(), &log, false);
    let (_svc, events) = start(&bus);
    state(&events, "ilk okuma", |s| s.available);
    wait_until("ajan kaydı", || fake.agent().is_some());
    let service = fake.agent().unwrap();

    let attacker = bus.connect();
    let settings: HashMap<&str, HashMap<&str, Value<'_>>> =
        HashMap::from([("802-11-wireless-security", HashMap::from([("key-mgmt", Value::from("wpa-psk"))]))]);
    let err = attacker
        .call_method(
            Some(service.as_str()),
            SECRET_AGENT_PATH,
            Some("org.freedesktop.NetworkManager.SecretAgent"),
            "GetSecrets",
            &(settings, ObjectPath::try_from(SAVED_HOME).unwrap(), "802-11-wireless-security", Vec::<String>::new(), 1u32),
        )
        .expect_err("yabancı çağrı kabul edildi")
        .to_string();
    assert!(err.contains("PermissionDenied"), "{err}");
    let leaked =
        events.collect(Duration::from_millis(200)).into_iter().any(|e| matches!(e, NetworkEvent::SecretRequest(_)));
    assert!(!leaked);
}

#[test]
fn agent_id_taken_by_other_process() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeNm::start(bus.connect(), &log, false);
    // Başka bir süreç (ör. ikinci shell örneği) aynı kimliği tutuyor.
    let holder = bus.connect();
    holder
        .call_method(
            Some(NM),
            format!("{NM_PATH}/AgentManager").as_str(),
            Some("org.freedesktop.NetworkManager.AgentManager"),
            "Register",
            &(AGENT_ID,),
        )
        .unwrap();
    let holder_name = holder.unique_name().unwrap().to_string();

    let (svc, events) = start(&bus);
    state(&events, "ilk okuma", |s| s.available && s.access_points.len() == 2);
    wait_until("ret", || log.has(&format!("RegisterDenied {AGENT_ID}")));
    // Ret servisi bozmaz; ret sonrası boşta yeniden deneme yok.
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(log.count("RegisterDenied "), 1);
    svc.set_wifi_enabled(false);
    wait_until("wifi kapat", || log.has("WirelessEnabled false"));
    assert_eq!(log.count("RegisterDenied "), 1, "Wi-Fi aç/kapa yeniden kayıt denememeli");

    // Kimliği tutan süreç kapanır: panel kullanılınca (tarama) kayıt yeniden denenir.
    drop(holder);
    std::thread::sleep(Duration::from_millis(100));
    svc.scan();
    wait_until("yeniden kayıt", || log.count("Register ") == 2);
    let agent = fake.agent().unwrap();
    assert_ne!(agent, holder_name);

    // Kayıtlıyken tekrar kayıt yok (idempotent).
    svc.scan();
    svc.connect_access_point(AP_CAFE, Some(Secret::new("12345678")));
    wait_until("AddAndActivate", || log.count("AddAndActivate ") == 1);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!((log.count("Register "), log.count("RegisterDenied ")), (2, 1));
    drop(svc);
}

#[test]
/// Varsayılan seçenekler ajansız (Ayarlar unutsa bile ajan olamaz).
fn no_agent_by_default() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let _fake = FakeNm::start(bus.connect(), &log, false);
    let (sink, events) = channel();
    let options = NetworkOptions {
        bus_address: Some(bus.address.clone()),
        throttle: Duration::from_millis(20),
        ..NetworkOptions::default()
    };
    let svc = NetworkService::spawn(options, sink).unwrap();
    state(&events, "ilk okuma", |s| s.available);
    svc.scan();
    wait_until("tarama", || log.has("RequestScan"));
    assert_eq!(log.count("Register"), 0);
}

/// Ayarlar gibi ajansız istemci.
fn start_client(bus: &PrivateBus) -> (NetworkService, Events<NetworkEvent>) {
    let (sink, events) = channel();
    let options = NetworkOptions {
        bus_address: Some(bus.address.clone()),
        call_timeout: Duration::from_secs(2),
        throttle: Duration::from_millis(20),
        max_backoff: Duration::from_millis(200),
        ..NetworkOptions::default()
    };
    (NetworkService::spawn(options, sink).unwrap(), events)
}

fn done(events: &Events<NetworkEvent>, action: &'static str) -> Option<String> {
    match events.wait_for(action, |e| matches!(e, NetworkEvent::Done { action: a, .. } if *a == action)) {
        NetworkEvent::Done { object, .. } => object,
        _ => unreachable!(),
    }
}

fn failed(events: &Events<NetworkEvent>, action: &'static str) -> NmError {
    match events.wait_for(action, |e| matches!(e, NetworkEvent::Failed { action: a, .. } if *a == action)) {
        NetworkEvent::Failed { error, .. } => error,
        _ => unreachable!(),
    }
}

#[test]
fn hidden_network_and_setting_patches() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeNm::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    state(&events, "ilk okuma", |s| s.saved.iter().any(|c| c.path == SAVED_OFFICE));

    // Gizli ağ: hidden=true, şifre ayarda, cihaz ilk Wi-Fi, özel nesne yok.
    svc.connect_hidden(b"Gizli", Security::WpaPsk, Some(Secret::new("gizli-sifre")));
    let saved = done(&events, "connect_hidden").expect("kayıtlı bağlantı yolu");
    assert!(log.has(&format!("AddAndActivate Gizli wpa-psk gizli-sifre {DEV} /")));
    assert!(log.has("hidden Gizli"));
    let s = state(&events, "gizli kayıtlı", |s| s.saved.iter().any(|c| c.path == saved));
    assert_eq!(s.saved.iter().find(|c| c.path == saved).unwrap().ssid.as_deref(), Some(&b"Gizli"[..]));
    // Açık gizli ağ: güvenlik ayarı yok.
    svc.connect_hidden(b"Acik", Security::Open, None);
    done(&events, "connect_hidden");
    assert!(log.has(&format!("AddAndActivate Acik - - {DEV} /")));
    // Ön denetimler: NM'ye gitmez.
    svc.connect_hidden(b"", Security::Open, None);
    assert_eq!(failed(&events, "connect_hidden"), NmError::InvalidSsid);
    svc.connect_hidden(&[b'x'; 33], Security::Open, None);
    assert_eq!(failed(&events, "connect_hidden"), NmError::InvalidSsid);
    svc.connect_hidden(b"Kurum", Security::Enterprise, None);
    assert_eq!(failed(&events, "connect_hidden"), NmError::Enterprise);
    svc.connect_hidden(b"Kisa", Security::WpaPsk, Some(Secret::new("kısa")));
    assert_eq!(failed(&events, "connect_hidden"), NmError::InvalidPassword);
    assert_eq!(log.count("AddAndActivate "), 2);

    // Update yaması: yalnız connection.autoconnect değişir; gizli bilgi gönderilmez ve korunur.
    let before = fake.stored(SAVED_OFFICE).unwrap();
    svc.set_autoconnect(SAVED_OFFICE, false);
    assert_eq!(done(&events, "autoconnect").as_deref(), Some(SAVED_OFFICE));
    assert!(log.has(&format!("Update {SAVED_OFFICE} secrets=0")));
    let mut after = fake.stored(SAVED_OFFICE).unwrap();
    assert_eq!(after["connection"]["autoconnect"], Value::Bool(false).try_into().unwrap());
    after.get_mut("connection").unwrap().remove("autoconnect");
    assert_eq!(after, before, "başka alan değişmedi");
    assert_eq!(fake.stored_str(SAVED_OFFICE, "802-11-wireless-security", "psk").as_deref(), Some("ofis-sifre"));
    let s = state(&events, "autoconnect kapalı", |s| s.saved.iter().any(|c| c.path == SAVED_OFFICE && !c.autoconnect));
    assert_eq!(s.saved.iter().find(|c| c.path == SAVED_OFFICE).unwrap().metered, Metered::Unknown);

    svc.set_metered(SAVED_OFFICE, Metered::Yes);
    done(&events, "metered");
    let stored = fake.stored(SAVED_OFFICE).unwrap();
    assert_eq!(stored["connection"]["metered"], Value::I32(1).try_into().unwrap());
    assert_eq!(stored["connection"]["autoconnect"], Value::Bool(false).try_into().unwrap(), "önceki yama kaldı");
    assert_eq!(fake.stored_str(SAVED_OFFICE, "802-11-wireless-security", "psk").as_deref(), Some("ofis-sifre"));
    state(&events, "ölçülü", |s| s.saved.iter().any(|c| c.path == SAVED_OFFICE && c.metered == Metered::Yes));

    svc.set_autoconnect("/org/freedesktop/NetworkManager/Settings/77", true);
    assert_eq!(failed(&events, "autoconnect"), NmError::ConnectionUnknown);
    assert_eq!(log.count("Update "), 2);
    // Ajansız istemci: kayıt yok.
    assert_eq!(log.count("Register"), 0);
    drop(svc);
}

/// wg(8) kılavuzundaki yayımlanmış örnek anahtarlar: gerçek gizli bilgi değil.
const WG_PRIV: &str = "bnV4dXQtd2lyZWd1YXJkLXRlc3QtcHJpdmF0ZS1rMDE=";
const WG_PUB: &str = "bnV4dXQtd2lyZWd1YXJkLXRlc3QtcHVibGljLWtleTE=";

#[test]
fn wireguard_import_and_delete() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeNm::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    state(&events, "ilk okuma", |s| s.vpns().count() == 1);

    // Hatalı dosya: hemen, tür olarak; NM'ye gitmez.
    let bad = svc.import_wireguard("is-vpn", &Secret::new("[Interface]\nPrivateKey = bozuk\n")).unwrap_err();
    assert_eq!((bad.line, bad.kind), (2, WireGuardErrorKind::InvalidKey));
    let bad = svc.import_wireguard("çok/kötü ad", &Secret::new(String::new())).unwrap_err();
    assert_eq!(bad.kind, WireGuardErrorKind::InvalidName);

    let conf = format!(
        "[Interface]\nPrivateKey = {WG_PRIV}\nAddress = 10.8.0.2/24\nDNS = 10.8.0.1\n\n\
         [Peer]\nPublicKey = {WG_PUB}\nAllowedIPs = 0.0.0.0/0\nEndpoint = vpn.example.com:51820\n"
    );
    let import = svc.import_wireguard("is-vpn", &Secret::new(conf)).unwrap();
    assert!(import.request.is_some() && import.ignored.is_empty());
    let path = done(&events, "import_wireguard").expect("yeni bağlantı yolu");
    assert!(log.has("AddConnection wireguard is-vpn"));
    assert_eq!(fake.stored_str(&path, "wireguard", "private-key").as_deref(), Some(WG_PRIV));
    assert_eq!(fake.stored_str(&path, "connection", "interface-name").as_deref(), Some("is-vpn"));
    assert_eq!(fake.stored_str(&path, "ipv4", "method").as_deref(), Some("manual"));
    let stored = fake.stored(&path).unwrap();
    assert_eq!(stored["wireguard"]["peers"].value_signature().to_string(), "aa{sv}");
    let s = state(&events, "VPN listede", |s| s.vpns().any(|v| v.path == path));
    let vpn = s.vpns().find(|v| v.path == path).unwrap();
    assert_eq!((vpn.id.as_str(), &vpn.kind, vpn.autoconnect), ("is-vpn", &ConnectionKind::WireGuard, false));

    // VPN silme (`forget` → `Delete`): listeden kalkar.
    svc.forget(&path);
    wait_until("silindi", || log.has(&format!("Delete {path}")));
    state(&events, "VPN gitti", |s| !s.vpns().any(|v| v.path == path) && s.vpns().count() == 1);
    drop(svc);
}

#[test]
fn device_details_and_activation_failure() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let _fake = FakeNm::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    let s = state(&events, "ilk okuma", |s| s.devices.len() == 2 && !s.saved.is_empty());
    let eth = s.devices.iter().find(|d| d.kind == DeviceKind::Ethernet).unwrap();
    assert_eq!((eth.path.as_str(), eth.interface.as_str(), eth.connected), (DEV_ETH, "enp1s0", true));
    assert_eq!(eth.active_connection.as_deref(), Some(ACTIVE_WIRED));

    svc.device_details(DEV_ETH);
    let d = match events.wait_for("ayrıntı", |e| matches!(e, NetworkEvent::DeviceDetails(_))) {
        NetworkEvent::DeviceDetails(d) => d,
        _ => unreachable!(),
    };
    assert_eq!((d.interface.as_str(), d.kind, d.speed_mbps), ("enp1s0", DeviceKind::Ethernet, Some(1000)));
    assert_eq!(d.hw_address.as_deref(), Some("52:54:00:12:34:56"));
    assert_eq!(d.ipv4[0].to_string(), "192.168.1.20/24");
    assert_eq!(d.ipv4_gateway, Some("192.168.1.1".parse().unwrap()));
    assert_eq!(d.ipv6[0].to_string(), "fd00::20/64");
    assert_eq!(d.dns, ["192.168.1.1".parse::<std::net::IpAddr>().unwrap(), "fd00::1".parse().unwrap()]);

    svc.device_details(DEV);
    let d = match events.wait_for("Wi-Fi ayrıntı", |e| matches!(e, NetworkEvent::DeviceDetails(_))) {
        NetworkEvent::DeviceDetails(d) => d,
        _ => unreachable!(),
    };
    assert_eq!((d.kind, d.speed_mbps, d.ipv4.len()), (DeviceKind::Wifi, Some(433), 0));
    svc.device_details("/org/freedesktop/NetworkManager/Devices/9");
    assert_eq!(failed(&events, "device_details"), NmError::DeviceUnknown);

    // Ajansız istemci: şifresi NM'de olmayan ağ → NO_SECRETS → "şifreyi güncelle".
    svc.activate(SAVED_HOME);
    match events.wait_for("etkinleşme hatası", |e| matches!(e, NetworkEvent::ActivationFailed { .. })) {
        NetworkEvent::ActivationFailed { connection, reason } => {
            assert_eq!((connection.as_deref(), reason), (Some(SAVED_HOME), ActivationFailure::NoSecrets));
        }
        _ => unreachable!(),
    }
    let asked = events.collect(Duration::from_millis(100)).into_iter().any(|e| matches!(e, NetworkEvent::SecretRequest(_)));
    assert!(!asked);
    assert_eq!(log.count("Register"), 0);
    drop(svc);
}

/// Kullanılamama nedeni: bus'a ulaşılamıyor (bir kez yayınlanır) / NM çalışmıyor / NM gelince yok.
#[test]
fn unavailable_reasons() {
    let (sink, events) = channel();
    let options = NetworkOptions {
        bus_address: Some("unix:path=/nonexistent/nuxut-test-bus".into()),
        max_backoff: Duration::from_millis(50),
        ..NetworkOptions::default()
    };
    let svc = NetworkService::spawn(options, sink).unwrap();
    let states: Vec<NetworkState> = events
        .collect(Duration::from_millis(600))
        .into_iter()
        .filter_map(|e| match e {
            NetworkEvent::State(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(states.len(), 1, "yeniden denemelerde aynı durum tekrar yayınlanmaz");
    assert_eq!((states[0].available, states[0].unavailable), (false, Some(Unavailable::BusUnreachable)));
    drop(svc);

    let Some(bus) = PrivateBus::start() else { return };
    let (svc, events) = start_client(&bus);
    state(&events, "NM yok", |s| !s.available && s.unavailable == Some(Unavailable::ServiceNotRunning));
    let log = Log::default();
    let _fake = FakeNm::start(bus.connect(), &log, false);
    state(&events, "NM geldi", |s| s.available && s.unavailable.is_none());
    drop(svc);
}

/// `Done`'u bekler; öncesinde gelen olaylar `seen`'e (durum sırası denetimi).
fn done_for(events: &Events<NetworkEvent>, request: RequestId, seen: &mut Vec<NetworkEvent>) -> Option<String> {
    let e = events.wait_for("Done", |e| {
        let hit = matches!(e, NetworkEvent::Done { request: r, .. } if *r == request);
        if !hit {
            seen.push(e.clone());
        }
        hit
    });
    match e {
        NetworkEvent::Done { object, .. } => object,
        _ => unreachable!(),
    }
}

/// Koşulu sağlayan durum ya `seen`'de (sonuçtan önce gelmiş) ya da sonra gelir.
fn state_seen_or_next(events: &Events<NetworkEvent>, seen: &[NetworkEvent], what: &str, pred: impl Fn(&NetworkState) -> bool) {
    if seen.iter().any(|e| matches!(e, NetworkEvent::State(s) if pred(s))) {
        return;
    }
    state(events, what, pred);
}

fn failed_for(events: &Events<NetworkEvent>, request: RequestId) -> NmError {
    match events.wait_for("Failed", |e| matches!(e, NetworkEvent::Failed { request: Some(r), .. } if *r == request)) {
        NetworkEvent::Failed { error, .. } => error,
        _ => unreachable!(),
    }
}

/// e2e (1): kayıtlı Wi-Fi şifresi güncellenir — yalnız şifre değişir, diğer alanlar korunur;
/// "Ev" yanlış şifreyle etkinleşmez (NO_SECRETS), doğrusuyla etkinleşir.
#[test]
fn wifi_password_update() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeNm::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    state(&events, "ilk okuma", |s| s.saved.iter().any(|c| c.path == SAVED_OFFICE));

    let r = svc.set_autoconnect(SAVED_HOME, false).unwrap();
    done_for(&events, r, &mut Vec::new());
    let r = svc.set_wifi_password(SAVED_HOME, Secret::new("kısa")).unwrap();
    assert_eq!(failed_for(&events, r), NmError::InvalidPassword);
    let r = svc.set_wifi_password("/org/freedesktop/NetworkManager/Settings/77", Secret::new("12345678")).unwrap();
    assert_eq!(failed_for(&events, r), NmError::ConnectionUnknown);

    let r = svc.set_wifi_password(SAVED_HOME, Secret::new("yanlis-sifre")).unwrap();
    assert_eq!(done_for(&events, r, &mut Vec::new()).as_deref(), Some(SAVED_HOME));
    assert!(log.has(&format!("Update {SAVED_HOME} secrets=1")));
    assert_eq!(fake.stored_str(SAVED_HOME, "802-11-wireless-security", "psk").as_deref(), Some("yanlis-sifre"));
    let stored = fake.stored(SAVED_HOME).unwrap();
    assert_eq!(stored["connection"]["autoconnect"], Value::Bool(false).try_into().unwrap(), "diğer alan korundu");
    assert_eq!(fake.stored_str(SAVED_HOME, "802-11-wireless-security", "key-mgmt").as_deref(), Some("wpa-psk"));

    svc.activate(SAVED_HOME);
    match events.wait_for("yanlış şifre", |e| matches!(e, NetworkEvent::ActivationFailed { .. })) {
        NetworkEvent::ActivationFailed { reason, .. } => assert_eq!(reason, ActivationFailure::NoSecrets),
        _ => unreachable!(),
    }
    let r = svc.set_wifi_password(SAVED_HOME, Secret::new(EV_PASSWORD)).unwrap();
    done_for(&events, r, &mut Vec::new());
    let r = svc.activate(SAVED_HOME).unwrap();
    done_for(&events, r, &mut Vec::new());
    assert!(!log.has(&format!("WrongPassword {SAVED_HOME}")) || log.count("WrongPassword ") == 1);
    drop(svc);
}

/// e2e (2) + (3): polkit reddi tür olarak; eylemler istek kimlikli sonuç verir, yeni durum
/// sonuçtan önce gelir; yeni bağlantı ve VPN gerçekten etkinleşir.
#[test]
fn request_results_and_denied() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeNm::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    state(&events, "ilk okuma", |s| s.saved.iter().any(|c| c.path == SAVED_VPN));

    let r = svc.set_wifi_enabled(false).unwrap();
    assert_eq!(done_for(&events, r, &mut Vec::new()), None);
    let r2 = svc.set_wifi_enabled(true).unwrap();
    assert_ne!(r, r2, "kimlikler artar");
    done_for(&events, r2, &mut Vec::new());

    // Kayıtlı Ofis'i etkinleştir: etkin nesne oluşur; durum Done'dan önce.
    let r = svc.activate(SAVED_OFFICE).unwrap();
    let mut seen = Vec::new();
    done_for(&events, r, &mut seen);
    let active_before = seen.iter().any(|e| {
        matches!(e, NetworkEvent::State(s) if s.saved.iter().any(|c| c.path == SAVED_OFFICE && c.active.is_some()))
    });
    assert!(active_before, "yeni durum sonuçtan önce yayınlanmalı");

    // Yeni ağ (Kafe): Done.object yeni kayıtlı bağlantı, etkin.
    let r = svc.connect_access_point(AP_CAFE, Some(Secret::new("12345678"))).unwrap();
    let mut seen = Vec::new();
    let saved = done_for(&events, r, &mut seen).expect("kayıtlı bağlantı");
    state_seen_or_next(&events, &seen, "Kafe etkin", |s| s.saved.iter().any(|c| c.path == saved && c.active.is_some()));

    // VPN etkinleşir, kapatılır.
    // Durum sonuçtan önce geldiğinden etkin VPN `seen` içinde.
    let r = svc.activate(SAVED_VPN).unwrap();
    let mut seen = Vec::new();
    done_for(&events, r, &mut seen);
    let active = seen
        .iter()
        .rev()
        .find_map(|e| match e {
            NetworkEvent::State(s) => s.vpns().find(|v| v.path == SAVED_VPN)?.active.clone(),
            _ => None,
        })
        .expect("VPN etkin durumu Done'dan önce");
    let r = svc.deactivate(&active).unwrap();
    let mut seen = Vec::new();
    done_for(&events, r, &mut seen);
    state_seen_or_next(&events, &seen, "VPN kapalı", |s| s.vpns().any(|v| v.path == SAVED_VPN && v.active.is_none()));

    // polkit reddi.
    fake.set_deny(true);
    let r = svc.forget(SAVED_OFFICE).unwrap();
    assert_eq!(failed_for(&events, r), NmError::Denied);
    let r = svc.set_metered(SAVED_OFFICE, Metered::Yes).unwrap();
    assert_eq!(failed_for(&events, r), NmError::Denied);
    fake.set_deny(false);
    let r = svc.forget(SAVED_OFFICE).unwrap();
    done_for(&events, r, &mut Vec::new());
    drop(svc);
}

/// e2e (4): yok sayılan wg-quick anahtarları adlarıyla bildirilir (değer asla).
#[test]
fn wireguard_ignored_keys_reported() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let _fake = FakeNm::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    state(&events, "ilk okuma", |s| s.available);
    let conf = format!(
        "[Interface]\nPrivateKey = {WG_PRIV}\nPostUp = iptables -A gizli-komut\nTable = off\nSaveConfig = true\n\
         PostUp = ikinci\n[Peer]\nPublicKey = {WG_PUB}\n"
    );
    let import = svc.import_wireguard("wg-ign", &Secret::new(conf)).unwrap();
    assert_eq!(import.ignored, ["PostUp", "Table", "SaveConfig"]);
    assert!(!format!("{import:?}").contains("gizli-komut"));
    done_for(&events, import.request.unwrap(), &mut Vec::new());
    drop(svc);
}

/// e2e (5): sahte NM sahneleri.
#[test]
fn fake_scenes() {
    for (scene, check) in [
        (Scene::NoDevice, (|s: &NetworkState| s.wifi_devices.is_empty() && s.access_points.is_empty() && s.saved.is_empty())
            as fn(&NetworkState) -> bool),
        (Scene::Empty, |s| s.wifi_devices.len() == 1 && s.access_points.is_empty() && s.saved.is_empty() && s.wifi_enabled),
        (Scene::WifiOff, |s| !s.wifi_enabled && s.access_points.is_empty() && !s.saved.is_empty()),
        (Scene::Demo, |s| s.access_points.len() == 4 && s.vpns().count() == 1),
    ] {
        let Some(bus) = PrivateBus::start() else { return };
        let log = Log::default();
        let _fake = FakeNm::start_scene(bus.connect(), &log, scene);
        let (_svc, events) = start_client(&bus);
        state(&events, &format!("{scene:?}"), |s| s.available && check(s));
    }
    assert_eq!(Scene::parse("wifi-off"), Some(Scene::WifiOff));
    assert_eq!(Scene::parse("default"), Some(Scene::Demo));
    assert_eq!(Scene::parse("bilinmeyen"), None);
}

/// Review şartı (1)(2): `psk-flags` 1/4 → şifre sistemde saklanır (`psk-flags=0`); 2 →
/// `SecretNotSaved`, ayarlar değişmez; WEP → `Unsupported`, ayarlar (wep-key0..3 dahil) değişmez.
#[test]
fn wifi_password_flags_and_wep() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let fake = FakeNm::start(bus.connect(), &log, true);
    let (svc, events) = start_client(&bus);
    state(&events, "ilk okuma", |s| s.saved.iter().any(|c| c.path == SAVED_WEP));
    let flags = |p: &str| -> Option<u32> { fake.stored(p)?["802-11-wireless-security"].get("psk-flags")?.try_into().ok() };

    for (path, pw) in [(SAVED_AGENT_OWNED, "ajan-sifre-1"), (SAVED_NOT_REQUIRED, "gerek-sifre-1")] {
        let r = svc.set_wifi_password(path, Secret::new(pw)).unwrap();
        done_for(&events, r, &mut Vec::new());
        assert_eq!(fake.stored_str(path, "802-11-wireless-security", "psk").as_deref(), Some(pw), "kalıcı");
        assert_eq!(flags(path), Some(0), "sistem sahipli");
    }

    let before = fake.stored(SAVED_NOT_SAVED).unwrap();
    let r = svc.set_wifi_password(SAVED_NOT_SAVED, Secret::new("sorulsun-123")).unwrap();
    assert_eq!(failed_for(&events, r), NmError::SecretNotSaved);
    assert_eq!(fake.stored(SAVED_NOT_SAVED).unwrap(), before, "değişmedi");
    assert_eq!(flags(SAVED_NOT_SAVED), Some(2));

    let before = fake.stored(SAVED_WEP).unwrap();
    let r = svc.set_wifi_password(SAVED_WEP, Secret::new("12345")).unwrap();
    assert_eq!(failed_for(&events, r), NmError::Unsupported);
    assert_eq!(fake.stored(SAVED_WEP).unwrap(), before, "WEP değişmedi");
    for k in ["wep-key0", "wep-key1", "wep-key2", "wep-key3"] {
        assert!(fake.stored_str(SAVED_WEP, "802-11-wireless-security", k).is_some(), "{k} korundu");
    }
    // Reddedilenlerde NM'ye Update gitmedi.
    assert!(!log.has(&format!("Update {SAVED_NOT_SAVED} secrets=1")));
    assert!(!log.has(&format!("Update {SAVED_WEP} secrets=1")));
    // Sahte günlüğü şifre değeri yazmaz.
    assert!(!log.has("ajan-sifre-1") && log.count("Update ") == 2);
    drop(svc);
}
