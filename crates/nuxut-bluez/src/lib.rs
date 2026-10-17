//! Bluetooth: BlueZ (`org.bluez`, sistem bus'ı) — adaptör aç/kapa, istek üzerine zaman aşımlı
//! tarama, cihazlar (ad, ikon, bağlı, eşleşmiş, pil), bağlan/kopar/eşleştir/unut ve
//! eşleştirme ajanı.
//!
//! **Durum.** BlueZ'in ObjectManager'ı (`/`) bir kez `GetManagedObjects` ile okunur;
//! `InterfacesAdded`/`InterfacesRemoved` ve `PropertiesChanged` ile güncellenir. Cihaz sayısı
//! [`MAX_DEVICES`] ile sınırlı. Taramada RSSI sık değişir: anlık görüntü en fazla
//! [`BluetoothOptions::throttle`] aralıkla yayınlanır. BlueZ düşüp geri gelirse durum
//! temizlenir, yeniden okunur ve ajan yeniden kaydedilir.
//!
//! **Ajan.** `org.bluez.Agent1` nesnesi ([`BluetoothOptions::agent_path`], kabukta
//! [`AGENT_PATH`]) `KeyboardDisplay` yeteneğiyle kaydedilir ve varsayılan ajan istenir; yol
//! `None` ise (varsayılan; Ayarlar) ajan hiç yayınlanmaz/kaydedilmez. Kabuk
//! [`BluetoothOptions::with_agent`] kullanır.
//! PIN/passkey/onay istekleri UI'a [`BluetoothEvent::Pairing`] olarak verilir, cevap [`BluetoothService::reply`] ile beklenir;
//! [`BluetoothOptions::agent_timeout`] içinde cevap gelmezse istek reddedilir. Ajan çağrıları
//! yalnızca BlueZ'in şu anki benzersiz sahibinden kabul edilir; başka bir istemcinin çağrısı
//! `Rejected` alır.
//!
//! **Eylemler** ayrı görevlerde çalışır (BlueZ `Connect`/`Pair` onlarca saniye sürebilir);
//! döngü beklemez. Hata [`BluetoothEvent::Failed`] olarak gelir; tanınan `org.bluez.Error.*`
//! adları türdür ([`BluezError::AuthenticationRejected`] … — `Pair` her zaman denenir, sonuç
//! türüne göre gösterilir). Aşama 2 ayarları: adaptör adı/görünürlük (süreli)/eşleştirilebilirlik,
//! cihaz güven/engel/ad; başarı durumda (`Properties.Set` → `PropertiesChanged`) görünür.
//!
//! **Bus.** Varsayılan sistem bus'ı; sahte BlueZ (`examples/fake_bluez`, `dbus-run-session`
//! içinde) ile canlı testte `bus_from_env()` (`NUXUT_BLUEZ_BUS=session`; yalnız `test-support`
//! özelliği, üründe derlenmez).
//!
//! # Ayarlar kullanım kalıbı
//!
//! Ayarlar ajan **olmaz** (varsayılan [`BluetoothOptions`] ajansızdır): eşleştirme onayı kabuğundur.
//! `sink` olayı paylaşılan kuyruğa koyar ve UI döngüsünü uyandırır; ayrı `notify` kancası yok:
//!
//! ```no_run
//! use std::sync::{Arc, Mutex};
//! use nuxut_bluez::{BluetoothEvent, BluetoothOptions, BluetoothService};
//!
//! let inbox: Arc<Mutex<Vec<BluetoothEvent>>> = Arc::default();
//! let wake = || { /* ör. calloop::ping::Ping::ping(&ping) */ };
//! // Canlı testte (dev-dependency, `test-support`): `bus: nuxut_bluez::bus_from_env()`.
//! let options = BluetoothOptions::default();
//! let queue = Arc::clone(&inbox);
//! let bt = BluetoothService::spawn(options, move |event| {
//!     queue.lock().unwrap().push(event);
//!     wake();
//! })?;
//! bt.set_discoverable("/org/bluez/hci0", true, 120);
//! # Ok::<(), std::io::Error>(())
//! ```

mod model;

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_channel::Sender;
use futures_lite::future::FutureExt;
use zbus::message::Header;
use zbus::zvariant::{ObjectPath, Value};
use zbus::{Connection, Message};

pub use nuxut_dbus::{BusKind, RequestId, Unavailable};
pub use model::{
    AgentStatus, Adapter, BluetoothState, Device, MAX_ADAPTERS, MAX_DEVICES, PowerState, Rfkill, read_rfkill, valid_passkey,
    valid_pin,
};

use nuxut_dbus::{
    Backoff, NameOwner, ServiceContext, ServiceThread, clean_str, close, connect, forward, get_name_owner,
    owner_changes, parse_owner_change, set_property, signals_from, tick_connection,
};
use nuxut_dbus::{apply_object_signal, get_managed_objects};
use model::{ADAPTER_IFACE, DEVICE_IFACE, Objects};

const BLUEZ: &str = "org.bluez";
const AGENT_MANAGER_IFACE: &str = "org.bluez.AgentManager1";
/// Ajan nesnesinin yolu.
pub const AGENT_PATH: &str = "/org/nuxut/shell/BluetoothAgent";
const INPUT_CAPACITY: usize = 512;
/// `GetManagedObjects`'te kabul edilen en fazla nesne.
const MAX_OBJECTS: usize = 1024;
/// Aynı anda bekleyen en fazla ajan isteği.
const MAX_PENDING: usize = 4;

/// Eşleştirme isteğinin türü.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairingKind {
    /// PIN girilmeli → [`PairingReply::Pin`].
    PinCode,
    /// 6 haneli passkey girilmeli → [`PairingReply::Passkey`].
    Passkey,
    /// Cihazda görünen sayı bu mu? → `Accept`/`Reject`.
    Confirm { passkey: u32 },
    /// Bu PIN'i cihaza girin (yalnızca bilgi; cevap gerekmez).
    DisplayPinCode { pin: String },
    /// Bu passkey'i cihaza girin; `entered` girilen hane sayısı (bilgi).
    DisplayPasskey { passkey: u32, entered: u16 },
    /// Eşleşmemiş cihaz bağlanmak istiyor → `Accept`/`Reject`.
    Authorize,
    /// Cihaz bir servise erişmek istiyor → `Accept`/`Reject`.
    AuthorizeService { uuid: String },
}

impl PairingKind {
    /// UI'dan cevap bekleniyor mu?
    pub fn needs_reply(&self) -> bool {
        !matches!(self, Self::DisplayPinCode { .. } | Self::DisplayPasskey { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingRequest {
    pub id: u64,
    /// Cihaz yolu ([`Device::path`]).
    pub device: String,
    /// Cihazın görünen adı (bilinmiyorsa yol).
    pub name: String,
    pub kind: PairingKind,
}

/// UI'ın eşleştirme cevabı.
#[derive(Clone, PartialEq, Eq)]
pub enum PairingReply {
    Pin(String),
    Passkey(u32),
    Accept,
    Reject,
}

impl std::fmt::Debug for PairingReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // PIN/passkey loglanmaz.
        match self {
            Self::Pin(_) => f.write_str("Pin(<gizli>)"),
            Self::Passkey(_) => f.write_str("Passkey(<gizli>)"),
            Self::Accept => f.write_str("Accept"),
            Self::Reject => f.write_str("Reject"),
        }
    }
}

/// Eylem hatası. Crate UI'sızdır: kullanıcıya gösterilecek metni çağıran üretir (kabukta
/// `shell-bluetooth-error-*` anahtarları). `Display` yalnız log içindir.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BluezError {
    /// D-Bus çağrısı başarısız; BlueZ'in hata metni (temizlenmiş, en fazla 200 bayt).
    CallFailed(String),
    /// Çağrı zaman aşımına uğradı ([`BluetoothOptions::action_timeout`]).
    Timeout,
    /// Adaptör yok.
    NoAdapter,
    /// BlueZ çalışmıyor (bus'ta sahibi yok).
    BluezUnavailable,
    /// Bilinmeyen cihaz.
    UnknownDevice,
    /// Eşleştirme reddedildi (`org.bluez.Error.AuthenticationRejected`: kullanıcı ya da
    /// cihaz reddetti).
    AuthenticationRejected,
    /// Eşleştirme başarısız (`AuthenticationFailed`: yanlış PIN/passkey, ajan yok…).
    AuthenticationFailed,
    /// Eşleştirme iptal edildi (`AuthenticationCanceled`).
    AuthenticationCanceled,
    /// Eşleştirme zaman aşımı (`AuthenticationTimeout`: onay gelmedi).
    AuthenticationTimeout,
    /// Cihaza bağlanılamadı (`ConnectionAttemptFailed`: kapsam dışı, kapalı).
    ConnectionAttemptFailed,
    /// Zaten var (`AlreadyExists`: ör. zaten eşleşmiş).
    AlreadyExists,
    /// Aynı işlem sürüyor (`InProgress`).
    InProgress,
    /// Adaptör hazır değil (`NotReady`: kapalı).
    NotReady,
    /// Geçersiz değer (ad çok uzun vb.).
    InvalidValue,
    /// Adaptör rfkill ile engelli (`org.bluez.Error.Blocked`; `Powered` yazılamadı). `hard`:
    /// sysfs'e göre donanım anahtarı (`Some(true)`: yazılım kaldıramaz), `Some(false)`: yalnız
    /// yazılım engeli, `None`: sysfs okunamadı.
    RfkillBlocked { hard: Option<bool> },
}

impl BluezError {
    /// BlueZ hata adının türü (`org.bluez.Error.<ad>`); tanınmayan ad `None`.
    pub fn from_dbus_name(name: &str) -> Option<Self> {
        Some(match name.strip_prefix("org.bluez.Error.")? {
            "AuthenticationRejected" => Self::AuthenticationRejected,
            "AuthenticationFailed" => Self::AuthenticationFailed,
            "AuthenticationCanceled" => Self::AuthenticationCanceled,
            "AuthenticationTimeout" => Self::AuthenticationTimeout,
            "ConnectionAttemptFailed" => Self::ConnectionAttemptFailed,
            "AlreadyExists" => Self::AlreadyExists,
            "InProgress" => Self::InProgress,
            "NotReady" => Self::NotReady,
            "Blocked" => Self::RfkillBlocked { hard: None },
            _ => return None,
        })
    }
}

/// Redakte: `CallFailed` metni cihaz adı/adresi içerebilir (log politikası).
impl std::fmt::Debug for BluezError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CallFailed(_) => f.write_str("CallFailed(<redacted>)"),
            Self::Timeout => f.write_str("Timeout"),
            Self::NoAdapter => f.write_str("NoAdapter"),
            Self::BluezUnavailable => f.write_str("BluezUnavailable"),
            Self::UnknownDevice => f.write_str("UnknownDevice"),
            Self::AuthenticationRejected => f.write_str("AuthenticationRejected"),
            Self::AuthenticationFailed => f.write_str("AuthenticationFailed"),
            Self::AuthenticationCanceled => f.write_str("AuthenticationCanceled"),
            Self::AuthenticationTimeout => f.write_str("AuthenticationTimeout"),
            Self::ConnectionAttemptFailed => f.write_str("ConnectionAttemptFailed"),
            Self::AlreadyExists => f.write_str("AlreadyExists"),
            Self::InProgress => f.write_str("InProgress"),
            Self::NotReady => f.write_str("NotReady"),
            Self::RfkillBlocked { hard } => write!(f, "RfkillBlocked {{ hard: {hard:?} }}"),
            Self::InvalidValue => f.write_str("InvalidValue"),
        }
    }
}

impl std::fmt::Display for BluezError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CallFailed(detail) => write!(f, "çağrı başarısız: {detail}"),
            Self::Timeout => f.write_str("zaman aşımı"),
            Self::NoAdapter => f.write_str("adaptör yok"),
            Self::BluezUnavailable => f.write_str("BlueZ çalışmıyor"),
            Self::UnknownDevice => f.write_str("bilinmeyen cihaz"),
            Self::AuthenticationRejected => f.write_str("org.bluez.Error.AuthenticationRejected"),
            Self::AuthenticationFailed => f.write_str("org.bluez.Error.AuthenticationFailed"),
            Self::AuthenticationCanceled => f.write_str("org.bluez.Error.AuthenticationCanceled"),
            Self::AuthenticationTimeout => f.write_str("org.bluez.Error.AuthenticationTimeout"),
            Self::ConnectionAttemptFailed => f.write_str("org.bluez.Error.ConnectionAttemptFailed"),
            Self::AlreadyExists => f.write_str("org.bluez.Error.AlreadyExists"),
            Self::InProgress => f.write_str("org.bluez.Error.InProgress"),
            Self::NotReady => f.write_str("org.bluez.Error.NotReady"),
            Self::InvalidValue => f.write_str("geçersiz değer"),
            Self::RfkillBlocked { .. } => f.write_str("org.bluez.Error.Blocked"),
        }
    }
}

#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub enum BluetoothEvent {
    /// Durum (tam anlık görüntü).
    State(BluetoothState),
    /// Ajan isteği; cevap gerekiyorsa [`BluetoothService::reply`].
    Pairing(PairingRequest),
    /// İstek bitti (cevaplandı, BlueZ iptal etti ya da zaman aşımı): UI diyaloğu kapatır.
    PairingDone { id: u64 },
    /// Eylem başarısız. `request`: isteğin kimliği (eylem metodunun döndürdüğü; eşleştirme
    /// sonrası otomatik güven/bağlan adımlarında `None`).
    Failed { request: Option<RequestId>, device: Option<String>, action: &'static str, error: BluezError },
    /// Eylem başarılı (BlueZ çağrıyı tamamladı; `pair`: eşleşme kuruldu). O ana dek alınan
    /// durum değişikliği bu olaydan önce [`BluetoothEvent::State`] olarak yayınlanır.
    Done { request: RequestId, device: Option<String>, action: &'static str },
}

/// Redakte: cihaz adı/adresi ve passkey yazılmaz (log politikası).
impl std::fmt::Debug for BluetoothEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(s) => f
                .debug_struct("State")
                .field("available", &s.available)
                .field("unavailable", &s.unavailable)
                .field("adapters", &s.adapters.len())
                .field("devices", &s.devices.len())
                .finish(),
            Self::Pairing(r) => f.debug_struct("Pairing").field("id", &r.id).finish_non_exhaustive(),
            Self::PairingDone { id } => f.debug_struct("PairingDone").field("id", id).finish(),
            Self::Failed { request, action, .. } => {
                f.debug_struct("Failed").field("request", request).field("action", action).finish_non_exhaustive()
            }
            Self::Done { request, action, .. } => {
                f.debug_struct("Done").field("request", request).field("action", action).finish_non_exhaustive()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct BluetoothOptions {
    /// Hangi bus (varsayılan sistem). Sahte BlueZ ile canlı testte [`bus_from_env`] ile seçilir.
    pub bus: BusKind,
    /// D-Bus adresi (testler); verilirse [`BluetoothOptions::bus`] yerine bu adrese bağlanılır.
    pub bus_address: Option<String>,
    pub call_timeout: Duration,
    /// `Connect`/`Pair` gibi uzun çağrıların zaman aşımı.
    pub action_timeout: Duration,
    /// Taramanın kendiliğinden durduğu süre.
    pub discovery_timeout: Duration,
    /// Ajan isteğine UI cevabı için süre.
    pub agent_timeout: Duration,
    /// Durum yayını en sık bu aralıkla.
    pub throttle: Duration,
    pub max_backoff: Duration,
    /// Eşleştirme ajanının nesne yolu; `None` (**varsayılan**): ajan yayınlanmaz,
    /// `RegisterAgent` / `RequestDefaultAgent` çağrılmaz ([`BluetoothEvent::Pairing`] hiç
    /// gelmez). Varsayılan ajan **yalnız kabuktur**: kabuk [`BluetoothOptions::with_agent`] ile
    /// açıkça ister; Ayarlar varsayılanı kullanır — unutsa bile kabuğun ajanını alamaz.
    pub agent_path: Option<String>,
    /// rfkill engelini okumak için sysfs dizini (salt okunur; varsayılan `/sys/class/rfkill`,
    /// testlerde geçici dizin; `None`: okunmaz).
    pub rfkill_dir: Option<std::path::PathBuf>,
}

impl Default for BluetoothOptions {
    /// Sistem bus'ı, **ajansız** (güvenli varsayılan; 2026-10-13).
    fn default() -> Self {
        Self {
            bus: BusKind::System,
            bus_address: None,
            call_timeout: Duration::from_secs(3),
            action_timeout: Duration::from_secs(40),
            discovery_timeout: Duration::from_secs(30),
            agent_timeout: Duration::from_secs(60),
            throttle: Duration::from_millis(100),
            max_backoff: Duration::from_secs(30),
            agent_path: None,
            rfkill_dir: Some(std::path::PathBuf::from("/sys/class/rfkill")),
        }
    }
}

impl BluetoothOptions {
    /// Ajanlı seçenekler (yalnız kabuk): ajan [`AGENT_PATH`]'te, varsayılan ajan istenir.
    pub fn with_agent() -> Self {
        Self { agent_path: Some(AGENT_PATH.to_owned()), ..Self::default() }
    }
}

/// Sahte BlueZ ile canlı test anahtarı (`bus_from_env`); yalnız `test-support`.
#[cfg(feature = "test-support")]
pub const BUS_ENV: &str = "NUXUT_BLUEZ_BUS";

/// `NUXUT_BLUEZ_BUS=session` ise oturum bus'ı, değilse sistem bus'ı. Yalnız `test-support`
/// özelliğiyle derlenir (üründe yok); çağıran açıkça kullanır.
#[cfg(feature = "test-support")]
pub fn bus_from_env() -> BusKind {
    BusKind::from_env(BUS_ENV)
}

#[derive(Debug)]
enum Command {
    SetPowered(bool),
    StartDiscovery,
    StopDiscovery,
    Connect(String),
    Disconnect(String),
    Pair(String),
    Forget(String),
    SetTrusted(String, bool),
    SetBlocked(String, bool),
    SetDeviceAlias(String, String),
    SetAdapterAlias(String, String),
    SetDiscoverable { adapter: String, on: bool, timeout: u32 },
    SetPairable(String, bool),
    CancelPairing(String),
    Reply { id: u64, reply: PairingReply },
}

/// Çalışan Bluetooth servisi. Düşürülünce temiz kapanır (tarama durur, ajan bırakılır).
#[derive(Debug)]
pub struct BluetoothService {
    thread: ServiceThread<Request>,
    next: AtomicU64,
}

/// Kuyruktaki istek: kimlik + komut.
#[derive(Debug)]
struct Request {
    id: RequestId,
    command: Command,
}

impl BluetoothService {
    pub fn spawn<F>(options: BluetoothOptions, sink: F) -> io::Result<Self>
    where
        F: Fn(BluetoothEvent) + Send + 'static,
    {
        let thread = ServiceThread::spawn("nuxut-bluetooth", 64, move |ctx| run(options, sink, ctx))?;
        Ok(Self { thread, next: AtomicU64::new(1) })
    }

    fn send(&self, command: Command) -> Option<RequestId> {
        let id = RequestId(self.next.fetch_add(1, Ordering::Relaxed));
        self.thread.send(Request { id, command }).then_some(id)
    }

    /// Tüm adaptörleri açar/kapatır.
    pub fn set_powered(&self, on: bool) -> Option<RequestId> {
        self.send(Command::SetPowered(on))
    }

    /// Taramayı başlatır; [`BluetoothOptions::discovery_timeout`] sonra kendiliğinden durur.
    /// Yalnızca panel açıkken çağrılmalı.
    pub fn start_discovery(&self) -> Option<RequestId> {
        self.send(Command::StartDiscovery)
    }

    pub fn stop_discovery(&self) -> Option<RequestId> {
        self.send(Command::StopDiscovery)
    }

    pub fn connect(&self, device: &str) -> Option<RequestId> {
        self.send(Command::Connect(device.to_owned()))
    }

    pub fn disconnect(&self, device: &str) -> Option<RequestId> {
        self.send(Command::Disconnect(device.to_owned()))
    }

    /// Eşleştirir; başarılıysa güvenilir işaretleyip bağlanır.
    pub fn pair(&self, device: &str) -> Option<RequestId> {
        self.send(Command::Pair(device.to_owned()))
    }

    /// Eşleşmeyi siler (`Adapter1.RemoveDevice`).
    pub fn forget(&self, device: &str) -> Option<RequestId> {
        self.send(Command::Forget(device.to_owned()))
    }

    pub fn set_trusted(&self, device: &str, trusted: bool) -> Option<RequestId> {
        self.send(Command::SetTrusted(device.to_owned(), trusted))
    }

    /// Cihazı engeller/engeli kaldırır (`Device1.Blocked`; engelli cihaz bağlanamaz).
    pub fn set_blocked(&self, device: &str, blocked: bool) -> Option<RequestId> {
        self.send(Command::SetBlocked(device.to_owned(), blocked))
    }

    /// Cihazın görünen adı (`Device1.Alias`; boş: cihazın kendi adına döner).
    pub fn set_device_alias(&self, device: &str, alias: &str) -> Option<RequestId> {
        self.send(Command::SetDeviceAlias(device.to_owned(), alias.to_owned()))
    }

    /// Adaptörün (bu bilgisayarın) Bluetooth adı (`Adapter1.Alias`; boş: sistem adı).
    pub fn set_adapter_alias(&self, adapter: &str, alias: &str) -> Option<RequestId> {
        self.send(Command::SetAdapterAlias(adapter.to_owned(), alias.to_owned()))
    }

    /// Görünürlük: önce `DiscoverableTimeout` (saniye; 0 = süresiz), sonra `Discoverable`.
    /// Süre dolunca BlueZ kendisi kapatır; durum [`Adapter::discoverable`] ile gelir.
    pub fn set_discoverable(&self, adapter: &str, on: bool, timeout_secs: u32) -> Option<RequestId> {
        self.send(Command::SetDiscoverable { adapter: adapter.to_owned(), on, timeout: timeout_secs })
    }

    /// Eşleştirilebilirlik (`Adapter1.Pairable`).
    pub fn set_pairable(&self, adapter: &str, on: bool) -> Option<RequestId> {
        self.send(Command::SetPairable(adapter.to_owned(), on))
    }

    /// Süren eşleştirmeyi iptal eder (`Device1.CancelPairing`); `pair` çağrısı
    /// `Failed { action: "pair", error: AuthenticationCanceled }` ile biter.
    pub fn cancel_pairing(&self, device: &str) -> Option<RequestId> {
        self.send(Command::CancelPairing(device.to_owned()))
    }

    /// Ajan isteğine cevap.
    pub fn reply(&self, id: u64, reply: PairingReply) -> Option<RequestId> {
        self.send(Command::Reply { id, reply })
    }
}

#[derive(Debug)]
enum Input {
    Owner,
    Ignore,
    Signal(Message),
    Agent(AgentCall),
    AgentCancel,
    Failed { request: Option<RequestId>, device: Option<String>, action: &'static str, error: BluezError },
    Done { request: RequestId, device: Option<String>, action: &'static str },
    Paired(String),
    /// Tarama çağrısı başarısız; `generation`: isteğin kuşağı (eski cevap yeni taramayı etkilemez).
    DiscoveryFailed { request: Option<RequestId>, generation: u64, error: BluezError },
}

#[derive(Debug)]
struct AgentCall {
    id: u64,
    device: String,
    kind: PairingKind,
    reply: Sender<PairingReply>,
}

enum End {
    Stop,
    Lost,
}

enum Wake {
    Stop,
    Lost,
    Input(Input),
    Command(Request),
    Timer,
}

async fn run<F: Fn(BluetoothEvent)>(options: BluetoothOptions, sink: F, ctx: ServiceContext<Request>) {
    let mut backoff = Backoff::new(options.max_backoff);
    let ids = Arc::new(AtomicU64::new(1));
    let mut unreachable_sent = false;
    let agent_path = options.agent_path.as_deref().and_then(|p| match ObjectPath::try_from(p) {
        Ok(p) => Some(p.into_owned()),
        Err(_) => {
            tracing::warn!("bluetooth: geçersiz ajan yolu, ajan kaydedilmeyecek");
            None
        }
    });
    loop {
        match connect(options.bus, options.bus_address.as_deref(), options.call_timeout).await {
            Ok(conn) => {
                backoff.reset();
                let ticker = tick_connection(&ctx.ex, &conn);
                let mut rt = Runtime {
                    sink: &sink,
                    options: &options,
                    conn: conn.clone(),
                    bluez: NameOwner::new(),
                    agent_path: agent_path.clone(),
                    agent_owner: Arc::new(Mutex::new(None)),
                    objects: model::new_objects(),
                    pending: HashMap::new(),
                    discovery_until: None,
                    discovery_gen: 0,
                    dirty: None,
                    emitted: None,
                    agent_status: AgentStatus::default(),
                    current: None,
                };
                let end = rt.session(&ctx, &ids).await;
                if rt.emitted.is_some() {
                    unreachable_sent = false;
                }
                rt.cancel_pending();
                drop(rt);
                drop(ticker);
                close(conn).await;
                if matches!(end, End::Stop) {
                    return;
                }
                tracing::warn!("bluetooth: sistem bus bağlantısı koptu, yeniden bağlanılacak");
            }
            Err(e) => tracing::warn!("bluetooth: sistem bus'ına bağlanılamadı: {e}"),
        }
        // Bus yok: ayrı neden, aynı durum iki kez yayınlanmaz (yeniden denemelerde).
        if !unreachable_sent {
            unreachable_sent = true;
            sink(BluetoothEvent::State(BluetoothState { unavailable: Some(Unavailable::BusUnreachable), ..BluetoothState::default() }));
        }
        if !ctx.sleep(backoff.next_delay()).await {
            return;
        }
    }
}

struct Runtime<'a, F> {
    sink: &'a F,
    options: &'a BluetoothOptions,
    conn: Connection,
    bluez: NameOwner,
    /// Ajan nesnesinin yolu (`None`: ajansız istemci).
    agent_path: Option<ObjectPath<'static>>,
    /// Ajan nesnesinin gördüğü BlueZ sahibi (ajan çağrısı doğrulaması).
    agent_owner: Arc<Mutex<Option<String>>>,
    objects: Objects,
    /// Cevap bekleyen ajan istekleri.
    pending: HashMap<u64, Sender<PairingReply>>,
    discovery_until: Option<Instant>,
    /// Her başlat/durdur isteğinde artar (review notu d).
    discovery_gen: u64,
    dirty: Option<Instant>,
    emitted: Option<BluetoothState>,
    /// Bu istemcinin ajan kaydı (kabuk; ajansız istemcide hep `false`).
    agent_status: AgentStatus,
    /// İşlenen isteğin kimliği (komut işlenirken; sonuç olaylarına taşınır).
    current: Option<RequestId>,
}

impl<F: Fn(BluetoothEvent)> Runtime<'_, F> {
    async fn session(&mut self, ctx: &ServiceContext<Request>, ids: &Arc<AtomicU64>) -> End {
        let (tx, rx) = async_channel::bounded::<Input>(INPUT_CAPACITY);
        let agent = Agent {
            input: tx.clone(),
            owner: Arc::clone(&self.agent_owner),
            ids: Arc::clone(ids),
            timeout: self.options.agent_timeout,
        };
        if let Some(path) = self.agent_path.clone()
            && let Err(e) = self.conn.object_server().at(path, agent).await
        {
            tracing::warn!("bluetooth: ajan nesnesi yayınlanamadı: {e}");
        }
        let streams = async {
            Ok::<_, zbus::Error>((
                owner_changes(&self.conn, BLUEZ).await?,
                signals_from(&self.conn, BLUEZ, None, None).await?,
            ))
        };
        let (owners, signals) = match streams.await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("bluetooth: sinyal aboneliği kurulamadı: {e}");
                return End::Lost;
            }
        };
        let _tasks = [
            forward(&ctx.ex, owners, tx.clone(), |m| {
                if parse_owner_change(&m, BLUEZ).is_some() { Input::Owner } else { Input::Ignore }
            }),
            forward(&ctx.ex, signals, tx.clone(), Input::Signal),
        ];
        let owner = get_name_owner(&self.conn, BLUEZ).await;
        self.set_owner(owner).await;

        loop {
            let deadline = match (self.dirty, self.discovery_until) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let wake = async { rx.recv().await.map_or(Wake::Lost, Wake::Input) }
                .or(async { ctx.commands.recv().await.map_or(Wake::Stop, Wake::Command) })
                .or(async { ctx.stop.recv().await.map_or(Wake::Stop, |()| Wake::Stop) })
                .or(async {
                    match deadline {
                        Some(at) => {
                            async_io::Timer::at(at).await;
                        }
                        None => std::future::pending::<()>().await,
                    }
                    Wake::Timer
                })
                .await;
            match wake {
                Wake::Stop => return End::Stop,
                Wake::Lost => return End::Lost,
                Wake::Timer => {
                    let now = Instant::now();
                    if self.discovery_until.is_some_and(|t| t <= now) {
                        self.discovery_until = None;
                        self.discovery(ctx, &tx, false);
                    }
                    if self.dirty.is_some_and(|t| t <= now) {
                        self.flush();
                    }
                }
                Wake::Command(c) => self.command(ctx, &tx, c),
                Wake::Input(i) => self.input(ctx, &tx, i).await,
            }
        }
    }

    fn mark_dirty(&mut self) {
        if self.dirty.is_none() {
            self.dirty = Some(Instant::now() + self.options.throttle);
        }
    }

    fn state(&self) -> BluetoothState {
        let mut s = model::state(&self.objects, self.bluez.owner.is_some());
        s.own_discovery = self.discovery_until.is_some();
        s.agent = self.agent_status;
        // sysfs yalnız bir adaptör rfkill ile engelliyken okunur (boşta okuma yok).
        if s.adapters.iter().any(|a| a.power_state == PowerState::Blocked) {
            s.rfkill = self.options.rfkill_dir.as_deref().and_then(read_rfkill);
        }
        s
    }

    /// rfkill hatasını tanır ve sysfs'ten yumuşak/sert ayrımını ekler. Kaynaklar: BlueZ'in
    /// `org.bluez.Error.Blocked` adı (adapter.c `MGMT_STATUS_RFKILLED`) ya da — ad başka gelse
    /// bile — `Powered` yazımı başarısızken adaptörün `PowerState` değeri `off-blocked`.
    fn with_rfkill(&self, action: &'static str, error: BluezError) -> BluezError {
        let blocked_now = || self.state().adapters.iter().any(|a| a.power_state == PowerState::Blocked);
        let rfkill = matches!(error, BluezError::RfkillBlocked { .. })
            || (action == "power" && matches!(error, BluezError::CallFailed(_)) && blocked_now());
        if !rfkill {
            return error;
        }
        BluezError::RfkillBlocked { hard: self.options.rfkill_dir.as_deref().and_then(read_rfkill).map(|r| r.hard) }
    }

    fn flush(&mut self) {
        self.dirty = None;
        let state = self.state();
        if self.emitted.as_ref() == Some(&state) {
            return;
        }
        self.emitted = Some(state.clone());
        (self.sink)(BluetoothEvent::State(state));
    }

    fn cancel_pending(&mut self) {
        for (id, _) in self.pending.drain() {
            (self.sink)(BluetoothEvent::PairingDone { id });
        }
    }

    async fn set_owner(&mut self, owner: Option<String>) {
        if owner.is_some() && owner == self.bluez.owner {
            return;
        }
        self.bluez.set_owner(owner.clone());
        if let Ok(mut o) = self.agent_owner.lock() {
            *o = owner;
        }
        self.objects.clear();
        // Yeni BlueZ sürecinde ajan kayıtlı değildir.
        self.agent_status = AgentStatus::default();
        self.cancel_pending();
        self.discovery_until = None;
        if self.bluez.owner.is_some() {
            match get_managed_objects(&self.conn, BLUEZ, "/", MAX_OBJECTS).await {
                Ok((objects, reply)) => {
                    self.bluez.loaded(&reply);
                    for (path, ifaces) in objects {
                        self.objects.add(path.as_str(), ifaces);
                    }
                }
                Err(e) => tracing::warn!("bluetooth: GetManagedObjects: {e}"),
            }
            self.register_agent().await;
        }
        self.dirty = Some(Instant::now());
    }

    /// Ajanı kaydeder ve varsayılan ister; sonuç [`BluetoothState::agent`]'te.
    async fn register_agent(&mut self) {
        let Some(path) = self.agent_path.clone() else { return };
        let registered = self
            .conn
            .call_method(Some(BLUEZ), "/org/bluez", Some(AGENT_MANAGER_IFACE), "RegisterAgent", &(&path, "KeyboardDisplay"))
            .await;
        if let Err(e) = registered {
            tracing::warn!("bluetooth: eşleştirme ajanı kaydedilemedi: {e}");
            return;
        }
        self.agent_status.registered = true;
        let default = self
            .conn
            .call_method(Some(BLUEZ), "/org/bluez", Some(AGENT_MANAGER_IFACE), "RequestDefaultAgent", &(&path,))
            .await;
        match default {
            Ok(_) => self.agent_status.default = true,
            Err(e) => tracing::warn!("bluetooth: varsayılan ajan olunamadı: {e}"),
        }
        self.mark_dirty();
    }

    async fn input(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>, input: Input) {
        match input {
            Input::Ignore => {}
            Input::Owner => {
                let owner = get_name_owner(&self.conn, BLUEZ).await;
                self.set_owner(owner).await;
            }
            Input::Signal(msg) => {
                if self.bluez.accepts(&msg) && apply_object_signal(&mut self.objects, &msg) {
                    self.mark_dirty();
                }
            }
            Input::Agent(call) => {
                if self.pending.len() >= MAX_PENDING {
                    return; // Gönderen düşürüldü → ajan Rejected döner.
                }
                let name = self
                    .state()
                    .devices
                    .into_iter()
                    .find(|d| d.path == call.device)
                    .map(|d| d.name)
                    .unwrap_or_else(|| call.device.clone());
                if call.kind.needs_reply() {
                    self.pending.insert(call.id, call.reply);
                }
                (self.sink)(BluetoothEvent::Pairing(PairingRequest {
                    id: call.id,
                    device: call.device,
                    name,
                    kind: call.kind,
                }));
            }
            Input::AgentCancel => self.cancel_pending(),
            Input::DiscoveryFailed { request, generation, error } => {
                // Yalnız son isteğin hatası kendi tarama bayrağını indirir: eski (geç gelen) bir
                // başarısızlık sonra başlatılan taramayı etkilemez.
                if generation == self.discovery_gen && self.discovery_until.take().is_some() {
                    self.mark_dirty();
                }
                log_failure("discovery", &error);
                (self.sink)(BluetoothEvent::Failed { request, device: None, action: "discovery", error });
            }
            Input::Failed { request, device, action, error } => {
                let error = self.with_rfkill(action, error);
                log_failure(action, &error);
                (self.sink)(BluetoothEvent::Failed { request, device, action, error });
            }
            Input::Done { request, device, action } => {
                // Alınmış durum değişikliği (ör. `Paired=true`) sonuçtan önce yayınlansın.
                if self.dirty.is_some() {
                    self.flush();
                }
                (self.sink)(BluetoothEvent::Done { request, device, action });
            }
            Input::Paired(device) => {
                // Eşleşme tamam: güvenilir yap ve bağlan.
                self.spawn_call(ctx, tx, Some(device.clone()), "trust", {
                    let conn = self.conn.clone();
                    let device = device.clone();
                    async move { set_property(&conn, BLUEZ, &device, DEVICE_IFACE, "Trusted", Value::Bool(true)).await }
                });
                self.device_call(ctx, tx, &device, "Connect", "connect");
            }
        }
    }

    fn spawn_call(
        &self,
        ctx: &ServiceContext<Request>,
        tx: &Sender<Input>,
        device: Option<String>,
        action: &'static str,
        call: impl std::future::Future<Output = zbus::Result<()>> + 'static,
    ) {
        let tx = tx.clone();
        let request = self.current;
        ctx.ex
            .spawn(async move {
                let input = match call.await {
                    Ok(()) => match request {
                        Some(request) => Input::Done { request, device, action },
                        None => return,
                    },
                    Err(e) => Input::Failed { request, device, action, error: failure_error(&e) },
                };
                let _ = tx.send(input).await;
            })
            .detach();
    }

    fn known_device(&self, path: &str) -> bool {
        self.objects.has(path, DEVICE_IFACE)
    }

    fn known_adapter(&self, path: &str) -> bool {
        self.objects.has(path, ADAPTER_IFACE)
    }

    fn no_adapter(&self, action: &'static str) {
        (self.sink)(BluetoothEvent::Failed { request: self.current, device: None, action, error: BluezError::NoAdapter });
    }

    fn invalid(&self, device: Option<String>, action: &'static str) {
        (self.sink)(BluetoothEvent::Failed { request: self.current, device, action, error: BluezError::InvalidValue });
    }

    /// Tek özellik yazımı (`Properties.Set`).
    #[allow(clippy::too_many_arguments)]
    fn set_prop(
        &self,
        ctx: &ServiceContext<Request>,
        tx: &Sender<Input>,
        device: Option<String>,
        action: &'static str,
        path: String,
        iface: &'static str,
        name: &'static str,
        value: Value<'static>,
    ) {
        let conn = self.conn.clone();
        self.spawn_call(ctx, tx, device, action, async move {
            set_property(&conn, BLUEZ, &path, iface, name, value).await
        });
    }

    fn device_call(
        &self,
        ctx: &ServiceContext<Request>,
        tx: &Sender<Input>,
        device: &str,
        method: &'static str,
        action: &'static str,
    ) {
        let conn = self.conn.clone();
        let path = device.to_owned();
        let timeout = self.options.action_timeout;
        self.spawn_call(ctx, tx, Some(device.to_owned()), action, async move {
            with_timeout(timeout, conn.call_method(Some(BLUEZ), path.as_str(), Some(DEVICE_IFACE), method, &()))
                .await
                .map(drop)
        });
    }

    fn discovery(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>, start: bool) {
        let Some(adapter) = self.state().adapter().map(|a| a.path.clone()) else {
            if start {
                (self.sink)(BluetoothEvent::Failed {
                    request: self.current,
                    device: None,
                    action: "discovery",
                    error: BluezError::NoAdapter,
                });
            }
            return;
        };
        self.discovery_until = start.then(|| Instant::now() + self.options.discovery_timeout);
        self.mark_dirty();
        self.discovery_gen += 1;
        let generation = self.discovery_gen;
        let request = self.current;
        let conn = self.conn.clone();
        let tx = tx.clone();
        let call = async move {
            let call = |method: &'static str| {
                conn.call_method(Some(BLUEZ), adapter.as_str(), Some(ADAPTER_IFACE), method, &())
            };
            if !start {
                return call("StopDiscovery").await.map(drop);
            }
            // Filtre bu istemcinin oturumuna aittir (BlueZ istemci başına tutar): Transport
            // auto (LE + BR/EDR), DuplicateData false (RSSI değişikliği yine gelir).
            let filter: HashMap<&str, Value<'_>> =
                HashMap::from([("Transport", Value::from("auto")), ("DuplicateData", Value::Bool(false))]);
            conn.call_method(Some(BLUEZ), adapter.as_str(), Some(ADAPTER_IFACE), "SetDiscoveryFilter", &(filter,))
                .await?;
            call("StartDiscovery").await.map(drop)
        };
        ctx.ex
            .spawn(async move {
                let input = match call.await {
                    Ok(()) => match request {
                        Some(request) => Input::Done { request, device: None, action: "discovery" },
                        None => return,
                    },
                    Err(e) => Input::DiscoveryFailed { request, generation, error: failure_error(&e) },
                };
                let _ = tx.send(input).await;
            })
            .detach();
    }

    fn command(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>, request: Request) {
        self.current = Some(request.id);
        self.run_command(ctx, tx, request.command);
        self.current = None;
    }

    fn run_command(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>, command: Command) {
        if self.bluez.owner.is_none() {
            if let Command::Reply { id, .. } = command {
                self.pending.remove(&id);
                return;
            }
            (self.sink)(BluetoothEvent::Failed {
                request: self.current,
                device: None, action: "bluez", error: BluezError::BluezUnavailable
            });
            return;
        }
        let unknown = |this: &Self, d: &str| {
            (this.sink)(BluetoothEvent::Failed {
                request: this.current,
                device: Some(d.to_owned()),
                action: "device",
                error: BluezError::UnknownDevice,
            })
        };
        match command {
            Command::SetPowered(on) => {
                for adapter in self.state().adapters {
                    let conn = self.conn.clone();
                    self.spawn_call(ctx, tx, None, "power", async move {
                        set_property(&conn, BLUEZ, &adapter.path, ADAPTER_IFACE, "Powered", Value::Bool(on)).await
                    });
                }
                if !on {
                    self.discovery_until = None;
                }
            }
            Command::StartDiscovery => self.discovery(ctx, tx, true),
            Command::StopDiscovery => {
                if self.discovery_until.take().is_some() {
                    self.discovery(ctx, tx, false);
                }
            }
            Command::Connect(d) if self.known_device(&d) => self.device_call(ctx, tx, &d, "Connect", "connect"),
            Command::Disconnect(d) if self.known_device(&d) => {
                self.device_call(ctx, tx, &d, "Disconnect", "disconnect")
            }
            Command::Pair(d) if self.known_device(&d) => {
                let conn = self.conn.clone();
                let timeout = self.options.action_timeout;
                let tx2 = tx.clone();
                let device = d.clone();
                self.spawn_call(ctx, tx, Some(d), "pair", async move {
                    with_timeout(
                        timeout,
                        conn.call_method(Some(BLUEZ), device.as_str(), Some(DEVICE_IFACE), "Pair", &()),
                    )
                    .await?;
                    let _ = tx2.send(Input::Paired(device)).await;
                    Ok(())
                });
            }
            Command::Forget(d) if self.known_device(&d) => {
                let adapter =
                    self.state().devices.into_iter().find(|x| x.path == d).map(|x| x.adapter).unwrap_or_default();
                let conn = self.conn.clone();
                let device = d.clone();
                self.spawn_call(ctx, tx, Some(d), "forget", async move {
                    let path = ObjectPath::try_from(device.as_str()).map_err(zbus::Error::from)?;
                    conn.call_method(Some(BLUEZ), adapter.as_str(), Some(ADAPTER_IFACE), "RemoveDevice", &(path,))
                        .await
                        .map(drop)
                });
            }
            Command::SetTrusted(d, trusted) if self.known_device(&d) => {
                let conn = self.conn.clone();
                let device = d.clone();
                self.spawn_call(ctx, tx, Some(d), "trust", async move {
                    set_property(&conn, BLUEZ, &device, DEVICE_IFACE, "Trusted", Value::Bool(trusted)).await
                });
            }
            Command::SetBlocked(d, blocked) if self.known_device(&d) => {
                self.set_prop(ctx, tx, Some(d.clone()), "block", d, DEVICE_IFACE, "Blocked", Value::Bool(blocked));
            }
            Command::SetDeviceAlias(d, alias) if self.known_device(&d) => {
                if !valid_alias(&alias) {
                    self.invalid(Some(d), "alias");
                    return;
                }
                self.set_prop(ctx, tx, Some(d.clone()), "alias", d, DEVICE_IFACE, "Alias", Value::from(alias));
            }
            Command::CancelPairing(d) if self.known_device(&d) => {
                self.device_call(ctx, tx, &d, "CancelPairing", "cancel_pairing");
            }
            Command::Connect(d)
            | Command::Disconnect(d)
            | Command::Pair(d)
            | Command::Forget(d)
            | Command::SetTrusted(d, _)
            | Command::SetBlocked(d, _)
            | Command::SetDeviceAlias(d, _)
            | Command::CancelPairing(d) => unknown(self, &d),
            Command::SetAdapterAlias(a, alias) => {
                if !self.known_adapter(&a) {
                    return self.no_adapter("adapter_alias");
                }
                if !valid_alias(&alias) {
                    return self.invalid(None, "adapter_alias");
                }
                self.set_prop(ctx, tx, None, "adapter_alias", a, ADAPTER_IFACE, "Alias", Value::from(alias));
            }
            Command::SetPairable(a, on) => {
                if !self.known_adapter(&a) {
                    return self.no_adapter("pairable");
                }
                self.set_prop(ctx, tx, None, "pairable", a, ADAPTER_IFACE, "Pairable", Value::Bool(on));
            }
            Command::SetDiscoverable { adapter, on, timeout } => {
                if !self.known_adapter(&adapter) {
                    return self.no_adapter("discoverable");
                }
                let conn = self.conn.clone();
                self.spawn_call(ctx, tx, None, "discoverable", async move {
                    // Süre önce: açılır açılmaz doğru süreyle sayılsın.
                    if on {
                        set_property(&conn, BLUEZ, &adapter, ADAPTER_IFACE, "DiscoverableTimeout", Value::U32(timeout))
                            .await?;
                    }
                    set_property(&conn, BLUEZ, &adapter, ADAPTER_IFACE, "Discoverable", Value::Bool(on)).await
                });
            }
            Command::Reply { id, reply } => {
                if let Some(sender) = self.pending.remove(&id) {
                    let _ = sender.try_send(reply);
                    (self.sink)(BluetoothEvent::PairingDone { id });
                }
            }
        }
    }
}

async fn with_timeout<T>(d: Duration, f: impl std::future::Future<Output = zbus::Result<T>>) -> zbus::Result<T> {
    f.or(async {
        async_io::Timer::after(d).await;
        Err(zbus::Error::InputOutput(Arc::new(io::Error::from(io::ErrorKind::TimedOut))))
    })
    .await
}

/// Hata türü: zaman aşımı ve tanınan `org.bluez.Error.*` adları ayrı, diğerleri D-Bus
/// hatasının ham metniyle.
fn failure_error(e: &zbus::Error) -> BluezError {
    match e {
        zbus::Error::InputOutput(err) if err.kind() == io::ErrorKind::TimedOut => BluezError::Timeout,
        zbus::Error::MethodError(name, _, _) => BluezError::from_dbus_name(name.as_str())
            .unwrap_or_else(|| BluezError::CallFailed(clean_str(&e.to_string(), 200))),
        _ => BluezError::CallFailed(clean_str(&e.to_string(), 200)),
    }
}

/// Eylem hatası logu (review şartı 2): cihaz yolu (MAC içerir) ve `CallFailed` ayrıntısı
/// (cihaz adı/adresi içerebilir) yazılmaz; yalnız eylem ve redakte `Debug`.
fn log_failure(action: &'static str, error: &BluezError) {
    tracing::warn!(action, error = ?error, "bluetooth eylemi başarısız");
}

/// Bluetooth adı/takma adı geçerli mi: en fazla 248 bayt, kontrol karaktersiz. Boş ad
/// BlueZ'de varsayılana döner (adaptörde sistem adı, cihazda cihazın kendi adı).
pub fn valid_alias(alias: &str) -> bool {
    alias.len() <= 248 && !alias.chars().any(char::is_control)
}

// ---------------------------------------------------------------------------------------
// Ajan

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
    Canceled(String),
}

struct Agent {
    input: Sender<Input>,
    owner: Arc<Mutex<Option<String>>>,
    ids: Arc<AtomicU64>,
    timeout: Duration,
}

impl Agent {
    /// Çağrı BlueZ'in şu anki sahibinden mi?
    fn check(&self, header: &Header<'_>) -> Result<(), AgentError> {
        let owner = self.owner.lock().ok().and_then(|o| o.clone());
        match (header.sender(), owner) {
            (Some(s), Some(o)) if s.as_str() == o => Ok(()),
            _ => {
                tracing::warn!("bluetooth: BlueZ dışından ajan çağrısı reddedildi");
                Err(AgentError::Rejected("yetkisiz çağıran".into()))
            }
        }
    }

    /// İsteği UI'a iletir ve (gerekiyorsa) cevabı bekler.
    async fn ask(
        &self,
        header: &Header<'_>,
        device: &ObjectPath<'_>,
        kind: PairingKind,
    ) -> Result<PairingReply, AgentError> {
        self.check(header)?;
        if device.as_str().len() > 128 {
            return Err(AgentError::Rejected("geçersiz cihaz".into()));
        }
        let needs_reply = kind.needs_reply();
        let (reply_tx, reply_rx) = async_channel::bounded(1);
        let id = self.ids.fetch_add(1, Ordering::Relaxed);
        let call = AgentCall { id, device: device.to_string(), kind, reply: reply_tx };
        if self.input.try_send(Input::Agent(call)).is_err() {
            return Err(AgentError::Rejected("meşgul".into()));
        }
        if !needs_reply {
            return Ok(PairingReply::Accept);
        }
        let reply = async { reply_rx.recv().await.map_err(|_| AgentError::Canceled("iptal".into())) }
            .or(async {
                async_io::Timer::after(self.timeout).await;
                Err(AgentError::Canceled("zaman aşımı".into()))
            })
            .await;
        if reply.is_err() {
            // Zaman aşımında UI diyaloğu kapansın.
            let _ = self.input.try_send(Input::AgentCancel);
        }
        reply
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {
        tracing::info!("bluetooth: BlueZ ajanı bıraktı");
    }

    async fn request_pin_code(
        &self,
        #[zbus(header)] header: Header<'_>,
        device: ObjectPath<'_>,
    ) -> Result<String, AgentError> {
        match self.ask(&header, &device, PairingKind::PinCode).await? {
            PairingReply::Pin(pin) if valid_pin(&pin) => Ok(pin),
            _ => Err(AgentError::Rejected("reddedildi".into())),
        }
    }

    async fn display_pin_code(
        &self,
        #[zbus(header)] header: Header<'_>,
        device: ObjectPath<'_>,
        pincode: String,
    ) -> Result<(), AgentError> {
        let pin = clean_str(&pincode, 16);
        self.ask(&header, &device, PairingKind::DisplayPinCode { pin }).await.map(drop)
    }

    async fn request_passkey(
        &self,
        #[zbus(header)] header: Header<'_>,
        device: ObjectPath<'_>,
    ) -> Result<u32, AgentError> {
        match self.ask(&header, &device, PairingKind::Passkey).await? {
            PairingReply::Passkey(k) if valid_passkey(k) => Ok(k),
            _ => Err(AgentError::Rejected("reddedildi".into())),
        }
    }

    async fn display_passkey(
        &self,
        #[zbus(header)] header: Header<'_>,
        device: ObjectPath<'_>,
        passkey: u32,
        entered: u16,
    ) -> Result<(), AgentError> {
        self.ask(&header, &device, PairingKind::DisplayPasskey { passkey: passkey.min(999_999), entered })
            .await
            .map(drop)
    }

    async fn request_confirmation(
        &self,
        #[zbus(header)] header: Header<'_>,
        device: ObjectPath<'_>,
        passkey: u32,
    ) -> Result<(), AgentError> {
        accept(self.ask(&header, &device, PairingKind::Confirm { passkey: passkey.min(999_999) }).await?)
    }

    async fn request_authorization(
        &self,
        #[zbus(header)] header: Header<'_>,
        device: ObjectPath<'_>,
    ) -> Result<(), AgentError> {
        accept(self.ask(&header, &device, PairingKind::Authorize).await?)
    }

    async fn authorize_service(
        &self,
        #[zbus(header)] header: Header<'_>,
        device: ObjectPath<'_>,
        uuid: String,
    ) -> Result<(), AgentError> {
        let uuid = clean_str(&uuid, 64);
        accept(self.ask(&header, &device, PairingKind::AuthorizeService { uuid }).await?)
    }

    fn cancel(&self, #[zbus(header)] header: Header<'_>) -> Result<(), AgentError> {
        self.check(&header)?;
        let _ = self.input.try_send(Input::AgentCancel);
        Ok(())
    }
}

fn accept(reply: PairingReply) -> Result<(), AgentError> {
    match reply {
        PairingReply::Accept => Ok(()),
        _ => Err(AgentError::Rejected("reddedildi".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_debug_is_redacted() {
        let e = BluezError::CallFailed("Kulaklığım AA:BB:CC:DD:EE:FF".into());
        assert_eq!(format!("{e:?}"), "CallFailed(<redacted>)");
        let ev = BluetoothEvent::Failed { request: None, device: None, action: "pair", error: e };
        assert!(!format!("{ev:?}").contains("Kulaklığım"));
        assert_eq!(format!("{:?}", BluezError::Timeout), "Timeout");
        assert_eq!(format!("{:?}", BluezError::AuthenticationRejected), "AuthenticationRejected");
    }

    /// Olay alanlarını metne yazan en küçük abone (yeni crate yok).
    struct Capture(Arc<Mutex<String>>);

    impl tracing::Subscriber for Capture {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct V<'a>(&'a mut String);
            impl tracing::field::Visit for V<'_> {
                fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                    self.0.push_str(&format!("{}={value:?} ", field.name()));
                }
            }
            event.record(&mut V(&mut self.0.lock().unwrap()));
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    #[test]
    fn failure_log_is_redacted() {
        let out = Arc::new(Mutex::new(String::new()));
        tracing::subscriber::with_default(Capture(Arc::clone(&out)), || {
            log_failure("pair", &BluezError::CallFailed("Kulaklığım AA:BB:CC:DD:EE:FF".into()));
            log_failure("connect", &BluezError::AuthenticationRejected);
        });
        let out = out.lock().unwrap().clone();
        assert!(out.contains("CallFailed(<redacted>)"), "{out}");
        assert!(out.contains("AuthenticationRejected"), "{out}");
        assert!(out.contains("action=\"pair\""), "{out}");
        assert!(!out.contains("Kulaklığım") && !out.contains("AA:BB"), "{out}");
    }

    #[test]
    fn bluez_error_names() {
        assert_eq!(
            BluezError::from_dbus_name("org.bluez.Error.AuthenticationRejected"),
            Some(BluezError::AuthenticationRejected)
        );
        assert_eq!(BluezError::from_dbus_name("org.bluez.Error.InProgress"), Some(BluezError::InProgress));
        assert_eq!(BluezError::from_dbus_name("org.bluez.Error.Blocked"), Some(BluezError::RfkillBlocked { hard: None }));
        assert_eq!(BluezError::from_dbus_name("org.bluez.Error.Failed"), None);
        assert_eq!(BluezError::from_dbus_name("org.freedesktop.DBus.Error.AuthenticationFailed"), None);
        let timeout = zbus::Error::InputOutput(Arc::new(io::Error::from(io::ErrorKind::TimedOut)));
        assert_eq!(failure_error(&timeout), BluezError::Timeout);
    }

    #[test]
    fn aliases() {
        assert!(valid_alias("Masaüstü"));
        assert!(valid_alias(""));
        assert!(!valid_alias("a\nb"));
        assert!(!valid_alias(&"x".repeat(249)));
    }
}
