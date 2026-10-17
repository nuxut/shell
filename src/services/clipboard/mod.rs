//! Pano geçmişi servisi (t060; tasarım `docs/design/clipboard.md`).
//!
//! Pano izleyicisi odak istemeyen data-control protokolüyle çalışır; Wayland bağlantısı
//! framework'ündür (`nuxut-wayland` `DataControl`). Shell yalnızca [`ClipboardSource`]
//! trait'ini görür: olaylar [`SourceSink`] ile servise gelir, okuma ve seçimi koyma trait
//! üzerinden yapılır. Framework adaptörü [`source::DataControlSource`]; yeniden başlatma
//! politikası [`source::Restart`].
//!
//! Tek thread (`nuxut-clipboard`): teklifi gizlilik filtresinden geçirir ([`filter`]; hassas
//! teklif **okunmaz**), kabul edilen tek MIME'ı sınırlı ve süreli okur (bloklayan okuma ana
//! thread dışında), geçmişi tutar ([`history`]), isteğe bağlı diske yazar ([`store`]; 2 sn
//! birleştirme) ve her değişiklikte [`ClipboardEvent::Snapshot`] yayınlar. Art arda gelen
//! tekliflerden yalnızca sonuncusu okunur (eskimiş teklif okunmaz).
//!
//! Gizlilik bağlamı (oturum kilidi, etkin pencere sınıfı) [`Gate`] ile paylaşılır: ana thread
//! günceller, karar anında en güncel değer okunur.
//!
//! Tüm içerik kullanıcı verisidir: loglanmaz; `Debug` yalnızca tür ve uzunluk yazar; hata
//! iletileri içerik taşımaz.

#[cfg(feature = "clipboard-demo")]
pub mod demo;
pub mod filter;
pub mod history;
pub mod source;
pub mod store;
pub mod takeover;
pub mod x11;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use nuxut::i18n::t;
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::schema::{SettingSchema, SettingValue};

pub use history::{Item, ItemId};

/// Bir teklifin okunma süresi sınırı.
pub const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// Diske yazım birleştirme süresi.
pub const PERSIST_DELAY: Duration = Duration::from_secs(2);
/// Servis kuyruğu (dolarsa teklif ya da komut atılır; teklif atılması güvenlidir).
const QUEUE: usize = 64;

/// Kaynağın teklif kimliği (framework'ün teklif nesnesine karşılık).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OfferId(pub u64);

/// Kaynaktan gelen olay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceEvent {
    /// Yeni seçim. `mimes`: sunulan türler (framework ≤ 64 öğe, her biri ≤ 256 bayt kırpar);
    /// `flags`: framework işaretleri (`own`, `sensitive`, `truncated`; her biri okumayı engeller).
    Offered { id: OfferId, mimes: Vec<String>, flags: filter::Flags },
    /// Seçim temizlendi (geçmiş değişmez).
    Cleared,
}

/// Okuma hatası (içerik taşımaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("sınırı aşıyor")]
    TooLarge,
    #[error("zaman aşımı")]
    Timeout,
    #[error("teklif geçersiz")]
    Gone,
    #[error("okunamadı")]
    Failed,
}

/// Seçimi koyma hatası.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SetError {
    #[error("pano protokolü yok")]
    Unsupported,
    #[error("seçim konamadı")]
    Failed,
}

/// Pano kaynağı (framework `DataControl` adaptörü ya da testte sahte kaynak). Yalnızca
/// `Selection::Clipboard`. Her iki çağrı da servis thread'inden yapılır; bloklayabilir.
pub trait ClipboardSource: Send + Sync {
    /// Teklifin `mime` türünü en çok `max_bytes` ve `timeout` ile okur. Sınır aşılırsa okuma
    /// durur ([`ReadError::TooLarge`]).
    fn read(&self, id: OfferId, mime: &str, max_bytes: usize, timeout: Duration) -> Result<Zeroizing<Vec<u8>>, ReadError>;

    /// Seçimi bu (MIME, bayt) çiftleriyle koyar; odak/seri numarası istemez. Kaynak, shell
    /// yaşadıkça sunulmaya devam eder.
    fn set_selection(&self, data: Vec<(String, Arc<[u8]>)>) -> Result<(), SetError>;
}

/// Gizlilik bağlamı: ana thread günceller, servis ve VM pano köprüsü karar anında okur.
///
/// Oturum kilidi logind `LockedHint`'tir: kilit ekranı ayrı süreç (nuxut-lock) olduğundan
/// framework'ün kendi "kilitliyken okuma/koyma yok" kuralı kabukta hiç devreye girmez; kilitte
/// durması gereken her yol (geçmiş, köprü) bu kapıya bakar.
#[derive(Debug, Default)]
pub struct Gate {
    state: Mutex<GateState>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct GateState {
    locked: bool,
    active_class: Option<String>,
    /// `[clipboard] exclude_apps` (köprü için; geçmiş filtresi config'ten okur, aynı değer).
    exclude_apps: Vec<String>,
}

impl Gate {
    fn lock(&self) -> std::sync::MutexGuard<'_, GateState> {
        // Zehirlenmiş kilit: içerideki değer yine geçerli (düz alanlar).
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Oturum kilidi ve etkin pencere sınıfı. Değiştiyse `true`.
    pub fn set(&self, locked: bool, active_class: Option<&str>) -> bool {
        let mut s = self.lock();
        if s.locked == locked && s.active_class.as_deref() == active_class {
            return false;
        }
        s.locked = locked;
        s.active_class = active_class.map(str::to_owned);
        true
    }

    /// Hariç tutulan uygulamalar (config yüklenince ve değişince).
    pub fn set_exclude_apps(&self, apps: &[String]) {
        let mut s = self.lock();
        if s.exclude_apps != apps {
            s.exclude_apps = apps.to_vec();
        }
    }

    pub fn locked(&self) -> bool {
        self.lock().locked
    }

    /// Pano içeriği şu an taşınmamalı mı: oturum kilitli ya da etkin pencere hariç tutulan bir
    /// uygulama (VM pano köprüsünün iki yönü de; geçmiş filtresinin aynı iki kuralı).
    pub fn blocks_transfer(&self) -> bool {
        let s = self.lock();
        s.locked || filter::excluded(s.active_class.as_deref(), &s.exclude_apps)
    }

    fn get(&self) -> GateState {
        self.lock().clone()
    }
}

/// `[clipboard]` (shell.toml).
///
/// ```toml
/// [clipboard]
/// enabled = true          # izleyici; false: hiç okuma yok
/// max_items = 100         # 10..=500
/// images = true           # görsel öğeler (png, jpeg)
/// persist = false         # diske yazma
/// primary = false         # birincil seçim (seç-bırak / orta tık) de geçmişe girer
/// exclude_apps = ["org.keepassxc.KeePassXC", "KeePassXC", "Bitwarden", "1Password"]
/// x11_bridge = "auto"    # VM pano köprüsü: auto (VM konuk ajanı varken) | on | off
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ClipboardConfig {
    pub enabled: bool,
    pub max_items: u32,
    pub images: bool,
    pub persist: bool,
    /// Birincil seçim (`Selection::Primary`) de izlenir; varsayılan kapalı (seç-bırak çok sık
    /// değişir). Geri koyma her zaman panoya (`Selection::Clipboard`).
    pub primary: bool,
    /// Kopyalama anında etkin pencere sınıfı bunlardan biriyse teklif okunmaz (büyük/küçük
    /// harf duyarsız).
    pub exclude_apps: Vec<String>,
    /// VM pano köprüsü (Xwayland `CLIPBOARD` ↔ Wayland; `x11::policy::BridgeMode`).
    pub x11_bridge: x11::BridgeMode,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_items: 100,
            images: true,
            persist: false,
            primary: false,
            exclude_apps: ["org.keepassxc.KeePassXC", "KeePassXC", "Bitwarden", "1Password"].map(String::from).to_vec(),
            x11_bridge: x11::BridgeMode::Auto,
        }
    }
}

/// `exclude_apps` sınırları.
const MAX_EXCLUDE_APPS: usize = 64;
const MAX_CLASS_LEN: usize = 256;

impl ClipboardConfig {
    /// Ayarlar şeması. `exclude_apps` liste olduğu için `custom` türü (satır içi TOML dizisi;
    /// Ayarlar kendi bloğuyla yazar).
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(SettingSchema::bool("clipboard.enabled", t!("shell-config-clipboard-enabled-label"), d.enabled));
        out.push(SettingSchema::int(
            "clipboard.max_items",
            t!("shell-config-clipboard-max-items-label"),
            d.max_items.into(),
            history::MIN_ITEMS as i64,
            history::MAX_ITEMS as i64,
        ));
        out.push(SettingSchema::bool("clipboard.images", t!("shell-config-clipboard-images-label"), d.images));
        out.push(
            SettingSchema::bool("clipboard.persist", t!("shell-config-clipboard-persist-label"), d.persist)
                .describe(t!("shell-config-clipboard-persist-describe")),
        );
        out.push(
            SettingSchema::bool("clipboard.primary", t!("shell-config-clipboard-primary-label"), d.primary)
                .describe(t!("shell-config-clipboard-primary-describe")),
        );
        let apps = toml::Value::Array(d.exclude_apps.iter().map(|a| toml::Value::String(a.clone())).collect());
        out.push(
            SettingSchema::custom("clipboard.exclude_apps", t!("shell-config-clipboard-exclude-apps-label"), SettingValue::List(apps))
                .describe(t!("shell-config-clipboard-exclude-apps-describe")),
        );
        let modes = x11::BridgeMode::ALL;
        out.push(
            SettingSchema::choice(
                "clipboard.x11_bridge",
                t!("shell-config-clipboard-x11-bridge-label"),
                d.x11_bridge.as_str(),
                modes.iter().map(|m| m.as_str()).collect(),
            )
            .labels(
                modes
                    .iter()
                    .map(|m| match m {
                        x11::BridgeMode::Auto => t!("shell-config-clipboard-x11-bridge-auto"),
                        x11::BridgeMode::On => t!("shell-config-clipboard-x11-bridge-on"),
                        x11::BridgeMode::Off => t!("shell-config-clipboard-x11-bridge-off"),
                    })
                    .collect(),
            )
            .describe(t!("shell-config-clipboard-x11-bridge-describe")),
        );
    }

    /// Değerleri sınırlar; her düzeltme için uyarı (değerler uyarıya yazılmaz).
    pub fn validate(&mut self, w: &mut Vec<String>) {
        let range = history::MIN_ITEMS as u32..=history::MAX_ITEMS as u32;
        if !range.contains(&self.max_items) {
            self.max_items = self.max_items.clamp(*range.start(), *range.end());
            w.push(t!("shell-config-clipboard-max-items-clamped", value = self.max_items));
        }
        let before = self.exclude_apps.len();
        self.exclude_apps.retain(|a| !a.trim().is_empty() && a.len() <= MAX_CLASS_LEN && !a.chars().any(char::is_control));
        self.exclude_apps.truncate(MAX_EXCLUDE_APPS);
        if self.exclude_apps.len() != before {
            w.push(t!("shell-config-clipboard-exclude-apps-invalid"));
        }
    }

    fn limits(&self) -> history::Limits {
        history::Limits { max_items: self.max_items as usize, max_total: history::MAX_TOTAL_BYTES }
    }
}

/// UI ve IPC'nin gördüğü geçmiş (öğeler paylaşılır, kopya yok).
#[derive(Debug, Clone)]
pub struct ClipboardSnapshot {
    pub enabled: bool,
    /// Pano izleyicisi çalışıyor (framework protokolü var).
    pub available: bool,
    /// En yeni başta.
    pub items: Vec<Arc<Item>>,
}

impl Default for ClipboardSnapshot {
    fn default() -> Self {
        Self { enabled: true, available: false, items: Vec::new() }
    }
}

/// `msg clipboard list` biçim sürümü.
pub const LIST_VERSION: u32 = 1;

impl ClipboardSnapshot {
    pub fn get(&self, id: ItemId) -> Option<&Arc<Item>> {
        self.items.iter().find(|i| i.id == id)
    }

    /// `msg clipboard list` (tek satır JSON v1). Görselin baytı yazılmaz; önizleme ≤ 200 karakter.
    pub fn list_json(&self, now: SystemTime) -> String {
        let items: Vec<serde_json::Value> = self
            .items
            .iter()
            .map(|i| {
                let age = now.duration_since(i.copied_at).map_or(0, |d| d.as_secs());
                match &i.content {
                    history::Content::Text(_) => serde_json::json!({
                        "id": i.id, "kind": "text", "len": i.content.len(), "preview": i.preview(), "age_s": age,
                    }),
                    history::Content::Image { mime, .. } => serde_json::json!({
                        "id": i.id, "kind": "image", "mime": mime.as_str(), "len": i.content.len(), "age_s": age,
                    }),
                }
            })
            .collect();
        serde_json::json!({ "v": LIST_VERSION, "enabled": self.enabled, "available": self.available, "items": items }).to_string()
    }
}

/// Servisten ana döngüye.
#[derive(Debug, Clone)]
pub enum ClipboardEvent {
    Snapshot(Arc<ClipboardSnapshot>),
}

/// `msg clipboard …` alt komutları.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardCommand {
    List,
    Pick(ItemId),
    Delete(ItemId),
    Clear,
}

pub const USAGE: &str = "kullanım: clipboard list | clipboard pick <id> | clipboard delete <id> | clipboard clear"; // i18n: tanı msg yanıtı/bildirim ayrıntısı

/// `clipboard` sonrası argümanlar. Kimlik: yalnızca ondalık rakam, baştaki sıfır yok.
pub fn parse_command(args: &[&str]) -> Result<ClipboardCommand, String> {
    let id = |w: &str| -> Result<ItemId, String> {
        let ok = !w.is_empty() && w.len() <= 20 && w.bytes().all(|b| b.is_ascii_digit()) && !w.starts_with('0');
        ok.then(|| w.parse::<ItemId>().ok()).flatten().ok_or_else(|| format!("invalid: geçersiz öğe kimliği; {USAGE}")) // i18n: tanı msg yanıtı/bildirim ayrıntısı
    };
    Ok(match args {
        ["list"] => ClipboardCommand::List,
        ["pick", w] => ClipboardCommand::Pick(id(w)?),
        ["delete", w] => ClipboardCommand::Delete(id(w)?),
        ["clear"] => ClipboardCommand::Clear,
        _ => return Err(USAGE.into()),
    })
}

enum Input {
    Source(SourceEvent),
    Pick(ItemId),
    Delete(ItemId),
    Clear,
    Config(ClipboardConfig),
    /// Kaynağın izleme durumu değişti (değer paylaşılan bayrakta; kuyruk doluysa sonraki
    /// grupta yayınlanır).
    Available,
    Stop,
}

/// Kaynağın olay ucu (adaptör tutar). Kuyruk doluysa olay atılır (bloklamaz).
#[derive(Clone)]
pub struct SourceSink {
    tx: SyncSender<Input>,
    available: Arc<AtomicBool>,
}

impl std::fmt::Debug for SourceSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SourceSink")
    }
}

impl SourceSink {
    pub fn send(&self, event: SourceEvent) {
        match self.tx.try_send(Input::Source(event)) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => tracing::debug!("pano kuyruğu dolu; teklif atıldı"),
        }
    }

    /// Kaynağın izleme durumu (başladı / durdu). Değer paylaşılan bayrağa yazılır, servis
    /// dürtülür; kuyruk doluysa dürtü atılır ama durum sonraki grupta yine yayınlanır.
    pub fn set_available(&self, on: bool) {
        if self.available.swap(on, Ordering::Relaxed) != on {
            let _ = self.tx.try_send(Input::Available);
        }
    }
}

/// Servisin gelen kutusu: [`channel`] ile kurulur, [`ClipboardService::spawn`]'a verilir.
pub struct Inbox {
    rx: Receiver<Input>,
    tx: SyncSender<Input>,
    available: Arc<AtomicBool>,
}

impl std::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Inbox")
    }
}

/// Kaynak ve servis arasındaki kuyruk: önce kurulur (adaptör olay ucunu alır), sonra servis.
pub fn channel() -> (SourceSink, Inbox) {
    let (tx, rx) = mpsc::sync_channel(QUEUE);
    let available = Arc::new(AtomicBool::new(false));
    (SourceSink { tx: tx.clone(), available: available.clone() }, Inbox { rx, tx, available })
}

/// Servis ayarları.
#[derive(Debug, Clone)]
pub struct ServiceOptions {
    pub config: ClipboardConfig,
    /// Disk geçmişinin yolu; `None`: hiç yazılmaz (yazım kapalı ortam).
    pub persist_path: Option<PathBuf>,
}

/// Çalışan servis. Düşürülünce thread durur (bekleyen yazım yapılır) ve join edilir.
#[derive(Debug)]
pub struct ClipboardService {
    tx: SyncSender<Input>,
    thread: Option<JoinHandle<()>>,
}

impl ClipboardService {
    pub fn spawn<F>(inbox: Inbox, source: Arc<dyn ClipboardSource>, gate: Arc<Gate>, options: ServiceOptions, sink: F) -> std::io::Result<Self>
    where
        F: Fn(ClipboardEvent) + Send + 'static,
    {
        let Inbox { rx, tx, available } = inbox;
        let worker = Worker::new(source, gate, available, options, Box::new(sink));
        let thread = std::thread::Builder::new().name("nuxut-clipboard".into()).spawn(move || worker.run(&rx))?;
        Ok(Self { tx, thread: Some(thread) })
    }

    pub fn pick(&self, id: ItemId) {
        self.send(Input::Pick(id));
    }

    pub fn delete(&self, id: ItemId) {
        self.send(Input::Delete(id));
    }

    pub fn clear(&self) {
        self.send(Input::Clear);
    }

    pub fn set_config(&self, config: ClipboardConfig) {
        self.send(Input::Config(config));
    }

    fn send(&self, input: Input) {
        match self.tx.try_send(input) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => tracing::warn!("pano kuyruğu dolu; istek atıldı"),
            Err(TrySendError::Disconnected(_)) => tracing::warn!("pano thread'i kapalı"),
        }
    }
}

impl Drop for ClipboardService {
    fn drop(&mut self) {
        // Kuyruk doluysa bile durdurma gider (thread en geç bir okuma süresi sonra boşaltır).
        let _ = self.tx.send(Input::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

type Sink = Box<dyn Fn(ClipboardEvent) + Send>;

/// Servis thread'inin durumu (testte thread'siz sürülür).
struct Worker {
    source: Arc<dyn ClipboardSource>,
    gate: Arc<Gate>,
    config: ClipboardConfig,
    history: history::History,
    persist_path: Option<PathBuf>,
    /// Bekleyen disk yazımının zamanı.
    dirty_at: Option<Instant>,
    /// Kaynak izliyor (adaptör yazar; [`SourceSink::set_available`]).
    available: Arc<AtomicBool>,
    /// Son yayınlanan `available` (değişince yeniden yayın).
    published_available: bool,
    /// Disk hatası bir kez uyarılır.
    warned: bool,
    sink: Sink,
}

impl Worker {
    fn new(source: Arc<dyn ClipboardSource>, gate: Arc<Gate>, available: Arc<AtomicBool>, options: ServiceOptions, sink: Sink) -> Self {
        let history = history::History::new(options.config.limits());
        Self {
            source,
            gate,
            config: options.config,
            history,
            persist_path: options.persist_path,
            dirty_at: None,
            available,
            published_available: false,
            warned: false,
            sink,
        }
    }

    fn run(mut self, rx: &Receiver<Input>) {
        self.load();
        self.publish();
        loop {
            let first = match self.dirty_at {
                Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                    Ok(i) => i,
                    Err(RecvTimeoutError::Timeout) => {
                        self.flush();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match rx.recv() {
                    Ok(i) => i,
                    Err(_) => break,
                },
            };
            let mut batch = vec![first];
            while batch.len() < QUEUE * 2 {
                match rx.try_recv() {
                    Ok(i) => batch.push(i),
                    Err(_) => break,
                }
            }
            if !self.handle(batch) {
                break;
            }
        }
        self.flush();
    }

    /// Bir grup girdiyi işler; değiştiyse yayınlar. Durdurma geldiyse `false`.
    fn handle(&mut self, batch: Vec<Input>) -> bool {
        // Yalnızca son kaynak olayı geçerli: daha yeni teklif varken eskisi okunmaz.
        let last_source = batch.iter().rposition(|i| matches!(i, Input::Source(_)));
        let mut changed = false;
        let mut running = true;
        for (i, input) in batch.into_iter().enumerate() {
            match input {
                Input::Source(e) if Some(i) == last_source => changed |= self.on_source(e),
                Input::Source(_) => {}
                Input::Pick(id) => changed |= self.pick(id),
                Input::Delete(id) => {
                    if self.history.remove(id) {
                        self.mark_dirty();
                        changed = true;
                    }
                }
                Input::Clear => {
                    self.clear();
                    changed = true;
                }
                Input::Config(c) => {
                    self.set_config(c);
                    changed = true;
                }
                // Değer aşağıda bayraktan okunur (dürtü atılmış olsa da).
                Input::Available => {}
                Input::Stop => {
                    running = false;
                    break;
                }
            }
        }
        changed |= self.available.load(Ordering::Relaxed) != self.published_available;
        if changed {
            self.publish();
        }
        running
    }

    fn on_source(&mut self, event: SourceEvent) -> bool {
        let SourceEvent::Offered { id, mimes, mut flags } = event else { return false };
        // VM pano köprüsünün X11'den getirdiği seçim bizimdir ama kopyalama başka yerde (ana
        // makine/X11 uygulaması) yapıldı: geçmişe girer (aynı gizlilik kurallarıyla).
        if flags.own && mimes.iter().any(|m| m == x11::policy::BRIDGE_MIME) {
            flags.own = false;
        }
        let gate = self.gate.get();
        let cx = filter::Context {
            enabled: self.config.enabled,
            images: self.config.images,
            locked: gate.locked,
            active_class: gate.active_class.as_deref(),
            exclude_apps: &self.config.exclude_apps,
        };
        let (mime, kind) = match filter::decide(&mimes, flags, &cx) {
            filter::Decision::Accept { mime, kind } => (mime, kind),
            filter::Decision::Skip(reason) => {
                tracing::debug!(?reason, "pano teklifi okunmadı");
                return false;
            }
        };
        let max = match kind {
            filter::Kind::Text => history::MAX_TEXT_BYTES,
            filter::Kind::Image(_) => history::MAX_IMAGE_BYTES,
        };
        let bytes = match self.source.read(id, &mime, max, READ_TIMEOUT) {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!(error = %e, "pano teklifi okunamadı");
                return false;
            }
        };
        // Okuma sürerken oturum kilitlendiyse saklanmaz.
        if self.gate.get().locked {
            return false;
        }
        let content = match kind {
            filter::Kind::Text => history::Content::text(bytes),
            filter::Kind::Image(m) => history::Content::image(m, bytes),
        };
        match content {
            Ok(c) => {
                self.history.push(c, SystemTime::now());
                self.mark_dirty();
                true
            }
            Err(reason) => {
                tracing::debug!(?reason, "pano içeriği saklanmadı");
                false
            }
        }
    }

    fn pick(&mut self, id: ItemId) -> bool {
        let Some(item) = self.history.get(id) else { return false };
        match self.source.set_selection(item.content.offer()) {
            Ok(()) => {
                self.history.touch(id, SystemTime::now());
                self.mark_dirty();
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "pano öğesi panoya konamadı");
                false
            }
        }
    }

    fn clear(&mut self) {
        self.history.clear();
        self.dirty_at = None;
        if let Some(p) = &self.persist_path {
            store::remove(p);
        }
    }

    fn set_config(&mut self, config: ClipboardConfig) {
        let old = std::mem::replace(&mut self.config, config);
        self.history.set_limits(self.config.limits());
        if old.enabled && !self.config.enabled {
            // Kapatılınca bellekteki geçmiş bırakılır (disk dosyası `persist`'e bağlı kalır).
            self.history.clear();
            self.dirty_at = None;
        } else if !old.enabled && self.config.enabled {
            self.load();
        }
        if old.persist && !self.config.persist {
            self.dirty_at = None;
            if let Some(p) = &self.persist_path {
                store::remove(p);
            }
        } else if (!old.persist && self.config.persist && self.config.enabled) || old.max_items != self.config.max_items {
            self.mark_dirty();
        }
    }

    fn persisting(&self) -> Option<&PathBuf> {
        self.persist_path.as_ref().filter(|_| self.config.persist && self.config.enabled)
    }

    fn mark_dirty(&mut self) {
        if self.persisting().is_some() && self.dirty_at.is_none() {
            self.dirty_at = Some(Instant::now() + PERSIST_DELAY);
        }
    }

    fn load(&mut self) {
        let Some(path) = self.persisting().cloned() else { return };
        match store::load(&path) {
            Ok(items) => {
                for (content, at) in items {
                    self.history.push_loaded(content, at);
                }
            }
            Err(e) => tracing::warn!("pano geçmişi dosyası yok sayıldı: {e}"),
        }
    }

    fn flush(&mut self) {
        if self.dirty_at.take().is_none() {
            return;
        }
        let Some(path) = self.persisting().cloned() else { return };
        let bytes = store::encode(self.history.iter().map(|i| (&i.content, i.copied_at)));
        if let Err(e) = store::save(&path, &bytes)
            && !std::mem::replace(&mut self.warned, true)
        {
            tracing::warn!("pano geçmişi diske yazılamadı: {e}");
        }
    }

    fn publish(&mut self) {
        self.published_available = self.available.load(Ordering::Relaxed);
        let snapshot = ClipboardSnapshot { enabled: self.config.enabled, available: self.published_available, items: self.history.snapshot() };
        (self.sink)(ClipboardEvent::Snapshot(Arc::new(snapshot)));
    }
}

#[cfg(test)]
mod tests;
