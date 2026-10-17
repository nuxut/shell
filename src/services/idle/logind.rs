//! Boşta/uyku için logind bağlantısı (sistem bus'ı): `PrepareForSleep`, `idle` blok
//! inhibitörleri (`BlockInhibited`), oturumun `LockedHint`'i, uyku gecikme inhibitörü
//! (`Inhibit("sleep", …, "delay")`) ve boşta eylemleri (`Session.Lock`, `Suspend`). Oturumun
//! `Lock` sinyalini de iletir: kabuğun kilit tetikleyicisi buna bağlıdır, boşta ayarlarından
//! bağımsız çalışır ([`crate::services::lock_launcher`]).
//!
//! Oturum kartının servisinden ayrıdır: boşta eylemlerinin hataları kartta görünmez, sinyal
//! dinlemek için zaman uyumsuz döngü gerekir. Sinyaller yalnızca `org.freedesktop.login1`
//! adının güncel benzersiz sahibinden kabul edilir ([`nuxut_dbus`]). logind
//! düşüp gelirse durum sıfırlanır, yeniden okunur ve istenmişse inhibitör yeniden alınır.
//! Gecikme inhibitörü yalnızca istendiğinde ([`IdleLogind::set_sleep_delay`]) tutulur;
//! dosya tanıtıcısı düşürülünce logind onu bırakılmış sayar.

use std::io;
use std::time::Duration;

use futures_lite::future::FutureExt;
use zbus::zvariant::{OwnedFd, OwnedObjectPath, OwnedValue};
use zbus::{Connection, Message};

use nuxut_dbus::{
    Backoff, BusKind, MAX_PATH, NameOwner, PROPERTIES, PropMap, ServiceContext, ServiceThread, close, connect,
    forward, get_all, get_name_owner, owner_changes, parse_owner_change, parse_properties_changed, prop_bool, prop_i64,
    prop_str, signals_from, tick_connection,
};
use crate::services::session::FailureReason;

const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
const SESSION_PREFIX: &str = "/org/freedesktop/login1/session/";
const AUTO_SESSION: &str = "/org/freedesktop/login1/session/auto";
const MAX_CHANGED: usize = 64;
/// `BlockInhibited` gibi iki nokta ayrımlı listeler.
const MAX_LIST: usize = 1024;

/// logind'den bilinen durum (tam anlık görüntü; yalnızca değişince yayınlanır).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogindState {
    /// login1'in sahibi var ve okundu.
    pub available: bool,
    /// `BlockInhibited` içinde `idle` var.
    pub idle_inhibited: bool,
    /// Oturumumuzun `LockedHint`'i.
    pub locked_hint: bool,
    /// Oturum nesnemiz çözüldü: `Lock` sinyali bize ulaşır.
    pub has_session: bool,
    /// `InhibitDelayMaxUSec` (okunamazsa 0).
    pub delay_max: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogindAction {
    Lock,
    Suspend,
    /// Uyku gecikme inhibitörü alınamadı.
    Inhibit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogindEvent {
    State(LogindState),
    /// `PrepareForSleep(true)` uyku başlıyor, `false` uyanıldı.
    PrepareForSleep(bool),
    /// Oturumumuzun `Session.Lock` sinyali (logind'in kilit isteği: Kilitle, `loginctl
    /// lock-session`, boşta kilit). `Unlock` sinyali bilerek iletilmez: kilidi yalnızca kimlik
    /// doğrulama açar.
    LockRequested,
    /// Gecikme inhibitörü tutuluyor mu (değişince).
    SleepDelayHeld(bool),
    Failed { action: LogindAction, reason: FailureReason },
}

#[derive(Debug, Clone)]
pub struct LogindOptions {
    /// D-Bus adresi; `None`: sistem bus'ı (testler sahte login1 adresi verir).
    pub bus_address: Option<String>,
    pub call_timeout: Duration,
    pub max_backoff: Duration,
    /// Oturumu çözmek için PID.
    pub pid: u32,
    /// Kendi PID'imiz oturumda değilse oturum compositor'ın PID'inden çözülür
    /// (`services::session::SessionOptions::compositor_pid`).
    pub compositor_pid: Option<u32>,
}

impl Default for LogindOptions {
    fn default() -> Self {
        Self {
            bus_address: None,
            call_timeout: Duration::from_secs(3),
            max_backoff: Duration::from_secs(30),
            pid: std::process::id(),
            compositor_pid: crate::services::hyprland::compositor_pid(),
        }
    }
}

#[derive(Debug)]
enum Command {
    SleepDelay(bool),
    IdleHint(bool),
    Act(LogindAction),
}

/// Çalışan logind bağlantısı. Düşürülünce inhibitör bırakılır, thread join edilir.
#[derive(Debug)]
pub struct IdleLogind {
    thread: ServiceThread<Command>,
}

impl IdleLogind {
    /// `sink` servis thread'inden çağrılır; hızlı dönmeli.
    pub fn spawn<F>(options: LogindOptions, sink: F) -> io::Result<Self>
    where
        F: Fn(LogindEvent) + Send + 'static,
    {
        let thread = ServiceThread::spawn("nuxut-idle-logind", 16, move |ctx| run(options, sink, ctx))?;
        Ok(Self { thread })
    }

    /// Uyku gecikme inhibitörünü tut / bırak (istek kalıcıdır: logind yeniden gelince uygulanır).
    pub fn set_sleep_delay(&self, hold: bool) -> bool {
        self.thread.send(Command::SleepDelay(hold))
    }

    /// Oturumun `IdleHint`'i (`Session.SetIdleHint`; istek kalıcıdır, yeniden bağlanınca yazılır).
    pub fn set_idle_hint(&self, idle: bool) -> bool {
        self.thread.send(Command::IdleHint(idle))
    }

    /// Oturumu kilitle (`Session.Lock`).
    pub fn lock(&self) -> bool {
        self.thread.send(Command::Act(LogindAction::Lock))
    }

    /// Askıya al (`Suspend(false)`: polkit sorusu açılmaz).
    pub fn suspend(&self) -> bool {
        self.thread.send(Command::Act(LogindAction::Suspend))
    }
}

#[derive(Debug)]
enum Input {
    Owner,
    Ignore,
    Signal(Message),
}

enum End {
    Stop,
    Lost,
}

enum Wake {
    Stop,
    Lost,
    Input(Input),
    Command(Command),
}

async fn run<F: Fn(LogindEvent)>(options: LogindOptions, sink: F, ctx: ServiceContext<Command>) {
    let mut backoff = Backoff::new(options.max_backoff);
    let mut rt = Runtime {
        sink,
        pid: options.pid,
        compositor_pid: options.compositor_pid,
        state: LogindState::default(),
        emitted: None,
        want_delay: false,
        delay: None,
        want_hint: None,
        session: None,
    };
    loop {
        match connect(BusKind::System, options.bus_address.as_deref(), options.call_timeout).await {
            Ok(conn) => {
                backoff.reset();
                let ticker = tick_connection(&ctx.ex, &conn);
                let end = rt.session(&ctx, &conn).await;
                rt.drop_delay();
                drop(ticker);
                close(conn).await;
                if matches!(end, End::Stop) {
                    return;
                }
                tracing::warn!("boşta: logind bağlantısı koptu, yeniden bağlanılacak");
            }
            Err(e) => tracing::warn!("boşta: sistem bus'ına bağlanılamadı: {e}"),
        }
        rt.state = LogindState::default();
        rt.session = None;
        rt.publish();
        if !rt.wait_offline(&ctx, backoff.next_delay()).await {
            return;
        }
    }
}

struct Runtime<F> {
    sink: F,
    pid: u32,
    compositor_pid: Option<u32>,
    state: LogindState,
    emitted: Option<LogindState>,
    want_delay: bool,
    delay: Option<OwnedFd>,
    /// İstenen IdleHint; `None`: hiç istenmedi, logind'e dokunulmaz (boşta kalma kapalıyken
    /// başka bir boşta kalma aracının, ör. hypridle, ipucu ezilmez). İstendiyse her bağlanışta
    /// yeniden yazılır.
    want_hint: Option<bool>,
    session: Option<String>,
}

impl<F: Fn(LogindEvent)> Runtime<F> {
    fn publish(&mut self) {
        if self.emitted != Some(self.state) {
            self.emitted = Some(self.state);
            (self.sink)(LogindEvent::State(self.state));
        }
    }

    fn drop_delay(&mut self) {
        if self.delay.take().is_some() {
            tracing::debug!("uyku gecikme inhibitörü bırakıldı");
            (self.sink)(LogindEvent::SleepDelayHeld(false));
        }
    }

    /// Bağlantı yokken bekler; komutlar kaybolmaz: gecikme isteği saklanır, eylemler hata döner.
    async fn wait_offline(&mut self, ctx: &ServiceContext<Command>, d: Duration) -> bool {
        let deadline = std::time::Instant::now() + d;
        loop {
            let wake = async {
                async_io::Timer::at(deadline).await;
                None
            }
            .or(async { ctx.stop.recv().await.map_or(Some(Wake::Stop), |()| Some(Wake::Stop)) })
            .or(async { Some(ctx.commands.recv().await.map_or(Wake::Stop, Wake::Command)) })
            .await;
            match wake {
                None => return true,
                Some(Wake::Command(Command::SleepDelay(w))) => self.want_delay = w,
                Some(Wake::Command(Command::IdleHint(h))) => self.want_hint = Some(h),
                Some(Wake::Command(Command::Act(action))) => {
                    (self.sink)(LogindEvent::Failed { action, reason: FailureReason::Unreachable });
                }
                Some(_) => return false,
            }
        }
    }

    async fn session(&mut self, ctx: &ServiceContext<Command>, conn: &Connection) -> End {
        let (tx, rx) = async_channel::bounded::<Input>(64);
        let mut owner = NameOwner::new();
        // Akışlar sahip/özellik okunmadan önce kurulur: aradaki değişiklik kaçmaz.
        let streams = async {
            Ok::<_, zbus::Error>((owner_changes(conn, LOGIN1).await?, signals_from(conn, LOGIN1, Some(LOGIN1_PATH), None).await?))
        };
        let (owner_stream, signals) = match streams.await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("boşta: logind sinyal aboneliği kurulamadı: {e}");
                return End::Lost;
            }
        };
        let _tasks = [
            forward(&ctx.ex, owner_stream, tx.clone(), |m| {
                if parse_owner_change(&m, LOGIN1).is_some() { Input::Owner } else { Input::Ignore }
            }),
            forward(&ctx.ex, signals, tx.clone(), Input::Signal),
        ];
        self.owner_changed(conn, &mut owner, get_name_owner(conn, LOGIN1).await).await;
        self.publish();

        loop {
            let wake = async { rx.recv().await.map_or(Wake::Lost, Wake::Input) }
                .or(async { ctx.commands.recv().await.map_or(Wake::Stop, Wake::Command) })
                .or(async { ctx.stop.recv().await.map_or(Wake::Stop, |()| Wake::Stop) })
                .await;
            match wake {
                Wake::Stop => return End::Stop,
                Wake::Lost => return End::Lost,
                Wake::Input(Input::Ignore) => {}
                Wake::Input(Input::Owner) => {
                    let o = get_name_owner(conn, LOGIN1).await;
                    self.owner_changed(conn, &mut owner, o).await;
                }
                Wake::Input(Input::Signal(msg)) => self.signal(conn, &mut owner, &msg).await,
                Wake::Command(Command::SleepDelay(w)) => {
                    self.want_delay = w;
                    self.sync_delay(conn, &owner).await;
                }
                Wake::Command(Command::IdleHint(h)) => {
                    self.want_hint = Some(h);
                    self.sync_hint(conn, &owner).await;
                }
                Wake::Command(Command::Act(action)) => self.act(conn, &owner, action).await,
            }
            self.publish();
        }
    }

    async fn owner_changed(&mut self, conn: &Connection, owner: &mut NameOwner, new: Option<String>) {
        if new.is_some() && new == owner.owner {
            return;
        }
        owner.set_owner(new);
        self.drop_delay();
        self.state = LogindState::default();
        self.session = None;
        if owner.owner.is_none() {
            return;
        }
        match get_all(conn, LOGIN1, LOGIN1_PATH, MANAGER).await {
            Ok((props, reply)) => {
                owner.loaded(&reply);
                self.apply_manager(&props);
            }
            Err(e) => {
                tracing::warn!("boşta: logind okunamadı: {e}");
                return;
            }
        }
        self.session = resolve_session(conn, self.pid, self.compositor_pid).await;
        match &self.session {
            Some(path) => match get_all(conn, LOGIN1, path, SESSION).await {
                Ok((props, reply)) => {
                    owner.loaded(&reply);
                    self.apply_session(&props);
                }
                Err(e) => tracing::warn!("boşta: oturum okunamadı: {e}"),
            },
            None => tracing::warn!("boşta: logind oturumu bulunamadı; kilit durumu izlenmeyecek"),
        }
        self.state.has_session = self.session.is_some();
        self.state.available = true;
        self.sync_delay(conn, owner).await;
        self.sync_hint(conn, owner).await;
    }

    /// `Session.SetIdleHint(want_hint)`. Hata yalnızca loglanır (kullanıcıya gösterilecek bir
    /// eylem değil; logind'in IdleAction'ı ve oturum izleyicileri için bilgi).
    async fn sync_hint(&mut self, conn: &Connection, owner: &NameOwner) {
        let Some(hint) = self.want_hint else { return };
        let Some(path) = self.session.as_deref().filter(|_| owner.owner.is_some()) else { return };
        let reply = conn.call_method(Some(LOGIN1), path, Some(SESSION), "SetIdleHint", &(hint,)).await;
        if let Err(e) = reply {
            tracing::warn!(idle = hint, "boşta: IdleHint yazılamadı: {}", nuxut_dbus::clean_str(&e.to_string(), 200));
        }
    }

    fn apply_manager(&mut self, props: &PropMap) {
        if let Some(list) = prop_str(props, "BlockInhibited", MAX_LIST) {
            self.state.idle_inhibited = list.split(':').any(|w| w == "idle");
        }
        if let Some(us) = prop_i64(props, "InhibitDelayMaxUSec") {
            self.state.delay_max = Duration::from_micros(us.max(0).unsigned_abs());
        }
    }

    fn apply_session(&mut self, props: &PropMap) {
        if let Some(b) = prop_bool(props, "LockedHint") {
            self.state.locked_hint = b;
        }
    }

    async fn signal(&mut self, conn: &Connection, owner: &mut NameOwner, msg: &Message) {
        if !owner.accepts(msg) {
            return;
        }
        let header = msg.header();
        let path = header.path().map(|p| p.as_str().to_owned()).unwrap_or_default();
        let iface = header.interface().map(|i| i.as_str().to_owned()).unwrap_or_default();
        let member = header.member().map(|m| m.as_str().to_owned()).unwrap_or_default();
        match (iface.as_str(), member.as_str()) {
            (MANAGER, "PrepareForSleep") if path == LOGIN1_PATH => {
                if let Ok(start) = msg.body().deserialize::<bool>() {
                    (self.sink)(LogindEvent::PrepareForSleep(start));
                }
            }
            // Yalnızca kendi oturumumuz: `loginctl lock-sessions` her oturuma ayrı sinyal yollar.
            (SESSION, "Lock") if self.session.as_deref() == Some(path.as_str()) => {
                (self.sink)(LogindEvent::LockRequested);
            }
            (PROPERTIES, "PropertiesChanged") => {
                let Some((changed_iface, changed, invalidated)) = parse_properties_changed(msg, MAX_CHANGED) else { return };
                if path == LOGIN1_PATH && changed_iface == MANAGER {
                    self.apply_manager(&changed);
                    if invalidated.iter().any(|p| p == "BlockInhibited")
                        && let Some(v) = get_property(conn, LOGIN1_PATH, MANAGER, "BlockInhibited").await
                    {
                        self.apply_manager(&PropMap::from([("BlockInhibited".to_owned(), v)]));
                    }
                } else if self.session.as_deref() == Some(path.as_str()) && changed_iface == SESSION {
                    self.apply_session(&changed);
                    if invalidated.iter().any(|p| p == "LockedHint")
                        && let Some(v) = get_property(conn, &path, SESSION, "LockedHint").await
                    {
                        self.apply_session(&PropMap::from([("LockedHint".to_owned(), v)]));
                    }
                }
            }
            _ => {}
        }
    }

    async fn sync_delay(&mut self, conn: &Connection, owner: &NameOwner) {
        if !self.want_delay {
            self.drop_delay();
            return;
        }
        if self.delay.is_some() || owner.owner.is_none() {
            return;
        }
        let why = nuxut::i18n::t!("shell-idle-sleep-inhibit-why");
        let reply =
            conn.call_method(Some(LOGIN1), LOGIN1_PATH, Some(MANAGER), "Inhibit", &("sleep", "Nuxut", why.as_str(), "delay")).await;
        match reply.and_then(|r| r.body().deserialize::<OwnedFd>()) {
            Ok(fd) => {
                self.delay = Some(fd);
                (self.sink)(LogindEvent::SleepDelayHeld(true));
            }
            Err(e) => {
                tracing::warn!("boşta: uyku gecikme inhibitörü alınamadı: {}", nuxut_dbus::clean_str(&e.to_string(), 200));
                (self.sink)(LogindEvent::Failed { action: LogindAction::Inhibit, reason: FailureReason::from_zbus(&e) });
            }
        }
    }

    async fn act(&mut self, conn: &Connection, owner: &NameOwner, action: LogindAction) {
        let result = if owner.owner.is_none() {
            Err(zbus::Error::Failure("logind yok".into()))
        } else {
            match action {
                LogindAction::Lock => match &self.session {
                    Some(path) => conn.call_method(Some(LOGIN1), path.as_str(), Some(SESSION), "Lock", &()).await.map(drop),
                    None => Err(zbus::Error::Failure("oturum yok".into())),
                },
                // interactive = false: polkit sorusu açılmaz.
                LogindAction::Suspend => {
                    conn.call_method(Some(LOGIN1), LOGIN1_PATH, Some(MANAGER), "Suspend", &(false,)).await.map(drop)
                }
                LogindAction::Inhibit => {
                    self.sync_delay(conn, owner).await;
                    Ok(())
                }
            }
        };
        match result {
            Ok(()) => tracing::info!(?action, "boşta eylemi logind'e iletildi"),
            Err(e) => {
                let reason = match &e {
                    zbus::Error::Failure(_) if owner.owner.is_none() => FailureReason::Unreachable,
                    zbus::Error::Failure(_) => FailureReason::NoSession,
                    e => FailureReason::from_zbus(e),
                };
                tracing::warn!(?action, ?reason, "boşta eylemi başarısız: {}", nuxut_dbus::clean_str(&e.to_string(), 200));
                (self.sink)(LogindEvent::Failed { action, reason });
            }
        }
    }
}

/// Oturum nesnesi: kendi PID'imizden; systemd kullanıcı servisi oturum dışında olduğundan
/// olmazsa compositor'ın PID'inden, o da olmazsa `session/auto`'nun kimliği (kullanıcının
/// görüntü oturumu) → `GetSession`.
async fn resolve_session(conn: &Connection, pid: u32, compositor_pid: Option<u32>) -> Option<String> {
    for pid in std::iter::once(pid).chain(compositor_pid) {
        let by_pid = conn.call_method(Some(LOGIN1), LOGIN1_PATH, Some(MANAGER), "GetSessionByPID", &(pid,)).await;
        if let Ok(path) = by_pid.and_then(|r| r.body().deserialize::<OwnedObjectPath>()) {
            return valid_session(path.as_str());
        }
    }
    let id = get_property(conn, AUTO_SESSION, SESSION, "Id").await?;
    let id: String = nuxut_dbus::value_str(&id, 64)?;
    let reply = conn.call_method(Some(LOGIN1), LOGIN1_PATH, Some(MANAGER), "GetSession", &(id.as_str(),)).await.ok()?;
    let path: OwnedObjectPath = reply.body().deserialize().ok()?;
    valid_session(path.as_str())
}

fn valid_session(path: &str) -> Option<String> {
    (path.starts_with(SESSION_PREFIX) && path.len() <= MAX_PATH).then(|| path.to_owned())
}

async fn get_property(conn: &Connection, path: &str, iface: &str, name: &str) -> Option<OwnedValue> {
    let reply = conn.call_method(Some(LOGIN1), path, Some(PROPERTIES), "Get", &(iface, name)).await.ok()?;
    reply.body().deserialize::<OwnedValue>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_paths_are_validated() {
        assert_eq!(valid_session("/org/freedesktop/login1/session/_32").as_deref(), Some("/org/freedesktop/login1/session/_32"));
        assert_eq!(valid_session("/org/freedesktop/login1"), None);
        assert_eq!(valid_session("/evil/session"), None);
        let long = format!("{SESSION_PREFIX}{}", "x".repeat(300));
        assert_eq!(valid_session(&long), None);
    }
}
