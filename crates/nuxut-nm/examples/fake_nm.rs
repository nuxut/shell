//! Sahte NetworkManager — **yalnız `dbus-run-session` içinde**, oturum bus'ında.
//!
//! Ayarlar'ın (ve kabuğun) canlı testi gerçek sistem NM'ine dokunmadan bunu kullanır:
//!
//! ```sh
//! dbus-run-session -- sh test.sh   # test.sh içinde:
//! NUXUT_FAKE_BUS=1 cargo run -p nuxut-nm --features test-support --example fake_nm &
//! NUXUT_NM_BUS=session <istemci>   # istemci NetworkOptions.bus = nuxut_nm::bus_from_env()
//! ```
//!
//! İçerik (`tests/fake_nm` ile aynı kod): Wi-Fi cihazı (wlan0) + erişim noktaları Ev (WPA2,
//! kayıtlı, etkin; şifresi NM'de yok → etkinleştirmede ajana sorulur), Kafe (WPA2), Misafir
//! (açık), gizli (boş SSID), Komsu (WPA3); kayıtlı Ofis (şifre kayıtlı), "İş VPN" (WireGuard),
//! "Kablolu" (ethernet enp1s0, IP/DNS/hız). "Ev"in doğru şifresi `ev-sifre-123` (kayıtlı şifre
//! başkaysa etkinleştirme NO_SECRETS ile düşer). Sahne `NUXUT_FAKE_NM_SCENE`: `default`,
//! `no-device`, `empty`, `wifi-off`, `basic`. Her çağrı stdout'a yazılır.

#[path = "../tests/fake_nm/mod.rs"]
mod fake_nm;

fn main() {
    // Yalnız dbus-run-session'ın yalıtılmış bus'ında ve açık NUXUT_FAKE_BUS=1 ile.
    let var = |k: &str| std::env::var(k).ok();
    if let Err(why) = nuxut_dbus::fake_bus_guard(
        var(nuxut_dbus::FAKE_BUS_ENV).as_deref(),
        var("DBUS_SESSION_BUS_ADDRESS").as_deref(),
        var("XDG_RUNTIME_DIR").as_deref(),
    ) {
        eprintln!("fake_nm: reddedildi ({why}); yalnız `dbus-run-session` içinde NUXUT_FAKE_BUS=1 ile");
        std::process::exit(2);
    }
    let conn = match zbus::blocking::Connection::session() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fake_nm: oturum bus'ına bağlanılamadı: {e}");
            std::process::exit(1);
        }
    };
    let log = fake_nm::Log::echo();
    // Sahne: NUXUT_FAKE_NM_SCENE = default | no-device | empty | wifi-off | basic.
    let scene_name = std::env::var("NUXUT_FAKE_NM_SCENE").unwrap_or_default();
    let Some(scene) = fake_nm::Scene::parse(&scene_name) else {
        eprintln!("fake_nm: bilinmeyen sahne {scene_name:?} (default|no-device|empty|wifi-off|basic)");
        std::process::exit(2);
    };
    let _fake = fake_nm::FakeNm::start_scene(conn, &log, scene);
    println!("fake_nm: hazır ({} oturum bus'ında)", fake_nm::NM);
    // Sınırlı ömür: betik öldürmeyi unutsa da en geç bir saatte çıkar.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3600);
    while std::time::Instant::now() < until {
        std::thread::park_timeout(until.saturating_duration_since(std::time::Instant::now()));
    }
}
