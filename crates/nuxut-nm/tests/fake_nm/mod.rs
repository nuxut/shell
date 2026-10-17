//! Sahte NetworkManager: entegrasyon testleri (`tests/network_service.rs`, özel bus) ve
//! `examples/fake_nm.rs` (canlı test, `dbus-run-session` içindeki oturum bus'ı) paylaşır.
//!
//! Durumlu: kayıtlı bağlantılar tam ayarlarıyla (gizli bilgiler dahil) saklanır; `GetSettings`
//! gizli bilgisiz döner, `Update` gizli bilgisiz gelirse kayıtlıları korur (gerçek NM gibi),
//! `AddConnection`/`AddAndActivateConnection` yeni nesne, `Delete` nesneyi kaldırır.
//! Kayıtlı Wi-Fi bağlantısında gizli bilgi yoksa `ActivateConnection` ajana sorar; ajan yoksa
//! cihaz `FAILED` / `NO_SECRETS` sinyali verir. Her çağrı [`Log`]'a yazılır.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use zbus::fdo;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

pub const NM: &str = "org.freedesktop.NetworkManager";
pub const NM_PATH: &str = "/org/freedesktop/NetworkManager";
pub const SECRET_AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
pub const DEV: &str = "/org/freedesktop/NetworkManager/Devices/1";
pub const DEV_ETH: &str = "/org/freedesktop/NetworkManager/Devices/2";
pub const AP_HOME: &str = "/org/freedesktop/NetworkManager/AccessPoint/1";
pub const AP_CAFE: &str = "/org/freedesktop/NetworkManager/AccessPoint/2";
pub const AP_GUEST: &str = "/org/freedesktop/NetworkManager/AccessPoint/3";
pub const AP_HIDDEN: &str = "/org/freedesktop/NetworkManager/AccessPoint/4";
pub const AP_SAE: &str = "/org/freedesktop/NetworkManager/AccessPoint/5";
pub const SAVED_HOME: &str = "/org/freedesktop/NetworkManager/Settings/1";
pub const SAVED_OFFICE: &str = "/org/freedesktop/NetworkManager/Settings/2";
pub const SAVED_VPN: &str = "/org/freedesktop/NetworkManager/Settings/3";
pub const SAVED_WIRED: &str = "/org/freedesktop/NetworkManager/Settings/4";
/// Şifresi ajan sahipli (`psk-flags=1`) Wi-Fi.
pub const SAVED_AGENT_OWNED: &str = "/org/freedesktop/NetworkManager/Settings/5";
/// Şifresi saklanmayan (`psk-flags=2`) Wi-Fi.
pub const SAVED_NOT_SAVED: &str = "/org/freedesktop/NetworkManager/Settings/6";
/// Şifresi gerekli değil işaretli (`psk-flags=4`) Wi-Fi.
pub const SAVED_NOT_REQUIRED: &str = "/org/freedesktop/NetworkManager/Settings/7";
/// WEP (`key-mgmt=none`, `wep-key0..3`).
pub const SAVED_WEP: &str = "/org/freedesktop/NetworkManager/Settings/8";
pub const ACTIVE: &str = "/org/freedesktop/NetworkManager/ActiveConnection/1";
pub const ACTIVE_WIRED: &str = "/org/freedesktop/NetworkManager/ActiveConnection/2";
pub const IP4: &str = "/org/freedesktop/NetworkManager/IP4Config/1";
pub const IP6: &str = "/org/freedesktop/NetworkManager/IP6Config/1";
/// Örnek WireGuard özel anahtarı (yalnız sahte; gerçek anahtar değil).
pub const DEMO_WG_KEY: &str = "bnV4dXQtd2lyZWd1YXJkLXRlc3QtcHJpdmF0ZS1rMDE=";

pub type Settings = HashMap<String, HashMap<String, OwnedValue>>;

/// Gizli bilgi anahtarları (grup, anahtar): `GetSettings` bunları vermez.
const SECRET_KEYS: &[(&str, &str)] = &[
    ("802-11-wireless-security", "psk"),
    ("802-11-wireless-security", "wep-key0"),
    ("802-11-wireless-security", "wep-key1"),
    ("802-11-wireless-security", "wep-key2"),
    ("802-11-wireless-security", "wep-key3"),
    ("wireguard", "private-key"),
];

/// Sahteye gelen çağrıların kaydı. `echo`: her satırı stdout'a da yazar (örnek program).
#[derive(Clone, Default)]
pub struct Log {
    entries: Arc<Mutex<Vec<String>>>,
    echo: bool,
}

impl Log {
    pub fn echo() -> Self {
        Self { entries: Arc::default(), echo: true }
    }
    pub fn push(&self, s: String) {
        if self.echo {
            println!("fake_nm: {s}");
        }
        self.entries.lock().unwrap().push(s);
    }
    pub fn has(&self, s: &str) -> bool {
        self.entries.lock().unwrap().iter().any(|e| e == s)
    }
    pub fn count(&self, prefix: &str) -> usize {
        self.entries.lock().unwrap().iter().filter(|e| e.starts_with(prefix)).count()
    }
}

pub type AgentSlot = Arc<Mutex<Option<String>>>;

pub fn path(p: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p).unwrap()
}

pub fn ov(v: Value<'_>) -> OwnedValue {
    OwnedValue::try_from(v).unwrap()
}

fn get_str(s: &Settings, group: &str, key: &str) -> Option<String> {
    s.get(group)?.get(key)?.try_clone().ok()?.try_into().ok()
}

/// Ortak durum: kayıtlı bağlantıların tam ayarları, ajan, yeni yol sayacı.
#[derive(Clone)]
pub struct Shared {
    pub stored: Arc<Mutex<BTreeMap<String, Settings>>>,
    pub agent: AgentSlot,
    pub log: Log,
    next: Arc<AtomicU32>,
    /// Kayıtlı bağlantı → etkin bağlantı nesnesi.
    pub active: Arc<Mutex<BTreeMap<String, String>>>,
    /// Doğruysa `Update`/`Delete` polkit reddi döner (`…Settings.Connection.PermissionDenied`).
    pub deny: Arc<AtomicBool>,
}

impl Shared {
    fn new_path(&self, kind: &str) -> String {
        format!("{NM_PATH}/{kind}/{}", self.next.fetch_add(1, Ordering::Relaxed))
    }

    /// Bağlantıyı etkin yapar: varsa mevcut etkin nesne, yoksa yeni `Connection.Active`
    /// (`InterfacesAdded`; durum `Activated`).
    async fn activate(&self, server: &zbus::ObjectServer, connection: &str) -> fdo::Result<String> {
        if let Some(a) = self.active.lock().unwrap().get(connection) {
            return Ok(a.clone());
        }
        let settings = self.stored.lock().unwrap().get(connection).cloned_settings().unwrap_or_default();
        let id = get_str(&settings, "connection", "id").unwrap_or_default();
        let kind = get_str(&settings, "connection", "type").unwrap_or_default();
        let p = self.new_path("ActiveConnection");
        server
            .at(p.as_str(), Active { id, kind, connection: connection.to_owned() })
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        self.active.lock().unwrap().insert(connection.to_owned(), p.clone());
        self.log.push(format!("Activated {connection} {p}"));
        Ok(p)
    }

    /// Yeni kayıtlı bağlantı nesnesi (yol önceden ayrılmış).
    async fn add_saved(&self, server: &zbus::ObjectServer, path: &str, settings: Settings) -> fdo::Result<()> {
        self.stored.lock().unwrap().insert(path.to_owned(), settings);
        server
            .at(path, SavedConnection { path: path.to_owned(), shared: self.clone() })
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }
}

/// "Ev" ağının doğru şifresi (sahte): kayıtlı şifre bundan farklıysa etkinleştirme
/// `FAILED` / `NO_SECRETS` (7) ile biter. Diğer ağlar her şifreyi kabul eder.
pub const EV_PASSWORD: &str = "ev-sifre-123";

struct Manager {
    wifi: bool,
    /// Birincil etkin bağlantı (`/`: yok).
    primary: String,
    shared: Shared,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    #[zbus(property)]
    fn state(&self) -> u32 {
        if self.primary == "/" { 20 } else { 70 }
    }
    #[zbus(property)]
    fn connectivity(&self) -> u32 {
        if self.primary == "/" { 1 } else { 4 }
    }
    #[zbus(property)]
    fn networking_enabled(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn wireless_enabled(&self) -> bool {
        self.wifi
    }
    #[zbus(property)]
    fn set_wireless_enabled(&mut self, v: bool) {
        self.shared.log.push(format!("WirelessEnabled {v}"));
        self.wifi = v;
    }
    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn primary_connection(&self) -> OwnedObjectPath {
        path(&self.primary)
    }
    #[zbus(property)]
    fn primary_connection_type(&self) -> String {
        if self.primary == "/" { String::new() } else { "802-11-wireless".into() }
    }

    /// Gerçek NM gibi: kayıtlı Wi-Fi bağlantısının şifresi yoksa ajana sorar; ajan yoksa
    /// cihaz `FAILED` (120) / `NO_SECRETS` (7) olur. "Ev" için kayıtlı şifre
    /// [`EV_PASSWORD`] değilse aynı hata. Başarıda bağlantı gerçekten etkinleşir (etkin nesne).
    async fn activate_connection(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        connection: ObjectPath<'_>,
        device: ObjectPath<'_>,
        specific: ObjectPath<'_>,
    ) -> fdo::Result<OwnedObjectPath> {
        self.shared.log.push(format!("Activate {connection} {device} {specific}"));
        let settings = self.shared.stored.lock().unwrap().get(connection.as_str()).cloned_settings();
        let Some(settings) = settings else { return Err(fdo::Error::UnknownObject("bağlantı yok".into())) };
        let fail = || async {
            conn.emit_signal(None::<()>, DEV, "org.freedesktop.NetworkManager.Device", "StateChanged", &(120u32, 50u32, 7u32))
                .await
        };
        let wifi_psk = get_str(&settings, "802-11-wireless-security", "key-mgmt").as_deref() == Some("wpa-psk");
        let stored_psk = get_str(&settings, "802-11-wireless-security", "psk");
        if wifi_psk
            && let Some(psk) = &stored_psk
            && get_str(&settings, "connection", "id").as_deref() == Some("Ev")
            && psk != EV_PASSWORD
        {
            self.shared.log.push(format!("WrongPassword {connection}"));
            fail().await?;
            return Ok(path("/"));
        }
        if wifi_psk && stored_psk.is_none() {
            let agent = self.shared.agent.lock().unwrap().clone();
            let Some(agent) = agent else {
                fail().await?;
                return Ok(path("/"));
            };
            let request: HashMap<&str, HashMap<&str, Value<'_>>> = HashMap::from([
                ("connection", HashMap::from([("id", Value::from("Ev")), ("type", Value::from("802-11-wireless"))])),
                ("802-11-wireless", HashMap::from([("ssid", Value::from(b"Ev".to_vec()))])),
                ("802-11-wireless-security", HashMap::from([("key-mgmt", Value::from("wpa-psk"))])),
            ]);
            let reply = conn
                .call_method(
                    Some(agent.as_str()),
                    SECRET_AGENT_PATH,
                    Some("org.freedesktop.NetworkManager.SecretAgent"),
                    "GetSecrets",
                    &(request, &connection, "802-11-wireless-security", Vec::<String>::new(), 1u32),
                )
                .await
                .map_err(|e| fdo::Error::Failed(format!("secrets: {e}")))?;
            let secrets: Settings = reply.body().deserialize().map_err(|e| fdo::Error::Failed(e.to_string()))?;
            let psk: String = secrets["802-11-wireless-security"]["psk"].try_clone().unwrap().try_into().unwrap();
            self.shared.log.push(format!("secret {psk}"));
        }
        let active = self.shared.activate(server, connection.as_str()).await?;
        Ok(path(&active))
    }

    async fn add_and_activate_connection(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        settings: Settings,
        device: ObjectPath<'_>,
        specific: ObjectPath<'_>,
    ) -> fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        let ssid: Vec<u8> = settings["802-11-wireless"]["ssid"].try_clone().unwrap().try_into().unwrap();
        let ssid = String::from_utf8_lossy(&ssid).into_owned();
        let km = get_str(&settings, "802-11-wireless-security", "key-mgmt").unwrap_or_else(|| "-".into());
        let psk = get_str(&settings, "802-11-wireless-security", "psk").unwrap_or_else(|| "-".into());
        self.shared.log.push(format!("AddAndActivate {ssid} {km} {psk} {device} {specific}"));
        let hidden = settings["802-11-wireless"].get("hidden").is_some_and(|h| bool::try_from(h).unwrap_or(false));
        if hidden {
            self.shared.log.push(format!("hidden {ssid}"));
        }
        let saved = self.shared.new_path("Settings");
        let mut settings = settings;
        settings.entry("connection".into()).or_default().insert("uuid".into(), ov(Value::from(format!("uuid-{saved}"))));
        self.shared.add_saved(server, &saved, settings).await?;
        let active = self.shared.activate(server, &saved).await?;
        Ok((path(&saved), path(&active)))
    }

    /// Etkin nesneyi kaldırır (cevaptan sonra, ayrı görevde).
    fn deactivate_connection(&self, #[zbus(connection)] conn: &zbus::Connection, active: ObjectPath<'_>) {
        self.shared.log.push(format!("Deactivate {active}"));
        let p = active.to_string();
        self.shared.active.lock().unwrap().retain(|_, a| *a != p);
        let conn2 = conn.clone();
        conn.executor()
            .spawn(
                async move {
                    let _ = conn2.object_server().remove::<Active, _>(p.as_str()).await;
                },
                "fake-nm-deactivate",
            )
            .detach();
    }
}

trait ClonedSettings {
    fn cloned_settings(self) -> Option<Settings>;
}

impl ClonedSettings for Option<&Settings> {
    fn cloned_settings(self) -> Option<Settings> {
        self.map(|s| {
            s.iter()
                .map(|(g, m)| (g.clone(), m.iter().map(|(k, v)| (k.clone(), v.try_clone().unwrap())).collect()))
                .collect()
        })
    }
}

/// `org.freedesktop.NetworkManager.Settings` (`AddConnection`).
struct SettingsRoot {
    shared: Shared,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
impl SettingsRoot {
    async fn add_connection(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        settings: Settings,
    ) -> fdo::Result<OwnedObjectPath> {
        let kind = get_str(&settings, "connection", "type").unwrap_or_default();
        let id = get_str(&settings, "connection", "id").unwrap_or_default();
        self.shared.log.push(format!("AddConnection {kind} {id}"));
        let p = self.shared.new_path("Settings");
        let mut settings = settings;
        settings.entry("connection".into()).or_default().insert("uuid".into(), ov(Value::from(format!("uuid-{p}"))));
        self.shared.add_saved(server, &p, settings).await?;
        Ok(path(&p))
    }
}

struct AgentManager {
    shared: Shared,
    /// kimlik → kaydeden bağlantı (gerçek NM'deki gibi kimlik kullanıcı başına tek).
    ids: HashMap<String, String>,
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.AgentManager")]
enum AgentManagerError {
    #[zbus(error)]
    ZBus(zbus::Error),
    PermissionDenied(String),
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.AgentManager")]
impl AgentManager {
    /// NM kuralı: aynı kimlik, sahibi hâlâ bus'taysa reddedilir (bağlantıdan bağımsız);
    /// sahibi gitmişse kayıt düşmüş sayılır.
    async fn register(
        &mut self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        identifier: String,
    ) -> Result<(), AgentManagerError> {
        let sender = hdr.sender().unwrap().to_string();
        if let Some(owner) = self.ids.get(&identifier) {
            let dbus = fdo::DBusProxy::new(conn).await?;
            let name = zbus::names::BusName::try_from(owner.as_str()).map_err(zbus::Error::from)?;
            if dbus.name_has_owner(name).await.map_err(zbus::Error::from)? {
                self.shared.log.push(format!("RegisterDenied {identifier}"));
                return Err(AgentManagerError::PermissionDenied(
                    "An agent with this ID is already registered for this user.".into(),
                ));
            }
        }
        self.shared.log.push(format!("Register {identifier}"));
        self.ids.insert(identifier, sender.clone());
        *self.shared.agent.lock().unwrap() = Some(sender);
        Ok(())
    }
}

struct Device {
    interface: &'static str,
    kind: u32,
    state: u32,
    hw: &'static str,
    active: &'static str,
    ip4: &'static str,
    ip6: &'static str,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    #[zbus(property)]
    fn interface(&self) -> String {
        self.interface.into()
    }
    #[zbus(property)]
    fn device_type(&self) -> u32 {
        self.kind
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.state
    }
    #[zbus(property)]
    fn managed(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn hw_address(&self) -> String {
        self.hw.into()
    }
    #[zbus(property)]
    fn active_connection(&self) -> OwnedObjectPath {
        path(self.active)
    }
    #[zbus(property)]
    fn ip4_config(&self) -> OwnedObjectPath {
        path(self.ip4)
    }
    #[zbus(property)]
    fn ip6_config(&self) -> OwnedObjectPath {
        path(self.ip6)
    }
}

struct Wireless {
    log: Log,
    aps: Vec<&'static str>,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl Wireless {
    #[zbus(property)]
    fn access_points(&self) -> Vec<OwnedObjectPath> {
        self.aps.iter().map(|p| path(p)).collect()
    }
    #[zbus(property)]
    fn active_access_point(&self) -> OwnedObjectPath {
        path(AP_HOME)
    }
    #[zbus(property)]
    fn bitrate(&self) -> u32 {
        433_000
    }
    fn request_scan(&self, _options: HashMap<String, OwnedValue>) {
        self.log.push("RequestScan".into());
    }
}

struct Wired;

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wired")]
impl Wired {
    #[zbus(property)]
    fn speed(&self) -> u32 {
        1000
    }
}

fn addr_data(addr: &str, prefix: u32) -> HashMap<String, OwnedValue> {
    HashMap::from([("address".into(), ov(Value::from(addr))), ("prefix".into(), ov(Value::U32(prefix)))])
}

struct Ip4Config;

#[zbus::interface(name = "org.freedesktop.NetworkManager.IP4Config")]
impl Ip4Config {
    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![addr_data("192.168.1.20", 24)]
    }
    #[zbus(property)]
    fn gateway(&self) -> String {
        "192.168.1.1".into()
    }
    #[zbus(property)]
    fn nameserver_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![HashMap::from([("address".into(), ov(Value::from("192.168.1.1")))])]
    }
}

struct Ip6Config;

#[zbus::interface(name = "org.freedesktop.NetworkManager.IP6Config")]
impl Ip6Config {
    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![addr_data("fd00::20", 64)]
    }
    #[zbus(property)]
    fn gateway(&self) -> String {
        "fd00::1".into()
    }
    #[zbus(property)]
    fn nameservers(&self) -> Vec<Vec<u8>> {
        vec!["fd00::1".parse::<std::net::Ipv6Addr>().unwrap().octets().to_vec()]
    }
}

struct AccessPoint {
    ssid: Vec<u8>,
    strength: u8,
    flags: u32,
    rsn: u32,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> Vec<u8> {
        self.ssid.clone()
    }
    #[zbus(property)]
    fn strength(&self) -> u8 {
        self.strength
    }
    #[zbus(property)]
    fn flags(&self) -> u32 {
        self.flags
    }
    #[zbus(property)]
    fn wpa_flags(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn rsn_flags(&self) -> u32 {
        self.rsn
    }
    #[zbus(property)]
    fn frequency(&self) -> u32 {
        2412
    }
}

struct Active {
    id: String,
    kind: String,
    connection: String,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl Active {
    #[zbus(property)]
    fn id(&self) -> String {
        self.id.clone()
    }
    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        self.kind.clone()
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        2
    }
    #[zbus(property)]
    fn connection(&self) -> OwnedObjectPath {
        path(&self.connection)
    }
}

pub struct SavedConnection {
    path: String,
    shared: Shared,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl SavedConnection {
    /// Gizli bilgisiz (gerçek NM gibi).
    fn get_settings(&self) -> fdo::Result<Settings> {
        let stored = self.shared.stored.lock().unwrap();
        let mut s = stored.get(&self.path).cloned_settings().ok_or_else(|| fdo::Error::UnknownObject("yok".into()))?;
        for (group, key) in SECRET_KEYS {
            if let Some(g) = s.get_mut(*group) {
                g.remove(*key);
            }
        }
        Ok(s)
    }

    /// Tüm ayarları değiştirir. Gerçek NM gibi (nm-settings-connection.c `update_auth_cb`):
    /// gelen ayarda **hiç** gizli bilgi yoksa kayıtlı gizli bilgilerin tümü yeni ayara katılır;
    /// en az bir gizli bilgi varsa birleştirme yok (gelen ayar aynen).
    async fn update(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        settings: Settings,
    ) -> Result<(), ConnError> {
        if self.denied().is_err() {
            self.shared.log.push(format!("UpdateDenied {}", self.path));
            return self.denied();
        }
        let incoming = SECRET_KEYS.iter().filter(|(g, k)| settings.get(*g).is_some_and(|m| m.contains_key(*k))).count();
        self.shared.log.push(format!("Update {} secrets={incoming}", self.path));
        {
            let mut stored = self.shared.stored.lock().unwrap();
            let old = stored.get(&self.path).cloned_settings().unwrap_or_default();
            let mut new = settings;
            for (group, key) in SECRET_KEYS.iter().filter(|_| incoming == 0) {
                if let Some(v) = old.get(*group).and_then(|g| g.get(*key)) {
                    new.entry((*group).to_owned()).or_default().insert((*key).to_owned(), v.try_clone().unwrap());
                }
            }
            stored.insert(self.path.clone(), new);
        }
        Self::updated(&emitter).await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn updated(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    /// Nesneyi kaldırır (cevaptan sonra, ayrı görevde: kendi çağrısı sürerken kaldırılmaz).
    fn delete(&self, #[zbus(connection)] conn: &zbus::Connection) -> Result<(), ConnError> {
        self.shared.log.push(format!("Delete {}", self.path));
        self.denied()?;
        self.shared.stored.lock().unwrap().remove(&self.path);
        let conn2 = conn.clone();
        let p = self.path.clone();
        conn.executor()
            .spawn(
                async move {
                    let _ = conn2.object_server().remove::<SavedConnection, _>(p.as_str()).await;
                },
                "fake-nm-delete",
            )
            .detach();
        Ok(())
    }
}

fn wifi_settings(id: &str, ssid: &[u8], psk: Option<&str>) -> Settings {
    let mut s: Settings = HashMap::from([
        (
            "connection".into(),
            HashMap::from([
                ("id".into(), ov(Value::from(id))),
                ("uuid".into(), ov(Value::from(format!("uuid-{id}")))),
                ("type".into(), ov(Value::from("802-11-wireless"))),
            ]),
        ),
        ("802-11-wireless".into(), HashMap::from([("ssid".into(), ov(Value::from(ssid.to_vec())))])),
    ]);
    if let Some(psk) = psk {
        s.insert(
            "802-11-wireless-security".into(),
            HashMap::from([("key-mgmt".into(), ov(Value::from("wpa-psk"))), ("psk".into(), ov(Value::from(psk)))]),
        );
    } else {
        s.insert("802-11-wireless-security".into(), HashMap::from([("key-mgmt".into(), ov(Value::from("wpa-psk")))]));
    }
    s
}

/// Basit bağlantı ayarları (ad, tür).
pub fn plain_settings(id: &str, kind: &str) -> Settings {
    HashMap::from([(
        "connection".into(),
        HashMap::from([
            ("id".into(), ov(Value::from(id))),
            ("uuid".into(), ov(Value::from(format!("uuid-{id}")))),
            ("type".into(), ov(Value::from(kind))),
        ]),
    )])
}

/// Sahte NM sahnesi (`examples/fake_nm`: `NUXUT_FAKE_NM_SCENE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    /// Testlerin temel ağacı: Wi-Fi cihazı, Ev (kayıtlı, etkin, şifresi NM'de yok) + Kafe.
    Basic,
    /// `default`: Basic + açık/gizli/WPA3 erişim noktaları, Ofis, İş VPN, ethernet.
    Demo,
    /// `no-device`: Wi-Fi cihazı yok (erişim noktası, kayıtlı/etkin bağlantı yok).
    NoDevice,
    /// `empty`: Wi-Fi cihazı var, açık; erişim noktası ve kayıtlı bağlantı yok.
    Empty,
    /// `wifi-off`: Demo, ama Wi-Fi kapalı (erişim noktası görünmez, Wi-Fi etkin değil).
    WifiOff,
}

impl Scene {
    /// `NUXUT_FAKE_NM_SCENE` değeri; bilinmeyen değer `None`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "" | "default" => Self::Demo,
            "basic" => Self::Basic,
            "no-device" => Self::NoDevice,
            "empty" => Self::Empty,
            "wifi-off" => Self::WifiOff,
            _ => return None,
        })
    }
}

pub struct FakeNm {
    pub conn: zbus::blocking::Connection,
    pub shared: Shared,
}

impl FakeNm {
    /// `demo`: [`Scene::Demo`], değilse [`Scene::Basic`].
    pub fn start(conn: zbus::blocking::Connection, log: &Log, demo: bool) -> Self {
        Self::start_scene(conn, log, if demo { Scene::Demo } else { Scene::Basic })
    }

    pub fn start_scene(conn: zbus::blocking::Connection, log: &Log, scene: Scene) -> Self {
        let shared = Shared {
            stored: Arc::default(),
            agent: AgentSlot::default(),
            log: log.clone(),
            next: Arc::new(AtomicU32::new(10)),
            active: Arc::default(),
            deny: Arc::default(),
        };
        let fake = Self { conn, shared: shared.clone() };
        let demo = matches!(scene, Scene::Demo | Scene::WifiOff);
        let wifi_device = scene != Scene::NoDevice;
        let aps_visible = matches!(scene, Scene::Basic | Scene::Demo);
        let ev_active = aps_visible;
        let mut aps = Vec::new();
        if aps_visible {
            aps.extend([AP_HOME, AP_CAFE]);
            if demo {
                aps.extend([AP_GUEST, AP_HIDDEN, AP_SAE]);
            }
        }
        {
            let s = fake.conn.object_server();
            s.at("/org/freedesktop", fdo::ObjectManager).unwrap();
            let primary = if ev_active { ACTIVE } else if demo { ACTIVE_WIRED } else { "/" };
            let manager = Manager { wifi: scene != Scene::WifiOff, primary: primary.into(), shared: shared.clone() };
            s.at(NM_PATH, manager).unwrap();
            s.at(format!("{NM_PATH}/Settings"), SettingsRoot { shared: shared.clone() }).unwrap();
            s.at(format!("{NM_PATH}/AgentManager"), AgentManager { shared: shared.clone(), ids: HashMap::new() }).unwrap();
            if wifi_device {
                let active = if ev_active { ACTIVE } else { "/" };
                let wifi = Device { interface: "wlan0", kind: 2, state: 100, hw: "02:00:00:00:00:01", active, ip4: "/", ip6: "/" };
                s.at(DEV, wifi).unwrap();
                s.at(DEV, Wireless { log: log.clone(), aps: aps.clone() }).unwrap();
            }
            if aps_visible {
                s.at(AP_HOME, AccessPoint { ssid: b"Ev".to_vec(), strength: 70, flags: 1, rsn: 0x100 }).unwrap();
                s.at(AP_CAFE, AccessPoint { ssid: b"Kafe".to_vec(), strength: 40, flags: 1, rsn: 0x100 }).unwrap();
                if demo {
                    s.at(AP_GUEST, AccessPoint { ssid: b"Misafir".to_vec(), strength: 55, flags: 0, rsn: 0 }).unwrap();
                    s.at(AP_HIDDEN, AccessPoint { ssid: Vec::new(), strength: 60, flags: 1, rsn: 0x100 }).unwrap();
                    s.at(AP_SAE, AccessPoint { ssid: b"Komsu".to_vec(), strength: 30, flags: 1, rsn: 0x400 }).unwrap();
                }
            }
            if ev_active {
                let a = Active { id: "Ev".into(), kind: "802-11-wireless".into(), connection: SAVED_HOME.into() };
                s.at(ACTIVE, a).unwrap();
                shared.active.lock().unwrap().insert(SAVED_HOME.into(), ACTIVE.into());
            }
            if demo {
                let eth = Device {
                    interface: "enp1s0",
                    kind: 1,
                    state: 100,
                    hw: "52:54:00:12:34:56",
                    active: ACTIVE_WIRED,
                    ip4: IP4,
                    ip6: IP6,
                };
                s.at(DEV_ETH, eth).unwrap();
                s.at(DEV_ETH, Wired).unwrap();
                s.at(IP4, Ip4Config).unwrap();
                s.at(IP6, Ip6Config).unwrap();
                let a = Active { id: "Kablolu".into(), kind: "802-3-ethernet".into(), connection: SAVED_WIRED.into() };
                s.at(ACTIVE_WIRED, a).unwrap();
                shared.active.lock().unwrap().insert(SAVED_WIRED.into(), ACTIVE_WIRED.into());
            }
        }
        if matches!(scene, Scene::Basic | Scene::Demo | Scene::WifiOff) {
            fake.add_saved(SAVED_HOME, wifi_settings("Ev", b"Ev", None));
        }
        if demo {
            fake.add_saved(SAVED_OFFICE, wifi_settings("Ofis", b"Ofis", Some("ofis-sifre")));
            let mut vpn = plain_settings("İş VPN", "wireguard");
            vpn.insert("wireguard".into(), HashMap::from([("private-key".into(), ov(Value::from(DEMO_WG_KEY)))]));
            fake.add_saved(SAVED_VPN, vpn);
            fake.add_saved(SAVED_WIRED, plain_settings("Kablolu", "802-3-ethernet"));
            fake.add_saved(SAVED_AGENT_OWNED, flagged_wifi("Ajanli", 1));
            fake.add_saved(SAVED_NOT_SAVED, flagged_wifi("Sormali", 2));
            fake.add_saved(SAVED_NOT_REQUIRED, flagged_wifi("Gereksiz", 4));
            fake.add_saved(SAVED_WEP, wep_settings("EskiWep"));
        }
        fake.conn.request_name(NM).unwrap();
        fake
    }

    /// Kayıtlı bağlantı ekler (tam ayarlarla).
    pub fn add_saved(&self, p: &str, settings: Settings) {
        self.shared.stored.lock().unwrap().insert(p.to_owned(), settings);
        self.conn.object_server().at(p, SavedConnection { path: p.to_owned(), shared: self.shared.clone() }).unwrap();
    }

    /// Kayıtlı bağlantının NM'de saklanan tam ayarları (gizli bilgiler dahil).
    pub fn stored(&self, p: &str) -> Option<Settings> {
        self.shared.stored.lock().unwrap().get(p).cloned_settings()
    }

    pub fn stored_str(&self, p: &str, group: &str, key: &str) -> Option<String> {
        get_str(&self.stored(p)?, group, key)
    }

    /// Kayıtlı bağlantı yolu (bağlantı adına göre).
    pub fn find(&self, id: &str) -> Option<String> {
        let stored = self.shared.stored.lock().unwrap();
        stored.iter().find(|(_, s)| get_str(s, "connection", "id").as_deref() == Some(id)).map(|(p, _)| p.clone())
    }

    pub fn agent(&self) -> Option<String> {
        self.shared.agent.lock().unwrap().clone()
    }
}

/// `org.freedesktop.NetworkManager.Settings.Connection` hataları (sahte polkit reddi).
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.NetworkManager.Settings.Connection")]
pub enum ConnError {
    #[zbus(error)]
    ZBus(zbus::Error),
    PermissionDenied(String),
}

impl SavedConnection {
    fn denied(&self) -> Result<(), ConnError> {
        if self.shared.deny.load(Ordering::Relaxed) {
            return Err(ConnError::PermissionDenied("Insufficient privileges".into()));
        }
        Ok(())
    }
}

impl FakeNm {
    /// polkit reddini aç/kapa (`Delete` → `…PermissionDenied`).
    pub fn set_deny(&self, on: bool) {
        self.shared.deny.store(on, Ordering::Relaxed);
    }
}

/// `psk-flags` ayarlı WPA-PSK Wi-Fi (şifre kayıtlı değil; GetSettings bayrağı verir).
pub fn flagged_wifi(id: &str, flags: u32) -> Settings {
    let mut s = wifi_settings(id, id.as_bytes(), None);
    s.get_mut("802-11-wireless-security").unwrap().insert("psk-flags".into(), ov(Value::U32(flags)));
    s
}

/// WEP bağlantısı: dört anahtar kayıtlı.
pub fn wep_settings(id: &str) -> Settings {
    let mut s = wifi_settings(id, id.as_bytes(), None);
    let sec = s.get_mut("802-11-wireless-security").unwrap();
    sec.insert("key-mgmt".into(), ov(Value::from("none")));
    for (i, k) in ["wep-key0", "wep-key1", "wep-key2", "wep-key3"].iter().enumerate() {
        sec.insert((*k).into(), ov(Value::from(format!("wepanahtar{i}"))));
    }
    sec.insert("wep-key-type".into(), ov(Value::U32(2)));
    s
}
