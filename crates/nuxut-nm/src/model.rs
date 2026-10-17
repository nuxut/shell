//! Saf NetworkManager modeli: ObjectManager ağacı + kayıtlı bağlantı ayarlarından türetilen
//! durum (bağlantı durumu, birincil tür, Wi-Fi, erişim noktaları, kayıtlı bağlantılar, VPN).

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv6Addr};

use nuxut_dbus::Unavailable;
use nuxut_dbus::{
    ObjectStore, PropMap, clean_str, prop_bool, prop_bytes, prop_str, prop_u32, value_bytes, value_i64, value_str,
};
use zbus::zvariant::Value;

use crate::wireguard::IpNet;

pub(crate) const NM_IFACE: &str = "org.freedesktop.NetworkManager";
pub(crate) const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
pub(crate) const WIRELESS_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
pub(crate) const AP_IFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";
pub(crate) const ACTIVE_IFACE: &str = "org.freedesktop.NetworkManager.Connection.Active";
pub(crate) const SETTINGS_CONN_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
pub(crate) const NM_PATH: &str = "/org/freedesktop/NetworkManager";

/// En fazla erişim noktası (kalabalık ortamlarda yüzlerce olabilir).
pub const MAX_ACCESS_POINTS: usize = 256;
pub const MAX_DEVICES: usize = 64;
pub const MAX_CONNECTIONS: usize = 256;
/// SSID en fazla 32 bayttır (802.11).
pub const MAX_SSID: usize = 32;
const MAX_NAME: usize = 256;

pub(crate) fn new_objects() -> ObjectStore {
    ObjectStore::new(NM_PATH, |iface| match iface {
        NM_IFACE => Some(1),
        DEVICE_IFACE | WIRELESS_IFACE => Some(MAX_DEVICES),
        AP_IFACE => Some(MAX_ACCESS_POINTS),
        ACTIVE_IFACE => Some(MAX_CONNECTIONS),
        SETTINGS_CONN_IFACE => Some(MAX_CONNECTIONS),
        _ => None,
    })
}

/// NM genel durumu (`NMState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NmState {
    #[default]
    Unknown,
    Asleep,
    Disconnected,
    Disconnecting,
    Connecting,
    /// Yalnızca yerel ağ.
    ConnectedLocal,
    /// Site (internet yok).
    ConnectedSite,
    /// İnternet.
    ConnectedGlobal,
}

impl NmState {
    fn from_nm(n: u32) -> Self {
        match n {
            10 => Self::Asleep,
            20 => Self::Disconnected,
            30 => Self::Disconnecting,
            40 => Self::Connecting,
            50 => Self::ConnectedLocal,
            60 => Self::ConnectedSite,
            70 => Self::ConnectedGlobal,
            _ => Self::Unknown,
        }
    }

    pub fn is_connected(self) -> bool {
        matches!(self, Self::ConnectedLocal | Self::ConnectedSite | Self::ConnectedGlobal)
    }
}

/// `NMConnectivityState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Connectivity {
    #[default]
    Unknown,
    None,
    /// Esir portal (giriş sayfası).
    Portal,
    Limited,
    Full,
}

impl Connectivity {
    fn from_nm(n: u32) -> Self {
        match n {
            1 => Self::None,
            2 => Self::Portal,
            3 => Self::Limited,
            4 => Self::Full,
            _ => Self::Unknown,
        }
    }
}

/// Bağlantı türü (NM `connection.type`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ConnectionKind {
    Wifi,
    Ethernet,
    Vpn,
    WireGuard,
    Bluetooth,
    Mobile,
    #[default]
    Other,
}

impl ConnectionKind {
    pub fn parse(s: &str) -> Self {
        match s {
            "802-11-wireless" => Self::Wifi,
            "802-3-ethernet" => Self::Ethernet,
            "vpn" => Self::Vpn,
            "wireguard" => Self::WireGuard,
            "bluetooth" => Self::Bluetooth,
            "gsm" | "cdma" => Self::Mobile,
            _ => Self::Other,
        }
    }

    pub fn is_vpn(&self) -> bool {
        matches!(self, Self::Vpn | Self::WireGuard)
    }
}

/// Erişim noktasının güvenliği.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Security {
    #[default]
    Open,
    /// OWE (fırsatçı şifreleme): şifresiz ama şifreli.
    Owe,
    Wep,
    /// WPA/WPA2 kişisel (PSK).
    WpaPsk,
    /// WPA3 kişisel (SAE).
    Sae,
    /// 802.1X: kullanıcı adı/sertifika — shell'den bağlanılmaz, ayarlara yönlendirilir.
    Enterprise,
}

impl Security {
    /// Bağlanmak için şifre gerekir mi?
    pub fn needs_password(self) -> bool {
        matches!(self, Self::Wep | Self::WpaPsk | Self::Sae)
    }

    /// NM `802-11-wireless-security.key-mgmt`.
    pub(crate) fn key_mgmt(self) -> Option<&'static str> {
        match self {
            Self::Open => None,
            Self::Owe => Some("owe"),
            Self::Wep => Some("none"),
            Self::WpaPsk => Some("wpa-psk"),
            Self::Sae => Some("sae"),
            Self::Enterprise => Some("wpa-eap"),
        }
    }
}

/// NM `Flags`/`WpaFlags`/`RsnFlags` → güvenlik.
pub fn security_from_flags(flags: u32, wpa: u32, rsn: u32) -> Security {
    const PRIVACY: u32 = 0x1;
    const KEY_MGMT_PSK: u32 = 0x100;
    const KEY_MGMT_802_1X: u32 = 0x200;
    const KEY_MGMT_SAE: u32 = 0x400;
    const KEY_MGMT_OWE: u32 = 0x800;
    const KEY_MGMT_OWE_TM: u32 = 0x1000;
    const KEY_MGMT_EAP_SUITE_B_192: u32 = 0x2000;
    let all = wpa | rsn;
    if all & (KEY_MGMT_802_1X | KEY_MGMT_EAP_SUITE_B_192) != 0 {
        Security::Enterprise
    } else if all & KEY_MGMT_PSK != 0 {
        // WPA2/WPA3 geçiş modu PSK ile de bağlanır.
        Security::WpaPsk
    } else if rsn & KEY_MGMT_SAE != 0 {
        Security::Sae
    } else if rsn & (KEY_MGMT_OWE | KEY_MGMT_OWE_TM) != 0 {
        Security::Owe
    } else if flags & PRIVACY != 0 {
        Security::Wep
    } else {
        Security::Open
    }
}

/// SSID baytlarını gösterilebilir metne çevirir (geçersiz UTF-8 → `�`, kontrol karakterleri
/// atılır). Bağlanırken her zaman ham baytlar kullanılır.
pub fn ssid_display(bytes: &[u8]) -> String {
    clean_str(&String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_SSID)]), MAX_SSID * 3)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPoint {
    /// En güçlü BSS'in yolu (bağlanırken `specific_object`).
    pub path: String,
    /// Gösterilecek ad.
    pub ssid: String,
    /// Ham SSID (en fazla 32 bayt).
    pub ssid_bytes: Vec<u8>,
    /// 0..=100.
    pub strength: u8,
    pub security: Security,
    /// MHz.
    pub frequency: u32,
    /// Şu an bağlı olunan ağ mı?
    pub active: bool,
    /// Bu SSID için kayıtlı bağlantı var mı?
    pub saved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiDevice {
    pub path: String,
    pub interface: String,
    /// Bağlı erişim noktasının yolu.
    pub active_ap: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedConnection {
    pub path: String,
    pub id: String,
    pub uuid: String,
    pub kind: ConnectionKind,
    /// Wi-Fi bağlantısının SSID'si.
    pub ssid: Option<Vec<u8>>,
    /// `connection.autoconnect` (NM varsayılanı açık).
    pub autoconnect: bool,
    /// `connection.metered`.
    pub metered: Metered,
    /// Etkin bağlantının yolu (etkinse).
    pub active: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveState {
    #[default]
    Unknown,
    Activating,
    Activated,
    Deactivating,
    Deactivated,
}

impl ActiveState {
    fn from_nm(n: u32) -> Self {
        match n {
            1 => Self::Activating,
            2 => Self::Activated,
            3 => Self::Deactivating,
            4 => Self::Deactivated,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveConnection {
    pub path: String,
    pub id: String,
    pub kind: ConnectionKind,
    pub state: ActiveState,
    /// Kayıtlı bağlantının yolu.
    pub connection: String,
}

/// `connection.metered` (`NMMetered`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Metered {
    /// NM tahmin eder (varsayılan).
    #[default]
    Unknown,
    Yes,
    No,
    /// NM tahmini (yalnız okunur; yazılırken anlamı yok).
    GuessYes,
    GuessNo,
}

impl Metered {
    pub(crate) fn from_nm(n: i64) -> Self {
        match n {
            1 => Self::Yes,
            2 => Self::No,
            3 => Self::GuessYes,
            4 => Self::GuessNo,
            _ => Self::Unknown,
        }
    }

    pub(crate) fn to_nm(self) -> i32 {
        match self {
            Self::Unknown => 0,
            Self::Yes => 1,
            Self::No => 2,
            Self::GuessYes => 3,
            Self::GuessNo => 4,
        }
    }

    /// Ölçülü sayılıyor mu (kesin ya da tahmin)?
    pub fn is_metered(self) -> bool {
        matches!(self, Self::Yes | Self::GuessYes)
    }
}

/// Ağ cihazı türü (`NMDeviceType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DeviceKind {
    Ethernet,
    Wifi,
    WireGuard,
    Bluetooth,
    Mobile,
    #[default]
    Other,
}

impl DeviceKind {
    fn from_nm(n: u32) -> Self {
        match n {
            1 => Self::Ethernet,
            2 => Self::Wifi,
            5 => Self::Bluetooth,
            8 => Self::Mobile,
            29 => Self::WireGuard,
            _ => Self::Other,
        }
    }
}

/// Ağ cihazı (döngü cihazı hariç).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetDevice {
    pub path: String,
    pub interface: String,
    pub kind: DeviceKind,
    /// NM yönetiyor mu?
    pub managed: bool,
    /// Etkin (`NM_DEVICE_STATE_ACTIVATED`).
    pub connected: bool,
    /// Etkin bağlantının yolu.
    pub active_connection: Option<String>,
}

/// Bir cihazın ayrıntısı ([`crate::NetworkService::device_details`]; salt okunur).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceDetails {
    pub path: String,
    pub interface: String,
    pub kind: DeviceKind,
    /// Donanım (MAC) adresi.
    pub hw_address: Option<String>,
    /// Bağlantı hızı, Mb/s (kablolu `Speed`; Wi-Fi `Bitrate`).
    pub speed_mbps: Option<u32>,
    pub ipv4: Vec<IpNet>,
    pub ipv4_gateway: Option<IpAddr>,
    pub ipv6: Vec<IpNet>,
    pub ipv6_gateway: Option<IpAddr>,
    /// DNS sunucuları (önce IPv4).
    pub dns: Vec<IpAddr>,
}

/// Ağ durumunun tamamı.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkState {
    /// NetworkManager çalışıyor mu?
    pub available: bool,
    /// `available == false` iken nedeni: bus'a ulaşılamıyor mu, NM mi çalışmıyor? `None`:
    /// kullanılabilir ya da henüz okunmadı (`Default`).
    pub unavailable: Option<Unavailable>,
    pub state: NmState,
    pub connectivity: Connectivity,
    /// Birincil (varsayılan yol) bağlantının türü ve adı.
    pub primary_kind: Option<ConnectionKind>,
    pub primary_name: Option<String>,
    pub networking_enabled: bool,
    pub wifi_enabled: bool,
    /// Donanım (rfkill) anahtarı açık mı?
    pub wifi_hardware_enabled: bool,
    pub wifi_devices: Vec<WifiDevice>,
    /// Tüm ağ cihazları (döngü hariç), yol sırasıyla.
    pub devices: Vec<NetDevice>,
    /// SSID başına en güçlü BSS; etkin önce, sonra sinyale göre. Gizli ağlar (boş SSID) yok.
    pub access_points: Vec<AccessPoint>,
    /// Ada göre.
    pub saved: Vec<SavedConnection>,
    pub active: Vec<ActiveConnection>,
}

impl NetworkState {
    pub fn vpns(&self) -> impl Iterator<Item = &SavedConnection> {
        self.saved.iter().filter(|c| c.kind.is_vpn())
    }

    /// Wi-Fi ile birincil bağlıysa etkin erişim noktası.
    pub fn active_access_point(&self) -> Option<&AccessPoint> {
        self.access_points.iter().find(|a| a.active)
    }
}

/// `GetSettings` çıktısından bir kayıtlı bağlantının özeti.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingsSummary {
    pub id: String,
    pub uuid: String,
    pub kind: ConnectionKind,
    pub ssid: Option<Vec<u8>>,
    pub autoconnect: bool,
    pub metered: Metered,
}

pub(crate) type Settings = HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>;

impl SettingsSummary {
    pub fn parse(settings: &Settings) -> Option<Self> {
        let c = settings.get("connection")?;
        let id = prop_str(c, "id", MAX_NAME)?;
        let uuid = prop_str(c, "uuid", 64)?;
        let kind = ConnectionKind::parse(&prop_str(c, "type", 64).unwrap_or_default());
        let ssid = settings.get("802-11-wireless").and_then(|w| prop_bytes(w, "ssid", MAX_SSID));
        let autoconnect = c.get("autoconnect").and_then(|v| nuxut_dbus::value_bool(v)).unwrap_or(true);
        let metered = Metered::from_nm(c.get("metered").and_then(|v| nuxut_dbus::value_i64(v)).unwrap_or(0));
        Some(Self { id, uuid, kind, ssid, autoconnect, metered })
    }
}

/// Bağlantı ayarlarından `key-mgmt` (gizli bilgi isteğinde hangi anahtarın döneceği için).
pub(crate) fn key_mgmt(settings: &Settings) -> Option<String> {
    settings.get("802-11-wireless-security").and_then(|s| prop_str(s, "key-mgmt", 32))
}

/// `NM_DEVICE_TYPE_LOOPBACK`.
const LOOPBACK: u32 = 32;
/// `NM_DEVICE_STATE_ACTIVATED`.
pub(crate) const DEVICE_ACTIVATED: u32 = 100;

/// Ağaç + ayarlar → durum.
pub(crate) fn state(
    objects: &ObjectStore,
    settings: &BTreeMap<String, SettingsSummary>,
    available: bool,
) -> NetworkState {
    let mut s = NetworkState {
        available,
        unavailable: (!available).then_some(Unavailable::ServiceNotRunning),
        ..Default::default()
    };
    let active: Vec<ActiveConnection> = objects
        .with(ACTIVE_IFACE)
        .map(|(path, p)| ActiveConnection {
            path: path.to_owned(),
            id: prop_str(p, "Id", MAX_NAME).unwrap_or_default(),
            kind: ConnectionKind::parse(&prop_str(p, "Type", 64).unwrap_or_default()),
            state: ActiveState::from_nm(prop_u32(p, "State").unwrap_or(0)),
            connection: prop_str(p, "Connection", 256).unwrap_or_default(),
        })
        .collect();
    if let Some(m) = objects.get(NM_PATH, NM_IFACE) {
        s.state = NmState::from_nm(prop_u32(m, "State").unwrap_or(0));
        s.connectivity = Connectivity::from_nm(prop_u32(m, "Connectivity").unwrap_or(0));
        s.networking_enabled = prop_bool(m, "NetworkingEnabled").unwrap_or(false);
        s.wifi_enabled = prop_bool(m, "WirelessEnabled").unwrap_or(false);
        s.wifi_hardware_enabled = prop_bool(m, "WirelessHardwareEnabled").unwrap_or(false);
        let primary = prop_str(m, "PrimaryConnection", 256).filter(|p| p != "/");
        if let Some(a) = primary.and_then(|p| active.iter().find(|a| a.path == p)) {
            s.primary_kind = Some(a.kind.clone());
            s.primary_name = Some(a.id.clone());
        } else if let Some(t) = prop_str(m, "PrimaryConnectionType", 64).filter(|t| !t.is_empty()) {
            s.primary_kind = Some(ConnectionKind::parse(&t));
        }
    }
    for (path, w) in objects.with(WIRELESS_IFACE) {
        let interface = objects.get(path, DEVICE_IFACE).and_then(|d| prop_str(d, "Interface", 32)).unwrap_or_default();
        let active_ap = prop_str(w, "ActiveAccessPoint", 256).filter(|p| p != "/");
        s.wifi_devices.push(WifiDevice { path: path.to_owned(), interface, active_ap });
    }
    for (path, d) in objects.with(DEVICE_IFACE) {
        let kind = prop_u32(d, "DeviceType").unwrap_or(0);
        if kind == LOOPBACK {
            continue;
        }
        s.devices.push(NetDevice {
            path: path.to_owned(),
            interface: prop_str(d, "Interface", 32).unwrap_or_default(),
            kind: DeviceKind::from_nm(kind),
            managed: prop_bool(d, "Managed").unwrap_or(true),
            connected: prop_u32(d, "State") == Some(DEVICE_ACTIVATED),
            active_connection: prop_str(d, "ActiveConnection", 256).filter(|p| p != "/"),
        });
    }
    let active_aps: Vec<&str> = s.wifi_devices.iter().filter_map(|d| d.active_ap.as_deref()).collect();
    let active_ssids: Vec<Vec<u8>> = active_aps
        .iter()
        .filter_map(|p| objects.get(p, AP_IFACE).and_then(|a| prop_bytes(a, "Ssid", MAX_SSID)))
        .collect();
    let mut by_ssid: BTreeMap<Vec<u8>, AccessPoint> = BTreeMap::new();
    for (path, a) in objects.with(AP_IFACE) {
        let Some(ssid_bytes) = prop_bytes(a, "Ssid", MAX_SSID).filter(|b| !b.is_empty()) else { continue };
        let strength = prop_u32(a, "Strength").unwrap_or(0).min(100) as u8;
        let ap = AccessPoint {
            path: path.to_owned(),
            ssid: ssid_display(&ssid_bytes),
            strength,
            security: security_from_flags(
                prop_u32(a, "Flags").unwrap_or(0),
                prop_u32(a, "WpaFlags").unwrap_or(0),
                prop_u32(a, "RsnFlags").unwrap_or(0),
            ),
            frequency: prop_u32(a, "Frequency").unwrap_or(0),
            active: active_ssids.contains(&ssid_bytes),
            saved: settings.values().any(|c| c.ssid.as_deref() == Some(ssid_bytes.as_slice())),
            ssid_bytes: ssid_bytes.clone(),
        };
        match by_ssid.get_mut(&ssid_bytes) {
            // Etkin BSS'i tercih et, yoksa en güçlüsünü.
            Some(old) if active_aps.contains(&old.path.as_str()) => {}
            Some(old) if active_aps.contains(&path) || strength > old.strength => *old = ap,
            Some(_) => {}
            None => {
                by_ssid.insert(ssid_bytes, ap);
            }
        }
    }
    s.access_points = by_ssid.into_values().collect();
    s.access_points.sort_by(|a, b| b.active.cmp(&a.active).then(b.strength.cmp(&a.strength)).then(a.ssid.cmp(&b.ssid)));
    for (path, c) in settings {
        s.saved.push(SavedConnection {
            path: path.clone(),
            id: c.id.clone(),
            uuid: c.uuid.clone(),
            kind: c.kind.clone(),
            ssid: c.ssid.clone(),
            autoconnect: c.autoconnect,
            metered: c.metered,
            active: active
                .iter()
                .find(|a| a.connection == *path && a.state != ActiveState::Deactivated)
                .map(|a| a.path.clone()),
        });
    }
    s.saved.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()).then(a.path.cmp(&b.path)));
    s.active = active;
    s
}

/// NM'den gelen `a{sa{sv}}` ayarlarından bir ayar grubunun (`setting`) değerini okur.
pub(crate) fn setting_bytes(settings: &Settings, group: &str, key: &str, max: usize) -> Option<Vec<u8>> {
    settings.get(group).and_then(|g| g.get(key)).and_then(|v| value_bytes(v, max))
}

pub(crate) fn setting_str(settings: &Settings, group: &str, key: &str, max: usize) -> Option<String> {
    settings.get(group).and_then(|g| g.get(key)).and_then(|v| value_str(v, max))
}

pub(crate) const IP4_IFACE: &str = "org.freedesktop.NetworkManager.IP4Config";
pub(crate) const IP6_IFACE: &str = "org.freedesktop.NetworkManager.IP6Config";
pub(crate) const WIRED_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wired";
/// Adres/DNS listesi başına en fazla öğe.
const MAX_ADDRESSES: usize = 64;

/// `aa{sv}` listesindeki her sözlükten `address` (dize) ve `prefix` (sayı).
fn address_data(v: Option<&zbus::zvariant::OwnedValue>) -> Vec<(String, Option<i64>)> {
    let Some(Value::Array(a)) = v.map(|v| &**v) else { return Vec::new() };
    a.inner()
        .iter()
        .take(MAX_ADDRESSES)
        .filter_map(|e| {
            let Value::Dict(d) = e else { return None };
            let (mut address, mut prefix) = (None, None);
            for (k, val) in d.iter() {
                match value_str(k, 32).as_deref() {
                    Some("address") => address = value_str(val, 64),
                    Some("prefix") => prefix = value_i64(val),
                    _ => {}
                }
            }
            Some((address?, prefix))
        })
        .collect()
}

fn nets(list: Vec<(String, Option<i64>)>) -> Vec<IpNet> {
    list.into_iter()
        .filter_map(|(a, p)| {
            let addr: IpAddr = a.parse().ok()?;
            let prefix = u8::try_from(p?).ok()?;
            (prefix <= if addr.is_ipv4() { 32 } else { 128 }).then_some(IpNet { addr, prefix })
        })
        .collect()
}

fn gateway(props: Option<&PropMap>) -> Option<IpAddr> {
    props.and_then(|p| prop_str(p, "Gateway", 64)).and_then(|g| g.parse().ok())
}

/// Ağaçtaki cihaz + okunan IP4Config/IP6Config/Wired özellikleri → ayrıntı.
pub(crate) fn details(
    objects: &ObjectStore,
    path: &str,
    ip4: Option<&PropMap>,
    ip6: Option<&PropMap>,
    wired: Option<&PropMap>,
) -> Option<DeviceDetails> {
    let d = objects.get(path, DEVICE_IFACE)?;
    let wireless = objects.get(path, WIRELESS_IFACE);
    let hw_address = prop_str(d, "HwAddress", 64)
        .or_else(|| wired.and_then(|w| prop_str(w, "HwAddress", 64)))
        .or_else(|| wireless.and_then(|w| prop_str(w, "HwAddress", 64)))
        .filter(|a| !a.is_empty());
    let speed_mbps = wired
        .and_then(|w| prop_u32(w, "Speed"))
        .or_else(|| wireless.and_then(|w| prop_u32(w, "Bitrate")).map(|kbps| kbps / 1000))
        .filter(|s| *s > 0);
    let mut dns: Vec<IpAddr> = address_data(ip4.and_then(|p| p.get("NameserverData")))
        .into_iter()
        .filter_map(|(a, _)| a.parse().ok())
        .collect();
    if let Some(Value::Array(a)) = ip6.and_then(|p| p.get("Nameservers")).map(|v| &**v) {
        for e in a.inner().iter().take(MAX_ADDRESSES) {
            if let Some(b) = value_bytes(e, 16)
                && let Ok(octets) = <[u8; 16]>::try_from(b.as_slice())
            {
                dns.push(IpAddr::V6(Ipv6Addr::from(octets)));
            }
        }
    }
    Some(DeviceDetails {
        path: path.to_owned(),
        interface: prop_str(d, "Interface", 32).unwrap_or_default(),
        kind: DeviceKind::from_nm(prop_u32(d, "DeviceType").unwrap_or(0)),
        hw_address,
        speed_mbps,
        ipv4: nets(address_data(ip4.and_then(|p| p.get("AddressData")))),
        ipv4_gateway: gateway(ip4),
        ipv6: nets(address_data(ip6.and_then(|p| p.get("AddressData")))),
        ipv6_gateway: gateway(ip6),
        dns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_dbus::PropMap;
    use zbus::zvariant::{OwnedValue, Value};

    fn props(entries: &[(&str, Value<'static>)]) -> PropMap {
        entries.iter().map(|(k, v)| (k.to_string(), OwnedValue::try_from(v.try_clone().unwrap()).unwrap())).collect()
    }

    fn ap(o: &mut ObjectStore, n: u32, ssid: &[u8], strength: u8, rsn: u32) -> String {
        let path = format!("{NM_PATH}/AccessPoint/{n}");
        o.add(
            &path,
            [(
                AP_IFACE.to_owned(),
                props(&[
                    ("Ssid", Value::from(ssid.to_vec())),
                    ("Strength", Value::U8(strength)),
                    ("Flags", Value::U32(1)),
                    ("WpaFlags", Value::U32(0)),
                    ("RsnFlags", Value::U32(rsn)),
                    ("Frequency", Value::U32(5180)),
                ]),
            )],
        );
        path
    }

    #[test]
    fn security_flags() {
        assert_eq!(security_from_flags(0, 0, 0), Security::Open);
        assert_eq!(security_from_flags(1, 0, 0), Security::Wep);
        assert_eq!(security_from_flags(1, 0, 0x100), Security::WpaPsk);
        assert_eq!(security_from_flags(1, 0, 0x500), Security::WpaPsk, "geçiş modu");
        assert_eq!(security_from_flags(1, 0, 0x400), Security::Sae);
        assert_eq!(security_from_flags(1, 0, 0x200), Security::Enterprise);
        assert_eq!(security_from_flags(0, 0, 0x800), Security::Owe);
    }

    #[test]
    fn ssid_is_sanitized() {
        assert_eq!(ssid_display(b"Ev\x07Agi"), "EvAgi");
        assert_eq!(ssid_display(&[0xff, b'a']), "\u{fffd}a");
        assert_eq!(ssid_display(&[b'x'; 40]).len(), 32);
    }

    #[test]
    fn state_derivation() {
        let mut o = new_objects();
        o.add(
            NM_PATH,
            [(
                NM_IFACE.to_owned(),
                props(&[
                    ("State", Value::U32(70)),
                    ("Connectivity", Value::U32(4)),
                    ("WirelessEnabled", Value::Bool(true)),
                    ("WirelessHardwareEnabled", Value::Bool(true)),
                    ("NetworkingEnabled", Value::Bool(true)),
                    (
                        "PrimaryConnection",
                        Value::from(
                            zbus::zvariant::ObjectPath::try_from("/org/freedesktop/NetworkManager/ActiveConnection/1")
                                .unwrap(),
                        ),
                    ),
                    ("PrimaryConnectionType", Value::from("802-11-wireless")),
                ]),
            )],
        );
        let weak = ap(&mut o, 1, b"Ev", 40, 0x100);
        let strong = ap(&mut o, 2, b"Ev", 80, 0x100);
        ap(&mut o, 3, b"Komsu", 90, 0x200);
        ap(&mut o, 4, b"", 99, 0);
        let dev = format!("{NM_PATH}/Devices/3");
        o.add(
            &dev,
            [
                (DEVICE_IFACE.to_owned(), props(&[("Interface", Value::from("wlan0"))])),
                (
                    WIRELESS_IFACE.to_owned(),
                    props(&[(
                        "ActiveAccessPoint",
                        Value::from(zbus::zvariant::ObjectPath::try_from(weak.clone()).unwrap()),
                    )]),
                ),
            ],
        );
        o.add(
            &format!("{NM_PATH}/ActiveConnection/1"),
            [(
                ACTIVE_IFACE.to_owned(),
                props(&[
                    ("Id", Value::from("Ev")),
                    ("Type", Value::from("802-11-wireless")),
                    ("State", Value::U32(2)),
                    (
                        "Connection",
                        Value::from(
                            zbus::zvariant::ObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/1").unwrap(),
                        ),
                    ),
                ]),
            )],
        );
        let mut settings = BTreeMap::new();
        settings.insert(
            "/org/freedesktop/NetworkManager/Settings/1".to_owned(),
            SettingsSummary {
                id: "Ev".into(),
                uuid: "u1".into(),
                kind: ConnectionKind::Wifi,
                ssid: Some(b"Ev".to_vec()),
                autoconnect: true,
                metered: Metered::Unknown,
            },
        );
        settings.insert(
            "/org/freedesktop/NetworkManager/Settings/2".to_owned(),
            SettingsSummary {
                id: "İş VPN".into(),
                uuid: "u2".into(),
                kind: ConnectionKind::WireGuard,
                ssid: None,
                autoconnect: false,
                metered: Metered::Yes,
            },
        );
        let s = state(&o, &settings, true);
        assert!(s.state.is_connected());
        assert_eq!(s.connectivity, Connectivity::Full);
        assert_eq!(s.primary_kind, Some(ConnectionKind::Wifi));
        assert_eq!(s.primary_name.as_deref(), Some("Ev"));
        assert_eq!(s.access_points.len(), 2, "aynı SSID birleşir, gizli ağ yok");
        let ev = &s.access_points[0];
        assert!(ev.active && ev.saved);
        assert_eq!(ev.path, weak, "etkin BSS tercih edilir");
        assert_ne!(ev.path, strong);
        assert_eq!(s.access_points[1].security, Security::Enterprise);
        assert_eq!(s.wifi_devices[0].interface, "wlan0");
        assert_eq!(s.vpns().count(), 1);
        let vpn = s.vpns().next().unwrap();
        assert_eq!((vpn.autoconnect, vpn.metered), (false, Metered::Yes));
        assert_eq!(s.devices.len(), 1);
        assert_eq!((s.devices[0].interface.as_str(), s.devices[0].kind), ("wlan0", DeviceKind::Other));
        assert!(s.saved.iter().find(|c| c.id == "Ev").unwrap().active.is_some());
    }

    #[test]
    fn summary_reads_autoconnect_and_metered() {
        let conn = |extra: &[(&str, Value<'static>)]| -> Settings {
            let mut c = props(&[("id", Value::from("x")), ("uuid", Value::from("u")), ("type", Value::from("vpn"))]);
            c.extend(props(extra));
            HashMap::from([("connection".to_owned(), c)])
        };
        let s = SettingsSummary::parse(&conn(&[])).unwrap();
        assert_eq!((s.autoconnect, s.metered), (true, Metered::Unknown), "NM varsayılanları");
        let s = SettingsSummary::parse(&conn(&[("autoconnect", Value::Bool(false)), ("metered", Value::I32(2))])).unwrap();
        assert_eq!((s.autoconnect, s.metered), (false, Metered::No));
        assert_eq!(Metered::from_nm(i64::from(Metered::GuessYes.to_nm())), Metered::GuessYes);
    }

    #[test]
    fn device_details_from_configs() {
        use zbus::zvariant::ObjectPath;
        let mut o = new_objects();
        let dev = format!("{NM_PATH}/Devices/2");
        o.add(
            &dev,
            [(
                DEVICE_IFACE.to_owned(),
                props(&[
                    ("Interface", Value::from("enp1s0")),
                    ("DeviceType", Value::U32(1)),
                    ("HwAddress", Value::from("52:54:00:12:34:56")),
                    ("Ip4Config", Value::from(ObjectPath::try_from("/org/freedesktop/NetworkManager/IP4Config/1").unwrap())),
                ]),
            )],
        );
        let addr = |a: &str, p: u32| HashMap::from([("address", Value::from(a.to_owned())), ("prefix", Value::U32(p))]);
        let ip4 = props(&[
            ("AddressData", Value::from(vec![addr("192.168.1.20", 24), addr("bozuk", 24)])),
            ("Gateway", Value::from("192.168.1.1")),
            ("NameserverData", Value::from(vec![HashMap::from([("address", Value::from("192.168.1.1".to_owned()))])])),
        ]);
        let ip6 = props(&[
            ("AddressData", Value::from(vec![addr("fd00::20", 64)])),
            ("Gateway", Value::from("")),
            ("Nameservers", Value::from(vec![std::net::Ipv6Addr::LOCALHOST.octets().to_vec()])),
        ]);
        let wired = props(&[("Speed", Value::U32(1000))]);
        let d = details(&o, &dev, Some(&ip4), Some(&ip6), Some(&wired)).unwrap();
        assert_eq!((d.interface.as_str(), d.kind), ("enp1s0", DeviceKind::Ethernet));
        assert_eq!(d.hw_address.as_deref(), Some("52:54:00:12:34:56"));
        assert_eq!(d.speed_mbps, Some(1000));
        assert_eq!(d.ipv4.iter().map(ToString::to_string).collect::<Vec<_>>(), ["192.168.1.20/24"]);
        assert_eq!(d.ipv4_gateway, Some("192.168.1.1".parse().unwrap()));
        assert_eq!(d.ipv6[0].to_string(), "fd00::20/64");
        assert_eq!(d.ipv6_gateway, None);
        assert_eq!(d.dns, ["192.168.1.1".parse::<IpAddr>().unwrap(), "::1".parse().unwrap()]);
        assert!(details(&o, "/yok", None, None, None).is_none());
    }
}
