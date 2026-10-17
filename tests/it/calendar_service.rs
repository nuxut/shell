//! Takvim istemcisi: sahte `org.nuxut.Calendar1` ile uçtan uca. Test kendi özel
//! `dbus-daemon`'ını başlatır; kullanıcının oturum bus'ına dokunulmaz.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::support::skip;

use nuxut_calendar_ipc::OccurrenceEntry;
use nuxut_shell::services::calendar::{CalendarEvent, CalendarOptions, CalendarService};

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

#[derive(Clone, Default)]
struct State {
    summary: Arc<Mutex<String>>,
    queries: Arc<Mutex<Vec<(i64, i64)>>>,
}

struct FakeCalendar {
    state: State,
}

#[zbus::interface(name = "org.nuxut.Calendar1")]
impl FakeCalendar {
    fn get_occurrences(&self, from: i64, to: i64) -> Vec<OccurrenceEntry> {
        self.state.queries.lock().unwrap().push((from, to));
        let summary = self.state.summary.lock().unwrap().clone();
        vec![
            OccurrenceEntry { start: from + 7200, end: from + 10800, summary: "ikinci".into(), ..Default::default() },
            OccurrenceEntry { start: from + 3600, end: from + 7200, summary, color: "#336699".into(), ..Default::default() },
        ]
    }

    #[zbus(signal)]
    async fn occurrences_changed(emitter: &zbus::object_server::SignalEmitter<'_>, from: i64, to: i64) -> zbus::Result<()>;
}

const WAIT: Duration = Duration::from_secs(5);

#[test]
fn queries_range_and_requeries_on_change_signal() {
    let Some(bus) = PrivateBus::start() else { return };
    let state = State::default();
    *state.summary.lock().unwrap() = "toplantı".into();
    let server = zbus::blocking::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("org.nuxut.Calendar1")
        .unwrap()
        .serve_at("/org/nuxut/Calendar1", FakeCalendar { state: state.clone() })
        .unwrap()
        .build()
        .unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let svc = CalendarService::spawn(CalendarOptions { bus_address: Some(bus.address.clone()) }, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();

    svc.set_range(1000, 1000 + 42 * 86_400);
    let CalendarEvent::Occurrences { from, entries, .. } = rx.recv_timeout(WAIT).unwrap() else { panic!("oluşumlar gelmeli") };
    assert_eq!(from, 1000);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].summary, "toplantı", "başlangıca göre sıralı");
    assert_eq!(entries[0].color, "#336699");

    // Daemon değişiklik bildirir: patlama tek sorguya iner.
    *state.summary.lock().unwrap() = "ertelendi".into();
    let iface = server
        .object_server()
        .interface::<_, FakeCalendar>("/org/nuxut/Calendar1")
        .unwrap();
    for _ in 0..5 {
        zbus::block_on(FakeCalendar::occurrences_changed(iface.signal_emitter(), 0, 1)).unwrap();
    }
    let CalendarEvent::Occurrences { entries, .. } = rx.recv_timeout(WAIT).unwrap() else { panic!() };
    assert_eq!(entries[0].summary, "ertelendi");
    std::thread::sleep(Duration::from_millis(400));
    assert!(rx.try_recv().is_err(), "beş sinyal tek yeniden sorgu olmalı");
    assert_eq!(state.queries.lock().unwrap().len(), 2);

    // Daemon gidince (ad bırakıldı) sorgu hatası → Unavailable.
    server.release_name("org.nuxut.Calendar1").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    while rx.try_recv().is_ok() {} // NameOwnerChanged kaynaklı yeniden sorgu sonucu
    svc.set_range(0, 86_400);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), CalendarEvent::Unavailable);
}

#[test]
fn ignores_change_signals_from_non_owner() {
    let Some(bus) = PrivateBus::start() else { return };
    let state = State::default();
    let _server = zbus::blocking::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name("org.nuxut.Calendar1")
        .unwrap()
        .serve_at("/org/nuxut/Calendar1", FakeCalendar { state: state.clone() })
        .unwrap()
        .build()
        .unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let svc = CalendarService::spawn(CalendarOptions { bus_address: Some(bus.address.clone()) }, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    svc.set_range(0, 86_400);
    assert!(matches!(rx.recv_timeout(WAIT).unwrap(), CalendarEvent::Occurrences { .. }));

    // Adı tutmayan bir bağlantı daemon'u taklit eder: yeniden sorgu tetiklenmemeli.
    let spoofer = zbus::blocking::connection::Builder::address(bus.address.as_str()).unwrap().build().unwrap();
    for _ in 0..3 {
        spoofer
            .emit_signal(None::<&str>, "/org/nuxut/Calendar1", "org.nuxut.Calendar1", "OccurrencesChanged", &(0i64, 1i64))
            .unwrap();
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(rx.try_recv().is_err(), "sahibi olmayan göndericinin sinyali yok sayılmalı");
    assert_eq!(state.queries.lock().unwrap().len(), 1);
}
