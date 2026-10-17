//! Sahte `org.freedesktop.UDisks2` (yalnızca izole test ortamında canlı test için): bir sistem
//! diski ve açılışta takılı olmayan bir USB bellek. `--plug-after <sn>` sonra belleği "takar"
//! (bildirim çıkmalı); `--busy` ile `Unmount` `DeviceBusy` döner. Hiçbir gerçek aygıta dokunmaz:
//! `Mount` yalnızca özelliği değiştirir, diğer yöntemler stderr'e `ÇAĞRI: …` yazar.
//!
//! ```sh
//! dbus-run-session -- betik.sh            # özel oturum bus'ı
//! cargo run --example fake_udisks &       # o bus'ta UDisks2 adını alır
//! DBUS_SYSTEM_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" nuxut-shell
//! ```
//!
//! Güvenlik: yalnızca `DBUS_SESSION_BUS_ADDRESS` bus'ına bağlanır (sistem bus'ında UDisks2 adını
//! gerçek udisksd tutar); kullanıcının oturum bus'ıysa (`$XDG_RUNTIME_DIR/bus`) çalışmaz.

#[path = "../tests/support/udisks.rs"]
mod udisks;

use std::time::Duration;

fn main() {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    if address.is_empty() || (!runtime.is_empty() && address.contains(&format!("{runtime}/bus"))) {
        eprintln!("fake_udisks: yalnızca dbus-run-session'ın özel bus'ında çalışır");
        std::process::exit(2);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let plug_after = args
        .iter()
        .position(|a| a == "--plug-after")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3);
    let fake = udisks::FakeUdisks::unnamed(&address);
    if args.iter().any(|a| a == "--busy") {
        fake.shared.busy.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fake.add_drive("Sata", "ATA", "SSD", false);
    fake.add_block("sda", "Sata", true);
    fake.add_volume("sda1", "Sata", "ROOT", true, "/");
    fake.claim();
    eprintln!("sahte udisks hazır; {plug_after} sn sonra bellek takılacak");
    std::thread::sleep(Duration::from_secs(plug_after));
    fake.add_drive("Kingston", "Kingston", "DataTraveler 3.0", true);
    fake.add_block("sdb", "Kingston", false);
    fake.add_mounted_volume("sdb1", "Kingston", "YEDEK");
    eprintln!("bellek takıldı: sdb1");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
