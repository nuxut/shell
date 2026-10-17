//! Tepsi servisi uçtan uca testleri. Her test kendi özel `dbus-daemon`'unu başlatır (kullanıcı
//! oturumuna asla dokunmaz, testler paralel çalışabilir) ve zbus ile yazılmış sahte
//! StatusNotifierItem + dbusmenu uygulamaları kullanır. `dbus-daemon` başlatılamazsa testler panikler
//! (yalnızca `NUXUT_SKIP_DBUS_TESTS=1` ise atlanır).
//!
//! Kapsam: watcher sahiplenme ve host yolu, watcher devralma, ikon güncelleme birleştirme,
//! menü okuma/yama/yeniden okuma, tıklama olayı, eylemler, öğenin bus'tan düşmesi, cevap
//! vermeyen uygulama, öğe sınırı, temiz kapanış.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nuxut_shell::services::tray::{
    ItemId, MAX_ITEMS, MenuEventKind, ScrollOrientation, Status, ToggleKind, TrayEvent, TrayModel, TrayOptions,
    TrayRole, TrayService, WATCHER_NAME, WATCHER_PATH, next_menu_request,
};
use zbus::blocking::Connection;
use zbus::zvariant::{Array, Dict, OwnedObjectPath, OwnedValue, Signature, StructureBuilder, Value};

#[path = "support/png.rs"]
mod png_fixture;
use png_fixture::solid_png;

#[path = "support/skip.rs"]
mod skip;

#[path = "support/sandbox.rs"]
mod sandbox_support;

const RED: [u8; 4] = [255, 0, 0, 255];

const WAIT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------------------
// Özel bus

struct PrivateBus {
    child: Child,
    address: String,
}

impl PrivateBus {
    fn start() -> Option<Self> {
        let mut child = match Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => return skip::skip_or_panic("dbus-daemon", skip::SKIP_DBUS, e),
        };
        let mut line = String::new();
        let _ = BufReader::new(child.stdout.take().unwrap()).read_line(&mut line);
        let address = line.trim().to_owned();
        if address.is_empty() {
            let _ = child.kill();
            let _ = child.wait();
            return skip::skip_or_panic("dbus-daemon", skip::SKIP_DBUS, "adres yazmadı");
        }
        Some(Self { child, address })
    }

    fn connect(&self) -> Connection {
        zbus::blocking::connection::Builder::address(self.address.as_str()).unwrap().build().unwrap()
    }

    fn options(&self) -> TrayOptions {
        TrayOptions {
            bus_address: Some(self.address.clone()),
            call_timeout: Duration::from_millis(400),
            coalesce_quiet: Duration::from_millis(30),
            coalesce_max_delay: Duration::from_millis(150),
            initial_read_attempts: 2,
            retry_delay: Duration::from_millis(50),
            icon_px: 32,
            min_refresh_interval: Duration::from_millis(100),
            sandbox: None,
        }
    }

    fn has_owner(&self, conn: &Connection, name: &str) -> bool {
        conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(name,))
            .unwrap()
            .body()
            .deserialize::<bool>()
            .unwrap()
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------------------
// Servis + olay alıcısı

struct Tray {
    service: TrayService,
    events: Receiver<TrayEvent>,
    model: TrayModel,
}

impl Tray {
    fn start(bus: &PrivateBus) -> Self {
        Self::start_with(bus.options())
    }

    fn start_with(options: TrayOptions) -> Self {
        let (tx, events) = mpsc::channel();
        let service = TrayService::spawn(options, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        Self { service, events, model: TrayModel::default() }
    }

    /// `pred` tutan ilk olayı bekler (aradakiler modele uygulanır).
    fn wait_for(&mut self, what: &str, mut pred: impl FnMut(&TrayEvent) -> bool) -> TrayEvent {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let event = self.events.recv_timeout(left).unwrap_or_else(|_| panic!("beklenen olay gelmedi: {what}"));
            self.model.apply(&event);
            if pred(&event) {
                return event;
            }
        }
    }

    /// `dur` boyunca gelen olaylar.
    fn collect(&mut self, dur: Duration) -> Vec<TrayEvent> {
        let deadline = Instant::now() + dur;
        let mut out = Vec::new();
        while let Ok(e) = self.events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            self.model.apply(&e);
            out.push(e);
        }
        out
    }

    fn added(&mut self, title: &str) -> ItemId {
        match self.wait_for(&format!("ItemAdded {title}"), |e| matches!(e, TrayEvent::ItemAdded(i) if i.title == title)) {
            TrayEvent::ItemAdded(i) => i.id,
            _ => unreachable!(),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Sahte uygulama

#[derive(Debug, Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn push(&self, s: String) {
        self.0.lock().unwrap().push(s);
    }

    fn wait_for(&self, entry: &str) {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if self.0.lock().unwrap().iter().any(|e| e == entry) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("uygulama çağrıyı almadı: {entry}; alınanlar: {:?}", self.0.lock().unwrap());
    }

    fn count(&self, prefix: &str) -> usize {
        self.0.lock().unwrap().iter().filter(|e| e.starts_with(prefix)).count()
    }
}

type RawPixmap = (i32, i32, Vec<u8>);

struct FakeItem {
    title: String,
    icon_name: String,
    status: String,
    pixmaps: Vec<RawPixmap>,
    menu: Option<OwnedObjectPath>,
    activate_fails: bool,
    theme_path: String,
    log: Log,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl FakeItem {
    #[zbus(property)]
    fn id(&self) -> String {
        "fake-app".into()
    }
    #[zbus(property)]
    fn category(&self) -> String {
        "Communications".into()
    }
    #[zbus(property)]
    fn status(&self) -> String {
        self.status.clone()
    }
    #[zbus(property)]
    fn title(&self) -> String {
        // Her GetAll bir kez okur: okuma sayacı.
        self.log.push("read".into());
        self.title.clone()
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> String {
        self.theme_path.clone()
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        self.icon_name.clone()
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<RawPixmap> {
        self.pixmaps.clone()
    }
    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<RawPixmap>, String, String) {
        (String::new(), Vec::new(), "İpucu".into(), "<b>kalın</b>".into())
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        self.menu.clone().unwrap_or_else(|| OwnedObjectPath::try_from("/NO_DBUSMENU").unwrap())
    }

    fn activate(&self, x: i32, y: i32) -> zbus::fdo::Result<()> {
        self.log.push(format!("Activate {x} {y}"));
        if self.activate_fails { Err(zbus::fdo::Error::NotSupported("menü kullan".into())) } else { Ok(()) }
    }
    fn secondary_activate(&self, x: i32, y: i32) {
        self.log.push(format!("SecondaryActivate {x} {y}"));
    }
    fn context_menu(&self, x: i32, y: i32) {
        self.log.push(format!("ContextMenu {x} {y}"));
    }
    fn scroll(&self, delta: i32, orientation: &str) {
        self.log.push(format!("Scroll {delta} {orientation}"));
    }
}

/// Basit bir dbusmenu: kök → [Aç, ayraç, Seçenekler ▸ [Wi-Fi ✓]].
struct FakeMenu {
    revision: u32,
    open_label: String,
    hang: bool,
    /// Her `GetLayout` yanıtından önce bekleme (yarış testleri).
    delay: Duration,
    log: Log,
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

type Layout = (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>));

impl FakeMenu {
    fn layout_children(&self) -> Vec<Value<'static>> {
        vec![
            node(1, &[("label", Value::from(self.open_label.clone())), ("icon-name", Value::from("menu-icon-symbolic"))], vec![]),
            node(2, &[("type", Value::from("separator"))], vec![]),
            node(
                3,
                &[("label", Value::from("_Seçenekler")), ("children-display", Value::from("submenu"))],
                vec![node(4, &[("label", Value::from("Wi-Fi")), ("toggle-type", Value::from("checkmark")), ("toggle-state", Value::from(1i32))], vec![])],
            ),
            // Geçersiz ad ve sahte PNG: çözülmez, menü yine gelir.
            node(9, &[("label", Value::from("kötü")), ("icon-name", Value::from("../../etc/passwd"))], vec![]),
            node(10, &[("label", Value::from("bozuk png")), ("icon-data", Value::from(b"\x89PNG\r\n\x1a\nbozuk".to_vec()))], vec![]),
        ]
    }
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl FakeMenu {
    async fn get_layout(&self, parent: i32, depth: i32, _props: Vec<String>) -> Layout {
        self.log.push(format!("GetLayout {parent} {depth}"));
        if !self.delay.is_zero() {
            async_io::Timer::after(self.delay).await;
        }
        if self.hang {
            async_io::Timer::after(Duration::from_secs(30)).await;
        }
        let children = self.layout_children();
        let (props, children) = if parent == 0 {
            (HashMap::from([("children-display".to_owned(), OwnedValue::from(zbus::zvariant::Str::from("submenu")))]), children)
        } else {
            // Alt ağaç isteği: yalnızca 3 destekleniyor.
            let Value::Structure(s) = &children[2] else { unreachable!() };
            let Value::Array(a) = &s.fields()[2] else { unreachable!() };
            let kids = a.inner().iter().map(|v| v.try_clone().unwrap()).collect();
            (HashMap::new(), kids)
        };
        let children = children.into_iter().map(|c| OwnedValue::try_from(c).unwrap()).collect();
        (self.revision, (parent, props, children))
    }

    fn about_to_show(&self, id: i32) -> bool {
        self.log.push(format!("AboutToShow {id}"));
        id == 3
    }

    fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) {
        self.log.push(format!("Event {id} {event_id}"));
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(signal)]
    async fn layout_updated(emitter: &zbus::object_server::SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;
}

struct FakeApp {
    conn: Connection,
    log: Log,
    path: &'static str,
}

#[derive(Clone)]
struct AppSpec {
    title: &'static str,
    path: &'static str,
    with_menu: bool,
    activate_fails: bool,
    menu_hangs: bool,
    menu_delay: Duration,
}

impl AppSpec {
    fn new(title: &'static str) -> Self {
        Self { title, path: "/StatusNotifierItem", with_menu: true, activate_fails: false, menu_hangs: false, menu_delay: Duration::ZERO }
    }
}

impl FakeApp {
    fn start(bus: &PrivateBus, spec: AppSpec) -> Self {
        let log = Log::default();
        let conn = bus.connect();
        let menu_path = "/MenuBar";
        let item = FakeItem {
            title: spec.title.into(),
            icon_name: "icon-a".into(),
            status: "Active".into(),
            pixmaps: vec![(1, 1, vec![0xff, 0x10, 0x20, 0x30]), (2, 2, vec![0x80; 16])],
            menu: spec.with_menu.then(|| OwnedObjectPath::try_from(menu_path).unwrap()),
            activate_fails: spec.activate_fails,
            theme_path: String::new(),
            log: log.clone(),
        };
        conn.object_server().at(spec.path, item).unwrap();
        if spec.with_menu {
            let menu = FakeMenu { revision: 1, open_label: "_Aç".into(), hang: spec.menu_hangs, delay: spec.menu_delay, log: log.clone() };
            conn.object_server().at(menu_path, menu).unwrap();
        }
        Self { conn, log, path: spec.path }
    }

    /// Klasik kayıt: iyi bilinen ad alır ve onu kaydeder.
    fn register_by_name(&self, name: &str) {
        self.conn.request_name(name).unwrap();
        self.register(name);
    }

    fn register(&self, service: &str) {
        self.conn
            .call_method(Some(WATCHER_NAME), WATCHER_PATH, Some("org.kde.StatusNotifierWatcher"), "RegisterStatusNotifierItem", &(service,))
            .unwrap();
    }

    fn unique(&self) -> String {
        self.conn.unique_name().unwrap().to_string()
    }

    fn signal(&self, path: &str, iface: &str, member: &str) {
        self.conn.emit_signal(None::<&str>, path, iface, member, &()).unwrap();
    }

    fn item(&self) -> zbus::blocking::object_server::InterfaceRef<FakeItem> {
        self.conn.object_server().interface::<_, FakeItem>(self.path).unwrap()
    }

    fn menu(&self) -> zbus::blocking::object_server::InterfaceRef<FakeMenu> {
        self.conn.object_server().interface::<_, FakeMenu>("/MenuBar").unwrap()
    }
}

fn registered_items(conn: &Connection) -> Vec<String> {
    let reply = conn
        .call_method(Some(WATCHER_NAME), WATCHER_PATH, Some("org.freedesktop.DBus.Properties"), "Get", &("org.kde.StatusNotifierWatcher", "RegisteredStatusNotifierItems"))
        .unwrap();
    Vec::<String>::try_from(reply.body().deserialize::<OwnedValue>().unwrap()).unwrap()
}

macro_rules! bus_or_skip {
    () => {
        match PrivateBus::start() {
            Some(bus) => bus,
            None => return,
        }
    };
}

// ---------------------------------------------------------------------------------------
// Testler

#[test]
fn watcher_role_item_icon_menu_and_removal() {
    let bus = bus_or_skip!();
    let mut tray = Tray::start(&bus);
    tray.wait_for("Role(Watcher)", |e| *e == TrayEvent::Role(TrayRole::Watcher));
    let client = bus.connect();
    assert!(bus.has_owner(&client, WATCHER_NAME));

    let app = FakeApp::start(&bus, AppSpec::new("Sahte"));
    app.register_by_name("org.kde.StatusNotifierItem-4242-1");
    let id = tray.added("Sahte");
    assert_eq!(id.as_str(), format!("{}/StatusNotifierItem", app.unique()));
    let item = tray.model.get(&id).unwrap().clone();
    assert_eq!(item.app_id, "fake-app");
    assert_eq!(item.status, Status::Active);
    assert_eq!(item.icon.name, "icon-a");
    assert_eq!(item.tooltip.title, "İpucu");
    assert_eq!(item.menu.as_deref(), Some("/MenuBar"));
    let px = item.icon.pixmaps.best(1).unwrap();
    assert_eq!((px.width, &px.rgba[..]), (1, &[0x10, 0x20, 0x30, 0xff][..]), "ARGB → RGBA");
    assert_eq!(item.icon.pixmaps.best(22).unwrap().width, 2);
    assert_eq!(registered_items(&client), ["org.kde.StatusNotifierItem-4242-1/StatusNotifierItem"]);

    // İkon güncellemesi: sinyal patlaması tek (en fazla iki) okumaya dönüşür.
    app.item().get_mut().icon_name = "icon-b".into();
    for member in ["NewIcon", "NewIcon", "NewToolTip", "NewIcon", "NewTitle"] {
        app.signal(app.path, "org.kde.StatusNotifierItem", member);
    }
    tray.wait_for("icon-b", |e| matches!(e, TrayEvent::ItemUpdated(i) if i.icon.name == "icon-b"));
    let extra = tray.collect(Duration::from_millis(300));
    assert!(extra.iter().all(|e| !matches!(e, TrayEvent::ItemUpdated(_))), "değişmeyen veri tekrar yayınlanmaz: {extra:?}");

    // Durum: NeedsAttention.
    app.item().get_mut().status = "NeedsAttention".into();
    app.signal(app.path, "org.kde.StatusNotifierItem", "NewStatus");
    tray.wait_for("NeedsAttention", |e| matches!(e, TrayEvent::ItemUpdated(i) if i.status == Status::NeedsAttention));

    // Menü.
    let handle = tray.service.handle();
    assert!(handle.open_menu(&id, next_menu_request()));
    let TrayEvent::MenuLayout { menu, .. } = tray.wait_for("menü", |e| matches!(e, TrayEvent::MenuLayout { .. })) else { unreachable!() };
    app.log.wait_for("AboutToShow 0");
    assert_eq!(menu.root.children.len(), 5);
    assert_eq!(menu.find(1).unwrap().label, "Aç");
    assert!(menu.find(2).unwrap().is_separator());
    let sub = menu.find(3).unwrap();
    assert!(sub.has_submenu() && sub.label == "Seçenekler" && sub.mnemonic == Some('s'));
    let wifi = menu.find(4).unwrap();
    assert_eq!(wifi.toggle_kind, Some(ToggleKind::Checkmark));
    assert!(wifi.is_checked());

    // Tıklama.
    assert!(handle.menu_event(&id, 1, MenuEventKind::Clicked));
    app.log.wait_for("Event 1 clicked");
    // Tıklama menüyü kapatır (servis izlemeyi bırakır); yeniden aç.
    assert!(handle.open_menu(&id, next_menu_request()));
    tray.wait_for("menü yeniden", |e| matches!(e, TrayEvent::MenuLayout { .. }));

    // LayoutUpdated → yeniden okuma.
    {
        let menu = app.menu();
        let mut m = menu.get_mut();
        m.revision = 2;
        m.open_label = "_Yeni".into();
    }
    zbus::block_on(FakeMenu::layout_updated(app.menu().signal_emitter(), 2, 0)).unwrap();
    tray.wait_for("yeni menü", |e| matches!(e, TrayEvent::MenuLayout { menu, .. } if menu.revision == 2 && menu.find(1).unwrap().label == "Yeni"));

    // ItemsPropertiesUpdated → yerinde yama (D-Bus çağrısı yok).
    let reads = app.log.count("GetLayout");
    let updated: Vec<(i32, HashMap<String, OwnedValue>)> =
        vec![(1, HashMap::from([("enabled".to_owned(), OwnedValue::from(false))]))];
    let removed: Vec<(i32, Vec<String>)> = vec![(4, vec!["toggle-state".into()])];
    app.conn.emit_signal(None::<&str>, "/MenuBar", "com.canonical.dbusmenu", "ItemsPropertiesUpdated", &(updated, removed)).unwrap();
    let TrayEvent::MenuLayout { menu, .. } = tray.wait_for("yamalı menü", |e| matches!(e, TrayEvent::MenuLayout { menu, .. } if !menu.find(1).unwrap().enabled)) else { unreachable!() };
    assert!(!menu.find(4).unwrap().is_checked());
    assert_eq!(app.log.count("GetLayout"), reads, "yama yeniden okuma gerektirmez");

    // Alt menü açılıyor: AboutToShow(3) true → alt ağaç okunur.
    assert!(handle.menu_about_to_show(&id, &[3]));
    app.log.wait_for("GetLayout 3 -1");
    tray.wait_for("alt ağaç", |e| matches!(e, TrayEvent::MenuLayout { .. }));

    // Menü kapalıyken değişiklik izlenmez.
    assert!(handle.close_menu(&id));
    tray.collect(Duration::from_millis(100));
    let reads = app.log.count("GetLayout");
    zbus::block_on(FakeMenu::layout_updated(app.menu().signal_emitter(), 3, 0)).unwrap();
    assert!(tray.collect(Duration::from_millis(300)).iter().all(|e| !matches!(e, TrayEvent::MenuLayout { .. })));
    assert_eq!(app.log.count("GetLayout"), reads);

    // Eylemler.
    assert!(handle.activate(&id, 10, 20));
    assert!(handle.secondary_activate(&id, 1, 2));
    assert!(handle.scroll(&id, -120, ScrollOrientation::Horizontal));
    assert!(handle.context_menu(&id, 3, 4));
    app.log.wait_for("Activate 10 20");
    app.log.wait_for("SecondaryActivate 1 2");
    app.log.wait_for("Scroll -120 horizontal");
    app.log.wait_for("ContextMenu 3 4");

    // Uygulama bus'tan düşer → öğe kalkar, watcher listesi boşalır.
    drop(app);
    tray.wait_for("ItemRemoved", |e| *e == TrayEvent::ItemRemoved(id.clone()));
    assert!(tray.model.items().is_empty());
    assert!(registered_items(&client).is_empty());

    // Temiz kapanış: ad bırakılır.
    let started = Instant::now();
    drop(tray);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!bus.has_owner(&client, WATCHER_NAME), "watcher adı bırakılmalı");
}

#[test]
fn host_role_with_external_watcher_and_takeover() {
    let bus = bus_or_skip!();
    let mut owner = Tray::start(&bus);
    owner.wait_for("Role(Watcher)", |e| *e == TrayEvent::Role(TrayRole::Watcher));
    let mut host = Tray::start(&bus);
    host.wait_for("Role(Host)", |e| *e == TrayEvent::Role(TrayRole::Host));

    // Kayıtlı host adı.
    let client = bus.connect();
    let names: Vec<String> = client
        .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    assert!(names.iter().any(|n| n.starts_with("org.kde.StatusNotifierHost-")), "{names:?}");

    // Ayatana tarzı yalnızca-yol kaydı; iki servis de görür.
    let spec = AppSpec { path: "/org/ayatana/NotificationItem/fake", ..AppSpec::new("Yol") };
    let app = FakeApp::start(&bus, spec);
    app.register("/org/ayatana/NotificationItem/fake");
    let id = owner.added("Yol");
    assert_eq!(host.added("Yol"), id);

    // Host da eylem gönderebilir.
    host.service.handle().activate(&id, 5, 6);
    app.log.wait_for("Activate 5 6");

    drop(app);
    owner.wait_for("owner ItemRemoved", |e| matches!(e, TrayEvent::ItemRemoved(_)));
    host.wait_for("host ItemRemoved", |e| matches!(e, TrayEvent::ItemRemoved(_)));

    // Watcher kapanınca host devralır ve yeni kayıtları kabul eder.
    drop(owner);
    host.wait_for("devralma", |e| *e == TrayEvent::Role(TrayRole::Watcher));
    let app = FakeApp::start(&bus, AppSpec::new("Sonra"));
    app.register(&app.unique());
    host.added("Sonra");
    assert_eq!(registered_items(&client), [format!("{}/StatusNotifierItem", app.unique())]);
}

#[test]
fn activate_failure_requests_menu() {
    let bus = bus_or_skip!();
    let mut tray = Tray::start(&bus);
    tray.wait_for("Role", |e| matches!(e, TrayEvent::Role(_)));
    let app = FakeApp::start(&bus, AppSpec { activate_fails: true, ..AppSpec::new("Menülü") });
    app.register(&app.unique());
    let id = tray.added("Menülü");
    tray.service.handle().activate(&id, 0, 0);
    let TrayEvent::MenuRequested { request, .. } =
        tray.wait_for("MenuRequested", |e| matches!(e, TrayEvent::MenuRequested { item, .. } if *item == id))
    else {
        unreachable!()
    };
    // Servis menüyü kendiliğinden, aynı istek kimliğiyle okumaya başlar.
    tray.wait_for("menü", |e| matches!(e, TrayEvent::MenuLayout { item, request: r, .. } if *item == id && *r == request));
    assert_eq!(tray.model.menu_request(), Some((request, &id)));

    // Tıklama izlemeyi bırakır: sonraki LayoutUpdated yeniden okuma yaptırmaz.
    tray.service.handle().menu_event(&id, 1, MenuEventKind::Clicked);
    app.log.wait_for("Event 1 clicked");
    let reads = app.log.count("GetLayout");
    zbus::block_on(FakeMenu::layout_updated(app.menu().signal_emitter(), 9, 0)).unwrap();
    assert!(tray.collect(Duration::from_millis(300)).iter().all(|e| !matches!(e, TrayEvent::MenuLayout { .. })));
    assert_eq!(app.log.count("GetLayout"), reads);
}

/// Kullanıcı yorumu 4 regresyonu ("sağ tık bazen menü açmıyor, hızlı tıklamada karışıyor"):
/// - okuma sürerken kapat + yeniden aç: eski açılışın geç yanıtı yayınlanmaz, yeni açılış
///   kendi yanıtını alır;
/// - aynı menüyü yeniden açmak aynı ağacı getirir ama model yine değişir (UI uyanır);
/// - başka öğenin menüsü açılınca öncekinin izlemesi bırakılır (tek menü).
#[test]
fn menu_requests_are_answered_per_open() {
    let bus = bus_or_skip!();
    let mut tray = Tray::start(&bus);
    tray.wait_for("Role", |e| matches!(e, TrayEvent::Role(_)));
    let app = FakeApp::start(&bus, AppSpec { menu_delay: Duration::from_millis(250), ..AppSpec::new("Gecikmeli") });
    app.register(&app.unique());
    let id = tray.added("Gecikmeli");
    let handle = tray.service.handle();

    // Okuma sürerken kapat ve hemen yeniden aç.
    let first = next_menu_request();
    assert!(handle.open_menu(&id, first));
    app.log.wait_for("GetLayout 0 -1");
    assert!(handle.close_menu(&id));
    let second = next_menu_request();
    assert!(handle.open_menu(&id, second));
    let events = tray.collect(Duration::from_millis(1500));
    let layouts: Vec<_> =
        events.iter().filter_map(|e| if let TrayEvent::MenuLayout { request, .. } = e { Some(*request) } else { None }).collect();
    assert!(!layouts.is_empty() && layouts.iter().all(|r| *r == second), "yalnızca geçerli açılışın yanıtı: {layouts:?}");
    assert_eq!(app.log.count("AboutToShow 0"), 2, "her açılış AboutToShow(0) yapar");

    // Aynı menüyü yeniden aç: ağaç aynı, istek yeni → model değişir.
    let before = tray.model.clone();
    assert!(handle.close_menu(&id));
    let third = next_menu_request();
    assert!(handle.open_menu(&id, third));
    let TrayEvent::MenuLayout { menu, .. } =
        tray.wait_for("yeniden açılış", |e| matches!(e, TrayEvent::MenuLayout { request, .. } if *request == third))
    else {
        unreachable!()
    };
    assert_eq!(before.menu(&id), Some(&menu), "ağaç aynı");
    assert_ne!(before, tray.model, "yeni açılışın yanıtı modeli değiştirir");
    assert_eq!(tray.model.menu_layout(&id).map(|(r, _)| r), Some(third));

    // Başka öğe açılınca bu öğenin değişiklikleri izlenmez.
    let other = FakeApp::start(&bus, AppSpec::new("Diğer"));
    other.register(&other.unique());
    let other_id = tray.added("Diğer");
    let fourth = next_menu_request();
    assert!(handle.open_menu(&other_id, fourth));
    tray.wait_for("diğer menü", |e| matches!(e, TrayEvent::MenuLayout { request, .. } if *request == fourth));
    let reads = app.log.count("GetLayout");
    zbus::block_on(FakeMenu::layout_updated(app.menu().signal_emitter(), 5, 0)).unwrap();
    let events = tray.collect(Duration::from_millis(500));
    assert!(events.iter().all(|e| !matches!(e, TrayEvent::MenuLayout { item, .. } if *item == id)), "{events:?}");
    assert_eq!(app.log.count("GetLayout"), reads);
}

#[test]
fn unresponsive_apps_cannot_block_the_service() {
    let bus = bus_or_skip!();
    // Uzun zaman aşımı: servis takılan çağrıları bekleseydi gecikme ≥ 1.5 s olurdu; yüklü
    // makinede bile normal yol bunun çok altında kalır.
    let timeout = Duration::from_millis(1500);
    let mut tray = Tray::start_with(TrayOptions { call_timeout: timeout, ..bus.options() });
    tray.wait_for("Role", |e| matches!(e, TrayEvent::Role(_)));

    // Hiç okuma yapmayan bağlantı: çağrılar asla cevaplanmaz.
    let dead = zbus::block_on(zbus::connection::Builder::address(bus.address.as_str()).unwrap().internal_executor(false).build()).unwrap();
    let dead_name = dead.unique_name().unwrap().to_string();
    let registrar = bus.connect();
    registrar
        .call_method(Some(WATCHER_NAME), WATCHER_PATH, Some("org.kde.StatusNotifierWatcher"), "RegisterStatusNotifierItem", &(dead_name.as_str(),))
        .unwrap();

    // Menüsü takılan uygulama.
    let slow = FakeApp::start(&bus, AppSpec { menu_hangs: true, ..AppSpec::new("Yavaş") });
    slow.register(&slow.unique());
    let slow_id = tray.added("Yavaş");
    tray.service.handle().open_menu(&slow_id, next_menu_request());

    // Takılı çağrılar sürerken servis döngüsü ve watcher arayüzü hemen cevap verir
    // (servis çağrıları bekleseydi gecikme zaman aşımı kadar olurdu). Ölçüm yalnızca
    // servisin kendisini kapsar; test sürecindeki sahte uygulamaların zamanlamasını değil.
    for _ in 0..5 {
        let started = Instant::now();
        let _ = registered_items(&registrar);
        assert!(started.elapsed() < timeout / 3, "watcher takıldı: {:?}", started.elapsed());
        std::thread::sleep(Duration::from_millis(50));
    }

    // Normal uygulama eklenir ve çalışır (yük altında ilk okuması zaman aşımına uğrasa bile
    // tekrar denenir).
    let ok = FakeApp::start(&bus, AppSpec::new("Normal"));
    ok.register(&ok.unique());
    let id = tray.added("Normal");
    tray.service.handle().activate(&id, 1, 1);
    ok.log.wait_for("Activate 1 1");

    // Cevapsız öğe zaman aşımı + denemeler sonunda listeden düşer, hiç eklenmez.
    let deadline = Instant::now() + WAIT * 2;
    while registered_items(&registrar).iter().any(|s| s.starts_with(&dead_name)) {
        assert!(Instant::now() < deadline, "cevapsız öğe düşürülmedi");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(tray.model.items().iter().all(|i| i.bus_name != dead_name));

    // Takılı çağrılar varken kapanış bekletmez.
    let started = Instant::now();
    drop(tray);
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    drop(dead);
}

#[test]
fn item_count_is_limited() {
    let bus = bus_or_skip!();
    let mut tray = Tray::start(&bus);
    tray.wait_for("Role", |e| matches!(e, TrayEvent::Role(_)));
    let conn = bus.connect();
    let log = Log::default();
    let paths: Vec<String> = (0..MAX_ITEMS + 6).map(|i| format!("/item/n{i}")).collect();
    for path in &paths {
        let item = FakeItem {
            title: path.clone(),
            icon_name: String::new(),
            status: "Passive".into(),
            pixmaps: Vec::new(),
            menu: None,
            activate_fails: false,
            theme_path: String::new(),
            log: log.clone(),
        };
        conn.object_server().at(path.as_str(), item).unwrap();
    }
    for path in &paths {
        conn.call_method(Some(WATCHER_NAME), WATCHER_PATH, Some("org.kde.StatusNotifierWatcher"), "RegisterStatusNotifierItem", &(path.as_str(),))
            .unwrap();
    }
    let events = tray.collect(Duration::from_millis(1500));
    let added = events.iter().filter(|e| matches!(e, TrayEvent::ItemAdded(_))).count();
    assert_eq!(added, MAX_ITEMS);
    assert_eq!(tray.model.items().len(), MAX_ITEMS);
    assert_eq!(tray.model.visible().count(), 0, "Passive öğeler görünür listede yok");
    let order: Vec<&str> = tray.model.items().iter().map(|i| i.title.as_str()).collect();
    let expected: Vec<&str> = paths.iter().take(MAX_ITEMS).map(String::as_str).collect();
    assert_eq!(order, expected, "kayıt sırası korunur");
}

#[test]
fn missing_bus_reports_failure() {
    let (tx, rx) = mpsc::channel();
    let options = TrayOptions { bus_address: Some("unix:path=/nonexistent/nuxut-test-bus".into()), ..TrayOptions::default() };
    let service = TrayService::spawn(options, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    assert!(matches!(rx.recv_timeout(WAIT), Ok(TrayEvent::Failed(_))));
    // Durmuş servise komut güvenle reddedilir.
    drop(service);
}

#[test]
fn theme_icons_are_resolved_in_background() {
    let bus = bus_or_skip!();
    let mut tray = Tray::start(&bus);
    tray.wait_for("Role", |e| matches!(e, TrayEvent::Role(_)));
    let app = FakeApp::start(&bus, AppSpec::new("Temalı"));
    app.register(&app.unique());
    let id = tray.added("Temalı");
    // "icon-a" temada yok: uygulamanın pixmap'i (32 px'e en uygun: 2x2) kullanılır.
    let image = tray.model.get(&id).unwrap().image.clone().expect("pixmap yedeği");
    assert_eq!((image.width, image.height), (2, 2));

    // Uygulamanın kendi ikon dizini (Electron tarzı düz dosya) → hedef boyutta çözülür.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app-icon.png"), solid_png(32, 32, RED)).unwrap();
    {
        let item = app.item();
        let mut it = item.get_mut();
        it.icon_name = "app-icon".into();
        it.theme_path = dir.path().to_string_lossy().into_owned();
    }
    app.signal(app.path, "org.kde.StatusNotifierItem", "NewIcon");
    tray.wait_for("32 px tema ikonu", |e| {
        matches!(e, TrayEvent::ItemUpdated(i) if i.image.as_ref().is_some_and(|p| p.width == 32 && p.rgba[..4] == [255, 0, 0, 255]))
    });

    // Boyut değişince yeniden çözülür.
    assert!(tray.service.handle().set_icon_size(16));
    tray.wait_for("16 px tema ikonu", |e| matches!(e, TrayEvent::ItemUpdated(i) if i.image.as_ref().is_some_and(|p| p.width == 16)));
}

/// ADR-0007: uygulamanın kendi dizinindeki SVG (Electron/Flatpak tarzı) Sandbox'lı serviste
/// gerçek işçide çözülür; bozuk dosya öğenin pixmap'ine düşer; servis ve Sandbox düşünce işçi
/// süreci kalmaz.
#[test]
fn app_svg_icon_is_decoded_in_sandbox() {
    let bus = bus_or_skip!();
    let (_serial, sb) = sandbox_support::real_sandbox();
    let mut tray = Tray::start_with(TrayOptions { sandbox: Some(nuxut_shell::services::image_sandbox::isolated(&sb)), ..bus.options() });
    tray.wait_for("Role", |e| matches!(e, TrayEvent::Role(_)));
    let app = FakeApp::start(&bus, AppSpec::new("Vektör"));
    app.register(&app.unique());
    let id = tray.added("Vektör");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("vec-icon.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="red"/></svg>"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("broken-icon.png"), b"\x89PNG\r\n\x1a\nbozuk").unwrap();
    let set_icon = |name: &str| {
        {
            let item = app.item();
            let mut it = item.get_mut();
            it.icon_name = name.into();
            it.theme_path = dir.path().to_string_lossy().into_owned();
        }
        app.signal(app.path, "org.kde.StatusNotifierItem", "NewIcon");
    };
    set_icon("vec-icon");
    tray.wait_for("işçide çözülen SVG", |e| {
        matches!(e, TrayEvent::ItemUpdated(i) if i.image.as_ref().is_some_and(|p| p.width == 32 && p.rgba[..4] == [255, 0, 0, 255]))
    });
    let stats = sb.stats();
    assert!(stats.spawned >= 1 && stats.decoded >= 1, "{stats:?}");

    // Bozuk → uygulamanın pixmap'i (2x2).
    set_icon("broken-icon");
    tray.wait_for("pixmap yedeği", |e| {
        matches!(e, TrayEvent::ItemUpdated(i) if i.image.as_ref().is_some_and(|p| (p.width, p.height) == (2, 2)))
    });
    assert_eq!(tray.model.get(&id).unwrap().image.as_ref().map(|p| (p.width, p.height)), Some((2, 2)));

    drop(app);
    drop(tray);
    drop(sb);
    sandbox_support::assert_no_workers_left();
}

#[test]
fn signal_storm_is_rate_limited() {
    let bus = bus_or_skip!();
    let mut tray = Tray::start(&bus);
    tray.wait_for("Role", |e| matches!(e, TrayEvent::Role(_)));
    let app = FakeApp::start(&bus, AppSpec::new("Fırtına"));
    app.register(&app.unique());
    tray.added("Fırtına");
    let before = app.log.count("read");
    let started = Instant::now();
    let mut sent = 0;
    while started.elapsed() < Duration::from_secs(1) {
        app.signal(app.path, "org.kde.StatusNotifierItem", "NewIcon");
        sent += 1;
    }
    std::thread::sleep(Duration::from_millis(400));
    let reads = app.log.count("read") - before;
    let elapsed_ms = started.elapsed().as_millis() as usize;
    // En fazla 100 ms'de bir okuma (+ önde kenar ve sondaki birleşik okuma payı).
    assert!(reads <= elapsed_ms / 100 + 3, "{sent} sinyal → {reads} okuma ({elapsed_ms} ms)");
    assert!(sent > 100, "fırtına üretilemedi: {sent}");
    // Servis hâlâ cevap veriyor.
    let client = bus.connect();
    let t = Instant::now();
    let _ = registered_items(&client);
    assert!(t.elapsed() < Duration::from_millis(500));
}

#[test]
fn thousand_registrations_are_bounded() {
    let bus = bus_or_skip!();
    let mut tray = Tray::start(&bus);
    tray.wait_for("Role", |e| matches!(e, TrayEvent::Role(_)));
    let conn = bus.connect();
    let mut rejected = 0;
    for i in 0..1000 {
        let path = format!("/flood/n{i}");
        let r = conn.call_method(Some(WATCHER_NAME), WATCHER_PATH, Some("org.kde.StatusNotifierWatcher"), "RegisterStatusNotifierItem", &(path.as_str(),));
        rejected += usize::from(r.is_err());
    }
    let client = bus.connect();
    let t = Instant::now();
    let items = registered_items(&client);
    assert!(t.elapsed() < Duration::from_millis(500), "servis takıldı");
    assert!(items.len() <= MAX_ITEMS, "{} kayıt", items.len());
    // Hiçbiri gerçek nesne değil: hiç öğe eklenmez, okuma denemeleri bitince liste boşalır.
    let deadline = Instant::now() + WAIT * 2;
    while !registered_items(&client).is_empty() {
        assert!(Instant::now() < deadline, "sahte kayıtlar düşürülmedi");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(tray.collect(Duration::from_millis(100)).iter().all(|e| !matches!(e, TrayEvent::ItemAdded(_))));
    let _ = rejected;
}

#[test]
fn hostile_icon_paths_fall_back_to_pixmap() {
    let bus = bus_or_skip!();
    let mut tray = Tray::start(&bus);
    tray.wait_for("Role", |e| matches!(e, TrayEvent::Role(_)));
    let app = FakeApp::start(&bus, AppSpec::new("Kötü"));
    {
        let item = app.item();
        let mut it = item.get_mut();
        it.icon_name = "../../etc/passwd".into();
        it.theme_path = "/proc/self".into();
    }
    app.register(&app.unique());
    let id = tray.added("Kötü");
    let image = tray.model.get(&id).unwrap().image.clone().unwrap();
    assert_eq!((image.width, image.height), (2, 2), "uygulamanın pixmap'i");
    for (name, path) in [("environ", "/proc/self"), ("/proc/self/environ", ""), ("/etc/passwd", ""), ("passwd", "/etc")] {
        {
            let item = app.item();
            let mut it = item.get_mut();
            it.icon_name = name.into();
            it.theme_path = path.into();
        }
        app.signal(app.path, "org.kde.StatusNotifierItem", "NewIcon");
        std::thread::sleep(Duration::from_millis(300));
        tray.collect(Duration::from_millis(100));
        let image = tray.model.get(&id).unwrap().image.clone().unwrap();
        assert_eq!((image.width, image.height), (2, 2), "{name} / {path}");
    }
}

#[test]
fn menu_icons_are_resolved_safely() {
    let bus = bus_or_skip!();
    let mut tray = Tray::start(&bus);
    tray.wait_for("Role", |e| matches!(e, TrayEvent::Role(_)));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("menu-icon-symbolic.png"), solid_png(32, 32, RED)).unwrap();
    let app = FakeApp::start(&bus, AppSpec::new("İkonlu menü"));
    app.item().get_mut().theme_path = dir.path().to_string_lossy().into_owned();
    app.register(&app.unique());
    let id = tray.added("İkonlu menü");
    tray.service.handle().open_menu(&id, next_menu_request());
    // Önce ikonsuz ağaç gelir; ikon çözülünce yeni MenuLayout.
    let TrayEvent::MenuLayout { menu, .. } = tray.wait_for("ikonlu menü", |e| {
        matches!(e, TrayEvent::MenuLayout { menu, .. } if menu.find(1).is_some_and(|n| n.image.is_some()))
    }) else {
        unreachable!()
    };
    let img = menu.find(1).unwrap().image.clone().unwrap();
    assert_eq!(&img.rgba[..4], &[255, 0, 0, 255]);
    assert!(nuxut_shell::services::tray::is_symbolic(menu.find(1).unwrap()));
    assert!(menu.find(9).unwrap().image.is_none(), "geçersiz ad");
    assert!(menu.find(10).unwrap().image.is_none(), "bozuk PNG");
    let model = nuxut_shell::services::tray::menu_model(&menu);
    assert_eq!(model.items.len(), 5);
}
