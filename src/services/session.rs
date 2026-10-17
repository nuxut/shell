//! Oturum: kullanıcı bilgisi (ad, avatar, açık kalma süresi) ve logind eylemleri
//! (kilitle, oturumu kapat, askıya al, yeniden başlat, kapat).
//!
//! Eylemler systemd-logind'e D-Bus ile gider (`org.freedesktop.login1`); hiçbir süreç
//! çalıştırılmaz. Oturum nesnesi kendi PID'imizden (`GetSessionByPID`), olmazsa `session/auto`'dan çözülür; kilit ve oturum
//! kapatma o nesne üzerinden (`Session.Lock` / `Session.Terminate`). Bloklayan çağrılar servis
//! thread'inde, zaman aşımıyla; sonuç ve yetenekler (`CanSuspend`…) callback ile döner.
//!
//! Kullanıcı bilgisi için saf fonksiyonlar (`/proc/uptime`, `/etc/passwd`, `/proc/self/status`)
//! ayrı ve test edilebilir; dosya okumaları boyut sınırlıdır.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use nuxut::i18n::t;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
/// Çağıranın oturumu; çağıran oturum kapsamında değilse kullanıcının görüntü oturumu (logind).
const SESSION_AUTO: &str = "/org/freedesktop/login1/session/auto";
/// Okunan sistem dosyalarının sınırı.
const MAX_FILE: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionAction {
    Lock,
    Logout,
    Suspend,
    Reboot,
    PowerOff,
}

impl SessionAction {
    pub const ALL: [SessionAction; 5] =
        [SessionAction::Lock, SessionAction::Logout, SessionAction::Suspend, SessionAction::Reboot, SessionAction::PowerOff];

    pub fn label(self) -> String {
        match self {
            SessionAction::Lock => t!("shell-session-action-lock"),
            SessionAction::Logout => t!("shell-session-action-logout"),
            SessionAction::Suspend => t!("shell-session-action-suspend"),
            SessionAction::Reboot => t!("shell-session-action-reboot"),
            SessionAction::PowerOff => t!("shell-session-action-poweroff"),
        }
    }

    /// Geri alınamaz mı (oturum kapanır / makine durur). Oturum kartında her eylem yerinde onay
    /// ister; geri alınamayanlar onayı uyarı renginde gösterir. Kilitle ve askıya al oturumu korur.
    pub fn is_destructive(self) -> bool {
        !matches!(self, SessionAction::Lock | SessionAction::Suspend)
    }

    /// logind'deki yetenek sorgusu.
    fn can_method(self) -> Option<&'static str> {
        match self {
            SessionAction::Suspend => Some("CanSuspend"),
            SessionAction::Reboot => Some("CanReboot"),
            SessionAction::PowerOff => Some("CanPowerOff"),
            SessionAction::Lock | SessionAction::Logout => None,
        }
    }
}

/// logind `Can*` cevabı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Capability {
    Yes,
    /// Yetki istenecek (polkit).
    Challenge,
    No,
    #[default]
    Unknown,
}

impl Capability {
    pub fn parse(s: &str) -> Self {
        match s {
            "yes" => Capability::Yes,
            "challenge" => Capability::Challenge,
            "no" | "na" => Capability::No,
            _ => Capability::Unknown,
        }
    }

    /// Düğme etkin mi? Bilinmiyorsa denemeye izin ver (logind reddederse hata gösterilir).
    pub fn allowed(self) -> bool {
        self != Capability::No
    }
}

/// Başarısız oturum eyleminin kullanıcıya gösterilen nedeni. Ham D-Bus ayrıntısı yalnızca loga
/// yazılır; kartta kısa, anlaşılır bir metin görünür.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// Yetki reddedildi (polkit / D-Bus `AccessDenied`).
    Denied,
    /// Etkileşimli yetki gerekiyor (polkit sorusu açmıyoruz: `interactive = false`).
    AuthRequired,
    /// Başka bir kapatma/askıya alma zaten sürüyor.
    Busy,
    /// Sistem bu eylemi desteklemiyor.
    Unsupported,
    /// Oturum nesnesi bulunamadı.
    NoSession,
    /// logind'e ulaşılamadı (bağlantı yok, hizmet yok).
    Unreachable,
    /// Zaman aşımı.
    Timeout,
    Other,
}

impl FailureReason {
    pub fn label(self) -> String {
        match self {
            FailureReason::Denied => t!("shell-session-error-denied"),
            FailureReason::AuthRequired => t!("shell-session-error-auth-required"),
            FailureReason::Busy => t!("shell-session-error-busy"),
            FailureReason::Unsupported => t!("shell-session-error-unsupported"),
            FailureReason::NoSession => t!("shell-session-error-no-session"),
            FailureReason::Unreachable => t!("shell-session-error-unreachable"),
            FailureReason::Timeout => t!("shell-session-error-timeout"),
            FailureReason::Other => t!("shell-session-error-other"),
        }
    }

    /// D-Bus hata adından neden (logind, polkit ve standart D-Bus adları).
    pub fn from_error_name(name: &str) -> Self {
        match name {
            "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.DBus.Error.AuthFailed"
            | "org.freedesktop.PolicyKit1.Error.NotAuthorized" => FailureReason::Denied,
            "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired" => FailureReason::AuthRequired,
            "org.freedesktop.login1.OperationInProgress" | "org.freedesktop.login1.ShutdownAlreadyInProgress" => {
                FailureReason::Busy
            }
            "org.freedesktop.login1.SleepVerbNotSupported"
            | "org.freedesktop.DBus.Error.NotSupported"
            | "org.freedesktop.DBus.Error.UnknownMethod"
            | "org.freedesktop.DBus.Error.UnknownInterface"
            | "org.freedesktop.DBus.Error.UnknownObject" => FailureReason::Unsupported,
            "org.freedesktop.login1.NoSuchSession" | "org.freedesktop.login1.NoSessionForPID" => {
                FailureReason::NoSession
            }
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.NoServer"
            | "org.freedesktop.DBus.Error.Disconnected" => FailureReason::Unreachable,
            "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout" | "org.freedesktop.DBus.Error.TimedOut" => {
                FailureReason::Timeout
            }
            _ => FailureReason::Other,
        }
    }

    /// zbus hatasından neden.
    pub fn from_zbus(err: &zbus::Error) -> Self {
        use zbus::DBusError;
        match err {
            zbus::Error::MethodError(name, _, _) => Self::from_error_name(name.as_str()),
            zbus::Error::FDO(e) => match e.as_ref() {
                zbus::fdo::Error::ZBus(inner) => Self::from_zbus(inner),
                e => Self::from_error_name(e.name().as_str()),
            },
            zbus::Error::InputOutput(e) if e.kind() == std::io::ErrorKind::TimedOut => FailureReason::Timeout,
            zbus::Error::InputOutput(_) | zbus::Error::Address(_) | zbus::Error::Handshake(_) => {
                FailureReason::Unreachable
            }
            _ => FailureReason::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// Başlangıç sorgusu: eylem başına yetenek (kilit/çıkış her zaman `Yes`).
    Capabilities(Vec<(SessionAction, Capability)>),
    /// Eylem logind'e iletildi.
    Done(SessionAction),
    /// Eylem başarısız; ham hata yalnızca loglanır.
    Failed { action: SessionAction, reason: FailureReason },
}

#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// D-Bus adresi; `None`: sistem bus'ı (testler sahte bir login1 adresi verir).
    pub bus_address: Option<String>,
    pub call_timeout: Duration,
    /// Oturumu çözmek için PID (testlerde sahte login1 bu PID'i tanır).
    pub pid: u32,
    /// Kendi PID'imiz oturumda değilse (systemd kullanıcı servisi) oturum compositor'ın PID'inden
    /// çözülür (`hyprland.lock`); `session/auto` yalnızca son çare (birden çok oturumda yanlış
    /// oturumu seçebilir, ör. ssh).
    pub compositor_pid: Option<u32>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            bus_address: None,
            call_timeout: Duration::from_secs(5),
            pid: std::process::id(),
            compositor_pid: crate::services::hyprland::compositor_pid(),
        }
    }
}

enum Command {
    Perform(SessionAction),
}

/// Çalışan oturum servisi. Düşürülünce kuyruk kapanır, thread join edilir.
#[derive(Debug)]
pub struct SessionService {
    tx: Option<mpsc::Sender<Command>>,
    thread: Option<JoinHandle<()>>,
}

impl SessionService {
    pub fn spawn<F>(options: SessionOptions, sink: F) -> std::io::Result<Self>
    where
        F: Fn(SessionEvent) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<Command>();
        let thread = std::thread::Builder::new()
            .name("nuxut-session".into())
            .spawn(move || worker(&options, &rx, &sink))?;
        Ok(Self { tx: Some(tx), thread: Some(thread) })
    }

    /// Eylemi kuyruğa koyar; sonucu `Done`/`Failed` olarak gelir. Bloklamaz.
    pub fn perform(&self, action: SessionAction) {
        if let Some(tx) = &self.tx
            && tx.send(Command::Perform(action)).is_err()
        {
            tracing::warn!("oturum servisi durmuş, {action:?} iletilemedi");
        }
    }
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::Perform(a) => write!(f, "Perform({a:?})"),
        }
    }
}

impl Drop for SessionService {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("oturum servisi thread'i panikle sonlandı");
        }
    }
}

fn connect(options: &SessionOptions) -> zbus::Result<Connection> {
    let builder = match &options.bus_address {
        Some(addr) => zbus::blocking::connection::Builder::address(addr.as_str())?,
        None => zbus::blocking::connection::Builder::system()?,
    };
    builder.method_timeout(options.call_timeout).build()
}

fn worker<F: Fn(SessionEvent)>(options: &SessionOptions, rx: &mpsc::Receiver<Command>, sink: &F) {
    let conn = match connect(options) {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::warn!("logind'e bağlanılamadı, oturum eylemleri devre dışı: {e}");
            None
        }
    };
    let caps = SessionAction::ALL
        .into_iter()
        .map(|a| {
            let cap = match (&conn, a.can_method()) {
                (None, _) => Capability::No,
                (Some(_), None) => Capability::Yes,
                (Some(c), Some(m)) => query_capability(c, m).unwrap_or_else(|e| {
                    tracing::debug!("{m} sorgulanamadı: {e}");
                    Capability::Unknown
                }),
            };
            (a, cap)
        })
        .collect();
    sink(SessionEvent::Capabilities(caps));

    // Kuyruk kapanınca (servis düşürüldü) döngü biter.
    while let Ok(Command::Perform(action)) = rx.recv() {
        let result = match &conn {
            Some(c) => perform(c, options, action).map_err(|e| (FailureReason::from_zbus(&e), clip(&e.to_string()))),
            None => Err((FailureReason::Unreachable, "not connected to logind".to_owned())),
        };
        match result {
            Ok(()) => {
                tracing::info!(?action, "oturum eylemi iletildi");
                sink(SessionEvent::Done(action));
            }
            Err((reason, detail)) => {
                // Ham ayrıntı (D-Bus hata adı ve iletisi) yalnızca logda.
                tracing::warn!(?action, ?reason, "oturum eylemi başarısız: {detail}");
                sink(SessionEvent::Failed { action, reason });
            }
        }
    }
}

fn manager(conn: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(conn, LOGIN1, LOGIN1_PATH, MANAGER)
}

fn query_capability(conn: &Connection, method: &str) -> zbus::Result<Capability> {
    let answer: String = manager(conn)?.call(method, &())?;
    Ok(Capability::parse(&answer))
}

fn perform(conn: &Connection, options: &SessionOptions, action: SessionAction) -> zbus::Result<()> {
    let m = manager(conn)?;
    match action {
        // interactive = false: polkit sorusu açılmaz; yetki yoksa hata döner ve gösterilir.
        SessionAction::Suspend => m.call_method("Suspend", &(false,)).map(drop),
        SessionAction::Reboot => m.call_method("Reboot", &(false,)).map(drop),
        SessionAction::PowerOff => m.call_method("PowerOff", &(false,)).map(drop),
        SessionAction::Lock | SessionAction::Logout => {
            let path = resolve_session(conn, &m, options.pid, options.compositor_pid)?;
            let session = Proxy::new(conn, LOGIN1, path, SESSION)?;
            let method = if action == SessionAction::Lock { "Lock" } else { "Terminate" };
            session.call_method(method, &()).map(drop)
        }
    }
}

/// Oturum nesnesi: önce kendi PID'imizden. Kabuk oturum kapsamı dışında başlamışsa (systemd
/// kullanıcı birimi, ya da kendi scope'unu açan bir terminalden `setsid`) logind `NoSessionForPID`
/// der; o zaman compositor'ın PID'i (`hyprland.lock`; tty'den `start-hyprland` + servis olarak
/// kabuk, 2026-10-16), o da olmazsa `session/auto`'nun kimliği (çağıranın oturumu, yoksa
/// kullanıcının görüntü oturumu) → `GetSession`. 2026-10-14: kilitle/oturumu kapat "oturum bulunamadı" veriyordu
/// (kitty'den başlatılan kabuk `kitty-*.scope` içindeydi); boşta servisi ve nuxut-lock zaten
/// bu yedeği kullanıyordu.
fn resolve_session(
    conn: &Connection,
    m: &Proxy<'_>,
    pid: u32,
    compositor_pid: Option<u32>,
) -> zbus::Result<OwnedObjectPath> {
    resolve_session_how(conn, m, pid, compositor_pid).map(|(path, _)| path)
}

/// [`resolve_session`] ve hangi yoldan çözüldüğü (`true`: `session/auto` yedeği).
fn resolve_session_how(conn: &Connection, m: &Proxy<'_>, pid: u32, compositor_pid: Option<u32>) -> zbus::Result<(OwnedObjectPath, bool)> {
    let by_pid = match m.call::<_, _, OwnedObjectPath>("GetSessionByPID", &(pid,)) {
        Ok(path) => return Ok((path, false)),
        Err(e) => e,
    };
    if let Some(Ok(path)) = compositor_pid.map(|p| m.call::<_, _, OwnedObjectPath>("GetSessionByPID", &(p,))) {
        return Ok((path, false));
    }
    let auto = Proxy::new(conn, LOGIN1, SESSION_AUTO, SESSION)?;
    let Ok(id) = auto.get_property::<String>("Id") else { return Err(by_pid) };
    if id.is_empty() || id.len() > 64 {
        return Err(by_pid);
    }
    m.call("GetSession", &(id.as_str(),)).map(|p| (p, true)).or(Err(by_pid))
}

/// Bir PID'in logind oturumu nasıl çözülüyor (`nuxut-shell doctor`; yalnız okur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionResolution {
    /// `GetSessionByPID` oturumu verdi (kabuğun ya da compositor'ın PID'i).
    ByPid,
    /// Yalnız `session/auto` yedeğiyle (kabuk oturum kapsamı dışında başlamış).
    Auto,
    /// Çözülemedi (logind yok ya da oturum yok).
    Missing,
}

/// `pid`'in oturumu: [`resolve_session`] ile aynı sıra. `address` `None` ise sistem bus'ı.
pub fn session_resolution(address: Option<&str>, pid: u32, compositor_pid: Option<u32>) -> SessionResolution {
    let options = SessionOptions { bus_address: address.map(str::to_owned), call_timeout: Duration::from_secs(2), pid, compositor_pid };
    let Ok(conn) = connect(&options) else { return SessionResolution::Missing };
    let Ok(m) = manager(&conn) else { return SessionResolution::Missing };
    match resolve_session_how(&conn, &m, pid, compositor_pid) {
        Ok((_, false)) => SessionResolution::ByPid,
        Ok((_, true)) => SessionResolution::Auto,
        Err(_) => SessionResolution::Missing,
    }
}

fn clip(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(200).collect()
}

// ---------------------------------------------------------------------------------------
// Kullanıcı bilgisi (saf)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    pub login: String,
    /// GECOS'taki tam ad (ilk alan); yoksa `None`.
    pub real_name: Option<String>,
    /// `~/.face` (ya da `~/.face.icon`) varsa.
    pub avatar: Option<PathBuf>,
}

impl UserInfo {
    /// Görünen ad: tam ad, yoksa kullanıcı adı.
    pub fn display_name(&self) -> &str {
        self.real_name.as_deref().unwrap_or(&self.login)
    }

    /// Sistemden okur. Hiçbir adım panik etmez; bilinmeyen alanlar boş kalır.
    pub fn current() -> Self {
        let uid = read_limited(Path::new("/proc/self/status")).and_then(|s| parse_uid(&s));
        let passwd = read_limited(Path::new("/etc/passwd")).unwrap_or_default();
        let entry = uid.and_then(|u| passwd_entry(&passwd, u));
        let login = entry
            .as_ref()
            .map(|e| e.0.clone())
            .or_else(|| std::env::var("USER").ok())
            .unwrap_or_else(|| "?".into());
        let home = entry.as_ref().map(|e| PathBuf::from(&e.2)).or_else(|| std::env::var_os("HOME").map(PathBuf::from));
        let avatar = home.and_then(|h| [".face", ".face.icon"].into_iter().map(|f| h.join(f)).find(|p| p.is_file()));
        Self { login: clip_name(&login), real_name: entry.and_then(|e| e.1), avatar }
    }
}

/// `/proc/uptime` → açık kalma süresi.
pub fn parse_uptime(text: &str) -> Option<Duration> {
    let secs: f64 = text.split_whitespace().next()?.parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
}

pub fn uptime() -> Option<Duration> {
    read_limited(Path::new("/proc/uptime")).and_then(|s| parse_uptime(&s))
}

/// "3 gün 2 sa", "2 sa 14 dk", "5 dk", "1 dk'dan az".
pub fn format_uptime(d: Duration) -> String {
    let mins = d.as_secs() / 60;
    let (days, hours, m) = (mins / 1440, (mins / 60) % 24, mins % 60);
    match (days, hours, m) {
        (0, 0, 0) => t!("shell-session-uptime-under-minute"),
        (0, 0, m) => t!("shell-session-uptime-minutes", minutes = m),
        (0, h, m) => t!("shell-session-uptime-hours-minutes", hours = h, minutes = m),
        (d, h, _) => t!("shell-session-uptime-days-hours", days = d, hours = h),
    }
}

/// `/proc/self/status` → gerçek UID.
fn parse_uid(status: &str) -> Option<u32> {
    status.lines().find_map(|l| l.strip_prefix("Uid:")).and_then(|r| r.split_whitespace().next()).and_then(|u| u.parse().ok())
}

/// `/etc/passwd` → (kullanıcı adı, tam ad, ev dizini).
fn passwd_entry(passwd: &str, uid: u32) -> Option<(String, Option<String>, String)> {
    passwd.lines().find_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 7 || f[2].parse::<u32>().ok()? != uid {
            return None;
        }
        let real = f[4].split(',').next().map(str::trim).filter(|s| !s.is_empty()).map(clip_name);
        Some((f[0].to_owned(), real, f[5].to_owned()))
    })
}

fn clip_name(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(64).collect()
}

fn read_limited(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(MAX_FILE).read_to_string(&mut text).ok()?;
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_parsing_and_formatting() {
        crate::i18n::use_turkish_for_tests();
        assert_eq!(parse_uptime("12345.67 54321.00\n"), Some(Duration::from_secs_f64(12345.67)));
        assert_eq!(parse_uptime("garbage"), None);
        assert_eq!(parse_uptime("-5 1"), None);
        assert_eq!(format_uptime(Duration::from_secs(20)), "1 dk'dan az");
        assert_eq!(format_uptime(Duration::from_secs(5 * 60 + 9)), "5 dk");
        assert_eq!(format_uptime(Duration::from_secs(2 * 3600 + 14 * 60)), "2 sa 14 dk");
        assert_eq!(format_uptime(Duration::from_secs(3 * 86400 + 2 * 3600 + 59)), "3 gün 2 sa");
    }

    #[test]
    fn passwd_and_status_parsing() {
        let passwd = "root:x:0:0::/root:/bin/bash\nayse:x:1000:1000:Ayşe Yılmaz,,,:/home/ayse:/bin/zsh\nbad line\nbob:x:1001:1001::/home/bob:/bin/sh\n";
        assert_eq!(
            passwd_entry(passwd, 1000),
            Some(("ayse".into(), Some("Ayşe Yılmaz".into()), "/home/ayse".into()))
        );
        assert_eq!(passwd_entry(passwd, 1001), Some(("bob".into(), None, "/home/bob".into())));
        assert_eq!(passwd_entry(passwd, 4242), None);
        assert_eq!(parse_uid("Name:\tx\nUid:\t1000\t1000\t1000\t1000\n"), Some(1000));
        assert_eq!(parse_uid("Name: x\n"), None);
    }

    #[test]
    fn capability_parsing() {
        assert_eq!(Capability::parse("yes"), Capability::Yes);
        assert_eq!(Capability::parse("challenge"), Capability::Challenge);
        assert_eq!(Capability::parse("na"), Capability::No);
        assert!(!Capability::No.allowed());
        assert!(Capability::Unknown.allowed());
    }

    #[test]
    fn actions_metadata() {
        assert!(!SessionAction::Lock.is_destructive() && !SessionAction::Suspend.is_destructive());
        assert!(SessionAction::ALL.iter().filter(|a| a.is_destructive()).count() == 3);
        for a in SessionAction::ALL {
            assert!(!a.label().is_empty());
        }
    }

    #[test]
    fn failure_reason_from_dbus_names() {
        use FailureReason as R;
        let cases = [
            ("org.freedesktop.DBus.Error.AccessDenied", R::Denied),
            ("org.freedesktop.PolicyKit1.Error.NotAuthorized", R::Denied),
            ("org.freedesktop.DBus.Error.InteractiveAuthorizationRequired", R::AuthRequired),
            ("org.freedesktop.login1.OperationInProgress", R::Busy),
            ("org.freedesktop.login1.SleepVerbNotSupported", R::Unsupported),
            ("org.freedesktop.login1.NoSessionForPID", R::NoSession),
            ("org.freedesktop.DBus.Error.ServiceUnknown", R::Unreachable),
            ("org.freedesktop.DBus.Error.NoReply", R::Timeout),
            ("com.example.Whatever", R::Other),
            ("", R::Other),
        ];
        for (name, want) in cases {
            assert_eq!(R::from_error_name(name), want, "{name}");
        }
    }

    #[test]
    fn failure_reason_from_zbus_errors() {
        use std::sync::Arc;
        use zbus::fdo;
        let fdo = |e: fdo::Error| zbus::Error::FDO(Box::new(e));
        assert_eq!(FailureReason::from_zbus(&fdo(fdo::Error::AccessDenied("x".into()))), FailureReason::Denied);
        assert_eq!(FailureReason::from_zbus(&fdo(fdo::Error::NoReply("x".into()))), FailureReason::Timeout);
        assert_eq!(
            FailureReason::from_zbus(&fdo(fdo::Error::ZBus(zbus::Error::Address("x".into())))),
            FailureReason::Unreachable
        );
        let timed_out = zbus::Error::InputOutput(Arc::new(std::io::Error::from(std::io::ErrorKind::TimedOut)));
        assert_eq!(FailureReason::from_zbus(&timed_out), FailureReason::Timeout);
        let refused = zbus::Error::InputOutput(Arc::new(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)));
        assert_eq!(FailureReason::from_zbus(&refused), FailureReason::Unreachable);
        assert_eq!(FailureReason::from_zbus(&zbus::Error::InvalidReply), FailureReason::Other);
    }

    #[test]
    fn failure_labels_are_short_and_free_of_dbus_names() {
        use FailureReason as R;
        for r in [R::Denied, R::AuthRequired, R::Busy, R::Unsupported, R::NoSession, R::Unreachable, R::Timeout, R::Other] {
            let l = r.label();
            assert!(!l.is_empty() && l.chars().count() <= 32, "{l}");
            assert!(!l.contains("org.") && !l.contains("Error"), "{l}");
        }
    }

    #[test]
    fn current_user_never_panics() {
        let u = UserInfo::current();
        assert!(!u.login.is_empty());
        assert!(!u.display_name().is_empty());
    }
}
