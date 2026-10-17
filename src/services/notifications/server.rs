//! `org.freedesktop.Notifications` D-Bus sunucusu: [`NotificationServer`] ve [`NotifyHandle`].
//!
//! Sunucu modeli tutmaz (o UI'daki `NotificationCenter`'dır); yalnızca canlı kimliklerin küçük
//! bir kaydını tutar: kimlik dağıtımı, `replaces_id`, eylem/`resident` doğrulaması ve
//! kapanışta tek `NotificationClosed` için.
//!
//! Thread'ler:
//! - zbus'un iç yürütücü thread'i yöntem çağrılarını işler ve ad kaybını (`NameLost`) izler.
//! - `nuxut-notify` thread'i D-Bus sinyallerini sırayla yayar; `NotifyHandle` çağrıları ona
//!   kanalla iletilir, böylece UI thread'i asla D-Bus'ta beklemez.
//!
//! Kayıt ve olay callback'i tek kilit altındadır: callback'e giden olay sırası kayıttaki
//! değişiklik sırasıyla aynıdır. Callback hızlı dönmelidir (kanala göndermek).

use std::collections::{HashMap, VecDeque};
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use zbus::blocking;
use zbus::export::futures_core::Stream;
use zbus::fdo::{self, RequestNameFlags, RequestNameReply};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;

use nuxut_image::{Decoded, IconTheme, IsolatedDecoder, LoadHandle, LoadKeys, Loader};

use super::budget::{self, DecodeBudget, Reservation};
use super::hints::{HintValue, Hints, NotifyIcon, RawIcon, parse_icon_ref};
use super::images::{self, PendingImage, Slot};
use super::sender::SenderKeys;
use crate::services::image_sandbox::{KeyDomain, source_key};
use super::markup::{parse_markup, sanitize, truncate_bytes, truncate_chars, validate_link};
use super::model::{CloseReason, Expiry, Notification, NotifyAction, NotifyEvent};

pub const BUS_NAME: &str = "org.freedesktop.Notifications";
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";
pub const INTERFACE: &str = "org.freedesktop.Notifications";

pub const MAX_APP_NAME_CHARS: usize = 128;
pub const MAX_SUMMARY_CHARS: usize = 256;
pub const MAX_BODY_BYTES: usize = 4 * 1024;
/// `default` hariç düğme sayısı.
pub const MAX_ACTIONS: usize = 8;
pub const MAX_ACTION_TEXT_CHARS: usize = 64;
pub const MAX_TOKEN_BYTES: usize = 512;
/// Kayıttaki en fazla canlı bildirim; UI hiç kapatmasa bile bellek sınırlı kalır.
pub const MAX_LIVE: usize = 1024;

/// Sunucu ayarları ([`NotificationServer::spawn`] varsayılanları kullanır).
#[derive(Debug, Clone, Default)]
pub struct ServerOptions {
    /// Ad başkasındaysa `DBUS_NAME_FLAG_REPLACE_EXISTING` ile devralmayı dene.
    pub replace: bool,
    /// Başka bir sunucu `replace` ile adı bizden alabilsin. Alırsa tüm bildirimler
    /// `Closed { reason: Undefined }` olur ve sunucu yeni bildirim kabul etmez.
    pub allow_replacement: bool,
    /// Bus adresi; `None` → oturum bus'ı. Testler özel bus verir.
    pub address: Option<String>,
    /// `Added`'ın görsel çözümü için en fazla bekletilmesi; `None` → [`IMAGE_WAIT`].
    pub image_wait: Option<Duration>,
    /// Bekleyen görsel çözmelerinin (toplam, bildirim başına) bayt bütçesi; `None` →
    /// [`budget::MAX_PENDING_IMAGE_BYTES`], [`budget::MAX_PENDING_IMAGE_BYTES_PER_NOTIFICATION`].
    pub pending_image_budget: Option<(usize, usize)>,
    /// Güvenilmeyen görsellerin yalıtılmış çözücüsü (ADR-0007; shell genelinde tek Sandbox).
    /// `None`: bugünkü davranış (dosyadaki SVG reddedilir, raster süreç içinde).
    pub sandbox: Option<Arc<dyn IsolatedDecoder>>,
}

#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    #[error("{BUS_NAME} adı başka bir süreçte ({owner}); bildirim sunucusu devre dışı")]
    NameTaken { owner: String },
    #[error("D-Bus: {0}")]
    Bus(#[from] zbus::Error),
    #[error("D-Bus: {0}")]
    Fdo(#[from] fdo::Error),
    #[error("thread başlatılamadı: {0}")]
    Io(#[from] std::io::Error),
}

/// Görsel çözülürken `Added`'ın en fazla bekletileceği süre; aşılırsa bildirim görselsiz
/// eklenir, görsel hazır olunca `ImageReady` gelir.
pub const IMAGE_WAIT: Duration = Duration::from_millis(100);

/// Görsel çözme thread'lerinin üst sınırı. Eşzamanlı çözme başına geçici bellek en çok
/// `Limits::ICON` tuvali kadardır; sınır bu tepe değeri de tutar.
pub const DECODE_THREADS_MAX: usize = 3;

/// Çözme thread sayısı: çekirdeklerin yarısı, 1..=[`DECODE_THREADS_MAX`] (UI'ya pay kalır).
pub fn decode_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    (cores / 2).clamp(1, DECODE_THREADS_MAX)
}

/// Sinyal thread'ine giden işler.
#[derive(Debug)]
enum Outbound {
    Closed {
        id: u32,
        reason: CloseReason,
    },
    ActivationToken {
        id: u32,
        token: String,
    },
    ActionInvoked {
        id: u32,
        key: String,
    },
    /// Yeni bekleyen `Added` var: zamanlayıcıyı yeniden kur.
    Wake,
    Stop,
}

impl Outbound {
    /// Log için tür adı (jeton ve eylem anahtarı yazılmaz).
    fn kind(&self) -> &'static str {
        match self {
            Self::Closed { .. } => "NotificationClosed",
            Self::ActivationToken { .. } => "ActivationToken",
            Self::ActionInvoked { .. } => "ActionInvoked",
            Self::Wake => "Wake",
            Self::Stop => "Stop",
        }
    }
}

#[derive(Debug)]
struct Live {
    /// `default` dahil geçerli eylem anahtarları.
    actions: Vec<String>,
    resident: bool,
    /// Her `Notify`'da artar; eski (değiştirilmiş) bildirimin çözme sonucu atılır.
    generation: u64,
    /// Süren görsel çözme işleri (kapanınca/değişince iptal).
    loads: Vec<LoadHandle>,
}

/// Görseli çözülürken bekletilen `Added`.
#[derive(Debug)]
struct PendingAdd {
    notification: Notification,
    remaining: usize,
    deadline: Instant,
}

type Sink = Box<dyn Fn(NotifyEvent) + Send>;

struct Registry {
    live: HashMap<u32, Live>,
    /// Ekleme sırası (en eskisi önde); `MAX_LIVE` taşmasında en eskisi kapanır.
    order: VecDeque<u32>,
    pending: HashMap<u32, PendingAdd>,
    last_id: u32,
    last_generation: u64,
    image_wait: Duration,
    /// Ad kaybedildi; yeni bildirim kabul edilmez.
    stopped: bool,
    sink: Sink,
    signals: Sender<Outbound>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("live", &self.live.len())
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

impl Registry {
    fn alloc_id(&mut self) -> u32 {
        loop {
            // Dahili bildirim aralığı (`INTERNAL_ID_BASE..`) hiç verilmez.
            self.last_id = self.last_id.wrapping_add(1) % super::internal::INTERNAL_ID_BASE;
            if self.last_id != 0 && !self.live.contains_key(&self.last_id) {
                return self.last_id;
            }
        }
    }

    /// Canlıysa kaldırır, çözme işlerini iptal eder, sinyali kuyruğa koyar ve UI'ya `Closed`
    /// verir (görseli beklerken kapanan bildirim için UI `Added` görmemiş olabilir; zararsız).
    fn close(&mut self, id: u32, reason: CloseReason) -> bool {
        let Some(live) = self.live.remove(&id) else {
            return false;
        };
        live.loads.iter().for_each(LoadHandle::cancel);
        self.pending.remove(&id);
        self.order.retain(|&x| x != id);
        let _ = self.signals.send(Outbound::Closed { id, reason });
        (self.sink)(NotifyEvent::Closed { id, reason });
        true
    }

    /// Süresi dolan bekleyen `Added`'ları (görsel tamamlanmamış olsa da) yayınlar; en yakın
    /// sonraki süreyi döndürür.
    fn flush_pending(&mut self, now: Instant) -> Option<Instant> {
        let due: Vec<u32> = self.pending.iter().filter(|(_, p)| p.deadline <= now).map(|(&id, _)| id).collect();
        for id in due {
            if let Some(p) = self.pending.remove(&id) {
                (self.sink)(NotifyEvent::Added(p.notification));
            }
        }
        self.pending.values().map(|p| p.deadline).min()
    }

    /// Çözme sonucu: bildirim hâlâ aynı kuşaktaysa bekleyen `Added`'a yazılır (son görselse
    /// `Added` gider), `Added` zaten gittiyse `ImageReady` olur.
    fn image_decoded(&mut self, id: u32, generation: u64, slot: Slot, icon: NotifyIcon) {
        if self.live.get(&id).map(|l| l.generation) != Some(generation) {
            return;
        }
        if let Some(p) = self.pending.get_mut(&id) {
            // Başarısız çözme yer tutucuyu (ör. bulunamayan tema adı) ezmez.
            if !icon.is_none() {
                match slot {
                    Slot::AppIcon => p.notification.app_icon = icon,
                    Slot::Image => p.notification.image = icon,
                }
            }
            p.remaining = p.remaining.saturating_sub(1);
            if p.remaining == 0
                && let Some(p) = self.pending.remove(&id)
            {
                (self.sink)(NotifyEvent::Added(p.notification));
            }
        } else if !icon.is_none() {
            let (app_icon, image) = match slot {
                Slot::AppIcon => (Some(icon), None),
                Slot::Image => (None, Some(icon)),
            };
            (self.sink)(NotifyEvent::ImageReady { id, app_icon, image });
        }
    }
}

#[derive(Debug)]
struct Shared {
    registry: Mutex<Registry>,
    /// Görsel çözme havuzu (`nuxut-image`); sunucu düşerken `None` yapılıp join edilir.
    loader: Mutex<Option<Loader>>,
    /// Bekleyen çözmelerin bayt bütçesi (bellek şişirmeye karşı).
    budget: Arc<DecodeBudget>,
    /// Tema ikon adlarını çözmek için; arama Loader thread'inde yapılır.
    theme: Arc<IconTheme>,
    /// Uygulama simgesinin fiziksel kenarı (`images::app_icon_side`, çıkış ölçeğinden).
    app_icon_side: AtomicU32,
    /// Gönderen → görsel kaynak anahtarı (ADR-0007; `app_name` değil).
    senders: SenderKeys,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Registry> {
        // Kilit altında panik olmaz (sink paniklemedikçe); olsa bile kayıt tutarlı kalır.
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Görselleri çözme havuzuna verir ve iptal tutamaklarını bildirime bağlar. Kayıt kilidi
    /// tutulmadan çağrılmalı: Loader kuyruk kapalıysa callback'i hemen (bu thread'de) çağırır.
    ///
    /// `sender` gönderenin kaynak anahtarıdır ([`SenderKeys`]). İptal anahtarı (ADR-0007)
    /// gönderen + görsel yeri + bildirim kimliğidir: aynı gönderenin farklı bildirimleri birbirinin
    /// görselini iptal etmez; `replaces_id` ile değiştirilen bildirimin eski görseli iptal edilir
    /// (kuşak denetimi zaten yok sayar).
    /// Gerekçe: `image_sandbox` modül belgesi.
    fn submit(self: &Arc<Self>, id: u32, generation: u64, sender: u64, images: Vec<(PendingImage, Option<Reservation>)>) {
        let mut handles = Vec::with_capacity(images.len());
        {
            let mut loader = self.loader.lock().unwrap_or_else(PoisonError::into_inner);
            for (job, reservation) in images {
                let slot = job.slot;
                let domain = match slot {
                    Slot::AppIcon => KeyDomain::NotifyAppIcon,
                    Slot::Image => KeyDomain::NotifyImage,
                };
                // İptal: gönderen + yuva + bildirim kimliği (art arda gelen bildirimler birbirini
                // iptal etmez, `replaces_id` aynı kimlikle eskisini iptal eder). Kaynak: gönderen
                // (susturma, hız sınırı ve negatif önbellek gönderen başına ortak).
                let keys = LoadKeys { cancel: source_key(domain, &(sender, id)), source: sender };
                let weak: Weak<Self> = Arc::downgrade(self);
                let request = job.request(&self.theme, self.app_icon_side.load(Ordering::Relaxed));
                let done = move |result: Result<Decoded, nuxut_image::ImageError>| {
                    // Kodlanmış baytlar artık gerekmiyor: bütçe geri verilir (iş hiç
                    // çağrılmadan düşerse de `Drop` ile).
                    drop(reservation);
                    let Some(shared) = weak.upgrade() else { return };
                    let icon = match result {
                        Ok(decoded) => images::to_icon(decoded),
                        Err(nuxut_image::ImageError::Cancelled) => return,
                        Err(e) => {
                            // Yalıtım hataları (zaman aşımı, öldü, hız sınırı) dahil: yedek ikon.
                            tracing::warn!(error = %e, "bildirim görseli çözülemedi");
                            NotifyIcon::None
                        }
                    };
                    shared.lock().image_decoded(id, generation, slot, icon);
                };
                match loader.as_mut() {
                    Some(loader) => handles.push(loader.load_with_keys(keys, request, done)),
                    None => done(Err(nuxut_image::ImageError::Cancelled)),
                }
            }
        }
        let mut reg = self.lock();
        match reg.live.get_mut(&id).filter(|l| l.generation == generation) {
            Some(live) => live.loads.extend(handles),
            None => handles.iter().for_each(LoadHandle::cancel),
        }
    }
}

// ---------------------------------------------------------------------------------------
// İstek ayrıştırma

/// `Notify` argümanlarını sınırlar, temizler ve bildirime çevirir (`id` sonra atanır).
/// Dosyadan okunmuş kodlanmış görseller bildirimde [`NotifyIcon::None`] olarak kalır ve
/// çözülmek üzere ikinci değerde döner.
#[allow(clippy::too_many_arguments)]
pub fn build_notification(
    app_name: &str,
    app_icon: &str,
    summary: &str,
    body: &str,
    actions: &[String],
    hints: &HashMap<String, HintValue>,
    expire_timeout: i32,
) -> (Notification, Vec<PendingImage>) {
    let hints = Hints::parse(hints);
    // `app_icon` yoksa masaüstü girdisinin simgesi.
    let app_icon = match parse_icon_ref(app_icon) {
        RawIcon::None => hints.desktop_entry.as_deref().and_then(images::desktop_entry_icon).unwrap_or_default(),
        icon => icon,
    };
    let (app_icon, app_icon_job) = images::split(app_icon, Slot::AppIcon);
    let (image, image_job) = images::split(hints.image, Slot::Image);
    let jobs: Vec<PendingImage> = app_icon_job.into_iter().chain(image_job).collect();
    let one_line = |s: &str, max| sanitize(truncate_chars(s.trim(), max)).replace(['\n', '\t'], " ");

    let truncated = body.len() > MAX_BODY_BYTES;
    let mut body = truncate_bytes(body, MAX_BODY_BYTES);
    if truncated && let Some(open) = body.rfind('<').filter(|&lt| !body[lt..].contains('>')) {
        // Kesme bir etiketin ortasına denk geldiyse yarım etiketi at.
        body = &body[..open];
    }

    let mut has_default_action = false;
    let mut buttons: Vec<NotifyAction> = Vec::new();
    for pair in actions.chunks_exact(2) {
        let key = sanitize(truncate_chars(&pair[0], MAX_ACTION_TEXT_CHARS));
        if key == "default" {
            has_default_action = true;
            continue;
        }
        if key.is_empty() || buttons.len() >= MAX_ACTIONS || buttons.iter().any(|a| a.key == key) {
            continue;
        }
        buttons.push(NotifyAction { key, label: one_line(&pair[1], MAX_ACTION_TEXT_CHARS) });
    }

    let notification = Notification {
        id: 0,
        app_name: one_line(app_name, MAX_APP_NAME_CHARS),
        app_icon,
        summary: one_line(summary, MAX_SUMMARY_CHARS),
        body: parse_markup(body.trim()),
        actions: buttons,
        has_default_action,
        urgency: hints.urgency,
        expire: Expiry::from_dbus(expire_timeout),
        image,
        category: hints.category,
        desktop_entry: hints.desktop_entry,
        transient: hints.transient,
        resident: hints.resident,
        suppress_sound: hints.suppress_sound,
        received: SystemTime::now(),
        // Maskot yalnızca dahili bildirimlerde (`InternalNotice::with_mascot`); hiçbir hint onu kurmaz.
        mascot: None,
    };
    (notification, jobs)
}

// ---------------------------------------------------------------------------------------
// D-Bus arayüzü

struct Server {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    // Eşzamansız: görselli bildirimde gönderenin kimliği bus'a sorulur (bağlantı başına bir kez).
    // Kayıt (kimlik, sıra) beklemeden önce yapılır; yalnızca görsel gönderimi bekler.
    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, HintValue>,
        expire_timeout: i32,
    ) -> fdo::Result<u32> {
        let (mut n, jobs) = build_notification(&app_name, &app_icon, &summary, &body, &actions, &hints, expire_timeout);
        // Sığmayan görsel düşer; bildirim görselsiz kalır (yer tutucu zaten None).
        let (jobs, _dropped) = self.shared.budget.admit(jobs);
        let (id, generation) = {
            let mut reg = self.shared.lock();
            if reg.stopped {
                return Err(fdo::Error::Failed("bildirim sunucusu durdu".into()));
            }
            let id = if replaces_id != 0 && reg.live.contains_key(&replaces_id) {
                reg.order.retain(|&x| x != replaces_id);
                replaces_id
            } else {
                reg.alloc_id()
            };
            let mut keys: Vec<String> = n.actions.iter().map(|a| a.key.clone()).collect();
            if n.has_default_action {
                keys.push("default".into());
            }
            reg.last_generation += 1;
            let generation = reg.last_generation;
            let old = reg.live.insert(id, Live { actions: keys, resident: n.resident, generation, loads: Vec::new() });
            // Değiştirilen bildirimin süren çözmeleri ve bekleyen `Added`'ı geçersiz.
            if let Some(old) = old {
                old.loads.iter().for_each(LoadHandle::cancel);
            }
            reg.pending.remove(&id);
            reg.order.push_back(id);
            n.id = id;
            if jobs.is_empty() {
                (reg.sink)(NotifyEvent::Added(n));
            } else {
                let deadline = Instant::now() + reg.image_wait;
                reg.pending.insert(id, PendingAdd { notification: n, remaining: jobs.len(), deadline });
                let _ = reg.signals.send(Outbound::Wake);
            }
            while reg.order.len() > MAX_LIVE {
                let Some(oldest) = reg.order.front().copied() else { break };
                reg.close(oldest, CloseReason::Expired);
            }
            (id, generation)
        };
        if !jobs.is_empty() {
            // Kaynak anahtarı istemcinin `app_name`'i değil, D-Bus'ın doğruladığı gönderen.
            let sender = self.shared.senders.key(conn, header.sender()).await;
            self.shared.submit(id, generation, sender, jobs);
        }
        Ok(id)
    }

    fn close_notification(&self, id: u32) {
        // Spec: yoksa boş hata dönülebilir; istemciler bunu beklemiyor, sessizce yok sayılır.
        self.shared.lock().close(id, CloseReason::ClosedByCall);
    }

    fn get_capabilities(&self) -> Vec<&'static str> {
        vec!["actions", "body", "body-markup", "body-hyperlinks", "icon-static", "persistence"]
    }

    fn get_server_information(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        ("nuxut", "nuxut", env!("CARGO_PKG_VERSION"), "1.2")
    }

    // Sinyaller tanıtım (introspection) için burada; yayını sinyal thread'i yapar.
    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn activation_token(emitter: &SignalEmitter<'_>, id: u32, activation_token: &str) -> zbus::Result<()>;
}

// ---------------------------------------------------------------------------------------
// Servis

/// Çalışan bildirim sunucusu. Düşürülünce adı bırakır, sinyal thread'ini join eder.
#[derive(Debug)]
pub struct NotificationServer {
    handle: NotifyHandle,
    signals: Sender<Outbound>,
    worker: Option<JoinHandle<()>>,
    name_watch: Option<zbus::Task<()>>,
}

/// UI'nın sunucuya komutları. `Clone + Send`; çağrılar kısa bir kilit alır ve D-Bus'ta
/// beklemez. Sunucu düştükten sonra çağrılar etkisizdir.
#[derive(Debug, Clone)]
pub struct NotifyHandle {
    shared: Arc<Shared>,
    conn: blocking::Connection,
}

impl NotificationServer {
    /// Oturum bus'ında varsayılan ayarlarla başlatır.
    pub fn spawn<F>(sink: F) -> Result<Self, NotifyError>
    where
        F: Fn(NotifyEvent) + Send + 'static,
    {
        Self::spawn_with(ServerOptions::default(), sink)
    }

    /// Bus'a bağlanır, arayüzü yayınlar ve adı ister. Ad başkasındaysa (ve `replace` yoksa ya
    /// da devralma reddedildiyse) [`NotifyError::NameTaken`]: çağıran log'lar, shell
    /// bildirimsiz devam eder. `sink` zbus ve UI (handle) thread'lerinden çağrılır.
    pub fn spawn_with<F>(options: ServerOptions, sink: F) -> Result<Self, NotifyError>
    where
        F: Fn(NotifyEvent) + Send + 'static,
    {
        let (signals, signal_rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            registry: Mutex::new(Registry {
                live: HashMap::new(),
                order: VecDeque::new(),
                pending: HashMap::new(),
                last_id: 0,
                last_generation: 0,
                image_wait: options.image_wait.unwrap_or(IMAGE_WAIT),
                stopped: false,
                sink: Box::new(sink),
                signals: signals.clone(),
            }),
            // İlk görselde başlar. Görseller seyrektir ama patlama hâlinde (aynı anda çok
            // bildirim) tek thread kuyruğu çekirdek sayısından bağımsız sıraya dizer: büyük bir
            // görsel ~100 ms sürer. Yarı çekirdek, en çok DECODE_THREADS_MAX.
            loader: Mutex::new(Some(match options.sandbox.clone() {
                Some(sandbox) => Loader::new(decode_threads()).with_sandbox(sandbox),
                None => Loader::new(decode_threads()),
            })),
            budget: {
                let (total, per) = options.pending_image_budget.unwrap_or((
                    budget::MAX_PENDING_IMAGE_BYTES,
                    budget::MAX_PENDING_IMAGE_BYTES_PER_NOTIFICATION,
                ));
                DecodeBudget::new(total, per)
            },
            theme: Arc::new(IconTheme::system()),
            app_icon_side: AtomicU32::new(images::app_icon_side(1.0)),
            senders: SenderKeys::default(),
        });
        let builder = match &options.address {
            Some(address) => blocking::connection::Builder::address(address.as_str())?,
            None => blocking::connection::Builder::session()?,
        };
        let conn = builder.serve_at(OBJECT_PATH, Server { shared: Arc::clone(&shared) })?.build()?;

        // NameLost aboneliği ad istenmeden önce kurulur ki sinyal kaçmasın.
        let lost_stream = zbus::block_on(async {
            let proxy = fdo::DBusProxy::new(conn.inner()).await?;
            proxy.receive_name_lost_with_args(&[(0, BUS_NAME)]).await
        })?;

        let mut flags = RequestNameFlags::DoNotQueue.into();
        if options.replace {
            flags |= RequestNameFlags::ReplaceExisting;
        }
        if options.allow_replacement {
            flags |= RequestNameFlags::AllowReplacement;
        }
        match conn.request_name_with_flags(BUS_NAME, flags) {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
            Ok(RequestNameReply::Exists | RequestNameReply::InQueue) | Err(zbus::Error::NameTaken) => {
                let owner = blocking::fdo::DBusProxy::new(&conn)
                    .ok()
                    .and_then(|p| p.get_name_owner(BUS_NAME.try_into().ok()?).ok())
                    .map_or_else(|| "bilinmiyor".to_owned(), |o| o.to_string());
                return Err(NotifyError::NameTaken { owner });
            }
            Err(e) => return Err(e.into()),
        }

        let name_watch = {
            let shared = Arc::clone(&shared);
            let mut stream = lost_stream;
            conn.inner().executor().spawn(
                async move {
                    if poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await.is_some() {
                        tracing::warn!("{BUS_NAME} adı başka bir sunucuya geçti; bildirim sunucusu durdu");
                        let mut reg = shared.lock();
                        reg.stopped = true;
                        let ids: Vec<u32> = reg.order.drain(..).collect();
                        for live in reg.live.values() {
                            live.loads.iter().for_each(LoadHandle::cancel);
                        }
                        reg.live.clear();
                        reg.pending.clear();
                        for id in ids {
                            (reg.sink)(NotifyEvent::Closed { id, reason: CloseReason::Undefined });
                        }
                    }
                },
                "nuxut-notify-name-lost",
            )
        };

        let worker = {
            let conn = conn.clone();
            let shared = Arc::downgrade(&shared);
            std::thread::Builder::new()
                .name("nuxut-notify".into())
                .spawn(move || signal_loop(&signal_rx, &conn, &shared))?
        };
        tracing::info!("bildirim sunucusu {BUS_NAME} adını aldı");
        Ok(Self { handle: NotifyHandle { shared, conn }, signals, worker: Some(worker), name_watch: Some(name_watch) })
    }

    pub fn handle(&self) -> NotifyHandle {
        self.handle.clone()
    }
}

impl NotifyHandle {
    /// Bildirimi kapatır: `NotificationClosed(id, reason)` ve UI'ya `Closed`. Kullanıcı
    /// kapatınca `Dismissed`, model kapatınca `NotificationCenter::take_closed`'daki neden.
    pub fn close(&self, id: u32, reason: CloseReason) {
        self.shared.lock().close(id, reason);
    }

    /// Çıkış ölçeği (çok monitörde en büyüğü): bundan sonra çözülen uygulama simgeleri
    /// `64 × ölçek` fiziksel piksel olur (üst sınır 128). Mevcut bildirimler değişmez.
    /// Bekleyen görsel çözmelerinin ayırdığı kodlanmış bayt (bütçe gözlemi, testler).
    pub fn pending_image_bytes(&self) -> usize {
        self.shared.budget.used()
    }

    pub fn set_output_scale(&self, scale: f32) {
        self.shared.app_icon_side.store(images::app_icon_side(scale), Ordering::Relaxed);
    }

    /// Eylem seçildi: `ActionInvoked`; `resident` değilse ardından `Dismissed` ile kapanır.
    /// Geçersiz kimlik/anahtar yok sayılır (`false`).
    pub fn invoke_action(&self, id: u32, key: &str) -> bool {
        self.invoke_action_with_token(id, key, None)
    }

    /// [`invoke_action`](Self::invoke_action); xdg-activation belirteci varsa
    /// `ActivationToken` sinyali `ActionInvoked`'dan önce gönderilir (spec 1.2).
    pub fn invoke_action_with_token(&self, id: u32, key: &str, activation_token: Option<&str>) -> bool {
        let mut reg = self.shared.lock();
        let Some(live) = reg.live.get(&id).filter(|l| l.actions.iter().any(|k| k == key)) else {
            return false;
        };
        let resident = live.resident;
        if let Some(token) = activation_token.filter(|t| !t.is_empty() && t.len() <= MAX_TOKEN_BYTES) {
            let _ = reg.signals.send(Outbound::ActivationToken { id, token: token.to_owned() });
        }
        let _ = reg.signals.send(Outbound::ActionInvoked { id, key: key.to_owned() });
        if !resident {
            reg.close(id, CloseReason::Dismissed);
        }
        true
    }

    /// Kullanıcının tıkladığı gövde bağlantısını `xdg-desktop-portal` `OpenURI` ile açar
    /// (xdg-open çalıştırılmaz; varsayılan uygulamayı portal seçer). Yalnızca
    /// http/https/mailto; bağlantı yeniden doğrulanır ve normalleştirilmiş hali açılır
    /// (`BodyLink::url` verilmesi önerilir). İstek arka planda gider; reddedilende `false`.
    pub fn open_link(&self, url: &str, activation_token: Option<&str>) -> bool {
        let Some(link) = validate_link(url) else {
            tracing::warn!(len = url.len(), "güvensiz bağlantı açılmadı");
            return false;
        };
        let conn = self.conn.inner().clone();
        let url = link.url;
        let mut options: HashMap<&'static str, zbus::zvariant::OwnedValue> = HashMap::new();
        if let Some(token) = activation_token.filter(|t| !t.is_empty() && t.len() <= MAX_TOKEN_BYTES)
            && let Ok(value) = zbus::zvariant::Value::from(token).try_to_owned()
        {
            options.insert("activation_token", value);
        }
        let task = {
            let conn = conn.clone();
            async move {
                let reply = conn
                    .call_method(
                        Some("org.freedesktop.portal.Desktop"),
                        "/org/freedesktop/portal/desktop",
                        Some("org.freedesktop.portal.OpenURI"),
                        "OpenURI",
                        &("", url.as_str(), options),
                    )
                    .await;
                if let Err(e) = reply {
                    tracing::warn!(error = %e, "portal OpenURI başarısız");
                }
            }
        };
        conn.executor().spawn(task, "nuxut-notify-open-uri").detach();
        true
    }
}

impl Drop for NotificationServer {
    fn drop(&mut self) {
        let _ = self.signals.send(Outbound::Stop);
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            tracing::error!("bildirim sinyal thread'i panikle sonlandı");
        }
        // Ad bırakılmadan önce ad kaybı izleyicisini durdur (kendi bırakmamız "kayıp" değil).
        drop(self.name_watch.take());
        self.handle.shared.lock().stopped = true;
        // Çözme havuzu: bekleyen işler iptal, süren iş biter, thread'ler join edilir. Kayıt
        // kilidi tutulmuyor (callback'ler onu alır).
        let loader = self.handle.shared.loader.lock().unwrap_or_else(PoisonError::into_inner).take();
        drop(loader);
        let _ = self.handle.conn.release_name(BUS_NAME);
    }
}

/// D-Bus sinyallerini sırayla yayar; ayrıca görsel bekleyen `Added`'ların 100 ms
/// zamanlayıcısıdır.
fn signal_loop(rx: &Receiver<Outbound>, conn: &blocking::Connection, shared: &Weak<Shared>) {
    let mut next_deadline: Option<Instant> = None;
    loop {
        let received = match next_deadline {
            Some(deadline) => match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(signal) => Some(signal),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(signal) => Some(signal),
                Err(_) => return,
            },
        };
        next_deadline = match shared.upgrade() {
            Some(shared) => shared.lock().flush_pending(Instant::now()),
            None => None,
        };
        let Some(signal) = received else { continue };
        let result = match &signal {
            Outbound::Stop => return,
            Outbound::Wake => continue,
            Outbound::Closed { id, reason } => {
                conn.emit_signal(None::<&str>, OBJECT_PATH, INTERFACE, "NotificationClosed", &(*id, *reason as u32))
            }
            Outbound::ActivationToken { id, token } => {
                conn.emit_signal(None::<&str>, OBJECT_PATH, INTERFACE, "ActivationToken", &(*id, token.as_str()))
            }
            Outbound::ActionInvoked { id, key } => {
                conn.emit_signal(None::<&str>, OBJECT_PATH, INTERFACE, "ActionInvoked", &(*id, key.as_str()))
            }
        };
        if let Err(e) = result {
            tracing::warn!(signal = signal.kind(), error = %e, "bildirim sinyali gönderilemedi");
        }
    }
}
