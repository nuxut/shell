//! logind `Session.SetLockedHint`: kilit durumunu oturuma bildirir (shell, hypridle ve diğer
//! dinleyiciler `LockedHint` özelliğini izler). Yalnızca bilgi amaçlıdır; kilidin güvenliği buna
//! dayanmaz. Oturum nesnesi `session/auto`: çağıranın oturumu, değilse kullanıcının grafik
//! oturumu (logind'in çözümü).

use std::time::Duration;

pub const LOGIN1: &str = "org.freedesktop.login1";
pub const SESSION_AUTO: &str = "/org/freedesktop/login1/session/auto";
pub const SESSION: &str = "org.freedesktop.login1.Session";

/// Bloklar (en çok `timeout`); arka plan thread'inden çağrılır.
pub fn set_locked_hint(
    bus_address: Option<&str>,
    locked: bool,
    timeout: Duration,
) -> zbus::Result<()> {
    let builder = match bus_address {
        Some(addr) => zbus::blocking::connection::Builder::address(addr)?,
        None => zbus::blocking::connection::Builder::system()?,
    };
    let conn = builder.method_timeout(timeout).build()?;
    let session = zbus::blocking::Proxy::new(&conn, LOGIN1, SESSION_AUTO, SESSION)?;
    session.call::<_, _, ()>("SetLockedHint", &(locked,))
}

/// Sıralı `SetLockedHint` işçisi: çağrılar gönderildiği sırayla yapılır (hızlı açmada `false`
/// `true`'yu geçemez). Hata loglanır; kilidin güvenliği buna dayanmaz.
#[derive(Debug)]
pub struct HintWorker {
    tx: std::sync::mpsc::Sender<(bool, Option<std::sync::mpsc::Sender<()>>)>,
}

impl HintWorker {
    pub fn spawn(bus_address: Option<String>, timeout: Duration) -> std::io::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<(bool, Option<std::sync::mpsc::Sender<()>>)>();
        std::thread::Builder::new()
            .name("nuxut-logind".into())
            .spawn(move || {
                for (locked, done) in rx {
                    if let Err(e) = set_locked_hint(bus_address.as_deref(), locked, timeout) {
                        tracing::warn!("logind SetLockedHint({locked}) başarısız: {e}");
                    }
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                }
            })?;
        Ok(Self { tx })
    }

    pub fn set(&self, locked: bool) {
        let _ = self.tx.send((locked, None));
    }

    /// Öncekiler dahil bu çağrının bitmesini en çok `wait` bekler (çıkıştan hemen önce).
    pub fn set_and_wait(&self, locked: bool, wait: Duration) {
        let (done, rx) = std::sync::mpsc::channel();
        if self.tx.send((locked, Some(done))).is_ok() {
            let _ = rx.recv_timeout(wait);
        }
    }
}

/// [`set_locked_hint`]'i ayrı thread'de çalıştırır; hata loglanır, beklenmez.
pub fn set_locked_hint_async(locked: bool) {
    let spawned = std::thread::Builder::new()
        .name("nuxut-logind".into())
        .spawn(move || {
            if let Err(e) = set_locked_hint(None, locked, Duration::from_secs(2)) {
                tracing::warn!("logind SetLockedHint({locked}) başarısız: {e}");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("logind thread'i başlatılamadı: {e}");
    }
}
