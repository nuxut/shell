//! Sahte `org.freedesktop.UDisks2` (yalnızca özel/test bus'ında): ObjectManager, `Block`,
//! `Filesystem`, `Drive`, `Encrypted` arayüzleri. Hiçbir gerçek aygıta dokunmaz: `Mount` yalnızca
//! `MountPoints` özelliğini değiştirir, `Unmount`/`PowerOff`/`Eject`/`Lock` yalnızca çağrı
//! günlüğüne yazar. `tests/it/removable_media.rs` ve canlı test örneği
//! (`examples/fake_udisks.rs`, `#[path]` ile) paylaşır.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use zbus::blocking;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

pub const UDISKS: &str = "org.freedesktop.UDisks2";
pub const ROOT: &str = "/org/freedesktop/UDisks2";
pub const BLOCKS: &str = "/org/freedesktop/UDisks2/block_devices/";
pub const DRIVES: &str = "/org/freedesktop/UDisks2/drives/";

/// Çağrı günlüğü (`"Mount sdb1"`, `"Unmount sdb1"`, `"PowerOff Kingston"`…) ve meşgul anahtarı.
#[derive(Clone, Default)]
pub struct Shared {
    pub calls: Arc<Mutex<Vec<String>>>,
    /// `true`: `Unmount` `DeviceBusy` döner.
    pub busy: Arc<AtomicBool>,
    /// `true`: `PowerOff`/`Eject` `DeviceBusy` döner (sürücüyü başka süreç tutuyor).
    pub drive_busy: Arc<AtomicBool>,
    /// `Unmount` gecikmesi (ms; yavaş USB önbellek boşaltması).
    pub unmount_delay_ms: Arc<AtomicU64>,
}

impl Shared {
    fn log(&self, s: String) {
        eprintln!("ÇAĞRI: {s}");
        self.calls.lock().unwrap().push(s);
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.UDisks2.Error")]
pub enum UdisksError {
    #[zbus(error)]
    ZBus(zbus::Error),
    DeviceBusy(String),
    NotMounted(String),
}

fn path(p: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p.to_owned()).unwrap()
}

pub struct Block {
    drive: OwnedObjectPath,
    label: String,
    system: bool,
    ignore: bool,
    size: u64,
    backing: OwnedObjectPath,
}

#[zbus::interface(name = "org.freedesktop.UDisks2.Block")]
impl Block {
    #[zbus(property)]
    fn drive(&self) -> OwnedObjectPath {
        self.drive.clone()
    }
    #[zbus(property)]
    fn id_label(&self) -> String {
        self.label.clone()
    }
    #[zbus(property)]
    fn hint_system(&self) -> bool {
        self.system
    }
    #[zbus(property)]
    fn hint_ignore(&self) -> bool {
        self.ignore
    }
    #[zbus(property)]
    fn size(&self) -> u64 {
        self.size
    }
    #[zbus(property)]
    fn crypto_backing_device(&self) -> OwnedObjectPath {
        self.backing.clone()
    }
}

pub struct Filesystem {
    id: String,
    mount_points: Vec<Vec<u8>>,
    shared: Shared,
}

fn nul(s: &str) -> Vec<u8> {
    let mut b = s.as_bytes().to_vec();
    b.push(0);
    b
}

#[zbus::interface(name = "org.freedesktop.UDisks2.Filesystem")]
impl Filesystem {
    #[zbus(property)]
    fn mount_points(&self) -> Vec<Vec<u8>> {
        self.mount_points.clone()
    }

    async fn mount(
        &mut self,
        _options: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<String, UdisksError> {
        self.shared.log(format!("Mount {}", self.id));
        let point = format!("/run/media/nuxut-test/{}", self.id);
        self.mount_points = vec![nul(&point)];
        self.mount_points_changed(&emitter).await?;
        Ok(point)
    }

    async fn unmount(
        &mut self,
        _options: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), UdisksError> {
        self.shared.log(format!("Unmount {}", self.id));
        let delay = self.shared.unmount_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            async_io::Timer::after(std::time::Duration::from_millis(delay)).await;
        }
        if self.shared.busy.load(Ordering::SeqCst) {
            return Err(UdisksError::DeviceBusy("target is busy".into()));
        }
        if self.mount_points.is_empty() {
            return Err(UdisksError::NotMounted("not mounted".into()));
        }
        self.mount_points.clear();
        self.mount_points_changed(&emitter).await?;
        Ok(())
    }
}

pub struct Drive {
    name: String,
    vendor: String,
    model: String,
    bus: String,
    removable: bool,
    shared: Shared,
}

#[zbus::interface(name = "org.freedesktop.UDisks2.Drive")]
impl Drive {
    #[zbus(property)]
    fn vendor(&self) -> String {
        self.vendor.clone()
    }
    #[zbus(property)]
    fn model(&self) -> String {
        self.model.clone()
    }
    #[zbus(property)]
    fn connection_bus(&self) -> String {
        self.bus.clone()
    }
    #[zbus(property)]
    fn removable(&self) -> bool {
        self.removable
    }
    #[zbus(property)]
    fn media_removable(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn ejectable(&self) -> bool {
        self.removable
    }
    #[zbus(property)]
    fn can_power_off(&self) -> bool {
        self.removable
    }
    fn eject(&self, _options: HashMap<String, OwnedValue>) -> Result<(), UdisksError> {
        self.shared.log(format!("Eject {}", self.name));
        self.drive_result()
    }
    fn power_off(&self, _options: HashMap<String, OwnedValue>) -> Result<(), UdisksError> {
        self.shared.log(format!("PowerOff {}", self.name));
        self.drive_result()
    }
}

impl Drive {
    fn drive_result(&self) -> Result<(), UdisksError> {
        if self.shared.drive_busy.load(Ordering::SeqCst) {
            return Err(UdisksError::DeviceBusy("device is in use".into()));
        }
        Ok(())
    }
}

pub struct Encrypted {
    id: String,
    shared: Shared,
}

#[zbus::interface(name = "org.freedesktop.UDisks2.Encrypted")]
impl Encrypted {
    fn lock(&self, _options: HashMap<String, OwnedValue>) {
        self.shared.log(format!("Lock {}", self.id));
    }
}

/// Bus'ta `org.freedesktop.UDisks2` adını tutan sahte udisksd.
pub struct FakeUdisks {
    pub conn: blocking::Connection,
    pub shared: Shared,
}

impl FakeUdisks {
    /// Adı hemen alır (nesneler sonra eklenir).
    pub fn start(address: &str) -> Self {
        let fake = Self::unnamed(address);
        fake.claim();
        fake
    }

    /// Adı almadan: gerçek udisksd gibi önce nesneler (coldplug), sonra [`claim`](Self::claim).
    pub fn unnamed(address: &str) -> Self {
        let conn = blocking::connection::Builder::address(address)
            .unwrap()
            .serve_at(ROOT, zbus::fdo::ObjectManager)
            .unwrap()
            .build()
            .unwrap();
        Self { conn, shared: Shared::default() }
    }

    pub fn claim(&self) {
        self.conn.request_name(UDISKS).unwrap();
    }

    /// Sürücü: `usb = true` ise USB, çıkarılabilir, güç kesilebilir; değilse sabit SATA disk.
    pub fn add_drive(&self, name: &str, vendor: &str, model: &str, usb: bool) {
        let d = Drive {
            name: name.to_owned(),
            vendor: vendor.to_owned(),
            model: model.to_owned(),
            bus: if usb { "usb" } else { "" }.to_owned(),
            removable: usb,
            shared: self.shared.clone(),
        };
        self.conn.object_server().at(format!("{DRIVES}{name}"), d).unwrap();
    }

    /// Dosya sistemsiz blok (bölüm tablosu ya da kilitli kripto kabı).
    pub fn add_block(&self, id: &str, drive: &str, system: bool) {
        self.conn.object_server().at(format!("{BLOCKS}{id}"), self.block(drive, "", system, "/")).unwrap();
    }

    /// Kilitli kripto kabı (`Encrypted` arayüzlü, dosya sistemsiz blok).
    pub fn add_crypto(&self, id: &str, drive: &str) {
        self.add_block(id, drive, false);
        let e = Encrypted { id: id.to_owned(), shared: self.shared.clone() };
        self.conn.object_server().at(format!("{BLOCKS}{id}"), e).unwrap();
    }

    fn block(&self, drive: &str, label: &str, system: bool, backing: &str) -> Block {
        let drive = if drive.is_empty() { "/".to_owned() } else { format!("{DRIVES}{drive}") };
        let backing = if backing == "/" { "/".to_owned() } else { format!("{BLOCKS}{backing}") };
        Block { drive: path(&drive), label: label.to_owned(), system, ignore: false, size: 16_008_609_792, backing: path(&backing) }
    }

    /// Dosya sistemli blok (bağlı değil). `backing`: kripto arka bloğu kimliği ya da `/`.
    pub fn add_volume(&self, id: &str, drive: &str, label: &str, system: bool, backing: &str) {
        let p = format!("{BLOCKS}{id}");
        let server = self.conn.object_server();
        server.at(p.clone(), self.block(drive, label, system, backing)).unwrap();
        let fs = Filesystem { id: id.to_owned(), mount_points: Vec::new(), shared: self.shared.clone() };
        server.at(p, fs).unwrap();
    }

    /// Bağlı (bağlama noktalı) birim.
    pub fn add_mounted_volume(&self, id: &str, drive: &str, label: &str) {
        let p = format!("{BLOCKS}{id}");
        let server = self.conn.object_server();
        server.at(p.clone(), self.block(drive, label, false, "/")).unwrap();
        let fs = Filesystem {
            id: id.to_owned(),
            mount_points: vec![nul(&format!("/run/media/nuxut-test/{id}"))],
            shared: self.shared.clone(),
        };
        server.at(p, fs).unwrap();
    }

    /// Birim süzgecine takılan (`HintIgnore`) ama bağlı dosya sistemi (ör. kurtarma bölümü).
    pub fn add_hidden_mounted_volume(&self, id: &str, drive: &str) {
        let p = format!("{BLOCKS}{id}");
        let server = self.conn.object_server();
        server.at(p.clone(), Block { ignore: true, ..self.block(drive, "GIZLI", false, "/") }).unwrap();
        let fs = Filesystem {
            id: id.to_owned(),
            mount_points: vec![nul(&format!("/run/media/nuxut-test/{id}"))],
            shared: self.shared.clone(),
        };
        server.at(p, fs).unwrap();
    }

    pub fn remove_volume(&self, id: &str) {
        let p = format!("{BLOCKS}{id}");
        let server = self.conn.object_server();
        let _ = server.remove::<Filesystem, _>(p.as_str());
        let _ = server.remove::<Block, _>(p.as_str());
    }

    pub fn remove_drive(&self, name: &str) {
        let _ = self.conn.object_server().remove::<Drive, _>(format!("{DRIVES}{name}"));
    }
}
