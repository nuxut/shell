//! Saf BlueZ modeli: ObjectManager nesnelerinden adaptör ve cihaz listesi.

use nuxut_dbus::{ObjectStore, Unavailable, prop_bool, prop_i64, prop_str, prop_u32};

pub(crate) const ADAPTER_IFACE: &str = "org.bluez.Adapter1";
pub(crate) const DEVICE_IFACE: &str = "org.bluez.Device1";
pub(crate) const BATTERY_IFACE: &str = "org.bluez.Battery1";

/// Ad sınırı (Bluetooth cihaz adı en fazla 248 bayt).
const MAX_NAME: usize = 248;
/// En fazla izlenen cihaz (taramada çevredeki her şey görünür).
pub const MAX_DEVICES: usize = 256;
pub const MAX_ADAPTERS: usize = 8;

/// `Adapter1.PowerState` (BlueZ ≥ 5.67, deneysel; yoksa `Unknown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PowerState {
    #[default]
    Unknown,
    On,
    Off,
    /// Kapalıdan açığa geçiyor.
    Enabling,
    /// Açıktan kapalıya geçiyor.
    Disabling,
    /// rfkill ile engelli (`off-blocked`); ayrıntı [`BluetoothState::rfkill`].
    Blocked,
}

impl PowerState {
    fn parse(s: &str) -> Self {
        match s {
            "on" => Self::On,
            "off" => Self::Off,
            "off-enabling" => Self::Enabling,
            "on-disabling" => Self::Disabling,
            "off-blocked" => Self::Blocked,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Adapter {
    /// D-Bus yolu (`/org/bluez/hci0`): kimlik.
    pub path: String,
    pub name: String,
    pub address: String,
    pub powered: bool,
    /// Güç durumu (rfkill engeli dahil).
    pub power_state: PowerState,
    /// Herhangi bir istemcinin taraması sürüyor mu (bizimki için [`BluetoothState::own_discovery`]).
    pub discovering: bool,
    pub pairable: bool,
    /// Başka cihazlara görünür mü?
    pub discoverable: bool,
    /// Görünürlüğün kendiliğinden kapanma süresi (saniye; 0 = süresiz).
    pub discoverable_timeout: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Device {
    /// D-Bus yolu (`/org/bluez/hci0/dev_AA_BB_…`): kimlik.
    pub path: String,
    pub adapter: String,
    pub address: String,
    /// Gösterilecek ad: `Alias` (boşsa `Name`, o da yoksa adres).
    pub name: String,
    /// Ham `Alias` (BlueZ: kullanıcının verdiği ad, yoksa `Name`, o da yoksa adresten türetilmiş).
    pub alias: String,
    /// Ham `Name` (cihazın kendi bildirdiği ad). `None`: BlueZ ad bilmiyor — tarama listesinde
    /// adsız cihaz gizlenebilir.
    pub remote_name: Option<String>,
    /// Ham freedesktop ikon adı (`audio-headset`, `input-mouse`...); eşleme Ayarlar'da.
    pub icon: Option<String>,
    /// Ham `Class` (BR/EDR cihaz sınıfı).
    pub class: Option<u32>,
    /// Ham `Appearance` (LE görünüm kodu).
    pub appearance: Option<u16>,
    pub connected: bool,
    /// Servisler çözüldü (bağlantı kullanıma hazır).
    pub services_resolved: bool,
    pub paired: bool,
    pub trusted: bool,
    pub blocked: bool,
    /// Pil yüzdesi (`Battery1`; destekleyen cihazlarda).
    pub battery: Option<u8>,
    /// Sinyal gücü (yalnızca taramada görünenlerde).
    pub rssi: Option<i16>,
}

/// Bluetooth rfkill engeli (`/sys/class/rfkill/*`, `type` = `bluetooth`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rfkill {
    /// Yazılımla engelli (`soft`; kullanıcı/uçak kipi kaldırabilir).
    pub soft: bool,
    /// Donanımla engelli (`hard`; fiziksel anahtar — yazılım kaldıramaz).
    pub hard: bool,
}

impl Rfkill {
    pub fn blocked(self) -> bool {
        self.soft || self.hard
    }
}

/// `dir` altındaki rfkill girdilerinden Bluetooth olanların birleşik engeli (herhangi biri).
/// Bluetooth girdisi yoksa ya da okunamazsa `None`. Yalnız okuma; yazma yok.
pub fn read_rfkill(dir: &std::path::Path) -> Option<Rfkill> {
    // Her dosyadan en fazla 64 bayt (sysfs değerleri kısa; beklenmedik büyük dosya okunmaz).
    let read = |p: std::path::PathBuf| {
        use std::io::Read;
        let mut s = String::new();
        std::fs::File::open(p).ok()?.take(64).read_to_string(&mut s).ok()?;
        Some(s.trim().to_owned())
    };
    let mut found: Option<Rfkill> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten().take(64) {
        let p = entry.path();
        if read(p.join("type")).as_deref() != Some("bluetooth") {
            continue;
        }
        let r = found.get_or_insert_with(Rfkill::default);
        r.soft |= read(p.join("soft")).as_deref() == Some("1");
        r.hard |= read(p.join("hard")).as_deref() == Some("1");
    }
    found
}

/// Bu istemcinin eşleştirme ajanı durumu (kabuğun `msg bluetooth agent-status` yanıtı).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AgentStatus {
    /// `AgentManager1.RegisterAgent` başarılı (bu BlueZ sürecinde).
    pub registered: bool,
    /// `RequestDefaultAgent` başarılı: başka istemcilerin (Ayarlar) `Pair`'ı da bu ajana gelir.
    pub default: bool,
}

/// Durumun tamamı.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BluetoothState {
    /// BlueZ çalışıyor mu?
    pub available: bool,
    /// `available == false` iken nedeni (bus'a ulaşılamıyor / BlueZ çalışmıyor); `None`:
    /// kullanılabilir ya da henüz okunmadı (`Default`).
    pub unavailable: Option<Unavailable>,
    /// Adaptörler (yol sırasıyla; v1 ilkini kullanır).
    pub adapters: Vec<Adapter>,
    /// Sıralama: bağlı → eşleşmiş → diğerleri; her grupta ada göre.
    pub devices: Vec<Device>,
    /// Bu istemcinin başlattığı tarama sürüyor ([`crate::BluetoothService::start_discovery`]).
    /// `Adapter::discovering` başka bir istemcinin taramasını da gösterir.
    pub own_discovery: bool,
    /// Bir adaptör rfkill ile engelliyken sysfs'ten okunan engel (yumuşak/sert); değilse `None`.
    pub rfkill: Option<Rfkill>,
    /// Bu istemcinin ajan durumu (ajansız istemcide hep `false`).
    pub agent: AgentStatus,
}

impl BluetoothState {
    /// Varsayılan adaptör (ilki).
    pub fn adapter(&self) -> Option<&Adapter> {
        self.adapters.first()
    }

    pub fn powered(&self) -> bool {
        self.adapters.iter().any(|a| a.powered)
    }
}

/// Ham nesne ağacı (BlueZ ObjectManager).
pub(crate) type Objects = ObjectStore;

pub(crate) fn new_objects() -> Objects {
    ObjectStore::new("/org/bluez/", |iface| match iface {
        ADAPTER_IFACE => Some(MAX_ADAPTERS),
        DEVICE_IFACE | BATTERY_IFACE => Some(MAX_DEVICES),
        _ => None,
    })
}

/// Ağaçtan durum (`own_discovery` / `rfkill` çağıranca doldurulur).
pub(crate) fn state(objects: &Objects, available: bool) -> BluetoothState {
    let mut adapters = Vec::new();
    let mut devices = Vec::new();
    for (path, ifaces) in objects.iter() {
        if let Some(p) = ifaces.get(ADAPTER_IFACE) {
            adapters.push(Adapter {
                path: path.to_owned(),
                name: prop_str(p, "Alias", MAX_NAME).or_else(|| prop_str(p, "Name", MAX_NAME)).unwrap_or_default(),
                address: prop_str(p, "Address", 32).unwrap_or_default(),
                powered: prop_bool(p, "Powered").unwrap_or(false),
                power_state: prop_str(p, "PowerState", 32).map(|s| PowerState::parse(&s)).unwrap_or_default(),
                discovering: prop_bool(p, "Discovering").unwrap_or(false),
                pairable: prop_bool(p, "Pairable").unwrap_or(false),
                discoverable: prop_bool(p, "Discoverable").unwrap_or(false),
                discoverable_timeout: prop_u32(p, "DiscoverableTimeout").unwrap_or(0),
            });
        }
        if let Some(p) = ifaces.get(DEVICE_IFACE) {
            let address = prop_str(p, "Address", 32).unwrap_or_default();
            let alias = prop_str(p, "Alias", MAX_NAME).unwrap_or_default();
            let remote_name = prop_str(p, "Name", MAX_NAME).filter(|s| !s.is_empty());
            let name = Some(alias.clone())
                .filter(|s| !s.is_empty())
                .or_else(|| remote_name.clone())
                .unwrap_or_else(|| address.clone());
            devices.push(Device {
                path: path.to_owned(),
                adapter: prop_str(p, "Adapter", 128).unwrap_or_default(),
                name,
                alias,
                remote_name,
                address,
                icon: prop_str(p, "Icon", 64).filter(|s| !s.is_empty()),
                class: prop_u32(p, "Class"),
                appearance: prop_u32(p, "Appearance").and_then(|v| u16::try_from(v).ok()),
                connected: prop_bool(p, "Connected").unwrap_or(false),
                services_resolved: prop_bool(p, "ServicesResolved").unwrap_or(false),
                paired: prop_bool(p, "Paired").unwrap_or(false),
                trusted: prop_bool(p, "Trusted").unwrap_or(false),
                blocked: prop_bool(p, "Blocked").unwrap_or(false),
                battery: ifaces.get(BATTERY_IFACE).and_then(|b| prop_u32(b, "Percentage")).map(|v| v.min(100) as u8),
                rssi: prop_i64(p, "RSSI").map(|v| v.clamp(-128, 127) as i16),
            });
        }
    }
    devices.sort_by(|a, b| {
        let rank = |d: &Device| (!d.connected, !d.paired);
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.path.cmp(&b.path))
    });
    BluetoothState {
        available,
        unavailable: (!available).then_some(Unavailable::ServiceNotRunning),
        adapters,
        devices,
        own_discovery: false,
        rfkill: None,
        agent: AgentStatus::default(),
    }
}

/// PIN kodu geçerli mi (BlueZ: 1–16 yazdırılabilir ASCII karakter).
pub fn valid_pin(pin: &str) -> bool {
    (1..=16).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_graphic())
}

/// Passkey 0..=999999.
pub fn valid_passkey(passkey: u32) -> bool {
    passkey <= 999_999
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_dbus::PropMap;
    use zbus::zvariant::{OwnedValue, Value};

    fn props(entries: &[(&str, Value<'static>)]) -> PropMap {
        entries.iter().map(|(k, v)| (k.to_string(), OwnedValue::try_from(v.try_clone().unwrap()).unwrap())).collect()
    }

    #[test]
    fn state_from_objects() {
        let mut o = new_objects();
        o.add(
            "/org/bluez/hci0",
            [(ADAPTER_IFACE.to_owned(), props(&[("Alias", Value::from("laptop")), ("Powered", Value::Bool(true))]))],
        );
        o.add(
            "/org/bluez/hci0/dev_1",
            [
                (DEVICE_IFACE.to_owned(), props(&[("Address", Value::from("AA")), ("Paired", Value::Bool(true))])),
                (BATTERY_IFACE.to_owned(), props(&[("Percentage", Value::U8(87))])),
                ("org.bluez.MediaControl1".to_owned(), props(&[])),
            ],
        );
        o.add(
            "/org/bluez/hci0/dev_2",
            [(
                DEVICE_IFACE.to_owned(),
                props(&[
                    ("Alias", Value::from("Kulaklık")),
                    ("Connected", Value::Bool(true)),
                    ("RSSI", Value::I16(-60)),
                ]),
            )],
        );
        assert!(!o.add("/evil", [(DEVICE_IFACE.to_owned(), props(&[]))]));
        let s = state(&o, true);
        assert!(s.powered());
        assert_eq!(s.adapter().unwrap().name, "laptop");
        assert_eq!(s.devices.len(), 2);
        assert_eq!(s.devices[0].name, "Kulaklık", "bağlı önce");
        assert_eq!(s.devices[0].rssi, Some(-60));
        assert_eq!(s.devices[1].name, "AA", "ad yoksa adres");
        assert_eq!(s.devices[1].battery, Some(87));

        assert!(o.update("/org/bluez/hci0/dev_1", DEVICE_IFACE, props(&[("Connected", Value::Bool(true))]), &[]));
        assert!(!o.update("/org/bluez/hci0/dev_9", DEVICE_IFACE, props(&[]), &[]));
        assert!(o.remove("/org/bluez/hci0/dev_1", &[BATTERY_IFACE.to_owned()]));
        assert_eq!(state(&o, true).devices.iter().find(|d| d.address == "AA").unwrap().battery, None);
        assert!(o.remove("/org/bluez/hci0/dev_1", &[DEVICE_IFACE.to_owned()]));
        assert_eq!(state(&o, true).devices.len(), 1);
    }

    #[test]
    fn device_limit() {
        let mut o = new_objects();
        for i in 0..MAX_DEVICES + 10 {
            o.add(&format!("/org/bluez/hci0/dev_{i}"), [(DEVICE_IFACE.to_owned(), props(&[]))]);
        }
        assert_eq!(state(&o, true).devices.len(), MAX_DEVICES);
    }

    #[test]
    fn pins() {
        assert!(valid_pin("0000"));
        assert!(!valid_pin(""));
        assert!(!valid_pin("12 34"));
        assert!(!valid_pin(&"1".repeat(17)));
        assert!(valid_passkey(999_999));
        assert!(!valid_passkey(1_000_000));
    }

    #[test]
    fn raw_fields_and_power_state() {
        let mut o = new_objects();
        o.add(
            "/org/bluez/hci0",
            [(ADAPTER_IFACE.to_owned(), props(&[("Alias", Value::from("pc")), ("PowerState", Value::from("off-blocked"))]))],
        );
        o.add(
            "/org/bluez/hci0/dev_1",
            [(
                DEVICE_IFACE.to_owned(),
                props(&[
                    ("Address", Value::from("AA:BB")),
                    ("Alias", Value::from("AA-BB")),
                    ("Class", Value::U32(0x240404)),
                    ("Appearance", Value::U16(0x03c1)),
                    ("ServicesResolved", Value::Bool(true)),
                ]),
            )],
        );
        o.add(
            "/org/bluez/hci0/dev_2",
            [(DEVICE_IFACE.to_owned(), props(&[("Alias", Value::from("Benim")), ("Name", Value::from("JBL Go"))]))],
        );
        let s = state(&o, true);
        assert_eq!(s.adapter().unwrap().power_state, PowerState::Blocked);
        let anon = s.devices.iter().find(|d| d.path.ends_with("dev_1")).unwrap();
        assert_eq!((anon.remote_name.as_deref(), anon.alias.as_str()), (None, "AA-BB"), "adsız: gizlenebilir");
        assert_eq!((anon.class, anon.appearance, anon.services_resolved), (Some(0x240404), Some(0x03c1), true));
        let named = s.devices.iter().find(|d| d.path.ends_with("dev_2")).unwrap();
        assert_eq!((named.name.as_str(), named.remote_name.as_deref()), ("Benim", Some("JBL Go")));
        assert_eq!(PowerState::parse("on-disabling"), PowerState::Disabling);
        assert_eq!(PowerState::parse("başka"), PowerState::Unknown);
    }

    #[test]
    fn rfkill_from_sysfs_dir() {
        let dir = std::env::temp_dir().join(format!("nuxut-rfkill-{}", std::process::id()));
        let entry = |name: &str, kind: &str, soft: &str, hard: &str| {
            let p = dir.join(name);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("type"), format!("{kind}\n")).unwrap();
            std::fs::write(p.join("soft"), format!("{soft}\n")).unwrap();
            std::fs::write(p.join("hard"), format!("{hard}\n")).unwrap();
        };
        entry("rfkill0", "wlan", "1", "1");
        assert_eq!(read_rfkill(&dir), None, "Bluetooth girdisi yok");
        entry("rfkill1", "bluetooth", "1", "0");
        assert_eq!(read_rfkill(&dir), Some(Rfkill { soft: true, hard: false }));
        entry("rfkill2", "bluetooth", "0", "1");
        let r = read_rfkill(&dir).unwrap();
        assert!(r.soft && r.hard && r.blocked());
        assert_eq!(read_rfkill(&dir.join("yok")), None);
        // Review notu (c): okuma sınırlı — büyük dosya yalnız ilk 64 baytıyla değerlendirilir.
        std::fs::remove_dir_all(dir.join("rfkill2")).unwrap();
        let big = dir.join("rfkill9");
        std::fs::create_dir_all(&big).unwrap();
        std::fs::write(big.join("type"), format!("bluetooth{}", "x".repeat(1 << 20))).unwrap();
        std::fs::write(big.join("hard"), "1").unwrap();
        assert_eq!(read_rfkill(&dir), Some(Rfkill { soft: true, hard: false }), "dev 'type' sayılmadı");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
