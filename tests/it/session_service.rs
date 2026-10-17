//! Oturum servisi: sahte `org.freedesktop.login1` ile uçtan uca. Test kendi özel
//! `dbus-daemon`'ını başlatır; kullanıcının sistem/oturum bus'ına dokunulmaz.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::support::skip;

use nuxut_shell::services::session::{
    Capability, FailureReason, SessionAction, SessionEvent, SessionOptions, SessionService,
};
use zbus::zvariant::OwnedObjectPath;

const SESSION_PATH: &str = "/org/freedesktop/login1/session/_32";
const AUTO_PATH: &str = "/org/freedesktop/login1/session/auto";
const SESSION_ID: &str = "2";
const PID: u32 = 4242;

type Calls = Arc<Mutex<Vec<String>>>;

struct FakeManager {
    calls: Calls,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeManager {
    fn can_suspend(&self) -> String {
        "yes".into()
    }
    fn can_reboot(&self) -> String {
        "challenge".into()
    }
    fn can_power_off(&self) -> String {
        "no".into()
    }
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, pid: u32) -> zbus::fdo::Result<OwnedObjectPath> {
        if pid != PID {
            return Err(zbus::fdo::Error::Failed(format!("PID {pid} oturumu yok")));
        }
        Ok(OwnedObjectPath::try_from(SESSION_PATH).unwrap())
    }
    fn get_session(&self, id: String) -> zbus::fdo::Result<OwnedObjectPath> {
        if id != SESSION_ID {
            return Err(zbus::fdo::Error::Failed(format!("{id} oturumu yok")));
        }
        Ok(OwnedObjectPath::try_from(SESSION_PATH).unwrap())
    }
    fn suspend(&self, interactive: bool) {
        self.calls.lock().unwrap().push(format!("Suspend({interactive})"));
    }
    fn reboot(&self, _interactive: bool) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::AccessDenied("yetki yok".into()))
    }
}

struct FakeSession {
    calls: Calls,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl FakeSession {
    fn lock(&self) {
        self.calls.lock().unwrap().push("Lock".into());
    }
    fn terminate(&self) {
        self.calls.lock().unwrap().push("Terminate".into());
    }
}

/// `session/auto`: logind'de çağıranın ya da kullanıcının görüntü oturumu; yalnız `Id` okunur.
struct FakeAutoSession;

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl FakeAutoSession {
    #[zbus(property)]
    fn id(&self) -> String {
        SESSION_ID.into()
    }
}

/// Yalnızca bu test için `dbus-daemon` (servis etkinleştirme yok, her şeye izin).
struct PrivateBus {
    child: Child,
    address: String,
    _dir: tempfile::TempDir,
}

impl PrivateBus {
    /// Başlatılamazsa panikler; yalnızca `NUXUT_SKIP_DBUS_TESTS=1` ise `None` (atla).
    fn start() -> Option<Self> {
        let dir = tempfile::tempdir().expect("geçici dizin oluşturulamadı");
        let config = dir.path().join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
                dir.path().display()
            ),
        )
        .expect("bus.conf yazılamadı");
        let mut child = match Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return skip::skip_or_panic("dbus-daemon", skip::SKIP_DBUS, e),
        };
        let mut line = String::new();
        let _ = BufReader::new(child.stdout.take().unwrap()).read_line(&mut line);
        let address = line.trim().to_owned();
        if address.is_empty() {
            let _ = child.kill();
            let _ = child.wait();
            return skip::skip_or_panic("dbus-daemon", skip::SKIP_DBUS, "adres yazmadı");
        }
        Some(Self { child, address, _dir: dir })
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn session_actions_go_to_logind() {
    let Some(bus) = PrivateBus::start() else { return };
    let address = bus.address.clone();
    let calls: Calls = Arc::default();
    let _server = zbus::blocking::connection::Builder::address(address.as_str())
        .unwrap()
        .name("org.freedesktop.login1")
        .unwrap()
        .serve_at("/org/freedesktop/login1", FakeManager { calls: calls.clone() })
        .unwrap()
        .serve_at(SESSION_PATH, FakeSession { calls: calls.clone() })
        .unwrap()
        .build()
        .unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let options = SessionOptions { bus_address: Some(address), call_timeout: Duration::from_secs(2), pid: PID, compositor_pid: None };
    let service = SessionService::spawn(options, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    let wait = || rx.recv_timeout(Duration::from_secs(5)).expect("olay gelmedi");

    let SessionEvent::Capabilities(caps) = wait() else { panic!("önce yetenekler gelmeli") };
    let cap = |a| caps.iter().find(|(x, _)| *x == a).unwrap().1;
    assert_eq!(cap(SessionAction::Suspend), Capability::Yes);
    assert_eq!(cap(SessionAction::Reboot), Capability::Challenge);
    assert_eq!(cap(SessionAction::PowerOff), Capability::No);
    assert_eq!(cap(SessionAction::Lock), Capability::Yes);

    service.perform(SessionAction::Lock);
    assert_eq!(wait(), SessionEvent::Done(SessionAction::Lock));
    service.perform(SessionAction::Suspend);
    assert_eq!(wait(), SessionEvent::Done(SessionAction::Suspend));
    service.perform(SessionAction::Logout);
    assert_eq!(wait(), SessionEvent::Done(SessionAction::Logout));
    service.perform(SessionAction::Reboot);
    match wait() {
        SessionEvent::Failed { action, reason } => {
            assert_eq!(action, SessionAction::Reboot);
            assert_eq!(reason, FailureReason::Denied);
        }
        other => panic!("başarısızlık beklenirdi: {other:?}"),
    }
    assert_eq!(*calls.lock().unwrap(), ["Lock", "Suspend(false)", "Terminate"]);

    let t = std::time::Instant::now();
    drop(service);
    assert!(t.elapsed() < crate::support::PROMPT, "temiz ve hızlı kapanış: {:?}", t.elapsed());
}

#[test]
fn missing_logind_disables_actions_without_panicking() {
    let (tx, rx) = std::sync::mpsc::channel();
    let options = SessionOptions {
        bus_address: Some("unix:path=/nonexistent/nuxut-test-bus".into()),
        call_timeout: Duration::from_millis(200),
        pid: PID,
        compositor_pid: None,
    };
    let service = SessionService::spawn(options, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    let SessionEvent::Capabilities(caps) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else { panic!() };
    assert!(caps.iter().all(|(_, c)| *c == Capability::No));
    service.perform(SessionAction::PowerOff);
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        SessionEvent::Failed { reason: FailureReason::Unreachable, .. }
    ));
}

/// Kabuk oturum kapsamı dışında (systemd kullanıcı birimi, terminalin scope'undan `setsid`):
/// `GetSessionByPID` reddeder, oturum `session/auto`'nun kimliğinden bulunur. 2026-10-14
/// kullanıcı hatası: kilitle ve oturumu kapat "oturum bulunamadı" veriyordu.
#[test]
fn session_outside_scope_falls_back_to_auto() {
    let Some(bus) = PrivateBus::start() else { return };
    let address = bus.address.clone();
    let calls: Calls = Arc::default();
    let _server = zbus::blocking::connection::Builder::address(address.as_str())
        .unwrap()
        .name("org.freedesktop.login1")
        .unwrap()
        .serve_at("/org/freedesktop/login1", FakeManager { calls: calls.clone() })
        .unwrap()
        .serve_at(SESSION_PATH, FakeSession { calls: calls.clone() })
        .unwrap()
        .serve_at(AUTO_PATH, FakeAutoSession)
        .unwrap()
        .build()
        .unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let options = SessionOptions { bus_address: Some(address), call_timeout: Duration::from_secs(2), pid: PID + 1, compositor_pid: None };
    let service = SessionService::spawn(options, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    let wait = || rx.recv_timeout(Duration::from_secs(5)).expect("olay gelmedi");
    assert!(matches!(wait(), SessionEvent::Capabilities(_)));

    service.perform(SessionAction::Lock);
    assert_eq!(wait(), SessionEvent::Done(SessionAction::Lock));
    service.perform(SessionAction::Logout);
    assert_eq!(wait(), SessionEvent::Done(SessionAction::Logout));
    assert_eq!(*calls.lock().unwrap(), ["Lock", "Terminate"]);
}

/// systemd kullanıcı servisi olarak çalışan kabuk (tty'den `start-hyprland`, 2026-10-16): kendi
/// PID'i oturumda değil; oturum compositor'ın PID'inden (`hyprland.lock`) çözülür. `session/auto`
/// hiç sunulmaz: birden çok oturumda (ör. ssh) yanlış oturumu seçebileceği için son çaredir.
#[test]
fn session_outside_scope_resolves_via_compositor_pid() {
    let Some(bus) = PrivateBus::start() else { return };
    let address = bus.address.clone();
    let calls: Calls = Arc::default();
    let _server = zbus::blocking::connection::Builder::address(address.as_str())
        .unwrap()
        .name("org.freedesktop.login1")
        .unwrap()
        .serve_at("/org/freedesktop/login1", FakeManager { calls: calls.clone() })
        .unwrap()
        .serve_at(SESSION_PATH, FakeSession { calls: calls.clone() })
        .unwrap()
        .build()
        .unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let options =
        SessionOptions { bus_address: Some(address), call_timeout: Duration::from_secs(2), pid: PID + 1, compositor_pid: Some(PID) };
    let service = SessionService::spawn(options, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    let wait = || rx.recv_timeout(Duration::from_secs(5)).expect("olay gelmedi");
    assert!(matches!(wait(), SessionEvent::Capabilities(_)));

    service.perform(SessionAction::Lock);
    assert_eq!(wait(), SessionEvent::Done(SessionAction::Lock));
    assert_eq!(*calls.lock().unwrap(), ["Lock"]);
}

/// `nuxut-shell doctor`: oturum çözümünün yolu (yalnız okur). Kendi PID'i → `ByPid`, oturum
/// dışı PID → `session/auto` yedeği `Auto`; auto da yoksa ya da logind'e ulaşılamazsa `Missing`.
#[test]
fn session_resolution_reports_the_path() {
    use nuxut_shell::services::session::{SessionResolution, session_resolution};
    let Some(bus) = PrivateBus::start() else { return };
    let address = bus.address.clone();
    let calls: Calls = Arc::default();
    let server = zbus::blocking::connection::Builder::address(address.as_str())
        .unwrap()
        .name("org.freedesktop.login1")
        .unwrap()
        .serve_at("/org/freedesktop/login1", FakeManager { calls: calls.clone() })
        .unwrap()
        .serve_at(SESSION_PATH, FakeSession { calls: calls.clone() })
        .unwrap()
        .serve_at(AUTO_PATH, FakeAutoSession)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(session_resolution(Some(&address), PID, None), SessionResolution::ByPid);
    assert_eq!(session_resolution(Some(&address), PID + 1, None), SessionResolution::Auto);
    assert_eq!(session_resolution(Some(&address), PID + 1, Some(PID)), SessionResolution::ByPid, "compositor'ın PID'i");
    assert!(calls.lock().unwrap().is_empty(), "yalnız okur");
    // `session/auto` yoksa (oturumsuz kullanıcı) çözülemez.
    server.object_server().remove::<FakeAutoSession, _>(AUTO_PATH).unwrap();
    assert_eq!(session_resolution(Some(&address), PID + 1, None), SessionResolution::Missing);
    drop(server);
    drop(bus);
    assert_eq!(session_resolution(Some(&address), PID, None), SessionResolution::Missing, "logind yok");
}
