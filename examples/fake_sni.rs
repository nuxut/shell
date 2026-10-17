//! Canlı tepsi testleri için sahte StatusNotifierItem uygulaması (isteğe bağlı dbusmenu ile).
//! Çağrıları stdout'a yazar. Yalnızca yalıtılmış bus içinde çalıştırın (`dbus-run-session`).
//!
//! ```sh
//! cargo run --example fake_sni -- Başlık [--no-menu] [--icon AD] [--secs N]
//! ```
//! Menü: Aç, ayraç, Seçenekler ▸ [Wi-Fi ✓, Bluetooth], Hakkında (devre dışı), Çıkış.

use std::collections::HashMap;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::{Array, Dict, OwnedObjectPath, OwnedValue, Signature, StructureBuilder, Value};

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const MENU_PATH: &str = "/MenuBar";

type RawPixmap = (i32, i32, Vec<u8>);

struct Item {
    title: String,
    icon_name: String,
    menu: bool,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn id(&self) -> String {
        "fake-sni".into()
    }
    #[zbus(property)]
    fn category(&self) -> String {
        "ApplicationStatus".into()
    }
    #[zbus(property)]
    fn status(&self) -> String {
        "Active".into()
    }
    #[zbus(property)]
    fn title(&self) -> String {
        self.title.clone()
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        self.icon_name.clone()
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<RawPixmap> {
        // Tema ikonu bulunamazsa görünür bir yedek: 16×16 düz renk (ARGB, ağ bayt sırası).
        vec![(16, 16, [0xff, 0x4c, 0x8b, 0xf5].repeat(256))]
    }
    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<RawPixmap>, String, String) {
        (String::new(), Vec::new(), self.title.clone(), "sahte öğe".into())
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(if self.menu { MENU_PATH } else { "/NO_DBUSMENU" }).unwrap()
    }

    fn activate(&self, x: i32, y: i32) {
        println!("Activate {x} {y}");
    }
    fn secondary_activate(&self, x: i32, y: i32) {
        println!("SecondaryActivate {x} {y}");
    }
    fn context_menu(&self, x: i32, y: i32) {
        println!("ContextMenu {x} {y}");
    }
    fn scroll(&self, delta: i32, orientation: &str) {
        println!("Scroll {delta} {orientation}");
    }
}

fn node(id: i32, props: &[(&str, Value<'static>)], children: Vec<Value<'static>>) -> Value<'static> {
    let mut d = Dict::new(&Signature::Str, &Signature::Variant);
    for (k, v) in props {
        d.append(Value::from(k.to_string()), Value::Value(Box::new(v.try_clone().unwrap()))).unwrap();
    }
    let mut a = Array::new(&Signature::Variant);
    for c in children {
        a.append(Value::Value(Box::new(c))).unwrap();
    }
    Value::Structure(StructureBuilder::new().add_field(id).append_field(Value::Dict(d)).append_field(Value::Array(a)).build().unwrap())
}

fn submenu() -> Vec<Value<'static>> {
    vec![
        node(4, &[("label", Value::from("Wi-Fi")), ("toggle-type", Value::from("checkmark")), ("toggle-state", Value::from(1i32))], vec![]),
        node(5, &[("label", Value::from("Bluetooth")), ("toggle-type", Value::from("checkmark")), ("toggle-state", Value::from(0i32))], vec![]),
    ]
}

fn root_children() -> Vec<Value<'static>> {
    vec![
        node(1, &[("label", Value::from("_Aç")), ("icon-name", Value::from("document-open"))], vec![]),
        node(2, &[("type", Value::from("separator"))], vec![]),
        node(3, &[("label", Value::from("_Seçenekler")), ("children-display", Value::from("submenu"))], submenu()),
        node(6, &[("label", Value::from("Hakkında")), ("enabled", Value::from(false))], vec![]),
        node(7, &[("label", Value::from("Çıkış"))], vec![]),
    ]
}

type Layout = (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>));

struct Menu;

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn get_layout(&self, parent: i32, _depth: i32, _props: Vec<String>) -> Layout {
        let (props, children) = match parent {
            0 => (HashMap::from([("children-display".to_owned(), OwnedValue::from(zbus::zvariant::Str::from("submenu")))]), root_children()),
            3 => (HashMap::new(), submenu()),
            _ => (HashMap::new(), Vec::new()),
        };
        (1, (parent, props, children.into_iter().map(|c| OwnedValue::try_from(c).unwrap()).collect()))
    }

    fn about_to_show(&self, id: i32) -> bool {
        println!("AboutToShow {id}");
        false
    }

    fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) {
        println!("Event {id} {event_id}");
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
}

fn main() -> zbus::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut title = "Sahte".to_owned();
    let mut icon_name = "network-wireless".to_owned();
    let mut menu = true;
    let mut secs = 60;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--no-menu" => menu = false,
            "--icon" => icon_name = args.next().unwrap_or_default(),
            "--secs" => secs = args.next().and_then(|s| s.parse().ok()).unwrap_or(secs),
            _ => title = a,
        }
    }
    let conn = Connection::session()?;
    conn.object_server().at("/StatusNotifierItem", Item { title, icon_name, menu })?;
    if menu {
        conn.object_server().at(MENU_PATH, Menu)?;
    }
    let unique = conn.unique_name().map(|n| n.to_string()).unwrap_or_default();
    conn.call_method(Some(WATCHER_NAME), WATCHER_PATH, Some(WATCHER_NAME), "RegisterStatusNotifierItem", &(unique.as_str(),))?;
    println!("kayıtlı: {unique}");
    std::thread::sleep(Duration::from_secs(secs));
    Ok(())
}
