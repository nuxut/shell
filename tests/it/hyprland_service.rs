//! Sahte Hyprland soketleriyle uçtan uca servis testleri: ilk durum, olay patlaması
//! birleştirme, tekrar yayınlamama, uzun satır, kopma/yeniden bağlanma, temiz kapanış,
//! istemci zaman aşımı, boyut sınırı ve başarısız sorgunun geri çekilmeyle yeniden denenmesi.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nuxut_shell::services::hyprland::{
    HyprError, HyprlandClient, HyprlandEvent, HyprlandPaths, HyprlandService, HyprSnapshot, ServiceOptions,
    WorkspaceTarget,
};
use serde_json::Value;

const MONITORS: &str = include_str!("../fixtures/hyprland/monitors.json");
const WORKSPACES: &str = include_str!("../fixtures/hyprland/workspaces.json");
const ACTIVE_WINDOW: &str = include_str!("../fixtures/hyprland/activewindow.json");
const SWITCH_BURST: &str = include_str!("../fixtures/hyprland/socket2-workspace-switch.txt");

use nuxut_shell::services::hyprland::SNAPSHOT_COMMAND as BATCH;
const CLIENTS: &str = include_str!("../fixtures/hyprland/clients.json");

const WAIT: Duration = Duration::from_secs(3);
/// Sahte sunucuyla konuşan istemcinin istek süresi. Ürün varsayılanı (1 sn) gerçek compositor
/// içindir; yük altında sahte sunucunun thread'i geç planlanır ve istek EAGAIN ile düşerdi
/// (`client_dispatch_sends_typed_command`, 2026-10-15). Zaman aşımını sınayan test kendi
/// kısa süresini verir.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

/// socket1'i taklit eder: istek başına bağlantı, cevap sonra kapat.
struct FakeSocket1 {
    monitors: Arc<Mutex<Value>>,
    requests: Arc<Mutex<Vec<String>>>,
    /// Bu kadar toplu durum sorgusu başarısız olur: sayaç tekse bağlantı cevapsız kapanır,
    /// çiftse bozuk JSON döner (iki hata biçimi de denenir).
    fail_batches: Arc<AtomicUsize>,
    /// `j/devices` cevabındaki etkin düzen (`tr,us`); düzen sorguları `requests`e yazılmaz
    /// (durum sorgusu sayan testler etkilenmesin).
    layout: Arc<AtomicUsize>,
    devices: Arc<AtomicUsize>,
}

impl FakeSocket1 {
    fn start(path: &Path) -> Self {
        let listener = UnixListener::bind(path).unwrap();
        let monitors = Arc::new(Mutex::new(serde_json::from_str::<Value>(MONITORS).unwrap()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let fail_batches = Arc::new(AtomicUsize::new(0));
        let (layout, devices) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (lay, dev) = (Arc::clone(&layout), Arc::clone(&devices));
        let (m, r, f) = (Arc::clone(&monitors), Arc::clone(&requests), Arc::clone(&fail_batches));
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { return };
                let mut buf = [0u8; 4096];
                let n = conn.read(&mut buf).unwrap_or(0);
                let cmd = String::from_utf8_lossy(&buf[..n]).into_owned();
                if cmd == "j/devices" {
                    dev.fetch_add(1, Ordering::SeqCst);
                    let i = lay.load(Ordering::SeqCst);
                    let names = ["Turkish", "English (US)"];
                    let reply = serde_json::json!({"keyboards": [{"name": "kbd", "layout": "tr,us", "variant": ",", "active_layout_index": i, "active_keymap": names[i % 2], "main": true}]});
                    let _ = conn.write_all(reply.to_string().as_bytes());
                    continue;
                }
                if cmd == BATCH {
                    let left = f.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
                    if let Ok(left) = left {
                        r.lock().unwrap().push(cmd);
                        if left % 2 == 0 {
                            let _ = conn.write_all(b"[{\"bozuk\": ");
                        }
                        continue;
                    }
                }
                let reply = match cmd.as_str() {
                    "j/monitors" => m.lock().unwrap().to_string(),
                    "j/workspaces" => WORKSPACES.to_owned(),
                    "j/activewindow" => ACTIVE_WINDOW.to_owned(),
                    // Hyprland toplu cevapları üç satır sonuyla ayırır.
                    BATCH => format!("{}\n\n\n{WORKSPACES}\n\n\n{ACTIVE_WINDOW}\n\n\n{CLIENTS}", m.lock().unwrap()),
                    c if c.starts_with("dispatch workspace ") => "ok".to_owned(),
                    "dispatch dpms on" | "dispatch dpms off" => "ok".to_owned(),
                    _ => "unknown request".to_owned(),
                };
                r.lock().unwrap().push(cmd);
                let _ = conn.write_all(reply.as_bytes());
            }
        });
        Self { monitors, requests, fail_batches, layout, devices }
    }

    fn set_active_workspace(&self, id: i64) {
        self.monitors.lock().unwrap()[0]["activeWorkspace"] = serde_json::json!({ "id": id, "name": id.to_string() });
    }

    fn fail_next_batches(&self, n: usize) {
        self.fail_batches.store(n, Ordering::SeqCst);
    }

    fn batch_count(&self) -> usize {
        self.requests.lock().unwrap().iter().filter(|r| *r == BATCH).count()
    }

    fn take_requests(&self) -> Vec<String> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

/// socket2'yi taklit eder: kabul edilen bağlantılar kanaldan teste verilir.
fn fake_socket2(path: &Path) -> Receiver<UnixStream> {
    let listener = UnixListener::bind(path).unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { return };
            if tx.send(conn).is_err() {
                return;
            }
        }
    });
    rx
}

fn fast_options() -> ServiceOptions {
    ServiceOptions {
        request_timeout: CLIENT_TIMEOUT,
        coalesce_quiet: Duration::from_millis(30),
        coalesce_max_delay: Duration::from_millis(150),
        reconnect_min: Duration::from_millis(20),
        reconnect_max: Duration::from_millis(100),
        stable_after: Duration::from_secs(10),
        query_retry_min: Duration::from_millis(20),
        query_retry_max: Duration::from_millis(100),
    }
}

/// Klavye düzeni olayları (`Keyboard`) süzülür: onları `keyboard_layouts_on_connect_and_change`
/// sınar, diğer testlerin olay sırası değişmez.
fn spawn(paths: HyprlandPaths) -> (HyprlandService, Receiver<HyprlandEvent>) {
    spawn_with(paths, false)
}

fn spawn_with(paths: HyprlandPaths, keyboard: bool) -> (HyprlandService, Receiver<HyprlandEvent>) {
    let (tx, rx) = mpsc::channel();
    let service = HyprlandService::spawn(paths, fast_options(), move |e| {
        if keyboard || !matches!(e, HyprlandEvent::Keyboard(_)) {
            let _ = tx.send(e);
        }
    })
    .unwrap();
    (service, rx)
}

/// Bar düzen göstergesi: bağlanınca ana klavyenin düzenleri yayınlanır; `activelayout`'ta yeniden
/// sorgulanır ve yalnız değiştiyse yeni olay gelir; durum sorgusu yapılmaz.
#[test]
fn keyboard_layouts_on_connect_and_change() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let (service, rx) = spawn_with(paths, true);
    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    expect_snapshot(&rx);
    let keyboard = |rx: &Receiver<HyprlandEvent>| match rx.recv_timeout(WAIT) {
        Ok(HyprlandEvent::Keyboard(Some(k))) => k,
        other => panic!("Keyboard bekleniyordu: {other:?}"),
    };
    let k = keyboard(&rx);
    assert_eq!((k.layouts.len(), k.active, k.active_name.as_str()), (2, 0, "Turkish"));
    socket1.take_requests();
    socket1.layout.store(1, Ordering::SeqCst);
    conn.write_all(b"activelayout>>kbd,English (US)\n").unwrap();
    assert!(matches!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::ActiveLayout { .. }));
    let k = keyboard(&rx);
    assert_eq!((k.active, k.active_label().as_deref()), (1, Some("US")));
    // Aynı düzen yeniden bildirilirse sorgu yapılır ama olay gelmez.
    let before = socket1.devices.load(Ordering::SeqCst);
    conn.write_all(b"activelayout>>kbd,English (US)\n").unwrap();
    assert!(matches!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::ActiveLayout { .. }));
    expect_silence(&rx, Duration::from_millis(250));
    assert!(socket1.devices.load(Ordering::SeqCst) > before);
    assert!(socket1.take_requests().is_empty(), "düzen değişimi durum sorgusu yapmaz");
    drop(service);
}

fn expect_snapshot(rx: &Receiver<HyprlandEvent>) -> HyprSnapshot {
    match rx.recv_timeout(WAIT) {
        Ok(HyprlandEvent::Snapshot(s)) => s,
        other => panic!("Snapshot bekleniyordu: {other:?}"),
    }
}

fn expect_silence(rx: &Receiver<HyprlandEvent>, d: Duration) {
    if let Ok(e) = rx.recv_timeout(d) {
        panic!("olay beklenmiyordu: {e:?}");
    }
}

#[test]
fn service_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let (service, rx) = spawn(paths);

    // Bağlanınca ilk durum gecikmesiz gelir.
    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    let s = expect_snapshot(&rx);
    assert_eq!(s.monitors[0].active_workspace.id, 2);
    assert_eq!(s.workspaces.iter().map(|w| w.id).collect::<Vec<_>>(), [1, 2, 3, 4]);
    assert_eq!(s.active_window.as_ref().unwrap().class, "code-oss");
    expect_silence(&rx, Duration::from_millis(100));
    socket1.take_requests();

    // Gerçek bir workspace geçişi patlaması: tek yayın, en fazla iki sorgu turu. (Önde kenarın
    // gecikmesizliğini yükten bağımsız olarak `leading_edge_does_not_wait_for_quiet` sınar.)
    socket1.set_active_workspace(1);
    conn.write_all(SWITCH_BURST.as_bytes()).unwrap();
    let s = expect_snapshot(&rx);
    assert_eq!(s.monitors[0].active_workspace.id, 1);
    expect_silence(&rx, Duration::from_millis(250));
    let rounds = socket1.take_requests().iter().filter(|r| *r == BATCH).count();
    assert!((1..=2).contains(&rounds), "sorgu turu: {rounds}");

    // Durum değişmeden gelen olaylar sorgulanır ama yeniden yayınlanmaz.
    conn.write_all(SWITCH_BURST.as_bytes()).unwrap();
    expect_silence(&rx, Duration::from_millis(250));
    assert!(socket1.take_requests().iter().any(|r| r == BATCH));

    // İlgisiz olaylar hiç sorgu yapmaz.
    conn.write_all(b"openlayer>>nuxut-shell\ncloselayer>>nuxut-shell\nsubmap>>resize\n").unwrap();
    expect_silence(&rx, Duration::from_millis(200));
    assert!(socket1.take_requests().is_empty());

    // Sınırı aşan satır atlanır, akış senkron kalır.
    let mut long = vec![b'x'; 200 * 1024];
    long.push(b'\n');
    conn.write_all(&long).unwrap();
    socket1.set_active_workspace(4);
    conn.write_all(b"workspacev2>>4,4\n").unwrap();
    assert_eq!(expect_snapshot(&rx).monitors[0].active_workspace.id, 4);

    // Kopma → Disconnected → yeniden bağlanma → durum yeniden yayınlanır.
    drop(conn);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::Disconnected);
    let _conn = accepted.recv_timeout(WAIT).unwrap();
    assert_eq!(expect_snapshot(&rx).monitors[0].active_workspace.id, 4);

    // Temiz kapanış: bloklayan okuma soket kapatılarak kesilir.
    let t = Instant::now();
    drop(service);
    assert!(t.elapsed() < Duration::from_millis(500), "kapanış {:?}", t.elapsed());
}

/// Önde kenar: boştaki servise gelen ilk olay sessizlik süresini beklemeden yayınlanır. Duvar
/// saati sınırı yerine sessizlik uzun (2 s) seçilir: önde kenarı olmayan bir birleştirici en az
/// 2 s bekler; önde kenarlı olan, makine ne kadar yüklü olursa olsun bunun çok altında kalır
/// (sınır 1 s). İlk durum da birleştiriciyi soğutur: önce boşa düşmesi beklenir.
#[test]
fn leading_edge_does_not_wait_for_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let quiet = Duration::from_secs(2);
    let options = ServiceOptions { coalesce_quiet: quiet, coalesce_max_delay: quiet * 2, ..fast_options() };
    let (tx, rx) = mpsc::channel();
    let _service = HyprlandService::spawn(paths, options, move |e| {
        if !matches!(e, HyprlandEvent::Keyboard(_)) {
            let _ = tx.send(e);
        }
    })
    .unwrap();

    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    expect_snapshot(&rx);
    // Soğuma (quiet) bitsin, birleştirici boşa düşsün; yük altında geç uyanmaya pay.
    thread::sleep(quiet + Duration::from_secs(1));
    socket1.set_active_workspace(1);
    let t = Instant::now();
    conn.write_all(SWITCH_BURST.as_bytes()).unwrap();
    let s = expect_snapshot(&rx);
    let latency = t.elapsed();
    assert_eq!(s.monitors[0].active_workspace.id, 1);
    assert!(latency < quiet / 2, "önde kenar sessizliği beklememeli: {latency:?}");
}

/// Workspace hapı regresyonu (servis tarafı): 1→2→3→2 yavaş geçişlerde her geçiş tam bir
/// anlık görüntüyle yayınlanır; hızlı patlamada (3→2→3→1) son durum (arka kenar) asla
/// kaybolmaz ve ondan sonra bayat durum gelmez.
#[test]
fn workspace_switches_slow_and_fast_publish_final_state() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let (service, rx) = spawn(paths);
    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    assert_eq!(expect_snapshot(&rx).monitors[0].active_workspace.id, 2);

    let switch = |conn: &mut UnixStream, id: i64| {
        socket1.set_active_workspace(id);
        conn.write_all(format!("workspace>>{id}\nworkspacev2>>{id},{id}\nactivewindow>>,\n").as_bytes()).unwrap();
    };

    // Yavaş: her geçiş ayrı yayın, doğru aktif workspace.
    for id in [1, 2, 3, 2] {
        switch(&mut conn, id);
        assert_eq!(expect_snapshot(&rx).monitors[0].active_workspace.id, id);
        expect_silence(&rx, Duration::from_millis(120));
    }

    // Hızlı: sessizlik penceresinden kısa aralıklarla; ara durumlar birleşebilir ama son
    // yayınlanan durum son geçiştir. Son durum patlama öncesindekinden (2) farklı seçilir:
    // yük altında bütün patlama tek sorguya birleşebilir ve o sorgu öncekiyle aynı durumu
    // görürse servis (doğru olarak) hiç yayın yapmaz — eski test bunu kayıp sanıp düşüyordu.
    let fast = [3, 2, 3, 1];
    for id in fast {
        switch(&mut conn, id);
        thread::sleep(Duration::from_millis(10));
    }
    // Koşul tabanlı bekleme: son durum gelene kadar (cömert üst sınır), yayınlanan her ara
    // durum patlamadaki geçişlerden biri olmalı.
    let deadline = Instant::now() + WAIT;
    let mut seen = Vec::new();
    while seen.last() != Some(&1) {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(HyprlandEvent::Snapshot(s)) => seen.push(s.monitors[0].active_workspace.id),
            other => panic!("son geçiş yayınlanmadı: {other:?}, görülen {seen:?}"),
        }
    }
    assert!(seen.iter().all(|id| fast.contains(id)), "beklenmeyen ara durum: {seen:?}");
    // Son durumdan sonra bayat bir durum gelmez (arka kenar sonuncudur).
    expect_silence(&rx, Duration::from_millis(250));
    drop(service);
}

#[test]
fn missing_hyprland_retries_quietly_and_shuts_down_fast() {
    let dir = tempfile::tempdir().unwrap();
    let (service, rx) = spawn(HyprlandPaths::in_instance_dir(dir.path()));
    expect_silence(&rx, Duration::from_millis(300));
    let t = Instant::now();
    drop(service);
    assert!(t.elapsed() < Duration::from_millis(200), "kapanış {:?}", t.elapsed());
}

#[test]
fn late_hyprland_is_picked_up_by_reconnect() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let (service, rx) = spawn(paths.clone());
    thread::sleep(Duration::from_millis(150));
    let _socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let _conn = accepted.recv_timeout(WAIT).unwrap();
    assert_eq!(expect_snapshot(&rx).monitors.len(), 1);
    drop(service);
}

#[test]
fn client_dispatch_sends_typed_command() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let client = HyprlandClient::new(paths.socket1).with_timeout(CLIENT_TIMEOUT);
    client.dispatch_workspace(WorkspaceTarget::Id(3)).unwrap();
    client.dispatch_workspace(WorkspaceTarget::Relative(-1)).unwrap();
    assert!(client.dispatch_workspace(WorkspaceTarget::Id(0)).is_err());
    assert_eq!(socket1.take_requests(), ["dispatch workspace 3", "dispatch workspace e-1"]);
    client.dispatch_dpms(false).unwrap();
    client.dispatch_dpms(true).unwrap();
    assert_eq!(socket1.take_requests(), ["dispatch dpms off", "dispatch dpms on"]);
}

#[test]
fn client_times_out_on_hung_compositor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".socket.sock");
    let listener = UnixListener::bind(&path).unwrap();
    thread::spawn(move || {
        // Bağlantıyı kabul et ama hiç cevap verme.
        let held: Vec<UnixStream> = listener.incoming().map_while(Result::ok).collect();
        drop(held);
    });
    let client = HyprlandClient::new(path).with_timeout(Duration::from_millis(150));
    let t = Instant::now();
    let err = client.monitors().unwrap_err();
    // Soket süre aşımı (EAGAIN) zaman aşımı olarak raporlanır.
    assert!(matches!(&err, HyprError::Io(e) if e.kind() == std::io::ErrorKind::TimedOut), "{err:?}");
    // Yalnız güvenlik ağı (yük altında zamanlayıcı geç uyanabilir); asıl iddia hata türü.
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
}

#[test]
fn client_rejects_oversized_response() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".socket.sock");
    let listener = UnixListener::bind(&path).unwrap();
    thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            let mut buf = [0u8; 64];
            let _ = conn.read(&mut buf);
            let chunk = vec![b' '; 1024 * 1024];
            for _ in 0..5 {
                if conn.write_all(&chunk).is_err() {
                    return;
                }
            }
        }
    });
    let client = HyprlandClient::new(path).with_timeout(CLIENT_TIMEOUT);
    assert!(matches!(client.workspaces(), Err(HyprError::TooLarge)));
}

/// Bir olayın tetiklediği sorgu başarısız olursa bekleyen değişiklik kaybolmaz: servis geri
/// çekilmeyle yeniden sorgular ve son durumu **yeni bir olay gelmeden** yayınlar. (Eski hâlde
/// olay karşılanmış sayılır, durum bir sonraki Hyprland olayına kadar bayat kalırdı.)
#[test]
fn failed_query_is_retried_without_new_event() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let (service, rx) = spawn(paths);
    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    assert_eq!(expect_snapshot(&rx).monitors[0].active_workspace.id, 2);
    socket1.take_requests();

    // İlk deneme bozuk JSON, ikincisi cevapsız kapanan bağlantı; üçüncüsü doğru cevap. Tek bir
    // olay satırı gönderilir: arka kenar yenilemesi hatayı örtemez.
    socket1.set_active_workspace(3);
    socket1.fail_next_batches(2);
    conn.write_all(b"workspacev2>>3,3\n").unwrap();
    assert_eq!(expect_snapshot(&rx).monitors[0].active_workspace.id, 3);
    assert!(socket1.batch_count() >= 3, "yeniden deneme sayısı: {}", socket1.batch_count());
    // Aynı durum ikinci kez yayınlanmaz; başarıdan sonra ek sorgu (timer) yok.
    expect_silence(&rx, Duration::from_millis(150));
    socket1.take_requests();
    expect_silence(&rx, Duration::from_millis(150));
    assert_eq!(socket1.batch_count(), 0, "hata yokken boşta sorgu yapılmamalı");

    // Geri çekilme başarıda sıfırlanmıştır: sonraki geçiş normal yayınlanır.
    socket1.set_active_workspace(1);
    conn.write_all(b"workspacev2>>1,1\n").unwrap();
    assert_eq!(expect_snapshot(&rx).monitors[0].active_workspace.id, 1);
    drop(service);
}

/// Kalıcı sorgu hatasında sıcak döngü yok: denemeler arası bekleme ikiye katlanarak tavana
/// (100 ms) çıkar ve orada kalır. Alt sınır yükten bağımsızdır (beklemeler yalnızca uzayabilir);
/// üst sınır cömerttir (tavansız ikiye katlama 10. denemeye ~20 s'de varırdı). Hata geçince
/// durum yeni olay gelmeden yayınlanır; kapanış süren yeniden denemeye takılmaz.
#[test]
fn persistent_query_failure_backs_off_to_cap() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let (service, rx) = spawn(paths);
    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    expect_snapshot(&rx);
    socket1.take_requests();

    socket1.set_active_workspace(3);
    socket1.fail_next_batches(usize::MAX);
    let t = Instant::now();
    conn.write_all(b"workspacev2>>3,3\n").unwrap();
    // 10 deneme: beklemeler 20+40+80+100×6 = 740 ms.
    const ATTEMPTS: usize = 10;
    let deadline = t + WAIT;
    while socket1.batch_count() < ATTEMPTS {
        assert!(Instant::now() < deadline, "yeniden deneme durdu: {} deneme", socket1.batch_count());
        thread::sleep(Duration::from_millis(5));
    }
    let elapsed = t.elapsed();
    let count = socket1.batch_count();
    assert!(elapsed >= Duration::from_millis(700), "geri çekilme yok (sıcak döngü): {count} deneme {elapsed:?}");
    assert!(count <= ATTEMPTS + 1, "deneme sayısı sınırı aştı: {count}");
    // Hata sürerken hiçbir şey yayınlanmaz (son iyi durum geçerli kalır).
    assert!(rx.try_recv().is_err());

    // Hata geçti: ek olay gönderilmeden son durum gelir.
    socket1.fail_next_batches(0);
    assert_eq!(expect_snapshot(&rx).monitors[0].active_workspace.id, 3);

    // Kalıcı hatanın ortasında kapanış da hızlıdır.
    socket1.fail_next_batches(usize::MAX);
    socket1.set_active_workspace(4);
    conn.write_all(b"workspacev2>>4,4\n").unwrap();
    while socket1.batch_count() < count + 3 {
        assert!(Instant::now() < t + WAIT * 2, "hata sonrası sorgu yok");
        thread::sleep(Duration::from_millis(5));
    }
    let t = Instant::now();
    drop(service);
    assert!(t.elapsed() < Duration::from_millis(500), "kapanış {:?}", t.elapsed());
}

/// `configreloaded` ayrı bir olay olarak iletilir (büyüteç kendi büyütmesini yeniden uygular);
/// durum yenilemesi de yapılır ama durum aynıysa yeniden yayınlanmaz.
#[test]
fn config_reloaded_is_forwarded() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let (service, rx) = spawn(paths);
    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    expect_snapshot(&rx);
    socket1.take_requests();
    conn.write_all(b"configreloaded>>\n").unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::ConfigReloaded);
    expect_silence(&rx, Duration::from_millis(250));
    assert!(socket1.take_requests().iter().any(|r| r == BATCH), "durum da yenilendi");
    // İlgisiz olay ConfigReloaded üretmez.
    conn.write_all(b"workspacev2>>2,2\n").unwrap();
    expect_silence(&rx, Duration::from_millis(250));
    drop(service);
}

/// `activelayout` ekran klavyesine iletilir; durum yenilemesi tetiklemez, bozuk veri atlanır.
#[test]
fn active_layout_is_forwarded_without_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let (service, rx) = spawn(paths);
    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    expect_snapshot(&rx);
    socket1.take_requests();
    conn.write_all(b"activelayout>>at-translated-set-2-keyboard,English (US, intl., with dead keys)\n").unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::ActiveLayout { keyboard: "at-translated-set-2-keyboard".into(), layout: "English (US, intl., with dead keys)".into() });
    conn.write_all(b"activelayout>>no-comma\nactivelayout>>kbd,\n").unwrap();
    conn.write_all(b"activelayout>>kbd,Turkish\n").unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::ActiveLayout { keyboard: "kbd".into(), layout: "Turkish".into() });
    expect_silence(&rx, Duration::from_millis(250));
    assert!(socket1.take_requests().is_empty(), "düzen değişimi durum sorgusu yapmaz");
    drop(service);
}

/// `screencast` sayaç olayı olduğu gibi iletilir (gizlilik göstergesi); durum yenilemesi yok.
#[test]
fn screencast_is_forwarded_without_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let (service, rx) = spawn(paths);
    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    expect_snapshot(&rx);
    socket1.take_requests();
    conn.write_all(b"screencast>>1,0\nscreencast>>garbage\nscreencast>>0,1\n").unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::Screencast(true));
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::Screencast(false));
    expect_silence(&rx, Duration::from_millis(250));
    assert!(socket1.take_requests().is_empty(), "screencast durum yenilemesi istemez");
    drop(service);
}

/// Başka olayların verisi (`>>` ve virgül içeren pencere başlığı) `activelayout` sayılmaz:
/// ad yalnızca satır başındaki olay adından okunur. Satır sonu içeren başlık socket2'de ayrı bir
/// satır olur (Hyprland kaçırmaz) — o sınır `docs/design/osk.md` "Sınırlar"da.
#[test]
fn other_events_carrying_activelayout_text_are_not_layout_changes() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let _socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let (service, rx) = spawn(paths);
    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    expect_snapshot(&rx);
    conn.write_all(b"activewindow>>kitty,x activelayout>>kbd,Evil\nwindowtitlev2>>abc,activelayout>>kbd,Evil\nactivelayoutv2>>kbd,Evil\n")
        .unwrap();
    loop {
        match rx.recv_timeout(Duration::from_millis(400)) {
            Ok(HyprlandEvent::ActiveLayout { .. }) => panic!("başka olaydan düzen değişimi üretildi"),
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    // Düzen adındaki `>>` olduğu gibi ad sayılır (yalnızca ilk `>>` olay adını ayırır).
    conn.write_all(b"activelayout>>kbd,A>>B\n").unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::ActiveLayout { keyboard: "kbd".into(), layout: "A>>B".into() });
    drop(service);
}

/// Düşmanca: pencere başlığı / sınıfı / paylaşım adı içine yazılmış `screencast>>0,0` sahte olay
/// üretmez (Hyprland satır sonunu boşluğa çevirir; olay adı satırın başındaki addır). İki
/// eşzamanlı paylaşımın başla/bitirleri ayrı ayrı iletilir (sayım modelde).
#[test]
fn screencast_cannot_be_spoofed_from_titles() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HyprlandPaths::in_instance_dir(dir.path());
    let socket1 = FakeSocket1::start(&paths.socket1);
    let accepted = fake_socket2(&paths.socket2);
    let (service, rx) = spawn(paths);
    let mut conn = accepted.recv_timeout(WAIT).unwrap();
    expect_snapshot(&rx);
    socket1.take_requests();
    conn.write_all(b"screencast>>1,0\nscreencast>>1,1\n").unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::Screencast(true));
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), HyprlandEvent::Screencast(true));
    // Hyprland'in başlıktaki "\n"i boşluğa çevirdiği hâl ve diğer başlık taşıyan olaylar.
    conn.write_all(b"activewindow>>kitty,evil screencast>>0,0\n").unwrap();
    conn.write_all(b"windowtitlev2>>55aa,screencast>>0,0\n").unwrap();
    conn.write_all(b"screencastv2>>0,1,screencast>>0,0\n").unwrap();
    conn.write_all(b"screencast>>0,0 extra\nscreencast>>0\nScreencast>>0,0\n").unwrap();
    // Başlık olayları durum yenilemesi üretir (Snapshot); hiçbiri Screencast(false) değil.
    let deadline = std::time::Instant::now() + Duration::from_millis(600);
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        if let Ok(HyprlandEvent::Screencast(on)) = rx.recv_timeout(left) {
            panic!("sahte screencast olayı geçti: {on}");
        }
    }
    // Gerçek bitişler yine iletilir.
    conn.write_all(b"screencast>>0,0\n").unwrap();
    let got = loop {
        match rx.recv_timeout(WAIT).unwrap() {
            HyprlandEvent::Screencast(on) => break on,
            _ => continue,
        }
    };
    assert!(!got);
    drop(service);
}
