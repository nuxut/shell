//! Sistem tepsisi: StatusNotifierWatcher/Host ve DBusMenu istemcisi.
//!
//! KDE, GNOME AppIndicator (libayatana), Electron ve Qt uygulamalarının tepsi öğeleri
//! `org.kde.StatusNotifierItem` nesneleridir; menüleri `com.canonical.dbusmenu` ile sunulur.
//!
//! **Watcher rolü.** Oturumda `org.kde.StatusNotifierWatcher` yoksa servis onu sahiplenir
//! ([`TrayRole::Watcher`]): öğe kayıtlarını alır, listeyi sunar, öğe sahibi bus'tan düşünce
//! (`NameOwnerChanged`) kaydı siler. Varsa ona `org.kde.StatusNotifierHost-<pid>-<n>` adıyla
//! host olarak kaydolur ([`TrayRole::Host`]) ve kayıt sinyallerini izler; o watcher
//! kaybolursa adı devralmayı dener.
//!
//! **Thread modeli.** Tek thread, üzerinde tek iş parçacıklı bir yürütücü (`LocalExecutor`);
//! zbus'un kendi yürütücüsü de bu thread'de döndürülür (`internal_executor(false)`). Ana
//! döngü yalnızca bellek içi durumu günceller ve **hiçbir D-Bus cevabını beklemez**: her
//! çağrı ayrı bir görevde, bağlantı düzeyinde zaman aşımıyla yapılır; sonuç ana döngüye
//! kanalla döner. Böylece cevap vermeyen bir uygulama yalnızca kendi öğesini geciktirir.
//! [`TrayService`] düşürülünce döngü durur, bağlantı kapanır (bus adları bırakılır), thread
//! join edilir.
//!
//! **Birleştirme.** `NewIcon`/`NewStatus`/`NewTitle`/`NewToolTip`... sinyalleri öğeyi
//! "kirli" işaretler. Boştayken ilk sinyal hemen `GetAll` yapar (önde kenar); patlamanın
//! kalanı [`TrayOptions::coalesce_quiet`] sessizlikten (sürekli akışta en geç
//! [`TrayOptions::coalesce_max_delay`]) sonra tek `GetAll`'a dönüşür. Öğe başına aynı anda en
//! fazla bir okuma. Menüler yalnızca UI açtığında ([`TrayHandle::open_menu`]) okunur ve
//! kapanana kadar `LayoutUpdated`/`ItemsPropertiesUpdated` ile güncel tutulur.
//!
//! **Sınırlar.** Öğe sayısı [`MAX_ITEMS`], metinler/pixmap'ler [`item`] sabitleri, menüler
//! [`menu`] sabitleri ile sınırlıdır; kötü veri varsayılan değere düşer, panik yoktur.

mod host;
mod icons;
pub mod item;
pub mod menu;
mod model;
pub mod ui_model;
mod value;
mod watcher;

use std::io;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

pub use item::{Category, Icon, ItemId, Pixmap, Pixmaps, Status, ToolTip, TrayItem};
pub use menu::{MenuItemKind, MenuNode, MenuTree, ToggleKind, ToggleState};
pub use model::TrayModel;
pub use ui_model::{apply_menu_event, dbus_menu_id, is_symbolic, menu_item_id, menu_model, menu_model_with, tray_icon_source};
pub use watcher::{WATCHER_NAME, WATCHER_PATH};

/// Aynı anda izlenen en fazla öğe; fazlası yok sayılır.
pub const MAX_ITEMS: usize = 64;
/// UI → servis komut kuyruğu kapasitesi.
const COMMAND_CAPACITY: usize = 64;

/// Menü açma isteğinin kimliği. Her açılış (sağ tık, yalnızca-menü öğeye sol tık, servisin
/// `Activate` yedeği) yeni bir kimlik alır; servis o açılış için okuduğu her ağacı bu kimlikle
/// yayınlar ([`TrayEvent::MenuLayout`]). Böylece:
/// - aynı ağaç yeniden geldiğinde bile model "yeni yanıt" olarak değişir (UI uyanır);
/// - önceki bir açılışın geç gelen yanıtı yeni isteği ezemez (kimlik tutmaz, atılır).
pub type MenuRequest = u64;

/// Süreç genelinde artan menü isteği kimliği (birden çok bar/monitör widget'ı çakışmasın).
static MENU_REQUEST: AtomicU64 = AtomicU64::new(1);

/// Yeni, benzersiz bir menü isteği kimliği.
pub fn next_menu_request() -> MenuRequest {
    MENU_REQUEST.fetch_add(1, Ordering::Relaxed)
}

/// Servisin oturumdaki rolü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayRole {
    /// `org.kde.StatusNotifierWatcher` bu servis.
    Watcher,
    /// Başka bir watcher'a host olarak kayıtlı.
    Host,
}

/// Servisin dışarı verdiği olaylar. [`TrayModel::apply`] ile modele uygulanır.
#[derive(Clone, PartialEq)]
pub enum TrayEvent {
    /// Bus'a bağlanıldı ve rol belirlendi; rol değişince (watcher devralma) tekrar gelir.
    Role(TrayRole),
    /// Oturum bus'ına bağlanılamadı; servis çalışmıyor.
    Failed(String),
    /// Yeni öğe (ilk okuması başarılı).
    ItemAdded(TrayItem),
    /// Öğenin özellikleri değişti (tam anlık görüntü).
    ItemUpdated(TrayItem),
    ItemRemoved(ItemId),
    /// Açık bir menünün güncel hali ([`TrayHandle::open_menu`] sonrası ve her değişiklikte);
    /// `request`: hangi açılışa ait olduğu.
    MenuLayout { item: ItemId, request: MenuRequest, menu: MenuTree },
    /// `Activate` veya `ContextMenu` uygulamada desteklenmiyor (hata döndü) ve öğenin menüsü
    /// var: UI menüyü kendisi açmalı. Servis menüyü `request` kimliğiyle okumaya başladı.
    MenuRequested { item: ItemId, request: MenuRequest },
}

/// Debug redakte (log politikası): öğe başlığı, ipucu ve menü etiketleri yazılmaz; yalnızca
/// öğe kimliği (bus adı + nesne yolu).
impl std::fmt::Debug for TrayEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Role(r) => f.debug_tuple("Role").field(r).finish(),
            Self::Failed(_) => f.write_str("Failed(..)"),
            Self::ItemAdded(i) => f.debug_tuple("ItemAdded").field(&i.id).finish(),
            Self::ItemUpdated(i) => f.debug_tuple("ItemUpdated").field(&i.id).finish(),
            Self::ItemRemoved(id) => f.debug_tuple("ItemRemoved").field(id).finish(),
            Self::MenuLayout { item, request, menu } => f
                .debug_struct("MenuLayout")
                .field("item", item)
                .field("request", request)
                .field("nodes", &menu.root.count())
                .finish(),
            Self::MenuRequested { item, request } => {
                f.debug_struct("MenuRequested").field("item", item).field("request", request).finish()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollOrientation {
    Vertical,
    Horizontal,
}

impl ScrollOrientation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
        }
    }
}

/// dbusmenu `Event` türleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEventKind {
    Clicked,
    Hovered,
    Opened,
    Closed,
}

impl MenuEventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Clicked => "clicked",
            Self::Hovered => "hovered",
            Self::Opened => "opened",
            Self::Closed => "closed",
        }
    }
}

/// Ayarlar. Varsayılanlar canlı oturum için; testler kısaltır.
#[derive(Debug, Clone)]
pub struct TrayOptions {
    /// D-Bus adresi; `None`: oturum bus'ı (`DBUS_SESSION_BUS_ADDRESS`).
    pub bus_address: Option<String>,
    /// Her D-Bus çağrısının zaman aşımı. `GetAll` büyük pixmap'lerle yavaş olabilir.
    pub call_timeout: Duration,
    pub coalesce_quiet: Duration,
    pub coalesce_max_delay: Duration,
    /// İlk okuması başarısız öğe için deneme sayısı (sonra kayıt silinir).
    pub initial_read_attempts: u32,
    /// İlk okuma denemeleri arası ilk bekleme (her denemede iki katı).
    pub retry_delay: Duration,
    /// Tema ikonlarının çözüldüğü fiziksel boyut (ikon boyu × çıktı ölçeği); çalışırken
    /// [`TrayHandle::set_icon_size`] ile değişir.
    pub icon_px: u32,
    /// Öğe başına iki özellik/menü okuması arasındaki en kısa süre (sinyal fırtınası sınırı).
    pub min_refresh_interval: Duration,
    /// Güvenilmeyen ikonların yalıtılmış çözücüsü (ADR-0007; shell genelinde tek Sandbox).
    /// `None`: bugünkü davranış (uygulamanın SVG'si reddedilir, raster süreç içinde).
    pub sandbox: Option<std::sync::Arc<dyn nuxut_image::IsolatedDecoder>>,
}

impl Default for TrayOptions {
    fn default() -> Self {
        Self {
            bus_address: None,
            call_timeout: Duration::from_secs(2),
            coalesce_quiet: Duration::from_millis(30),
            coalesce_max_delay: Duration::from_millis(200),
            initial_read_attempts: 4,
            retry_delay: Duration::from_millis(250),
            icon_px: 32,
            min_refresh_interval: Duration::from_millis(100),
            sandbox: None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Command {
    Activate { item: ItemId, x: i32, y: i32 },
    SecondaryActivate { item: ItemId, x: i32, y: i32 },
    ContextMenu { item: ItemId, x: i32, y: i32 },
    Scroll { item: ItemId, delta: i32, orientation: ScrollOrientation },
    OpenMenu { item: ItemId, request: MenuRequest },
    CloseMenu(ItemId),
    MenuAboutToShow { item: ItemId, ids: Vec<i32> },
    MenuEvent { item: ItemId, id: i32, kind: MenuEventKind },
    SetIconSize(u32),
}

/// UI'dan servise komut. Klonlanabilir; hiçbir metot bloklamaz (kuyruk doluysa komut
/// atılır ve `false` döner).
#[derive(Debug, Clone)]
pub struct TrayHandle {
    tx: async_channel::Sender<Command>,
}

impl TrayHandle {
    fn send(&self, command: Command) -> bool {
        let ok = self.tx.try_send(command).is_ok();
        if !ok {
            tracing::warn!("tepsi komut kuyruğu dolu veya servis durmuş, komut atıldı");
        }
        ok
    }

    /// Birincil tıklama. `x`, `y` ekran koordinatı (Wayland'de çoğu uygulama yok sayar).
    pub fn activate(&self, item: &ItemId, x: i32, y: i32) -> bool {
        self.send(Command::Activate { item: item.clone(), x, y })
    }

    /// Orta tıklama.
    pub fn secondary_activate(&self, item: &ItemId, x: i32, y: i32) -> bool {
        self.send(Command::SecondaryActivate { item: item.clone(), x, y })
    }

    /// Uygulamanın kendi bağlam menüsünü göstermesini ister. Menüsü olan öğeler için tercih
    /// edilen yol [`open_menu`](Self::open_menu)'dür (menüyü shell çizer).
    pub fn context_menu(&self, item: &ItemId, x: i32, y: i32) -> bool {
        self.send(Command::ContextMenu { item: item.clone(), x, y })
    }

    pub fn scroll(&self, item: &ItemId, delta: i32, orientation: ScrollOrientation) -> bool {
        self.send(Command::Scroll { item: item.clone(), delta, orientation })
    }

    /// Menüyü okur (`AboutToShow(0)` + `GetLayout`) ve `request` kimliğiyle
    /// [`TrayEvent::MenuLayout`] yayınlar; [`close_menu`](Self::close_menu)'ye ya da başka bir
    /// öğenin menüsü açılana kadar (aynı anda tek menü) değişiklikleri izler. Önceki
    /// açılışların süren okumaları atılır. `request`: [`next_menu_request`].
    pub fn open_menu(&self, item: &ItemId, request: MenuRequest) -> bool {
        self.send(Command::OpenMenu { item: item.clone(), request })
    }

    pub fn close_menu(&self, item: &ItemId) -> bool {
        self.send(Command::CloseMenu(item.clone()))
    }

    /// Alt menü(ler) açılıyor: `AboutToShow` / `AboutToShowGroup`; uygulama güncelleme
    /// gerektiğini söylerse alt ağaç yeniden okunur.
    pub fn menu_about_to_show(&self, item: &ItemId, ids: &[i32]) -> bool {
        self.send(Command::MenuAboutToShow { item: item.clone(), ids: ids.iter().copied().take(64).collect() })
    }

    /// Tema ikonlarının çözüleceği fiziksel boyut (ör. ikon boyu × en büyük çıktı ölçeği).
    /// Değişirse tüm ikonlar yeniden çözülür ve `ItemUpdated` gelir.
    pub fn set_icon_size(&self, px: u32) -> bool {
        self.send(Command::SetIconSize(px))
    }

    pub fn menu_event(&self, item: &ItemId, id: i32, kind: MenuEventKind) -> bool {
        self.send(Command::MenuEvent { item: item.clone(), id, kind })
    }
}

/// Çalışan tepsi servisi. Düşürülünce temiz kapanır.
#[derive(Debug)]
pub struct TrayService {
    handle: TrayHandle,
    /// Düşürülünce kanal kapanır → döngü durur.
    stop: Option<async_channel::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

/// Aynı süreçte birden çok servis (testler) farklı host adı alsın.
static HOST_COUNTER: AtomicU32 = AtomicU32::new(1);

impl TrayService {
    /// Servisi başlatır. `sink` servis thread'inden çağrılır; hızlı dönmeli. Bus'a
    /// bağlanma arka planda yapılır; başarısızlık [`TrayEvent::Failed`] olarak gelir.
    pub fn spawn<F>(options: TrayOptions, sink: F) -> io::Result<Self>
    where
        F: Fn(TrayEvent) + Send + 'static,
    {
        let (stop_tx, stop_rx) = async_channel::bounded(1);
        let (cmd_tx, cmd_rx) = async_channel::bounded(COMMAND_CAPACITY);
        let host_name = format!("org.kde.StatusNotifierHost-{}-{}", std::process::id(), HOST_COUNTER.fetch_add(1, Ordering::Relaxed));
        let thread = std::thread::Builder::new()
            .name("nuxut-tray".into())
            .spawn(move || host::run(options, host_name, Box::new(sink), stop_rx, cmd_rx))?;
        Ok(Self { handle: TrayHandle { tx: cmd_tx }, stop: Some(stop_tx), thread: Some(thread) })
    }

    pub fn handle(&self) -> TrayHandle {
        self.handle.clone()
    }
}

impl Drop for TrayService {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("tepsi servis thread'i panikle sonlandı");
        }
    }
}
