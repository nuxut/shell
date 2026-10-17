//! Çıkarılabilir medya izleyicisi: sahte `org.freedesktop.UDisks2` ile özel bus'ta uçtan uca.
//! Kullanıcının sistem bus'ına ve aygıtlarına dokunulmaz (gerçek udisks'e `Mount`/`Unmount`/
//! `PowerOff` gitmez; sahte servis yalnızca özellik değiştirir ve çağrıyı günlüğe yazar).
//!
//! Kapsam: açılışta takılı olan birim bildirilmez (taban çizgisi), sonradan takılan bir kez
//! bildirilir; sistem diski, bölüm tablosu ve kilitli kripto kabı birim değil; sökülen birim
//! `Removed`; dinleyici udisks'ten önce başlarsa udisks gelince taban çizgisi; Aç bağlar, Çıkar
//! ayırır + güç keser (sıra), meşgulse `Busy` ve güç kesilmez; kripto açık metni çıkarılınca arka
//! aygıt kilitlenir; yalnız izleme kipi hiçbir yöntem çağırmaz; bilinmeyen kimlik; sahte sinyal
//! reddi; temiz kapanış.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use nuxut_shell::services::removable_media::{MediaAction, MediaError, MediaEvent, MediaOptions, MediaService};
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::support::udisks::FakeUdisks;
use crate::support::{Events, PrivateBus, channel};

fn options(bus: &PrivateBus) -> MediaOptions {
    MediaOptions {
        bus_address: Some(bus.address.clone()),
        call_timeout: Duration::from_millis(500),
        action_timeout: Duration::from_secs(3),
        max_backoff: Duration::from_millis(200),
        activate: false,
        actions: true,
    }
}

fn start(options: MediaOptions) -> (MediaService, Events<MediaEvent>) {
    let (sink, events) = channel();
    (MediaService::spawn(options, sink).unwrap(), events)
}

/// Bir sistem diski + açılışta takılı bir USB bellek. Gerçek udisksd gibi ad, nesneler
/// (coldplug) hazır olduktan sonra alınır.
fn fake_with_baseline(bus: &PrivateBus) -> FakeUdisks {
    let fake = FakeUdisks::unnamed(&bus.address);
    fake.add_drive("Sata", "ATA", "SSD", false);
    fake.add_block("sda", "Sata", true);
    fake.add_volume("sda1", "Sata", "ROOT", true, "/");
    fake.add_drive("Kingston", "Kingston", "DataTraveler", true);
    fake.add_block("sdb", "Kingston", false);
    fake.add_volume("sdb1", "Kingston", "YEDEK", false, "/");
    fake.claim();
    fake
}

fn ids(e: &MediaEvent) -> Option<Vec<String>> {
    match e {
        MediaEvent::Volumes { available: true, volumes } => Some(volumes.iter().map(|v| v.id.clone()).collect()),
        _ => None,
    }
}

/// Aç: `Opened` ve bağlama noktalı liste (sırası belirsiz: özellik sinyali yöntem cevabından
/// önce gelir) birlikte beklenir.
fn wait_opened(events: &Events<MediaEvent>) -> MediaEvent {
    let (mut opened, mut mounted) = (None, false);
    events.wait_for("açıldı ve bağlı", |e| {
        match e {
            MediaEvent::Opened { .. } => opened = Some(e.clone()),
            MediaEvent::Volumes { volumes, .. } if volumes.iter().any(|v| v.mounted()) => mounted = true,
            _ => {}
        }
        opened.is_some() && mounted
    });
    opened.unwrap()
}

fn wait_volumes(events: &Events<MediaEvent>, want: &[&str]) {
    events.wait_for(&format!("birimler {want:?}"), |e| ids(e).is_some_and(|v| v == want));
}

#[test]
fn baseline_is_silent_and_hotplug_is_reported_once() {
    let Some(bus) = PrivateBus::start() else { return };
    let fake = fake_with_baseline(&bus);
    let (service, events) = start(options(&bus));
    wait_volumes(&events, &["sdb1"]);

    // Sonradan takılan bellek: önce sürücü ve bölüm tablosu, sonra (yoklamadan sonra) bölüm.
    fake.add_drive("SanDisk", "SanDisk", "Ultra", true);
    fake.add_block("sdc", "SanDisk", false);
    fake.add_volume("sdc1", "SanDisk", "", false, "/");
    let added = events.wait_for("sdc1 takıldı", |e| matches!(e, MediaEvent::Added(_)));
    let MediaEvent::Added(v) = added else { unreachable!() };
    assert_eq!(v.id, "sdc1");
    assert_eq!(v.name, nuxut_shell::services::removable_media::VolumeName::Drive("SanDisk Ultra".into()));
    let rest = events.collect(Duration::from_millis(300));
    assert!(!rest.iter().any(|e| matches!(e, MediaEvent::Added(_))), "ikinci Added: {rest:?}");

    // Kilitli kripto kabı birim değil.
    fake.add_crypto("sdd1", "SanDisk");
    assert!(!events.collect(Duration::from_millis(300)).iter().any(|e| matches!(e, MediaEvent::Added(_))));

    fake.remove_volume("sdc1");
    events.wait_for("sdc1 söküldü", |e| *e == MediaEvent::Removed { id: "sdc1".into() });

    let t = std::time::Instant::now();
    drop(service);
    assert!(t.elapsed() < crate::support::PROMPT, "temiz ve hızlı kapanış: {:?}", t.elapsed());
}

#[test]
fn udisks_appearing_later_is_a_new_baseline() {
    let Some(bus) = PrivateBus::start() else { return };
    let (_service, events) = start(options(&bus));
    events.wait_for("udisks yok", |e| matches!(e, MediaEvent::Volumes { available: false, .. }));
    // udisks sonradan gelir (etkinleştirme): o an takılı olanlar bildirilmez.
    let fake = fake_with_baseline(&bus);
    wait_volumes(&events, &["sdb1"]);
    assert!(!events.collect(Duration::from_millis(300)).iter().any(|e| matches!(e, MediaEvent::Added(_))));
    fake.add_drive("SanDisk", "SanDisk", "Ultra", true);
    fake.add_volume("sdc1", "SanDisk", "FOTO", false, "/");
    events.wait_for("sdc1 takıldı", |e| matches!(e, MediaEvent::Added(v) if v.id == "sdc1"));
}

#[test]
fn open_mounts_and_eject_unmounts_then_powers_off() {
    let Some(bus) = PrivateBus::start() else { return };
    let fake = fake_with_baseline(&bus);
    let (service, events) = start(options(&bus));
    wait_volumes(&events, &["sdb1"]);

    assert!(service.open("sdb1"));
    let opened = wait_opened(&events);
    assert_eq!(opened, MediaEvent::Opened { id: "sdb1".into(), mount_point: PathBuf::from("/run/media/nuxut-test/sdb1") });
    // İkinci "Aç" yeniden bağlamaz.
    assert!(service.open("sdb1"));
    events.wait_for("yine açıldı", |e| matches!(e, MediaEvent::Opened { .. }));
    assert_eq!(fake.shared.calls(), ["Mount sdb1"]);

    assert!(service.eject("sdb1"));
    let done = events.wait_for("çıkarıldı", |e| matches!(e, MediaEvent::Ejected { .. } | MediaEvent::Failed { .. }));
    assert_eq!(done, MediaEvent::Ejected { id: "sdb1".into(), name: "YEDEK".into() });
    assert_eq!(fake.shared.calls(), ["Mount sdb1", "Unmount sdb1", "PowerOff Kingston"]);
}

#[test]
fn busy_unmount_fails_without_power_off() {
    let Some(bus) = PrivateBus::start() else { return };
    let fake = FakeUdisks::start(&bus.address);
    fake.add_drive("Kingston", "Kingston", "DataTraveler", true);
    fake.add_mounted_volume("sdb1", "Kingston", "YEDEK");
    fake.shared.busy.store(true, std::sync::atomic::Ordering::SeqCst);
    let (service, events) = start(options(&bus));
    wait_volumes(&events, &["sdb1"]);
    assert!(service.eject("sdb1"));
    let failed = events.wait_for("meşgul", |e| matches!(e, MediaEvent::Failed { .. } | MediaEvent::Ejected { .. }));
    assert_eq!(
        failed,
        MediaEvent::Failed { id: "sdb1".into(), name: "YEDEK".into(), action: MediaAction::Eject, error: MediaError::Busy }
    );
    assert_eq!(fake.shared.calls(), ["Unmount sdb1"]);
}

#[test]
fn eject_of_unlocked_crypto_locks_backing_device() {
    let Some(bus) = PrivateBus::start() else { return };
    let fake = FakeUdisks::start(&bus.address);
    fake.add_drive("Kingston", "Kingston", "DataTraveler", true);
    fake.add_crypto("sdb1", "Kingston");
    let (service, events) = start(options(&bus));
    events.wait_for("udisks", |e| matches!(e, MediaEvent::Volumes { available: true, .. }));
    // Kilidi açıldı: açık metin aygıtı sürücüsünü arka aygıttan alır.
    fake.add_volume("dm_2d0", "", "GIZLI", true, "sdb1");
    events.wait_for("açık metin takıldı", |e| matches!(e, MediaEvent::Added(v) if v.id == "dm_2d0"));
    assert!(service.open("dm_2d0"));
    wait_opened(&events);
    assert!(service.eject("dm_2d0"));
    events.wait_for("çıkarıldı", |e| matches!(e, MediaEvent::Ejected { .. }));
    assert_eq!(fake.shared.calls(), ["Mount dm_2d0", "Unmount dm_2d0", "Lock sdb1", "PowerOff Kingston"]);
}

#[test]
fn listen_only_calls_nothing_and_unknown_ids_fail() {
    let Some(bus) = PrivateBus::start() else { return };
    let fake = fake_with_baseline(&bus);
    let (service, events) = start(MediaOptions { actions: false, ..options(&bus) });
    wait_volumes(&events, &["sdb1"]);
    assert!(service.open("sdb1") && service.eject("sdb1"));
    for action in [MediaAction::Open, MediaAction::Eject] {
        events.wait_for("devre dışı", |e| {
            matches!(e, MediaEvent::Failed { action: a, error: MediaError::Disabled, .. } if *a == action)
        });
    }
    assert!(fake.shared.calls().is_empty());

    let (service, events) = start(options(&bus));
    wait_volumes(&events, &["sdb1"]);
    assert!(service.eject("sdz9"));
    events.wait_for("bilinmeyen", |e| matches!(e, MediaEvent::Failed { error: MediaError::Unknown, .. }));
    assert!(fake.shared.calls().is_empty());
}

#[test]
fn spoofed_interfaces_added_is_ignored() {
    let Some(bus) = PrivateBus::start() else { return };
    let _fake = fake_with_baseline(&bus);
    let (_service, events) = start(options(&bus));
    wait_volumes(&events, &["sdb1"]);
    let attacker = bus.connect();
    let names: Vec<String> = attacker
        .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    let drive = OwnedObjectPath::try_from("/org/freedesktop/UDisks2/drives/Kingston").unwrap();
    let block: HashMap<&str, Value<'_>> = HashMap::from([
        ("Drive", Value::from(drive)),
        ("IdLabel", Value::from("SAHTE")),
        ("HintSystem", Value::from(false)),
    ]);
    let fs: HashMap<&str, Value<'_>> = HashMap::from([("MountPoints", Value::from(Vec::<Vec<u8>>::new()))]);
    let ifaces = HashMap::from([("org.freedesktop.UDisks2.Block", block), ("org.freedesktop.UDisks2.Filesystem", fs)]);
    let path = OwnedObjectPath::try_from("/org/freedesktop/UDisks2/block_devices/sdx1").unwrap();
    for name in names.iter().filter(|n| n.starts_with(':')) {
        attacker
            .emit_signal(Some(name.as_str()), "/org/freedesktop/UDisks2", "org.freedesktop.DBus.ObjectManager", "InterfacesAdded", &(&path, &ifaces))
            .unwrap();
    }
    let spoofed = events.collect(Duration::from_millis(400)).into_iter().any(|e| matches!(e, MediaEvent::Added(_)));
    assert!(!spoofed, "sahte sinyal kabul edildi");
}

#[test]
fn eject_unmounts_hidden_filesystems_on_the_same_drive() {
    let Some(bus) = PrivateBus::start() else { return };
    let fake = FakeUdisks::start(&bus.address);
    fake.add_drive("Kingston", "Kingston", "DataTraveler", true);
    fake.add_mounted_volume("sdb1", "Kingston", "YEDEK");
    // Süzgece takılan ama bağlı bölüm: listede yok, yine de ayrılmalı.
    fake.add_hidden_mounted_volume("sdb2", "Kingston");
    // Başka sürücü: dokunulmaz.
    fake.add_drive("SanDisk", "SanDisk", "Ultra", true);
    fake.add_mounted_volume("sdc1", "SanDisk", "FOTO");
    let (service, events) = start(options(&bus));
    wait_volumes(&events, &["sdb1", "sdc1"]);
    assert!(service.eject("sdb1"));
    events.wait_for("çıkarıldı", |e| matches!(e, MediaEvent::Ejected { .. }));
    assert_eq!(fake.shared.calls(), ["Unmount sdb1", "Unmount sdb2", "PowerOff Kingston"]);
}

#[test]
fn busy_power_off_is_a_failure_not_safe_to_remove() {
    let Some(bus) = PrivateBus::start() else { return };
    let fake = FakeUdisks::start(&bus.address);
    fake.add_drive("Kingston", "Kingston", "DataTraveler", true);
    fake.add_mounted_volume("sdb1", "Kingston", "YEDEK");
    fake.shared.drive_busy.store(true, std::sync::atomic::Ordering::SeqCst);
    let (service, events) = start(options(&bus));
    wait_volumes(&events, &["sdb1"]);
    assert!(service.eject("sdb1"));
    let done = events.wait_for("sonuç", |e| matches!(e, MediaEvent::Failed { .. } | MediaEvent::Ejected { .. }));
    assert_eq!(
        done,
        MediaEvent::Failed { id: "sdb1".into(), name: "YEDEK".into(), action: MediaAction::Eject, error: MediaError::Busy }
    );
    assert_eq!(fake.shared.calls(), ["Unmount sdb1", "PowerOff Kingston"]);
}

#[test]
fn slow_unmount_has_no_timeout_and_second_eject_is_ignored() {
    let Some(bus) = PrivateBus::start() else { return };
    let fake = FakeUdisks::start(&bus.address);
    fake.add_drive("Kingston", "Kingston", "DataTraveler", true);
    fake.add_mounted_volume("sdb1", "Kingston", "YEDEK");
    // call_timeout (500 ms) ve action_timeout'tan (1 sn) uzun önbellek boşaltma.
    fake.shared.unmount_delay_ms.store(1500, std::sync::atomic::Ordering::SeqCst);
    let (service, events) = start(MediaOptions { action_timeout: Duration::from_secs(1), ..options(&bus) });
    wait_volumes(&events, &["sdb1"]);
    // Düğme + msg: aynı sürücüye iki istek; ikincisi yok sayılır.
    assert!(service.eject("sdb1") && service.eject("sdb1"));
    let done = events.wait_for("çıkarıldı", |e| matches!(e, MediaEvent::Ejected { .. } | MediaEvent::Failed { .. }));
    assert_eq!(done, MediaEvent::Ejected { id: "sdb1".into(), name: "YEDEK".into() });
    let rest = events.collect(Duration::from_millis(400));
    assert!(!rest.iter().any(|e| matches!(e, MediaEvent::Ejected { .. } | MediaEvent::Failed { .. })), "{rest:?}");
    assert_eq!(fake.shared.calls(), ["Unmount sdb1", "PowerOff Kingston"]);
    // Bitti: aynı sürücüye yeni istek yine kabul edilir (bağlı birim kalmadığından yalnız güç kesme).
    assert!(service.eject("sdb1"));
    events.wait_for("yeniden çıkarıldı", |e| matches!(e, MediaEvent::Ejected { .. }));
}
