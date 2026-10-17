//! Saf ses modeli: PipeWire düğümleri/cihazları ve `default` metadata'sından türetilen durum;
//! `Props`/`Route` parametrelerinin ayrıştırılması, seviyenin hangi yoldan okunup yazılacağı
//! (cihaz route'u mu düğüm `Props`'u mu) ve yazılacak parametrelerin kurulması. PipeWire
//! bağlantısından bağımsızdır (birim testleri burada).
//!
//! **Seviyenin kaynağı** WirePlumber mixer-api (`collect_node_info`) ve pipewire-pulse
//! (`collect_device_info`) ile aynıdır: düğüm bir kart cihazına aitse (`device.id` +
//! `card.profile.device`) ve cihazın etkin route'larından biri bu profil cihazına aitse seviye o
//! route'un `props`'undan okunur ve oraya yazılır; aksi halde düğümün `Props`'u. Düğümün
//! `Props`'u kart düğümünde route'un gecikmeli bir yansımasıdır (ACP → WirePlumber → düğüm);
//! yazım ona yapılırsa yalnızca yazılım seviyesi değişir, donanım karıştırıcısı yerinde kalır.

use std::collections::BTreeMap;

use pipewire::spa::pod::{Object, Property, PropertyFlags, Value, ValueArray};
use pipewire::spa::sys;
use pipewire::spa::utils::Id;

use super::writes::Writes;
use nuxut_dbus::clean_str;

/// En fazla izlenen düğüm (cihaz + akış).
pub const MAX_NODES: usize = 512;
/// En fazla izlenen ses cihazı (kart).
pub const MAX_DEVICES: usize = 64;
/// Bir düğümde kabul edilen en fazla kanal.
pub const MAX_CHANNELS: usize = 64;
/// Cihaz başına en fazla etkin route.
const MAX_ROUTES: usize = 32;
const MAX_NAME: usize = 256;
const MAX_ICON: usize = 64;
/// Route `port.type` (sınıf: `speaker`, `headphones`, `hdmi`…) üst sınırı.
const MAX_PORT_TYPE: usize = 32;
/// Metadata değeri (JSON) üst sınırı.
const MAX_METADATA_VALUE: usize = 1024;

pub(crate) const KEY_DEFAULT_SINK: &str = "default.audio.sink";
pub(crate) const KEY_DEFAULT_SOURCE: &str = "default.audio.source";
pub(crate) const KEY_CONFIGURED_SINK: &str = "default.configured.audio.sink";
pub(crate) const KEY_CONFIGURED_SOURCE: &str = "default.configured.audio.source";

/// Düğümün türü (`media.class`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Sink,
    Source,
    /// Uygulamanın çaldığı akış.
    Playback,
    /// Uygulamanın kaydettiği akış.
    Recording,
}

impl NodeKind {
    pub fn from_class(class: &str) -> Option<Self> {
        match class {
            "Audio/Sink" => Some(Self::Sink),
            "Audio/Source" | "Audio/Source/Virtual" => Some(Self::Source),
            "Stream/Output/Audio" => Some(Self::Playback),
            "Stream/Input/Audio" => Some(Self::Recording),
            _ => None,
        }
    }

    /// Tanıda gösterilen sınıf (`media.class`).
    pub fn class(self) -> &'static str {
        match self {
            Self::Sink => "Audio/Sink",
            Self::Source => "Audio/Source",
            Self::Playback => "Stream/Output/Audio",
            Self::Recording => "Stream/Input/Audio",
        }
    }
}

/// Düğümün bilgi sözlüğünden okunan alanlar (`node.info`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct NodeInfo {
    pub name: String,
    pub description: String,
    pub app: Option<String>,
    pub icon: Option<String>,
    pub media: Option<String>,
    pub device_id: Option<u32>,
    /// `card.profile.device`: cihazın route'larından hangisine ait olduğu.
    pub profile_device: Option<i32>,
    /// pavucontrol vb. tepe ölçer akışı (`stream.monitor`).
    pub monitor: bool,
    /// Cihaz sınıfı ipuçları (`device.api`, `device.bus`, `device.form-factor`); yalnızca
    /// küçük harf sınıf adları, ikon seçimi için.
    pub api: Option<String>,
    pub bus: Option<String>,
    pub form_factor: Option<String>,
}

/// Sınıf ipucu: kısa, yalnızca `[a-z0-9_-]` (başka değer yok sayılır).
fn class_hint(v: &str) -> Option<String> {
    let v = v.trim();
    (!v.is_empty() && v.len() <= MAX_PORT_TYPE && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)))
        .then(|| v.to_ascii_lowercase())
}

/// Cihazın türü (ikon seçimi). Servis UI sözlüğü bilmez; ikon eşlemesi widget'ta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceClass {
    #[default]
    Speaker,
    Headphones,
    /// Mikrofonlu kulaklık (çıkış ya da girişi).
    Headset,
    /// HDMI / DisplayPort (ekranın hoparlörü).
    Display,
    /// Kulaklık/hoparlör olarak tanınmayan Bluetooth ses cihazı.
    Bluetooth,
    Usb,
    Microphone,
    Webcam,
}

impl DeviceClass {
    /// Sıra: (1) Bluetooth kulaklık/hoparlör ayrımı form faktöründen, (2) kart düğümünün etkin
    /// route'unun `port.type`'ı (kulaklık takılınca "headphones" olur), (3) `device.form-factor`,
    /// (4) ad (`hdmi`/`displayport`), (5) veri yolu (USB), (6) yöne göre hoparlör/mikrofon.
    pub(crate) fn classify(sink: bool, info: &NodeInfo, port_type: Option<&str>) -> Self {
        let ff = info.form_factor.as_deref();
        let bluetooth = info.api.as_deref() == Some("bluez5") || info.bus.as_deref() == Some("bluetooth");
        if bluetooth {
            return match ff {
                Some("headphone") => Self::Headphones,
                Some("headset" | "hands-free" | "handset") => Self::Headset,
                _ => Self::Bluetooth,
            };
        }
        let by_port = match port_type {
            Some("headphones") => Some(Self::Headphones),
            Some("headset" | "handset" | "earpiece" | "handsfree") => Some(Self::Headset),
            Some("hdmi" | "video") => Some(Self::Display),
            Some("mic") => Some(Self::Microphone),
            Some("speaker") => Some(Self::Speaker),
            Some("usb") => Some(Self::Usb),
            _ => None,
        };
        if let Some(c) = by_port {
            return c;
        }
        let by_ff = match ff {
            Some("headphone") => Some(Self::Headphones),
            Some("headset" | "handset" | "hands-free") => Some(Self::Headset),
            Some("tv" | "monitor") => Some(Self::Display),
            Some("webcam") => Some(Self::Webcam),
            Some("microphone") => Some(Self::Microphone),
            _ => None,
        };
        if let Some(c) = by_ff {
            return c;
        }
        let name = info.name.to_ascii_lowercase();
        if name.contains("hdmi") || name.contains("displayport") {
            return Self::Display;
        }
        if info.bus.as_deref() == Some("usb") {
            return Self::Usb;
        }
        if sink { Self::Speaker } else { Self::Microphone }
    }
}

impl NodeInfo {
    /// Sözlükten (anahtar → değer) okur; tüm metinler temizlenir ve sınırlanır.
    pub fn parse<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Self {
        let text = |k: &str, max: usize| get(k).map(|v| clean_str(v, max)).filter(|v| !v.is_empty());
        let name = text("node.name", MAX_NAME).unwrap_or_default();
        let description = text("node.description", MAX_NAME)
            .or_else(|| text("node.nick", MAX_NAME))
            .unwrap_or_else(|| name.clone());
        Self {
            description,
            app: text("application.name", MAX_NAME),
            icon: text("application.icon-name", MAX_ICON)
                .filter(|i| i.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))),
            media: text("media.name", MAX_NAME),
            device_id: get("device.id").and_then(|v| v.parse().ok()),
            profile_device: get("card.profile.device").and_then(|v| v.parse().ok()),
            monitor: get("stream.monitor") == Some("true"),
            api: get("device.api").and_then(class_hint),
            bus: get("device.bus").and_then(class_hint),
            form_factor: get("device.form-factor").and_then(class_hint),
            name,
        }
    }
}

/// Bir `Props` nesnesinin (düğüm parametresi ya da route `props`'u) ses alanları.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct VolumeProps {
    /// Doğrusal kanal seviyeleri (`channelVolumes`; yoksa tek `volume`).
    pub channels: Option<Vec<f32>>,
    /// Yazılım payı (`softVolumes`): donanım karıştırıcısının karşılayamadığı kısım.
    pub soft: Option<Vec<f32>>,
    pub mute: Option<bool>,
    /// `volume` (tek değer; audioconvert'te kanallardan bağımsız çarpan).
    pub volume: Option<f32>,
    pub base: Option<f32>,
    pub step: Option<f32>,
    /// `channelVolumes` donanım bayraklı (`SPA_POD_PROP_FLAG_HARDWARE`).
    pub hw_volume: bool,
    pub hw_mute: bool,
}

fn finite_volume(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 16.0) } else { 0.0 }
}

fn float_array(v: &Value) -> Option<Vec<f32>> {
    match v {
        Value::ValueArray(ValueArray::Float(v)) if !v.is_empty() && v.len() <= MAX_CHANNELS => {
            Some(v.iter().copied().map(finite_volume).collect())
        }
        _ => None,
    }
}

impl VolumeProps {
    /// `SPA_TYPE_OBJECT_Props` nesnesi; başka türse `None`.
    pub fn parse(value: &Value) -> Option<Self> {
        let Value::Object(o) = value else { return None };
        if o.type_ != sys::SPA_TYPE_OBJECT_Props {
            return None;
        }
        let mut out = Self::default();
        let hw = |p: &Property| p.flags.contains(PropertyFlags::HARDWARE);
        for p in &o.properties {
            match (p.key, &p.value) {
                (sys::SPA_PROP_channelVolumes, v) => {
                    if let Some(c) = float_array(v) {
                        out.channels = Some(c);
                        out.hw_volume = hw(p);
                    }
                }
                (sys::SPA_PROP_softVolumes, v) => out.soft = float_array(v),
                (sys::SPA_PROP_volume, Value::Float(v)) => out.volume = Some(finite_volume(*v)),
                (sys::SPA_PROP_mute, Value::Bool(m)) => {
                    out.mute = Some(*m);
                    out.hw_mute = hw(p);
                }
                (sys::SPA_PROP_volumeBase, Value::Float(v)) if v.is_finite() => out.base = Some(*v),
                (sys::SPA_PROP_volumeStep, Value::Float(v)) if v.is_finite() => out.step = Some(*v),
                _ => {}
            }
        }
        if out.channels.is_none() {
            out.channels = out.volume.map(|v| vec![v]);
        }
        Some(out)
    }

    /// Seviye taşıyor mu (mixer-api'nin `node_info_fill` koşulu: `channelVolumes` var).
    pub fn has_volume(&self) -> bool {
        self.channels.is_some()
    }

    /// Gelen parçayı birleştirir (bir düğüm birden çok `Props` nesnesi yayınlar: audioconvert'in
    /// seviyeli olanı ve sürücünün seviyesiz olanı). Değiştiyse `true`.
    pub fn merge(&mut self, other: VolumeProps) -> bool {
        let before = self.clone();
        if other.channels.is_some() {
            self.channels = other.channels;
            self.hw_volume = other.hw_volume;
        }
        if other.mute.is_some() {
            self.mute = other.mute;
            self.hw_mute = other.hw_mute;
        }
        macro_rules! take {
            ($($f:ident),*) => { $( if other.$f.is_some() { self.$f = other.$f; } )* };
        }
        take!(soft, volume, base, step);
        *self != before
    }
}

/// Cihazın etkin route'u (`SPA_PARAM_Route`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Route {
    pub index: i32,
    pub direction: u32,
    /// Profil cihazı (düğümün `card.profile.device`'ı ile eşleşir).
    pub device: i32,
    /// Route'un seviyesi (`props`).
    pub props: Option<VolumeProps>,
    /// `info` sözlüğündeki `port.type` (yalnızca sınıf; ad değil).
    pub port_type: Option<String>,
}

impl Route {
    pub fn parse(value: &Value) -> Option<Self> {
        let Value::Object(o) = value else { return None };
        if o.type_ != sys::SPA_TYPE_OBJECT_ParamRoute {
            return None;
        }
        let (mut index, mut direction, mut device, mut props, mut port_type) = (None, None, None, None, None);
        for p in &o.properties {
            match (p.key, &p.value) {
                (sys::SPA_PARAM_ROUTE_index, Value::Int(i)) => index = Some(*i),
                (sys::SPA_PARAM_ROUTE_direction, Value::Id(Id(d))) => direction = Some(*d),
                (sys::SPA_PARAM_ROUTE_device, Value::Int(d)) => device = Some(*d),
                (sys::SPA_PARAM_ROUTE_props, v) => props = VolumeProps::parse(v),
                (sys::SPA_PARAM_ROUTE_info, Value::Struct(items)) => port_type = info_port_type(items),
                _ => {}
            }
        }
        Some(Self { index: index?, direction: direction?, device: device?, props, port_type })
    }

    pub fn has_volume(&self) -> bool {
        self.props.as_ref().is_some_and(VolumeProps::has_volume)
    }
}

/// Route `info` yapısı: `Int(n)`, ardından `n` adet anahtar/değer dizgisi.
fn info_port_type(items: &[Value]) -> Option<String> {
    let mut it = items.iter().skip(1);
    while let (Some(Value::String(k)), Some(Value::String(v))) = (it.next(), it.next()) {
        if k == "port.type" {
            let v = clean_str(v, MAX_PORT_TYPE);
            return (!v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))).then_some(v);
        }
    }
    None
}

/// Yazılacak ses değişikliği.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VolumeWrite {
    pub channels: Option<Vec<f32>>,
    pub mute: Option<bool>,
}

impl VolumeWrite {
    /// Yenisi eskisinin üstüne: verilmeyen alan eskisinden kalır (birleştirme).
    pub fn merged(older: Option<VolumeWrite>, newer: VolumeWrite) -> VolumeWrite {
        match older {
            None => newer,
            Some(o) => VolumeWrite { channels: newer.channels.or(o.channels), mute: newer.mute.or(o.mute) },
        }
    }

    /// Düğüme yazılacak `Props` nesnesi.
    pub fn props(&self) -> Object {
        let mut properties = Vec::new();
        if let Some(c) = &self.channels {
            properties.push(Property::new(sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(c.clone()))));
        }
        if let Some(m) = self.mute {
            properties.push(Property::new(sys::SPA_PROP_mute, Value::Bool(m)));
        }
        Object { type_: sys::SPA_TYPE_OBJECT_Props, id: sys::SPA_PARAM_Props, properties }
    }

    /// Cihaza yazılacak `Route` nesnesi: mixer-api / pipewire-pulse ile aynı alanlar
    /// (`index`, `device`, `props`, `save = true`; WirePlumber saklar).
    pub fn route(&self, route: &Route) -> Object {
        Object {
            type_: sys::SPA_TYPE_OBJECT_ParamRoute,
            id: sys::SPA_PARAM_Route,
            properties: vec![
                Property::new(sys::SPA_PARAM_ROUTE_index, Value::Int(route.index)),
                Property::new(sys::SPA_PARAM_ROUTE_device, Value::Int(route.device)),
                Property::new(sys::SPA_PARAM_ROUTE_props, Value::Object(self.props())),
                Property::new(sys::SPA_PARAM_ROUTE_save, Value::Bool(true)),
            ],
        }
    }
}

/// Doğrusal → algısal (kübik; wpctl/pavucontrol ölçeği). 1.0 = %100.
pub fn to_perceptual(linear: f32) -> f32 {
    finite_volume(linear).cbrt()
}

/// Algısal → doğrusal.
pub fn to_linear(perceptual: f32) -> f32 {
    let p = if perceptual.is_finite() { perceptual.max(0.0) } else { 0.0 };
    p * p * p
}

/// En yüksek kanal.
pub(crate) fn peak(channels: &[f32]) -> f32 {
    channels.iter().copied().fold(0.0f32, f32::max)
}

/// Kanal seviyelerini yeni algısal seviyeye ölçekler; kanal dengesi (oran) korunur, en yüksek
/// kanal hedefe gelir (pavucontrol/pipewire-pulse'un `pa_cvolume_scale`'i gibi).
pub(crate) fn scale_channels(channels: &[f32], perceptual: f32) -> Vec<f32> {
    let target = to_linear(perceptual);
    let max = peak(channels);
    if channels.is_empty() {
        return vec![target];
    }
    if max <= f32::EPSILON {
        return vec![target; channels.len()];
    }
    channels.iter().map(|c| c / max * target).collect()
}

/// `default` metadata değerinden (`{"name":"..."}`) düğüm adı.
pub(crate) fn parse_default_name(type_: Option<&str>, value: Option<&str>) -> Option<String> {
    let value = value?;
    if value.len() > MAX_METADATA_VALUE || type_.is_some_and(|t| t != "Spa:String:JSON") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(value).ok()?;
    let name = v.get("name")?.as_str()?;
    (!name.is_empty() && name.len() <= MAX_NAME).then(|| name.to_owned())
}

/// `default.configured.audio.*` için JSON değeri.
pub(crate) fn default_value(name: &str) -> String {
    serde_json::json!({ "name": name }).to_string()
}

// ---------------------------------------------------------------------------------------
// Seviye yolu

/// Seviyenin okunduğu/yazıldığı yer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum VolumePath {
    /// Kart cihazının etkin route'u (donanım karıştırıcısı + yazılım payı, WirePlumber saklar).
    Route { device: u32, route: Route },
    /// Kart düğümü ama cihazın route listesi henüz okunmadı: yazım bekletilir (Props'a düşmek
    /// donanım seviyesini yerinde bırakırdı).
    AwaitRoutes { device: u32 },
    /// Düğümün `Props`'u.
    Props(PropsReason),
}

/// Neden `Props` yolu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropsReason {
    /// Uygulama akışı.
    Stream,
    /// `device.id` yok (sanal/filtre düğümü).
    NoDeviceId,
    /// `card.profile.device` yok.
    NoProfileDevice,
    /// `device.id`'deki cihaz izlenmiyor (ses cihazı değil ya da sınır).
    DeviceNotTracked,
    /// Cihazın etkin route'ları arasında bu profil cihazı yok (ör. pro-audio).
    NoRoute,
    /// Route var ama seviye taşımıyor.
    RouteWithoutVolume,
}

impl PropsReason {
    pub fn describe(self) -> &'static str {
        // i18n: tanı yalnızca `audio diagnose` CLI çıktısı
        match self {
            Self::Stream => "uygulama akışı",
            Self::NoDeviceId => "düğümde device.id yok (sanal/filtre düğümü)",
            Self::NoProfileDevice => "düğümde card.profile.device yok",
            Self::DeviceNotTracked => "device.id'deki cihaz izlenmiyor",
            Self::NoRoute => "cihazın etkin route'ları arasında bu profil cihazı yok",
            Self::RouteWithoutVolume => "route seviye (channelVolumes) taşımıyor",
        }
    }
}

// ---------------------------------------------------------------------------------------
// Ağaç

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NodeEntry {
    pub kind: NodeKind,
    pub info: NodeInfo,
    /// Düğümün `Props`'u (birleştirilmiş; gelene dek boş).
    pub props: VolumeProps,
    /// Düğüm durumu `running` mi (akışta: ses gerçekten akıyor; duraklatılmış/corked akış `idle`).
    pub running: bool,
}

impl NodeEntry {
    pub fn new(kind: NodeKind, info: NodeInfo) -> Self {
        Self { kind, info, props: VolumeProps::default(), running: false }
    }
}

/// Bir ses cihazının route durumu.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DeviceEntry {
    /// Son tamamlanan numaralandırmanın etkin route'ları.
    pub routes: Vec<Route>,
    /// En az bir numaralandırma tamamlandı mı (route yokluğu ancak o zaman kesindir).
    pub committed: bool,
    /// Süren numaralandırma: (sıra no, toplanan route'lar).
    pending: Option<(i32, Vec<Route>)>,
    /// Tamamlanan numaralandırma sayısı (tanı).
    pub enumerations: u32,
}

#[derive(Debug, Default)]
pub(crate) struct Model {
    pub nodes: BTreeMap<u32, NodeEntry>,
    /// Cihaz id → route durumu.
    pub devices: BTreeMap<u32, DeviceEntry>,
    pub default_sink: Option<String>,
    pub default_source: Option<String>,
    /// Gönderilen/bekleyen yazımlar ve gösterilen hedef seviyeler.
    pub writes: Writes,
}

impl Model {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn add_node(&mut self, id: u32, entry: NodeEntry) -> bool {
        if !self.nodes.contains_key(&id) && self.nodes.len() >= MAX_NODES {
            return false;
        }
        self.nodes.insert(id, entry);
        true
    }

    pub fn add_device(&mut self, id: u32) -> bool {
        if !self.devices.contains_key(&id) && self.devices.len() >= MAX_DEVICES {
            return false;
        }
        self.devices.entry(id).or_default();
        true
    }

    /// Nesne kaldırıldı; değiştiyse `true`.
    pub fn remove(&mut self, id: u32) -> bool {
        self.writes.forget(id);
        self.nodes.remove(&id).is_some() | self.devices.remove(&id).is_some()
    }

    pub fn set_info(&mut self, id: u32, info: NodeInfo) -> bool {
        match self.nodes.get_mut(&id) {
            Some(n) if n.info != info => {
                n.info = info;
                true
            }
            _ => false,
        }
    }

    /// Düğüm durumu değişti (`running` mi); değiştiyse `true`.
    pub fn set_running(&mut self, id: u32, running: bool) -> bool {
        self.nodes.get_mut(&id).is_some_and(|n| std::mem::replace(&mut n.running, running) != running)
    }

    /// Düğümün `Props` parametresi geldi; değiştiyse `true`.
    pub fn set_props(&mut self, id: u32, props: VolumeProps) -> bool {
        self.nodes.get_mut(&id).is_some_and(|n| n.props.merge(props))
    }

    /// Cihazın route numaralandırması başladı (öncekini geçersiz kılar).
    pub fn begin_routes(&mut self, device: u32, seq: i32) -> bool {
        let Some(d) = self.devices.get_mut(&device) else { return false };
        d.pending = Some((seq, Vec::new()));
        true
    }

    /// Numaralandırmadan bir route; sıra no süren numaralandırmanınki değilse yok sayılır.
    pub fn add_route(&mut self, device: u32, seq: i32, route: Route) -> bool {
        let Some((s, routes)) = self.devices.get_mut(&device).and_then(|d| d.pending.as_mut()) else { return false };
        if *s != seq || routes.len() >= MAX_ROUTES {
            return false;
        }
        // Aynı profil cihazı iki kez gelirse sonuncusu geçerli.
        routes.retain(|r| !(r.device == route.device && r.direction == route.direction));
        routes.push(route);
        true
    }

    /// Numaralandırma bitti (sunucu gidiş-dönüşü): route listesi bütünüyle değiştirilir —
    /// kaybolan route'lar düşer. Liste değiştiyse `true`.
    pub fn commit_routes(&mut self, device: u32, seq: i32) -> bool {
        let Some(d) = self.devices.get_mut(&device) else { return false };
        if d.pending.as_ref().is_none_or(|(s, _)| *s != seq) {
            return false;
        }
        let (_, routes) = d.pending.take().unwrap_or_default();
        d.enumerations = d.enumerations.saturating_add(1);
        let changed = !d.committed || d.routes != routes;
        d.committed = true;
        d.routes = routes;
        changed
    }

    /// Seviyenin okunduğu/yazıldığı yer (mixer-api `collect_node_info` ile aynı karar; tek fark:
    /// route listesi henüz okunmamışsa Props'a düşmek yerine beklenir).
    pub fn path(&self, id: u32) -> Option<VolumePath> {
        let n = self.nodes.get(&id)?;
        if !matches!(n.kind, NodeKind::Sink | NodeKind::Source) {
            return Some(VolumePath::Props(PropsReason::Stream));
        }
        let Some(device) = n.info.device_id else { return Some(VolumePath::Props(PropsReason::NoDeviceId)) };
        let Some(profile_device) = n.info.profile_device else {
            return Some(VolumePath::Props(PropsReason::NoProfileDevice));
        };
        let Some(d) = self.devices.get(&device) else { return Some(VolumePath::Props(PropsReason::DeviceNotTracked)) };
        if !d.committed {
            return Some(VolumePath::AwaitRoutes { device });
        }
        let mut matching = d.routes.iter().filter(|r| r.device == profile_device).peekable();
        if matching.peek().is_none() {
            return Some(VolumePath::Props(PropsReason::NoRoute));
        }
        match matching.find(|r| r.has_volume()) {
            Some(r) => Some(VolumePath::Route { device, route: r.clone() }),
            None => Some(VolumePath::Props(PropsReason::RouteWithoutVolume)),
        }
    }

    /// Sunucudaki seviye (doğrusal kanallar, sessiz): route yolunda route'tan, değilse düğümün
    /// `Props`'undan. Kanallar bilinmiyorsa boş.
    pub fn actual(&self, id: u32) -> Option<(Vec<f32>, bool)> {
        let n = self.nodes.get(&id)?;
        let props = match self.path(id) {
            Some(VolumePath::Route { route, .. }) => route.props.unwrap_or_default(),
            _ => n.props.clone(),
        };
        Some((props.channels.unwrap_or_default(), props.mute.unwrap_or(false)))
    }

    /// Gösterilen seviye: sunucudaki, üstüne gönderilmiş/bekleyen yazımın hedefi.
    pub fn levels(&self, id: u32) -> Option<(Vec<f32>, bool)> {
        let (channels, mute) = self.actual(id)?;
        let (c, m) = self.writes.overlay(id);
        Some((c.map(<[f32]>::to_vec).unwrap_or(channels), m.unwrap_or(mute)))
    }

    /// Gösterilen algısal seviye (en yüksek kanal) ve sessizlik.
    pub fn volume(&self, id: u32) -> Option<(f32, bool)> {
        let (c, m) = self.levels(id)?;
        (!c.is_empty()).then(|| (to_perceptual(peak(&c)), m))
    }

    /// Yazım bitip sunucu hedefe ulaştıysa (ya da süre dolduysa) hedef seviyeleri bırakır.
    /// Bir şey değiştiyse `true`.
    pub fn reconcile(&mut self, now: std::time::Instant) -> bool {
        let ids = self.writes.ids();
        let mut changed = false;
        for id in ids {
            if let Some((c, m)) = self.actual(id) {
                changed |= self.writes.reconcile(id, &c, m);
            }
        }
        changed | self.writes.expire(now)
    }

    /// Adı verilen türdeki varsayılan düğüm.
    pub fn default_node(&self, kind: NodeKind) -> Option<u32> {
        let name = match kind {
            NodeKind::Sink => self.default_sink.as_deref()?,
            NodeKind::Source => self.default_source.as_deref()?,
            _ => return None,
        };
        self.nodes.iter().find(|(_, n)| n.kind == kind && n.info.name == name).map(|(id, _)| *id)
    }

    /// Kart düğümünün etkin route'unun `port.type`'ı (cihaz sınıfı ipucu).
    pub fn port_type(&self, id: u32) -> Option<&str> {
        let n = self.nodes.get(&id)?;
        let d = self.devices.get(&n.info.device_id?)?;
        let profile_device = n.info.profile_device?;
        d.routes.iter().find(|r| r.device == profile_device)?.port_type.as_deref()
    }

    pub fn state(&self, available: bool) -> AudioState {
        let mut s = AudioState { available, ..Default::default() };
        let default_sink = self.default_node(NodeKind::Sink);
        let default_source = self.default_node(NodeKind::Source);
        for (&id, n) in &self.nodes {
            let (volume, muted) = self.volume(id).unwrap_or((0.0, self.levels(id).is_some_and(|l| l.1)));
            match n.kind {
                NodeKind::Sink | NodeKind::Source => {
                    let d = AudioDevice {
                        id,
                        name: n.info.name.clone(),
                        description: n.info.description.clone(),
                        volume,
                        muted,
                        is_default: Some(id) == default_sink || Some(id) == default_source,
                        class: DeviceClass::classify(n.kind == NodeKind::Sink, &n.info, self.port_type(id)),
                    };
                    if n.kind == NodeKind::Sink { s.sinks.push(d) } else { s.sources.push(d) }
                }
                NodeKind::Playback | NodeKind::Recording => {
                    if n.info.monitor {
                        continue;
                    }
                    let st = AudioStream {
                        id,
                        app: n.info.app.clone().unwrap_or_else(|| n.info.description.clone()),
                        icon: n.info.icon.clone(),
                        media: n.info.media.clone(),
                        volume,
                        muted,
                        running: n.running,
                    };
                    if n.kind == NodeKind::Playback { s.playback.push(st) } else { s.recording.push(st) }
                }
            }
        }
        let by_desc = |a: &AudioDevice, b: &AudioDevice| {
            a.description.to_lowercase().cmp(&b.description.to_lowercase()).then(a.id.cmp(&b.id))
        };
        s.sinks.sort_by(by_desc);
        s.sources.sort_by(by_desc);
        s
    }
}

// ---------------------------------------------------------------------------------------
// Dış durum

/// Ses cihazı (sink ya da source).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioDevice {
    /// PipeWire nesne id'si (komutlarda kullanılır).
    pub id: u32,
    /// `node.name` (kararlı ad).
    pub name: String,
    /// Gösterilecek ad.
    pub description: String,
    /// Algısal seviye; 1.0 = %100 (üstü yükseltme).
    pub volume: f32,
    pub muted: bool,
    pub is_default: bool,
    /// Tür (ikon seçimi): Bluetooth, route port türü, form faktörü ve addan.
    pub class: DeviceClass,
}

/// Uygulama akışı.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioStream {
    pub id: u32,
    /// Uygulama adı.
    pub app: String,
    /// İkon adı (`application.icon-name`, yalnızca güvenli karakterler).
    pub icon: Option<String>,
    /// Çalınan içeriğin adı (gösterim; loglanmaz).
    pub media: Option<String>,
    pub volume: f32,
    pub muted: bool,
    /// Ses şu an akıyor mu (PipeWire düğümü `running`; duraklatılmış akış `false`).
    pub running: bool,
}

/// Ses durumunun tamamı.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioState {
    /// PipeWire'a bağlı mı?
    pub available: bool,
    /// Çıkış cihazları (ada göre).
    pub sinks: Vec<AudioDevice>,
    /// Giriş cihazları (ada göre).
    pub sources: Vec<AudioDevice>,
    /// Çalan uygulamalar (id sırasıyla).
    pub playback: Vec<AudioStream>,
    /// Kayıt yapan uygulamalar (mikrofon kullanımı).
    pub recording: Vec<AudioStream>,
}

impl AudioState {
    pub fn default_sink(&self) -> Option<&AudioDevice> {
        self.sinks.iter().find(|d| d.is_default)
    }

    pub fn default_source(&self) -> Option<&AudioDevice> {
        self.sources.iter().find(|d| d.is_default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pipewire::spa::pod::deserialize::PodDeserializer;
    use pipewire::spa::pod::serialize::PodSerializer;

    fn info(pairs: &[(&str, &str)]) -> NodeInfo {
        NodeInfo::parse(|k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| *v))
    }

    fn roundtrip(o: Object) -> Value {
        let (cursor, _) = PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(o)).unwrap();
        let bytes = cursor.into_inner();
        PodDeserializer::deserialize_any_from(&bytes).unwrap().1
    }

    fn vol(channels: &[f32], mute: bool) -> VolumeProps {
        VolumeProps { channels: Some(channels.to_vec()), mute: Some(mute), ..Default::default() }
    }

    fn route(index: i32, device: i32, props: Option<VolumeProps>) -> Route {
        Route { index, direction: sys::SPA_DIRECTION_OUTPUT, device, props, port_type: None }
    }

    /// ACP'nin (`alsa-acp-device.c` `build_route`) kurduğu biçimde tam bir route nesnesi:
    /// bayraklı özellikler, `info` yapısı, Id/Int dizileri, Long.
    fn acp_route(index: i32, device: i32, volumes: &[f32]) -> Object {
        let hw = PropertyFlags::HARDWARE;
        let props = Object {
            type_: sys::SPA_TYPE_OBJECT_Props,
            id: sys::SPA_PARAM_Route,
            properties: vec![
                Property { key: sys::SPA_PROP_mute, flags: hw, value: Value::Bool(false) },
                Property {
                    key: sys::SPA_PROP_channelVolumes,
                    flags: hw,
                    value: Value::ValueArray(ValueArray::Float(volumes.to_vec())),
                },
                Property { key: sys::SPA_PROP_volumeBase, flags: PropertyFlags::READONLY, value: Value::Float(1.0) },
                Property { key: sys::SPA_PROP_volumeStep, flags: PropertyFlags::READONLY, value: Value::Float(1.0 / 65536.0) },
                Property::new(sys::SPA_PROP_channelMap, Value::ValueArray(ValueArray::Id(vec![Id(3), Id(4)]))),
                Property::new(sys::SPA_PROP_softVolumes, Value::ValueArray(ValueArray::Float(vec![1.0; volumes.len()]))),
                Property::new(sys::SPA_PROP_latencyOffsetNsec, Value::Long(0)),
            ],
        };
        Object {
            type_: sys::SPA_TYPE_OBJECT_ParamRoute,
            id: sys::SPA_PARAM_Route,
            properties: vec![
                Property::new(sys::SPA_PARAM_ROUTE_index, Value::Int(index)),
                Property::new(sys::SPA_PARAM_ROUTE_direction, Value::Id(Id(sys::SPA_DIRECTION_OUTPUT))),
                Property::new(sys::SPA_PARAM_ROUTE_name, Value::String("analog-output-speaker".into())),
                Property::new(sys::SPA_PARAM_ROUTE_description, Value::String("Hoparlör".into())),
                Property::new(sys::SPA_PARAM_ROUTE_priority, Value::Int(100)),
                Property::new(sys::SPA_PARAM_ROUTE_available, Value::Id(Id(2))),
                Property {
                    key: sys::SPA_PARAM_ROUTE_info,
                    flags: PropertyFlags::HINT_DICT,
                    value: Value::Struct(vec![
                        Value::Int(3),
                        Value::String("port.type".into()),
                        Value::String("speaker".into()),
                        Value::String("route.hw-mute".into()),
                        Value::String("true".into()),
                        Value::String("route.hw-volume".into()),
                        Value::String("true".into()),
                    ]),
                },
                Property::new(sys::SPA_PARAM_ROUTE_profiles, Value::ValueArray(ValueArray::Int(vec![1, 2]))),
                Property::new(sys::SPA_PARAM_ROUTE_device, Value::Int(device)),
                Property::new(sys::SPA_PARAM_ROUTE_props, Value::Object(props)),
                Property::new(sys::SPA_PARAM_ROUTE_devices, Value::ValueArray(ValueArray::Int(vec![device]))),
                Property::new(sys::SPA_PARAM_ROUTE_profile, Value::Int(1)),
                Property::new(sys::SPA_PARAM_ROUTE_save, Value::Bool(false)),
            ],
        }
    }

    /// Kart düğümü (sink 10 → cihaz 9, profil cihazı 1).
    fn card_model() -> Model {
        let mut m = Model::default();
        let sink = info(&[("node.name", "spk"), ("device.id", "9"), ("card.profile.device", "1")]);
        m.add_node(10, NodeEntry::new(NodeKind::Sink, sink));
        m.default_sink = Some("spk".into());
        m
    }

    #[test]
    fn perceptual_scale() {
        assert!((to_perceptual(0.125) - 0.5).abs() < 1e-6);
        assert!((to_linear(0.5) - 0.125).abs() < 1e-6);
        assert_eq!(to_linear(f32::NAN), 0.0);
        assert_eq!(to_perceptual(f32::INFINITY), 0.0);
        let c = scale_channels(&[0.5, 0.25], 0.5);
        assert!((c[0] - 0.125).abs() < 1e-6 && (c[1] - 0.0625).abs() < 1e-6, "denge korunur");
        assert_eq!(scale_channels(&[0.0, 0.0], 1.0), [1.0, 1.0]);
        assert_eq!(scale_channels(&[1.0, 1.0], 1.0), [1.0, 1.0], "%100 = 0 dB = doğrusal 1.0");
    }

    #[test]
    fn props_roundtrip() {
        let w = VolumeWrite { channels: Some(vec![0.25, 0.5]), mute: Some(true) };
        let p = VolumeProps::parse(&roundtrip(w.props())).unwrap();
        assert_eq!(p.channels, Some(vec![0.25, 0.5]));
        assert_eq!(p.mute, Some(true));
        let only_mute = VolumeWrite { channels: None, mute: Some(false) };
        assert_eq!(VolumeProps::parse(&roundtrip(only_mute.props())).unwrap().channels, None);
        // Route içindeki Props da aynı biçimde; mixer-api'nin yazdığı alanlar.
        let r = route(3, 7, None);
        let v = roundtrip(w.route(&r));
        let Value::Object(o) = &v else { panic!() };
        assert_eq!(o.type_, sys::SPA_TYPE_OBJECT_ParamRoute);
        let keys: Vec<u32> = o.properties.iter().map(|p| p.key).collect();
        assert_eq!(
            keys,
            [sys::SPA_PARAM_ROUTE_index, sys::SPA_PARAM_ROUTE_device, sys::SPA_PARAM_ROUTE_props, sys::SPA_PARAM_ROUTE_save]
        );
        let inner = o.properties.iter().find(|p| p.key == sys::SPA_PARAM_ROUTE_props).unwrap();
        assert_eq!(VolumeProps::parse(&inner.value).unwrap().mute, Some(true));
    }

    #[test]
    fn hostile_props_are_bounded() {
        let o = Object {
            type_: sys::SPA_TYPE_OBJECT_Props,
            id: sys::SPA_PARAM_Props,
            properties: vec![Property::new(
                sys::SPA_PROP_channelVolumes,
                Value::ValueArray(ValueArray::Float(vec![1.0; MAX_CHANNELS + 1])),
            )],
        };
        assert_eq!(VolumeProps::parse(&Value::Object(o)).unwrap().channels, None);
        let o = Object {
            type_: sys::SPA_TYPE_OBJECT_Props,
            id: sys::SPA_PARAM_Props,
            properties: vec![Property::new(
                sys::SPA_PROP_channelVolumes,
                Value::ValueArray(ValueArray::Float(vec![f32::NAN, 1e9])),
            )],
        };
        assert_eq!(VolumeProps::parse(&Value::Object(o)).unwrap().channels, Some(vec![0.0, 16.0]));
        assert_eq!(VolumeProps::parse(&Value::Int(1)), None);
    }

    #[test]
    fn acp_route_parses_with_props_and_flags() {
        let r = Route::parse(&roundtrip(acp_route(2, 5, &[0.4, 0.2]))).unwrap();
        assert_eq!((r.index, r.direction, r.device), (2, sys::SPA_DIRECTION_OUTPUT, 5));
        assert_eq!(r.port_type.as_deref(), Some("speaker"));
        let p = r.props.unwrap();
        assert_eq!(p.channels, Some(vec![0.4, 0.2]));
        assert_eq!(p.soft, Some(vec![1.0, 1.0]));
        assert!(p.hw_volume && p.hw_mute);
        assert_eq!(p.base, Some(1.0));
        // Profil cihazı olmayan (etkin olmayan) route kullanılamaz.
        let mut o = acp_route(2, 5, &[1.0]);
        o.properties.retain(|p| p.key != sys::SPA_PARAM_ROUTE_device);
        assert_eq!(Route::parse(&roundtrip(o)), None);
    }

    #[test]
    fn node_props_merge_keeps_volume_from_audioconvert() {
        // Düğüm iki Props yayınlar: seviyeli (audioconvert) ve seviyesiz (sürücü).
        let mut m = card_model();
        assert!(m.set_props(10, VolumeProps { soft: Some(vec![0.5]), ..vol(&[0.125, 0.125], false) }));
        assert!(!m.set_props(10, VolumeProps::default()), "seviyesiz Props bir şey silmez");
        assert_eq!(m.nodes[&10].props.channels, Some(vec![0.125, 0.125]));
        assert_eq!(m.nodes[&10].props.soft, Some(vec![0.5]));
    }

    #[test]
    fn node_info_sanitized() {
        let i = info(&[
            ("node.name", "alsa_output.pci"),
            ("node.nick", "Hoparlör\u{7}"),
            ("application.icon-name", "../../etc/passwd"),
            ("device.id", "42"),
            ("card.profile.device", "x"),
        ]);
        assert_eq!(i.description, "Hoparlör");
        assert_eq!(i.icon, None);
        assert_eq!(i.device_id, Some(42));
        assert_eq!(i.profile_device, None);
        assert_eq!(info(&[("node.name", "n")]).description, "n");
    }

    #[test]
    fn device_class_from_bluetooth_port_form_factor_and_name() {
        use DeviceClass as C;
        let c = |sink, props: &[(&str, &str)], port| DeviceClass::classify(sink, &info(props), port);
        let bt = |ff| c(true, &[("node.name", "bluez_output.x"), ("device.api", "bluez5"), ("device.form-factor", ff)], None);
        assert_eq!(bt("headphone"), C::Headphones);
        assert_eq!(bt("headset"), C::Headset);
        assert_eq!(bt("speaker"), C::Bluetooth);
        // Etkin route: aynı kart düğümü kulaklık takılınca kulaklık olur.
        let card = [("node.name", "alsa_output.pci.analog-stereo"), ("device.form-factor", "internal")];
        assert_eq!(c(true, &card, Some("speaker")), C::Speaker);
        assert_eq!(c(true, &card, Some("headphones")), C::Headphones);
        assert_eq!(c(true, &[("node.name", "alsa_output.pci.hdmi-stereo")], None), C::Display);
        assert_eq!(c(true, &[("node.name", "x")], Some("hdmi")), C::Display);
        assert_eq!(c(false, &[("node.name", "cam"), ("device.form-factor", "webcam")], None), C::Webcam);
        assert_eq!(c(false, &[("node.name", "u"), ("device.bus", "usb")], None), C::Usb);
        assert_eq!(c(false, &card, None), C::Microphone);
        assert_eq!(c(true, &[("node.name", "x")], None), C::Speaker);
        // Geçersiz ipucu yok sayılır.
        assert_eq!(info(&[("device.form-factor", "head phone")]).form_factor, None);
        assert_eq!(info(&[("device.api", "BlueZ5")]).api.as_deref(), Some("bluez5"));
    }

    #[test]
    fn default_metadata() {
        assert_eq!(parse_default_name(Some("Spa:String:JSON"), Some(r#"{"name":"a.b"}"#)).as_deref(), Some("a.b"));
        assert_eq!(parse_default_name(None, Some(r#"{"name":"a"}"#)).as_deref(), Some("a"));
        assert_eq!(parse_default_name(Some("Spa:Id"), Some(r#"{"name":"a"}"#)), None);
        assert_eq!(parse_default_name(None, Some("bozuk")), None);
        assert_eq!(parse_default_name(None, None), None);
        assert_eq!(parse_default_name(None, Some(&format!(r#"{{"name":"{}"}}"#, "x".repeat(2000)))), None);
        assert_eq!(parse_default_name(None, Some(&default_value("a\"b"))).as_deref(), Some("a\"b"));
    }

    #[test]
    fn state_sorting_streams_and_props_path() {
        let mut m = Model::default();
        m.add_node(11, NodeEntry::new(NodeKind::Sink, info(&[("node.name", "hdmi"), ("node.description", "HDMI")])));
        m.add_node(10, NodeEntry::new(NodeKind::Sink, info(&[("node.name", "spk"), ("node.description", "Hoparlör")])));
        m.add_node(12, NodeEntry::new(NodeKind::Playback, info(&[("node.name", "x"), ("application.name", "Firefox")])));
        m.add_node(13, NodeEntry::new(NodeKind::Recording, info(&[("node.name", "peak"), ("stream.monitor", "true")])));
        assert!(m.set_props(10, vol(&[0.125, 0.125], false)));
        assert!(!m.set_props(10, vol(&[0.125, 0.125], false)));
        m.default_sink = Some("spk".into());
        let s = m.state(true);
        assert_eq!(s.sinks.iter().map(|d| d.description.as_str()).collect::<Vec<_>>(), ["HDMI", "Hoparlör"]);
        let d = s.default_sink().unwrap();
        assert_eq!(d.id, 10);
        assert!((d.volume - 0.5).abs() < 1e-6);
        assert_eq!(s.playback[0].app, "Firefox");
        assert!(!s.playback[0].running, "durum gelene dek akmıyor sayılır");
        assert!(s.recording.is_empty(), "tepe ölçer akışı gizli");
        assert!(m.set_running(12, true) && !m.set_running(12, true) && !m.set_running(99, true));
        assert!(m.state(true).playback[0].running);
        assert_eq!(m.path(10), Some(VolumePath::Props(PropsReason::NoDeviceId)));
        assert_eq!(m.path(12), Some(VolumePath::Props(PropsReason::Stream)));
        assert_eq!(m.path(99), None);
    }

    #[test]
    fn card_node_waits_for_routes_instead_of_falling_back_to_props() {
        // Eski yolun kusuru: route bilinmeyince düğüm Props'una yazılıyordu (yalnızca yazılım
        // seviyesi; donanım karıştırıcısı yerinde kalır).
        let mut m = card_model();
        let mut no_profile = info(&[("node.name", "x"), ("device.id", "9")]);
        no_profile.profile_device = None;
        m.add_node(20, NodeEntry::new(NodeKind::Sink, no_profile));
        assert_eq!(m.path(10), Some(VolumePath::Props(PropsReason::DeviceNotTracked)), "cihaz izlenmiyor");
        assert_eq!(m.path(20), Some(VolumePath::Props(PropsReason::NoProfileDevice)));
        m.add_device(9);
        assert_eq!(m.path(10), Some(VolumePath::AwaitRoutes { device: 9 }), "route listesi henüz okunmadı");
        assert!(m.begin_routes(9, 1));
        assert_eq!(m.path(10), Some(VolumePath::AwaitRoutes { device: 9 }), "numaralandırma sürüyor");
        assert!(m.add_route(9, 1, route(4, 1, Some(vol(&[0.4, 0.4], false)))));
        assert!(m.commit_routes(9, 1));
        let Some(VolumePath::Route { device: 9, route: r }) = m.path(10) else { panic!("{:?}", m.path(10)) };
        assert_eq!(r.index, 4);
    }

    #[test]
    fn volume_is_read_from_route_not_from_node_props() {
        // Düğüm Props'u route'un gecikmeli yansımasıdır; mixer-api/pulse route'u okur.
        let mut m = card_model();
        m.add_device(9);
        m.set_props(10, vol(&[0.125, 0.125], false));
        m.begin_routes(9, 1);
        m.add_route(9, 1, route(4, 1, Some(vol(&[1.0, 1.0], true))));
        m.commit_routes(9, 1);
        assert_eq!(m.volume(10), Some((1.0, true)));
        // Seviyesiz route (yalnızca port) → Props.
        m.begin_routes(9, 2);
        m.add_route(9, 2, route(4, 1, None));
        m.commit_routes(9, 2);
        assert_eq!(m.path(10), Some(VolumePath::Props(PropsReason::RouteWithoutVolume)));
        assert!((m.volume(10).unwrap().0 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn routes_replaced_per_enumeration_and_stale_ones_dropped() {
        let mut m = card_model();
        m.add_device(9);
        m.begin_routes(9, 1);
        m.add_route(9, 1, route(4, 1, Some(vol(&[0.5], false))));
        m.add_route(9, 1, Route { direction: sys::SPA_DIRECTION_INPUT, ..route(7, 2, Some(vol(&[0.3], false))) });
        assert!(m.commit_routes(9, 1));
        assert_eq!(m.devices[&9].routes.len(), 2);
        // Port değişimi (kulaklık takıldı): aynı profil cihazı, yeni index. Eski index'e yazmak
        // ACP'de portu geri değiştirirdi.
        m.begin_routes(9, 2);
        m.add_route(9, 2, route(5, 1, Some(vol(&[0.2], false))));
        assert!(m.commit_routes(9, 2));
        assert_eq!(m.devices[&9].routes.len(), 1, "kaybolan route düşer");
        let Some(VolumePath::Route { route: r, .. }) = m.path(10) else { panic!() };
        assert_eq!(r.index, 5);
        // Aynı liste: değişiklik yok.
        m.begin_routes(9, 3);
        m.add_route(9, 3, route(5, 1, Some(vol(&[0.2], false))));
        assert!(!m.commit_routes(9, 3));
        assert_eq!(m.devices[&9].enumerations, 3);
        // Profil değişimi: bu profil cihazının route'u kalmadı → Props (kesin).
        m.begin_routes(9, 4);
        m.add_route(9, 4, route(1, 3, Some(vol(&[1.0], false))));
        m.commit_routes(9, 4);
        assert_eq!(m.path(10), Some(VolumePath::Props(PropsReason::NoRoute)));
    }

    #[test]
    fn overlapping_enumerations_only_latest_counts() {
        let mut m = card_model();
        m.add_device(9);
        m.begin_routes(9, 1);
        m.add_route(9, 1, route(4, 1, Some(vol(&[0.5], false))));
        // Yeni bilgi geldi, yeni numaralandırma: eskisinin geç gelen parçaları ve bitişi yok sayılır.
        m.begin_routes(9, 2);
        assert!(!m.add_route(9, 1, route(9, 1, Some(vol(&[0.9], false)))));
        assert!(!m.commit_routes(9, 1));
        assert_eq!(m.path(10), Some(VolumePath::AwaitRoutes { device: 9 }));
        m.add_route(9, 2, route(6, 1, Some(vol(&[0.7], false))));
        assert!(m.commit_routes(9, 2));
        let Some(VolumePath::Route { route: r, .. }) = m.path(10) else { panic!() };
        assert_eq!(r.index, 6);
        // Kaldırılan cihaz: route bilgisi gider.
        assert!(m.remove(9));
        assert_eq!(m.path(10), Some(VolumePath::Props(PropsReason::DeviceNotTracked)));
    }

    #[test]
    fn route_limit() {
        let mut m = card_model();
        m.add_device(9);
        m.begin_routes(9, 1);
        for i in 0..MAX_ROUTES as i32 + 5 {
            m.add_route(9, 1, route(i, i, None));
        }
        m.commit_routes(9, 1);
        assert_eq!(m.devices[&9].routes.len(), MAX_ROUTES);
    }

    #[test]
    fn node_limit() {
        let mut m = Model::default();
        for i in 0..MAX_NODES as u32 + 5 {
            m.add_node(i, NodeEntry::new(NodeKind::Playback, NodeInfo::default()));
        }
        assert_eq!(m.nodes.len(), MAX_NODES);
    }
}
