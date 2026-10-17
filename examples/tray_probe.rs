//! Tepsi servisini gerçek uygulamalarla denemek için: öğeleri ve menülerini loglar.
//!
//! Kullanıcı oturumuna dokunmamak için yalıtılmış bus içinde çalıştırın:
//! ```sh
//! dbus-run-session -- sh -c 'cargo run --example tray_probe -- 10 & sleep 1; nm-applet --indicator & blueman-applet & wait'
//! ```
//! Argüman: çalışma süresi (saniye, varsayılan 10). Her öğe eklendiğinde menüsü açılır,
//! ağaç yazdırılır.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use nuxut_shell::services::tray::{MenuNode, TrayEvent, TrayModel, TrayOptions, TrayService};

fn print_menu(node: &MenuNode, depth: usize) {
    for child in &node.children {
        let pad = "  ".repeat(depth + 1);
        let mut flags = Vec::new();
        if !child.enabled {
            flags.push("devre dışı".to_owned());
        }
        if !child.visible {
            flags.push("gizli".to_owned());
        }
        if let Some(kind) = child.toggle_kind {
            flags.push(format!("{kind:?}={:?}", child.toggle_state));
        }
        if !child.icon_name.is_empty() {
            flags.push(format!("ikon={}", child.icon_name));
        }
        if child.icon_png.is_some() {
            flags.push("png".to_owned());
        }
        if child.has_submenu() {
            flags.push("alt menü".to_owned());
        }
        if child.is_separator() {
            println!("{pad}──────");
        } else {
            println!("{pad}[{}] {} {}", child.id, child.label, if flags.is_empty() { String::new() } else { format!("({})", flags.join(", ")) });
        }
        print_menu(child, depth + 1);
    }
}

fn main() {
    tracing_subscriber::fmt().with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "nuxut_shell=debug".into())).init();
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let (tx, rx) = mpsc::channel();
    let service = TrayService::spawn(TrayOptions::default(), move |e| {
        let _ = tx.send(e);
    })
    .expect("tepsi thread'i başlatılamadı");
    let handle = service.handle();
    let mut model = TrayModel::default();
    let deadline = Instant::now() + Duration::from_secs(secs);
    while let Ok(event) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        model.apply(&event);
        match &event {
            TrayEvent::ItemAdded(item) | TrayEvent::ItemUpdated(item) => {
                let added = matches!(event, TrayEvent::ItemAdded(_));
                let pixmaps: Vec<String> = item.icon.pixmaps.all().iter().map(|p| format!("{}x{}", p.width, p.height)).collect();
                println!(
                    "{} {} id={} app_id={:?} title={:?} status={:?} category={:?} icon={:?} pixmaps={pixmaps:?} theme_path={:?} attention={:?} tooltip={:?} item_is_menu={} menu={:?}",
                    if added { "EKLENDİ" } else { "GÜNCELLENDİ" },
                    item.display_name(),
                    item.id,
                    item.app_id,
                    item.title,
                    item.status,
                    item.category,
                    item.icon.name,
                    item.icon_theme_path,
                    item.attention_icon.name,
                    item.tooltip,
                    item.item_is_menu,
                    item.menu,
                );
                if added && item.menu.is_some() {
                    handle.open_menu(&item.id, nuxut_shell::services::tray::next_menu_request());
                }
            }
            TrayEvent::MenuLayout { item, menu, .. } => {
                println!("MENÜ {item} rev={} düğüm={}", menu.revision, menu.root.count());
                print_menu(&menu.root, 0);
            }
            other => println!("{other:?}"),
        }
    }
    println!("SON: {} öğe", model.items().len());
    for item in model.items() {
        println!("  {} ({})", item.display_name(), item.id);
    }
}
