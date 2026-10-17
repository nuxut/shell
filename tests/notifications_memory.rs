//! Bellek bütçesi ölçümü (200 bildirim, süreç RSS'i) ayrı test ikilisinde: RSS `/proc/self/statm`
//! ile süreç genelinde ölçülür; `notifications_dbus` içindeki zamanlama testleri aynı anda
//! çalışınca bu ölçümü ve süreyi bozuyordu. Bu ikilide tek test olduğu için ölçüm yalnızca onu
//! yansıtır. Ortak bus/proxy/yardımcılar `support/notify_bus.rs` içinde.

use std::time::{Duration, Instant};

use nuxut_shell::services::notifications::ServerOptions;
use zbus::zvariant::Value;

#[path = "support/notify_bus.rs"]
mod notify_bus;
use notify_bus::{NotificationsProxyBlocking, PrivateBus, hints, spawn};

#[path = "support/png.rs"]
mod png_fixture;
use png_fixture::png;

/// Resident set size (bayt), `/proc/self/statm`'den.
fn rss_bytes() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    statm.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok()).unwrap_or(0) * 4096
}

#[test]
fn memory_budget_200_notifications() {
    use nuxut_shell::services::notifications::{NotificationCenter, NotificationsConfig};

    let bus = PrivateBus::start();
    // Bu test modelin görsel belleğini ölçer: 200 bildirim aynı anda kuyruğa girer (~121 MiB
    // kodlanmış bayt). Kuyruk bütçesi (varsayılan 64 MiB) ayrı testte; burada hepsi sığsın.
    let options = ServerOptions { pending_image_budget: Some((usize::MAX, usize::MAX)), ..bus.options() };
    let (server, events) = spawn(options);
    let conn = bus.client();
    let proxy = NotificationsProxyBlocking::new(&conn).unwrap();
    let dir = tempfile::tempdir().unwrap();
    // En kötü durum: her bildirimde büyük içerik görseli + büyük app simgesi.
    let image = dir.path().join("big.png");
    // (2000² raster: 200 kez çözmek testi saniyeler içinde tutar; küçültme yolu aynı.)
    std::fs::write(&image, png(2000, 2000)).unwrap();
    let icon = dir.path().join("icon.png");
    std::fs::write(&icon, png(1024, 1024)).unwrap();

    let mut center = NotificationCenter::new(NotificationsConfig { history_limit: 200, ..Default::default() });
    let rss_before = rss_bytes();
    let mut ids = Vec::new();
    for i in 0..200 {
        let hint = hints([("image-path", Value::from(image.to_str().unwrap()))]);
        ids.push(proxy.notify("a", 0, icon.to_str().unwrap(), &format!("#{i}"), "", &[], hint, 0).unwrap());
    }
    // Olaylar iç içe gelir (Added / ImageReady); hepsi merkeze uygulanır.
    let budget = 200 * (256 * 256 * 4 + 64 * 64 * 4);
    let deadline = Instant::now() + Duration::from_secs(60);
    while center.image_bytes() < budget {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = events.recv_timeout(left).expect("görseller zamanında çözülmedi");
        center.on_event(event, Instant::now());
    }
    assert_eq!(ids.len(), 200);
    let model_bytes = center.image_bytes();
    let rss_after = rss_bytes();
    eprintln!(
        "200 bildirim: model görselleri {:.1} MiB (bütçe {:.1} MiB), RSS farkı {:.1} MiB",
        model_bytes as f64 / 1048576.0,
        budget as f64 / 1048576.0,
        rss_after.saturating_sub(rss_before) as f64 / 1048576.0
    );
    assert_eq!(center.history().count(), 200);
    assert_eq!(model_bytes, budget);
    assert_eq!(server.handle().pending_image_bytes(), 0, "çözmeler bitti: kuyruk bütçesi boş");
}
