//! Sahte BlueZ: entegrasyon testleri (`tests/bluetooth_service.rs`, özel bus) ve
//! `examples/fake_bluez.rs` (canlı test, `dbus-run-session` içindeki oturum bus'ı) paylaşır.
//!
//! ObjectManager + `Adapter1` (Alias/Discoverable/DiscoverableTimeout/Pairable yazılabilir) +
//! `Device1` (Trusted/Blocked/Alias yazılabilir) + `AgentManager1`. `Pair`: cihaza bir BlueZ
//! hatası atanmışsa onu döner; değilse kayıtlı ajan varsa onay sorar (ajan reddi →
//! `AuthenticationRejected`, iptal/zaman aşımı → `AuthenticationCanceled`), ajan yoksa "just
//! works" eşleşir. Her çağrı [`Log`]'a yazılır.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

pub const BLUEZ: &str = "org.bluez";
pub const HCI0: &str = "/org/bluez/hci0";

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
            println!("fake_bluez: {s}");
        }
        self.entries.lock().unwrap().push(s);
    }
    pub fn has(&self, s: &str) -> bool {
        self.entries.lock().unwrap().iter().any(|e| e == s)
    }
    pub fn count(&self, s: &str) -> usize {
        self.entries.lock().unwrap().iter().filter(|e| *e == s).count()
    }
    /// İlk eşleşen kaydın sırası (çağrı sırası denetimi).
    pub fn index(&self, s: &str) -> Option<usize> {
        self.entries.lock().unwrap().iter().position(|e| e == s)
    }
}

/// Sıradaki `StartDiscovery` çağrılarının gecikmeli başarısızlıkları.
type StartFailures = Arc<Mutex<std::collections::VecDeque<std::time::Duration>>>;

/// Kayıtlı ajan (gönderen, yol).
pub type AgentSlot = Arc<Mutex<Option<(String, String)>>>;

struct AgentManager {
    agent: AgentSlot,
    log: Log,
}

#[zbus::interface(name = "org.bluez.AgentManager1")]
impl AgentManager {
    fn register_agent(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        agent: ObjectPath<'_>,
        capability: String,
    ) {
        self.log.push(format!("RegisterAgent {capability}"));
        *self.agent.lock().unwrap() = Some((hdr.sender().unwrap().to_string(), agent.to_string()));
    }
    fn request_default_agent(&self, _agent: ObjectPath<'_>) {
        self.log.push("RequestDefaultAgent".into());
    }
}

struct Adapter {
    alias: String,
    powered: bool,
    discovering: Arc<AtomicBool>,
    start_failures: StartFailures,
    discoverable: bool,
    discoverable_timeout: u32,
    pairable: bool,
    rfkill: Arc<AtomicBool>,
    log: Log,
}

#[zbus::interface(name = "org.bluez.Adapter1")]
impl Adapter {
    #[zbus(property)]
    fn alias(&self) -> String {
        self.alias.clone()
    }
    #[zbus(property)]
    fn set_alias(&mut self, v: String) {
        self.log.push(format!("Alias {HCI0} {v}"));
        // BlueZ: boş takma ad sistem adına döner.
        self.alias = if v.is_empty() { "nuxut-vm".into() } else { v };
    }
    #[zbus(property)]
    fn address(&self) -> String {
        "00:11:22:33:44:55".into()
    }
    #[zbus(property)]
    fn powered(&self) -> bool {
        self.powered
    }
    /// rfkill engelliyken açma başarısız olur. Gerçek BlueZ `org.bluez.Error.Blocked` döner
    /// (adapter.c `property_set_mode_complete`: `MGMT_STATUS_RFKILLED`); zbus özellik yazıcısı
    /// yalnız `fdo` hatası dönebildiğinden sahte `Failed` döner — istemci `PowerState=off-blocked`
    /// ile de tanır (adın kendisi birim testinde).
    #[zbus(property)]
    fn set_powered(&mut self, v: bool) -> zbus::fdo::Result<()> {
        self.log.push(format!("Powered {v}"));
        if v && self.rfkill.load(Ordering::Relaxed) {
            return Err(zbus::fdo::Error::Failed("Blocked through rfkill".into()));
        }
        self.powered = v;
        Ok(())
    }
    #[zbus(property)]
    fn power_state(&self) -> String {
        match (self.rfkill.load(Ordering::Relaxed), self.powered) {
            (true, _) => "off-blocked".into(),
            (false, true) => "on".into(),
            (false, false) => "off".into(),
        }
    }
    fn set_discovery_filter(&self, filter: HashMap<String, OwnedValue>) {
        let transport: String = filter.get("Transport").and_then(|v| v.try_clone().ok()?.try_into().ok()).unwrap_or_default();
        let dup: bool = filter.get("DuplicateData").and_then(|v| bool::try_from(v).ok()).unwrap_or(true);
        self.log.push(format!("SetDiscoveryFilter {transport} {dup}"));
    }
    #[zbus(property)]
    fn discovering(&self) -> bool {
        self.discovering.load(Ordering::Relaxed)
    }
    #[zbus(property)]
    fn discoverable(&self) -> bool {
        self.discoverable
    }
    #[zbus(property)]
    fn set_discoverable(&mut self, v: bool) {
        self.log.push(format!("Discoverable {v}"));
        self.discoverable = v;
    }
    #[zbus(property)]
    fn discoverable_timeout(&self) -> u32 {
        self.discoverable_timeout
    }
    #[zbus(property)]
    fn set_discoverable_timeout(&mut self, v: u32) {
        self.log.push(format!("DiscoverableTimeout {v}"));
        self.discoverable_timeout = v;
    }
    #[zbus(property)]
    fn pairable(&self) -> bool {
        self.pairable
    }
    #[zbus(property)]
    fn set_pairable(&mut self, v: bool) {
        self.log.push(format!("Pairable {v}"));
        self.pairable = v;
    }
    /// `&self`: eşzamanlı çağrılar sıralanmaz (yavaş başarısız başlatma testi için).
    /// [`FakeBluez::fail_next_start`] kuyruğu doluysa o kadar bekleyip `InProgress` döner.
    async fn start_discovery(&self, #[zbus(signal_emitter)] e: SignalEmitter<'_>) -> Result<(), BluezErr> {
        let delay = self.start_failures.lock().unwrap().pop_front();
        if let Some(d) = delay {
            self.log.push("StartDiscoveryFailing".into());
            async_io::Timer::after(d).await;
            self.log.push("StartDiscoveryFailed".into());
            return Err(BluezErr::InProgress("sahte".into()));
        }
        self.log.push("StartDiscovery".into());
        self.discovering.store(true, Ordering::Relaxed);
        let _ = self.discovering_changed(&e).await;
        Ok(())
    }
    async fn stop_discovery(&self, #[zbus(signal_emitter)] e: SignalEmitter<'_>) {
        self.log.push("StopDiscovery".into());
        self.discovering.store(false, Ordering::Relaxed);
        let _ = self.discovering_changed(&e).await;
    }
    /// Cihazı gerçekten siler (`InterfacesRemoved`; cevaptan sonra, ayrı görevde).
    fn remove_device(&self, #[zbus(connection)] conn: &zbus::Connection, device: ObjectPath<'_>) {
        self.log.push(format!("RemoveDevice {device}"));
        let conn2 = conn.clone();
        let p = device.to_string();
        conn.executor()
            .spawn(
                async move {
                    let server = conn2.object_server();
                    let _ = server.remove::<Battery, _>(p.as_str()).await;
                    let _ = server.remove::<Device, _>(p.as_str()).await;
                },
                "fake-bluez-remove",
            )
            .detach();
    }
}

/// BlueZ'in `org.bluez.Error.*` hataları (sahte `Pair` için).
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum BluezErr {
    #[zbus(error)]
    ZBus(zbus::Error),
    Failed(String),
    AuthenticationRejected(String),
    AuthenticationFailed(String),
    AuthenticationCanceled(String),
    AuthenticationTimeout(String),
    ConnectionAttemptFailed(String),
    AlreadyExists(String),
    InProgress(String),
}

/// Cihaza atanabilecek eşleştirme hatası.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairError {
    Rejected,
    Failed,
    Canceled,
    Timeout,
    ConnectionAttemptFailed,
}

impl PairError {
    fn to_err(self) -> BluezErr {
        let m = || "sahte".to_owned();
        match self {
            Self::Rejected => BluezErr::AuthenticationRejected(m()),
            Self::Failed => BluezErr::AuthenticationFailed(m()),
            Self::Canceled => BluezErr::AuthenticationCanceled(m()),
            Self::Timeout => BluezErr::AuthenticationTimeout(m()),
            Self::ConnectionAttemptFailed => BluezErr::ConnectionAttemptFailed(m()),
        }
    }
}

/// Cihazın eşleştirme davranışı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairMode {
    /// Ajan varsa onay sor, yoksa "just works".
    Normal,
    /// Hep bu hatayı dön.
    Error(PairError),
    /// `CancelPairing` gelene kadar bekle (en fazla 5 sn), sonra `AuthenticationCanceled`.
    WaitCancel,
}

struct Device {
    path: String,
    alias: String,
    name: Option<&'static str>,
    icon: &'static str,
    class: u32,
    appearance: u16,
    paired: Arc<AtomicBool>,
    rssi: i16,
    trusted: bool,
    blocked: bool,
    connected: bool,
    services_resolved: bool,
    mode: PairMode,
    canceled: Arc<AtomicBool>,
    agent: AgentSlot,
    log: Log,
}

#[zbus::interface(name = "org.bluez.Device1")]
impl Device {
    #[zbus(property)]
    fn alias(&self) -> String {
        self.alias.clone()
    }
    #[zbus(property)]
    fn set_alias(&mut self, v: String) {
        self.log.push(format!("Alias {} {v}", self.path));
        self.alias = v;
    }
    /// Gerçek BlueZ'de `Name` ad bilinmiyorsa hiç yoktur; zbus özelliği koşullu saklayamadığından
    /// sahte boş dize verir (istemci boş adı "yok" sayar).
    #[zbus(property)]
    fn name(&self) -> String {
        self.name.unwrap_or_default().to_owned()
    }
    #[zbus(property)]
    fn address(&self) -> String {
        "AA:BB:CC:DD:EE:FF".into()
    }
    #[zbus(property)]
    fn adapter(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(HCI0).unwrap()
    }
    #[zbus(property)]
    fn icon(&self) -> String {
        self.icon.into()
    }
    #[zbus(property)]
    fn class(&self) -> u32 {
        self.class
    }
    #[zbus(property)]
    fn appearance(&self) -> u16 {
        self.appearance
    }
    #[zbus(property)]
    fn paired(&self) -> bool {
        self.paired.load(Ordering::Relaxed)
    }
    #[zbus(property, name = "RSSI")]
    fn rssi(&self) -> i16 {
        self.rssi
    }
    #[zbus(property)]
    fn trusted(&self) -> bool {
        self.trusted
    }
    #[zbus(property)]
    fn set_trusted(&mut self, v: bool) {
        self.log.push(format!("Trusted {} {v}", self.path));
        self.trusted = v;
    }
    #[zbus(property)]
    fn blocked(&self) -> bool {
        self.blocked
    }
    #[zbus(property)]
    fn set_blocked(&mut self, v: bool) {
        self.log.push(format!("Blocked {} {v}", self.path));
        self.blocked = v;
    }
    #[zbus(property)]
    fn connected(&self) -> bool {
        self.connected
    }
    #[zbus(property)]
    fn services_resolved(&self) -> bool {
        self.services_resolved
    }
    async fn connect(&mut self, #[zbus(signal_emitter)] e: SignalEmitter<'_>) {
        self.log.push(format!("Connect {}", self.path));
        self.connected = true;
        self.services_resolved = true;
        let _ = self.connected_changed(&e).await;
        let _ = self.services_resolved_changed(&e).await;
    }
    async fn disconnect(&mut self, #[zbus(signal_emitter)] e: SignalEmitter<'_>) {
        self.log.push(format!("Disconnect {}", self.path));
        self.connected = false;
        self.services_resolved = false;
        let _ = self.connected_changed(&e).await;
        let _ = self.services_resolved_changed(&e).await;
    }
    fn cancel_pairing(&self) {
        self.log.push(format!("CancelPairing {}", self.path));
        self.canceled.store(true, Ordering::Relaxed);
    }
    /// Gerçek BlueZ gibi ajana onay sorar (bkz. modül belgesi).
    async fn pair(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), BluezErr> {
        match self.mode {
            PairMode::Error(e) => {
                self.log.push(format!("PairError {} {e:?}", self.path));
                return Err(e.to_err());
            }
            PairMode::WaitCancel => {
                self.canceled.store(false, Ordering::Relaxed);
                self.log.push(format!("PairWaiting {}", self.path));
                for _ in 0..250 {
                    if self.canceled.load(Ordering::Relaxed) {
                        return Err(BluezErr::AuthenticationCanceled("CancelPairing".into()));
                    }
                    async_io::Timer::after(std::time::Duration::from_millis(20)).await;
                }
                return Err(BluezErr::AuthenticationTimeout("sahte zaman aşımı".into()));
            }
            PairMode::Normal => {}
        }
        let agent = self.agent.lock().unwrap().clone();
        if let Some((sender, agent)) = agent {
            let device = ObjectPath::try_from(self.path.as_str()).unwrap();
            let r = conn
                .call_method(
                    Some(sender.as_str()),
                    agent.as_str(),
                    Some("org.bluez.Agent1"),
                    "RequestConfirmation",
                    &(device, 123456u32),
                )
                .await;
            if let Err(e) = r {
                let canceled = matches!(&e, zbus::Error::MethodError(n, _, _) if n.as_str().ends_with("Canceled"));
                return Err(if canceled {
                    BluezErr::AuthenticationCanceled("ajan iptal etti".into())
                } else {
                    BluezErr::AuthenticationRejected("ajan reddetti".into())
                });
            }
        }
        // Gerçek BlueZ gibi: eşleşme kurulunca `Paired=true` (cevaptan önce sinyal).
        self.paired.store(true, Ordering::Relaxed);
        let _ = self.paired_changed(&emitter).await;
        self.log.push(format!("Paired {}", self.path));
        Ok(())
    }
}

/// `org.bluez.Battery1` (yüzde değiştirilebilir: [`FakeBluez::set_battery`]).
struct Battery {
    percentage: u8,
}

#[zbus::interface(name = "org.bluez.Battery1")]
impl Battery {
    #[zbus(property)]
    fn percentage(&self) -> u8 {
        self.percentage
    }
}

pub struct FakeBluez {
    pub conn: zbus::blocking::Connection,
    pub agent: AgentSlot,
    pub log: Log,
    rfkill: Arc<AtomicBool>,
    start_failures: StartFailures,
}

/// Eklenecek cihazın tanımı.
#[derive(Debug, Clone, Copy)]
pub struct DeviceSpec {
    pub alias: &'static str,
    /// Cihazın kendi adı (`None`: ad bilinmiyor).
    pub name: Option<&'static str>,
    pub icon: &'static str,
    pub class: u32,
    pub appearance: u16,
    pub paired: bool,
    pub mode: PairMode,
    /// Sinyal gücü (dBm).
    pub rssi: i16,
}

impl DeviceSpec {
    pub fn new(alias: &'static str, paired: bool) -> Self {
        let (icon, class, mode) = ("audio-headset", 0x240404, PairMode::Normal);
        Self { alias, name: Some(alias), icon, class, appearance: 0, paired, mode, rssi: -60 }
    }
}

/// Sahte BlueZ sahnesi (`examples/fake_bluez`: `NUXUT_FAKE_BLUEZ_SCENE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    /// Testlerin temel ağacı: adaptör kapalı + eşleşmiş Kulaklık.
    Basic,
    /// `default`: adaptör açık, beş cihaz (pil, sınıflar, adsız, iptal bekleyen, hep başarısız).
    Demo,
    /// `no-adapter`: BlueZ çalışıyor, adaptör yok.
    NoAdapter,
    /// `off`: Demo, adaptör kapalı.
    Off,
    /// `rfkill`: Demo, adaptör rfkill ile engelli (`PowerState=off-blocked`, açma reddedilir).
    Rfkill,
    /// `empty`: adaptör açık, cihaz yok.
    Empty,
}

impl Scene {
    /// `NUXUT_FAKE_BLUEZ_SCENE` değeri; bilinmeyen değer `None`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "" | "default" => Self::Demo,
            "basic" => Self::Basic,
            "no-adapter" => Self::NoAdapter,
            "off" => Self::Off,
            "rfkill" => Self::Rfkill,
            "empty" => Self::Empty,
            _ => return None,
        })
    }
}

impl FakeBluez {
    /// `demo`: [`Scene::Demo`], değilse [`Scene::Basic`].
    pub fn start(conn: zbus::blocking::Connection, log: &Log, demo: bool) -> Self {
        Self::start_scene(conn, log, if demo { Scene::Demo } else { Scene::Basic })
    }

    /// Kulaklık'ta pil (%80), eşleşmemiş "Klavye" (`dev_2`; ajan varsa onay sorar), eşleşmesi
    /// hep `AuthenticationFailed` dönen "Hoparlör" (`dev_3`), `CancelPairing` bekleyen "Saat"
    /// (`dev_4`; giyilebilir sınıf) ve adı bilinmeyen LE cihaz (`dev_5`) Demo'dadır.
    pub fn start_scene(conn: zbus::blocking::Connection, log: &Log, scene: Scene) -> Self {
        let agent = AgentSlot::default();
        let rfkill = Arc::new(AtomicBool::new(scene == Scene::Rfkill));
        let start_failures: StartFailures = Arc::default();
        conn.object_server().at("/", zbus::fdo::ObjectManager).unwrap();
        conn.object_server().at("/org/bluez", AgentManager { agent: agent.clone(), log: log.clone() }).unwrap();
        if scene != Scene::NoAdapter {
            let adapter = Adapter {
                alias: "nuxut-vm".into(),
                powered: matches!(scene, Scene::Demo | Scene::Empty),
                discovering: Arc::new(AtomicBool::new(false)),
                start_failures: start_failures.clone(),
                discoverable: false,
                discoverable_timeout: 180,
                pairable: true,
                rfkill: rfkill.clone(),
                log: log.clone(),
            };
            conn.object_server().at(HCI0, adapter).unwrap();
        }
        let fake = Self { conn, agent, log: log.clone(), rfkill, start_failures };
        let demo = matches!(scene, Scene::Demo | Scene::Off | Scene::Rfkill);
        if matches!(scene, Scene::Basic) || demo {
            let headset = fake.add_device("dev_1", "Kulaklık", true);
            if demo {
                fake.add_battery(&headset, 80);
            }
        }
        if demo {
            fake.add("dev_2", DeviceSpec { icon: "input-keyboard", class: 0x002540, rssi: -48, ..DeviceSpec::new("Klavye", false) });
            fake.add_failing_device("dev_3", "Hoparlör", PairError::Failed);
            fake.add(
                "dev_4",
                DeviceSpec {
                    icon: "",
                    // Giyilebilir / kol saati (CoD ana sınıf 0x07, alt 0x01) + LE görünüm "watch".
                    class: 0x000704,
                    appearance: 0x00c2,
                    mode: PairMode::WaitCancel,
                    rssi: -75,
                    ..DeviceSpec::new("Saat", false)
                },
            );
            fake.add("dev_5", DeviceSpec { name: None, icon: "", class: 0, rssi: -90, ..DeviceSpec::new("12-34-56-78-9A-BC", false) });
        }
        fake.conn.request_name(BLUEZ).unwrap();
        fake
    }

    /// Cihaz ekler; yolunu döner.
    pub fn add(&self, id: &str, spec: DeviceSpec) -> String {
        let path = format!("{HCI0}/{id}");
        let dev = Device {
            path: path.clone(),
            alias: spec.alias.into(),
            name: spec.name,
            icon: spec.icon,
            class: spec.class,
            appearance: spec.appearance,
            paired: Arc::new(AtomicBool::new(spec.paired)),
            rssi: spec.rssi,
            trusted: spec.paired,
            blocked: false,
            connected: false,
            services_resolved: false,
            mode: spec.mode,
            canceled: Arc::new(AtomicBool::new(false)),
            agent: self.agent.clone(),
            log: self.log.clone(),
        };
        self.conn.object_server().at(path.as_str(), dev).unwrap();
        path
    }

    pub fn add_device(&self, id: &str, alias: &'static str, paired: bool) -> String {
        self.add(id, DeviceSpec::new(alias, paired))
    }

    /// `Pair` hep `error` döner.
    pub fn add_failing_device(&self, id: &str, alias: &'static str, error: PairError) -> String {
        self.add(id, DeviceSpec { mode: PairMode::Error(error), ..DeviceSpec::new(alias, false) })
    }

    /// Cihaza `Battery1` ekler (`InterfacesAdded`).
    pub fn add_battery(&self, device: &str, percentage: u8) {
        self.conn.object_server().at(device, Battery { percentage }).unwrap();
    }

    /// Pil yüzdesini değiştirir (`PropertiesChanged`).
    pub fn set_battery(&self, device: &str, percentage: u8) {
        let iface = self.conn.object_server().interface::<_, Battery>(device).unwrap();
        iface.get_mut().percentage = percentage;
        zbus::block_on(iface.get().percentage_changed(iface.signal_emitter())).unwrap();
    }

    /// rfkill engelini aç/kapa: engelliyken `Powered=true` `org.bluez.Error.Blocked` döner,
    /// `PowerState` `off-blocked` olur. Engellenince adaptör kapanır.
    pub fn set_rfkill(&self, blocked: bool) {
        self.rfkill.store(blocked, Ordering::Relaxed);
        let iface = self.conn.object_server().interface::<_, Adapter>(HCI0).unwrap();
        if blocked {
            iface.get_mut().powered = false;
        }
        let a = iface.get();
        zbus::block_on(a.power_state_changed(iface.signal_emitter())).unwrap();
        zbus::block_on(a.powered_changed(iface.signal_emitter())).unwrap();
    }

    /// Başka bir istemcinin taraması (bizim oturumumuz değil).
    pub fn set_external_discovering(&self, on: bool) {
        let iface = self.conn.object_server().interface::<_, Adapter>(HCI0).unwrap();
        iface.get().discovering.store(on, Ordering::Relaxed);
        zbus::block_on(iface.get().discovering_changed(iface.signal_emitter())).unwrap();
    }
}

impl FakeBluez {
    /// Sıradaki `StartDiscovery` `delay` bekleyip `InProgress` ile başarısız olur (eski bir
    /// tarama hatasının yenisini etkilememesi testi).
    pub fn fail_next_start(&self, delay: std::time::Duration) {
        self.start_failures.lock().unwrap().push_back(delay);
    }
}
