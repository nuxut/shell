//! Bildirim sunucusu uçtan uca: her test kendi özel `dbus-daemon`'ını başlatır (kullanıcının
//! oturum bus'ına asla dokunmaz; orada eski shell ve dunst adı tutuyor). zbus istemcisiyle
//! Notify/Close/eylemler/replaces_id/bozuk image-data/aşırı uzun gövde, ad çakışması ve devralma,
//! NotificationCenter ile uçtan uca kapanış; `notify-send` ile gerçek istemci.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

use nuxut_shell::services::notifications::server::BUS_NAME;
use nuxut_shell::services::notifications::{
    CloseReason, NotificationServer, NotifyError, NotifyEvent as Ev, NotifyIcon, ServerOptions, Urgency, body_text,
};
use zbus::blocking;
use zbus::zvariant::Value;

#[path = "support/notify_bus.rs"]
mod notify_bus;
use notify_bus::{
    LIVENESS, NotificationsProxyBlocking, PrivateBus, Sig, added, hints, no_more, pixmap_size, settled, signals, spawn, svg,
    wait_for,
};

#[path = "support/png.rs"]
mod png_fixture;
use png_fixture::png;

#[path = "support/sandbox.rs"]
mod sandbox_support;

#[test]
fn notify_close_capabilities() {
    let bus = PrivateBus::start();
    let (server, events) = spawn(bus.options());
    let conn = bus.client();
    let sigs = signals(&conn);
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();

    let caps = proxy.get_capabilities().unwrap();
    assert_eq!(caps, ["actions", "body", "body-markup", "body-hyperlinks", "icon-static", "persistence"]);
    let (name, vendor, _version, spec) = proxy.get_server_information().unwrap();
    assert_eq!((name.as_str(), vendor.as_str(), spec.as_str()), ("nuxut", "nuxut", "1.2"));

    let id = proxy
        .notify(
            "Sohbet",
            0,
            "dialog-information",
            "Ayşe",
            "<b>Merhaba</b> <a href=\"https://example.org\">bağ</a> <span>x</span> <a href=\"file:///etc\">y</a>",
            &["default", "Aç", "reply", "Yanıtla"],
            hints([("urgency", Value::U8(2)), ("category", Value::from("im.received")), ("x", Value::I32(5))]),
            -1,
        )
        .unwrap();
    assert_ne!(id, 0);
    let n = added(&events);
    assert_eq!(n.id, id);
    assert_eq!(n.summary, "Ayşe");
    assert_eq!(n.urgency, Urgency::Critical);
    assert_eq!(n.category.as_deref(), Some("im.received"));
    // Tema adı: temada bulunduysa ≤128 px Pixmap'e çözülür, yoksa ad kalır.
    match &n.app_icon {
        NotifyIcon::Name(name) => assert_eq!(name, "dialog-information"),
        NotifyIcon::Pixmap { width, height, .. } => assert!(*width <= 64 && *height <= 64),
        other => panic!("{other:?}"),
    }
    assert_eq!(body_text(&n.body), "Merhaba bağ x y");
    assert!(n.body[0].style.bold);
    assert_eq!(
        n.body[2].link.as_ref().map(|l| (l.url.as_str(), l.host.as_str())),
        Some(("https://example.org/", "example.org"))
    );
    assert!(n.body.iter().filter(|s| s.link.is_some()).count() == 1);
    assert!(n.has_default_action);
    assert_eq!(n.actions.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["Yanıtla"]);

    proxy.close_notification(id).unwrap();
    assert_eq!(wait_for(&events, "Closed", |_| true), Ev::Closed { id, reason: CloseReason::ClosedByCall });
    assert_eq!(wait_for(&sigs, "NotificationClosed", |_| true), Sig::Closed(id, 3));
    // Bilinmeyen kimliği kapatmak hata değil, sinyal de yok.
    proxy.close_notification(9999).unwrap();
    server.handle().close(9999, CloseReason::Dismissed);
    no_more(&sigs, Duration::from_millis(150));
}

#[test]
fn replaces_id_and_handle_close() {
    let bus = PrivateBus::start();
    let (server, events) = spawn(bus.options());
    let handle = server.handle();
    let conn = bus.client();
    let sigs = signals(&conn);
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();

    let id = proxy.notify("a", 0, "", "%10", "", &[], HashMap::new(), -1).unwrap();
    added(&events);
    let again = proxy.notify("a", id, "", "%50", "", &[], HashMap::new(), -1).unwrap();
    assert_eq!(id, again);
    let n = added(&events);
    assert_eq!((n.id, n.summary.as_str()), (id, "%50"));
    let other = proxy.notify("a", 31337, "", "yeni", "", &[], HashMap::new(), -1).unwrap();
    assert!(other != id && other != 31337);

    // Handle başka thread'den de kullanılabilir.
    thread::spawn(move || handle.close(id, CloseReason::Dismissed)).join().unwrap();
    assert_eq!(wait_for(&sigs, "NotificationClosed", |_| true), Sig::Closed(id, 2));
    wait_for(&events, "Closed", |e| *e == Ev::Closed { id, reason: CloseReason::Dismissed });
    // Kapanan kimlikle replaces_id → yeni kimlik.
    let fresh = proxy.notify("a", id, "", "tekrar", "", &[], HashMap::new(), -1).unwrap();
    assert_ne!(fresh, id);
}

#[test]
fn actions_with_activation_token() {
    let bus = PrivateBus::start();
    let (server, events) = spawn(bus.options());
    let handle = server.handle();
    let conn = bus.client();
    let sigs = signals(&conn);
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();

    let id = proxy.notify("a", 0, "", "Soru", "", &["ok", "Tamam", "no", "Hayır"], HashMap::new(), 0).unwrap();
    assert!(!handle.invoke_action(id, "olmayan"));
    assert!(!handle.invoke_action(id + 100, "ok"));
    assert!(!handle.invoke_action(id, "default"), "default eylemi bildirilmedi");
    assert!(handle.invoke_action_with_token(id, "ok", Some("xdg-token-1")));
    assert_eq!(wait_for(&sigs, "ActivationToken", |_| true), Sig::Token(id, "xdg-token-1".into()));
    assert_eq!(wait_for(&sigs, "ActionInvoked", |_| true), Sig::Action(id, "ok".into()));
    assert_eq!(wait_for(&sigs, "Closed", |_| true), Sig::Closed(id, 2));
    wait_for(&events, "Closed olayı", |e| *e == Ev::Closed { id, reason: CloseReason::Dismissed });
    assert!(!handle.invoke_action(id, "ok"), "kapanmış bildirim");

    // resident: eylemden sonra açık kalır.
    let r =
        proxy.notify("a", 0, "", "Kalıcı", "", &["default", ""], hints([("resident", Value::Bool(true))]), 0).unwrap();
    assert!(handle.invoke_action(r, "default"));
    assert_eq!(wait_for(&sigs, "ActionInvoked", |_| true), Sig::Action(r, "default".into()));
    no_more(&sigs, Duration::from_millis(150));
    assert!(handle.invoke_action(r, "default"));
}

fn image_value(w: i32, h: i32, rowstride: i32, alpha: bool, channels: i32, data: Vec<u8>) -> Value<'static> {
    Value::from((w, h, rowstride, alpha, 8i32, channels, data))
}

#[test]
fn image_data_valid_bad_and_large() {
    let bus = PrivateBus::start();
    let (_server, events) = spawn(bus.options());
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();

    // Geçerli RGB, satır dolgulu → düz RGBA, alfa 255.
    let ok = image_value(2, 1, 8, false, 3, vec![1, 2, 3, 4, 5, 6, 0, 0]);
    proxy.notify("a", 0, "", "ok", "", &[], hints([("image-data", ok)]), -1).unwrap();
    match added(&events).image {
        NotifyIcon::Pixmap { width: 2, height: 1, rgba } => assert_eq!(&rgba[..], &[1, 2, 3, 255, 4, 5, 6, 255]),
        other => panic!("{other:?}"),
    }

    // Bozuk: kısa veri, dev/sınır üstü boyut, yanlış kanal, yanlış tür → görsel yok, bildirim kabul.
    for (key, bad) in [
        ("image-data", image_value(4, 4, 16, true, 4, vec![0; 10])),
        ("image_data", image_value(100_000, 100_000, 400_000, true, 4, vec![0; 16])),
        ("image-data", image_value(513, 1, 513 * 4, true, 4, vec![0; 513 * 4])),
        ("icon_data", image_value(1, 1, 4, true, 3, vec![0; 4])),
        ("image-data", Value::from("yanlış tür")),
    ] {
        proxy.notify("a", 0, "", key, "", &[], hints([(key, bad)]), -1).unwrap();
        assert_eq!(added(&events).image, NotifyIcon::None, "{key}");
    }

    // Ağ yolu reddedilir.
    proxy.notify("a", 0, "", "net", "", &[], hints([("image-path", Value::from("https://x/y.png"))]), -1).unwrap();
    assert_eq!(added(&events).image, NotifyIcon::None);

    // Sınırdaki 512x512 RGBA kabul edilir.
    let big = image_value(512, 512, 2048, true, 4, vec![7; 2048 * 512]);
    proxy.notify("a", 0, "", "büyük", "", &[], hints([("image-data", big)]), -1).unwrap();
    match added(&events).image {
        NotifyIcon::Pixmap { width, height, rgba } => assert_eq!((width, height, rgba.len()), (512, 512, 1 << 20)),
        other => panic!("{other:?}"),
    }

    // Tanınmayan hint türleri (dizi, iç içe variant, sözlük) sorunsuz atlanır.
    let odd = hints([
        ("x-array", Value::from(vec![1i32, 2, 3])),
        ("x-nested", Value::Value(Box::new(Value::from("iç")))),
        ("x-dict", Value::from(HashMap::from([("k", 1u32)]))),
        ("x-double", Value::F64(1.5)),
        ("x-bytes", Value::from(vec![0u8; 100_000])),
        ("urgency", Value::from("yüksek")),
    ]);
    proxy.notify("a", 0, "", "garip", "", &[], odd, -1).unwrap();
    assert_eq!(added(&events).urgency, Urgency::Normal);
}

#[test]
fn oversized_text_is_truncated() {
    let bus = PrivateBus::start();
    let (_server, events) = spawn(bus.options());
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();

    let body = "<i>".to_owned() + &"ğ".repeat(200_000) + "</i>";
    let summary = "s".repeat(10_000);
    let actions: Vec<String> = (0..500).map(|i| format!("a{i}")).collect();
    let action_refs: Vec<&str> = actions.iter().map(String::as_str).collect();
    proxy.notify(&"n".repeat(5000), 0, "", &summary, &body, &action_refs, HashMap::new(), -1).unwrap();
    let n = added(&events);
    assert_eq!(n.summary.len(), 256);
    assert_eq!(n.app_name.len(), 128);
    assert!(body_text(&n.body).len() <= 4 * 1024);
    assert!(n.body.iter().all(|s| s.style.italic));
    assert_eq!(n.actions.len(), 8);
}

#[test]
fn center_integration_transient_expiry_closes_over_dbus() {
    use nuxut_shell::services::notifications::{NotificationCenter, NotificationsConfig};

    let bus = PrivateBus::start();
    let (server, events) = spawn(bus.options());
    let handle = server.handle();
    let conn = bus.client();
    let sigs = signals(&conn);
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let mut center = NotificationCenter::new(NotificationsConfig { popup_timeout_ms: 100, ..Default::default() });

    let t = proxy.notify("a", 0, "", "geçici", "", &[], hints([("transient", Value::Bool(true))]), -1).unwrap();
    let kept = proxy.notify("a", 0, "", "kalıcı", "", &[], HashMap::new(), -1).unwrap();
    let start = Instant::now();
    center.on_event(wait_for(&events, "Added t", |_| true), start);
    center.on_event(wait_for(&events, "Added kept", |_| true), start);
    let deadline = center.next_deadline().unwrap();
    let mut expired = center.expire(deadline);
    expired.sort();
    assert_eq!(expired, {
        let mut v = vec![t, kept];
        v.sort();
        v
    });
    for (id, reason) in center.take_closed() {
        handle.close(id, reason);
    }
    assert_eq!(wait_for(&sigs, "NotificationClosed(t)", |_| true), Sig::Closed(t, 1));
    no_more(&sigs, Duration::from_millis(150));
    // Sunucudan dönen Closed olayı merkezde etkisiz (zaten yok).
    let closed = wait_for(&events, "Closed t", |_| true);
    assert!(!center.on_event(closed, Instant::now()));
    assert_eq!(center.history().map(|n| n.id).collect::<Vec<_>>(), vec![kept]);

    // Kullanıcı merkezden siler.
    let id = center.remove(kept).unwrap();
    handle.close(id, CloseReason::Dismissed);
    assert_eq!(wait_for(&sigs, "NotificationClosed(kept)", |_| true), Sig::Closed(kept, 2));
}

#[test]
fn name_taken_replace_and_release() {
    let bus = PrivateBus::start();
    let (first, first_events) = spawn(ServerOptions { allow_replacement: true, ..bus.options() });
    let first_handle = first.handle();
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let live = proxy.notify("a", 0, "", "ilk", "", &[], HashMap::new(), -1).unwrap();
    added(&first_events);

    // replace yok → NameTaken, sahibi raporlanır.
    let err = NotificationServer::spawn_with(bus.options(), |_| {}).unwrap_err();
    assert!(matches!(&err, NotifyError::NameTaken { owner } if owner.starts_with(':')), "{err}");

    // replace + karşı taraf izin veriyor → devralır; ilkinin canlı bildirimleri Undefined ile kapanır.
    let (second, second_events) = spawn(ServerOptions { replace: true, ..bus.options() });
    assert_eq!(
        wait_for(&first_events, "Closed(Undefined)", |_| true),
        Ev::Closed { id: live, reason: CloseReason::Undefined }
    );
    assert!(!first_handle.invoke_action(live, "default"));
    proxy.notify("a", 0, "", "ikinci", "", &[], HashMap::new(), -1).unwrap();
    added(&second_events);
    no_more(&first_events, Duration::from_millis(100));

    // ikinci izin vermiyor → replace de olsa alınamaz.
    let err = NotificationServer::spawn_with(ServerOptions { replace: true, ..bus.options() }, |_| {}).unwrap_err();
    assert!(matches!(err, NotifyError::NameTaken { .. }));

    // Düşürünce ad bırakılır; kalan handle etkisiz.
    let second_handle = second.handle();
    drop(first);
    let started = Instant::now();
    drop(second);
    assert!(started.elapsed() < Duration::from_secs(1));
    let dbus = blocking::fdo::DBusProxy::new(&conn).unwrap();
    assert!(!dbus.name_has_owner(BUS_NAME.try_into().unwrap()).unwrap());
    second_handle.close(1, CloseReason::Dismissed);
    // Yeniden alınabilir.
    let (_third, _) = spawn(bus.options());
}

#[test]
fn open_link_rejects_unsafe() {
    let bus = PrivateBus::start();
    let (server, _events) = spawn(bus.options());
    let handle = server.handle();
    assert!(!handle.open_link("file:///etc/passwd", None));
    assert!(!handle.open_link("javascript:alert(1)", None));
    // Özel bus'ta portal yok: istek arka planda başarısız olur, çağrı bloklamaz.
    let started = Instant::now();
    assert!(handle.open_link("https://example.org", Some("tok")));
    assert!(started.elapsed() < Duration::from_millis(500));
}

fn notify_send(bus: &PrivateBus, args: &[&str]) -> Command {
    let mut cmd = Command::new("notify-send");
    cmd.args(args).env("DBUS_SESSION_BUS_ADDRESS", &bus.address).env_remove("DBUS_SYSTEM_BUS_ADDRESS");
    cmd
}

#[test]
fn notify_send_real_client() {
    let bus = PrivateBus::start();
    let (server, events) = spawn(bus.options());

    let out =
        notify_send(&bus, &["-p", "-a", "nsapp", "-u", "critical", "-c", "email", "Başlık", "<b>gövde</b> &amp; x"])
            .output()
            .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let id: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    let n = added(&events);
    assert_eq!((n.id, n.app_name.as_str(), n.summary.as_str()), (id, "nsapp", "Başlık"));
    assert_eq!(n.urgency, Urgency::Critical);
    assert_eq!(n.category.as_deref(), Some("email"));
    assert_eq!(body_text(&n.body), "gövde & x");

    let out = notify_send(&bus, &["-p", "-r", &id.to_string(), "Güncel"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), id.to_string());
    let n = added(&events);
    assert_eq!((n.id, n.summary.as_str()), (id, "Güncel"));

    // Eylem: notify-send bekler, seçilen eylemi yazar ve çıkar.
    let child = notify_send(&bus, &["-A", "evet=Evet", "-A", "hayir=Hayır", "Onay?"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let q = added(&events);
    assert_eq!(q.actions.iter().map(|a| a.key.as_str()).collect::<Vec<_>>(), ["evet", "hayir"]);
    assert!(server.handle().invoke_action(q.id, "hayir"));
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hayir");
}

#[test]
fn image_path_decoded_and_downscaled() {
    let bus = PrivateBus::start();
    let (server, events) = spawn(bus.options());
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("shot.png");
    std::fs::write(&image, png(1000, 500)).unwrap();
    let icon = dir.path().join("app.png");
    std::fs::write(&icon, png(300, 300)).unwrap();
    let small = dir.path().join("small.svg");
    std::fs::write(&small, svg(40, 20)).unwrap();
    let garbage = dir.path().join("bozuk.png");
    std::fs::write(&garbage, b"\x89PNG bu bir png degil").unwrap();

    let id = proxy
        .notify(
            "a",
            0,
            icon.to_str().unwrap(),
            "görsel",
            "",
            &[],
            hints([("image-path", Value::from(format!("file://{}", image.display())))]),
            -1,
        )
        .unwrap();
    let (n, ready) = settled(&events, id, true, true);
    // Kart boyutuna küçültüldü: içerik ≤ 256, app simgesi 64 × ölçek; en-boy oranı korunur.
    assert_eq!(pixmap_size(&n.image), Some((256, 128, 256 * 128 * 4)));
    assert_eq!(pixmap_size(&n.app_icon), Some((64, 64, 64 * 64 * 4)), "ölçek 1: 64 px");
    // Varsayılan 100 ms içinde biten çözme `Added`'ı bekletir; ImageReady gerekmez.
    assert_eq!(ready, 0, "hızlı çözme Added ile birlikte gelmeliydi");

    // Çıkış ölçeği: 64 mantıksal × ölçek, üst sınır 128 (sonraki bildirimlerden itibaren).
    for (scale, side) in [(2.0, 128), (1.5, 96), (3.0, 128), (1.0, 64)] {
        server.handle().set_output_scale(scale);
        let id = proxy.notify("a", 0, icon.to_str().unwrap(), "ölçek", "", &[], HashMap::new(), -1).unwrap();
        let (n, _) = settled(&events, id, true, false);
        assert_eq!(pixmap_size(&n.app_icon), Some((side, side, (side * side * 4) as usize)), "ölçek {scale}");
    }

    // İstemcinin verdiği SVG güvenilmez (Trust::Untrusted): süreç içinde ayrıştırılmaz,
    // bildirim görselsiz gelir.
    let id = proxy
        .notify("a", 0, "", "svg", "", &[], hints([("image-path", Value::from(small.to_str().unwrap()))]), -1)
        .unwrap();
    let n = added(&events);
    assert_eq!((n.id, n.image.clone()), (id, NotifyIcon::None));

    // Çözülemeyen dosya: bildirim görselsiz gelir.
    let id = proxy
        .notify("a", 0, "", "bozuk", "", &[], hints([("image-path", Value::from(garbage.to_str().unwrap()))]), -1)
        .unwrap();
    let n = added(&events);
    assert_eq!((n.id, n.image.clone()), (id, NotifyIcon::None));
}

/// ADR-0007: Sandbox'lı sunucuda istemcinin SVG'si gerçek işçide çözülür; bozuk dosya yedeğe
/// (`NotifyIcon::None` → kartta semantik ikon) düşer; sunucu ve Sandbox düşünce işçi kalmaz.
#[test]
fn sandboxed_svg_is_decoded_and_broken_falls_back() {
    let bus = PrivateBus::start();
    let (_serial, sb) = sandbox_support::real_sandbox();
    let decoder = nuxut_shell::services::image_sandbox::isolated(&sb);
    let options = ServerOptions { sandbox: Some(decoder), ..bus.options() };
    let (server, events) = spawn(options);
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small.svg");
    std::fs::write(&small, svg(40, 20)).unwrap();
    let garbage = dir.path().join("bozuk.png");
    std::fs::write(&garbage, b"\x89PNG bu bir png degil").unwrap();

    let path_hint = |p: &std::path::Path| hints([("image-path", Value::from(p.to_str().unwrap().to_owned()))]);
    let id = proxy.notify("svg-app", 0, "", "svg", "", &[], path_hint(&small), -1).unwrap();
    let (n, _) = settled(&events, id, false, true);
    let (w, h, _) = pixmap_size(&n.image).expect("SVG işçide çözülmeli");
    assert!(w <= 256 && h <= 256 && w > 0 && h > 0, "{w}x{h}");
    // Uygulama simgesi olarak verilen SVG dosyası da (ayrı anahtar) işçide çözülür.
    let id = proxy.notify("svg-app", 0, small.to_str().unwrap(), "simge", "", &[], HashMap::new(), -1).unwrap();
    let (n, _) = settled(&events, id, true, false);
    assert!(pixmap_size(&n.app_icon).is_some(), "app_icon SVG");

    let id = proxy.notify("bozuk-app", 0, "", "bozuk", "", &[], path_hint(&garbage), -1).unwrap();
    let n = added(&events);
    assert_eq!((n.id, n.image.clone()), (id, NotifyIcon::None), "bozuk → yedek");

    let stats = sb.stats();
    assert!(stats.spawned >= 1 && stats.decoded >= 2, "{stats:?}");
    assert_eq!(sb.isolation(), nuxut_image::Isolation::Process);
    // Süreç çocuklarını saymak paralel testlerin işçilerini de sayar: bu Sandbox'ın kendi sayacı.
    assert!(sb.stats().live <= 2, "tek kullanımlık işçiler + en çok 1 önceden ısıtılmış: {:?}", sb.stats());
    drop(server);
    drop(sb);
    sandbox_support::assert_no_workers_left();
}

#[test]
fn slow_decode_adds_first_then_image_ready() {
    let bus = PrivateBus::start();
    let (server, events) = spawn(ServerOptions { image_wait: Some(Duration::ZERO), ..bus.options() });
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("shot.png");
    std::fs::write(&image, png(800, 800)).unwrap();
    let image_hint = || hints([("image-path", Value::from(image.to_str().unwrap()))]);

    // Bekleme 0: Added hemen (görselsiz olabilir), görsel ImageReady ile tamamlanır.
    let id = proxy.notify("a", 0, "", "yavaş", "", &[], image_hint(), -1).unwrap();
    let (n, _) = settled(&events, id, false, true);
    assert_eq!(pixmap_size(&n.image), Some((256, 256, 256 * 256 * 4)));

    // Kapanan bildirimin geç gelen sonucu atılır: Closed'dan sonra ImageReady yok.
    let id = proxy.notify("a", 0, "", "kapanacak", "", &[], image_hint(), -1).unwrap();
    server.handle().close(id, CloseReason::Dismissed);
    wait_for(&events, "Closed", |e| matches!(e, Ev::Closed { id: i, .. } if *i == id));
    let deadline = Instant::now() + Duration::from_millis(300);
    while let Ok(e) = events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        assert!(!matches!(e, Ev::ImageReady { id: i, .. } if i == id), "kapanmış bildirime görsel geldi");
    }

    // replaces_id: eski kuşağın sonucu yenisine yazılmaz; yenisi görselsiz ise görselsiz kalır.
    let id = proxy.notify("a", 0, "", "eski", "", &[], image_hint(), -1).unwrap();
    proxy.notify("a", id, "", "yeni", "", &[], HashMap::new(), -1).unwrap();
    let mut last_image = None;
    let deadline = Instant::now() + Duration::from_millis(400);
    while let Ok(e) = events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        match e {
            Ev::Added(n) if n.id == id => last_image = Some((n.summary.clone(), n.image.is_none())),
            Ev::ImageReady { id: i, image: Some(_), .. } if i == id => last_image = Some(("ImageReady".into(), false)),
            _ => {}
        }
    }
    assert_eq!(last_image, Some(("yeni".into(), true)));
}

#[test]
fn pending_image_budget_drops_image_and_is_returned() {
    use nuxut_shell::services::notifications::budget::MAX_PENDING_IMAGE_BYTES;

    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("big.png");
    let data = png(800, 800);
    let size = data.len();
    std::fs::write(&image, &data).unwrap();
    let image_hint = || hints([("image-path", Value::from(image.to_str().unwrap()))]);
    assert!(size < MAX_PENDING_IMAGE_BYTES);

    // Toplam bütçe tam bir görsellik: sığar, çözülür, bütçe geri verilir, ikincisi de sığar.
    let bus = PrivateBus::start();
    let (server, events) = spawn(ServerOptions { pending_image_budget: Some((size, size)), ..bus.options() });
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    for round in 0..2 {
        let id = proxy.notify("a", 0, "", &format!("#{round}"), "", &[], image_hint(), -1).unwrap();
        let (n, _) = settled(&events, id, false, true);
        assert!(!n.image.is_none(), "tur {round}: bütçe geri verildiyse görsel sığar");
        assert_eq!(server.handle().pending_image_bytes(), 0, "çözme bitince bütçe geri verilir");
    }
    drop((server, proxy));

    // Toplam bütçe aşımı: görsel düşer, bildirim görselsiz gelir; tema adlı simge etkilenmez.
    let bus = PrivateBus::start();
    let (server, events) = spawn(ServerOptions { pending_image_budget: Some((size - 1, usize::MAX)), ..bus.options() });
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let id = proxy.notify("a", 0, "dialog-information", "büyük", "", &[], image_hint(), -1).unwrap();
    let n = wait_for(&events, "Added", |e| matches!(e, Ev::Added(n) if n.id == id));
    let Ev::Added(n) = n else { unreachable!() };
    assert_eq!(n.image, NotifyIcon::None);
    assert_eq!(n.summary, "büyük");
    no_more_image(&events, id);
    assert_eq!(server.handle().pending_image_bytes(), 0);
    drop((server, proxy));

    // Bildirim başına sınır: toplam bütçe bol olsa da tek bildirimin görseli sınırı aşarsa düşer.
    let bus = PrivateBus::start();
    let (server, events) = spawn(ServerOptions { pending_image_budget: Some((usize::MAX, size - 1)), ..bus.options() });
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let id = proxy.notify("a", 0, "", "büyük", "", &[], image_hint(), -1).unwrap();
    let Ev::Added(n) = wait_for(&events, "Added", |e| matches!(e, Ev::Added(n) if n.id == id)) else { unreachable!() };
    assert_eq!(n.image, NotifyIcon::None);
    no_more_image(&events, id);
    assert_eq!(server.handle().pending_image_bytes(), 0);
}

/// `id` için içerik görseli gelmediğini doğrular (kısa bekleme).
fn no_more_image(rx: &Receiver<Ev>, id: u32) {
    let deadline = Instant::now() + Duration::from_millis(400);
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        assert!(!matches!(e, Ev::ImageReady { id: i, image: Some(_), .. } if i == id), "atılan görsel çözüldü");
    }
}

/// ADR-0007 bildirim anahtarları (`load_with_keys`): iptal anahtarı bildirim başına, kaynak
/// anahtarı GÖNDEREN başına (D-Bus'ın doğruladığı süreç kimliği; istemcinin `app_name`'i değil).
/// Düşmanca gönderen art arda bozuk görsel gönderince yalnızca KENDİ görselleri susturulur;
/// kurbanın `app_name`'ini taklit etmesi kurbanı susturmaz. Aynı gönderenin art arda gelen geçerli
/// bildirimleri birbirini iptal etmez.
#[test]
fn hostile_sender_is_muted_per_sender_and_cannot_mute_by_app_name() {
    let bus = PrivateBus::start();
    let (_serial, sb) = sandbox_support::real_sandbox();
    let decoder = nuxut_shell::services::image_sandbox::isolated(&sb);
    let options = ServerOptions { sandbox: Some(decoder), image_wait: Some(Duration::ZERO), ..bus.options() };
    let (server, events) = spawn(options);
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small.svg");
    std::fs::write(&small, svg(40, 20)).unwrap();
    let path_hint = |p: &std::path::Path| hints([("image-path", Value::from(p.to_str().unwrap().to_owned()))]);

    // Aynı uygulama, beklemeden art arda üç geçerli görsel: hepsi gelir (iptal anahtarı ayrı).
    let mut waiting: Vec<u32> = (0..3)
        .map(|i| proxy.notify("seri-app", 0, "", &format!("seri {i}"), "", &[], path_hint(&small), -1).unwrap())
        .collect();
    let deadline = Instant::now() + LIVENESS;
    while !waiting.is_empty() {
        let e = events.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("art arda bildirimler görselini kaybetti");
        let got = match e {
            Ev::Added(n) if !n.image.is_none() => Some(n.id),
            Ev::ImageReady { id, image: Some(_), .. } => Some(id),
            _ => None,
        };
        waiting.retain(|w| Some(*w) != got);
    }

    // Düşmanca gönderen: ayrı süreçler (`notify-send`; aynı ikili + cgroup = tek gönderen),
    // KURBANIN uygulama adıyla, her seferinde FARKLI bozuk bayt (negatif önbellek değil, susturma).
    let hostile = |summary: &str, image: &std::path::Path| -> u32 {
        let hint = format!("string:image-path:{}", image.display());
        let out = notify_send(&bus, &["-p", "-a", "kurban-app", "-h", &hint, summary]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    };
    for i in 0..6 {
        let bad = dir.path().join(format!("bozuk{i}.svg"));
        // İmza tanınır (işçiye gider), ayrıştırma işçide başarısız olur: kaynak anahtarına sayılır.
        std::fs::write(&bad, format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1{i}\" height=\"10\"><g>")).unwrap();
        let id = hostile("bozuk", &bad);
        let n = added(&events);
        assert_eq!(n.id, id);
        // Sonuca dayalı sıra: bu isteğin sonucu (başarısızlık ya da susturma) kapılara işlenmeden
        // sonraki gönderilmez. Eskiden 400 ms beklenip geçiliyordu; yükte çözme daha uzun sürünce
        // istekler üst üste biner, başarısızlıklar sayılmadan kapıdan geçer ve sayım kayardı.
        decodes_settled(&server);
        no_decoded_image(&events, id);
    }
    let muted = sb.stats().rate_limited;
    assert!(muted >= 3, "3 başarısızlıktan sonra gönderen susturulmalıydı: {:?}", sb.stats());
    // Susturulan gönderenin geçerli görseli de yedeğe düşer (kaynak = gönderen; yeni süreç kaçırmaz)...
    let id = hostile("gecerli", &small);
    let n = added(&events);
    assert_eq!((n.id, n.image.clone()), (id, NotifyIcon::None));
    decodes_settled(&server);
    no_decoded_image(&events, id);
    assert!(sb.stats().rate_limited > muted, "geçerli görsel de susturma kapısında durmalı: {:?}", sb.stats());
    // ...ama AYNI `app_name` ile gelen kurbanınki çözülür (susturma gönderenler arasında taşmaz).
    let id = proxy.notify("kurban-app", 0, "", "gecerli", "", &[], path_hint(&small), -1).unwrap();
    let (n, _) = settled(&events, id, false, true);
    assert_eq!(n.app_name, "kurban-app");
    assert!(pixmap_size(&n.image).is_some(), "uygulama adını taklit eden gönderen kurbanı susturdu");

    drop(server);
    drop(sb);
    sandbox_support::assert_no_workers_left();
}

/// Sunucunun bekleyen bütün görsel çözmeleri sonuçlanana dek bekler (sonuç tabanlı). Her dosya
/// görseli `Notify` yanıtından ÖNCE bayt bütçesinden ayırma alır ve ayırma ancak çözme sonucu
/// (başarı, başarısızlık ya da kapıda red) Sandbox'ın kapılarına işlendikten sonra düşer; sayaç
/// sıfırsa gönderilmiş her görselin sonucu sayılmıştır. Sınır yalnız canlılık içindir.
fn decodes_settled(server: &NotificationServer) {
    let deadline = Instant::now() + LIVENESS;
    while server.handle().pending_image_bytes() > 0 {
        assert!(Instant::now() < deadline, "görsel çözmesi sonuçlanmadı: {} bayt bekliyor", server.handle().pending_image_bytes());
        thread::sleep(Duration::from_millis(10));
    }
}

/// `id` için çözülmüş (yedek olmayan) içerik görseli gelmediğini doğrular; yedeğe düşüş
/// (`Some(NotifyIcon::None)`) serbest.
fn no_decoded_image(rx: &Receiver<Ev>, id: u32) {
    let deadline = Instant::now() + Duration::from_millis(400);
    while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        if let Ev::ImageReady { id: i, image: Some(icon), .. } = e {
            assert!(i != id || icon.is_none(), "susturulması gereken görsel çözüldü");
        }
    }
}

/// Fail-closed: kimliği çözülemeyen gönderenler (yanıt beklemeden çıkan kısa ömürlü süreçler —
/// sunucu PID'i sorduğunda bağlantı çoktan yok, her biri yeni benzersiz ad) ortak kovaya düşer.
/// 100 ayrı bağlantı aynı bozuk görseli gönderir: susturma/negatif önbellek devreye girer, işçi
/// yalnızca birkaç kez gerçekten çözmeye çalışır. Çözülebilen kurbanın görseli etkilenmez.
#[test]
fn unresolvable_short_lived_senders_share_one_muted_bucket() {
    if Command::new("busctl").arg("--version").output().is_err() {
        panic!("busctl yok (systemd); bu test onunla kısa ömürlü gönderen üretir");
    }
    let bus = PrivateBus::start();
    let (_serial, sb) = sandbox_support::real_sandbox();
    let decoder = nuxut_shell::services::image_sandbox::isolated(&sb);
    let options = ServerOptions { sandbox: Some(decoder), image_wait: Some(Duration::ZERO), ..bus.options() };
    let (server, events) = spawn(options);
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bozuk.svg");
    std::fs::write(&bad, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><g>").unwrap();
    let bad = bad.to_str().unwrap().to_owned();

    let before = sb.stats();
    const SENDERS: u64 = 100;
    for i in 0..SENDERS {
        // `--expect-reply=no`: gönderir ve hemen çıkar (bağlantı kapanır).
        let status = Command::new("busctl")
            .arg(format!("--address={}", bus.address))
            .args(["--expect-reply=no", "call", "--"])
            .args([BUS_NAME, "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "Notify"])
            .args(["susssasa{sv}i", "kurban-app", "0", "", &format!("sel {i}"), "", "0", "1", "image-path", "s", &bad, "-1"])
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
    let mut added_count = 0;
    while added_count < SENDERS {
        if let Ev::Added(_) = events.recv_timeout(LIVENESS).expect("Added gelmedi") {
            added_count += 1;
        }
    }
    // İşçiler tek kullanımlık: `spawned` gerçek çözme denemesi (+ önceden ısıtılmış boşta işçi).
    // Ortak kovada birkaç başarısızlıktan sonra gerisi kapıda durur (susturma/negatif önbellek);
    // fail-open'da her bağlantı yeni kova olur ve ~100 işçi açılırdı.
    let blocked = |s: &nuxut_shell::services::image_sandbox::SandboxStats| {
        (s.rate_limited + s.negative_hits) - (before.rate_limited + before.negative_hits)
    };
    let deadline = Instant::now() + LIVENESS;
    while blocked(&sb.stats()) < SENDERS - 10 {
        assert!(Instant::now() < deadline, "kısa ömürlü gönderenler susturulmadı (fail-open?): {:?}", sb.stats());
        thread::sleep(Duration::from_millis(50));
    }
    let s = sb.stats();
    assert!(s.spawned - before.spawned <= 10, "her kısa ömürlü bağlantı ayrı kova aldı (fail-open): {s:?}");

    // Çözülebilen gönderen (bu süreç), aynı uygulama adıyla: görseli çözülür.
    let small = dir.path().join("small.svg");
    std::fs::write(&small, svg(40, 20)).unwrap();
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let hint = hints([("image-path", Value::from(small.to_str().unwrap().to_owned()))]);
    let id = proxy.notify("kurban-app", 0, "", "gecerli", "", &[], hint, -1).unwrap();
    let (n, _) = settled(&events, id, false, true);
    assert!(pixmap_size(&n.image).is_some(), "ortak kova çözülebilen gönderene taştı");

    drop(server);
    drop(sb);
    sandbox_support::assert_no_workers_left();
}

/// Maskot yalnızca dahili bildirimlerde: D-Bus istemcisi uygulama adını "Nuxut" yapsa, maskot
/// adını hint/kategori/görsel olarak gönderse de bildirim maskotsuz gelir ve merkezde de öyle kalır.
#[test]
fn dbus_client_cannot_set_mascot() {
    use nuxut_shell::services::notifications::NotificationCenter;
    let bus = PrivateBus::start();
    let (_server, events) = spawn(bus.options());
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let id = proxy
        .notify(
            "Nuxut",
            0,
            "celebrate",
            "Nuxut güncellendi",
            "Sürüm 9.9.9",
            &[],
            hints([
                ("x-nuxut-mascot", Value::from("celebrate")),
                ("mascot", Value::from("Celebrate")),
                ("category", Value::from("x-nuxut.update")),
                ("image-path", Value::from("celebrate")),
            ]),
            -1,
        )
        .unwrap();
    let n = added(&events);
    assert_eq!(n.id, id);
    assert_eq!(n.mascot, None);
    let mut center = NotificationCenter::default();
    center.on_event(Ev::Added(n), Instant::now());
    assert_eq!(center.get(id).unwrap().mascot, None);
}
