//! Boşta kalma logind bağlantısı: sahte `org.freedesktop.login1` ile özel bus'ta uçtan uca.
//! Kullanıcının sistem bus'ına dokunulmaz (gerçek inhibitör alınmaz, kilit/uyku çağrılmaz).
//!
//! Kapsam: oturumun `session/auto` yedeğiyle çözülmesi, ilk okuma (`InhibitDelayMaxUSec`,
//! `BlockInhibited`, `LockedHint`), uyku gecikme inhibitörünün alınıp bırakılması (fd kapanır),
//! `PrepareForSleep`, özellik değişimleri, kilit/uyku çağrıları, sahte (unicast) sinyalin reddi,
//! logind'in düşüp gelmesi (istek kalıcı: inhibitör yeniden alınır), temiz kapanış.

use std::io::Read;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nuxut_shell::services::idle::{Effect, IdleConfig, IdleLogind, IdleMachine, LogindAction, LogindEvent, LogindOptions};
use nuxut_shell::services::lock_launcher::{LockConfig, LockLauncher, LockTrigger, decide};
use crate::support::{Events, PrivateBus, channel, wait_until};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;

const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION_PATH: &str = "/org/freedesktop/login1/session/c7";
const AUTO_PATH: &str = "/org/freedesktop/login1/session/auto";

type Calls = Arc<Mutex<Vec<String>>>;
type Held = Arc<Mutex<Vec<std::io::PipeReader>>>;

struct FakeManager {
    calls: Calls,
    held: Held,
    block: String,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeManager {
    #[zbus(property)]
    fn block_inhibited(&self) -> String {
        self.block.clone()
    }
    #[zbus(property, name = "InhibitDelayMaxUSec")]
    fn inhibit_delay_max_usec(&self) -> u64 {
        2_000_000
    }
    /// Servis süreci oturum dışında (systemd kullanıcı birimi gibi): `session/auto` yedeği denenir.
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, _pid: u32) -> zbus::fdo::Result<OwnedObjectPath> {
        Err(zbus::fdo::Error::Failed("NoSessionForPID".into()))
    }
    fn get_session(&self, id: String) -> zbus::fdo::Result<OwnedObjectPath> {
        if id != "c7" {
            return Err(zbus::fdo::Error::Failed("NoSuchSession".into()));
        }
        Ok(OwnedObjectPath::try_from(SESSION_PATH).unwrap())
    }
    fn inhibit(&self, what: String, _who: String, _why: String, mode: String) -> zbus::zvariant::OwnedFd {
        self.calls.lock().unwrap().push(format!("Inhibit({what},{mode})"));
        let (reader, writer) = std::io::pipe().unwrap();
        self.held.lock().unwrap().push(reader);
        zbus::zvariant::OwnedFd::from(std::os::fd::OwnedFd::from(writer))
    }
    fn suspend(&self, interactive: bool) {
        self.calls.lock().unwrap().push(format!("Suspend({interactive})"));
    }
    #[zbus(signal)]
    async fn prepare_for_sleep(emitter: &SignalEmitter<'_>, start: bool) -> zbus::Result<()>;
}

struct FakeSession {
    calls: Calls,
    hints: Calls,
    locked: bool,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl FakeSession {
    #[zbus(property)]
    fn id(&self) -> String {
        "c7".into()
    }
    #[zbus(property)]
    fn locked_hint(&self) -> bool {
        self.locked
    }
    fn lock(&self) {
        self.calls.lock().unwrap().push("Lock".into());
    }
    fn set_idle_hint(&self, idle: bool) {
        self.hints.lock().unwrap().push(idle.to_string());
    }
}

struct Fake {
    conn: zbus::blocking::Connection,
    calls: Calls,
    hints: Calls,
    held: Held,
}

fn serve(bus: &PrivateBus) -> Fake {
    let calls = Calls::default();
    let held = Held::default();
    let hints = Calls::default();
    let conn = bus.connect();
    let server = conn.object_server();
    server.at(LOGIN1_PATH, FakeManager { calls: calls.clone(), held: held.clone(), block: String::new() }).unwrap();
    server.at(SESSION_PATH, FakeSession { calls: calls.clone(), hints: hints.clone(), locked: false }).unwrap();
    server.at(AUTO_PATH, FakeSession { calls: calls.clone(), hints: Calls::default(), locked: false }).unwrap();
    conn.request_name(LOGIN1).unwrap();
    drop(server);
    Fake { conn, calls, hints, held }
}

impl Fake {
    fn prepare_for_sleep(&self, start: bool) {
        let iface = self.conn.object_server().interface::<_, FakeManager>(LOGIN1_PATH).unwrap();
        zbus::block_on(FakeManager::prepare_for_sleep(iface.signal_emitter(), start)).unwrap();
    }

    fn set_block(&self, block: &str) {
        let iface = self.conn.object_server().interface::<_, FakeManager>(LOGIN1_PATH).unwrap();
        let emitter = iface.signal_emitter().clone();
        let mut m = iface.get_mut();
        m.block = block.into();
        zbus::block_on(m.block_inhibited_changed(&emitter)).unwrap();
    }

    fn set_locked(&self, locked: bool) {
        let iface = self.conn.object_server().interface::<_, FakeSession>(SESSION_PATH).unwrap();
        let emitter = iface.signal_emitter().clone();
        let mut s = iface.get_mut();
        s.locked = locked;
        zbus::block_on(s.locked_hint_changed(&emitter)).unwrap();
    }

    /// Verilen son inhibitörün okuma ucu: yazma ucu (servisteki fd) kapanınca EOF.
    fn last_inhibitor(&self) -> Receiver<()> {
        let mut reader = self.held.lock().unwrap().pop().expect("inhibitör verilmedi");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf);
            let _ = tx.send(());
        });
        rx
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn hints(&self) -> Vec<String> {
        self.hints.lock().unwrap().clone()
    }
}

fn start(bus: &PrivateBus) -> (IdleLogind, Events<LogindEvent>) {
    let (sink, events) = channel();
    let options = LogindOptions {
        bus_address: Some(bus.address.clone()),
        call_timeout: Duration::from_millis(500),
        max_backoff: Duration::from_millis(200),
        pid: 999_999,
        compositor_pid: None,
    };
    (IdleLogind::spawn(options, sink).unwrap(), events)
}

fn wait_state(events: &Events<LogindEvent>, what: &str, pred: impl Fn(&nuxut_shell::services::idle::LogindState) -> bool) {
    events.wait_for(what, |e| matches!(e, LogindEvent::State(s) if pred(s)));
}

#[test]
fn sleep_delay_signals_and_actions() {
    let Some(bus) = PrivateBus::start() else { return };
    let fake = serve(&bus);
    let (service, events) = start(&bus);
    wait_state(&events, "ilk okuma", |s| {
        s.available && s.delay_max == Duration::from_secs(2) && !s.idle_inhibited && !s.locked_hint
    });
    assert!(fake.calls().is_empty(), "istenmeden inhibitör alınmaz: {:?}", fake.calls());
    // İstenmeden IdleHint yazılmaz (başka bir boşta kalma aracının ipucu ezilmez); istenince yazılır.
    std::thread::sleep(Duration::from_millis(200));
    assert!(fake.hints().is_empty(), "istenmeden IdleHint yazıldı: {:?}", fake.hints());
    service.set_idle_hint(true);
    wait_until("IdleHint true", || fake.hints() == ["true"]);

    // Gecikme inhibitörü: al, PrepareForSleep, kilit kuruldu, bırak (fd kapanır).
    service.set_sleep_delay(true);
    events.wait_for("inhibitör alındı", |e| *e == LogindEvent::SleepDelayHeld(true));
    assert_eq!(fake.calls(), ["Inhibit(sleep,delay)"]);
    let released = fake.last_inhibitor();
    fake.prepare_for_sleep(true);
    events.wait_for("uyku başlıyor", |e| *e == LogindEvent::PrepareForSleep(true));
    fake.set_locked(true);
    wait_state(&events, "LockedHint", |s| s.locked_hint);
    assert!(released.recv_timeout(Duration::from_millis(100)).is_err(), "istenmeden bırakılmaz");
    service.set_sleep_delay(false);
    events.wait_for("inhibitör bırakıldı", |e| *e == LogindEvent::SleepDelayHeld(false));
    released.recv_timeout(Duration::from_secs(5)).expect("fd kapanmadı: logind uykuyu bekletmeye devam ederdi");
    fake.prepare_for_sleep(false);
    events.wait_for("uyanış", |e| *e == LogindEvent::PrepareForSleep(false));

    // idle blok inhibitörü.
    fake.set_block("sleep:idle:handle-lid-switch");
    wait_state(&events, "idle engeli", |s| s.idle_inhibited);
    fake.set_block("sleep");
    wait_state(&events, "idle engeli kalktı", |s| !s.idle_inhibited);

    // Eylemler oturum nesnesine / yöneticiye gider.
    service.lock();
    service.suspend();
    wait_until("kilit ve uyku çağrıldı", || fake.calls().len() == 3);
    assert_eq!(fake.calls()[1..], ["Lock".to_owned(), "Suspend(false)".to_owned()]);

    let t = std::time::Instant::now();
    drop(service);
    assert!(t.elapsed() < Duration::from_secs(1), "temiz ve hızlı kapanış");
}

#[test]
fn spoofed_prepare_for_sleep_is_ignored() {
    let Some(bus) = PrivateBus::start() else { return };
    let _fake = serve(&bus);
    let (_service, events) = start(&bus);
    wait_state(&events, "ilk okuma", |s| s.available);
    let attacker = bus.connect();
    let names: Vec<String> = attacker
        .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    for name in names.iter().filter(|n| n.starts_with(':')) {
        let _ = attacker.emit_signal(Some(name.as_str()), LOGIN1_PATH, MANAGER, "PrepareForSleep", &(true,));
    }
    let spoofed = events.collect(Duration::from_millis(300)).into_iter().any(|e| e == LogindEvent::PrepareForSleep(true));
    assert!(!spoofed, "sahte sinyal kabul edildi");
}

#[test]
fn logind_restart_reacquires_requested_delay() {
    let Some(bus) = PrivateBus::start() else { return };
    let fake = serve(&bus);
    let (service, events) = start(&bus);
    wait_state(&events, "ilk okuma", |s| s.available);
    service.set_sleep_delay(true);
    service.set_idle_hint(true);
    events.wait_for("inhibitör alındı", |e| *e == LogindEvent::SleepDelayHeld(true));
    wait_until("IdleHint true", || fake.hints().last().is_some_and(|h| h == "true"));
    let released = fake.last_inhibitor();

    drop(fake);
    events.wait_for("inhibitör düştü", |e| *e == LogindEvent::SleepDelayHeld(false));
    wait_state(&events, "logind gitti", |s| !s.available);
    released.recv_timeout(Duration::from_secs(5)).expect("eski fd kapanmadı");

    let fake = serve(&bus);
    // Servis inhibitörü durumu yayınlamadan önce alır: olay sırası buna göre.
    events.wait_for("inhibitör yeniden alındı", |e| *e == LogindEvent::SleepDelayHeld(true));
    wait_state(&events, "logind geldi", |s| s.available);
    assert_eq!(fake.calls(), ["Inhibit(sleep,delay)"]);
    wait_until("IdleHint yeniden yazıldı", || fake.hints() == ["true"]);
}

#[test]
fn missing_logind_fails_actions_without_panicking() {
    let options = LogindOptions {
        bus_address: Some("unix:path=/nonexistent/nuxut-test-bus".into()),
        call_timeout: Duration::from_millis(200),
        max_backoff: Duration::from_millis(100),
        pid: 1,
        compositor_pid: None,
    };
    let (sink, events) = channel();
    let service = IdleLogind::spawn(options, sink).unwrap();
    service.lock();
    events.wait_for("kilit başarısız", |e| matches!(e, LogindEvent::Failed { .. }));
}

#[test]
fn session_lock_signal_only_from_logind_for_our_session() {
    const SESSION: &str = "org.freedesktop.login1.Session";
    let Some(bus) = PrivateBus::start() else { return };
    let fake = serve(&bus);
    let (_service, events) = start(&bus);
    wait_state(&events, "oturum çözüldü", |s| s.available && s.has_session);
    let emit = |conn: &zbus::blocking::Connection, dest: Option<&str>, path: &str, member: &str| {
        let _ = conn.emit_signal(dest, path, SESSION, member, &());
    };
    // logind'den ama başka oturumun kilidi (`loginctl lock-sessions`) ve Unlock: iletilmez.
    emit(&fake.conn, None, "/org/freedesktop/login1/session/c8", "Lock");
    emit(&fake.conn, None, SESSION_PATH, "Unlock");
    // Sahte gönderici: yayın ve her benzersiz ada tek hedefli.
    let attacker = bus.connect();
    emit(&attacker, None, SESSION_PATH, "Lock");
    let names: Vec<String> = attacker
        .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    for name in names.iter().filter(|n| n.starts_with(':')) {
        emit(&attacker, Some(name.as_str()), SESSION_PATH, "Lock");
    }
    let leaked = events.collect(Duration::from_millis(300)).into_iter().any(|e| e == LogindEvent::LockRequested);
    assert!(!leaked, "başka oturumun, Unlock'un ya da sahte göndericinin sinyali kilit isteği sayıldı");
    // Kendi oturumumuzun logind sinyali iletilir (Kilitle → Session.Lock → bu sinyal → nuxut-lock).
    emit(&fake.conn, None, SESSION_PATH, "Lock");
    events.wait_for("kilit isteği", |e| *e == LogindEvent::LockRequested);
}

/// Sahte `nuxut-lock`: her çalışmada `calls`'a satır ekler, `gate` silinene dek açık kalır.
fn fake_lock(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("nuxut-lock");
    let script = format!(
        "#!/bin/sh\necho run >> '{0}/calls'\nwhile [ -e '{0}/gate' ]; do sleep 0.02; done\n",
        dir.display()
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("gate"), "").unwrap();
    path
}

fn lock_runs(dir: &std::path::Path) -> usize {
    std::fs::read_to_string(dir.join("calls")).map(|s| s.lines().count()).unwrap_or(0)
}

/// Kabuğun `app/idle.rs` + `app/session.rs` bağlantısı: olay → `decide` → `LockLauncher`.
fn route(launcher: &LockLauncher, event: &LogindEvent) {
    let trigger = match event {
        LogindEvent::LockRequested => LockTrigger::LogindSignal,
        LogindEvent::Failed { action: LogindAction::Lock, .. } => LockTrigger::LockFailed,
        _ => return,
    };
    if decide(trigger, LockConfig::default().listen_logind) {
        launcher.launch().unwrap();
    }
}

#[test]
fn lock_signal_starts_nuxut_lock_once() {
    const SESSION: &str = "org.freedesktop.login1.Session";
    let Some(bus) = PrivateBus::start() else { return };
    let fake = serve(&bus);
    let (_service, events) = start(&bus);
    wait_state(&events, "oturum çözüldü", |s| s.available && s.has_session);
    let dir = tempfile::tempdir().unwrap();
    let launcher = LockLauncher::new(Some(fake_lock(dir.path())));
    // Kilitle iki kez (çift tık, SUPER+L + düğme): kilit açıkken ikinci süreç yok.
    for _ in 0..2 {
        fake.conn.emit_signal(None::<&str>, SESSION_PATH, SESSION, "Lock", &()).unwrap();
        let e = events.wait_for("kilit isteği", |e| *e == LogindEvent::LockRequested);
        route(&launcher, &e);
    }
    wait_until("nuxut-lock çalıştı", || lock_runs(dir.path()) == 1);
    assert!(launcher.is_running());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(lock_runs(dir.path()), 1, "çift kilit başlatıldı");
    std::fs::remove_file(dir.path().join("gate")).unwrap();
    wait_until("kilit kapandı", || !launcher.is_running());
}

#[test]
fn missing_logind_starts_nuxut_lock_directly() {
    let options = LogindOptions {
        bus_address: Some("unix:path=/nonexistent/nuxut-test-bus".into()),
        call_timeout: Duration::from_millis(200),
        max_backoff: Duration::from_millis(100),
        pid: 1,
        compositor_pid: None,
    };
    let (sink, events) = channel();
    let service = IdleLogind::spawn(options, sink).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let launcher = LockLauncher::new(Some(fake_lock(dir.path())));
    service.lock();
    let e = events.wait_for("kilit başarısız", |e| matches!(e, LogindEvent::Failed { action: LogindAction::Lock, .. }));
    route(&launcher, &e);
    wait_until("nuxut-lock doğrudan çalıştı", || lock_runs(dir.path()) == 1);
    std::fs::remove_file(dir.path().join("gate")).unwrap();
    wait_until("kilit kapandı", || !launcher.is_running());
}

/// Kabuğun `app/idle.rs` bağlantısı: makinenin etkileri servise (yalnızca kilit ve gecikme).
fn apply(service: &IdleLogind, fx: Vec<Effect>) {
    for e in fx {
        match e {
            Effect::SleepDelay(hold) => {
                let _ = service.set_sleep_delay(hold);
            }
            Effect::Lock => {
                let _ = service.lock();
            }
            _ => {}
        }
    }
}

/// Varsayılan ayarlar (uykudan önce kilit AÇIK) ile makine + servis + sahte logind.
fn sleep_rig(bus: &PrivateBus) -> (Fake, IdleLogind, Events<LogindEvent>, IdleMachine) {
    let fake = serve(bus);
    let (service, events) = start(bus);
    wait_state(&events, "ilk okuma", |s| s.available && s.has_session);
    let mut machine = IdleMachine::new(IdleConfig::default().settings());
    machine.set_delay_max(Duration::from_secs(2));
    apply(&service, machine.start());
    events.wait_for("varsayılan: gecikme inhibitörü alındı", |e| *e == LogindEvent::SleepDelayHeld(true));
    (fake, service, events, machine)
}

#[test]
fn default_sleep_waits_for_lock_then_releases() {
    let Some(bus) = PrivateBus::start() else { return };
    let (fake, service, events, mut machine) = sleep_rig(&bus);
    let released = fake.last_inhibitor();
    fake.prepare_for_sleep(true);
    events.wait_for("uyku başlıyor", |e| *e == LogindEvent::PrepareForSleep(true));
    apply(&service, machine.prepare_for_sleep(true, std::time::Instant::now()));
    // Sıra: önce kilit istenir, uyku kilit kurulana dek bekler.
    wait_until("Session.Lock çağrıldı", || fake.calls().last().is_some_and(|c| c == "Lock"));
    assert!(released.recv_timeout(Duration::from_millis(300)).is_err(), "kilit kurulmadan uyku bırakıldı");
    // nuxut-lock sunum kapısından sonra LockedHint yazar → inhibitör bırakılır.
    fake.set_locked(true);
    wait_state(&events, "LockedHint", |s| s.locked_hint);
    apply(&service, machine.set_locked(true));
    released.recv_timeout(Duration::from_secs(5)).expect("kilit kuruldu ama uyku bekletilmeye devam ediyor");
    // Uyanış: kilit kurulmuştu, yeniden istenmez; inhibitör yeniden alınır.
    fake.prepare_for_sleep(false);
    events.wait_for("uyanış", |e| *e == LogindEvent::PrepareForSleep(false));
    let fx = machine.prepare_for_sleep(false, std::time::Instant::now());
    assert!(!fx.contains(&Effect::Lock), "{fx:?}");
    apply(&service, fx);
    events.wait_for("inhibitör yeniden alındı", |e| *e == LogindEvent::SleepDelayHeld(true));
    assert_eq!(fake.calls().iter().filter(|c| *c == "Lock").count(), 1);
}

#[test]
fn sleep_lock_timeout_releases_and_wake_relocks() {
    let Some(bus) = PrivateBus::start() else { return };
    let (fake, service, events, mut machine) = sleep_rig(&bus);
    let released = fake.last_inhibitor();
    fake.prepare_for_sleep(true);
    events.wait_for("uyku başlıyor", |e| *e == LogindEvent::PrepareForSleep(true));
    let t0 = std::time::Instant::now();
    apply(&service, machine.prepare_for_sleep(true, t0));
    assert_eq!(machine.deadline(), Some(t0 + Duration::from_secs(2)), "InhibitDelayMaxUSec");
    // Kilit hiç kurulmuyor (LockedHint gelmiyor): süre dolunca uyku yine de bırakılır.
    assert!(machine.tick(t0 + Duration::from_secs(1)).is_empty());
    apply(&service, machine.tick(t0 + Duration::from_secs(2)));
    released.recv_timeout(Duration::from_secs(5)).expect("zaman aşımında inhibitör bırakılmadı");
    // Uyanış: masaüstü açık kalmasın, kilit yeniden istenir.
    fake.prepare_for_sleep(false);
    events.wait_for("uyanış", |e| *e == LogindEvent::PrepareForSleep(false));
    apply(&service, machine.prepare_for_sleep(false, t0 + Duration::from_secs(60)));
    wait_until("uyanışta ikinci Session.Lock", || fake.calls().iter().filter(|c| *c == "Lock").count() == 2);
}
