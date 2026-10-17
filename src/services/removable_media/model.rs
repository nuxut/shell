//! Saf UDisks2 modeli: ObjectManager nesnelerinden çıkarılabilir birim listesi.
//!
//! **Birim** = `Filesystem` arayüzü olan bir `Block` nesnesi (bağlanabilir dosya sistemi) ve
//! şu koşulların hepsi:
//! - `HintIgnore` değil (udisks "kullanıcıya gösterme" der; ör. kurtarma bölümleri);
//! - `HintSystem` değil (sistem diski; udisks çıkarılabilir sürücülerde `false` yapar);
//! - sürücüsü (`Block.Drive`) bilinen bir `Drive` nesnesi ve o sürücü `Removable` ya da
//!   `MediaRemovable` ya da `ConnectionBus` `usb`/`sdio`.
//!
//! Durumlar (`docs/design/removable-media.md`):
//! - **Kripto**: kilitli LUKS kabı (`IdUsage = crypto`) `Filesystem` taşımaz → birim değil
//!   (kilit açma istemi v1 dışı). Kilidi açılınca gelen açık metin aygıtı (`CryptoBackingDevice`
//!   dolu) kendi `Drive`'ı boşsa sürücüyü ve `HintSystem`'i arka aygıttan alır; çıkarırken önce
//!   ayrılır, sonra arka aygıt kilitlenir.
//! - **Loop** (`udisksctl loop-setup`, disk görüntüsü): sürücüsü yok (`Drive = /`) → birim değil.
//! - **Bölüm tablosu**: tablo ve uzatılmış bölüm kabı (`Partition.IsContainer`) `Filesystem`
//!   taşımaz → birim değil; her dosya sistemi bölümü ayrı birimdir. udev bölümleri parça parça
//!   getirir (önce tablo, sonra bölümler; `Filesystem` arayüzü yoklamadan sonra ayrı
//!   `InterfacesAdded` ile): birim kararı her değişimde yeniden hesaplanır, bir birim koşulları
//!   *sağladığı anda* "eklendi" sayılır.

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use nuxut::i18n::t;
use zbus::zvariant::Value;

use nuxut_dbus::{ObjectStore, PropMap, prop_bool, prop_i64, prop_str, value_bytes};

pub(crate) const BLOCK_IFACE: &str = "org.freedesktop.UDisks2.Block";
pub(crate) const FILESYSTEM_IFACE: &str = "org.freedesktop.UDisks2.Filesystem";
pub(crate) const DRIVE_IFACE: &str = "org.freedesktop.UDisks2.Drive";
pub(crate) const ENCRYPTED_IFACE: &str = "org.freedesktop.UDisks2.Encrypted";
/// udisks'in ObjectManager yolu.
pub const UDISKS_ROOT: &str = "/org/freedesktop/UDisks2";
/// Blok aygıtı nesnelerinin öneki; birim kimliği bunun ardındaki tek bileşendir (`sdb1`).
pub const BLOCK_PREFIX: &str = "/org/freedesktop/UDisks2/block_devices/";

/// En fazla izlenen blok aygıtı / dosya sistemi / sürücü (dev listeler belleği şişirmesin).
pub const MAX_BLOCKS: usize = 512;
pub const MAX_DRIVES: usize = 128;
/// Kimlik sınırı (udisks blok adları kısa: `sdb1`, `mmcblk0p1`, `dm_2d3`).
pub const MAX_ID: usize = 64;
const MAX_LABEL: usize = 128;
const MAX_MOUNT_POINTS: usize = 16;
const MAX_MOUNT_PATH: usize = 4096;

/// Birimin görünen adının kaynağı (dil bağımsız; metin [`Volume::display_name`] ile).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeName {
    /// Dosya sistemi etiketi (`IdLabel`).
    Label(String),
    /// Sürücü üreticisi + modeli.
    Drive(String),
    /// Ne etiket ne model: boyut (bayt) → "16 GB birim".
    Size(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// Kısa, güvenli kimlik: blok nesnesinin son bileşeni (`sdb1`); `removable open|eject <id>`.
    pub id: String,
    pub name: VolumeName,
    /// Boşsa bağlı değil.
    pub mount_points: Vec<PathBuf>,
    /// Sürücü nesnesi (`/org/freedesktop/UDisks2/drives/…`).
    pub drive: String,
    pub ejectable: bool,
    pub can_power_off: bool,
    /// Kilidi açılmış kripto aygıtıysa arka (kilitli) blok nesnesi.
    pub crypto_backing: Option<String>,
    pub size: u64,
}

impl Volume {
    /// Blok nesnesinin yolu.
    pub fn object_path(&self) -> String {
        format!("{BLOCK_PREFIX}{}", self.id)
    }

    /// Kullanıcıya görünen ad: etiket, yoksa "Üretici Model", yoksa "16 GB birim".
    pub fn display_name(&self) -> String {
        match &self.name {
            VolumeName::Label(s) | VolumeName::Drive(s) => s.clone(),
            VolumeName::Size(b) => t!("shell-media-size-name", size = size_text(*b)),
        }
    }

    pub fn mounted(&self) -> bool {
        !self.mount_points.is_empty()
    }
}

/// Kimlik geçerli mi: 1..=[`MAX_ID`] bayt, yalnızca `[A-Za-z0-9_]` (udisks nesne yolu
/// bileşenlerini bu kümeye kaçışlar; komut satırına ve yola güvenle girer).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Sürücü boyutu ondalık (SI) birimle, satıcıların yazdığı gibi: "16 GB", "1,5 TB", "512 MB".
/// 10'dan küçükse bir ondalık (",0" atılır), değilse tam sayı.
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i + 1 < UNITS.len() {
        v /= 1000.0;
        i += 1;
    }
    let one = (v * 10.0).round() / 10.0;
    let text = if i == 0 || v >= 9.95 || one.fract() == 0.0 {
        crate::services::sysmon::format::fixed(v.round(), 0)
    } else {
        crate::services::sysmon::format::fixed(one, 1)
    };
    format!("{text} {}", UNITS[i])
}

/// Ham nesne ağacı (yalnızca blok, dosya sistemi ve sürücü arayüzleri).
pub(crate) fn new_objects() -> ObjectStore {
    ObjectStore::new("/org/freedesktop/UDisks2/", |iface| match iface {
        BLOCK_IFACE | FILESYSTEM_IFACE => Some(MAX_BLOCKS),
        DRIVE_IFACE => Some(MAX_DRIVES),
        _ => None,
    })
}

/// Ağaçtaki çıkarılabilir birimler (kimlik sırasıyla).
pub(crate) fn volumes(objects: &ObjectStore) -> Vec<Volume> {
    objects.with(BLOCK_IFACE).filter_map(|(path, block)| volume(objects, path, block)).collect()
}

/// `"/"` ya da boş yol: yok.
fn object_ref(props: &PropMap, key: &str) -> Option<String> {
    prop_str(props, key, nuxut_dbus::MAX_PATH).filter(|p| !p.is_empty() && p != "/")
}

/// Bloğun sürücüsü: kendi `Drive`'ı, boşsa (açık metin kripto aygıtı) arka aygıtınki.
fn block_drive(objects: &ObjectStore, block: &PropMap) -> Option<String> {
    object_ref(block, "Drive").or_else(|| {
        let backing = object_ref(block, "CryptoBackingDevice")?;
        object_ref(objects.get(&backing, BLOCK_IFACE)?, "Drive")
    })
}

/// Bir sürücüdeki dosya sistemi (çıkarma için; birim süzgecinden bağımsız).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DriveFilesystem {
    /// Blok nesnesinin yolu.
    pub path: String,
    pub mounted: bool,
    pub crypto_backing: Option<String>,
}

/// `drive`'a ait **tüm** dosya sistemleri — süzgece takılanlar dahil (ör. `HintIgnore`'lu,
/// bağlı bir kurtarma bölümü): biri bağlı kalırsa sürücünün gücü kesilemez (inceleme bulgusu,
/// 2026-10-13).
pub(crate) fn drive_filesystems(objects: &ObjectStore, drive: &str) -> Vec<DriveFilesystem> {
    objects
        .with(FILESYSTEM_IFACE)
        .filter_map(|(path, fs)| {
            let block = objects.get(path, BLOCK_IFACE)?;
            (block_drive(objects, block).as_deref() == Some(drive)).then(|| DriveFilesystem {
                path: path.to_owned(),
                mounted: fs.get("MountPoints").is_some_and(|v| !mount_points(v).is_empty()),
                crypto_backing: object_ref(block, "CryptoBackingDevice"),
            })
        })
        .collect()
}

fn volume(objects: &ObjectStore, path: &str, block: &PropMap) -> Option<Volume> {
    let id = path.strip_prefix(BLOCK_PREFIX).filter(|id| valid_id(id))?;
    let fs = objects.get(path, FILESYSTEM_IFACE)?;
    if prop_bool(block, "HintIgnore").unwrap_or(false) {
        return None;
    }
    // Açık metin aygıtı: sürücü ve sistem ipucu arka aygıttan (dm aygıtında güvenilmez).
    let crypto_backing = object_ref(block, "CryptoBackingDevice");
    let backing = crypto_backing.as_deref().and_then(|b| objects.get(b, BLOCK_IFACE));
    let hint_source = backing.unwrap_or(block);
    if prop_bool(hint_source, "HintSystem").unwrap_or(true) {
        return None;
    }
    let drive_path = block_drive(objects, block)?;
    let drive = objects.get(&drive_path, DRIVE_IFACE)?;
    let bus = prop_str(drive, "ConnectionBus", 32).unwrap_or_default();
    let removable = prop_bool(drive, "Removable").unwrap_or(false)
        || prop_bool(drive, "MediaRemovable").unwrap_or(false)
        || matches!(bus.as_str(), "usb" | "sdio");
    if !removable {
        return None;
    }
    let size = prop_i64(block, "Size").and_then(|s| u64::try_from(s).ok()).unwrap_or(0);
    let name = name(block, drive, size);
    Some(Volume {
        id: id.to_owned(),
        name,
        mount_points: fs.get("MountPoints").map(|v| mount_points(v)).unwrap_or_default(),
        drive: drive_path,
        ejectable: prop_bool(drive, "Ejectable").unwrap_or(false),
        can_power_off: prop_bool(drive, "CanPowerOff").unwrap_or(false),
        crypto_backing,
        size,
    })
}

fn name(block: &PropMap, drive: &PropMap, size: u64) -> VolumeName {
    let trimmed = |s: Option<String>| s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    if let Some(label) = trimmed(prop_str(block, "IdLabel", MAX_LABEL)) {
        return VolumeName::Label(label);
    }
    let vendor = trimmed(prop_str(drive, "Vendor", MAX_LABEL));
    let model = trimmed(prop_str(drive, "Model", MAX_LABEL));
    match (vendor, model) {
        (Some(v), Some(m)) => VolumeName::Drive(format!("{v} {m}")),
        (Some(s), None) | (None, Some(s)) => VolumeName::Drive(s),
        (None, None) => VolumeName::Size(size),
    }
}

/// `MountPoints` (`aay`, her biri NUL ile biten bayt dizisi) → mutlak yollar.
fn mount_points(v: &Value<'_>) -> Vec<PathBuf> {
    let Value::Array(a) = v else { return Vec::new() };
    a.inner()
        .iter()
        .take(MAX_MOUNT_POINTS)
        .filter_map(|e| value_bytes(e, MAX_MOUNT_PATH))
        .filter_map(|mut b| {
            while b.last() == Some(&0) {
                b.pop();
            }
            (b.first() == Some(&b'/') && !b.contains(&0)).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(&b)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use zbus::zvariant::{ObjectPath, OwnedValue};

    use super::*;

    fn props(pairs: Vec<(&str, Value<'static>)>) -> PropMap {
        pairs.into_iter().map(|(k, v)| (k.to_owned(), OwnedValue::try_from(v).unwrap())).collect()
    }

    fn op(p: &'static str) -> Value<'static> {
        Value::ObjectPath(ObjectPath::from_static_str_unchecked(p))
    }

    const DRIVE: &str = "/org/freedesktop/UDisks2/drives/Kingston_DT_1";

    fn usb_drive(bus: &str, removable: bool) -> (String, PropMap) {
        let p = props(vec![
            ("Vendor", Value::from("Kingston")),
            ("Model", Value::from("DataTraveler 3.0")),
            ("ConnectionBus", Value::from(bus.to_owned())),
            ("Removable", Value::from(removable)),
            ("MediaRemovable", Value::from(false)),
            ("Ejectable", Value::from(true)),
            ("CanPowerOff", Value::from(true)),
        ]);
        (DRIVE.to_owned(), p)
    }

    fn block(drive: &'static str, label: &str, system: bool) -> PropMap {
        props(vec![
            ("Drive", op(drive)),
            ("IdLabel", Value::from(label.to_owned())),
            ("HintSystem", Value::from(system)),
            ("HintIgnore", Value::from(false)),
            ("Size", Value::U64(16_008_609_792)),
            ("CryptoBackingDevice", op("/")),
        ])
    }

    fn fs(mounts: &[&str]) -> PropMap {
        let aay: Vec<Vec<u8>> = mounts
            .iter()
            .map(|m| {
                let mut b = m.as_bytes().to_vec();
                b.push(0);
                b
            })
            .collect();
        props(vec![("MountPoints", Value::from(aay))])
    }

    fn store(objs: Vec<(String, Vec<(&str, PropMap)>)>) -> ObjectStore {
        let mut s = new_objects();
        for (path, ifaces) in objs {
            let map: HashMap<String, PropMap> = ifaces.into_iter().map(|(i, p)| (i.to_owned(), p)).collect();
            s.add(&path, map);
        }
        s
    }

    fn blk(id: &str) -> String {
        format!("{BLOCK_PREFIX}{id}")
    }

    #[test]
    fn usb_filesystem_is_a_volume() {
        crate::i18n::use_turkish_for_tests();
        let (dp, d) = usb_drive("usb", false);
        let s = store(vec![
            (dp, vec![(DRIVE_IFACE, d)]),
            (blk("sdb1"), vec![(BLOCK_IFACE, block(DRIVE, "YEDEK", false)), (FILESYSTEM_IFACE, fs(&["/run/media/u/YEDEK"]))]),
            // Bölüm tablosu (dosya sistemi yok): birim değil.
            (blk("sdb"), vec![(BLOCK_IFACE, block(DRIVE, "", false))]),
        ]);
        let v = volumes(&s);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "sdb1");
        assert_eq!(v[0].display_name(), "YEDEK");
        assert_eq!(v[0].mount_points, vec![PathBuf::from("/run/media/u/YEDEK")]);
        assert!(v[0].mounted() && v[0].can_power_off && v[0].ejectable);
        assert_eq!(v[0].object_path(), blk("sdb1"));
    }

    #[test]
    fn filter_rules() {
        // Sistem ipucu, yok say ipucu, sürücüsüz (loop), sabit SATA disk: birim değil.
        let (dp, d) = usb_drive("usb", false);
        let mut ignored = block(DRIVE, "A", false);
        ignored.insert("HintIgnore".into(), OwnedValue::from(true));
        let (_, sata) = usb_drive("", false);
        let s = store(vec![
            (dp, vec![(DRIVE_IFACE, d)]),
            ("/org/freedesktop/UDisks2/drives/Sata".into(), vec![(DRIVE_IFACE, sata)]),
            (blk("sdb1"), vec![(BLOCK_IFACE, block(DRIVE, "S", true)), (FILESYSTEM_IFACE, fs(&[]))]),
            (blk("sdb2"), vec![(BLOCK_IFACE, ignored), (FILESYSTEM_IFACE, fs(&[]))]),
            (blk("loop0"), vec![(BLOCK_IFACE, block("/", "ISO", false)), (FILESYSTEM_IFACE, fs(&[]))]),
            (
                blk("sda1"),
                vec![
                    (BLOCK_IFACE, block("/org/freedesktop/UDisks2/drives/Sata", "ROOT", false)),
                    (FILESYSTEM_IFACE, fs(&["/"])),
                ],
            ),
        ]);
        assert!(volumes(&s).is_empty(), "{:?}", volumes(&s));

        // Removable / MediaRemovable / sdio sürücüler sayılır (USB olmasa da).
        for (bus, removable, media) in [("", true, false), ("", false, true), ("sdio", false, false)] {
            let (dp, mut d) = usb_drive(bus, removable);
            d.insert("MediaRemovable".into(), OwnedValue::from(media));
            let s = store(vec![
                (dp, vec![(DRIVE_IFACE, d)]),
                (blk("mmcblk0p1"), vec![(BLOCK_IFACE, block(DRIVE, "SD", false)), (FILESYSTEM_IFACE, fs(&[]))]),
            ]);
            assert_eq!(volumes(&s).len(), 1, "bus={bus} removable={removable} media={media}");
        }
    }

    #[test]
    fn crypto_cleartext_inherits_drive_from_backing() {
        let (dp, d) = usb_drive("usb", false);
        let mut clear = block("/", "GIZLI", true);
        clear.insert("CryptoBackingDevice".into(), OwnedValue::try_from(op("/org/freedesktop/UDisks2/block_devices/sdc1")).unwrap());
        let s = store(vec![
            (dp, vec![(DRIVE_IFACE, d)]),
            // Kilitli kap: dosya sistemi yok → birim değil.
            (blk("sdc1"), vec![(BLOCK_IFACE, block(DRIVE, "", false))]),
            (blk("dm_2d0"), vec![(BLOCK_IFACE, clear), (FILESYSTEM_IFACE, fs(&[]))]),
        ]);
        let v = volumes(&s);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "dm_2d0");
        assert_eq!(v[0].drive, DRIVE);
        assert_eq!(v[0].crypto_backing.as_deref(), Some("/org/freedesktop/UDisks2/block_devices/sdc1"));
    }

    #[test]
    fn names_fall_back_to_drive_then_size() {
        crate::i18n::use_turkish_for_tests();
        let (dp, d) = usb_drive("usb", false);
        let s = store(vec![
            (dp.clone(), vec![(DRIVE_IFACE, d)]),
            (blk("sdb1"), vec![(BLOCK_IFACE, block(DRIVE, "  ", false)), (FILESYSTEM_IFACE, fs(&[]))]),
        ]);
        assert_eq!(volumes(&s)[0].display_name(), "Kingston DataTraveler 3.0");

        let (_, mut bare) = usb_drive("usb", false);
        bare.insert("Vendor".into(), OwnedValue::try_from(Value::from("")).unwrap());
        bare.insert("Model".into(), OwnedValue::try_from(Value::from(" ")).unwrap());
        let s = store(vec![
            (dp, vec![(DRIVE_IFACE, bare)]),
            (blk("sdb1"), vec![(BLOCK_IFACE, block(DRIVE, "", false)), (FILESYSTEM_IFACE, fs(&[]))]),
        ]);
        assert_eq!(volumes(&s)[0].display_name(), "16 GB birim");
    }

    #[test]
    fn size_and_ids() {
        crate::i18n::use_turkish_for_tests();
        assert_eq!(size_text(16_008_609_792), "16 GB");
        assert_eq!(size_text(1_500_000_000_000), "1,5 TB");
        assert_eq!(size_text(2_000_000_000), "2 GB");
        assert_eq!(size_text(512_000_000), "512 MB");
        assert_eq!(size_text(0), "0 B");
        assert!(valid_id("sdb1") && valid_id("mmcblk0p1") && valid_id("dm_2d0"));
        for bad in ["", "../x", "sdb 1", "a/b", "sdb1;rm", &"a".repeat(MAX_ID + 1)] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn mount_points_are_bounded_absolute_paths() {
        let raw: Vec<Vec<u8>> = vec![b"/run/media/u/A\0".to_vec(), b"relative\0".to_vec(), b"/x\0y\0".to_vec(), b"/b".to_vec()];
        assert_eq!(mount_points(&Value::from(raw)), vec![PathBuf::from("/run/media/u/A"), PathBuf::from("/b")]);
        assert!(mount_points(&Value::from("x")).is_empty());
    }

    #[test]
    fn drive_filesystems_include_filtered_and_cleartext() {
        let (dp, d) = usb_drive("usb", false);
        let mut hidden = block(DRIVE, "KURTARMA", false);
        hidden.insert("HintIgnore".into(), OwnedValue::from(true));
        let mut clear = block("/", "GIZLI", true);
        clear.insert("CryptoBackingDevice".into(), OwnedValue::try_from(op("/org/freedesktop/UDisks2/block_devices/sdb3")).unwrap());
        let s = store(vec![
            (dp, vec![(DRIVE_IFACE, d)]),
            (blk("sdb1"), vec![(BLOCK_IFACE, block(DRIVE, "A", false)), (FILESYSTEM_IFACE, fs(&[]))]),
            (blk("sdb2"), vec![(BLOCK_IFACE, hidden), (FILESYSTEM_IFACE, fs(&["/run/media/u/K"]))]),
            (blk("sdb3"), vec![(BLOCK_IFACE, block(DRIVE, "", false))]),
            (blk("dm_2d0"), vec![(BLOCK_IFACE, clear), (FILESYSTEM_IFACE, fs(&["/run/media/u/G"]))]),
            // Başka sürücü.
            (blk("sdc1"), vec![(BLOCK_IFACE, block("/org/freedesktop/UDisks2/drives/Other", "B", false)), (FILESYSTEM_IFACE, fs(&["/x"]))]),
        ]);
        let got = drive_filesystems(&s, DRIVE);
        let summary: Vec<(String, bool, Option<String>)> =
            got.into_iter().map(|f| (f.path.trim_start_matches(BLOCK_PREFIX).to_owned(), f.mounted, f.crypto_backing)).collect();
        assert_eq!(
            summary,
            vec![
                ("dm_2d0".into(), true, Some(blk("sdb3"))),
                ("sdb1".into(), false, None),
                ("sdb2".into(), true, None),
            ]
        );
    }
}
