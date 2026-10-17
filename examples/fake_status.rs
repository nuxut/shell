//! Sahte NetworkManager + UPower + BlueZ (yalnızca izole test ortamında görsel test için): bar'ın
//! `status` göstergelerini (ağ, Bluetooth, pil) gerçek sistem servisleri olmadan çizdirir.
//! Hiçbir yazma uygulanmaz; yalnızca okunan özellikler sunulur.
//!
//! ```sh
//! dbus-run-session -- sh betik.sh                       # özel oturum bus'ı
//! cargo run --example fake_status -- --battery 12 &     # o bus'ta üç adı alır
//! DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" nuxut-shell
//! ```
//!
//! Seçenekler: `--wifi <0..=100>` (bağlı Wi-Fi gücü; `--ethernet` kablolu, `--offline` bağlantı
//! yok), `--battery <0..=100>` (`--charging` şarjda), `--bt off|on|connected`.
//!
//! Güvenlik: yalnızca `DBUS_SESSION_BUS_ADDRESS` bus'ına bağlanır; sistem bus'ında bu adları
//! alamaz, oturum bus'ı ise `dbus-run-session`'ın özel bus'ı olmalıdır.

use std::collections::HashMap;
use std::time::Duration;

use zbus::fdo;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const DEV: &str = "/org/freedesktop/NetworkManager/Devices/1";
const AP: &str = "/org/freedesktop/NetworkManager/AccessPoint/1";
const ACTIVE: &str = "/org/freedesktop/NetworkManager/ActiveConnection/1";
const SAVED: &str = "/org/freedesktop/NetworkManager/Settings/1";
const DISPLAY: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const HCI0: &str = "/org/bluez/hci0";
const HEADSET: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF";

fn path(p: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p).expect("sabit yol")
}

#[derive(Clone, Copy, PartialEq)]
enum Link {
    Wifi(u8),
    Ethernet,
    Offline,
}

struct Options {
    link: Link,
    battery: Option<f64>,
    charging: bool,
    bt: &'static str,
}

fn options() -> Options {
    let mut o = Options { link: Link::Wifi(70), battery: Some(64.0), charging: false, bt: "connected" };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--wifi" => o.link = Link::Wifi(args.next().and_then(|v| v.parse().ok()).unwrap_or(70).min(100)),
            "--ethernet" => o.link = Link::Ethernet,
            "--offline" => o.link = Link::Offline,
            "--battery" => o.battery = args.next().and_then(|v| v.parse::<f64>().ok()).map(|v| v.clamp(0.0, 100.0)),
            "--no-battery" => o.battery = None,
            "--charging" => o.charging = true,
            "--bt" => {
                o.bt = match args.next().as_deref() {
                    Some("off") => "off",
                    Some("on") => "on",
                    _ => "connected",
                }
            }
            other => eprintln!("bilinmeyen seçenek yok sayıldı: {other}"),
        }
    }
    o
}

// --- NetworkManager

struct Manager {
    link: Link,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    #[zbus(property)]
    fn state(&self) -> u32 {
        if self.link == Link::Offline { 20 } else { 70 }
    }
    #[zbus(property)]
    fn connectivity(&self) -> u32 {
        if self.link == Link::Offline { 1 } else { 4 }
    }
    #[zbus(property)]
    fn networking_enabled(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn wireless_enabled(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn primary_connection(&self) -> OwnedObjectPath {
        path(if self.link == Link::Offline { "/" } else { ACTIVE })
    }
    #[zbus(property)]
    fn primary_connection_type(&self) -> String {
        match self.link {
            Link::Wifi(_) => "802-11-wireless".into(),
            Link::Ethernet => "802-3-ethernet".into(),
            Link::Offline => String::new(),
        }
    }
}

struct Device;

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    #[zbus(property)]
    fn interface(&self) -> String {
        "wlan0".into()
    }
    #[zbus(property)]
    fn device_type(&self) -> u32 {
        2
    }
}

struct Wireless {
    connected: bool,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl Wireless {
    #[zbus(property)]
    fn access_points(&self) -> Vec<OwnedObjectPath> {
        vec![path(AP)]
    }
    #[zbus(property)]
    fn active_access_point(&self) -> OwnedObjectPath {
        path(if self.connected { AP } else { "/" })
    }
    fn request_scan(&self, _options: HashMap<String, OwnedValue>) {}
}

struct AccessPoint {
    strength: u8,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> Vec<u8> {
        b"Nuxut Test".to_vec()
    }
    #[zbus(property)]
    fn strength(&self) -> u8 {
        self.strength
    }
    #[zbus(property)]
    fn flags(&self) -> u32 {
        1
    }
    #[zbus(property)]
    fn wpa_flags(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn rsn_flags(&self) -> u32 {
        0x100
    }
    #[zbus(property)]
    fn frequency(&self) -> u32 {
        5180
    }
}

struct Active {
    ethernet: bool,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl Active {
    #[zbus(property)]
    fn id(&self) -> String {
        if self.ethernet { "Kablolu bağlantı 1".into() } else { "Nuxut Test".into() }
    }
    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        if self.ethernet { "802-3-ethernet".into() } else { "802-11-wireless".into() }
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        2
    }
    #[zbus(property)]
    fn connection(&self) -> OwnedObjectPath {
        path(SAVED)
    }
}

struct Saved;

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl Saved {
    fn get_settings(&self) -> HashMap<String, HashMap<String, OwnedValue>> {
        let v = |s: &str| OwnedValue::try_from(Value::from(s)).expect("dize");
        HashMap::from([(
            "connection".to_owned(),
            HashMap::from([("id".to_owned(), v("Nuxut Test")), ("uuid".to_owned(), v("uuid-test")), ("type".to_owned(), v("802-11-wireless"))]),
        )])
    }
}

// --- UPower

struct UPower {
    on_battery: bool,
}

#[zbus::interface(name = "org.freedesktop.UPower")]
impl UPower {
    #[zbus(property)]
    fn on_battery(&self) -> bool {
        self.on_battery
    }
    #[zbus(property)]
    fn lid_is_present(&self) -> bool {
        true
    }
}

struct Battery {
    percentage: f64,
    charging: bool,
}

#[zbus::interface(name = "org.freedesktop.UPower.Device")]
impl Battery {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> u32 {
        2
    }
    #[zbus(property)]
    fn is_present(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn percentage(&self) -> f64 {
        self.percentage
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        if self.charging { 1 } else { 2 }
    }
    #[zbus(property)]
    fn time_to_empty(&self) -> i64 {
        if self.charging { 0 } else { (self.percentage * 90.0) as i64 }
    }
    #[zbus(property)]
    fn time_to_full(&self) -> i64 {
        if self.charging { ((100.0 - self.percentage) * 60.0) as i64 } else { 0 }
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        "battery-symbolic".into()
    }
    #[zbus(property)]
    fn warning_level(&self) -> u32 {
        1
    }
}

// --- BlueZ

struct Adapter {
    powered: bool,
}

#[zbus::interface(name = "org.bluez.Adapter1")]
impl Adapter {
    #[zbus(property)]
    fn alias(&self) -> String {
        "nuxut-test".into()
    }
    #[zbus(property)]
    fn address(&self) -> String {
        "00:11:22:33:44:55".into()
    }
    #[zbus(property)]
    fn powered(&self) -> bool {
        self.powered
    }
    #[zbus(property)]
    fn discovering(&self) -> bool {
        false
    }
}

struct AgentManager;

#[zbus::interface(name = "org.bluez.AgentManager1")]
impl AgentManager {
    fn register_agent(&self, _agent: zbus::zvariant::ObjectPath<'_>, _capability: String) {}
    fn request_default_agent(&self, _agent: zbus::zvariant::ObjectPath<'_>) {}
}

struct Headset {
    connected: bool,
}

#[zbus::interface(name = "org.bluez.Device1")]
impl Headset {
    #[zbus(property)]
    fn alias(&self) -> String {
        "Test Kulaklığı".into()
    }
    #[zbus(property)]
    fn address(&self) -> String {
        "AA:BB:CC:DD:EE:FF".into()
    }
    #[zbus(property)]
    fn adapter(&self) -> OwnedObjectPath {
        path(HCI0)
    }
    #[zbus(property)]
    fn icon(&self) -> String {
        "audio-headset".into()
    }
    #[zbus(property)]
    fn paired(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn trusted(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn connected(&self) -> bool {
        self.connected
    }
}

struct HeadsetBattery;

#[zbus::interface(name = "org.bluez.Battery1")]
impl HeadsetBattery {
    #[zbus(property)]
    fn percentage(&self) -> u8 {
        80
    }
}

fn main() -> zbus::Result<()> {
    let o = options();
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").map_err(|_| zbus::Error::Address("DBUS_SESSION_BUS_ADDRESS yok".into()))?;

    let nm = zbus::blocking::connection::Builder::address(address.as_str())?.build()?;
    {
        let s = nm.object_server();
        s.at("/org/freedesktop", fdo::ObjectManager)?;
        s.at(NM_PATH, Manager { link: o.link })?;
        s.at(DEV, Device)?;
        s.at(DEV, Wireless { connected: matches!(o.link, Link::Wifi(_)) })?;
        s.at(AP, AccessPoint { strength: if let Link::Wifi(v) = o.link { v } else { 60 } })?;
        if o.link != Link::Offline {
            s.at(ACTIVE, Active { ethernet: o.link == Link::Ethernet })?;
        }
        s.at(SAVED, Saved)?;
    }
    nm.request_name("org.freedesktop.NetworkManager")?;

    let upower = zbus::blocking::connection::Builder::address(address.as_str())?.build()?;
    if let Some(percentage) = o.battery {
        upower.object_server().at("/org/freedesktop/UPower", UPower { on_battery: !o.charging })?;
        upower.object_server().at(DISPLAY, Battery { percentage, charging: o.charging })?;
        upower.request_name("org.freedesktop.UPower")?;
    }

    let bluez = zbus::blocking::connection::Builder::address(address.as_str())?.build()?;
    if o.bt != "none" {
        let s = bluez.object_server();
        s.at("/", fdo::ObjectManager)?;
        s.at("/org/bluez", AgentManager)?;
        s.at(HCI0, Adapter { powered: o.bt != "off" })?;
        s.at(HEADSET, Headset { connected: o.bt == "connected" })?;
        s.at(HEADSET, HeadsetBattery)?;
        bluez.request_name("org.bluez")?;
    }

    eprintln!("sahte durum servisleri hazır");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
