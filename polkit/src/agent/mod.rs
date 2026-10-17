//! Yerleşik ajan: sistem bus'ında `org.freedesktop.PolicyKit1.AuthenticationAgent`, kayıt,
//! kuyruk ve istek başına diyalog süreci. Parola bu sürece hiç girmez (diyalogda kalır).
//!
//! Güvenlik kuralları:
//! - `BeginAuthentication`/`CancelAuthentication` çağrısının göndereni, o anda
//!   `org.freedesktop.PolicyKit1` adının sahibi olan benzersiz ad değilse
//!   `org.freedesktop.PolicyKit1.Error.Failed` ile reddedilir; diyalog açılmaz, kuyruk değişmez.
//!   Sahip her çağrıda yeniden sorulur (önbellek yok: polkitd yeniden başlarsa eski benzersiz
//!   ada güvenilmez).
//! - Kayıt yalnızca kendi logind oturumuna (`unix-session`), `fallback = false`.
//! - Başka ajan kayıtlıysa ("already exists") açık uyarı ve geri çekilmeli yeniden deneme;
//!   ajan başka hiçbir sürece (hyprpolkitagent dahil) dokunmaz.
//! - polkitd adı el değiştirince bekleyen/etkin her istek `Cancelled` ile kapanır. Yeniden
//!   kayıt yalnızca yeni sahip boş değilse ve kaydı kabul eden sahipten farklıysa yapılır
//!   ([`owner_change`]): polkitd yeniden başlarken gelen `eski→""` ve (kayıt çağrısının
//!   etkinleştirmesiyle) `""→yeni` sinyalleri ikinci bir kayıt ve "already exists" döngüsü
//!   doğurmaz.

pub mod process;
pub mod queue;

use std::collections::HashMap;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use futures_lite::StreamExt;
use zbus::Connection;
use zbus::message::Header;
use zbus::names::BusName;
use zbus::zvariant::{OwnedValue, Value};

use crate::logind;
use crate::request::{self, DialogOutcome, DialogRequest};
use crate::text;
use process::{DialogCommand, DialogProcess};
use queue::{Action, Queue, Reject, Reply, RequestId};

pub const OBJECT_PATH: &str = "/org/nuxut/PolicyKit1/AuthenticationAgent";
pub const AUTHORITY_NAME: &str = "org.freedesktop.PolicyKit1";
pub const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
pub const AUTHORITY_IFACE: &str = "org.freedesktop.PolicyKit1.Authority";

/// polkit hata adları (`org.freedesktop.PolicyKit1.Error.*`). polkitd yalnızca `Cancelled`'ı
/// "kullanıcı vazgeçti" olarak ayırır; gerisi başarısızlıktır.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
pub enum PolkitError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Failed(String),
    Cancelled(String),
}

/// Gönderen doğrulaması (saf): ikisi de bilinmeli ve aynı benzersiz ad olmalı.
pub fn sender_allowed(sender: Option<&str>, owner: Option<&str>) -> bool {
    matches!((sender, owner), (Some(s), Some(o)) if s == o && s.starts_with(':'))
}

/// "already exists" sonrası bekleme: 30 sn, 2 dk, sonra 5 dk.
pub fn backoff(attempt: u32) -> Duration {
    match attempt {
        0 => Duration::from_secs(30),
        1 => Duration::from_secs(120),
        _ => Duration::from_secs(300),
    }
}

/// Kayıt hatası türü.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// Bu oturuma başka bir ajan kayıtlı.
    AlreadyExists,
    Other(String),
}

/// polkitd hata mesajından tür (polkitbackendinteractiveauthority.c: "An authentication agent
/// already exists for the given subject").
pub fn classify_register_error(message: &str) -> RegisterError {
    if message.contains("already exists") {
        RegisterError::AlreadyExists
    } else {
        RegisterError::Other(message.to_owned())
    }
}

/// Kayıt `locale` değeri: `LC_ALL` > `LC_MESSAGES` > `LANG`, yoksa `C`.
pub fn locale_from_env(get: impl Fn(&str) -> Option<String>) -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| get(k))
        .find(|v| {
            !v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_graphic())
        })
        .unwrap_or_else(|| "C".to_owned())
}

#[derive(Debug, Clone)]
pub struct AgentOptions {
    pub dialog: DialogCommand,
    /// Diyalog etkileşimsiz bu süre sonra iptal olur (diyalogda uygulanır).
    pub timeout: Duration,
    /// Diyalog takılırsa ajan tarafı mutlak sınır.
    pub backstop: Duration,
    /// SIGTERM → SIGKILL → yuvayı zorla boşaltma aralığı.
    pub kill_grace: Duration,
    pub own_uid: u32,
    pub own_pid: u32,
    pub env_session: Option<String>,
    pub locale: String,
    /// Kendi ev dizini: altındaki yürütülebilir dosyalar doğrulanmaz.
    pub home: Option<String>,
    pub backoff: fn(u32) -> Duration,
}

impl AgentOptions {
    pub fn new(dialog: DialogCommand, timeout: Duration) -> Self {
        Self {
            dialog,
            timeout,
            backstop: timeout.saturating_mul(4).max(Duration::from_secs(60)),
            kill_grace: Duration::from_secs(3),
            own_uid: rustix::process::getuid().as_raw(),
            own_pid: std::process::id(),
            env_session: std::env::var("XDG_SESSION_ID").ok(),
            locale: locale_from_env(|k| std::env::var(k).ok()),
            home: request::own_home(),
            backoff,
        }
    }
}

enum Cmd {
    Begin {
        request: DialogRequest,
        reply: Sender<Reply>,
    },
    Cancel {
        cookie: String,
        reply: Sender<bool>,
    },
    Exited {
        id: RequestId,
        outcome: DialogOutcome,
    },
    Deadline(RequestId),
    /// 1: SIGKILL, 2: süreç hâlâ raporlamadıysa yuvayı `Failed` ile boşalt.
    Escalate(RequestId, u8),
    AuthorityChanged,
}

struct AgentIface {
    tx: Sender<Cmd>,
    timeout_s: u64,
    home: Option<String>,
}

async fn authority_owner(conn: &Connection) -> Option<String> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = BusName::try_from(AUTHORITY_NAME).ok()?;
    dbus.get_name_owner(name).await.ok().map(|o| o.to_string())
}

async fn check_sender(conn: &Connection, hdr: &Header<'_>) -> Result<(), PolkitError> {
    let sender = hdr.sender().map(|s| s.as_str());
    let owner = authority_owner(conn).await;
    if sender_allowed(sender, owner.as_deref()) {
        Ok(())
    } else {
        tracing::warn!(?sender, "polkitd olmayan gönderici reddedildi");
        Err(PolkitError::Failed("Yalnızca polkitd çağırabilir".into()))
    }
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl AgentIface {
    #[allow(clippy::too_many_arguments)]
    async fn begin_authentication(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> Result<(), PolkitError> {
        // İsteyen etiketi bu ana aittir: bundan sonra başlamış süreç asıl istekçi olamaz.
        let arrival = request::boot_ticks_now();
        check_sender(conn, &hdr).await?;
        let uids = request::user_identities(&identities);
        if uids.is_empty() {
            return Err(PolkitError::Failed("Desteklenen kimlik yok".into()));
        }
        let request = DialogRequest {
            cookie,
            // Ham: diyalog kaçışlı ve ortadan kırparak gösterir; uzunluk validate'te sınırlı.
            action_id,
            message: text::sanitize(&message, text::MAX_MESSAGE_CHARS),
            icon_name: text::sanitize(&icon_name, text::MAX_SHORT_CHARS),
            identities: uids,
            requester: request::requester(
                &details,
                self.home.as_deref(),
                |pid| request::read_proc(pid, arrival),
                request::stat_path,
            ),
            timeout_s: self.timeout_s,
        };
        if let Err(e) = request.validate() {
            return Err(PolkitError::Failed(e.into()));
        }
        tracing::info!(action = request.action_id, "kimlik doğrulama isteği");
        let (tx, rx) = async_channel::bounded(1);
        self.tx
            .send(Cmd::Begin { request, reply: tx })
            .await
            .map_err(|_| PolkitError::Failed("ajan kapanıyor".into()))?;
        match rx.recv().await {
            Ok(Reply::Done) => Ok(()),
            Ok(Reply::Cancelled) => Err(PolkitError::Cancelled("Kimlik doğrulama iptal edildi".into())),
            Ok(Reply::Failed) | Err(_) => Err(PolkitError::Failed("Kimlik doğrulama başarısız".into())),
        }
    }

    async fn cancel_authentication(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        cookie: String,
    ) -> Result<(), PolkitError> {
        check_sender(conn, &hdr).await?;
        let (tx, rx) = async_channel::bounded(1);
        self.tx
            .send(Cmd::Cancel { cookie, reply: tx })
            .await
            .map_err(|_| PolkitError::Failed("ajan kapanıyor".into()))?;
        if rx.recv().await.unwrap_or(false) {
            Ok(())
        } else {
            Err(PolkitError::Failed("Bu çerezle bekleyen istek yok".into()))
        }
    }
}

/// Başarılı kayıt: oturum ve kaydı kabul eden polkitd'nin benzersiz adı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    pub session: String,
    /// Kaydı yanıtlayan benzersiz ad (yanıtın göndereni; yoksa kayıttan sonra
    /// `GetNameOwner`). Bilinmiyorsa `None`.
    pub owner: Option<String>,
}

/// `org.freedesktop.PolicyKit1` adı el değiştirdiğinde ne yapılacağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerChange {
    /// Kaydolduğumuz polkitd adı (yeniden) aldı — ör. bus etkinleştirmesinin
    /// `""→:1.N` sinyali kayıt yanıtından önce tampona düştü. Hiçbir şey yapılmaz.
    Ignore,
    /// polkitd gitti (yeni sahip boş): bekleyen istekler iptal, kayıt denenmez (kayıt
    /// çağrısı polkitd'yi yeniden etkinleştirip aynı yarışı doğururdu).
    CancelOnly,
    /// Başka bir polkitd geldi: istekler iptal ve yeniden kayıt.
    Reregister,
}

/// Saf karar: `new_owner` sinyaldeki yeni sahip, `registered` kaydı kabul eden sahip.
pub fn owner_change(new_owner: Option<&str>, registered: Option<&str>) -> OwnerChange {
    match new_owner.filter(|o| !o.is_empty()) {
        None => OwnerChange::CancelOnly,
        Some(o) if Some(o) == registered => OwnerChange::Ignore,
        Some(_) => OwnerChange::Reregister,
    }
}

/// polkitd'ye kaydolur (kendi oturumu, fallback yok).
pub async fn register(conn: &Connection, opts: &AgentOptions) -> Result<Registered, RegisterError> {
    let session = logind::own_session(conn, opts.own_pid, opts.own_uid, opts.env_session.as_deref())
        .await
        .map_err(RegisterError::Other)?;
    let mut props: HashMap<&str, Value> = HashMap::new();
    props.insert("session-id", Value::from(session.as_str()));
    let subject = ("unix-session", props);
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("fallback", Value::from(false));
    let proxy = zbus::Proxy::new(conn, AUTHORITY_NAME, AUTHORITY_PATH, AUTHORITY_IFACE)
        .await
        .map_err(|e| RegisterError::Other(e.to_string()))?;
    let path = zbus::zvariant::ObjectPath::try_from(OBJECT_PATH).map_err(|e| RegisterError::Other(e.to_string()))?;
    match proxy
        .call_method(
            "RegisterAuthenticationAgentWithOptions",
            &(subject, opts.locale.as_str(), path, options),
        )
        .await
    {
        Ok(reply) => {
            // Yanıtın göndereni kaydı gerçekten kabul eden süreçtir (yarışsız). Yoksa (olmamalı)
            // kayıttan sonra sorulan sahip.
            let owner = match reply.header().sender() {
                Some(s) => Some(s.to_string()),
                None => authority_owner(conn).await,
            };
            Ok(Registered { session, owner })
        }
        Err(zbus::Error::MethodError(_, msg, _)) => {
            Err(classify_register_error(msg.as_deref().unwrap_or_default()))
        }
        Err(e) => Err(RegisterError::Other(e.to_string())),
    }
}

/// `NameOwnerChanged` sinyalindeki yeni sahip (boşsa `None`).
fn signal_new_owner(sig: &zbus::fdo::NameOwnerChanged) -> Option<String> {
    let args = sig.args().ok()?;
    args.new_owner().as_ref().map(|o| o.to_string()).filter(|o| !o.is_empty())
}

async fn registration_loop(conn: Connection, opts: AgentOptions, tx: Sender<Cmd>) {
    let changes = async {
        let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
        dbus.receive_name_owner_changed_with_args(&[(0, AUTHORITY_NAME)]).await
    }
    .await;
    let mut changes = match changes {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("NameOwnerChanged izlenemiyor: {e}");
            return;
        }
    };
    let mut attempt = 0u32;
    loop {
        match register(&conn, &opts).await {
            Ok(reg) => {
                attempt = 0;
                tracing::info!(session = reg.session, owner = ?reg.owner, "polkit ajanı kaydedildi");
                // Kaydolduğumuz polkitd'den başka bir sahip gelene dek bekle. Kayıt çağrısı
                // polkitd'yi etkinleştirdiyse `""→sahip` sinyali yanıttan önce tampona düşmüş
                // olabilir: o sinyal yok sayılır (yoksa ikinci kayıt "already exists" alır ve
                // uyarı döngüsü başlardı).
                loop {
                    let Some(sig) = changes.next().await else {
                        return;
                    };
                    let new_owner = signal_new_owner(&sig);
                    match owner_change(new_owner.as_deref(), reg.owner.as_deref()) {
                        OwnerChange::Ignore => continue,
                        OwnerChange::CancelOnly => {
                            tracing::warn!("polkitd gitti; bekleyen istekler iptal");
                            let _ = tx.send(Cmd::AuthorityChanged).await;
                        }
                        OwnerChange::Reregister => {
                            tracing::warn!("polkitd değişti; istekler iptal, yeniden kayıt");
                            let _ = tx.send(Cmd::AuthorityChanged).await;
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                let delay = (opts.backoff)(attempt);
                attempt = attempt.saturating_add(1);
                match &e {
                    RegisterError::AlreadyExists => tracing::warn!(
                        "Bu oturuma başka bir polkit ajanı kayıtlı (ör. hyprpolkitagent); \
                         nuxut-polkit ona dokunmaz. {}s sonra yeniden denenecek",
                        delay.as_secs()
                    ),
                    RegisterError::Other(msg) => tracing::warn!(
                        "polkit ajanı kaydedilemedi: {msg}; {}s sonra yeniden denenecek",
                        delay.as_secs()
                    ),
                }
                // Bekleme yalnızca reddeden polkitd'den başka (boş olmayan) bir sahip gelince
                // erken biter; polkitd'nin gitmesi yalnızca iptal ettirir. Reddeden sahip kayıt
                // çağrısının etkinleştirdiği süreç olabilir: onun tampondaki `""→sahip`
                // sinyali yeni deneme tetiklemez.
                let seen = authority_owner(&conn).await;
                let mut timer = async_io::Timer::after(delay);
                loop {
                    let next = futures_lite::future::or(
                        async {
                            (&mut timer).await;
                            None
                        },
                        async { Some(changes.next().await) },
                    )
                    .await;
                    match next {
                        None => break,
                        Some(None) => return,
                        Some(Some(sig)) => {
                            let new_owner = signal_new_owner(&sig);
                            match owner_change(new_owner.as_deref(), seen.as_deref()) {
                                OwnerChange::Ignore => {}
                                OwnerChange::CancelOnly => {
                                    let _ = tx.send(Cmd::AuthorityChanged).await;
                                }
                                OwnerChange::Reregister => {
                                    let _ = tx.send(Cmd::AuthorityChanged).await;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

struct Manager {
    conn: Connection,
    opts: AgentOptions,
    tx: Sender<Cmd>,
    queue: Queue,
    next_id: RequestId,
    pending: HashMap<RequestId, (DialogRequest, Sender<Reply>)>,
    procs: HashMap<RequestId, DialogProcess>,
}

impl Manager {
    fn after(&self, delay: Duration, cmd: Cmd) {
        let tx = self.tx.clone();
        self.conn
            .executor()
            .spawn(
                async move {
                    async_io::Timer::after(delay).await;
                    let _ = tx.send(cmd).await;
                },
                "nuxut-polkit-timer",
            )
            .detach();
    }

    fn perform(&mut self, actions: Vec<Action>) {
        let mut todo = std::collections::VecDeque::from(actions);
        while let Some(action) = todo.pop_front() {
            match action {
                Action::Spawn(id) => {
                    let Some((request, _)) = self.pending.get(&id) else {
                        todo.extend(self.queue.spawn_failed(id));
                        continue;
                    };
                    let tx = self.tx.clone();
                    match DialogProcess::spawn(&self.opts.dialog, request, move |outcome| {
                        let _ = tx.send_blocking(Cmd::Exited { id, outcome });
                    }) {
                        Ok(p) => {
                            tracing::info!(pid = p.pid(), "diyalog başlatıldı");
                            self.procs.insert(id, p);
                            self.after(self.opts.backstop, Cmd::Deadline(id));
                        }
                        Err(e) => {
                            tracing::error!("diyalog başlatılamadı: {e}");
                            todo.extend(self.queue.spawn_failed(id));
                        }
                    }
                }
                Action::Kill(id) => {
                    if let Some(p) = self.procs.get(&id) {
                        p.terminate();
                        self.after(self.opts.kill_grace, Cmd::Escalate(id, 1));
                    }
                }
                Action::Reply(id, reply) => {
                    if let Some((_, tx)) = self.pending.remove(&id) {
                        tracing::info!(?reply, "polkitd'ye yanıt");
                        let _ = tx.try_send(reply);
                    }
                }
            }
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Begin { request, reply } => {
                let id = self.next_id;
                self.next_id += 1;
                match self.queue.begin(id, &request.cookie) {
                    Ok(actions) => {
                        self.pending.insert(id, (request, reply));
                        self.perform(actions);
                    }
                    Err(e) => {
                        tracing::warn!(?e, "istek reddedildi");
                        let _ = reply.try_send(match e {
                            Reject::DuplicateCookie | Reject::QueueFull => Reply::Failed,
                        });
                    }
                }
            }
            Cmd::Cancel { cookie, reply } => {
                let found = self.queue.cancel(&cookie);
                let _ = reply.try_send(found.is_some());
                if let Some(actions) = found {
                    tracing::info!("polkitd isteği iptal etti");
                    self.perform(actions);
                }
            }
            Cmd::Exited { id, outcome } => {
                tracing::info!(?outcome, "diyalog bitti");
                self.procs.remove(&id);
                let actions = self.queue.dialog_exited(id, outcome);
                self.perform(actions);
            }
            Cmd::Deadline(id) => {
                let actions = self.queue.deadline(id);
                if !actions.is_empty() {
                    tracing::warn!("diyalog süre sınırını aştı; sonlandırılıyor");
                }
                self.perform(actions);
            }
            Cmd::Escalate(id, stage) => {
                if self.queue.active() != Some(id) {
                    return;
                }
                if stage == 1 {
                    if let Some(p) = self.procs.get(&id) {
                        tracing::warn!("diyalog SIGTERM'e yanıt vermedi; SIGKILL");
                        p.kill();
                    }
                    self.after(self.opts.kill_grace, Cmd::Escalate(id, 2));
                } else {
                    tracing::error!("diyalog çıkışı raporlanmadı; yuva boşaltılıyor");
                    self.procs.remove(&id);
                    let actions = self.queue.dialog_exited(id, DialogOutcome::Failed);
                    self.perform(actions);
                }
            }
            Cmd::AuthorityChanged => {
                let actions = self.queue.authority_gone();
                self.perform(actions);
            }
        }
    }
}

/// Ajanı çalıştırır: arayüzü `OBJECT_PATH`'te sunar, kaydolur, istekleri işler. Bağlantı
/// kapanana dek döner; dönerse çalışan diyaloglar sonlandırılır.
pub async fn run(conn: Connection, opts: AgentOptions) -> zbus::Result<()> {
    let (tx, rx): (Sender<Cmd>, Receiver<Cmd>) = async_channel::unbounded();
    let iface = AgentIface {
        tx: tx.clone(),
        timeout_s: opts.timeout.as_secs().max(1),
        home: opts.home.clone(),
    };
    conn.object_server().at(OBJECT_PATH, iface).await?;
    conn.executor()
        .spawn(
            registration_loop(conn.clone(), opts.clone(), tx.clone()),
            "nuxut-polkit-register",
        )
        .detach();
    let mut manager = Manager {
        conn,
        opts,
        tx,
        queue: Queue::new(),
        next_id: 1,
        pending: HashMap::new(),
        procs: HashMap::new(),
    };
    while let Ok(cmd) = rx.recv().await {
        manager.handle(cmd);
    }
    for p in manager.procs.values() {
        p.terminate();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_check() {
        assert!(sender_allowed(Some(":1.5"), Some(":1.5")));
        assert!(!sender_allowed(Some(":1.6"), Some(":1.5")));
        assert!(!sender_allowed(None, Some(":1.5")));
        assert!(!sender_allowed(Some(":1.5"), None));
        assert!(!sender_allowed(None, None));
        // İyi bilinen ad sahibin yerine geçmez.
        assert!(!sender_allowed(
            Some("org.freedesktop.PolicyKit1"),
            Some("org.freedesktop.PolicyKit1")
        ));
    }

    #[test]
    fn owner_change_decisions() {
        // polkitd gitti: yalnızca iptal (kayıt yok).
        assert_eq!(owner_change(None, Some(":1.5")), OwnerChange::CancelOnly);
        assert_eq!(owner_change(Some(""), Some(":1.5")), OwnerChange::CancelOnly);
        assert_eq!(owner_change(None, None), OwnerChange::CancelOnly);
        // Kaydolduğumuz sahibin (etkinleştirme) sinyali: yok say.
        assert_eq!(owner_change(Some(":1.5"), Some(":1.5")), OwnerChange::Ignore);
        // Başka polkitd: yeniden kayıt.
        assert_eq!(owner_change(Some(":1.9"), Some(":1.5")), OwnerChange::Reregister);
        assert_eq!(owner_change(Some(":1.9"), None), OwnerChange::Reregister);
    }

    #[test]
    fn backoff_schedule() {
        assert_eq!(backoff(0), Duration::from_secs(30));
        assert_eq!(backoff(1), Duration::from_secs(120));
        assert_eq!(backoff(2), Duration::from_secs(300));
        assert_eq!(backoff(50), Duration::from_secs(300));
    }

    #[test]
    fn register_error_kinds() {
        assert_eq!(
            classify_register_error("An authentication agent already exists for the given subject"),
            RegisterError::AlreadyExists
        );
        assert!(matches!(
            classify_register_error("Cannot determine user of subject"),
            RegisterError::Other(_)
        ));
    }

    #[test]
    fn locale_selection() {
        let env = |m: &'static [(&'static str, &'static str)]| {
            move |k: &str| m.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert_eq!(locale_from_env(env(&[("LANG", "tr_TR.UTF-8")])), "tr_TR.UTF-8");
        assert_eq!(
            locale_from_env(env(&[("LANG", "tr_TR.UTF-8"), ("LC_MESSAGES", "en_US.UTF-8")])),
            "en_US.UTF-8"
        );
        assert_eq!(locale_from_env(env(&[("LC_ALL", ""), ("LANG", "de_DE")])), "de_DE");
        assert_eq!(locale_from_env(env(&[("LANG", "a b")])), "C");
        assert_eq!(locale_from_env(env(&[])), "C");
    }
}
