//! Sahte `org.freedesktop.login1` (yalnızca izole test ortamında görsel test için): oturum
//! kartının düğmelerini gerçek logind olmadan etkin çizdirir. Hiçbir eylem uygulanmaz: her
//! eylem çağrısı stderr'e `ÇAĞRI: <yöntem>` yazar ve hata döner (kart hatayı gösterir).
//! İstisna `Session.Lock`: gerçek logind gibi yalnızca oturumun `Lock` sinyalini yayınlar
//! (kabuğun kilit tetikleyicisinin canlı testi).
//!
//! ```sh
//! dbus-run-session -- sh betik.sh       # özel oturum bus'ı
//! cargo run --example fake_logind &     # o bus'ta login1 adını alır
//! DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" nuxut-shell
//! ```
//!
//! Güvenlik: yalnızca `DBUS_SESSION_BUS_ADDRESS` bus'ına bağlanır; sistem bus'ında login1 adını
//! alamaz (gerçek logind tutar), oturum bus'ı ise `dbus-run-session`'ın özel bus'ı olmalıdır.

use std::time::Duration;

use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;

const SESSION_PATH: &str = "/org/freedesktop/login1/session/fake";

fn refuse(method: &str) -> zbus::fdo::Error {
    eprintln!("ÇAĞRI: {method}");
    zbus::fdo::Error::AccessDenied("sahte logind: eylem uygulanmadı".into())
}

struct Manager;

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl Manager {
    fn can_suspend(&self) -> String {
        "yes".into()
    }
    fn can_reboot(&self) -> String {
        "yes".into()
    }
    fn can_power_off(&self) -> String {
        "yes".into()
    }
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, _pid: u32) -> zbus::fdo::Result<OwnedObjectPath> {
        OwnedObjectPath::try_from(SESSION_PATH).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }
    fn suspend(&self, _interactive: bool) -> zbus::fdo::Result<()> {
        Err(refuse("Suspend"))
    }
    fn reboot(&self, _interactive: bool) -> zbus::fdo::Result<()> {
        Err(refuse("Reboot"))
    }
    fn power_off(&self, _interactive: bool) -> zbus::fdo::Result<()> {
        Err(refuse("PowerOff"))
    }
}

struct Session;

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl Session {
    /// Gerçek logind gibi: kilit uygulanmaz, oturumun `Lock` sinyali yayınlanır (kabuk dinler ve
    /// nuxut-lock'u başlatır; yalnızca bu özel bus'ta).
    async fn lock(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> zbus::fdo::Result<()> {
        eprintln!("ÇAĞRI: Lock (sinyal yayınlandı)");
        Self::lock_signal(&emitter).await.map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }
    fn terminate(&self) -> zbus::fdo::Result<()> {
        Err(refuse("Terminate"))
    }
    #[zbus(signal, name = "Lock")]
    async fn lock_signal(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

fn main() -> zbus::Result<()> {
    let _conn = zbus::blocking::connection::Builder::session()?
        .name("org.freedesktop.login1")?
        .serve_at("/org/freedesktop/login1", Manager)?
        .serve_at(SESSION_PATH, Session)?
        .build()?;
    eprintln!("sahte logind hazır");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
