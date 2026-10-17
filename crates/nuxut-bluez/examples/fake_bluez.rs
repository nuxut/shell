//! Sahte BlueZ — **yalnız `dbus-run-session` içinde**, oturum bus'ında.
//!
//! Ayarlar'ın (ve kabuğun) canlı testi gerçek sistem BlueZ'ine dokunmadan bunu kullanır:
//!
//! ```sh
//! dbus-run-session -- sh test.sh   # test.sh içinde:
//! NUXUT_FAKE_BUS=1 cargo run -p nuxut-bluez --features test-support --example fake_bluez &
//! NUXUT_BLUEZ_BUS=session <istemci>   # istemci BluetoothOptions.bus = nuxut_bluez::bus_from_env()
//! ```
//!
//! İçerik (`tests/fake_bluez` ile aynı kod): adaptör hci0 "nuxut-vm" (açık), eşleşmiş
//! "Kulaklık", eşleşmemiş "Klavye" (ajan varsa onay sorar), eşleşmesi hep
//! `AuthenticationFailed` dönen "Hoparlör", `CancelPairing` bekleyen "Saat" (giyilebilir), adsız
//! LE cihaz; eşleşince `Paired=true`, `RemoveDevice` cihazı siler. Sahne
//! `NUXUT_FAKE_BLUEZ_SCENE`: `default`, `no-adapter`, `off`, `rfkill`, `empty`, `basic`. Her çağrı
//! stdout'a yazılır.

#[path = "../tests/fake_bluez/mod.rs"]
mod fake_bluez;

fn main() {
    // Yalnız dbus-run-session'ın yalıtılmış bus'ında ve açık NUXUT_FAKE_BUS=1 ile.
    let var = |k: &str| std::env::var(k).ok();
    if let Err(why) = nuxut_dbus::fake_bus_guard(
        var(nuxut_dbus::FAKE_BUS_ENV).as_deref(),
        var("DBUS_SESSION_BUS_ADDRESS").as_deref(),
        var("XDG_RUNTIME_DIR").as_deref(),
    ) {
        eprintln!("fake_bluez: reddedildi ({why}); yalnız `dbus-run-session` içinde NUXUT_FAKE_BUS=1 ile");
        std::process::exit(2);
    }
    let conn = match zbus::blocking::Connection::session() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fake_bluez: oturum bus'ına bağlanılamadı: {e}");
            std::process::exit(1);
        }
    };
    let log = fake_bluez::Log::echo();
    // Sahne: NUXUT_FAKE_BLUEZ_SCENE = default | no-adapter | off | rfkill | empty | basic.
    let scene_name = std::env::var("NUXUT_FAKE_BLUEZ_SCENE").unwrap_or_default();
    let Some(scene) = fake_bluez::Scene::parse(&scene_name) else {
        eprintln!("fake_bluez: bilinmeyen sahne {scene_name:?} (default|no-adapter|off|rfkill|empty|basic)");
        std::process::exit(2);
    };
    let _fake = fake_bluez::FakeBluez::start_scene(conn, &log, scene);
    println!("fake_bluez: hazır ({} oturum bus'ında)", fake_bluez::BLUEZ);
    // Sınırlı ömür: betik öldürmeyi unutsa da en geç bir saatte çıkar.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3600);
    while std::time::Instant::now() < until {
        std::thread::park_timeout(until.saturating_duration_since(std::time::Instant::now()));
    }
}
