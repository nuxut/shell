//! Büyüteç (t124) sahte Hyprland soketiyle: gerçek `HyprlandClient` socket1'e tam olarak hangi
//! satırları yazıyor (hyprlang `keyword`, Lua `eval`), sınırlar, kullanıcının elle değiştirdiği
//! büyütmeye dokunmama ve çıkışta / çökme sonrası 1'e dönüş.

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use nuxut_shell::services::hyprland::HyprlandClient;
use nuxut_shell::services::magnifier::{
    Command, Factor, Magnifier, MagnifierConfig, Marker, PROVIDER_COMMAND, READ_COMMAND, recover, release, run,
};

/// socket1: istek başına bağlantı. `cursor:zoom_factor` durumunu tutar, istekleri kaydeder.
struct FakeHypr {
    zoom: Arc<Mutex<f64>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeHypr {
    fn start(path: &Path, provider: &'static str, zoom: f64) -> Self {
        let listener = UnixListener::bind(path).unwrap();
        let zoom = Arc::new(Mutex::new(zoom));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (z, r) = (Arc::clone(&zoom), Arc::clone(&requests));
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { return };
                let mut buf = [0u8; 4096];
                let n = conn.read(&mut buf).unwrap_or(0);
                let cmd = String::from_utf8_lossy(&buf[..n]).into_owned();
                let reply = if cmd == READ_COMMAND {
                    format!("{{\"option\": \"cursor:zoom_factor\", \"float\": {:.6}, \"set\": false }}", z.lock().unwrap())
                } else if cmd == PROVIDER_COMMAND {
                    format!("{{\"configProvider\": \"{provider}\", \"backend\": \"headless\"}}")
                } else if let Some(v) = cmd.strip_prefix("keyword cursor:zoom_factor ").filter(|_| provider == "hyprlang") {
                    *z.lock().unwrap() = v.parse().unwrap();
                    "ok".to_owned()
                } else if cmd.starts_with("keyword ") {
                    // Hyprland 0.56.2 Lua altında `keyword`'ü reddeder.
                    "keyword can't work with non-legacy parsers. Use eval.".to_owned()
                } else if let Some(v) = cmd
                    .strip_prefix("eval hl.config({cursor={zoom_factor=")
                    .and_then(|v| v.strip_suffix("}})"))
                    .filter(|_| provider == "lua")
                {
                    *z.lock().unwrap() = v.parse().unwrap();
                    "ok".to_owned()
                } else {
                    "unknown request".to_owned()
                };
                r.lock().unwrap().push(cmd);
                let _ = conn.write_all(reply.as_bytes());
            }
        });
        Self { zoom, requests }
    }

    fn zoom(&self) -> f64 {
        *self.zoom.lock().unwrap()
    }

    fn set_zoom(&self, v: f64) {
        *self.zoom.lock().unwrap() = v;
    }

    /// Yazma istekleri (okumalar hariç).
    fn writes(&self) -> Vec<String> {
        self.requests.lock().unwrap().iter().filter(|r| !r.starts_with("j/")).cloned().collect()
    }
}

fn client(dir: &Path) -> HyprlandClient {
    // Sahte sunucu yük altında geç planlanabilir: ürünün 1 sn'si yerine cömert süre.
    HyprlandClient::new(dir.join(".socket.sock")).with_timeout(std::time::Duration::from_secs(30))
}

#[test]
fn hyprlang_keywords_bounds_and_exit_reset() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeHypr::start(&dir.path().join(".socket.sock"), "hyprlang", 1.0);
    let c = client(dir.path());
    let mut m = Magnifier::new(&MagnifierConfig::default());

    // Çarpımsal adım 1.25, tavan 8.
    for expected in [1.25, 1.563, 1.954] {
        assert_eq!(run(&mut m, &c, Command::In).unwrap().factor, expected);
    }
    assert_eq!(fake.zoom(), 1.954);
    run(&mut m, &c, Command::Set(Factor::new(7.5))).unwrap();
    assert_eq!(run(&mut m, &c, Command::In).unwrap().factor, 8.0);
    let before = fake.writes().len();
    assert_eq!(run(&mut m, &c, Command::In).unwrap().factor, 8.0);
    assert_eq!(fake.writes().len(), before, "tavanda istek yok");
    assert!(run(&mut m, &c, Command::Set(Factor::new(9.0))).is_err(), "tavan üstü set reddedilir");
    assert_eq!(fake.zoom(), 8.0);
    assert_eq!(run(&mut m, &c, Command::Out).unwrap().factor, 6.4);

    assert_eq!(
        fake.writes(),
        [
            "keyword cursor:zoom_factor 1.250",
            "keyword cursor:zoom_factor 1.563",
            "keyword cursor:zoom_factor 1.954",
            "keyword cursor:zoom_factor 7.500",
            "keyword cursor:zoom_factor 8.000",
            "keyword cursor:zoom_factor 6.400",
        ]
    );

    // Çıkış: kabuğun yazdığı değer canlı → 1.
    assert_eq!(release(&mut m, &c), Ok(true));
    assert_eq!(fake.zoom(), 1.0);
    assert_eq!(fake.writes().last().unwrap(), "keyword cursor:zoom_factor 1.000");
}

#[test]
fn user_changed_zoom_is_left_alone_on_exit() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeHypr::start(&dir.path().join(".socket.sock"), "hyprlang", 1.0);
    let c = client(dir.path());
    let mut m = Magnifier::new(&MagnifierConfig::default());
    run(&mut m, &c, Command::Set(Factor::new(3.0))).unwrap();
    // Kullanıcı kendi bind'iyle (`hyprctl keyword`) 4 yaptı.
    fake.set_zoom(4.0);
    let n = fake.writes().len();
    assert_eq!(release(&mut m, &c), Ok(false));
    assert_eq!(fake.writes().len(), n);
    assert_eq!(fake.zoom(), 4.0);
}

#[test]
fn lua_config_uses_eval() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeHypr::start(&dir.path().join(".socket.sock"), "lua", 1.0);
    let c = client(dir.path());
    let mut m = Magnifier::new(&MagnifierConfig { step: 1.5, ..MagnifierConfig::default() });
    assert_eq!(run(&mut m, &c, Command::Toggle).unwrap().factor, 2.0);
    assert_eq!(run(&mut m, &c, Command::In).unwrap().factor, 3.0);
    assert_eq!(run(&mut m, &c, Command::Toggle).unwrap().factor, 1.0);
    assert_eq!(run(&mut m, &c, Command::Toggle).unwrap().factor, 3.0, "son çarpan hatırlanır");
    assert_eq!(
        fake.writes(),
        [
            "eval hl.config({cursor={zoom_factor=2.000}})",
            "eval hl.config({cursor={zoom_factor=3.000}})",
            "eval hl.config({cursor={zoom_factor=1.000}})",
            "eval hl.config({cursor={zoom_factor=3.000}})",
        ]
    );
    assert_eq!(release(&mut m, &c), Ok(true));
    assert_eq!(fake.zoom(), 1.0);
}

/// Önceki örnek çöktü: işaret dosyası kaldı. Yeni örnek canlı değer işaretle aynıysa 1'e döner,
/// değilse dokunmaz; işaret her durumda temizlenir.
#[test]
fn crash_marker_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeHypr::start(&dir.path().join(".socket.sock"), "hyprlang", 1.0);
    let c = client(dir.path());
    let marker = Marker::new(dir.path(), "fake_sig").unwrap();

    // "Çöken" örnek: büyüttü, işaret yazıldı, release çağrılmadı.
    let mut crashed = Magnifier::new(&MagnifierConfig::default());
    run(&mut crashed, &c, Command::Set(Factor::new(2.5))).unwrap();
    marker.sync(&crashed);
    assert_eq!(marker.read(), Some(2.5));

    let mut m = Magnifier::new(&MagnifierConfig::default());
    assert_eq!(recover(&mut m, &c, marker.read().unwrap()), Ok(true));
    marker.sync(&m);
    assert_eq!(fake.zoom(), 1.0);
    assert_eq!(marker.read(), None);

    // Kullanıcı arada değiştirmiş: dokunulmaz.
    fake.set_zoom(5.0);
    marker.write(2.5).unwrap();
    let mut m = Magnifier::new(&MagnifierConfig::default());
    assert_eq!(recover(&mut m, &c, marker.read().unwrap()), Ok(false));
    marker.sync(&m);
    assert_eq!(fake.zoom(), 5.0);
    assert_eq!(marker.read(), None);
}

#[test]
fn unreachable_hyprland_is_an_error_not_a_hang() {
    let dir = tempfile::tempdir().unwrap();
    let c = client(dir.path()).with_timeout(std::time::Duration::from_millis(200));
    let mut m = Magnifier::new(&MagnifierConfig::default());
    assert!(run(&mut m, &c, Command::In).is_err());
    assert_eq!(release(&mut m, &c), Ok(false), "sahiplik yokken istek yok");
}

/// Hyprland ayarı yeniden yüklendi (değer 1'e döndü): kabuğun büyütmesi `eval`/`keyword` ile geri
/// yazılır; kullanıcının ayarındaki başka bir değer ezilmez.
#[test]
fn config_reload_reapplies_own_zoom() {
    use nuxut_shell::services::magnifier::reapply;
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeHypr::start(&dir.path().join(".socket.sock"), "lua", 1.0);
    let c = client(dir.path());
    let mut m = Magnifier::new(&MagnifierConfig::default());
    run(&mut m, &c, Command::Set(Factor::new(2.5))).unwrap();
    fake.set_zoom(1.0);
    assert_eq!(reapply(&mut m, &c), Ok(Some(2.5)));
    assert_eq!(fake.zoom(), 2.5);
    assert_eq!(fake.writes().last().unwrap(), "eval hl.config({cursor={zoom_factor=2.500}})");
    fake.set_zoom(4.0);
    let n = fake.writes().len();
    assert_eq!(reapply(&mut m, &c), Ok(None));
    assert_eq!(fake.writes().len(), n);
    assert_eq!(fake.zoom(), 4.0);
    assert_eq!(release(&mut m, &c), Ok(false), "sahiplik bitti: çıkışta da dokunulmaz");
}
