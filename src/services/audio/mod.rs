//! Ses: PipeWire (doğrudan `libpipewire`, pipewire-rs) — çıkış/giriş cihazları, varsayılan
//! cihaz, seviye ve sessiz, uygulama akışları, OSD olayı.
//!
//! **Durum.** Registry'den yalnızca ses düğümleri (`Audio/Sink`, `Audio/Source`,
//! `Stream/Output/Audio`, `Stream/Input/Audio`), ses cihazları (`Audio/Device`) ve `default`
//! metadata'sı bağlanır. Düğümlerin `Props` parametresine abone olunur. Cihazların `Route`
//! listesi ise **her cihaz bilgisi değişiminde yeniden numaralandırılır** ve bir sunucu
//! gidiş-dönüşüyle (`core.sync`) bütün olarak değiştirilir (pipewire-pulse ve WirePlumber de
//! böyle yapar): WirePlumber'ın dışa aktardığı cihazlarda (`SpaDevice`; tüm ALSA/BlueZ kartları)
//! sunucu, abonelere param değişimini iletemez — `pw_impl_device_for_each_param` yanıtı eşzamansız
//! gelir ve geçici dinleyici o anda kaldırılmıştır (`impl-device.c`). Yalnızca abonelik, route
//! listesini ilk okumada dondurur: sonradan gelen route'lar görülmez (kart düğümü Props'a düşer,
//! yalnızca yazılım seviyesi değişir) ve port değişiminden sonra eski route index'ine yazılır
//! (ACP portu geri çevirir).
//!
//! **Okuma ve yazma** WirePlumber mixer-api / pipewire-pulse ile aynı anlamdadır (bkz.
//! [`model`]): kart düğümünde seviye cihazın etkin route'undan okunur ve oraya yazılır (`props
//! {channelVolumes, mute}`, `save = true`; donanım/yazılım bölüşümünü ACP yapar, WirePlumber
//! saklar); diğerlerinde düğümün `Props`'u. Route listesi henüz okunmamış kart düğümüne yazım,
//! Props'a düşmek yerine bekletilir. Seviyeler algısal (kübik) ölçektedir: 1.0 = %100 = 0 dB;
//! kanal dengesi korunur.
//!
//! **Yazım yolu** (`[audio] backend`, geçici): `wpctl` (varsayılan; alt süreç) ya da `native`
//! (süreç içi). Native yolda düğüm başına en çok bir yazım yoldadır; kaydırma patlamasında
//! yoldayken gelenler tek bekleyen yazımda birleşir (bkz. [`writes`]). `wpctl` yolunda süreçler
//! tek thread'de sırayla çalışır; bekleyen aynı tür yazım en yenisiyle değişir (bkz. [`wpctl`]).
//!
//! **OSD.** Varsayılan çıkışın/girişin seviyesi ya da sessizliği değişince (kimden gelirse
//! gelsin) [`AudioEvent::Osd`]. Başlangıç okuması ve varsayılan cihaz değişimi OSD değildir.
//!
//! PipeWire düşerse durum "kullanılamıyor" olur, üstel beklemeyle yeniden bağlanılır.
//! Cihaz/akış/medya adları loglanmaz.

mod diagnose;
pub use diagnose::{AudioSummary, PathKind};
mod model;
mod wpctl;
mod writes;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::rc::{Rc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nuxut::i18n::t;
use pipewire as pw;
use pw::device::{Device, DeviceChangeMask, DeviceListener};
use pw::metadata::{Metadata, MetadataListener};
use pw::node::{Node, NodeListener};
use pw::proxy::ProxyT;
use pw::spa::param::{ParamInfoFlags, ParamType};
use pw::spa::pod::deserialize::PodDeserializer;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Value};
use pw::spa::utils::dict::DictRef;
use pw::spa::utils::result::AsyncSeq;
use pw::types::ObjectType;
use serde::Deserialize;

pub use model::{AudioDevice, AudioState, AudioStream, DeviceClass, MAX_CHANNELS, MAX_DEVICES, MAX_NODES, to_linear, to_perceptual};

use nuxut_dbus::Backoff;
use model::{
    KEY_CONFIGURED_SINK, KEY_CONFIGURED_SOURCE, KEY_DEFAULT_SINK, KEY_DEFAULT_SOURCE, Model, NodeEntry, NodeInfo,
    NodeKind, Route, VolumePath, VolumeProps, VolumeWrite, default_value, parse_default_name, scale_channels,
};

/// Kabul edilen en büyük parametre (pod) boyutu.
const MAX_POD: usize = 64 * 1024;

/// Seviye/sessiz yazım yolu (`[audio] backend`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioBackend {
    /// `wpctl set-volume/set-mute` alt süreci (WirePlumber mixer-api).
    #[default]
    Wpctl,
    /// Süreç içi: route/Props'a doğrudan, mixer-api ile aynı anlamda.
    Native,
}

impl AudioBackend {
    pub const ALL: [AudioBackend; 2] = [AudioBackend::Wpctl, AudioBackend::Native];

    /// `shell.toml`'daki adı (serde adıyla aynı; şema testi doğrular).
    pub fn name(self) -> &'static str {
        match self {
            AudioBackend::Wpctl => "wpctl",
            AudioBackend::Native => "native",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AudioOptions {
    /// PipeWire uzak adı ya da soket yolu; `None`: varsayılan (`PIPEWIRE_REMOTE` / `pipewire-0`).
    pub remote: Option<String>,
    /// Ayarlanabilen en yüksek algısal seviye (1.0 = %100; yükseltme için ör. 1.5).
    pub max_volume: f32,
    pub backend: AudioBackend,
    pub coalesce: Duration,
    pub max_backoff: Duration,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            remote: None,
            max_volume: 1.0,
            backend: AudioBackend::default(),
            coalesce: Duration::from_millis(25),
            max_backoff: Duration::from_secs(30),
        }
    }
}

impl AudioOptions {
    /// `[audio] max_volume` yüzdesi (100..=150, sınırlanır) → algısal üst sınır (1.0..=1.5).
    pub fn max_volume_from_percent(percent: u32) -> f32 {
        percent.clamp(100, 150) as f32 / 100.0
    }
}

/// Komutun hedefi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTarget {
    DefaultSink,
    DefaultSource,
    /// [`AudioDevice::id`] ya da [`AudioStream::id`].
    Node(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsdKind {
    Output,
    Input,
}

/// Varsayılan cihazın seviyesi/sessizliği değişti.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Osd {
    pub kind: OsdKind,
    pub volume: f32,
    pub muted: bool,
}

#[derive(Clone, PartialEq)]
pub enum AudioEvent {
    /// Durum (tam anlık görüntü).
    State(AudioState),
    Osd(Osd),
    /// Komut uygulanamadı.
    Failed { action: &'static str, message: String },
}

/// Redakte: cihaz/uygulama/medya adı yazılmaz (log politikası).
impl std::fmt::Debug for AudioEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(s) => f
                .debug_struct("State")
                .field("available", &s.available)
                .field("sinks", &s.sinks.len())
                .field("sources", &s.sources.len())
                .field("playback", &s.playback.len())
                .field("recording", &s.recording.len())
                .finish(),
            Self::Osd(o) => f.debug_tuple("Osd").field(o).finish(),
            Self::Failed { action, message } => {
                f.debug_struct("Failed").field("action", action).field("message", message).finish()
            }
        }
    }
}

/// Tanı raporunu alan geri çağrı (ses thread'inden ayrı bir thread'de çağrılır).
pub type DiagnoseReply = Box<dyn FnOnce(String) + Send>;

/// `msg doctor` ses özetini alan geri çağrı (ses thread'inde çağrılır; hızlı olmalı).
pub type SummaryReply = Box<dyn FnOnce(AudioSummary) + Send>;

enum Command {
    Stop,
    SetVolume(AudioTarget, f32),
    AdjustVolume(AudioTarget, f32),
    SetMuted(AudioTarget, bool),
    ToggleMuted(AudioTarget),
    SetDefault(u32),
    Diagnose(DiagnoseReply),
    Summary(SummaryReply),
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stop => f.write_str("Stop"),
            Self::SetVolume(t, v) => f.debug_tuple("SetVolume").field(t).field(v).finish(),
            Self::AdjustVolume(t, v) => f.debug_tuple("AdjustVolume").field(t).field(v).finish(),
            Self::SetMuted(t, m) => f.debug_tuple("SetMuted").field(t).field(m).finish(),
            Self::ToggleMuted(t) => f.debug_tuple("ToggleMuted").field(t).finish(),
            Self::SetDefault(id) => f.debug_tuple("SetDefault").field(id).finish(),
            Self::Diagnose(_) => f.write_str("Diagnose"),
            Self::Summary(_) => f.write_str("Summary"),
        }
    }
}

/// Çalışan ses servisi. Düşürülünce PipeWire bağlantısı kapanır, thread join edilir.
pub struct AudioService {
    tx: pw::channel::Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for AudioService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioService").finish_non_exhaustive()
    }
}

impl AudioService {
    pub fn spawn<F>(options: AudioOptions, sink: F) -> io::Result<Self>
    where
        F: Fn(AudioEvent) + Send + 'static,
    {
        let (tx, rx) = pw::channel::channel();
        let thread = std::thread::Builder::new()
            .name("nuxut-audio".into())
            .spawn(move || run(options, Box::new(sink), rx))?;
        Ok(Self { tx, thread: Some(thread) })
    }

    fn send(&self, c: Command) -> bool {
        self.tx.send(c).is_ok()
    }

    /// Algısal seviyeyi ayarlar (0..=[`AudioOptions::max_volume`]).
    pub fn set_volume(&self, target: AudioTarget, volume: f32) -> bool {
        self.send(Command::SetVolume(target, volume))
    }

    /// Göreli değişim (ör. +0.05 / -0.05); OSD tuşları ve tekerlek için.
    pub fn adjust_volume(&self, target: AudioTarget, delta: f32) -> bool {
        self.send(Command::AdjustVolume(target, delta))
    }

    pub fn set_muted(&self, target: AudioTarget, muted: bool) -> bool {
        self.send(Command::SetMuted(target, muted))
    }

    pub fn toggle_muted(&self, target: AudioTarget) -> bool {
        self.send(Command::ToggleMuted(target))
    }

    /// Cihazı varsayılan yapar ([`AudioDevice::id`]; sink ya da source).
    pub fn set_default(&self, device: u32) -> bool {
        self.send(Command::SetDefault(device))
    }

    /// Tanı raporu (`msg audio-diagnose`). Rapor `reply`'a ~1 sn içinde verilir.
    pub fn diagnose(&self, reply: DiagnoseReply) -> bool {
        self.send(Command::Diagnose(reply))
    }

    /// `msg doctor` ses özeti (seviye yolları, ad yok). `reply` ses thread'inde çağrılır.
    pub fn summary(&self, reply: SummaryReply) -> bool {
        self.send(Command::Summary(reply))
    }
}

impl Drop for AudioService {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Stop);
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("ses thread'i panikle sonlandı");
        }
    }
}

// ---------------------------------------------------------------------------------------
// Thread

type Sink = Box<dyn Fn(AudioEvent) + Send>;

/// Bağlanan PipeWire nesnesi; dinleyici düşürülünce olaylar kesilir.
enum Bound {
    Node(#[allow(dead_code)] NodeListener, Node),
    Device(#[allow(dead_code)] DeviceListener, Device),
    Metadata(#[allow(dead_code)] MetadataListener, Metadata),
}

/// Sunucu gidiş-dönüşü (`core.sync`) bitince yapılacak iş.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Barrier {
    /// Cihazın route numaralandırması tamamlandı.
    Routes { device: u32, seq: i32 },
    /// Düğüme yazım sunucuda işlendi.
    Write { node: u32, token: u64 },
}

struct Inner {
    options: AudioOptions,
    sink: Sink,
    model: Model,
    bound: HashMap<u32, Bound>,
    /// Vekil (yerel) id → global id (çekirdek hatalarını nesneye bağlamak için).
    proxies: HashMap<u32, u32>,
    /// `default` metadata'sının global id'si.
    metadata: Option<u32>,
    core: Option<pw::core::CoreRc>,
    barriers: Vec<(AsyncSeq, Barrier)>,
    /// Numaralandırma sıra no'su.
    next_seq: i32,
    available: bool,
    stop: bool,
    /// Çekirdek bağlantısı koptu (thread döngüsü oturumu kapatır).
    lost: bool,
    dirty: Option<Instant>,
    emitted: Option<AudioState>,
    /// Son görülen varsayılan çıkış/giriş: (id, seviye, sessiz).
    last_osd: [Option<(u32, f32, bool)>; 2],
    backoff: Backoff,
    /// `wpctl` yolu seçiliyse ve wpctl varsa sıralı yazıcı.
    wpctl: Option<wpctl::Writer>,
}

/// Bir oturumun çekirdek nesneleri (düşürülünce bağlantı kapanır).
struct Session {
    _registry_listener: pw::registry::Listener,
    _core_listener: pw::core::Listener,
    _registry: pw::registry::RegistryRc,
    _core: pw::core::CoreRc,
}

fn run(options: AudioOptions, sink: Sink, rx: pw::channel::Receiver<Command>) {
    pw::init();
    let main_loop = match pw::main_loop::MainLoopRc::new(None) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("ses: PipeWire döngüsü kurulamadı: {e}");
            sink(AudioEvent::State(AudioState::default()));
            return;
        }
    };
    let context = match pw::context::ContextRc::new(&main_loop, None) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("ses: PipeWire bağlamı kurulamadı: {e}");
            sink(AudioEvent::State(AudioState::default()));
            return;
        }
    };
    let backoff = Backoff::new(options.max_backoff);
    let wpctl = if options.backend == AudioBackend::Wpctl { wpctl::Writer::spawn(options.remote.clone()) } else { None };
    let inner = Rc::new(RefCell::new(Inner {
        options,
        sink,
        model: Model::default(),
        bound: HashMap::new(),
        proxies: HashMap::new(),
        metadata: None,
        core: None,
        barriers: Vec::new(),
        next_seq: 0,
        available: false,
        stop: false,
        lost: false,
        dirty: None,
        emitted: None,
        last_osd: [None, None],
        backoff,
        wpctl,
    }));
    let _commands = rx.attach(main_loop.loop_(), {
        let weak = Rc::downgrade(&inner);
        move |c| with(&weak, |s| s.command(c))
    });

    let mut session: Option<Session> = None;
    let mut retry_at = Some(Instant::now());
    loop {
        let now = Instant::now();
        let lost = {
            let mut s = inner.borrow_mut();
            if s.stop {
                break;
            }
            std::mem::take(&mut s.lost)
        };
        if lost {
            close_session(&inner, &mut session);
            let delay = inner.borrow_mut().backoff.next_delay();
            tracing::warn!("ses: PipeWire bağlantısı koptu, yeniden bağlanılacak");
            retry_at = Some(now + delay);
        }
        if retry_at.is_some_and(|at| at <= now) {
            match connect(&inner, &context) {
                Ok(s) => {
                    session = Some(s);
                    retry_at = None;
                }
                Err(e) => {
                    let delay = {
                        let mut s = inner.borrow_mut();
                        s.mark_dirty();
                        s.backoff.next_delay()
                    };
                    tracing::debug!("ses: PipeWire'a bağlanılamadı: {e}");
                    retry_at = Some(now + delay);
                }
            }
        }
        let next = {
            let mut s = inner.borrow_mut();
            s.tick(now);
            if s.dirty.is_some_and(|at| at <= now) {
                s.flush();
            }
            [s.dirty, s.model.writes.next_deadline()].into_iter().flatten().min()
        };
        let next = [next, retry_at].into_iter().flatten().min();
        let timeout = match next {
            Some(at) => pw::loop_::Timeout::Finite(
                at.saturating_duration_since(Instant::now()).min(Duration::from_secs(3600)),
            ),
            None => pw::loop_::Timeout::Infinite,
        };
        main_loop.loop_().iterate(timeout);
    }
    close_session(&inner, &mut session);
}

/// Bağlı nesneleri ve oturumu kapatır; modeli boşaltır.
fn close_session(inner: &Rc<RefCell<Inner>>, session: &mut Option<Session>) {
    let (bound, core) = {
        let mut s = inner.borrow_mut();
        s.model.clear();
        s.metadata = None;
        s.available = false;
        s.barriers.clear();
        s.proxies.clear();
        s.mark_dirty();
        (std::mem::take(&mut s.bound), s.core.take())
    };
    // Vekiller çekirdekten önce düşürülür.
    drop(bound);
    drop(core);
    *session = None;
}

/// Zayıf referansla `Inner`'a erişir; iç içe çağrıda (olmamalı) olayı atlar, panik etmez.
fn with(weak: &Weak<RefCell<Inner>>, f: impl FnOnce(&mut Inner)) {
    let Some(rc) = weak.upgrade() else { return };
    match rc.try_borrow_mut() {
        Ok(mut s) => f(&mut s),
        Err(_) => tracing::debug!("ses: iç içe olay atlandı"),
    };
}

fn connect(inner: &Rc<RefCell<Inner>>, context: &pw::context::ContextRc) -> Result<Session, pw::Error> {
    let props = inner.borrow().options.remote.as_ref().map(|r| pw::properties::properties! { *pw::keys::REMOTE_NAME => r.as_str() });
    let core = context.connect_rc(props)?;
    let registry = core.get_registry_rc()?;
    let weak = Rc::downgrade(inner);
    let core_listener = core
        .add_listener_local()
        .info({
            let weak = weak.clone();
            move |_| {
                with(&weak, |s| {
                    if !s.available {
                        s.available = true;
                        s.backoff.reset();
                        s.mark_dirty();
                    }
                })
            }
        })
        .done({
            let weak = weak.clone();
            move |id, seq| {
                if id == pw::core::PW_ID_CORE {
                    with(&weak, |s| s.sync_done(seq));
                }
            }
        })
        .error({
            let weak = weak.clone();
            move |id, _seq, res, _message| {
                // Çekirdek (id 0) hatası: bağlantı gitti (EPIPE) ya da kullanılamaz. Diğerleri bir
                // vekile (düğüm/cihaz) yapılan isteğin reddi.
                if id == pw::core::PW_ID_CORE {
                    tracing::debug!(res, "ses: çekirdek hatası");
                    with(&weak, |s| s.lost = true);
                } else {
                    with(&weak, |s| s.proxy_error(id, res));
                }
            }
        })
        .register();
    let registry_listener = registry
        .add_listener_local()
        .global({
            let weak = weak.clone();
            let registry = registry.downgrade();
            move |g| {
                let Some(registry) = registry.upgrade() else { return };
                with(&weak, |s| s.global(&weak, &registry, g));
            }
        })
        .global_remove({
            let weak = weak.clone();
            move |id| {
                let mut removed = None;
                with(&weak, |s| removed = s.global_remove(id));
                drop(removed);
            }
        })
        .register();
    inner.borrow_mut().core = Some(core.clone());
    Ok(Session { _registry_listener: registry_listener, _core_listener: core_listener, _registry: registry, _core: core })
}

fn parse_pod(pod: Option<&Pod>) -> Option<Value> {
    let bytes = pod?.as_bytes();
    if bytes.len() > MAX_POD {
        return None;
    }
    PodDeserializer::deserialize_any_from(bytes).ok().map(|(_, v)| v)
}

fn serialize(object: Object) -> Option<Vec<u8>> {
    PodSerializer::serialize(io::Cursor::new(Vec::new()), &Value::Object(object)).ok().map(|(c, _)| c.into_inner())
}

fn node_info(dict: &DictRef) -> NodeInfo {
    NodeInfo::parse(|k| dict.get(k))
}

impl Inner {
    fn mark_dirty(&mut self) {
        if self.dirty.is_none() {
            self.dirty = Some(Instant::now() + self.options.coalesce);
        }
    }

    fn fail(&self, action: &'static str, message: &str) {
        tracing::debug!(action, "ses komutu uygulanamadı: {message}");
        (self.sink)(AudioEvent::Failed { action, message: message.to_owned() });
    }

    /// Zaman sınırları: teyidi gelmeyen hedefler bırakılır, route'u gelmeyen bekletilmiş
    /// yazımlar son çare olarak düğüm Props'una gider (mixer-api'nin route bulamayınca yaptığı).
    fn tick(&mut self, now: Instant) {
        for (id, w) in self.model.writes.parked_expired(now) {
            tracing::debug!(node = id, "ses: route listesi gelmedi, yazım düğüm Props'una");
            self.dispatch(id, w, false);
        }
        if self.model.reconcile(now) {
            self.mark_dirty();
        }
    }

    fn flush(&mut self) {
        self.dirty = None;
        self.model.reconcile(Instant::now());
        let state = self.model.state(self.available);
        // OSD: yalnızca aynı varsayılan cihazın seviyesi/sessizliği değişince.
        let mut osd = Vec::new();
        for (i, kind) in [NodeKind::Sink, NodeKind::Source].into_iter().enumerate() {
            let now = self
                .model
                .default_node(kind)
                .and_then(|id| self.model.volume(id).map(|(v, m)| (id, v, m)));
            if let (Some(prev), Some(cur)) = (self.last_osd[i], now)
                && prev.0 == cur.0
                && ((prev.1 - cur.1).abs() > 1e-4 || prev.2 != cur.2)
            {
                let kind = if kind == NodeKind::Sink { OsdKind::Output } else { OsdKind::Input };
                osd.push(Osd { kind, volume: cur.1, muted: cur.2 });
            }
            self.last_osd[i] = now;
        }
        if self.emitted.as_ref() != Some(&state) {
            self.emitted = Some(state.clone());
            (self.sink)(AudioEvent::State(state));
        }
        for o in osd {
            (self.sink)(AudioEvent::Osd(o));
        }
    }

    fn global(&mut self, weak: &Weak<RefCell<Inner>>, registry: &pw::registry::RegistryRc, g: &pw::registry::GlobalObject<&DictRef>) {
        let id = g.id;
        let prop = |k: &str| g.props.and_then(|p| p.get(k));
        match g.type_ {
            ObjectType::Node => {
                let Some(kind) = prop("media.class").and_then(NodeKind::from_class) else { return };
                if self.model.nodes.len() >= model::MAX_NODES {
                    tracing::debug!("ses: düğüm sınırı aşıldı, {id} atlandı");
                    return;
                }
                let node: Node = match registry.bind(g) {
                    Ok(n) => n,
                    Err(e) => return tracing::debug!("ses: düğüm {id} bağlanamadı: {e}"),
                };
                let listener = node
                    .add_listener_local()
                    .info({
                        let weak = weak.clone();
                        move |info| {
                            if info.change_mask().contains(pw::node::NodeChangeMask::STATE) {
                                // Ham alan: `NodeInfoRef::state()` hata metnini UTF-8 varsayıp panikler.
                                let running = info.as_raw().state == pw::sys::pw_node_state_PW_NODE_STATE_RUNNING;
                                with(&weak, |s| {
                                    if s.model.set_running(id, running) {
                                        s.mark_dirty();
                                    }
                                });
                            }
                            // Sözlük yalnızca PROPS değiştiyse dolu gelir.
                            if !info.change_mask().contains(pw::node::NodeChangeMask::PROPS) {
                                return;
                            }
                            let Some(dict) = info.props() else { return };
                            let info = node_info(dict);
                            with(&weak, |s| {
                                if s.model.set_info(id, info) {
                                    s.mark_dirty();
                                }
                            })
                        }
                    })
                    .param({
                        let weak = weak.clone();
                        move |_seq, kind, _index, _next, pod| {
                            if kind != ParamType::Props {
                                return;
                            }
                            let Some(props) = parse_pod(pod).as_ref().and_then(VolumeProps::parse) else { return };
                            with(&weak, |s| {
                                if s.model.set_props(id, props) {
                                    s.mark_dirty();
                                }
                            })
                        }
                    })
                    .register();
                // Düğümler sunucu içi (adapter) ya da client-node: param bildirimi güvenilir.
                node.subscribe_params(&[ParamType::Props]);
                let info = g.props.map(node_info).unwrap_or_default();
                self.model.add_node(id, NodeEntry::new(kind, info));
                self.proxies.insert(node.upcast_ref().id(), id);
                self.bound.insert(id, Bound::Node(listener, node));
                self.mark_dirty();
            }
            ObjectType::Device if prop("media.class") == Some("Audio/Device") => {
                if !self.model.add_device(id) {
                    tracing::debug!("ses: cihaz sınırı aşıldı, {id} atlandı");
                    return;
                }
                let device: Device = match registry.bind(g) {
                    Ok(d) => d,
                    Err(e) => {
                        self.model.remove(id);
                        return tracing::debug!("ses: cihaz {id} bağlanamadı: {e}");
                    }
                };
                let listener = device
                    .add_listener_local()
                    .info({
                        let weak = weak.clone();
                        move |info| {
                            // Bağlanınca tam bilgi gelir; sonra route/profil her değiştiğinde
                            // PARAMS bitiyle. Abonelik yerine burada yeniden numaralandırılır.
                            if !info.change_mask().contains(DeviceChangeMask::PARAMS) {
                                return;
                            }
                            let readable = info
                                .params()
                                .iter()
                                .any(|p| p.id() == ParamType::Route && p.flags().contains(ParamInfoFlags::READ));
                            with(&weak, |s| s.request_routes(id, readable));
                        }
                    })
                    .param({
                        let weak = weak.clone();
                        move |seq, kind, _index, _next, pod| {
                            if kind != ParamType::Route {
                                return;
                            }
                            let Some(route) = parse_pod(pod).as_ref().and_then(Route::parse) else { return };
                            with(&weak, |s| {
                                s.model.add_route(id, seq, route);
                            })
                        }
                    })
                    .register();
                self.proxies.insert(device.upcast_ref().id(), id);
                self.bound.insert(id, Bound::Device(listener, device));
            }
            ObjectType::Metadata if prop("metadata.name") == Some("default") => {
                if self.metadata.is_some() {
                    return;
                }
                let metadata: Metadata = match registry.bind(g) {
                    Ok(m) => m,
                    Err(e) => return tracing::debug!("ses: metadata bağlanamadı: {e}"),
                };
                let listener = metadata
                    .add_listener_local()
                    .property({
                        let weak = weak.clone();
                        move |subject, key, type_, value| {
                            if subject == 0 {
                                with(&weak, |s| s.default_changed(key, type_, value));
                            }
                            0
                        }
                    })
                    .register();
                self.metadata = Some(id);
                self.bound.insert(id, Bound::Metadata(listener, metadata));
            }
            _ => {}
        }
    }

    /// Kaldırılan nesnenin vekili (borç bırakıldıktan sonra düşürülür).
    fn global_remove(&mut self, id: u32) -> Option<Bound> {
        if self.model.remove(id) {
            self.mark_dirty();
        }
        if self.metadata == Some(id) {
            self.metadata = None;
            self.model.default_sink = None;
            self.model.default_source = None;
            self.mark_dirty();
        }
        self.proxies.retain(|_, g| *g != id);
        self.bound.remove(&id)
    }

    fn default_changed(&mut self, key: Option<&str>, type_: Option<&str>, value: Option<&str>) {
        match key {
            Some(KEY_DEFAULT_SINK) => self.model.default_sink = parse_default_name(type_, value),
            Some(KEY_DEFAULT_SOURCE) => self.model.default_source = parse_default_name(type_, value),
            // Tüm anahtarlar silindi.
            None => {
                self.model.default_sink = None;
                self.model.default_source = None;
            }
            _ => return,
        }
        self.mark_dirty();
    }

    /// Sunucu gidiş-dönüşü ister; bitince `barrier` işlenir. Çekirdek yoksa hemen işlenir.
    fn barrier(&mut self, barrier: Barrier) {
        match self.core.as_ref().map(|c| c.sync(0)) {
            Some(Ok(seq)) => self.barriers.push((seq, barrier)),
            _ => self.resolve(barrier),
        }
    }

    fn sync_done(&mut self, seq: AsyncSeq) {
        let (done, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.barriers).into_iter().partition(|(s, _)| *s == seq);
        self.barriers = rest;
        for (_, b) in done {
            self.resolve(b);
        }
    }

    fn resolve(&mut self, barrier: Barrier) {
        match barrier {
            Barrier::Routes { device, seq } => {
                if self.model.commit_routes(device, seq) {
                    self.mark_dirty();
                }
                // Route'u bekleyen yazımlar artık gönderilebilir.
                for id in self.model.writes.parked() {
                    if !matches!(self.model.path(id), Some(VolumePath::AwaitRoutes { .. }))
                        && let Some(w) = self.model.writes.take_parked(id)
                    {
                        self.dispatch(id, w, true);
                    }
                }
            }
            Barrier::Write { node, token } => {
                if let Some(next) = self.model.writes.done(node, token, Instant::now()) {
                    self.dispatch(node, next, true);
                }
                self.mark_dirty();
            }
        }
    }

    /// Cihazın route listesini yeniden okur (bilgisi değişti).
    fn request_routes(&mut self, device: u32, readable: bool) {
        self.next_seq = self.next_seq.wrapping_add(1) & 0x3fff_ffff;
        let seq = self.next_seq;
        if !self.model.begin_routes(device, seq) {
            return;
        }
        match self.bound.get(&device) {
            Some(Bound::Device(_, d)) if readable => d.enum_params(seq, Some(ParamType::Route), 0, u32::MAX),
            // Route parametresi yok: liste kesin olarak boş.
            _ => {}
        }
        self.barrier(Barrier::Routes { device, seq });
    }

    /// Bir vekile yapılan istek reddedildi (ör. geçersiz route): hedef bırakılır.
    fn proxy_error(&mut self, proxy: u32, res: i32) {
        let Some(&global) = self.proxies.get(&proxy) else { return };
        let affected: Vec<u32> = if self.model.nodes.contains_key(&global) {
            vec![global]
        } else {
            self.model
                .writes
                .ids()
                .into_iter()
                .filter(|id| self.model.nodes.get(id).and_then(|n| n.info.device_id) == Some(global))
                .collect()
        };
        if affected.is_empty() {
            return tracing::debug!(res, "ses: nesne {global} isteği reddedildi");
        }
        for id in affected {
            self.model.writes.forget(id);
        }
        self.mark_dirty();
        self.fail("volume", &t!("shell-audio-error-write-rejected", code = res.to_string()));
    }

    fn resolve_target(&self, target: AudioTarget) -> Option<u32> {
        match target {
            AudioTarget::DefaultSink => self.model.default_node(NodeKind::Sink),
            AudioTarget::DefaultSource => self.model.default_node(NodeKind::Source),
            AudioTarget::Node(id) => self.model.nodes.contains_key(&id).then_some(id),
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Stop => self.stop = true,
            Command::SetVolume(target, v) => self.volume(target, |_| v),
            Command::AdjustVolume(target, delta) => {
                let max = self.options.max_volume;
                // Sınırın üstündeyken artırma seviyeyi düşürmez.
                self.volume(target, |cur| if delta > 0.0 { (cur + delta).min(max.max(cur)) } else { cur + delta })
            }
            Command::SetMuted(target, m) => self.mute(target, |_| m),
            Command::ToggleMuted(target) => self.mute(target, |m| !m),
            Command::SetDefault(id) => self.set_default(id),
            Command::Diagnose(reply) => self.diagnose(reply),
            Command::Summary(reply) => reply(diagnose::summary(&self.model, self.options.backend, self.available)),
        }
    }

    fn volume(&mut self, target: AudioTarget, f: impl FnOnce(f32) -> f32) {
        let Some(id) = self.resolve_target(target) else { return self.fail("volume", &t!("shell-audio-error-unknown-target")) };
        let Some((channels, _)) = self.model.levels(id) else { return };
        if channels.is_empty() {
            return self.fail("volume", &t!("shell-audio-error-volume-unknown"));
        }
        let cur = to_perceptual(model::peak(&channels));
        let max = self.options.max_volume.max(cur);
        let v = f(cur);
        if !v.is_finite() {
            return self.fail("volume", &t!("shell-audio-error-volume-invalid"));
        }
        let channels = scale_channels(&channels, v.clamp(0.0, max));
        self.write(id, VolumeWrite { channels: Some(channels), mute: None });
    }

    fn mute(&mut self, target: AudioTarget, f: impl FnOnce(bool) -> bool) {
        let Some(id) = self.resolve_target(target) else { return self.fail("mute", &t!("shell-audio-error-unknown-target")) };
        let Some((_, muted)) = self.model.levels(id) else { return };
        self.write(id, VolumeWrite { channels: None, mute: Some(f(muted)) });
    }

    fn write(&mut self, id: u32, w: VolumeWrite) {
        if self.options.backend == AudioBackend::Wpctl {
            // WirePlumber'ın karıştırıcısı (wpctl): GNOME/KDE/pavucontrol ile aynı ölçek ve yol.
            // wpctl yoksa (ya da başlatılamıyorsa) süreç içi yazıma düşülür.
            if let Some(writer) = self.wpctl.as_ref().filter(|w| w.usable()) {
                for job in wpctl_jobs(id, &w) {
                    writer.push(job);
                }
                self.model.writes.optimistic(id, &w, Instant::now());
                self.mark_dirty();
                return;
            }
        }
        if let Some(w) = self.model.writes.request(id, w) {
            self.dispatch(id, w, true);
        }
        self.mark_dirty();
    }

    /// Yazımı seviyenin yoluna gönderir. `may_park`: route listesi gelmemişse bekletilebilir
    /// (yoksa son çare düğüm Props'u).
    fn dispatch(&mut self, id: u32, w: VolumeWrite, may_park: bool) {
        let action = if w.channels.is_some() { "volume" } else { "mute" };
        let sent = match self.model.path(id) {
            Some(VolumePath::AwaitRoutes { .. }) if may_park => {
                self.model.writes.park(id, w, Instant::now());
                return;
            }
            Some(VolumePath::Route { device, route }) => match self.bound.get(&device) {
                Some(Bound::Device(_, d)) => serialize(w.route(&route))
                    .and_then(|b| Pod::from_bytes(&b).map(|pod| d.set_param(ParamType::Route, 0, pod)))
                    .is_some(),
                _ => false,
            },
            Some(_) => match self.bound.get(&id) {
                Some(Bound::Node(_, node)) => serialize(w.props())
                    .and_then(|b| Pod::from_bytes(&b).map(|pod| node.set_param(ParamType::Props, 0, pod)))
                    .is_some(),
                _ => false,
            },
            None => false,
        };
        if !sent {
            self.model.writes.forget(id);
            self.mark_dirty();
            return self.fail(action, &t!("shell-audio-error-param-write"));
        }
        let token = self.model.writes.sent(id);
        self.barrier(Barrier::Write { node: id, token });
    }

    fn set_default(&mut self, id: u32) {
        let Some(node) = self.model.nodes.get(&id) else { return self.fail("default", &t!("shell-audio-error-unknown-device")) };
        let key = match node.kind {
            NodeKind::Sink => KEY_CONFIGURED_SINK,
            NodeKind::Source => KEY_CONFIGURED_SOURCE,
            _ => return self.fail("default", &t!("shell-audio-error-stream-default")),
        };
        let value = default_value(&node.info.name);
        match self.metadata.and_then(|m| self.bound.get(&m)) {
            Some(Bound::Metadata(_, md)) => md.set_property(0, key, Some("Spa:String:JSON"), Some(&value)),
            _ => self.fail("default", &t!("shell-audio-error-no-metadata")),
        }
    }

    /// Tanı raporu: model anlık görüntüsü burada, `wpctl` karşılaştırması ayrı thread'de (ses
    /// döngüsü beklemez).
    fn diagnose(&mut self, reply: DiagnoseReply) {
        let report = diagnose::report(&self.model, self.options.backend, self.available, self.options.remote.is_some());
        let remote = self.options.remote.clone();
        let spawned = std::thread::Builder::new()
            .name("nuxut-audio-diag".into())
            .spawn(move || reply(diagnose::complete(report, remote.as_deref())));
        if let Err(e) = spawned {
            tracing::debug!("ses: tanı thread'i başlatılamadı: {e}");
        }
    }
}

/// `wpctl` işleri: seviye algısal (kübik) ölçekte, 1.0 = %100.
fn wpctl_jobs(id: u32, w: &VolumeWrite) -> Vec<wpctl::Job> {
    let mut out = Vec::new();
    if let Some(c) = &w.channels {
        let level = to_perceptual(model::peak(c));
        out.push(wpctl::Job {
            node: id,
            kind: wpctl::Kind::Volume,
            args: vec!["set-volume".into(), id.to_string(), format!("{level:.4}")],
        });
    }
    if let Some(m) = w.mute {
        out.push(wpctl::Job {
            node: id,
            kind: wpctl::Kind::Mute,
            args: vec!["set-mute".into(), id.to_string(), if m { "1" } else { "0" }.into()],
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpctl_args_use_perceptual_scale() {
        // Doğrusal 0.125 = algısal 0.5 (%50).
        let w = VolumeWrite { channels: Some(vec![0.125, 0.125]), mute: Some(true) };
        let a = wpctl_jobs(42, &w);
        assert_eq!(a[0].args, ["set-volume", "42", "0.5000"]);
        assert_eq!(a[1].args, ["set-mute", "42", "1"]);
        assert_eq!((a[0].kind, a[1].kind), (wpctl::Kind::Volume, wpctl::Kind::Mute));
        let full = VolumeWrite { channels: Some(vec![1.0]), mute: None };
        let a = wpctl_jobs(7, &full);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].args, ["set-volume", "7", "1.0000"]);
    }

    #[test]
    fn max_volume_percent_is_clamped() {
        assert_eq!(AudioOptions::max_volume_from_percent(0), 1.0);
        assert_eq!(AudioOptions::max_volume_from_percent(125), 1.25);
        assert_eq!(AudioOptions::max_volume_from_percent(400), 1.5);
    }

    #[test]
    fn backend_names_match_serde() {
        #[derive(Deserialize)]
        struct T {
            b: AudioBackend,
        }
        for b in AudioBackend::ALL {
            let t: T = toml::from_str(&format!("b = \"{}\"", b.name())).unwrap();
            assert_eq!(t.b, b);
        }
    }
}
