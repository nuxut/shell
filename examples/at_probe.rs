//! Düşmanca yardımcı teknoloji (güvenlik testi, 2026-10-17): a11y bus'ındaki herhangi bir süreç
//! gibi davranır — Flatpak'teki bir uygulama da a11y bus'ına varsayılan olarak erişir. Adı
//! verilen düğümlere `DoAction` gönderir ve metin alanlarına `EditableText` ile yazmayı dener;
//! her denemenin sonucunu yazar. Erişim veren eylemlerin (dosya seçicinin kabulü, diyalog onayı)
//! AT'den TETİKLENEMEDİĞİNİ doğrulamak için.
//!
//! YALNIZ yalıtılmış ortamda (`framework/tools/a11y-e2e.sh`): kullanıcının a11y bus'ında
//! çalıştırmak onun uygulamalarına eylem göndermek demektir.
//!
//! ```sh
//! target/debug/examples/at_probe nuxut-filechooser --click secret.txt --action Aç --set-text Konum
//! target/debug/examples/at_probe nuxut-filechooser --gap 50 --click secret.txt --click secret.txt  # AT çift tıkı
//! ```
//!
//! `--inspect <ad>`: rol, arayüzler ve Text ile okunabilen metnin uzunluğu (içerik yazılmaz).
//! `--extents <ad>`: düğümün pencere (yüzey) koordinatında kutusu `x y genişlik yükseklik` (sentetik dokunma hedefi).
//!
//! Çıktı satırları: `action <ad>: ok|false|no-actions|yok|hata`, `set-text <ad>: <çağrı sonucu>; içerik YAZILDI|değişmedi`
//! (adaptörün cevabı değil, alanın yeniden okunan içeriği belirleyicidir).

#[cfg(target_os = "linux")]
fn main() {
    use std::time::{Duration, Instant};

    use atspi::proxy::accessible::AccessibleProxy;
    use atspi::proxy::action::ActionProxy;
    use atspi::proxy::editable_text::EditableTextProxy;
    use atspi::proxy::text::TextProxy;

    let mut app = String::new();
    let mut ops: Vec<(&'static str, String)> = Vec::new();
    let mut wait = 15u64;
    // Eylemler arası bekleme (ms); kısa değer çift tık aralığında art arda eylem dener.
    let mut gap = 400u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--click" | "--action" => ops.push(("action", args.next().unwrap_or_default())),
            "--set-text" => ops.push(("set-text", args.next().unwrap_or_default())),
            "--inspect" => ops.push(("inspect", args.next().unwrap_or_default())),
            "--extents" => ops.push(("extents", args.next().unwrap_or_default())),
            "--wait" => wait = args.next().and_then(|s| s.parse().ok()).unwrap_or(15),
            "--gap" => gap = args.next().and_then(|s| s.parse().ok()).unwrap_or(400),
            _ => app = a,
        }
    }

    /// (bus adı, yol, ad) — uygulamanın tüm düğümleri.
    async fn collect(bus: &zbus::Connection, dest: &str, path: &str, depth: usize, out: &mut Vec<(String, String, String)>) {
        let Ok(b) = AccessibleProxy::builder(bus).destination(dest.to_owned()) else { return };
        let Ok(b) = b.path(path.to_owned()) else { return };
        let Ok(acc) = b.cache_properties(zbus::proxy::CacheProperties::No).build().await else { return };
        out.push((dest.to_owned(), path.to_owned(), acc.name().await.unwrap_or_default()));
        if depth < 24 {
            for c in acc.get_children().await.unwrap_or_default() {
                let d = c.name_as_str().unwrap_or(dest).to_owned();
                Box::pin(collect(bus, &d, c.path_as_str(), depth + 1, out)).await;
            }
        }
    }

    let code = zbus::block_on(async {
        let session = zbus::Connection::session().await.expect("oturum bus'ı");
        session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "Set",
                &("org.a11y.Status", "IsEnabled", zbus::zvariant::Value::from(true)),
            )
            .await
            .expect("IsEnabled");
        let reply = session.call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &()).await.expect("adres");
        let address: String = reply.body().deserialize().expect("adres");
        let bus = zbus::connection::Builder::address(address.as_str()).expect("adres").build().await.expect("a11y bus");
        let root = AccessibleProxy::builder(&bus)
            .destination("org.a11y.atspi.Registry")
            .unwrap()
            .path("/org/a11y/atspi/accessible/root")
            .unwrap()
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(wait);
        let nodes = loop {
            let mut nodes = Vec::new();
            for a in root.get_children().await.unwrap_or_default() {
                let dest = a.name_as_str().unwrap_or_default().to_owned();
                let mut out = Vec::new();
                collect(&bus, &dest, a.path_as_str(), 0, &mut out).await;
                if out.first().is_some_and(|(_, _, n)| n.contains(app.as_str()) || app.is_empty()) || out.iter().any(|(_, _, n)| ops.iter().any(|(_, o)| o == n)) {
                    nodes.extend(out);
                }
            }
            if ops.iter().all(|(_, o)| nodes.iter().any(|(_, _, n)| n == o)) || Instant::now() >= deadline {
                break nodes;
            }
            async_io::Timer::after(Duration::from_millis(200)).await;
        };
        for (op, name) in &ops {
            let Some((dest, path, _)) = nodes.iter().find(|(_, _, n)| n == name) else {
                println!("{op} {name}: yok");
                continue;
            };
            let result = match *op {
                "action" => {
                    let p = ActionProxy::builder(&bus)
                        .destination(dest.clone())
                        .unwrap()
                        .path(path.clone())
                        .unwrap()
                        .cache_properties(zbus::proxy::CacheProperties::No)
                        .build()
                        .await
                        .unwrap();
                    // Saldırgan eylem listesine bakmadan dener: eylem sayısı ve DoAction(0) sonucu.
                    let n = p.get_actions().await.map(|a| a.len()).unwrap_or(0);
                    let r = match p.do_action(0).await {
                        Ok(b) => format!("{b}"),
                        Err(e) => format!("hata: {e}"),
                    };
                    format!("eylem {n}, DoAction {r}")
                }
                "extents" => {
                    // Pencere (yüzey) koordinatında kutu: x y genişlik yükseklik; canlı dokunma testinin hedefi.
                    let c = atspi::proxy::component::ComponentProxy::builder(&bus).destination(dest.clone()).unwrap().path(path.clone()).unwrap().build().await.unwrap();
                    match c.get_extents(atspi::CoordType::Window).await {
                        Ok((x, y, w, h)) => format!("{x} {y} {w} {h}"),
                        Err(e) => format!("hata: {e}"),
                    }
                }
                "inspect" => {
                    // Saldırganın göreceği: rol, sunulan arayüzler ve Text üzerinden okunabilen metin.
                    let a = AccessibleProxy::builder(&bus).destination(dest.clone()).unwrap().path(path.clone()).unwrap().build().await.unwrap();
                    let role = a.get_role_name().await.unwrap_or_default();
                    let ifaces = a.get_interfaces().await.map(|s| format!("{s:?}")).unwrap_or_default();
                    let t = TextProxy::builder(&bus).destination(dest.clone()).unwrap().path(path.clone()).unwrap().build().await.unwrap();
                    let text = match t.get_text(0, -1).await {
                        Ok(s) => format!("{} karakter", s.chars().count()),
                        Err(e) => format!("hata: {e}"),
                    };
                    format!("rol {role}, arayüzler {ifaces}, metin {text}")
                }
                _ => {
                    let p = EditableTextProxy::builder(&bus).destination(dest.clone()).unwrap().path(path.clone()).unwrap().build().await.unwrap();
                    let call = match p.set_text_contents("/etc/").await {
                        Ok(b) => format!("çağrı {b}"),
                        Err(e) => format!("hata: {e}"),
                    };
                    // Adaptörün cevabı değil, alanın gerçek içeriği önemli: yazıldı mı?
                    async_io::Timer::after(Duration::from_millis(400)).await;
                    let t = TextProxy::builder(&bus).destination(dest.clone()).unwrap().path(path.clone()).unwrap().build().await.unwrap();
                    let text = t.get_text(0, -1).await.unwrap_or_default();
                    format!("{call}; içerik {}", if text.contains("/etc/") { "YAZILDI" } else { "değişmedi" })
                }
            };
            println!("{op} {name}: {result}");
            async_io::Timer::after(Duration::from_millis(gap)).await;
        }
        0
    });
    std::process::exit(code);
}

#[cfg(not(target_os = "linux"))]
fn main() {}
