//! Servis döngüsü: bağlantı, sinyal yönlendiricileri, kayıt/okuma/menü durum makinesi.
//!
//! Kural: [`Runtime`] metotları hiçbir zaman D-Bus cevabı beklemez. Çağrılar
//! [`Runtime::spawn`] ile ayrı görevlerde yapılır ve sonuç [`Input`] olarak döner. Sinyal
//! yönlendiricileri ana döngüye sınırlı kanalla yazar; döngü yalnızca bellek içi iş yaptığı
//! için kanal hızla boşalır, zbus'un okuma kuyruğu tıkanmaz.

use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};
use async_executor::LocalExecutor;
use futures_lite::StreamExt;
use futures_lite::future::{self, FutureExt};
use zbus::fdo::{DBusProxy, RequestNameFlags, RequestNameReply};
use zbus::message::Type as MessageType;
use zbus::object_server::SignalEmitter;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{DynamicType, OwnedValue, Value};
use zbus::{Connection, MatchRule, Message, MessageStream};


use super::icons::{IconKey, IconSource, IconWorker, JobHandle, clamp_px, pixmap_fallback};
use crate::services::image_sandbox::{KeyDomain, source_key};
use super::item::{ITEM_PROPERTIES, ItemId, Pixmap, TrayItem};
use super::menu::{LayoutReply, MenuNode, MenuTree, PropertiesUpdate};
use super::watcher::{
    ItemRegistration, RegisteredItems, Registration, WATCHER_INTERFACE, WATCHER_NAME, WATCHER_PATH, Watcher,
    is_unique_name, parse_registration,
};
use super::{Command, MAX_ITEMS, MenuEventKind, MenuRequest, TrayEvent, TrayOptions, TrayRole, next_menu_request};

const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
const MENU_INTERFACE: &str = "com.canonical.dbusmenu";
const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
const DBUS_NAME: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";
/// Uygulamaların öğe için aldığı iyi bilinen ad önekleri (keşif için).
const ITEM_NAME_PREFIXES: [&str; 3] =
    ["org.kde.StatusNotifierItem-", "org.freedesktop.StatusNotifierItem-", "org.ayatana.StatusNotifierItem-"];

const INPUT_CAPACITY: usize = 1024;
const REGISTRATION_CAPACITY: usize = 64;
const NOTICE_CAPACITY: usize = 256;
/// Sinyal akışı başına zbus kuyruğu.
const SIGNAL_QUEUE: usize = 256;
/// Aynı anda çözülen (iyi bilinen ad → benzersiz ad) kayıt sınırı.
const MAX_PENDING_RESOLVES: usize = 32;
const CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
/// Bu sayıdan fazla sinyal/sn gönderen öğe loglanır (iş zaten hız sınırlı).
const SIGNAL_STORM_PER_SEC: u32 = 100;

pub(crate) type Sink = Box<dyn Fn(TrayEvent) + Send>;
type Props = HashMap<String, OwnedValue>;

/// Ana döngüye gelen iç olaylar.
#[derive(Debug)]
enum Input {
    NameOwnerChanged { name: String, new_owner: Option<String> },
    ItemSignal { sender: String, path: String },
    MenuSignal { sender: String, path: String, member: String, message: Message },
    WatcherSignal { sender: String, member: String, service: String },
    Resolved { registration: Registration, owner: Option<String> },
    Discovered(Vec<String>),
    External { owner: Option<String>, items: Vec<String> },
    Takeover(bool),
    ItemRead { id: ItemId, result: zbus::Result<Props> },
    /// `session`: okumanın başladığı menü açılışı (eskiyse sonuç atılır).
    MenuFetched { id: ItemId, session: MenuRequest, parent: i32, result: zbus::Result<LayoutReply> },
    MenuUpdatesNeeded { id: ItemId, session: MenuRequest, parents: Vec<i32> },
    ActionFailed { id: ItemId },
    /// Tema ikonu çözüldü (`None`: bulunamadı / çözülemedi).
    IconLoaded { id: ItemId, key: IconKey, image: Option<Pixmap> },
}

/// Watcher rolündeyken yayınlanacak sinyaller (sırayı korumak için tek görevden).
#[derive(Debug)]
enum Notice {
    Registered(String),
    Unregistered(String),
    HostRegistered,
}

// ---------------------------------------------------------------------------------------
// Birleştirme

/// Öğe başına "yeniden oku" durumu: kirli işareti sessizlik penceresi sonunda (en geç
/// `max_delay` içinde) tek okumaya dönüşür; aynı anda en fazla bir okuma ve iki okumanın
/// başlangıcı arasında en az `min_interval` (sinyal fırtınası hız sınırı).
#[derive(Debug, Default)]
struct Refresh {
    due: Option<Instant>,
    first: Option<Instant>,
    inflight: bool,
    /// Son okumanın başlangıcı (önde kenar kararı ve hız sınırı için).
    last_start: Option<Instant>,
    min_interval: Duration,
}

impl Refresh {
    fn with_min_interval(min_interval: Duration) -> Self {
        Self { min_interval, ..Self::default() }
    }

    /// Kirli işaretle. Boştayken (bekleyen/süren okuma yok, son okumadan beri `quiet`
    /// geçti) hemen okunur (önde kenar); patlamanın kalanı tek okumaya birleşir.
    fn mark(&mut self, now: Instant, quiet: Duration, max_delay: Duration) {
        let idle = !self.inflight
            && self.due.is_none()
            && self.last_start.is_none_or(|t| now.saturating_duration_since(t) >= quiet);
        if idle {
            self.due = Some(now);
            return;
        }
        let first = *self.first.get_or_insert(now);
        self.due = Some((now + quiet).min(first + max_delay));
    }

    fn now(&mut self, now: Instant) {
        self.due = Some(now);
    }

    fn at(&mut self, at: Instant) {
        self.due = Some(at);
        self.first = None;
    }

    /// Vakti geldiyse başlatır.
    fn take(&mut self, now: Instant) -> bool {
        if self.deadline().is_none_or(|d| d > now) {
            return false;
        }
        self.due = None;
        self.first = None;
        self.inflight = true;
        self.last_start = Some(now);
        true
    }

    /// Uyanma zamanı: bekleyen iş varsa, hız sınırı dolduktan sonra.
    fn deadline(&self) -> Option<Instant> {
        if self.inflight {
            return None;
        }
        let earliest = self.last_start.map(|t| t + self.min_interval);
        self.due.map(|d| earliest.map_or(d, |e| d.max(e)))
    }
}

#[derive(Debug, Default)]
struct MenuState {
    /// UI menüyü açık tutuyor: değişiklikler izlenir.
    open: bool,
    tree: Option<MenuTree>,
    /// Okunacak alt ağaç (0: tamamı).
    pending: Option<i32>,
    /// Sonraki okumadan önce `AboutToShow(0)` çağrılsın (yalnızca açılışta; her okumada
    /// çağırmak bazı uygulamalarda `LayoutUpdated` döngüsü yaratır).
    about_to_show: bool,
    fetch: Refresh,
    /// Yerinde yamanmış ağacın yayın zamanı.
    emit_due: Option<Instant>,
    /// Geçerli açılışın kimliği (UI'nın `OpenMenu` isteği). Okumalar başladıkları açılışı
    /// taşır; kapanıştan ya da yeni açılıştan sonra gelen eski sonuç atılır.
    session: MenuRequest,
    /// Geçerli açılış için en az bir ağaç yayınlandı mı.
    answered: bool,
    /// Geçerli açılışın ilk okumasının üst üste başarısızlıkları (yeniden deneme bütçesi).
    failures: u32,
}

impl MenuState {
    /// Yeni açılış: önceki açılışın süren okuması beklenmez (sonucu `session` tutmadığı
    /// için atılacak); yeni okuma hemen (hız sınırı içinde) başlar.
    fn open(&mut self, session: MenuRequest, now: Instant) {
        self.open = true;
        self.session = session;
        self.answered = false;
        self.failures = 0;
        self.about_to_show = true;
        self.pending = Some(0);
        self.emit_due = None;
        self.fetch.inflight = false;
        self.fetch.now(now);
    }

    /// İzlemeyi bırakır. Süren okuma sürer ama sonucu atılır.
    fn close(&mut self) {
        self.open = false;
        self.pending = None;
        self.fetch.due = None;
        self.emit_due = None;
    }

    /// Bu açılışa ait sonuç hâlâ geçerli mi?
    fn is_current(&self, session: MenuRequest) -> bool {
        self.open && self.session == session
    }

    fn request(&mut self, parent: i32) {
        self.pending = match self.pending {
            _ if self.tree.is_none() => Some(0),
            None => Some(parent),
            Some(p) if p == parent => Some(p),
            Some(_) => Some(0),
        };
    }
}

#[derive(Debug)]
struct ItemState {
    id: ItemId,
    owner: String,
    path: String,
    /// Kayıttaki bus adı (iyi bilinen olabilir) ve watcher listesindeki dizge.
    registered_bus: String,
    registered_as: String,
    seq: u64,
    item: Option<TrayItem>,
    read: Refresh,
    failures: u32,
    menu: MenuState,
    /// Çizilecek tema ikonunun anahtarı (`None`: ad yok, pixmap kullanılıyor).
    icon_key: Option<IconKey>,
    /// Çözülmüş ikonlar (öğe + menü öğeleri); `None` değer: çözülemedi. Sınırlı.
    icon_cache: HashMap<IconKey, Option<Pixmap>>,
    /// Süren ikon işi (öğe başına en fazla bir) ve anahtarı.
    icon_job: Option<(IconKey, JobHandle)>,
    /// Açık menünün çözülmeyi bekleyen ikonları (sınırlı; öğe ikonundan sonra sırayla).
    menu_icons_wanted: Vec<IconKey>,
    /// Sinyal fırtınası günlüğü: (pencere başlangıcı, penceredeki sinyal sayısı).
    signals: (Instant, u32),
}

// ---------------------------------------------------------------------------------------
// Giriş

pub(crate) fn run(
    options: TrayOptions,
    host_name: String,
    sink: Sink,
    stop: Receiver<()>,
    commands: Receiver<Command>,
) {
    let ex = Rc::new(LocalExecutor::new());
    let spawner = Rc::clone(&ex);
    future::block_on(ex.run(async move {
        let conn = match connect(&options).await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!("tepsi: D-Bus oturum bus'ına bağlanılamadı: {e}");
                sink(TrayEvent::Failed(e.to_string()));
                return;
            }
        };
        {
            let conn = conn.clone();
            spawner
                .spawn(async move {
                    loop {
                        conn.executor().tick().await;
                    }
                })
                .detach();
        }
        let (input_tx, input_rx) = async_channel::bounded(INPUT_CAPACITY);
        let (reg_tx, reg_rx) = async_channel::bounded(REGISTRATION_CAPACITY);
        let mut rt = Runtime {
            ex: Rc::clone(&spawner),
            conn: conn.clone(),
            icons: IconWorker::new(options.sandbox.clone()),
            dropped_registrations: 0,
            icon_px: clamp_px(options.icon_px),
            options,
            host_name,
            sink,
            input: input_tx,
            registrations: reg_tx,
            registered: RegisteredItems::default(),
            notices: None,
            role: TrayRole::Host,
            watcher_owner: None,
            items: Vec::new(),
            next_seq: 0,
            pending_resolves: 0,
        };
        match rt.setup().await {
            Ok(()) => rt.run(stop, commands, reg_rx, input_rx).await,
            Err(e) => {
                tracing::warn!("tepsi servisi başlatılamadı: {e}");
                (rt.sink)(TrayEvent::Failed(e.to_string()));
            }
        }
        drop(rt);
        // Bağlantıyı açıkça kapat: bus adları hemen bırakılır (görevler executor ile düşer).
        let _ = conn
            .close()
            .or(async {
                async_io::Timer::after(CLOSE_TIMEOUT).await;
                Ok(())
            })
            .await;
    }));
    // Kalan görevler (yönlendiriciler, bekleyen çağrılar) burada düşürülür.
    drop(ex);
}

async fn connect(options: &TrayOptions) -> zbus::Result<Connection> {
    let builder = match &options.bus_address {
        Some(address) => zbus::connection::Builder::address(address.as_str())?,
        None => zbus::connection::Builder::session()?,
    };
    builder.internal_executor(false).method_timeout(options.call_timeout).build().await
}

fn signal_rule(interface: &'static str) -> zbus::Result<MatchRule<'static>> {
    Ok(MatchRule::builder().msg_type(MessageType::Signal).interface(interface)?.build())
}

// ---------------------------------------------------------------------------------------
// Durum makinesi

struct Runtime {
    ex: Rc<LocalExecutor<'static>>,
    conn: Connection,
    options: TrayOptions,
    host_name: String,
    sink: Sink,
    input: Sender<Input>,
    registrations: Sender<ItemRegistration>,
    registered: RegisteredItems,
    notices: Option<Sender<Notice>>,
    role: TrayRole,
    /// Host rolünde izlenen watcher'ın benzersiz adı (sinyal göndereni doğrulamak için).
    watcher_owner: Option<String>,
    /// Kayıt sırasıyla.
    items: Vec<ItemState>,
    next_seq: u64,
    pending_resolves: usize,
    icons: IconWorker,
    /// Sınır yüzünden reddedilen kayıtlar (log seyreltmek için).
    dropped_registrations: u64,
    /// Tema ikonlarının çözüldüğü fiziksel boyut.
    icon_px: u32,
}

enum Wake {
    Stop,
    Input(Input),
    Registration(ItemRegistration),
    Command(Command),
    Timer,
}

impl Runtime {
    fn spawn(&self, task: impl Future<Output = ()> + 'static) {
        self.ex.spawn(task).detach();
    }

    /// Sonucu ana döngüye ileten görev.
    fn spawn_input(&self, task: impl Future<Output = Input> + 'static) {
        let input = self.input.clone();
        self.spawn(async move {
            let _ = input.send(task.await).await;
        });
    }

    fn emit(&self, event: TrayEvent) {
        (self.sink)(event);
    }

    async fn setup(&mut self) -> zbus::Result<()> {
        let dbus = DBusProxy::builder(&self.conn).cache_properties(CacheProperties::No).build().await?;

        // Akışlar rol belirlenmeden ve liste okunmadan önce kurulur: aradaki sinyal kaçmaz.
        let mut owners = dbus.receive_name_owner_changed().await?;
        let input = self.input.clone();
        self.spawn(async move {
            while let Some(signal) = owners.next().await {
                let Ok(args) = signal.args() else { continue };
                let name = args.name().to_string();
                let new_owner = args.new_owner().as_ref().map(|n| n.to_string());
                if input.send(Input::NameOwnerChanged { name, new_owner }).await.is_err() {
                    break;
                }
            }
        });

        let mut item_signals = MessageStream::for_match_rule(signal_rule(ITEM_INTERFACE)?, &self.conn, Some(SIGNAL_QUEUE)).await?;
        let input = self.input.clone();
        self.spawn(async move {
            while let Some(Ok(msg)) = item_signals.next().await {
                let header = msg.header();
                let (Some(sender), Some(path)) = (header.sender(), header.path()) else { continue };
                let signal = Input::ItemSignal { sender: sender.to_string(), path: path.to_string() };
                if input.send(signal).await.is_err() {
                    break;
                }
            }
        });

        let mut menu_signals = MessageStream::for_match_rule(signal_rule(MENU_INTERFACE)?, &self.conn, Some(SIGNAL_QUEUE)).await?;
        let input = self.input.clone();
        self.spawn(async move {
            while let Some(Ok(message)) = menu_signals.next().await {
                let header = message.header();
                let (Some(sender), Some(path), Some(member)) = (header.sender(), header.path(), header.member()) else {
                    continue;
                };
                let signal = Input::MenuSignal {
                    sender: sender.to_string(),
                    path: path.to_string(),
                    member: member.to_string(),
                    message: message.clone(),
                };
                if input.send(signal).await.is_err() {
                    break;
                }
            }
        });

        let mut watcher_signals =
            MessageStream::for_match_rule(signal_rule(WATCHER_INTERFACE)?, &self.conn, Some(SIGNAL_QUEUE)).await?;
        let input = self.input.clone();
        self.spawn(async move {
            while let Some(Ok(message)) = watcher_signals.next().await {
                let header = message.header();
                let (Some(sender), Some(member)) = (header.sender(), header.member()) else { continue };
                let member = member.to_string();
                if !member.starts_with("StatusNotifierItem") {
                    continue;
                }
                let Ok(service) = message.body().deserialize::<String>() else { continue };
                let signal = Input::WatcherSignal { sender: sender.to_string(), member, service };
                if input.send(signal).await.is_err() {
                    break;
                }
            }
        });

        if try_claim_watcher(&self.conn, self.registrations.clone(), self.registered.clone()).await? {
            self.become_watcher();
        } else {
            self.role = TrayRole::Host;
            tracing::info!("tepsi: mevcut StatusNotifierWatcher'a host olarak bağlanılıyor");
            if let Err(e) = self.conn.request_name(self.host_name.as_str()).await {
                tracing::debug!("tepsi: host adı alınamadı: {e}");
            }
            self.emit(TrayEvent::Role(TrayRole::Host));
            self.connect_external();
        }
        Ok(())
    }

    async fn run(
        &mut self,
        stop: Receiver<()>,
        commands: Receiver<Command>,
        registrations: Receiver<ItemRegistration>,
        inputs: Receiver<Input>,
    ) {
        loop {
            let deadline = self.deadline();
            let wake = async {
                let _ = stop.recv().await;
                Wake::Stop
            }
            .or(async {
                match inputs.recv().await {
                    Ok(input) => Wake::Input(input),
                    Err(_) => Wake::Stop,
                }
            })
            .or(async {
                match registrations.recv().await {
                    Ok(r) => Wake::Registration(r),
                    Err(_) => future::pending().await,
                }
            })
            .or(async {
                match commands.recv().await {
                    Ok(c) => Wake::Command(c),
                    Err(_) => future::pending().await,
                }
            })
            .or(async {
                match deadline {
                    Some(at) => {
                        async_io::Timer::at(at).await;
                        Wake::Timer
                    }
                    None => future::pending().await,
                }
            })
            .await;
            match wake {
                Wake::Stop => return,
                Wake::Input(input) => self.on_input(input),
                Wake::Registration(r) => self.register(&r.service, r.sender.as_deref()),
                Wake::Command(command) => self.on_command(command),
                Wake::Timer => {}
            }
            self.process_due(Instant::now());
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.items
            .iter()
            .flat_map(|s| [s.read.deadline(), s.menu.fetch.deadline(), s.menu.emit_due])
            .flatten()
            .min()
    }

    fn index(&self, id: &ItemId) -> Option<usize> {
        self.items.iter().position(|s| &s.id == id)
    }

    // -- roller ---------------------------------------------------------------------------

    fn become_watcher(&mut self) {
        self.role = TrayRole::Watcher;
        self.watcher_owner = None;
        tracing::info!("tepsi: {WATCHER_NAME} sahiplenildi");
        self.registered.set(self.items.iter().map(|s| s.registered_as.clone()).collect());
        let (tx, rx) = async_channel::bounded(NOTICE_CAPACITY);
        self.notices = Some(tx);
        let conn = self.conn.clone();
        self.spawn(notice_loop(conn, rx));
        self.notice(Notice::HostRegistered);
        self.emit(TrayEvent::Role(TrayRole::Watcher));
        // Watcher yokken açılmış öğeler: iyi bilinen adlarıyla keşfedilir (uygulamaların çoğu
        // watcher görününce kendiliğinden de yeniden kaydolur).
        let conn = self.conn.clone();
        self.spawn_input(async move {
            let names = call(&conn, DBUS_NAME, DBUS_PATH, DBUS_NAME, "ListNames", &())
                .await
                .and_then(|m| m.body().deserialize::<Vec<String>>())
                .unwrap_or_default();
            Input::Discovered(names)
        });
    }

    fn notice(&self, notice: Notice) {
        if let Some(tx) = &self.notices
            && tx.try_send(notice).is_err()
        {
            tracing::warn!("tepsi: watcher sinyal kuyruğu dolu, sinyal atıldı");
        }
    }

    fn sync_registered(&self) {
        if self.role == TrayRole::Watcher {
            self.registered.set(self.items.iter().map(|s| s.registered_as.clone()).collect());
        }
    }

    /// Host rolü: watcher'a kaydol ve mevcut listeyi oku.
    fn connect_external(&self) {
        let conn = self.conn.clone();
        let host_name = self.host_name.clone();
        self.spawn_input(async move {
            if let Err(e) =
                call(&conn, WATCHER_NAME, WATCHER_PATH, WATCHER_INTERFACE, "RegisterStatusNotifierHost", &(host_name.as_str(),))
                    .await
            {
                tracing::warn!("tepsi: RegisterStatusNotifierHost başarısız: {e}");
            }
            let owner = call(&conn, DBUS_NAME, DBUS_PATH, DBUS_NAME, "GetNameOwner", &(WATCHER_NAME,))
                .await
                .and_then(|m| m.body().deserialize::<String>())
                .ok();
            let items = call(
                &conn,
                WATCHER_NAME,
                WATCHER_PATH,
                PROPERTIES_INTERFACE,
                "Get",
                &(WATCHER_INTERFACE, "RegisteredStatusNotifierItems"),
            )
            .await
            .and_then(|m| m.body().deserialize::<OwnedValue>())
            .ok()
            .and_then(|v| Vec::<String>::try_from(v).ok())
            .unwrap_or_default();
            Input::External { owner, items }
        });
    }

    fn try_takeover(&self) {
        let conn = self.conn.clone();
        let registrations = self.registrations.clone();
        let registered = self.registered.clone();
        self.spawn_input(async move {
            match try_claim_watcher(&conn, registrations, registered).await {
                Ok(claimed) => Input::Takeover(claimed),
                Err(e) => {
                    tracing::warn!("tepsi: watcher devralınamadı: {e}");
                    Input::Takeover(false)
                }
            }
        });
    }

    // -- girdiler -------------------------------------------------------------------------

    fn on_input(&mut self, input: Input) {
        let now = Instant::now();
        match input {
            Input::NameOwnerChanged { name, new_owner } => self.on_name_owner_changed(&name, new_owner),
            Input::ItemSignal { sender, path } => {
                let (quiet, max) = (self.options.coalesce_quiet, self.options.coalesce_max_delay);
                if let Some(s) = self.items.iter_mut().find(|s| s.owner == sender && s.path == path) {
                    s.read.mark(now, quiet, max);
                    // Fırtına günlüğü: sinyaller birleşir (en fazla `min_refresh_interval`'da bir
                    // okuma); saniyede çok sinyal gönderen öğe bir kez loglanır.
                    let (start, count) = &mut s.signals;
                    *count += 1;
                    if now.saturating_duration_since(*start) >= Duration::from_secs(1) {
                        if *count > SIGNAL_STORM_PER_SEC {
                            tracing::warn!(id = %s.id, signals = *count, "tepsi: sinyal fırtınası, birleştirildi");
                        }
                        (*start, *count) = (now, 0);
                    }
                }
            }
            Input::MenuSignal { sender, path, member, message } => self.on_menu_signal(&sender, &path, &member, &message),
            Input::WatcherSignal { sender, member, service } => {
                if self.role != TrayRole::Host || self.watcher_owner.as_deref() != Some(sender.as_str()) {
                    return;
                }
                match member.as_str() {
                    "StatusNotifierItemRegistered" => self.register(&service, None),
                    "StatusNotifierItemUnregistered" => {
                        if let Some(r) = parse_registration(&service, None) {
                            let ids: Vec<ItemId> = self
                                .items
                                .iter()
                                .filter(|s| s.path == r.path && (s.owner == r.bus || s.registered_bus == r.bus))
                                .map(|s| s.id.clone())
                                .collect();
                            ids.iter().for_each(|id| self.remove(id));
                        }
                    }
                    _ => {}
                }
            }
            Input::Resolved { registration, owner } => {
                self.pending_resolves = self.pending_resolves.saturating_sub(1);
                match owner {
                    Some(owner) if is_unique_name(&owner) => self.add(&registration, &owner),
                    _ => tracing::debug!(bus = registration.bus, "tepsi: kayıtlı adın sahibi yok, atlandı"),
                }
            }
            Input::Discovered(names) => {
                for name in names.iter().filter(|n| ITEM_NAME_PREFIXES.iter().any(|p| n.starts_with(p))).take(MAX_ITEMS) {
                    self.register(name, None);
                }
            }
            Input::External { owner, items } => {
                if self.role != TrayRole::Host {
                    return;
                }
                self.watcher_owner = owner;
                tracing::info!(items = items.len(), "tepsi: harici watcher listesi okundu");
                for service in items.iter().take(MAX_ITEMS * 2) {
                    self.register(service, None);
                }
            }
            Input::Takeover(claimed) => {
                if claimed && self.role == TrayRole::Host {
                    self.become_watcher();
                }
            }
            Input::ItemRead { id, result } => self.on_item_read(&id, result, now),
            Input::MenuFetched { id, session, parent, result } => self.on_menu_fetched(&id, session, parent, result),
            Input::MenuUpdatesNeeded { id, session, parents } => {
                let (quiet, max) = (self.options.coalesce_quiet, self.options.coalesce_max_delay);
                if let Some(i) = self.index(&id) {
                    let menu = &mut self.items[i].menu;
                    if menu.is_current(session) {
                        for parent in parents {
                            menu.request(parent);
                        }
                        menu.fetch.mark(now, quiet, max);
                    }
                }
            }
            Input::ActionFailed { id } => {
                if self.items.iter().any(|s| s.id == id && s.item.as_ref().is_some_and(|i| i.menu.is_some())) {
                    // Menü hemen okunmaya başlar: UI isteği görünce ağaç yolda olur.
                    let request = next_menu_request();
                    self.on_command(Command::OpenMenu { item: id.clone(), request });
                    self.emit(TrayEvent::MenuRequested { item: id, request });
                }
            }
            Input::IconLoaded { id, key, image } => self.on_icon_loaded(&id, &key, image),
        }
    }

    fn note_dropped_registration(&mut self, reason: &str) {
        self.dropped_registrations += 1;
        let n = self.dropped_registrations;
        if n == 1 || n.is_multiple_of(100) {
            tracing::warn!(limit = MAX_ITEMS, dropped = n, "tepsi: kayıt reddedildi ({reason})");
        }
    }

    fn on_icon_loaded(&mut self, id: &ItemId, key: &IconKey, image: Option<Pixmap>) {
        let Some(i) = self.index(id) else { return };
        let px = self.icon_px;
        let s = &mut self.items[i];
        if s.icon_job.as_ref().is_some_and(|(k, _)| k == key) {
            s.icon_job = None;
        }
        if image.is_none() {
            tracing::debug!(id = %s.id, "tepsi: ikon çözülemedi");
        }
        let wanted = s.icon_key.as_ref() == Some(key) || s.menu_icons_wanted.contains(key);
        if wanted {
            cache_icon(s, key.clone(), image);
        }
        s.menu_icons_wanted.retain(|k| k != key);

        let mut events = Vec::new();
        if let Some(old) = &s.item {
            let mut item = old.clone();
            item.image = item_image(s, &item, px);
            if item != *old {
                s.item = Some(item.clone());
                events.push(TrayEvent::ItemUpdated(item));
            }
        }
        if s.menu.open && attach_menu_icons(s, px) {
            // Birden çok ikon art arda gelir: yayın birleştirilir.
            s.menu.emit_due.get_or_insert(Instant::now() + self.options.coalesce_quiet);
        }
        // Sıradaki iş (güncel öğe ikonu ya da bekleyen menü ikonu).
        pump_icons(s, &self.icons, &self.input);
        for event in events {
            self.emit(event);
        }
    }

    fn set_icon_px(&mut self, px: u32) {
        let px = clamp_px(px);
        if px == self.icon_px {
            return;
        }
        self.icon_px = px;
        for i in 0..self.items.len() {
            let s = &mut self.items[i];
            let Some(mut item) = s.item.clone() else { continue };
            prepare_icon(s, &mut item, px, &self.icons, &self.input);
            if s.item.as_ref() != Some(&item) {
                s.item = Some(item.clone());
                self.emit(TrayEvent::ItemUpdated(item));
            }
        }
    }

    fn on_name_owner_changed(&mut self, name: &str, new_owner: Option<String>) {
        if name == WATCHER_NAME {
            self.watcher_owner.clone_from(&new_owner);
            if self.role == TrayRole::Host {
                match new_owner {
                    None => {
                        tracing::info!("tepsi: harici watcher kayboldu, devralınmaya çalışılıyor");
                        self.try_takeover();
                    }
                    Some(owner) if Some(owner.as_str()) != self.conn.unique_name().map(|n| n.as_str()) => {
                        self.connect_external();
                    }
                    Some(_) => {}
                }
            }
            return;
        }
        if new_owner.is_none() {
            let gone: Vec<ItemId> = self.items.iter().filter(|s| s.owner == name).map(|s| s.id.clone()).collect();
            for id in gone {
                tracing::debug!(%id, "tepsi: öğe sahibi bus'tan ayrıldı");
                self.remove(&id);
            }
        }
    }

    /// Kayıt dizgisini işler (watcher çağrısı, harici watcher sinyali/listesi veya keşif).
    fn register(&mut self, service: &str, sender: Option<&str>) {
        let Some(registration) = parse_registration(service, sender) else {
            tracing::debug!(service, "tepsi: geçersiz öğe kaydı yok sayıldı");
            return;
        };
        if is_unique_name(&registration.bus) {
            let owner = registration.bus.clone();
            self.add(&registration, &owner);
            return;
        }
        if self.pending_resolves >= MAX_PENDING_RESOLVES {
            self.note_dropped_registration("çok fazla bekleyen kayıt");
            return;
        }
        self.pending_resolves += 1;
        let conn = self.conn.clone();
        self.spawn_input(async move {
            let owner = call(&conn, DBUS_NAME, DBUS_PATH, DBUS_NAME, "GetNameOwner", &(registration.bus.as_str(),))
                .await
                .and_then(|m| m.body().deserialize::<String>())
                .ok();
            Input::Resolved { registration, owner }
        });
    }

    fn add(&mut self, registration: &Registration, owner: &str) {
        let id = ItemId::new(owner, &registration.path);
        let now = Instant::now();
        if let Some(i) = self.index(&id) {
            // Yeniden kayıt (ör. watcher değişti): yalnızca tazele.
            self.items[i].read.now(now);
            return;
        }
        if self.items.len() >= MAX_ITEMS {
            self.note_dropped_registration("öğe sınırı aşıldı");
            return;
        }
        let registered_as = format!("{}{}", registration.bus, registration.path);
        let mut read = Refresh::with_min_interval(self.options.min_refresh_interval);
        read.now(now);
        self.items.push(ItemState {
            id,
            owner: owner.to_owned(),
            path: registration.path.clone(),
            registered_bus: registration.bus.clone(),
            registered_as: registered_as.clone(),
            seq: self.next_seq,
            item: None,
            read,
            failures: 0,
            menu: MenuState { fetch: Refresh::with_min_interval(self.options.min_refresh_interval), ..MenuState::default() },
            icon_key: None,
            icon_cache: HashMap::new(),
            menu_icons_wanted: Vec::new(),
            icon_job: None,
            signals: (now, 0),
        });
        self.next_seq += 1;
        self.sync_registered();
        self.notice(Notice::Registered(registered_as));
    }

    fn remove(&mut self, id: &ItemId) {
        let Some(i) = self.index(id) else { return };
        let state = self.items.remove(i);
        if let Some((_, job)) = &state.icon_job {
            job.cancel();
        }
        self.sync_registered();
        self.notice(Notice::Unregistered(state.registered_as));
        if state.item.is_some() {
            self.emit(TrayEvent::ItemRemoved(state.id));
        }
    }

    fn on_item_read(&mut self, id: &ItemId, result: zbus::Result<Props>, now: Instant) {
        let Some(i) = self.index(id) else { return };
        let attempts = self.options.initial_read_attempts.max(1);
        let retry = self.options.retry_delay;
        let px = self.icon_px;
        let s = &mut self.items[i];
        s.read.inflight = false;
        match result {
            Ok(props) => {
                s.failures = 0;
                let mut item = TrayItem::from_properties(s.id.clone(), s.seq, &s.owner, &s.path, &props);
                prepare_icon(s, &mut item, px, &self.icons, &self.input);
                let event = match &s.item {
                    None => Some(TrayEvent::ItemAdded(item.clone())),
                    Some(old) if *old != item => {
                        if old.menu != item.menu && s.menu.open {
                            s.menu.tree = None;
                            s.menu.request(0);
                            s.menu.fetch.now(now);
                        }
                        Some(TrayEvent::ItemUpdated(item.clone()))
                    }
                    Some(_) => None,
                };
                s.item = Some(item);
                if let Some(event) = event {
                    self.emit(event);
                }
            }
            Err(e) if s.item.is_none() => {
                s.failures += 1;
                if s.failures >= attempts {
                    tracing::info!(id = %s.id, "tepsi: öğe okunamadı, kayıt düşürüldü: {e}");
                    let id = s.id.clone();
                    self.remove(&id);
                } else {
                    tracing::debug!(id = %s.id, attempt = s.failures, "tepsi: öğe okunamadı, tekrar denenecek: {e}");
                    s.read.at(now + retry * 2u32.saturating_pow(s.failures - 1));
                }
            }
            Err(e) if is_gone(&e) => {
                tracing::debug!(id = %s.id, "tepsi: öğe nesnesi kayboldu: {e}");
                let id = s.id.clone();
                self.remove(&id);
            }
            Err(e) => tracing::debug!(id = %s.id, "tepsi: öğe güncellenemedi: {e}"),
        }
    }

    fn on_menu_signal(&mut self, sender: &str, path: &str, member: &str, message: &Message) {
        let now = Instant::now();
        let (quiet, max) = (self.options.coalesce_quiet, self.options.coalesce_max_delay);
        let Some(s) = self
            .items
            .iter_mut()
            .find(|s| s.owner == sender && s.item.as_ref().and_then(|i| i.menu.as_deref()) == Some(path))
        else {
            return;
        };
        let menu = &mut s.menu;
        if !menu.open {
            return;
        }
        match member {
            "LayoutUpdated" => {
                let parent = message.body().deserialize::<(u32, i32)>().map_or(0, |(_, parent)| parent);
                menu.request(parent);
                menu.fetch.mark(now, quiet, max);
            }
            "ItemsPropertiesUpdated" => {
                let Some(tree) = &mut menu.tree else { return };
                match message.body().deserialize::<PropertiesUpdate>() {
                    Ok(update) => {
                        if tree.patch(&update) {
                            // Değişen icon-name/icon-data için ikonlar yeniden eşlenir.
                            attach_menu_icons(s, self.icon_px);
                            pump_icons(s, &self.icons, &self.input);
                            s.menu.emit_due.get_or_insert(now + quiet);
                        }
                    }
                    Err(e) => tracing::debug!(id = %s.id, "tepsi: ItemsPropertiesUpdated çözülemedi: {e}"),
                }
            }
            _ => {}
        }
    }

    fn on_menu_fetched(&mut self, id: &ItemId, session: MenuRequest, parent: i32, result: zbus::Result<LayoutReply>) {
        let Some(i) = self.index(id) else { return };
        let attempts = self.options.initial_read_attempts.max(1);
        let retry = self.options.retry_delay;
        let s = &mut self.items[i];
        let menu = &mut s.menu;
        if menu.session != session {
            // Önceki açılışın geç gelen yanıtı: geçerli açılışın okumasına dokunmaz.
            tracing::debug!(id = %s.id, session, current = menu.session, "tepsi: eski menü yanıtı atıldı");
            return;
        }
        menu.fetch.inflight = false;
        if !menu.open {
            return;
        }
        let layout = match result {
            Ok(layout) => MenuTree::from_layout(&layout),
            Err(e) => {
                tracing::debug!(id = %s.id, parent, "tepsi: menü okunamadı: {e}");
                // Açılışın ilk yanıtı alınamadı (zaman aşımı/hata): önceki açılıştan kalan ağaç
                // varsa o gösterilsin; UI bu açılışı sonsuza dek beklemesin.
                if menu.answered {
                    return;
                }
                if let Some(tree) = menu.tree.clone() {
                    menu.answered = true;
                    let event = TrayEvent::MenuLayout { item: s.id.clone(), request: session, menu: tree };
                    self.emit(event);
                    return;
                }
                // Gösterilecek ağaç hiç yok: öğe okumasıyla aynı bütçe ve geri çekilmeyle yeniden
                // denenir (`AboutToShow` tekrarlanmaz). Eskiden tek zaman aşımı (yük altında
                // donma, meşgul uygulama) açılışı sonsuza dek cevapsız bırakıyordu: tıklanan menü
                // hiç açılmıyordu (2026-10-17).
                menu.failures += 1;
                if menu.failures < attempts && !is_gone(&e) {
                    menu.request(parent);
                    menu.fetch.at(Instant::now() + retry * 2u32.saturating_pow(menu.failures - 1));
                } else {
                    tracing::info!(id = %s.id, attempts = menu.failures, "tepsi: menü okunamadı, açılış bırakıldı: {e}");
                }
                return;
            }
        };
        match &mut menu.tree {
            Some(tree) if parent != 0 => {
                if !tree.splice(layout) {
                    menu.request(0);
                    menu.fetch.now(Instant::now());
                    return;
                }
            }
            _ => menu.tree = Some(layout),
        }
        menu.emit_due = None;
        menu.answered = true;
        attach_menu_icons(s, self.icon_px);
        pump_icons(s, &self.icons, &self.input);
        if let Some(tree) = &s.menu.tree {
            let event = TrayEvent::MenuLayout { item: s.id.clone(), request: session, menu: tree.clone() };
            self.emit(event);
        }
    }

    // -- komutlar -------------------------------------------------------------------------

    fn on_command(&mut self, command: Command) {
        match command {
            Command::Activate { item, x, y } => self.item_call(&item, "Activate", (x, y), true),
            Command::SecondaryActivate { item, x, y } => self.item_call(&item, "SecondaryActivate", (x, y), false),
            Command::ContextMenu { item, x, y } => self.item_call(&item, "ContextMenu", (x, y), true),
            Command::Scroll { item, delta, orientation } => {
                self.item_call(&item, "Scroll", (delta, orientation.as_str()), false)
            }
            Command::OpenMenu { item, request } => {
                let Some(i) = self.index(&item) else { return };
                if self.items[i].item.as_ref().and_then(|i| i.menu.as_ref()).is_none() {
                    tracing::debug!(id = %item, "tepsi: öğenin menüsü yok");
                    return;
                }
                // UI aynı anda tek menü gösterir: başka öğenin açık kalmış izlemesi bırakılır
                // (UI bekleyen bir açılışı kapatma komutu göndermeden değiştirmiş olabilir).
                for (j, s) in self.items.iter_mut().enumerate() {
                    if j != i && s.menu.open {
                        s.menu.close();
                        s.menu_icons_wanted.clear();
                    }
                }
                self.items[i].menu.open(request, Instant::now());
            }
            Command::CloseMenu(item) => {
                if let Some(i) = self.index(&item) {
                    self.items[i].menu.close();
                    self.items[i].menu_icons_wanted.clear();
                }
            }
            Command::MenuAboutToShow { item, ids } => {
                let Some((owner, path)) = self.menu_target(&item) else { return };
                let Some(i) = self.index(&item) else { return };
                let session = self.items[i].menu.session;
                let conn = self.conn.clone();
                self.spawn_input(async move {
                    let parents = about_to_show(&conn, &owner, &path, &ids).await;
                    Input::MenuUpdatesNeeded { id: item, session, parents }
                });
            }
            Command::SetIconSize(px) => self.set_icon_px(px),
            Command::MenuEvent { item, id, kind } => {
                let Some((owner, path)) = self.menu_target(&item) else { return };
                // Tıklama ya da kapanış menüyü kapatır: izlemeyi bırak (UI ayrıca CloseMenu
                // göndermek zorunda değil).
                if matches!(kind, MenuEventKind::Clicked | MenuEventKind::Closed)
                    && let Some(i) = self.index(&item)
                {
                    self.items[i].menu.close();
                    self.items[i].menu_icons_wanted.clear();
                }
                let conn = self.conn.clone();
                let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as u32);
                self.spawn(async move {
                    let body = (id, kind.as_str(), Value::from(0i32), timestamp);
                    if let Err(e) = call(&conn, &owner, &path, MENU_INTERFACE, "Event", &body).await {
                        tracing::debug!(%item, id, "tepsi: menü olayı gönderilemedi: {e}");
                    }
                });
            }
        }
    }

    fn menu_target(&self, id: &ItemId) -> Option<(String, String)> {
        let s = self.items.iter().find(|s| &s.id == id)?;
        let path = s.item.as_ref()?.menu.clone()?;
        Some((s.owner.clone(), path))
    }

    /// Öğe metodunu çağırır. `menu_fallback`: hata olursa UI'ya menüyü açmasını söyle.
    fn item_call<B>(&self, id: &ItemId, method: &'static str, body: B, menu_fallback: bool)
    where
        B: serde::Serialize + DynamicType + 'static,
    {
        let Some(s) = self.items.iter().find(|s| &s.id == id) else { return };
        let (conn, owner, path, id) = (self.conn.clone(), s.owner.clone(), s.path.clone(), id.clone());
        let input = self.input.clone();
        self.spawn(async move {
            if let Err(e) = call(&conn, &owner, &path, ITEM_INTERFACE, method, &body).await {
                tracing::debug!(%id, method, "tepsi: öğe çağrısı başarısız: {e}");
                if menu_fallback {
                    let _ = input.send(Input::ActionFailed { id }).await;
                }
            }
        });
    }

    // -- zamanlanmış işler ----------------------------------------------------------------

    fn process_due(&mut self, now: Instant) {
        for i in 0..self.items.len() {
            if self.items[i].read.take(now) {
                let s = &self.items[i];
                let (conn, id, owner, path) = (self.conn.clone(), s.id.clone(), s.owner.clone(), s.path.clone());
                self.spawn_input(async move {
                    let result = read_properties(&conn, &owner, &path).await;
                    Input::ItemRead { id, result }
                });
            }
            let s = &mut self.items[i];
            if s.menu.emit_due.is_some_and(|d| d <= now) {
                s.menu.emit_due = None;
                if let Some(tree) = &s.menu.tree {
                    let event = TrayEvent::MenuLayout { item: s.id.clone(), request: s.menu.session, menu: tree.clone() };
                    self.emit(event);
                }
            }
            let s = &mut self.items[i];
            let menu_path = s.item.as_ref().and_then(|i| i.menu.clone());
            if s.menu.open && s.menu.fetch.take(now) {
                let Some(path) = menu_path else {
                    s.menu.fetch.inflight = false;
                    continue;
                };
                let parent = s.menu.pending.take().unwrap_or(0);
                let about = std::mem::take(&mut s.menu.about_to_show);
                let (conn, id, owner, session) = (self.conn.clone(), s.id.clone(), s.owner.clone(), s.menu.session);
                self.spawn_input(async move {
                    if about {
                        // Sonuç önemsiz: amaç uygulamanın menüyü hazırlaması.
                        let _ = call(&conn, &owner, &path, MENU_INTERFACE, "AboutToShow", &(0i32,)).await;
                    }
                    let result = call(&conn, &owner, &path, MENU_INTERFACE, "GetLayout", &(parent, -1i32, Vec::<&str>::new()))
                        .await
                        .and_then(|m| m.body().deserialize::<LayoutReply>());
                    Input::MenuFetched { id, session, parent, result }
                });
            }
        }
    }
}

/// Öğe başına önbellekteki en fazla çözülmüş ikon (öğe + menü).
const MAX_ICON_CACHE: usize = 96;
/// Açık menü için çözülmeyi bekleyen en fazla ikon.
const MAX_MENU_ICONS: usize = 64;

fn cache_icon(s: &mut ItemState, key: IconKey, image: Option<Pixmap>) {
    if s.icon_cache.len() >= MAX_ICON_CACHE && !s.icon_cache.contains_key(&key) {
        s.icon_cache.clear();
    }
    s.icon_cache.insert(key, image);
}

/// Öğenin çizilecek görüntüsü: çözülmüş tema ikonu; çözülemediyse ya da ad yoksa pixmap;
/// çözüm sürüyorsa önceki görüntü (titreme yok). `s.item` eski hâlidir.
fn item_image(s: &ItemState, item: &TrayItem, px: u32) -> Option<Pixmap> {
    match &s.icon_key {
        None => pixmap_fallback(item, px),
        Some(k) => match s.icon_cache.get(k) {
            Some(Some(p)) => Some(p.clone()),
            Some(None) => pixmap_fallback(item, px),
            None => s.item.as_ref().and_then(|o| o.image.clone()).or_else(|| pixmap_fallback(item, px)),
        },
    }
}

/// Öğenin çizime hazır ikonunu (`TrayItem::image`) hazırlar; ad değiştiyse çözüm arka planda
/// başlar. `s.item` henüz eski hâlidir.
fn prepare_icon(s: &mut ItemState, item: &mut TrayItem, px: u32, icons: &IconWorker, input: &Sender<Input>) {
    let key = IconKey::for_item(item, px);
    if key != s.icon_key {
        // Süren iş artık kimsenin değilse iptal edilir; bitince (iptal dahil) sıradaki başlar.
        if let Some((k, job)) = &s.icon_job
            && Some(k) != key.as_ref()
            && !s.menu_icons_wanted.contains(k)
        {
            job.cancel();
        }
        s.icon_key = key;
    }
    pump_icons(s, icons, input);
    item.image = item_image(s, item, px);
}

/// Sıradaki ikon işini başlatır: öğe başına aynı anda en fazla bir iş; önce öğe ikonu, sonra
/// açık menünün ikonları. Kuyruk doluysa o anahtar "çözülemedi" sayılır (yedek kullanılır).
fn pump_icons(s: &mut ItemState, icons: &IconWorker, input: &Sender<Input>) {
    if s.icon_job.is_some() {
        return;
    }
    let next = s.icon_key.iter().chain(&s.menu_icons_wanted).find(|k| !s.icon_cache.contains_key(*k)).cloned();
    let Some(key) = next else { return };
    let (input, id, k) = (input.clone(), s.id.clone(), key.clone());
    // Yalıtılmış çözümün kaynak anahtarı öğe kimliğidir (menü ikonları dahil): öğe başına aynı
    // anda tek iş olduğundan birleştirme yalnızca o öğenin eski işini düşürür.
    let source = source_key(KeyDomain::Tray, s.id.as_str());
    match icons.submit(source, key.clone(), move |image| {
        let _ = input.send_blocking(Input::IconLoaded { id, key: k, image });
    }) {
        Some(job) => s.icon_job = Some((key, job)),
        None => {
            s.menu_icons_wanted.retain(|k| *k != key);
            cache_icon(s, key, None);
        }
    }
}

/// Açık menü ağacına çözülmüş ikonları yerleştirir; eksikleri (sınırlı) bekleme listesine
/// ekler. Ağaç değiştiyse `true`.
fn attach_menu_icons(s: &mut ItemState, px: u32) -> bool {
    let theme_path = s.item.as_ref().map(|i| i.icon_theme_path.clone()).unwrap_or_default();
    let ItemState { menu, icon_cache, menu_icons_wanted, .. } = s;
    let Some(tree) = &mut menu.tree else { return false };
    fn walk(
        node: &mut MenuNode,
        theme_path: &str,
        px: u32,
        cache: &HashMap<IconKey, Option<Pixmap>>,
        wanted: &mut Vec<IconKey>,
    ) -> bool {
        let key = IconSource::for_menu_node(node, theme_path).map(|source| IconKey { source, px });
        let image = key.and_then(|k| match cache.get(&k) {
            Some(v) => v.clone(),
            None => {
                if wanted.len() < MAX_MENU_ICONS && !wanted.contains(&k) {
                    wanted.push(k);
                }
                None
            }
        });
        let mut changed = node.image != image;
        node.image = image;
        for child in &mut node.children {
            changed |= walk(child, theme_path, px, cache, wanted);
        }
        changed
    }
    walk(&mut tree.root, &theme_path, px, icon_cache, menu_icons_wanted)
}


// ---------------------------------------------------------------------------------------
// D-Bus yardımcıları (görevlerde çalışır)

async fn call<B>(conn: &Connection, dest: &str, path: &str, interface: &str, method: &str, body: &B) -> zbus::Result<Message>
where
    B: serde::Serialize + DynamicType,
{
    conn.call_method(Some(dest), path, Some(interface), method, body).await
}

/// `org.kde.StatusNotifierWatcher` adını almayı dener. Başarılıysa arayüz sunulur.
async fn try_claim_watcher(
    conn: &Connection,
    registrations: Sender<ItemRegistration>,
    items: RegisteredItems,
) -> zbus::Result<bool> {
    let server = conn.object_server();
    server.at(WATCHER_PATH, Watcher { registrations, items }).await?;
    let claimed = match conn.request_name_with_flags(WATCHER_NAME, RequestNameFlags::DoNotQueue.into()).await {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => true,
        Ok(_) | Err(zbus::Error::NameTaken) => false,
        Err(e) => {
            let _ = server.remove::<Watcher, _>(WATCHER_PATH).await;
            return Err(e);
        }
    };
    if !claimed {
        let _ = server.remove::<Watcher, _>(WATCHER_PATH).await;
    }
    Ok(claimed)
}

async fn notice_loop(conn: Connection, notices: Receiver<Notice>) {
    let Ok(emitter) = SignalEmitter::new(&conn, WATCHER_PATH) else { return };
    while let Ok(notice) = notices.recv().await {
        let result = match &notice {
            Notice::Registered(s) => Watcher::status_notifier_item_registered(&emitter, s).await,
            Notice::Unregistered(s) => Watcher::status_notifier_item_unregistered(&emitter, s).await,
            Notice::HostRegistered => Watcher::status_notifier_host_registered(&emitter).await,
        };
        if let Err(e) = result {
            tracing::debug!("tepsi: watcher sinyali gönderilemedi: {e}");
        }
        if matches!(notice, Notice::Registered(_) | Notice::Unregistered(_))
            && let Ok(iface) = conn.object_server().interface::<_, Watcher>(WATCHER_PATH).await
        {
            let _ = iface.get().await.registered_status_notifier_items_changed(&emitter).await;
        }
    }
}

/// `GetAll`; desteklenmezse (zaman aşımı veya kaybolma dışındaki hata) tek tek `Get`.
async fn read_properties(conn: &Connection, owner: &str, path: &str) -> zbus::Result<Props> {
    let error = match call(conn, owner, path, PROPERTIES_INTERFACE, "GetAll", &(ITEM_INTERFACE,)).await {
        Ok(reply) => return reply.body().deserialize::<Props>(),
        Err(e) if is_gone(&e) || is_timeout(&e) => return Err(e),
        Err(e) => e,
    };
    let mut props = Props::new();
    for name in ITEM_PROPERTIES {
        match call(conn, owner, path, PROPERTIES_INTERFACE, "Get", &(ITEM_INTERFACE, *name)).await {
            Ok(reply) => {
                if let Ok(value) = reply.body().deserialize::<OwnedValue>() {
                    props.insert((*name).to_owned(), value);
                }
            }
            Err(e) if is_gone(&e) || is_timeout(&e) => return Err(e),
            Err(_) => {}
        }
    }
    if props.is_empty() { Err(error) } else { Ok(props) }
}

/// Alt menüler açılıyor: güncellenmesi gereken alt ağaçların kimlikleri.
async fn about_to_show(conn: &Connection, owner: &str, path: &str, ids: &[i32]) -> Vec<i32> {
    if let [id] = ids {
        return match call(conn, owner, path, MENU_INTERFACE, "AboutToShow", &(*id,)).await {
            Ok(reply) if reply.body().deserialize::<bool>().unwrap_or(false) => vec![*id],
            _ => Vec::new(),
        };
    }
    match call(conn, owner, path, MENU_INTERFACE, "AboutToShowGroup", &(ids,)).await {
        Ok(reply) => reply.body().deserialize::<(Vec<i32>, Vec<i32>)>().map(|(updates, _)| updates).unwrap_or_default(),
        Err(_) => {
            // AboutToShowGroup'u desteklemeyen eski uygulamalar.
            let mut updates = Vec::new();
            for id in ids {
                if let Ok(reply) = call(conn, owner, path, MENU_INTERFACE, "AboutToShow", &(*id,)).await
                    && reply.body().deserialize::<bool>().unwrap_or(false)
                {
                    updates.push(*id);
                }
            }
            updates
        }
    }
}

fn error_name(e: &zbus::Error) -> Option<String> {
    match e {
        zbus::Error::MethodError(name, _, _) => Some(name.to_string()),
        zbus::Error::FDO(e) => Some(zbus::DBusError::name(&**e).to_string()),
        _ => None,
    }
}

/// Öğe sahibi veya nesnesi artık yok.
fn is_gone(e: &zbus::Error) -> bool {
    error_name(e).is_some_and(|n| {
        matches!(
            n.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.UnknownObject"
        )
    })
}

fn is_timeout(e: &zbus::Error) -> bool {
    matches!(e, zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::TimedOut)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn refresh_coalesces_and_bounds_latency() {
        let t0 = Instant::now();
        let mut r = Refresh::default();
        assert_eq!(r.deadline(), None);
        r.mark(t0, ms(30), ms(200));
        assert_eq!(r.deadline(), Some(t0), "boştayken önde kenar: hemen");
        assert!(r.take(t0));
        r.inflight = false;
        // Patlamanın kalanı sessizlik penceresinde birleşir, sürekli akışta en geç max_delay.
        r.mark(t0 + ms(10), ms(30), ms(200));
        assert_eq!(r.deadline(), Some(t0 + ms(40)));
        for i in 2..30 {
            r.mark(t0 + ms(i * 10), ms(30), ms(200));
        }
        assert_eq!(r.deadline(), Some(t0 + ms(210)));
        assert!(!r.take(t0 + ms(100)));
        assert!(r.take(t0 + ms(210)));
        assert_eq!(r.deadline(), None, "okuma sürerken uyanma yok");
        r.mark(t0 + ms(220), ms(30), ms(200));
        assert!(!r.take(t0 + ms(300)), "aynı anda iki okuma yok");
        r.inflight = false;
        assert!(r.take(t0 + ms(300)));
        r.inflight = false;
        r.mark(t0 + ms(1000), ms(30), ms(200));
        assert_eq!(r.deadline(), Some(t0 + ms(1000)), "sessizlikten sonra yine önde kenar");
    }

    #[test]
    fn menu_requests_merge() {
        let mut m = MenuState::default();
        m.request(5);
        assert_eq!(m.pending, Some(0), "ağaç yokken tamamı okunur");
        m.tree = Some(MenuTree { revision: 0, root: super::super::menu::MenuNode::new(0) });
        m.pending = None;
        m.request(5);
        m.request(5);
        assert_eq!(m.pending, Some(5));
        m.request(6);
        assert_eq!(m.pending, Some(0));
    }

    /// Hızlı kapat/aç: önceki açılışın süren okuması yeni açılışı bekletmez ve sonucu geçersizdir.
    #[test]
    fn menu_sessions_reject_stale_fetches() {
        let t0 = Instant::now();
        let mut m = MenuState { fetch: Refresh::with_min_interval(Duration::ZERO), ..MenuState::default() };
        m.open(1, t0);
        assert!(m.fetch.take(t0), "ilk okuma başlar");
        assert!(m.is_current(1));
        // Okuma sürerken kapanış ve yeni açılış.
        m.close();
        assert!(!m.is_current(1), "kapalı menünün yanıtı geçersiz");
        m.open(2, t0);
        assert!(!m.is_current(1), "eski açılışın yanıtı yeni açılışa ait değil");
        assert!(m.is_current(2));
        assert!(m.fetch.take(t0), "yeni açılış eski okumayı beklemeden okur");
        assert!(m.about_to_show && m.pending == Some(0), "AboutToShow(0) yeni açılışta da yapılır; tamamı okunur");
        assert!(!m.answered);
    }

    #[test]
    fn error_classification() {
        assert!(is_timeout(&zbus::Error::InputOutput(std::io::Error::from(std::io::ErrorKind::TimedOut).into())));
        assert!(!is_gone(&zbus::Error::InvalidReply));
        assert!(is_gone(&zbus::Error::FDO(Box::new(zbus::fdo::Error::ServiceUnknown("x".into())))));
    }
}
