//! Testler için özel `dbus-daemon` ve sahte polkitd Authority + logind. Kullanıcının bus'larına
//! dokunulmaz; gerçek polkitd'ye kayıt ya da parola gönderimi yok.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::message::Header;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub const AGENT_PATH: &str = "/org/nuxut/PolicyKit1/AuthenticationAgent";
pub const AGENT_IFACE: &str = "org.freedesktop.PolicyKit1.AuthenticationAgent";
/// `=1` ise özel `dbus-daemon` gerektiren testler bilerek atlanır (yalnızca `dbus-daemon`
/// olmayan derleme ortamları için; varsayılan: dbus-daemon yoksa test başarısız).
pub const SKIP_DBUS_ENV: &str = "NUXUT_POLKIT_SKIP_DBUS_TESTS";

pub struct PrivateBus {
    child: Child,
    pub address: String,
    _dir: tempfile::TempDir,
}

impl PrivateBus {
    /// Özel bus'ı başlatır. `dbus-daemon` yoksa ya da başlamazsa test **başarısız olur**
    /// (sessizce geçen güvenlik testi olmasın). Bilinçli atlama yalnızca açık ortam
    /// değişkeniyle: [`SKIP_DBUS_ENV`]`=1` ise `None` döner ve çağıran test atlar.
    pub fn start() -> Option<Self> {
        if std::env::var_os(SKIP_DBUS_ENV).is_some_and(|v| v == "1") {
            eprintln!("{SKIP_DBUS_ENV}=1: D-Bus testi bilerek atlandı");
            return None;
        }
        match Self::try_start() {
            Ok(bus) => Some(bus),
            Err(e) => panic!(
                "özel dbus-daemon başlatılamadı: {e}. dbus-daemon kurulu olmalı \
                 (Arch: dbus); bilerek atlamak için {SKIP_DBUS_ENV}=1"
            ),
        }
    }

    fn try_start() -> Result<Self, String> {
        let dir = tempfile::tempdir().map_err(|e| format!("geçici dizin: {e}"))?;
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
        .map_err(|e| format!("bus yapılandırması yazılamadı: {e}"))?;
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("dbus-daemon çalıştırılamadı ({e})"))?;
        let mut line = String::new();
        let stdout = child.stdout.take().ok_or("dbus-daemon stdout yok")?;
        let read = BufReader::new(stdout).read_line(&mut line);
        if read.is_err() || line.trim().is_empty() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("dbus-daemon adres yazmadı".into());
        }
        Ok(Self {
            child,
            address: line.trim().to_owned(),
            _dir: dir,
        })
    }

    pub fn connect(&self) -> Connection {
        zbus::blocking::connection::Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .unwrap()
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Registration {
    pub sender: String,
    pub kind: String,
    pub session_id: Option<String>,
    pub locale: String,
    pub path: String,
    pub fallback: Option<bool>,
}

#[derive(Debug, Default)]
pub struct AuthorityState {
    pub registrations: Vec<Registration>,
    /// Bu kadar kayıt daha "already exists" ile reddedilir.
    pub reject_registrations: u32,
    pub attempts: u32,
    pub responses: Vec<(u32, String, u32)>,
    /// Bu kadar başarılı kayıtta, yanıttan ÖNCE ad bırakılıp aynı bağlantıyla yeniden alınır:
    /// `sahip→""` ve `""→sahip` sinyalleri kayıt yanıtından önce ajana ulaşır (polkitd'nin
    /// bus etkinleştirmesi/yeniden başlaması yarışının benzetimi).
    pub bounce_name_on_register: u32,
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
pub enum FakeError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Failed(String),
}

pub struct FakeAuthority {
    pub state: Arc<Mutex<AuthorityState>>,
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
impl FakeAuthority {
    async fn register_authentication_agent_with_options(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        subject: (String, HashMap<String, OwnedValue>),
        locale: String,
        object_path: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
    ) -> Result<(), FakeError> {
        let registration = Registration {
            sender: hdr.sender().map(|n| n.to_string()).unwrap_or_default(),
            kind: subject.0,
            session_id: subject
                .1
                .get("session-id")
                .and_then(|v| String::try_from(v.try_clone().ok()?).ok()),
            locale,
            path: object_path.to_string(),
            fallback: options.get("fallback").and_then(|v| bool::try_from(v).ok()),
        };
        let bounce = {
            let mut s = self.state.lock().unwrap();
            s.attempts += 1;
            // Gerçek polkitd gibi: aynı oturuma ikinci kayıt reddedilir.
            let duplicate = s
                .registrations
                .iter()
                .any(|r| r.session_id == registration.session_id);
            if s.reject_registrations > 0 || duplicate {
                s.reject_registrations = s.reject_registrations.saturating_sub(1);
                return Err(FakeError::Failed(
                    "An authentication agent already exists for the given subject".into(),
                ));
            }
            s.registrations.push(registration);
            let bounce = s.bounce_name_on_register > 0;
            s.bounce_name_on_register = s.bounce_name_on_register.saturating_sub(1);
            bounce
        };
        if bounce {
            conn.release_name("org.freedesktop.PolicyKit1").await.map_err(FakeError::ZBus)?;
            conn.request_name("org.freedesktop.PolicyKit1").await.map_err(FakeError::ZBus)?;
        }
        Ok(())
    }

    fn authentication_agent_response2(
        &self,
        uid: u32,
        cookie: String,
        identity: (String, HashMap<String, OwnedValue>),
    ) -> Result<(), FakeError> {
        let who = identity.1.get("uid").and_then(|v| u32::try_from(v).ok()).unwrap_or(u32::MAX);
        self.state.lock().unwrap().responses.push((uid, cookie, who));
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LogindConfig {
    pub by_pid: Option<String>,
    /// oturum kimliği → sahibi uid.
    pub sessions: Vec<(String, u32)>,
    pub display: Option<String>,
}

impl LogindConfig {
    pub fn simple(uid: u32) -> Self {
        Self {
            by_pid: Some("c1".into()),
            sessions: vec![("c1".into(), uid)],
            display: Some("c1".into()),
        }
    }
}

struct FakeManager(LogindConfig);

fn session_path(id: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(format!("/org/freedesktop/login1/session/{id}")).unwrap()
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeManager {
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, _pid: u32) -> zbus::fdo::Result<OwnedObjectPath> {
        match &self.0.by_pid {
            Some(id) => Ok(session_path(id)),
            None => Err(zbus::fdo::Error::Failed("PID does not belong to any known session".into())),
        }
    }
    fn get_session(&self, id: &str) -> zbus::fdo::Result<OwnedObjectPath> {
        if self.0.sessions.iter().any(|(s, _)| s == id) {
            Ok(session_path(id))
        } else {
            Err(zbus::fdo::Error::Failed("No session".into()))
        }
    }
    fn get_user(&self, uid: u32) -> zbus::fdo::Result<OwnedObjectPath> {
        Ok(OwnedObjectPath::try_from(format!("/org/freedesktop/login1/user/_{uid}")).unwrap())
    }
}

struct FakeSession {
    id: String,
    uid: u32,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl FakeSession {
    #[zbus(property)]
    fn id(&self) -> String {
        self.id.clone()
    }
    #[zbus(property)]
    fn user(&self) -> (u32, OwnedObjectPath) {
        (
            self.uid,
            OwnedObjectPath::try_from(format!("/org/freedesktop/login1/user/_{}", self.uid)).unwrap(),
        )
    }
}

struct FakeUser {
    display: Option<String>,
}

#[zbus::interface(name = "org.freedesktop.login1.User")]
impl FakeUser {
    #[zbus(property)]
    fn display(&self) -> (String, OwnedObjectPath) {
        match &self.display {
            Some(id) => (id.clone(), session_path(id)),
            None => (String::new(), OwnedObjectPath::try_from("/").unwrap()),
        }
    }
}

/// Sahte polkitd: `org.freedesktop.PolicyKit1` adı + Authority nesnesi.
pub fn start_authority(bus: &PrivateBus) -> (Connection, Arc<Mutex<AuthorityState>>) {
    let state = Arc::new(Mutex::new(AuthorityState::default()));
    let conn = zbus::blocking::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .serve_at(
            "/org/freedesktop/PolicyKit1/Authority",
            FakeAuthority {
                state: Arc::clone(&state),
            },
        )
        .unwrap()
        .name("org.freedesktop.PolicyKit1")
        .unwrap()
        .build()
        .unwrap();
    (conn, state)
}

/// Sahte logind (aynı bus'ta).
pub fn start_logind(bus: &PrivateBus, cfg: &LogindConfig, uid: u32) -> Connection {
    let mut b = zbus::blocking::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .serve_at("/org/freedesktop/login1", FakeManager(cfg.clone()))
        .unwrap()
        .serve_at(
            format!("/org/freedesktop/login1/user/_{uid}"),
            FakeUser {
                display: cfg.display.clone(),
            },
        )
        .unwrap();
    for (id, owner) in &cfg.sessions {
        b = b
            .serve_at(
                format!("/org/freedesktop/login1/session/{id}"),
                FakeSession {
                    id: id.clone(),
                    uid: *owner,
                },
            )
            .unwrap();
    }
    b.name("org.freedesktop.login1").unwrap().build().unwrap()
}

pub fn wait_until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f()
}

/// `a(sa{sv})` tek unix-user kimliği.
pub fn user_identity(uid: u32) -> Vec<(&'static str, HashMap<&'static str, Value<'static>>)> {
    let mut m = HashMap::new();
    m.insert("uid", Value::U32(uid));
    vec![("unix-user", m)]
}

/// Sonuç: Ok, ya da hata adı (`...Error.Cancelled` gibi).
pub fn begin(
    conn: &Connection,
    agent: &str,
    cookie: &str,
    uid: u32,
    details: HashMap<&str, &str>,
) -> Result<(), String> {
    conn.call_method(
        Some(agent),
        AGENT_PATH,
        Some(AGENT_IFACE),
        "BeginAuthentication",
        &(
            "org.nuxut.test.action",
            "Test için kimlik doğrulama",
            "",
            details,
            cookie,
            user_identity(uid),
        ),
    )
    .map(|_| ())
    .map_err(error_name)
}

pub fn cancel(conn: &Connection, agent: &str, cookie: &str) -> Result<(), String> {
    conn.call_method(Some(agent), AGENT_PATH, Some(AGENT_IFACE), "CancelAuthentication", &(cookie,))
        .map(|_| ())
        .map_err(error_name)
}

pub fn error_name(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, _, _) => name.to_string(),
        other => format!("{other}"),
    }
}

pub fn uid() -> u32 {
    rustix::process::getuid().as_raw()
}
