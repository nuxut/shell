//! fprintd istemcisi, özel bir `dbus-daemon` üzerinde sahte fprintd ile. Kullanıcının sistem
//! bus'ına ve gerçek okuyucuya dokunulmaz.
//!
//! Yük altında kararlılık (2026-10-14): testler süreye değil sonuca dayanır. Sahte fprintd'nin
//! "yanıttan sonra" sinyalleri sabit bir gecikmeyle değil, istemcinin `Scanning` olayıyla (yani
//! `VerifyStart` yanıtını ALDIĞININ kanıtıyla) tetiklenir ([`Fixture::scanning`]); D-Bus aynı
//! göndericiden gelen iletilerin sırasını korur, böylece bu sinyaller her zaman bu neslin yanıtından
//! sonra alınır. Olumlu beklemeler [`LIVENESS`] ile yalnızca asılı kalmaya karşı sınırlıdır;
//! "kabul edilmedi" iddiaları mümkün olduğunca bir sonraki olumlu olayla senkronlanır.

mod common;

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{LIVENESS, PrivateBus};
use nuxut_lock::fingerprint::{
    DEVICE, FPRINT, Fingerprint, FingerprintOptions, FpEvent, MANAGER_PATH,
};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;

const DEVICE_PATH: &str = "/net/reactivated/Fprint/Device/0";

type Calls = Arc<Mutex<Vec<String>>>;

/// Sahte fprintd'nin bir sinyali: yanıttan önce (eski nesil) ya da sonra (gerçekçi).
#[derive(Clone, Copy)]
struct Emit {
    result: &'static str,
    done: bool,
    after_reply: bool,
}

fn after(result: &'static str, done: bool) -> Emit {
    Emit {
        result,
        done,
        after_reply: true,
    }
}

fn before(result: &'static str, done: bool) -> Emit {
    Emit {
        result,
        done,
        after_reply: false,
    }
}

/// Her VerifyStart için yayınlanacak sinyal toplulukları.
type Script = Arc<Mutex<VecDeque<Vec<Emit>>>>;
/// Son VerifyStart'ın yanıttan sonra yayınlanacak sinyalleri; test `Scanning`'i görünce yayınlar.
type Pending = Arc<Mutex<Vec<Emit>>>;

struct FakeManager;

#[zbus::interface(name = "net.reactivated.Fprint.Manager")]
impl FakeManager {
    fn get_default_device(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(DEVICE_PATH).unwrap()
    }
}

struct FakeDevice {
    calls: Calls,
    enrolled: Vec<String>,
    script: Script,
    pending: Pending,
    /// VerifyStop sırasında yayınlanan (bir sonraki nesle göre eski) sinyaller.
    on_stop: Vec<Emit>,
}

async fn emit_all(emitter: &SignalEmitter<'_>, items: &[Emit]) {
    for e in items {
        let _ = FakeDevice::verify_status(emitter, e.result, e.done).await;
    }
}

#[zbus::interface(name = "net.reactivated.Fprint.Device")]
impl FakeDevice {
    fn list_enrolled_fingers(&self, user: &str) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("ListEnrolledFingers {user}"));
        self.enrolled.clone()
    }
    fn claim(&self, user: &str) {
        self.calls.lock().unwrap().push(format!("Claim {user}"));
    }
    fn release(&self) {
        self.calls.lock().unwrap().push("Release".into());
    }
    async fn verify_start(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>, finger: &str) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("VerifyStart {finger}"));
        let batch = self.script.lock().unwrap().pop_front().unwrap_or_default();
        let (later, now): (Vec<Emit>, Vec<Emit>) = batch.into_iter().partition(|e| e.after_reply);
        // Yanıttan önce: bu neslin yanıtından önce alınır → eski sayılmalı.
        emit_all(&emitter, &now).await;
        // Yanıttan sonra: sabit gecikme yerine testin `Scanning`'i görmesini bekler (yük altında
        // gecikmeli yanıt sinyalin gerisinde kalıp sinyali "eski" yapıyordu).
        *self.pending.lock().unwrap() = later;
    }
    async fn verify_stop(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        self.calls.lock().unwrap().push("VerifyStop".into());
        emit_all(&emitter, &self.on_stop).await;
    }
    #[zbus(signal)]
    async fn verify_status(
        emitter: &SignalEmitter<'_>,
        result: &str,
        done: bool,
    ) -> zbus::Result<()>;
}

struct Fixture {
    bus: PrivateBus,
    calls: Calls,
    pending: Pending,
    server: Option<zbus::blocking::Connection>,
}

impl Fixture {
    /// Sahte fprintd'nin kendi bağlantısından (fprintd'nin benzersiz adıyla) bir VerifyStatus.
    fn emit(&self, result: &str, done: bool) {
        self.server
            .as_ref()
            .expect("sahte fprintd düşürüldü")
            .emit_signal(
                None::<()>,
                DEVICE_PATH,
                DEVICE,
                "VerifyStatus",
                &(result, done),
            )
            .unwrap();
    }

    /// Son VerifyStart'ın "yanıttan sonra" sinyallerini yayınlar.
    fn flush(&self) {
        let later = std::mem::take(&mut *self.pending.lock().unwrap());
        for e in later {
            self.emit(e.result, e.done);
        }
    }

    /// `Scanning` bekler (= istemci VerifyStart yanıtını aldı), sonra bu neslin sinyallerini yayınlar.
    fn scanning(&self, rx: &Receiver<FpEvent>) {
        assert_eq!(next(rx), FpEvent::Scanning);
        self.flush();
    }
}

fn fixture_with(enrolled: &[&str], script: Vec<Vec<Emit>>, on_stop: Vec<Emit>) -> Option<Fixture> {
    let bus = PrivateBus::start()?;
    let calls: Calls = Arc::default();
    let pending: Pending = Arc::default();
    let server = zbus::blocking::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .method_timeout(LIVENESS)
        .name(FPRINT)
        .unwrap()
        .serve_at(MANAGER_PATH, FakeManager)
        .unwrap()
        .serve_at(
            DEVICE_PATH,
            FakeDevice {
                calls: calls.clone(),
                enrolled: enrolled.iter().map(|s| s.to_string()).collect(),
                script: Arc::new(Mutex::new(script.into())),
                pending: pending.clone(),
                on_stop,
            },
        )
        .unwrap()
        .build()
        .unwrap();
    Some(Fixture {
        bus,
        calls,
        pending,
        server: Some(server),
    })
}

fn fixture(enrolled: &[&str], script: Vec<Vec<Emit>>) -> Option<Fixture> {
    fixture_with(enrolled, script, vec![])
}

fn start(address: &str, tries: u32) -> (Fingerprint, Receiver<FpEvent>) {
    let (tx, rx) = mpsc::channel();
    let options = FingerprintOptions {
        bus_address: Some(address.to_owned()),
        user: "deneme".into(),
        tries,
        // Üründe 5 sn (app.rs). Testte geniş: yük altında (MemoryHigh kısması) sahte fprintd'nin
        // yanıtı saniyelerce gecikebiliyor, zaman aşımı → Unavailable yanlış alarm veriyordu. Zaman
        // aşımının kendisi (hata → Unavailable, asla eşleşme) `run`'ın `?` yolunda; o yol
        // `missing_service_is_unavailable` ve sahibin düşmesi testleriyle kapsanır.
        call_timeout: LIVENESS,
    };
    let fp = Fingerprint::spawn(options, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    (fp, rx)
}

fn next(rx: &Receiver<FpEvent>) -> FpEvent {
    rx.recv_timeout(LIVENESS).expect("olay gelmedi")
}

fn wait_for_call(calls: &Calls, name: &str) {
    let deadline = Instant::now() + LIVENESS;
    while Instant::now() < deadline {
        if calls.lock().unwrap().iter().any(|c| c == name) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{name} çağrılmadı: {:?}", calls.lock().unwrap());
}

/// Sahte bir bağlantının yayınladığı her iletinin `dbus-daemon` tarafından dağıtıldığını kanıtlar:
/// daemon bir bağlantının iletilerini sırayla işler, bu çağrının yanıtı geldiğinde öncekiler
/// alıcıların kuyruğundadır.
fn roundtrip(conn: &zbus::blocking::Connection) {
    zbus::blocking::fdo::DBusProxy::new(conn)
        .unwrap()
        .get_id()
        .unwrap();
}

#[test]
fn match_after_a_miss() {
    let Some(f) = fixture(
        &["right-index-finger"],
        vec![
            vec![after("verify-no-match", true)],
            vec![after("verify-match", true)],
        ],
    ) else {
        return;
    };
    let (_fp, rx) = start(&f.bus.address, 3);
    f.scanning(&rx);
    assert_eq!(next(&rx), FpEvent::NoMatch { remaining: 2 });
    f.scanning(&rx);
    assert_eq!(next(&rx), FpEvent::Matched);
    wait_for_call(&f.calls, "Release");
    let calls = f.calls.lock().unwrap().clone();
    assert_eq!(calls[0], "ListEnrolledFingers deneme");
    assert_eq!(calls[1], "Claim deneme");
    assert_eq!(calls.iter().filter(|c| *c == "VerifyStart any").count(), 2);
}

#[test]
fn retries_do_not_count_and_tries_run_out() {
    let Some(f) = fixture(
        &["x"],
        vec![
            vec![
                after("verify-retry-scan", false),
                after("verify-no-match", true),
            ],
            vec![after("verify-no-match", true)],
        ],
    ) else {
        return;
    };
    let (_fp, rx) = start(&f.bus.address, 2);
    f.scanning(&rx);
    assert!(matches!(next(&rx), FpEvent::Retry(_)));
    assert_eq!(next(&rx), FpEvent::NoMatch { remaining: 1 });
    f.scanning(&rx);
    assert_eq!(next(&rx), FpEvent::Exhausted);
    wait_for_call(&f.calls, "Release");
}

#[test]
fn nothing_enrolled_is_unavailable_without_claim() {
    let Some(f) = fixture(&[], vec![]) else {
        return;
    };
    let (_fp, rx) = start(&f.bus.address, 3);
    assert_eq!(next(&rx), FpEvent::Unavailable);
    assert!(
        !f.calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.starts_with("Claim"))
    );
}

#[test]
fn missing_service_is_unavailable() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    let (_fp, rx) = start(&bus.address, 3);
    assert_eq!(next(&rx), FpEvent::Unavailable);
}

#[test]
fn fatal_status_is_unavailable() {
    let Some(f) = fixture(&["x"], vec![vec![after("verify-disconnected", true)]]) else {
        return;
    };
    let (_fp, rx) = start(&f.bus.address, 3);
    f.scanning(&rx);
    assert_eq!(next(&rx), FpEvent::Unavailable);
    wait_for_call(&f.calls, "Release");
}

#[test]
fn spoofed_match_from_other_sender_is_ignored() {
    let Some(f) = fixture(&["x"], vec![vec![]]) else {
        return;
    };
    let (mut fp, rx) = start(&f.bus.address, 3);
    f.scanning(&rx);
    // Aynı yol/arayüz/üye, ama fprintd adının sahibi olmayan bir bağlantıdan.
    let attacker = zbus::blocking::connection::Builder::address(f.bus.address.as_str())
        .unwrap()
        .build()
        .unwrap();
    for _ in 0..5 {
        attacker
            .emit_signal(
                None::<()>,
                DEVICE_PATH,
                DEVICE,
                "VerifyStatus",
                &("verify-match", true),
            )
            .unwrap();
    }
    // Sahte sinyaller istemcinin kuyruğunda; ardından gerçek fprintd'den bir no-match. Sıradaki olay
    // bu no-match olmalı: sahte eşleşmeler okundu ve yok sayıldı (süreye dayanmayan kanıt).
    roundtrip(&attacker);
    f.emit("verify-no-match", false);
    assert_eq!(
        next(&rx),
        FpEvent::NoMatch { remaining: 2 },
        "sahte sinyal kabul edildi"
    );
    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "sahte sinyal kabul edildi"
    );
    fp.stop();
    wait_for_call(&f.calls, "Release");
}

#[test]
fn stop_releases_device() {
    let Some(f) = fixture(&["x"], vec![vec![]]) else {
        return;
    };
    let (mut fp, rx) = start(&f.bus.address, 3);
    f.scanning(&rx);
    fp.stop();
    wait_for_call(&f.calls, "VerifyStop");
    wait_for_call(&f.calls, "Release");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

/// Gelen olayları kısa süre toplar (en çok `ms`). Yalnızca "olay GELMEMELİ" iddiaları için: yük
/// altında pencere kısa kalırsa test gevşer ama yanlış alarm vermez.
fn drain(rx: &Receiver<FpEvent>, ms: u64) -> Vec<FpEvent> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    let mut out = vec![];
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(e) => out.push(e),
            Err(_) => break,
        }
    }
    out
}

/// Oturumu bitiren olaya (`Matched`/`Unavailable`/`Exhausted`) kadar gelen her olay; ardından
/// thread biter, başka olay gelemez.
fn until_terminal(rx: &Receiver<FpEvent>) -> Vec<FpEvent> {
    let mut out = vec![];
    loop {
        let e = next(rx);
        let end = matches!(
            e,
            FpEvent::Matched | FpEvent::Unavailable | FpEvent::Exhausted
        );
        out.push(e);
        if end {
            return out;
        }
    }
}

#[test]
fn stale_match_before_verify_start_reply_is_ignored() {
    // fprintd yanıttan ÖNCE (önceki oturumdan kalma) "verify-match" yayınlar: kabul edilmemeli.
    let Some(f) = fixture(
        &["x"],
        vec![vec![
            before("verify-match", true),
            after("verify-no-match", false),
            after("verify-match", true),
        ]],
    ) else {
        return;
    };
    let (_fp, rx) = start(&f.bus.address, 3);
    f.scanning(&rx);
    assert_eq!(
        next(&rx),
        FpEvent::NoMatch { remaining: 2 },
        "eski nesil sinyali kabul edildi"
    );
    assert_eq!(next(&rx), FpEvent::Matched);
}

#[test]
fn late_match_from_previous_generation_is_ignored() {
    // 1. tarama no-match ile biter; VerifyStop sırasında geç bir "verify-match" gelir. 2. neslin
    // VerifyStart yanıtından önce alındığı için yok sayılmalı. 2. neslin no-match'i (yanıttan
    // sonra) sıradaki olay olmalı: geç eşleşme ondan önce okundu ve reddedildi.
    let Some(f) = fixture_with(
        &["x"],
        vec![
            vec![after("verify-no-match", true)],
            vec![after("verify-no-match", false)],
        ],
        vec![before("verify-match", true)],
    ) else {
        return;
    };
    let (mut fp, rx) = start(&f.bus.address, 3);
    f.scanning(&rx);
    assert_eq!(next(&rx), FpEvent::NoMatch { remaining: 2 });
    f.scanning(&rx);
    assert_eq!(
        next(&rx),
        FpEvent::NoMatch { remaining: 1 },
        "önceki neslin eşleşmesi kabul edildi"
    );
    let rest = drain(&rx, 300);
    assert!(
        !rest.contains(&FpEvent::Matched),
        "önceki neslin eşleşmesi kabul edildi: {rest:?}"
    );
    fp.stop();
}

#[test]
fn service_vanishing_after_a_signal_is_unavailable() {
    let Some(mut f) = fixture(&["x"], vec![vec![after("verify-no-match", false)]]) else {
        return;
    };
    let (_fp, rx) = start(&f.bus.address, 3);
    f.scanning(&rx);
    assert_eq!(next(&rx), FpEvent::NoMatch { remaining: 2 });
    // fprintd düşer: ad sahipsiz kalır.
    drop(f.server.take());
    assert_eq!(next(&rx), FpEvent::Unavailable);
    assert!(drain(&rx, 300).is_empty());
}

#[test]
fn new_owner_cannot_inject_a_match() {
    // fprintd düşer ve adı başka bir bağlantı alır, hemen "verify-match" yayınlar.
    let Some(mut f) = fixture(&["x"], vec![vec![]]) else {
        return;
    };
    let (_fp, rx) = start(&f.bus.address, 3);
    f.scanning(&rx);
    drop(f.server.take());
    let impostor = zbus::blocking::connection::Builder::address(f.bus.address.as_str())
        .unwrap()
        .name(FPRINT)
        .unwrap()
        .build()
        .unwrap();
    for _ in 0..5 {
        impostor
            .emit_signal(
                None::<()>,
                DEVICE_PATH,
                DEVICE,
                "VerifyStatus",
                &("verify-match", true),
            )
            .unwrap();
    }
    // Oturum bir son olayla biter (sahip değişti → Unavailable); ona kadar eşleşme olmamalı.
    let events = until_terminal(&rx);
    assert!(
        !events.contains(&FpEvent::Matched),
        "yeni sahip eşleşme enjekte etti: {events:?}"
    );
    assert_eq!(events.last(), Some(&FpEvent::Unavailable), "{events:?}");
}

#[test]
fn no_events_after_stop_even_if_match_arrives() {
    let Some(f) = fixture(&["x"], vec![vec![after("verify-match", true)]]) else {
        return;
    };
    let (mut fp, rx) = start(&f.bus.address, 3);
    assert_eq!(next(&rx), FpEvent::Scanning);
    // Önce dur, SONRA gerçek fprintd eşleşmeyi yayınlasın (eskiden 60 ms'lik gecikmeye dayanıyordu;
    // yük altında eşleşme durdurmadan önce gelip testi yanlış düşürebiliyordu).
    fp.stop();
    f.flush();
    assert!(drain(&rx, 500).is_empty(), "durdurulmuş oturum olay iletti");
}
