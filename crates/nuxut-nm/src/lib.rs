//! Ağ: NetworkManager (`org.freedesktop.NetworkManager`, sistem bus'ı) — bağlantı durumu,
//! birincil bağlantı türü, Wi-Fi aç/kapa, istek üzerine tarama, erişim noktaları, bağlan/kopar,
//! kayıtlı bağlantılar, VPN ve şifre için gizli bilgi ajanı.
//!
//! **Durum.** NM'nin ObjectManager'ı (`/org/freedesktop`) bir kez `GetManagedObjects` ile
//! okunur, `InterfacesAdded`/`InterfacesRemoved`/`PropertiesChanged` ile güncellenir. Kayıtlı
//! bağlantıların ayarları (`GetSettings`, gizli bilgiler hariç) bağlantı görününce ve
//! `Updated` sinyalinde okunur. Erişim noktası (en fazla [`MAX_ACCESS_POINTS`]) sinyal gücü
//! sık değişir: anlık görüntü en fazla [`NetworkOptions::throttle`] aralıkla yayınlanır.
//!
//! **Tarama** yalnızca [`NetworkService::scan`] ile (panel açıkken) istenir; servis kendisi
//! hiç tarama başlatmaz.
//!
//! **Gizli bilgi ajanı.** `org.freedesktop.NetworkManager.SecretAgent` nesnesi
//! `/org/freedesktop/NetworkManager/SecretAgent` yolunda yayınlanır ve `AgentManager.Register`
//! ile kaydedilir (NM yeniden başlarsa tekrar; bir NM sahibi için en fazla bir kez — aynı
//! kimlikle ikinci kayıt NM'de reddedilir). NM kuralı: bir ajan kimliği **kullanıcı (UID) başına**
//! tektir, bağlantı başına değil. Kimlik [`NetworkOptions::agent_id`] (`None`: ajan yok —
//! varsayılan, Ayarlar böyle açar; kabuk [`NetworkOptions::with_agent`] ile açıkça ister;
//! ikincil/test örneğinin kimlik
//! seçimi kabuktadır). Kimlik başka bir süreçte kayıtlıysa
//! ("An agent with this ID is already registered for this user") bir kez anlaşılır uyarı
//! yazılır, Wi-Fi paneli kullanılınca (tarama/bağlanma) sessizce yeniden denenir.
//!
//! NM bir Wi-Fi şifresi isteyince UI'a
//! [`NetworkEvent::SecretRequest`] gider, cevap [`NetworkService::provide_secret`] ile beklenir
//! ([`NetworkOptions::secret_timeout`]). Ajan çağrıları yalnızca NM'nin şu anki benzersiz
//! sahibinden kabul edilir. Şifreler [`Secret`] içinde tutulur: `Debug` çıktısında görünmez,
//! loglanmaz, düşürülünce bellekten silinir (zeroize); servis şifre saklamaz (NM saklar).
//! Kısa süreli tek istisna: [`NetworkService::connect_access_point`]'e verilen şifre, kayıtlı
//! bağlantı etkinleştirilirken NM sorarsa kullanılmak üzere en fazla [`PREFILL_TTL`] tutulur.
//! Girişte kopya yok: [`Secret::from_bytes`] çağıranın sıfırlanan tamponunu sahiplenir; NM'ye
//! giden ayarlarda değer şifreden ödünç alınır (bkz. `secret` modülü; bilinen sınır: zbus'ın
//! mesaj tamponu sıfırlanmaz).
//!
//! **Yazma API'leri (aşama 2).** [`NetworkService::connect_hidden`],
//! [`NetworkService::set_autoconnect`] / [`NetworkService::set_metered`] (`GetSettings` + tek
//! alan + `Update`; gizli bilgilere dokunulmaz), [`NetworkService::import_wireguard`] (kendi
//! ayrıştırıcımız, [`wireguard`]), [`NetworkService::forget`] (VPN dahil),
//! [`NetworkService::device_details`]. Başarı [`NetworkEvent::Done`] /
//! [`NetworkEvent::DeviceDetails`], hata [`NetworkEvent::Failed`]; NM'nin sonradan vazgeçtiği
//! etkinleştirme [`NetworkEvent::ActivationFailed`] (ör. ajansız istemcide `NoSecrets` →
//! "şifreyi güncelle").
//!
//! **Bus.** Varsayılan sistem bus'ı; sahte NM (`examples/fake_nm`, `dbus-run-session` içinde)
//! ile canlı testte `bus_from_env()` (`NUXUT_NM_BUS=session`; yalnız `test-support` özelliği,
//! üründe derlenmez) — crate ortamı kendisi okumaz.
//!
//! # Ayarlar kullanım kalıbı
//!
//! Ayarlar ajan **olmaz** (varsayılan [`NetworkOptions`] ajansızdır): şifre istemi kabuğundur. Olaylar
//! servis thread'inden `sink` ile gelir; ayrı bir `notify` kancası gerekmez — sink olayı
//! paylaşılan kuyruğa koyar ve UI döngüsünü uyandırır (calloop `Ping`, kanal vb.):
//!
//! ```no_run
//! use std::sync::{Arc, Mutex};
//! use nuxut_nm::{NetworkEvent, NetworkOptions, NetworkService, Secret, Security};
//!
//! let inbox: Arc<Mutex<Vec<NetworkEvent>>> = Arc::default();
//! let wake = || { /* ör. calloop::ping::Ping::ping(&ping) */ };
//! // Canlı testte (dev-dependency, `test-support`): `bus: nuxut_nm::bus_from_env()`.
//! let options = NetworkOptions::default();
//! let queue = Arc::clone(&inbox);
//! let net = NetworkService::spawn(options, move |event| {
//!     queue.lock().unwrap().push(event);
//!     wake();
//! })?;
//! // Parola alanından tek kopya: `SecretBuffer::to_zeroizing()` → `Secret::from_bytes`.
//! let typed = zeroize::Zeroizing::new(b"gizli-sifre".to_vec());
//! net.connect_hidden(b"Ev", Security::WpaPsk, Secret::from_bytes(typed));
//! // UI thread'inde, uyanınca:
//! for event in inbox.lock().unwrap().drain(..) {
//!     match event {
//!         NetworkEvent::State(_state) => { /* sayfayı yenile */ }
//!         NetworkEvent::Done { request, action, object } => { /* request: eylemin kimliği */ }
//!         NetworkEvent::Failed { request, action, error } => { /* türden çevrilmiş metin */ }
//!         NetworkEvent::ActivationFailed { connection, reason } => { /* NoSecrets → şifre alanı */ }
//!         _ => {}
//!     }
//! }
//! # Ok::<(), std::io::Error>(())
//! ```

mod model;
mod secret;
pub mod wireguard;

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_channel::Sender;
use futures_lite::future::FutureExt;
use zbus::message::Header;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Message};

pub use nuxut_dbus::{BusKind, RequestId, Unavailable};
pub use secret::Secret;
use secret::SecretsReply;
pub use model::{
    AccessPoint, ActiveConnection, ActiveState, ConnectionKind, Connectivity, DeviceDetails, DeviceKind,
    MAX_ACCESS_POINTS, MAX_CONNECTIONS, MAX_SSID, Metered, NetDevice, NetworkState, NmState, SavedConnection, Security,
    WifiDevice, security_from_flags, ssid_display,
};
pub use wireguard::{IpNet, WireGuardConfig, WireGuardError, WireGuardErrorKind, WireGuardPeer};

use nuxut_dbus::{
    Backoff, NameOwner, ObjectStore, ServiceContext, ServiceThread, apply_object_signal, clean_str, close,
    connect, forward, get_managed_objects, get_name_owner, owner_changes, parse_owner_change, prop_paths, set_property,
    signals_from, tick_connection,
};
use model::{
    AP_IFACE, NM_IFACE, NM_PATH, SETTINGS_CONN_IFACE, Settings, SettingsSummary, WIRELESS_IFACE, key_mgmt,
    setting_bytes,
};

const NM: &str = "org.freedesktop.NetworkManager";
const OBJECT_ROOT: &str = "/org/freedesktop";
const AGENT_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager/AgentManager";
const AGENT_MANAGER_IFACE: &str = "org.freedesktop.NetworkManager.AgentManager";
/// NM ajanları bu sabit yolda çağırır.
pub const SECRET_AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
/// Kabuğun (ana örnek) ajan kimliği; [`NetworkOptions::default`] bunu kullanır (NM'de kimlik
/// kullanıcı başına tektir).
pub const AGENT_ID: &str = "org.nuxut.shell";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS_IFACE: &str = "org.freedesktop.NetworkManager.Settings";
/// `NM_DEVICE_TYPE_ETHERNET`.
const DEVICE_ETHERNET: u32 = 1;
/// `NM_DEVICE_STATE_FAILED`.
const DEVICE_FAILED: u32 = 120;
/// `NM_ACTIVE_CONNECTION_STATE_DEACTIVATED`.
const ACTIVE_DEACTIVATED: u32 = 4;
const INPUT_CAPACITY: usize = 1024;
const MAX_OBJECTS: usize = 2048;
/// Aynı anda bekleyen en fazla gizli bilgi isteği.
const MAX_PENDING: usize = 4;
/// Önceden verilmiş şifrenin ömrü.
pub const PREFILL_TTL: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------------------
// Gizli bilgi

/// İstenen gizli bilginin türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SecretKind {
    /// WPA/WPA2/WPA3 kişisel şifre.
    WifiPassword,
    /// WEP anahtarı.
    WepKey,
}

/// Şifre bu tür için geçerli mi? WPA-PSK: 8–63 yazdırılabilir ASCII ya da 64 onaltılık;
/// SAE: 1–128 bayt; WEP: 1–64 bayt.
pub fn valid_secret(security: Security, s: &str) -> bool {
    match security {
        Security::WpaPsk => {
            ((8..=63).contains(&s.len()) && s.bytes().all(|b| (0x20..0x7f).contains(&b)))
                || (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        }
        Security::Sae => (1..=128).contains(&s.len()) && !s.chars().any(char::is_control),
        Security::Wep => (1..=64).contains(&s.len()) && !s.chars().any(char::is_control),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRequest {
    pub id: u64,
    /// Bağlantının adı (`connection.id`).
    pub connection: String,
    /// Wi-Fi ağının adı (gösterim).
    pub ssid: Option<String>,
    pub kind: SecretKind,
    /// NM yeni şifre istiyor (önceki yanlıştı).
    pub retry: bool,
}

// ---------------------------------------------------------------------------------------
// API

/// Eylem hatası. Crate UI'sızdır: kullanıcıya gösterilecek metni çağıran üretir (kabukta
/// `shell-network-error-*` anahtarları). `Display` yalnız log/yedek metin içindir.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NmError {
    /// D-Bus çağrısı başarısız; NM'nin hata metni (temizlenmiş, en fazla 200 bayt). Bağlantı
    /// adı/SSID içerebilir: loglanmaz.
    CallFailed(String),
    /// NetworkManager çalışmıyor (bus'ta sahibi yok).
    NmUnavailable,
    /// Bilinmeyen kayıtlı bağlantı.
    ConnectionUnknown,
    /// Bağlantı etkin değil.
    ConnectionInactive,
    /// Bilinmeyen erişim noktası.
    ApUnknown,
    /// Kurumsal (802.1X) ağ desteklenmiyor.
    Enterprise,
    /// Şifre bu güvenlik türü için geçersiz.
    InvalidPassword,
    /// Wi-Fi cihazı yok.
    NoWifiDevice,
    /// SSID 1–32 bayt olmalı ([`NetworkService::connect_hidden`]).
    InvalidSsid,
    /// Bilinmeyen ağ cihazı ([`NetworkService::device_details`]).
    DeviceUnknown,
    /// İzin reddedildi (polkit; NM `…PermissionDenied`).
    Denied,
    /// Bu bağlantı türünde desteklenmiyor (ör. WEP / kurumsal ağda şifre güncelleme).
    Unsupported,
    /// Şifre saklanmayacak biçimde ayarlı (`psk-flags` NOT_SAVED: kullanıcının seçimi);
    /// değiştirilmedi.
    SecretNotSaved,
}

impl NmError {
    /// D-Bus hatasından tür: `…PermissionDenied` adları (`org.freedesktop.NetworkManager`,
    /// `.Settings`, `.Settings.Connection`, `.AgentManager`) → [`NmError::Denied`]; diğerleri
    /// temizlenmiş metinle [`NmError::CallFailed`].
    pub fn from_dbus(e: &zbus::Error) -> Self {
        match e {
            zbus::Error::MethodError(name, _, _)
                if name.as_str().starts_with("org.freedesktop.NetworkManager")
                    && name.as_str().ends_with(".PermissionDenied") =>
            {
                Self::Denied
            }
            _ => Self::CallFailed(clean_str(&e.to_string(), 200)),
        }
    }
}

impl From<zbus::Error> for NmError {
    fn from(e: zbus::Error) -> Self {
        Self::from_dbus(&e)
    }
}

impl std::fmt::Display for NmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CallFailed(detail) => write!(f, "çağrı başarısız: {detail}"),
            Self::NmUnavailable => f.write_str("NetworkManager çalışmıyor"),
            Self::ConnectionUnknown => f.write_str("bilinmeyen bağlantı"),
            Self::ConnectionInactive => f.write_str("bağlantı etkin değil"),
            Self::ApUnknown => f.write_str("bilinmeyen erişim noktası"),
            Self::Enterprise => f.write_str("kurumsal ağ desteklenmiyor"),
            Self::InvalidPassword => f.write_str("geçersiz şifre"),
            Self::NoWifiDevice => f.write_str("Wi-Fi cihazı yok"),
            Self::InvalidSsid => f.write_str("geçersiz SSID"),
            Self::DeviceUnknown => f.write_str("bilinmeyen cihaz"),
            Self::Denied => f.write_str("izin reddedildi"),
            Self::Unsupported => f.write_str("desteklenmiyor"),
            Self::SecretNotSaved => f.write_str("şifre saklanmıyor"),
        }
    }
}

/// Bağlantının etkinleşememe nedeni (NM'nin cihaz/etkin bağlantı durum nedeni).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActivationFailure {
    /// Gizli bilgi yok ya da verilmedi (ajan yok/iptal; Wi-Fi şifresi yanlış olabilir).
    /// Ayarlar "şifreyi güncelle" alanını açar.
    NoSecrets,
    /// Kimlik doğrulama başarısız (Wi-Fi supplicant ya da VPN girişi).
    AuthFailed,
    /// Diğer neden (NM kodu: cihaz için `NMDeviceStateReason`, VPN için
    /// `NMActiveConnectionStateReason`).
    Other(u32),
}

/// Redakte: `CallFailed` metni SSID/bağlantı adı içerebilir (log politikası).
impl std::fmt::Debug for NmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CallFailed(_) => f.write_str("CallFailed(<redacted>)"),
            Self::NmUnavailable => f.write_str("NmUnavailable"),
            Self::ConnectionUnknown => f.write_str("ConnectionUnknown"),
            Self::ConnectionInactive => f.write_str("ConnectionInactive"),
            Self::ApUnknown => f.write_str("ApUnknown"),
            Self::Enterprise => f.write_str("Enterprise"),
            Self::InvalidPassword => f.write_str("InvalidPassword"),
            Self::NoWifiDevice => f.write_str("NoWifiDevice"),
            Self::InvalidSsid => f.write_str("InvalidSsid"),
            Self::DeviceUnknown => f.write_str("DeviceUnknown"),
            Self::Denied => f.write_str("Denied"),
            Self::Unsupported => f.write_str("Unsupported"),
            Self::SecretNotSaved => f.write_str("SecretNotSaved"),
        }
    }
}

/// [`NetworkService::import_wireguard`] sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireGuardImport {
    /// İsteğin kimliği; `None`: kuyruğa girmedi.
    pub request: Option<RequestId>,
    /// İçe aktarılmayan wg-quick anahtarları (yalnız **adları**; değer asla).
    pub ignored: Vec<&'static str>,
}

#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub enum NetworkEvent {
    /// Durum (tam anlık görüntü).
    State(NetworkState),
    /// Şifre isteniyor; [`NetworkService::provide_secret`] ile cevaplanmalı.
    SecretRequest(SecretRequest),
    /// İstek bitti (cevaplandı, NM iptal etti ya da zaman aşımı): UI diyaloğu kapatır.
    SecretDone { id: u64 },
    /// Eylem başarısız. `request`: isteğin kimliği (eylem metodunun döndürdüğü).
    Failed { request: Option<RequestId>, action: &'static str, error: NmError },
    /// Eylem başarılı (NM çağrıyı kabul etti; tarama hariç her eylem). O ana dek alınan durum
    /// değişikliği bu olaydan **önce** [`NetworkEvent::State`] olarak yayınlanır. `object`:
    /// oluşan/değişen kayıtlı bağlantının yolu (biliniyorsa). Etkinleştirmenin sonradan
    /// başarısızlığı ayrıca [`NetworkEvent::ActivationFailed`].
    Done { request: RequestId, action: &'static str, object: Option<String> },
    /// [`NetworkService::device_details`] cevabı.
    DeviceDetails(DeviceDetails),
    /// Bir bağlantı etkinleşemedi (D-Bus çağrısı başarılı olsa da NM sonradan vazgeçti).
    /// `connection`: kayıtlı bağlantının yolu (biliniyorsa).
    ActivationFailed { connection: Option<String>, reason: ActivationFailure },
}

/// Redakte: SSID/bağlantı adı yazılmaz (log politikası).
impl std::fmt::Debug for NetworkEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(s) => f
                .debug_struct("State")
                .field("available", &s.available)
                .field("unavailable", &s.unavailable)
                .field("state", &s.state)
                .field("wifi_enabled", &s.wifi_enabled)
                .field("access_points", &s.access_points.len())
                .field("saved", &s.saved.len())
                .field("active", &s.active.len())
                .finish(),
            Self::SecretRequest(r) => f.debug_struct("SecretRequest").field("id", &r.id).field("kind", &r.kind).finish(),
            Self::SecretDone { id } => f.debug_struct("SecretDone").field("id", id).finish(),
            Self::Failed { request, action, .. } => {
                f.debug_struct("Failed").field("request", request).field("action", action).finish_non_exhaustive()
            }
            Self::Done { request, action, .. } => {
                f.debug_struct("Done").field("request", request).field("action", action).finish_non_exhaustive()
            }
            Self::DeviceDetails(d) => f.debug_struct("DeviceDetails").field("kind", &d.kind).finish_non_exhaustive(),
            Self::ActivationFailed { reason, .. } => {
                f.debug_struct("ActivationFailed").field("reason", reason).finish_non_exhaustive()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct NetworkOptions {
    /// Hangi bus (varsayılan sistem). Sahte NM ile canlı testte `bus_from_env()` ile seçilir
    /// (`test-support` özelliği).
    pub bus: BusKind,
    /// D-Bus adresi (testler); verilirse [`NetworkOptions::bus`] yerine bu adrese bağlanılır.
    pub bus_address: Option<String>,
    pub call_timeout: Duration,
    pub secret_timeout: Duration,
    pub throttle: Duration,
    pub max_backoff: Duration,
    /// Gizli bilgi ajanı kimliği; `None` (**varsayılan**): ajan yayınlanmaz/kaydedilmez (şifre
    /// istemi yok, [`NetworkEvent::SecretRequest`] hiç gelmez). NM'de kimlik kullanıcı başına
    /// tektir ve ajan **yalnız kabuktur**: kabuk bunu açıkça doldurur ([`AGENT_ID`]); Ayarlar
    /// varsayılanı kullanır — unutsa bile ajan olamaz.
    pub agent_id: Option<String>,
}

impl Default for NetworkOptions {
    /// Sistem bus'ı, **ajansız** (güvenli varsayılan; 2026-10-13).
    fn default() -> Self {
        Self {
            bus: BusKind::System,
            agent_id: None,
            bus_address: None,
            call_timeout: Duration::from_secs(5),
            secret_timeout: Duration::from_secs(120),
            throttle: Duration::from_millis(150),
            max_backoff: Duration::from_secs(30),
        }
    }
}

impl NetworkOptions {
    /// Ajanlı seçenekler (yalnız kabuk): `id` NM ajan kimliği.
    pub fn with_agent(id: impl Into<String>) -> Self {
        Self { agent_id: Some(id.into()), ..Self::default() }
    }
}

/// Sahte NM ile canlı test anahtarı (`bus_from_env`); yalnız `test-support`.
#[cfg(feature = "test-support")]
pub const BUS_ENV: &str = "NUXUT_NM_BUS";

/// `NUXUT_NM_BUS=session` ise oturum bus'ı, değilse sistem bus'ı. Yalnız `test-support`
/// özelliğiyle derlenir (üründe yok); çağıran açıkça kullanır.
#[cfg(feature = "test-support")]
pub fn bus_from_env() -> BusKind {
    BusKind::from_env(BUS_ENV)
}

#[derive(Debug)]
enum Command {
    SetWifi(bool),
    Scan,
    ConnectAp { ap: String, password: Option<Secret> },
    ConnectHidden { ssid: Vec<u8>, security: Security, password: Option<Secret> },
    Activate(String),
    Deactivate(String),
    Forget(String),
    Patch { connection: String, patch: Patch },
    ImportWireGuard { name: String, config: Box<WireGuardConfig> },
    DeviceDetails(String),
    SetPassword { connection: String, password: Secret },
    Secret { id: u64, secret: Option<Secret> },
}

/// Kayıtlı bağlantıya `Update` yaması (`connection` grubunda tek alan).
#[derive(Debug, Clone, Copy)]
enum Patch {
    Autoconnect(bool),
    Metered(Metered),
}

impl Patch {
    fn action(self) -> &'static str {
        match self {
            Self::Autoconnect(_) => "autoconnect",
            Self::Metered(_) => "metered",
        }
    }

    /// `GetSettings` çıktısına uygular: yalnız `connection.<alan>` değişir.
    fn apply(self, settings: &mut Settings) -> zbus::Result<()> {
        let (key, value) = match self {
            Self::Autoconnect(on) => ("autoconnect", Value::Bool(on)),
            Self::Metered(m) => ("metered", Value::I32(m.to_nm())),
        };
        let group = settings.get_mut("connection").ok_or_else(|| zbus::Error::Failure("connection yok".into()))?;
        group.insert(key.to_owned(), OwnedValue::try_from(value).map_err(zbus::Error::from)?);
        Ok(())
    }
}

/// Kuyruktaki istek: kimlik + komut.
#[derive(Debug)]
struct Request {
    id: RequestId,
    command: Command,
}

/// Çalışan ağ servisi. Düşürülünce temiz kapanır (ajan bırakılır).
///
/// Eylemler kuyruğa alınır ve istek kimliği döner (`None`: kuyruk dolu ya da servis durmuş);
/// sonuç aynı kimlikle [`NetworkEvent::Done`] ya da [`NetworkEvent::Failed`] olarak gelir
/// (`action` eylemin adı). Tarama ve şifre cevabı sonuç olayı vermez.
#[derive(Debug)]
pub struct NetworkService {
    thread: ServiceThread<Request>,
    next: AtomicU64,
}

impl NetworkService {
    pub fn spawn<F>(options: NetworkOptions, sink: F) -> io::Result<Self>
    where
        F: Fn(NetworkEvent) + Send + 'static,
    {
        let thread = ServiceThread::spawn("nuxut-network", 64, move |ctx| run(options, sink, ctx))?;
        Ok(Self { thread, next: AtomicU64::new(1) })
    }

    fn send(&self, command: Command) -> Option<RequestId> {
        let id = RequestId(self.next.fetch_add(1, Ordering::Relaxed));
        self.thread.send(Request { id, command }).then_some(id)
    }

    /// Wi-Fi aç/kapa (`action` `wifi`).
    pub fn set_wifi_enabled(&self, on: bool) -> Option<RequestId> {
        self.send(Command::SetWifi(on))
    }

    /// Wi-Fi taraması ister (yalnızca panel/sayfa açıkken). Sonuç olayı yok (hata hariç).
    pub fn scan(&self) -> Option<RequestId> {
        self.send(Command::Scan)
    }

    /// Erişim noktasına bağlanır ([`AccessPoint::path`]; `action` `connect`). Kayıtlı bağlantı
    /// varsa o etkinleştirilir; yoksa yeni bağlantı oluşturulur (`Done.object` kayıtlı bağlantı).
    /// Şifre gerekip verilmemişse NM ajandan ister (kabukta [`NetworkEvent::SecretRequest`]).
    pub fn connect_access_point(&self, ap: &str, password: Option<Secret>) -> Option<RequestId> {
        self.send(Command::ConnectAp { ap: ap.to_owned(), password })
    }

    /// Gizli (yayın yapmayan) ağa bağlanır: yeni bağlantı (`802-11-wireless.hidden=true`),
    /// `AddAndActivateConnection`, ilk Wi-Fi cihazında. `ssid` 1–32 bayt; `Enterprise`
    /// desteklenmez; şifre [`valid_secret`] ile denetlenir. Başarıda
    /// `Done { action: "connect_hidden", object: Some(<kayıtlı bağlantı>) }`.
    pub fn connect_hidden(&self, ssid: &[u8], security: Security, password: Option<Secret>) -> Option<RequestId> {
        self.send(Command::ConnectHidden { ssid: ssid.to_vec(), security, password })
    }

    /// Kayıtlı bağlantıyı (Wi-Fi, kablolu, VPN) etkinleştirir ([`SavedConnection::path`];
    /// `action` `activate`).
    pub fn activate(&self, connection: &str) -> Option<RequestId> {
        self.send(Command::Activate(connection.to_owned()))
    }

    /// Etkin bağlantıyı kapatır ([`ActiveConnection::path`] ya da
    /// [`SavedConnection::active`]; `action` `deactivate`).
    pub fn deactivate(&self, active: &str) -> Option<RequestId> {
        self.send(Command::Deactivate(active.to_owned()))
    }

    /// Kayıtlı bağlantıyı (Wi-Fi, kablolu, VPN, WireGuard) siler (`Settings.Connection.Delete`;
    /// `action` `forget`).
    pub fn forget(&self, connection: &str) -> Option<RequestId> {
        self.send(Command::Forget(connection.to_owned()))
    }

    /// `connection.autoconnect` yaması: `GetSettings` + alan + `Update`. Gizli bilgiler
    /// okunmaz/gönderilmez; NM gizli bilgisiz `Update`'te kayıtlı gizli bilgileri korur.
    pub fn set_autoconnect(&self, connection: &str, on: bool) -> Option<RequestId> {
        self.send(Command::Patch { connection: connection.to_owned(), patch: Patch::Autoconnect(on) })
    }

    /// `connection.metered` yaması ([`NetworkService::set_autoconnect`] gibi).
    pub fn set_metered(&self, connection: &str, metered: Metered) -> Option<RequestId> {
        self.send(Command::Patch { connection: connection.to_owned(), patch: Patch::Metered(metered) })
    }

    /// Kayıtlı Wi-Fi bağlantısının şifresini değiştirir (`action` `wifi_password`):
    /// `GetSettings` + `802-11-wireless-security` şifre anahtarı (`psk`; WEP'te `wep-key0`) +
    /// `Update`; diğer alanlar (otomatik bağlanma, ölçülü…) aynen. Şifre `key-mgmt`'e göre
    /// [`valid_secret`] ile denetlenir (geçersiz → `InvalidPassword`); değer şifreden ödünç
    /// serileşir (kopya yok). Kurumsal/açık ağda `InvalidPassword`.
    pub fn set_wifi_password(&self, connection: &str, password: Secret) -> Option<RequestId> {
        self.send(Command::SetPassword { connection: connection.to_owned(), password })
    }

    /// WireGuard `.conf` içe aktarır: metin burada (çağıranın thread'inde) ayrıştırılır;
    /// hatalı dosya `Err` (satır + tür, değer içermez). Geçerliyse `Settings.AddConnection`
    /// (type `wireguard`, `name` bağlantı ve arayüz adı, otomatik bağlanma kapalı) kuyruğa
    /// alınır; sonuç [`WireGuardImport`] (istek kimliği + içe aktarılmayan wg-quick anahtarlarının
    /// **adları**); başarıda `Done { action: "import_wireguard", object }`. `conf` ödünç:
    /// ayrıştırıcı anahtarları kendi [`Secret`]'lerine kopyalar.
    pub fn import_wireguard(&self, name: &str, conf: &Secret) -> Result<WireGuardImport, WireGuardError> {
        if !wireguard::valid_name(name) {
            return Err(WireGuardError { line: 0, kind: WireGuardErrorKind::InvalidName });
        }
        let config = wireguard::parse(conf.expose())?;
        let ignored = config.ignored.clone();
        let request = self.send(Command::ImportWireGuard { name: name.to_owned(), config: Box::new(config) });
        Ok(WireGuardImport { request, ignored })
    }

    /// Cihaz ayrıntısı (IP adresleri, ağ geçidi, DNS, MAC, hız; salt okunur):
    /// [`NetworkEvent::DeviceDetails`] olarak gelir ([`NetDevice::path`]).
    pub fn device_details(&self, device: &str) -> Option<RequestId> {
        self.send(Command::DeviceDetails(device.to_owned()))
    }

    /// Şifre isteğine cevap (`None`: iptal). Sonuç olayı yok (`SecretDone` gelir).
    pub fn provide_secret(&self, id: u64, secret: Option<Secret>) -> Option<RequestId> {
        self.send(Command::Secret { id, secret })
    }
}

// ---------------------------------------------------------------------------------------
// Döngü

#[derive(Debug)]
enum Input {
    Owner,
    Ignore,
    Signal(Message),
    Settings { path: String, result: Option<Settings> },
    Secret(SecretCall),
    CancelSecret { connection_path: String },
    Failed { request: Option<RequestId>, action: &'static str, error: NmError },
    Done { request: RequestId, action: &'static str, object: Option<String> },
    Details(Box<DeviceDetails>),
    /// `AgentManager.Register` sonucu (`generation`: istendiği andaki NM sahibi kuşağı).
    AgentRegistered { generation: u64, result: Result<(), RegisterError> },
}

/// Ajan kaydı hatası.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RegisterError {
    /// Bu kimlik bu kullanıcı için başka bir bağlantıda (süreçte) zaten kayıtlı.
    Taken,
    /// Diğer hata (temizlenmiş, kısaltılmış metin).
    Other(String),
}

impl RegisterError {
    /// NM'nin cevabını sınıflar. "Zaten kayıtlı" NM'de `PermissionDenied` + sabit metindir
    /// (`nm-agent-manager.c`: "An agent with this ID is already registered for this user.").
    fn classify(e: &zbus::Error) -> Self {
        if let zbus::Error::MethodError(name, message, _) = e
            && name.as_str().ends_with(".PermissionDenied")
            && message.as_deref().is_some_and(|m| m.contains("already registered"))
        {
            return Self::Taken;
        }
        Self::Other(clean_str(&e.to_string(), 200))
    }
}

/// Bu bağlantıdaki, şu anki NM sahibi için ajan kaydı durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentReg {
    /// Kayıt yok (NM yok, kimlik yok ya da henüz istenmedi).
    None,
    Pending,
    Registered,
    /// NM reddetti; tarama/bağlanmada yeniden denenir.
    Rejected,
}

struct SecretCall {
    id: u64,
    connection_path: String,
    connection_id: String,
    ssid: Option<Vec<u8>>,
    kind: SecretKind,
    retry: bool,
    reply: Sender<Option<Secret>>,
}

impl std::fmt::Debug for SecretCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretCall").field("id", &self.id).field("connection_path", &self.connection_path).finish()
    }
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

async fn run<F: Fn(NetworkEvent)>(options: NetworkOptions, sink: F, ctx: ServiceContext<Request>) {
    let mut backoff = Backoff::new(options.max_backoff);
    let ids = Arc::new(AtomicU64::new(1));
    let mut unreachable_sent = false;
    loop {
        match connect(options.bus, options.bus_address.as_deref(), options.call_timeout).await {
            Ok(conn) => {
                backoff.reset();
                let ticker = tick_connection(&ctx.ex, &conn);
                let mut rt = Runtime {
                    sink: &sink,
                    options: &options,
                    conn: conn.clone(),
                    nm: NameOwner::new(),
                    agent_owner: Arc::new(Mutex::new(None)),
                    agent: AgentReg::None,
                    agent_generation: 0,
                    agent_warned: false,
                    objects: model::new_objects(),
                    settings: BTreeMap::new(),
                    loading: Vec::new(),
                    pending: HashMap::new(),
                    prefill: None,
                    dirty: None,
                    emitted: None,
                    current: None,
                };
                let end = rt.session(&ctx, &ids).await;
                if rt.emitted.is_some() {
                    unreachable_sent = false;
                }
                rt.cancel_secrets(None);
                drop(rt);
                drop(ticker);
                close(conn).await;
                if matches!(end, End::Stop) {
                    return;
                }
                tracing::warn!("ağ: sistem bus bağlantısı koptu, yeniden bağlanılacak");
            }
            Err(e) => tracing::warn!("ağ: sistem bus'ına bağlanılamadı: {e}"),
        }
        // Bus yok: ayrı neden, aynı durum iki kez yayınlanmaz (yeniden denemelerde).
        if !unreachable_sent {
            unreachable_sent = true;
            sink(NetworkEvent::State(NetworkState { unavailable: Some(Unavailable::BusUnreachable), ..NetworkState::default() }));
        }
        if !ctx.sleep(backoff.next_delay()).await {
            return;
        }
    }
}

struct Pending {
    connection_path: String,
    reply: Sender<Option<Secret>>,
}

struct Runtime<'a, F> {
    sink: &'a F,
    options: &'a NetworkOptions,
    conn: Connection,
    nm: NameOwner,
    agent_owner: Arc<Mutex<Option<String>>>,
    agent: AgentReg,
    /// NM sahibi her değiştiğinde artar; eski kayıt cevapları yok sayılır.
    agent_generation: u64,
    /// Kayıt hatası bir kez uyarı olarak yazıldı (sonrakiler debug).
    agent_warned: bool,
    objects: ObjectStore,
    settings: BTreeMap<String, SettingsSummary>,
    /// `GetSettings` süren bağlantılar.
    loading: Vec<String>,
    pending: HashMap<u64, Pending>,
    /// (bağlantı yolu, şifre, son geçerlilik)
    prefill: Option<(String, Secret, Instant)>,
    dirty: Option<Instant>,
    emitted: Option<NetworkState>,
    /// İşlenen isteğin kimliği (komut işlenirken; sonuç olaylarına taşınır).
    current: Option<RequestId>,
}

impl<F: Fn(NetworkEvent)> Runtime<'_, F> {
    async fn session(&mut self, ctx: &ServiceContext<Request>, ids: &Arc<AtomicU64>) -> End {
        let (tx, rx) = async_channel::bounded::<Input>(INPUT_CAPACITY);
        let agent = SecretAgent {
            input: tx.clone(),
            owner: Arc::clone(&self.agent_owner),
            ids: Arc::clone(ids),
            timeout: self.options.secret_timeout,
        };
        if self.options.agent_id.is_some()
            && let Err(e) = self.conn.object_server().at(SECRET_AGENT_PATH, agent).await
        {
            tracing::warn!("ağ: gizli bilgi ajanı yayınlanamadı: {e}");
        }
        let streams = async {
            Ok::<_, zbus::Error>((
                owner_changes(&self.conn, NM).await?,
                signals_from(&self.conn, NM, None, None).await?,
            ))
        };
        let (owners, signals) = match streams.await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("ağ: sinyal aboneliği kurulamadı: {e}");
                return End::Lost;
            }
        };
        let _tasks = [
            forward(&ctx.ex, owners, tx.clone(), |m| {
                if parse_owner_change(&m, NM).is_some() { Input::Owner } else { Input::Ignore }
            }),
            forward(&ctx.ex, signals, tx.clone(), Input::Signal),
        ];
        let owner = get_name_owner(&self.conn, NM).await;
        self.set_owner(ctx, &tx, owner).await;

        loop {
            let deadline = self.dirty;
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
                Wake::Timer => self.flush(),
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

    fn state(&self) -> NetworkState {
        model::state(&self.objects, &self.settings, self.nm.owner.is_some())
    }

    fn flush(&mut self) {
        self.dirty = None;
        let state = self.state();
        if self.emitted.as_ref() == Some(&state) {
            return;
        }
        self.emitted = Some(state.clone());
        (self.sink)(NetworkEvent::State(state));
    }

    fn fail(&self, action: &'static str, error: NmError) {
        // NM hata metni bağlantı adı/SSID içerebilir: yalnızca UI'a gider, loglanmaz.
        tracing::warn!(action, "ağ eylemi başarısız");
        (self.sink)(NetworkEvent::Failed { request: self.current, action, error });
    }

    /// Bekleyen gizli bilgi isteklerini iptal eder (`None`: hepsi).
    fn cancel_secrets(&mut self, connection_path: Option<&str>) {
        let ids: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, p)| connection_path.is_none_or(|c| p.connection_path == c))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.pending.remove(&id);
            (self.sink)(NetworkEvent::SecretDone { id });
        }
    }

    async fn set_owner(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>, owner: Option<String>) {
        if owner.is_some() && owner == self.nm.owner {
            return;
        }
        self.nm.set_owner(owner.clone());
        // Yeni NM sürecinde hiçbir ajan kayıtlı değildir; süren eski kayıt cevabı yok sayılır.
        self.agent_generation += 1;
        self.agent = AgentReg::None;
        if let Ok(mut o) = self.agent_owner.lock() {
            *o = owner;
        }
        self.objects.clear();
        self.settings.clear();
        self.loading.clear();
        self.cancel_secrets(None);
        if self.nm.owner.is_some() {
            match get_managed_objects(&self.conn, NM, OBJECT_ROOT, MAX_OBJECTS).await {
                Ok((objects, reply)) => {
                    self.nm.loaded(&reply);
                    for (path, ifaces) in objects {
                        self.objects.add(path.as_str(), ifaces);
                    }
                }
                Err(e) => tracing::warn!("ağ: GetManagedObjects: {e}"),
            }
            self.register_agent(ctx, tx);
            self.reconcile_settings(ctx, tx);
        }
        self.dirty = Some(Instant::now());
    }

    /// Ajanı şu anki NM sahibine kaydeder. İdempotent: kayıtlıyken ya da kayıt sürerken
    /// hiçbir şey yapmaz (aynı kimlikle ikinci kayıt NM'de reddedilir).
    fn register_agent(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>) {
        let Some(id) = self.options.agent_id.clone() else { return };
        if self.nm.owner.is_none() || matches!(self.agent, AgentReg::Pending | AgentReg::Registered) {
            return;
        }
        self.agent = AgentReg::Pending;
        let generation = self.agent_generation;
        let conn = self.conn.clone();
        let tx = tx.clone();
        ctx.ex
            .spawn(async move {
                let result = conn
                    .call_method(Some(NM), AGENT_MANAGER_PATH, Some(AGENT_MANAGER_IFACE), "Register", &(id.as_str(),))
                    .await
                    .map(drop)
                    .map_err(|e| RegisterError::classify(&e));
                let _ = tx.send(Input::AgentRegistered { generation, result }).await;
            })
            .detach();
    }

    fn agent_registered(&mut self, generation: u64, result: Result<(), RegisterError>) {
        if generation != self.agent_generation {
            return;
        }
        let id = self.options.agent_id.as_deref().unwrap_or_default();
        match result {
            Ok(()) => {
                if self.agent == AgentReg::Pending && self.agent_warned {
                    tracing::info!(id, "ağ: gizli bilgi ajanı sonunda kaydedildi");
                }
                self.agent = AgentReg::Registered;
            }
            Err(e) => {
                self.agent = AgentReg::Rejected;
                let first = !std::mem::replace(&mut self.agent_warned, true);
                match (&e, first) {
                    (RegisterError::Taken, true) => tracing::warn!(
                        id,
                        "ağ: NetworkManager gizli bilgi ajanı kimliği bu kullanıcı için başka bir süreçte zaten \
                         kayıtlı (büyük olasılıkla ikinci bir nuxut-shell örneği); bu örnek Wi-Fi şifresi soramaz, \
                         Wi-Fi paneli kullanılınca yeniden denenecek"
                    ),
                    (RegisterError::Other(m), true) => tracing::warn!(id, "ağ: gizli bilgi ajanı kaydedilemedi: {m}"),
                    (_, false) => tracing::debug!(id, ?e, "ağ: gizli bilgi ajanı yine kaydedilemedi"),
                }
            }
        }
    }

    /// Ağaçtaki kayıtlı bağlantılarla ayar önbelleğini eşitler: yenileri okur, gidenleri siler.
    fn reconcile_settings(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>) {
        let paths: Vec<String> = self.objects.with(SETTINGS_CONN_IFACE).map(|(p, _)| p.to_owned()).collect();
        let before = self.settings.len();
        self.settings.retain(|p, _| paths.contains(p));
        if self.settings.len() != before {
            self.mark_dirty();
        }
        for path in paths {
            if !self.settings.contains_key(&path) && !self.loading.contains(&path) {
                self.load_settings(ctx, tx, path);
            }
        }
    }

    fn load_settings(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>, path: String) {
        self.loading.push(path.clone());
        let conn = self.conn.clone();
        let tx = tx.clone();
        ctx.ex
            .spawn(async move {
                let result = async {
                    let reply = conn
                        .call_method(Some(NM), path.as_str(), Some(SETTINGS_CONN_IFACE), "GetSettings", &())
                        .await
                        .ok()?;
                    reply.body().deserialize::<Settings>().ok()
                }
                .await;
                let _ = tx.send(Input::Settings { path, result }).await;
            })
            .detach();
    }

    async fn input(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>, input: Input) {
        match input {
            Input::Ignore => {}
            Input::Owner => {
                let owner = get_name_owner(&self.conn, NM).await;
                self.set_owner(ctx, tx, owner).await;
            }
            Input::Signal(msg) => {
                if !self.nm.accepts(&msg) {
                    return;
                }
                let header = msg.header();
                // Kayıtlı bağlantı değişti: ayarları yeniden oku.
                if header.interface().map(|i| i.as_str()) == Some(SETTINGS_CONN_IFACE)
                    && header.member().map(|m| m.as_str()) == Some("Updated")
                    && let Some(path) = header.path().map(|p| p.to_string())
                    && self.settings.contains_key(&path)
                    && !self.loading.contains(&path)
                {
                    self.load_settings(ctx, tx, path);
                    return;
                }
                if header.member().map(|m| m.as_str()) == Some("StateChanged")
                    && let Some(failure) = self.activation_failure(&msg)
                {
                    (self.sink)(failure);
                }
                if apply_object_signal(&mut self.objects, &msg) {
                    self.mark_dirty();
                    if header.interface().map(|i| i.as_str()) == Some(nuxut_dbus::OBJECT_MANAGER) {
                        self.reconcile_settings(ctx, tx);
                    }
                }
            }
            Input::Settings { path, result } => {
                self.loading.retain(|p| *p != path);
                if !self.objects.has(&path, SETTINGS_CONN_IFACE) {
                    return;
                }
                match result.as_ref().and_then(SettingsSummary::parse) {
                    Some(summary) => {
                        self.settings.insert(path, summary);
                    }
                    None => tracing::debug!(path, "ağ: bağlantı ayarları okunamadı"),
                }
                self.mark_dirty();
            }
            Input::Secret(call) => self.secret_call(call),
            Input::CancelSecret { connection_path } => self.cancel_secrets(Some(&connection_path)),
            Input::Failed { request, action, error } => {
                self.current = request;
                self.fail(action, error);
                self.current = None;
            }
            Input::Done { request, action, object } => {
                // Alınmış durum değişikliği sonuçtan önce yayınlansın.
                if self.dirty.is_some() {
                    self.flush();
                }
                (self.sink)(NetworkEvent::Done { request, action, object });
            }
            Input::Details(d) => (self.sink)(NetworkEvent::DeviceDetails(*d)),
            Input::AgentRegistered { generation, result } => self.agent_registered(generation, result),
        }
    }

    fn secret_call(&mut self, call: SecretCall) {
        // Önceden verilmiş şifre (bağlanırken girilmiş) varsa UI'a sormadan kullan.
        if let Some((path, secret, until)) = self.prefill.take() {
            if path == call.connection_path && Instant::now() < until {
                let _ = call.reply.try_send(Some(secret));
                return;
            }
            if Instant::now() < until {
                self.prefill = Some((path, secret, until));
            }
        }
        if self.pending.len() >= MAX_PENDING {
            return; // Gönderen düşer → ajan iptal döner.
        }
        let request = SecretRequest {
            id: call.id,
            connection: call.connection_id,
            ssid: call.ssid.as_deref().map(ssid_display),
            kind: call.kind,
            retry: call.retry,
        };
        self.pending.insert(call.id, Pending { connection_path: call.connection_path, reply: call.reply });
        (self.sink)(NetworkEvent::SecretRequest(request));
    }

    /// Sonuç nesnesiz çağrı ([`Runtime::spawn_done`] gibi; başarıda `Done.object = None`).
    fn spawn_call(
        &self,
        ctx: &ServiceContext<Request>,
        tx: &Sender<Input>,
        action: &'static str,
        call: impl std::future::Future<Output = zbus::Result<()>> + 'static,
    ) {
        self.spawn_done(ctx, tx, action, async move { call.await.map(|()| None).map_err(NmError::from) });
    }

    fn command(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>, request: Request) {
        self.current = Some(request.id);
        self.run_command(ctx, tx, request.command);
        self.current = None;
    }

    fn run_command(&mut self, ctx: &ServiceContext<Request>, tx: &Sender<Input>, command: Command) {
        if let Command::Secret { id, secret } = command {
            if let Some(p) = self.pending.remove(&id) {
                let _ = p.reply.try_send(secret);
                (self.sink)(NetworkEvent::SecretDone { id });
            }
            return;
        }
        if self.nm.owner.is_none() {
            self.fail("networkmanager", NmError::NmUnavailable);
            return;
        }
        // Reddedilmiş kayıt (kimliği tutan diğer süreç kapanmış olabilir): şifre gerekebilecek
        // Wi-Fi eylemlerinde yeniden dene. Boşta zamanlayıcı/uyanma yok.
        if self.agent == AgentReg::Rejected
            && matches!(
                command,
                Command::Scan | Command::ConnectAp { .. } | Command::ConnectHidden { .. } | Command::Activate(_)
            )
        {
            self.register_agent(ctx, tx);
        }
        let conn = self.conn.clone();
        match command {
            Command::Secret { .. } => {}
            Command::SetWifi(on) => self.spawn_call(ctx, tx, "wifi", async move {
                set_property(&conn, NM, NM_PATH, NM_IFACE, "WirelessEnabled", Value::Bool(on)).await
            }),
            Command::Scan => {
                // Tarama sonuç olayı vermez (cihaz başına çağrı); hatası kimliksiz gelir.
                self.current = None;
                for (path, _) in self.objects.with(WIRELESS_IFACE) {
                    let conn = conn.clone();
                    let path = path.to_owned();
                    self.spawn_call(ctx, tx, "scan", async move {
                        let options: HashMap<&str, Value<'_>> = HashMap::new();
                        match conn
                            .call_method(Some(NM), path.as_str(), Some(WIRELESS_IFACE), "RequestScan", &(options,))
                            .await
                        {
                            // Çok sık tarama isteği NM'de hata verir; zararsız.
                            Err(zbus::Error::MethodError(name, _, _)) if name.as_str().ends_with("NotAllowed") => {
                                Ok(())
                            }
                            r => r.map(drop),
                        }
                    });
                }
            }
            Command::ConnectAp { ap, password } => self.connect_ap(ctx, tx, &ap, password),
            Command::Activate(path) => {
                if !self.settings.contains_key(&path) {
                    self.fail("activate", NmError::ConnectionUnknown);
                    return;
                }
                self.spawn_call(ctx, tx, "activate", async move { activate(&conn, &path, "/", "/").await });
            }
            Command::Deactivate(path) => {
                if !self.objects.has(&path, model::ACTIVE_IFACE) {
                    self.fail("deactivate", NmError::ConnectionInactive);
                    return;
                }
                self.spawn_call(ctx, tx, "deactivate", async move {
                    let p = ObjectPath::try_from(path.as_str()).map_err(zbus::Error::from)?;
                    conn.call_method(Some(NM), NM_PATH, Some(NM_IFACE), "DeactivateConnection", &(p,)).await.map(drop)
                });
            }
            Command::Forget(path) => {
                if !self.settings.contains_key(&path) {
                    self.fail("forget", NmError::ConnectionUnknown);
                    return;
                }
                self.spawn_call(ctx, tx, "forget", async move {
                    conn.call_method(Some(NM), path.as_str(), Some(SETTINGS_CONN_IFACE), "Delete", &()).await.map(drop)
                });
            }
            Command::ConnectHidden { ssid, security, password } => {
                self.connect_hidden(ctx, tx, ssid, security, password);
            }
            Command::Patch { connection, patch } => {
                let action = patch.action();
                if !self.settings.contains_key(&connection) {
                    self.fail(action, NmError::ConnectionUnknown);
                    return;
                }
                self.spawn_done(ctx, tx, action, async move {
                    // Gizli bilgisiz okuma; NM gizli bilgisiz `Update`'te kayıtlı gizli bilgileri
                    // korur (nm-settings-connection: "keep all the existing ones").
                    let reply = conn
                        .call_method(Some(NM), connection.as_str(), Some(SETTINGS_CONN_IFACE), "GetSettings", &())
                        .await?;
                    let mut settings: Settings = reply.body().deserialize()?;
                    patch.apply(&mut settings)?;
                    conn.call_method(Some(NM), connection.as_str(), Some(SETTINGS_CONN_IFACE), "Update", &(settings,))
                        .await?;
                    Ok(Some(connection))
                });
            }
            Command::ImportWireGuard { name, config } => {
                self.spawn_done(ctx, tx, "import_wireguard", async move {
                    // Anahtarlar `config`'ten ödünç; çağrı bitince sıfırlanır.
                    let settings = config.nm_settings(&name);
                    let reply = conn
                        .call_method(Some(NM), SETTINGS_PATH, Some(SETTINGS_IFACE), "AddConnection", &(settings,))
                        .await?;
                    let path: OwnedObjectPath = reply.body().deserialize()?;
                    Ok(Some(path.to_string()))
                });
            }
            Command::SetPassword { connection, password } => {
                const ACTION: &str = "wifi_password";
                if !self.settings.contains_key(&connection) {
                    self.fail(ACTION, NmError::ConnectionUnknown);
                    return;
                }
                self.spawn_done(ctx, tx, ACTION, async move {
                    let reply = conn
                        .call_method(Some(NM), connection.as_str(), Some(SETTINGS_CONN_IFACE), "GetSettings", &())
                        .await?;
                    let settings: Settings = reply.body().deserialize()?;
                    let body = password_update(&settings, &password)?;
                    conn.call_method(Some(NM), connection.as_str(), Some(SETTINGS_CONN_IFACE), "Update", &(body,))
                        .await?;
                    Ok(Some(connection))
                });
            }
            Command::DeviceDetails(path) => {
                let Some(device) = self.objects.get(&path, model::DEVICE_IFACE) else {
                    self.fail("device_details", NmError::DeviceUnknown);
                    return;
                };
                let config_path = |key: &str| nuxut_dbus::prop_str(device, key, 256).filter(|p| p != "/");
                let (ip4, ip6) = (config_path("Ip4Config"), config_path("Ip6Config"));
                let wired = nuxut_dbus::prop_u32(device, "DeviceType") == Some(DEVICE_ETHERNET);
                let objects = self.details_snapshot(&path);
                let tx2 = tx.clone();
                self.spawn_call(ctx, tx, "device_details", async move {
                    let get = |p: Option<String>, iface: &'static str| {
                        let conn = conn.clone();
                        async move {
                            match p {
                                Some(p) => nuxut_dbus::get_all(&conn, NM, &p, iface).await.ok().map(|(props, _)| props),
                                None => None,
                            }
                        }
                    };
                    let ip4 = get(ip4, model::IP4_IFACE).await;
                    let ip6 = get(ip6, model::IP6_IFACE).await;
                    let wired = get(wired.then(|| path.clone()), model::WIRED_IFACE).await;
                    let details = model::details(&objects, &path, ip4.as_ref(), ip6.as_ref(), wired.as_ref())
                        .ok_or_else(|| zbus::Error::Failure("cihaz kayboldu".into()))?;
                    let _ = tx2.send(Input::Details(Box::new(details))).await;
                    Ok(())
                });
            }
        }
    }

    /// Ayrıntı için cihazın ağaç kopyası (yalnız o yolun Device/Wireless arayüzleri).
    fn details_snapshot(&self, path: &str) -> ObjectStore {
        let mut store = model::new_objects();
        let ifaces = [model::DEVICE_IFACE, WIRELESS_IFACE]
            .into_iter()
            .filter_map(|i| self.objects.get(path, i).map(|p| (i.to_owned(), p.clone())));
        store.add(path, ifaces);
        store
    }

    /// Eylem çağrısı: başarıda (istek varsa) [`NetworkEvent::Done`], hatada
    /// [`NetworkEvent::Failed`]; ikisi de işlenen isteğin kimliğiyle.
    fn spawn_done(
        &self,
        ctx: &ServiceContext<Request>,
        tx: &Sender<Input>,
        action: &'static str,
        call: impl std::future::Future<Output = Result<Option<String>, NmError>> + 'static,
    ) {
        let tx = tx.clone();
        let request = self.current;
        ctx.ex
            .spawn(async move {
                let input = match call.await {
                    Ok(object) => match request {
                        Some(request) => Input::Done { request, action, object },
                        None => return,
                    },
                    Err(error) => Input::Failed { request, action, error },
                };
                let _ = tx.send(input).await;
            })
            .detach();
    }

    /// Cihazın ya da VPN etkin bağlantısının `StateChanged` sinyalinden etkinleşme hatası.
    /// Cihaz: yeni durum `FAILED` (120) ve neden gizli bilgi/supplicant. VPN/WireGuard etkin
    /// bağlantısı: `DEACTIVATED` (4) ve neden `NO_SECRETS`/`LOGIN_FAILED` (Wi-Fi'de cihaz
    /// sinyali yeter; çift bildirim olmasın diye etkin bağlantıda yalnız VPN).
    fn activation_failure(&self, msg: &Message) -> Option<NetworkEvent> {
        let header = msg.header();
        let path = header.path()?.to_string();
        let active_connection = |active: &str| {
            self.objects.get(active, model::ACTIVE_IFACE).and_then(|a| nuxut_dbus::prop_str(a, "Connection", 256))
        };
        match header.interface()?.as_str() {
            model::DEVICE_IFACE => {
                let (new, _old, reason): (u32, u32, u32) = msg.body().deserialize().ok()?;
                if new != DEVICE_FAILED {
                    return None;
                }
                let reason = match reason {
                    7 => ActivationFailure::NoSecrets,
                    8..=11 => ActivationFailure::AuthFailed,
                    r => ActivationFailure::Other(r),
                };
                let connection = self
                    .objects
                    .get(&path, model::DEVICE_IFACE)
                    .and_then(|d| nuxut_dbus::prop_str(d, "ActiveConnection", 256))
                    .filter(|p| p != "/")
                    .and_then(|a| active_connection(&a));
                Some(NetworkEvent::ActivationFailed { connection, reason })
            }
            model::ACTIVE_IFACE => {
                let (state, reason): (u32, u32) = msg.body().deserialize().ok()?;
                let a = self.objects.get(&path, model::ACTIVE_IFACE)?;
                let vpn = ConnectionKind::parse(&nuxut_dbus::prop_str(a, "Type", 64).unwrap_or_default()).is_vpn();
                let reason = match reason {
                    9 => ActivationFailure::NoSecrets,
                    10 => ActivationFailure::AuthFailed,
                    _ => return None,
                };
                (state == ACTIVE_DEACTIVATED && vpn)
                    .then(|| NetworkEvent::ActivationFailed { connection: active_connection(&path), reason })
            }
            _ => None,
        }
    }

    fn connect_hidden(
        &mut self,
        ctx: &ServiceContext<Request>,
        tx: &Sender<Input>,
        ssid: Vec<u8>,
        security: Security,
        password: Option<Secret>,
    ) {
        const ACTION: &str = "connect_hidden";
        if ssid.is_empty() || ssid.len() > MAX_SSID {
            self.fail(ACTION, NmError::InvalidSsid);
            return;
        }
        if security == Security::Enterprise {
            self.fail(ACTION, NmError::Enterprise);
            return;
        }
        if let Some(p) = &password
            && !valid_secret(security, p.expose())
        {
            self.fail(ACTION, NmError::InvalidPassword);
            return;
        }
        let Some(device) = self.objects.with(WIRELESS_IFACE).next().map(|(p, _)| p.to_owned()) else {
            self.fail(ACTION, NmError::NoWifiDevice);
            return;
        };
        let conn = self.conn.clone();
        self.spawn_done(ctx, tx, ACTION, async move {
            let id = ssid_display(&ssid);
            let settings = new_wifi_settings(&id, &ssid, security, password.as_ref(), true);
            let (saved, _active) = add_and_activate(&conn, settings, &device, "/").await?;
            Ok(Some(saved.to_string()))
        });
    }

    fn connect_ap(
        &mut self,
        ctx: &ServiceContext<Request>,
        tx: &Sender<Input>,
        ap_path: &str,
        password: Option<Secret>,
    ) {
        let state = self.state();
        let Some(ap) = state.access_points.iter().find(|a| a.path == ap_path).cloned().or_else(|| {
            // Birleştirilmiş listede görünmeyen BSS: doğrudan ağaçtan.
            let p = self.objects.get(ap_path, AP_IFACE)?;
            let ssid_bytes = nuxut_dbus::prop_bytes(p, "Ssid", MAX_SSID)?;
            Some(AccessPoint {
                path: ap_path.to_owned(),
                ssid: ssid_display(&ssid_bytes),
                security: security_from_flags(
                    nuxut_dbus::prop_u32(p, "Flags").unwrap_or(0),
                    nuxut_dbus::prop_u32(p, "WpaFlags").unwrap_or(0),
                    nuxut_dbus::prop_u32(p, "RsnFlags").unwrap_or(0),
                ),
                ssid_bytes,
                strength: 0,
                frequency: 0,
                active: false,
                saved: false,
            })
        }) else {
            self.fail("connect", NmError::ApUnknown);
            return;
        };
        if ap.security == Security::Enterprise {
            self.fail("connect", NmError::Enterprise);
            return;
        }
        if let Some(p) = &password
            && !valid_secret(ap.security, p.expose())
        {
            self.fail("connect", NmError::InvalidPassword);
            return;
        }
        // Erişim noktasını gören cihaz (yoksa ilk Wi-Fi cihazı).
        let device = self
            .objects
            .with(WIRELESS_IFACE)
            .find(|(_, w)| prop_paths(w, "AccessPoints", MAX_ACCESS_POINTS).iter().any(|p| p == ap_path))
            .or_else(|| self.objects.with(WIRELESS_IFACE).next())
            .map(|(p, _)| p.to_owned());
        let Some(device) = device else {
            self.fail("connect", NmError::NoWifiDevice);
            return;
        };
        let conn = self.conn.clone();
        let ap_path = ap.path.clone();
        let saved = self
            .settings
            .iter()
            .find(|(_, s)| s.ssid.as_deref() == Some(ap.ssid_bytes.as_slice()))
            .map(|(p, _)| p.clone());
        if let Some(saved) = saved {
            if let Some(pw) = password {
                self.prefill = Some((saved.clone(), pw, Instant::now() + PREFILL_TTL));
            }
            let object = saved.clone();
            self.spawn_done(ctx, tx, "connect", async move {
                activate(&conn, &saved, &device, &ap_path).await?;
                Ok(Some(object))
            });
            return;
        }
        self.spawn_done(ctx, tx, "connect", async move {
            // Şifre geleceğin içinde: ayarlar ondan ödünç alır, çağrı bitince sıfırlanır.
            let settings = new_wifi_settings(&ap.ssid, &ap.ssid_bytes, ap.security, password.as_ref(), false);
            let (saved, _active) = add_and_activate(&conn, settings, &device, &ap_path).await?;
            Ok(Some(saved.to_string()))
        });
    }
}

async fn activate(conn: &Connection, connection: &str, device: &str, specific: &str) -> zbus::Result<()> {
    let c = ObjectPath::try_from(connection).map_err(zbus::Error::from)?;
    let d = ObjectPath::try_from(device).map_err(zbus::Error::from)?;
    let s = ObjectPath::try_from(specific).map_err(zbus::Error::from)?;
    conn.call_method(Some(NM), NM_PATH, Some(NM_IFACE), "ActivateConnection", &(c, d, s)).await.map(drop)
}

/// `NMSettingSecretFlags`.
const SECRET_AGENT_OWNED: u32 = 0x1;
const SECRET_NOT_SAVED: u32 = 0x2;
const SECRET_NOT_REQUIRED: u32 = 0x4;

/// Şifre güncelleme `Update` gövdesi: `GetSettings` çıktısı (gizli bilgisiz) aynen + `psk` +
/// `psk-flags = 0`. Şifre değeri ödünç (kopya yok). Yalnız WPA/WPA2/WPA3 kişisel (`wpa-psk`,
/// `sae`); WEP ve diğerleri `Unsupported` (2026-10-14). `psk-flags`:
/// - `0` (sistem saklar): olduğu gibi.
/// - `AGENT_OWNED` (1) ve/veya `NOT_REQUIRED` (4): `0` yapılır — Nuxut'un kabuk ajanı
///   `SaveSecrets` saklamaz (ajan sahipli şifre Nuxut'ta kalıcı değil); şifre verildiğine göre
///   gerekli.
/// - `NOT_SAVED` (2) ya da tanınmayan bit: kullanıcının bilinçli seçimi / bilinmeyen anlam —
///   `SecretNotSaved` (değiştirilmez).
///
/// Gövdede gizli bilgi olduğundan NM kayıtlı gizli bilgileri birleştirmez; kişisel Wi-Fi'de tek
/// gizli bilgi bu şifredir.
fn password_update<'a>(
    settings: &'a Settings,
    password: &'a Secret,
) -> Result<HashMap<&'a str, HashMap<&'a str, Value<'a>>>, NmError> {
    let security = match key_mgmt(settings).as_deref() {
        Some("wpa-psk") => Security::WpaPsk,
        Some("sae") => Security::Sae,
        _ => return Err(NmError::Unsupported),
    };
    let flags = settings
        .get("802-11-wireless-security")
        .and_then(|g| g.get("psk-flags"))
        .and_then(|v| nuxut_dbus::value_i64(v))
        .unwrap_or(0);
    let known = i64::from(SECRET_AGENT_OWNED | SECRET_NOT_REQUIRED);
    if flags & i64::from(SECRET_NOT_SAVED) != 0 || flags & !known != 0 {
        return Err(NmError::SecretNotSaved);
    }
    if !valid_secret(security, password.expose()) {
        return Err(NmError::InvalidPassword);
    }
    let mut body: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::new();
    for (group, values) in settings {
        let mut inner = HashMap::new();
        for (k, v) in values {
            inner.insert(k.as_str(), Value::try_clone(v).map_err(|e| NmError::from(zbus::Error::from(e)))?);
        }
        body.insert(group.as_str(), inner);
    }
    let sec = body.entry("802-11-wireless-security").or_default();
    sec.insert("psk", password.value());
    sec.insert("psk-flags", Value::U32(0));
    Ok(body)
}

/// `AddAndActivateConnection`; (kayıtlı bağlantı, etkin bağlantı) yolları.
async fn add_and_activate(
    conn: &Connection,
    settings: HashMap<&'static str, HashMap<&'static str, Value<'_>>>,
    device: &str,
    specific: &str,
) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)> {
    let dev = ObjectPath::try_from(device).map_err(zbus::Error::from)?;
    let spec = ObjectPath::try_from(specific).map_err(zbus::Error::from)?;
    let reply =
        conn.call_method(Some(NM), NM_PATH, Some(NM_IFACE), "AddAndActivateConnection", &(settings, dev, spec)).await?;
    reply.body().deserialize()
}

/// Yeni Wi-Fi bağlantısının ayarları. Şifre verilmemişse NM ajandan ister. Şifre değeri
/// ödünçtür (kopya yok): ayarlar şifreden uzun yaşayamaz; çağrı geleceği şifreyi sahiplenir.
fn new_wifi_settings<'a>(
    id: &str,
    ssid: &[u8],
    security: Security,
    password: Option<&'a Secret>,
    hidden: bool,
) -> HashMap<&'static str, HashMap<&'static str, Value<'a>>> {
    let mut s = HashMap::new();
    s.insert("connection", HashMap::from([("id", Value::from(id.to_owned())), ("type", Value::from("802-11-wireless"))]));
    let mut wireless =
        HashMap::from([("ssid", Value::from(ssid.to_vec())), ("mode", Value::from("infrastructure"))]);
    if hidden {
        wireless.insert("hidden", Value::Bool(true));
    }
    s.insert("802-11-wireless", wireless);
    if let Some(km) = security.key_mgmt() {
        let mut sec = HashMap::from([("key-mgmt", Value::from(km))]);
        if let Some(pw) = password {
            match security {
                Security::Wep => {
                    // 5/13 karakter ya da 10/26 onaltılık: anahtar; değilse parola.
                    let key_type = if matches!(pw.len(), 5 | 10 | 13 | 26) { 1u32 } else { 2 };
                    sec.insert("wep-key0", pw.value());
                    sec.insert("wep-key-type", Value::U32(key_type));
                }
                Security::WpaPsk | Security::Sae => {
                    sec.insert("psk", pw.value());
                }
                _ => {}
            }
        }
        s.insert("802-11-wireless-security", sec);
    }
    s
}

// ---------------------------------------------------------------------------------------
// Gizli bilgi ajanı

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.SecretAgent")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    UserCanceled(String),
    NoSecrets(String),
    PermissionDenied(String),
}

/// NM `GetSecrets` bayrakları.
const ALLOW_INTERACTION: u32 = 0x1;
const REQUEST_NEW: u32 = 0x2;

struct SecretAgent {
    input: Sender<Input>,
    owner: Arc<Mutex<Option<String>>>,
    ids: Arc<AtomicU64>,
    timeout: Duration,
}

impl SecretAgent {
    fn check(&self, header: &Header<'_>) -> Result<(), AgentError> {
        let owner = self.owner.lock().ok().and_then(|o| o.clone());
        match (header.sender(), owner) {
            (Some(s), Some(o)) if s.as_str() == o => Ok(()),
            _ => {
                tracing::warn!("ağ: NetworkManager dışından ajan çağrısı reddedildi");
                Err(AgentError::PermissionDenied("yetkisiz çağıran".into()))
            }
        }
    }
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl SecretAgent {
    async fn get_secrets(
        &self,
        #[zbus(header)] header: Header<'_>,
        connection: Settings,
        connection_path: OwnedObjectPath,
        setting_name: String,
        _hints: Vec<String>,
        flags: u32,
    ) -> Result<SecretsReply, AgentError> {
        self.check(&header)?;
        if setting_name != "802-11-wireless-security" {
            return Err(AgentError::NoSecrets(format!("{} desteklenmiyor", clean_str(&setting_name, 64))));
        }
        let (kind, key) = match key_mgmt(&connection).as_deref() {
            Some("wpa-psk" | "sae") => (SecretKind::WifiPassword, "psk"),
            Some("none") => (SecretKind::WepKey, "wep-key0"),
            _ => return Err(AgentError::NoSecrets("bu güvenlik türü desteklenmiyor".into())),
        };
        if flags & ALLOW_INTERACTION == 0 {
            return Err(AgentError::NoSecrets("etkileşim yok".into()));
        }
        let connection_id = model::setting_str(&connection, "connection", "id", 256).unwrap_or_default();
        let ssid = setting_bytes(&connection, "802-11-wireless", "ssid", MAX_SSID);
        let (reply_tx, reply_rx) = async_channel::bounded(1);
        let call = SecretCall {
            id: self.ids.fetch_add(1, Ordering::Relaxed),
            connection_path: connection_path.to_string(),
            connection_id,
            ssid,
            kind,
            retry: flags & REQUEST_NEW != 0,
            reply: reply_tx,
        };
        let id = call.id;
        if self.input.try_send(Input::Secret(call)).is_err() {
            return Err(AgentError::UserCanceled("meşgul".into()));
        }
        let answer = async { reply_rx.recv().await.ok().flatten() }
            .or(async {
                async_io::Timer::after(self.timeout).await;
                None
            })
            .await;
        let Some(secret) = answer else {
            let _ = self.input.try_send(Input::CancelSecret { connection_path: connection_path.to_string() });
            tracing::debug!(id, "ağ: şifre isteği iptal/zaman aşımı");
            return Err(AgentError::UserCanceled("iptal".into()));
        };
        // Değer şifreden ödünç serileştirilir (ara kopya yok); cevap gidince şifre sıfırlanır.
        Ok(SecretsReply { setting: setting_name, key, secret })
    }

    fn cancel_get_secrets(
        &self,
        #[zbus(header)] header: Header<'_>,
        connection_path: OwnedObjectPath,
        _setting_name: String,
    ) -> Result<(), AgentError> {
        self.check(&header)?;
        let _ = self.input.try_send(Input::CancelSecret { connection_path: connection_path.to_string() });
        Ok(())
    }

    /// Şifreleri NM saklar; ajan saklamaz.
    fn save_secrets(
        &self,
        #[zbus(header)] header: Header<'_>,
        _connection: Settings,
        _connection_path: OwnedObjectPath,
    ) -> Result<(), AgentError> {
        self.check(&header)
    }

    fn delete_secrets(
        &self,
        #[zbus(header)] header: Header<'_>,
        _connection: Settings,
        _connection_path: OwnedObjectPath,
    ) -> Result<(), AgentError> {
        self.check(&header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_debug_is_redacted() {
        let e = NmError::CallFailed("Ev Ağı bağlanamadı".into());
        assert_eq!(format!("{e:?}"), "CallFailed(<redacted>)");
        let ev = NetworkEvent::Failed { request: Some(RequestId(1)), action: "connect", error: e };
        assert!(!format!("{ev:?}").contains("Ev Ağı"));
        assert_eq!(format!("{:?}", NmError::NoWifiDevice), "NoWifiDevice");
    }

    #[test]
    fn secret_is_redacted() {
        let s = Secret::new("çokgizli123");
        assert_eq!(format!("{s:?}"), "Secret(<gizli>)");
        assert!(!format!("{:?}", Command::Secret { id: 1, secret: Some(s) }).contains("çokgizli"));
    }

    #[test]
    fn register_error_classification() {
        let err = |name: &str, msg: &str| {
            zbus::Error::MethodError(
                zbus::names::OwnedErrorName::try_from(name.to_owned()).unwrap(),
                Some(msg.to_owned()),
                zbus::message::Message::method_call("/", "X").unwrap().build(&()).unwrap(),
            )
        };
        let taken = err(
            "org.freedesktop.NetworkManager.AgentManager.PermissionDenied",
            "An agent with this ID is already registered for this user.",
        );
        assert_eq!(RegisterError::classify(&taken), RegisterError::Taken);
        let other = err("org.freedesktop.NetworkManager.AgentManager.PermissionDenied", "Unable to determine UID");
        assert!(matches!(RegisterError::classify(&other), RegisterError::Other(_)));
        let invalid = err("org.freedesktop.NetworkManager.AgentManager.InvalidIdentifier", "already registered");
        assert!(matches!(RegisterError::classify(&invalid), RegisterError::Other(_)));
    }

    #[test]
    fn secret_validation() {
        assert!(valid_secret(Security::WpaPsk, "12345678"));
        assert!(!valid_secret(Security::WpaPsk, "1234567"));
        assert!(valid_secret(Security::WpaPsk, &"a".repeat(64).replace('a', "f")));
        assert!(!valid_secret(Security::WpaPsk, &"g".repeat(64)));
        assert!(!valid_secret(Security::WpaPsk, "şifreşifre"));
        assert!(valid_secret(Security::Sae, "şifre"));
        assert!(!valid_secret(Security::Open, "x"));
    }

    /// Şifre güncelleme gövdesi (güvenlik kritik, review şartı): yalnız WPA/SAE; bayraklar;
    /// şifre ödünç (kopya yok); diğer alanlar aynen; Debug'da şifre yok.
    #[test]
    fn password_update_rules() {
        fn ov(v: Value<'_>) -> OwnedValue {
            OwnedValue::try_from(v).unwrap()
        }
        let settings = |km: Option<&'static str>, flags: Option<u32>| -> Settings {
            let mut s: Settings = HashMap::from([(
                "connection".to_owned(),
                HashMap::from([("id".to_owned(), ov(Value::from("Ev"))), ("autoconnect".to_owned(), ov(Value::Bool(false)))]),
            )]);
            if let Some(km) = km {
                let mut sec = HashMap::from([("key-mgmt".to_owned(), ov(Value::from(km)))]);
                if let Some(f) = flags {
                    sec.insert("psk-flags".to_owned(), ov(Value::U32(f)));
                }
                s.insert("802-11-wireless-security".to_owned(), sec);
            }
            s
        };
        let pw = Secret::new("dogru-sifre-1");
        // Desteklenmeyen key-mgmt: WEP, kurumsal, OWE, açık ağ.
        for km in [Some("none"), Some("wpa-eap"), Some("owe"), Some("ieee8021x"), None] {
            assert_eq!(password_update(&settings(km, None), &pw).err(), Some(NmError::Unsupported), "{km:?}");
        }
        // Bayraklar: 2 (saklanmaz) ve tanınmayan bit → SecretNotSaved; 0/1/4/5 → psk-flags 0.
        for f in [2, 3, 6, 8, 0x10] {
            assert_eq!(password_update(&settings(Some("wpa-psk"), Some(f)), &pw).err(), Some(NmError::SecretNotSaved), "{f}");
        }
        for f in [None, Some(0), Some(1), Some(4), Some(5)] {
            let s = settings(Some("sae"), f);
            let body = password_update(&s, &pw).unwrap();
            assert_eq!(body["802-11-wireless-security"]["psk-flags"], Value::U32(0), "{f:?}");
            assert_eq!(body["802-11-wireless-security"]["key-mgmt"], Value::from("sae"));
            assert_eq!(body["connection"]["autoconnect"], Value::Bool(false), "diğer alan aynen");
            match &body["802-11-wireless-security"]["psk"] {
                Value::Str(v) => assert_eq!(v.as_str().as_ptr(), pw.expose().as_ptr(), "ödünç, kopya yok"),
                other => panic!("{other:?}"),
            }
        }
        // Geçersiz şifre.
        assert_eq!(password_update(&settings(Some("wpa-psk"), None), &Secret::new("kisa")).err(), Some(NmError::InvalidPassword));
        // Debug'da şifre yok (komut ve hata).
        let cmd = Command::SetPassword { connection: "/c".into(), password: Secret::new("gizli-sifre-9") };
        assert!(!format!("{cmd:?}").contains("gizli-sifre-9"));
    }

    #[test]
    fn new_settings_shape() {
        let pw = Secret::new("12345678");
        let s = new_wifi_settings("Ev", b"Ev", Security::WpaPsk, Some(&pw), false);
        assert_eq!(s["802-11-wireless-security"]["key-mgmt"], Value::from("wpa-psk"));
        assert_eq!(s["802-11-wireless-security"]["psk"], Value::from("12345678"));
        assert_eq!(s["802-11-wireless"]["ssid"], Value::from(b"Ev".to_vec()));
        assert!(!s["802-11-wireless"].contains_key("hidden"));
        // Şifre değeri ödünç (kopya yok): `Value::Str` şifrenin tamponunu gösterir.
        match &s["802-11-wireless-security"]["psk"] {
            Value::Str(v) => assert_eq!(v.as_str().as_ptr(), pw.expose().as_ptr()),
            other => panic!("{other:?}"),
        }
        assert!(!new_wifi_settings("Ev", b"Ev", Security::Open, None, false).contains_key("802-11-wireless-security"));
        let hidden = new_wifi_settings("Gizli", b"Gizli", Security::Open, None, true);
        assert_eq!(hidden["802-11-wireless"]["hidden"], Value::Bool(true));
    }
}
