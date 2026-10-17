//! Sistem servisi testleri için ortak yardımcılar: özel `dbus-daemon` ve olay bekleme.
//! Her test kendi bus'ını başlatır; kullanıcının sistem/oturum bus'ına dokunulmaz.

#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

pub mod repo;
pub mod skip;
pub mod udisks;

/// Olay/koşul beklemelerinin üst sınırı: yalnız güvenlik ağı (koşul gelince hemen döner). Yük
/// altında (paralel derleme + test, özel PipeWire/WirePlumber/D-Bus açılışı) 5 sn yetmiyordu
/// (audio_service, 2026-10-15); başarılı koşu süresi değişmez.
pub const WAIT: Duration = Duration::from_secs(30);

pub struct PrivateBus {
    child: Child,
    pub address: String,
}

impl PrivateBus {
    /// Özel `dbus-daemon` başlatır. Başlatılamazsa panikler; yalnızca
    /// `NUXUT_SKIP_DBUS_TESTS=1` ise uyarıp `None` döner (test atlanır).
    pub fn start() -> Option<Self> {
        let mut child = match Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
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
        Some(Self { child, address })
    }

    pub fn connect(&self) -> zbus::blocking::Connection {
        zbus::blocking::connection::Builder::address(self.address.as_str()).unwrap().build().unwrap()
    }

    /// Ad sahiplenen bir bağlantı (sahte servis).
    pub fn serve(&self, name: &str) -> zbus::blocking::Connection {
        let conn = self.connect();
        conn.request_name(name.to_owned()).unwrap();
        conn
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Servis olaylarını toplayan alıcı.
pub struct Events<E> {
    rx: Receiver<E>,
}

pub fn channel<E: Send + 'static>() -> (impl Fn(E) + Send + 'static, Events<E>) {
    let (tx, rx): (Sender<E>, Receiver<E>) = mpsc::channel();
    (
        move |e| {
            let _ = tx.send(e);
        },
        Events { rx },
    )
}

impl<E: std::fmt::Debug> Events<E> {
    /// `pred` tutan ilk olayı bekler.
    pub fn wait_for(&self, what: &str, mut pred: impl FnMut(&E) -> bool) -> E {
        let deadline = Instant::now() + WAIT;
        let mut seen = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(e) if pred(&e) => return e,
                Ok(e) => seen.push(format!("{e:?}")),
                Err(_) => panic!("beklenen olay gelmedi: {what}\ngelenler: {seen:#?}"),
            }
        }
    }

    /// `dur` boyunca gelen olaylar.
    pub fn collect(&self, dur: Duration) -> Vec<E> {
        let deadline = Instant::now() + dur;
        let mut out = Vec::new();
        while let Ok(e) = self.rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            out.push(e);
        }
        out
    }
}

/// Bir koşul tutana kadar bekler (sahte servisin aldığı çağrılar için).
pub fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("koşul gerçekleşmedi: {what}");
}
