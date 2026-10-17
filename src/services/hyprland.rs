//! Hyprland IPC servisi.
//!
//! - **socket1** (`.socket.sock`): istek başına bir bağlantı. Komut yazılır, cevap EOF'a kadar
//!   okunur (Hyprland cevaptan sonra bağlantıyı kapatır). Okuma/yazma zaman aşımı ve cevap
//!   boyut sınırı vardır; kilitlenen compositor shell'i kilitleyemez.
//! - **socket2** (`.socket2.sock`): `EVENT>>DATA\n` satır akışı. Olaylar yalnızca "durum
//!   değişti" sinyalidir; asıl değer her zaman socket1'den yeniden sorgulanan anlık
//!   görüntüdür ([`HyprSnapshot`]). Olay verisini ayrıştırmak (virgül içeren başlıklar,
//!   sürümler arası format farkı) kırılgan olduğu için yapılmaz.
//!
//! Olay patlamaları (bir workspace geçişi ~4-8 olay üretir) [`Coalescer`] ile birleştirilir:
//! boştayken gelen ilk olay **gecikmesiz** yenileme yapar, ardından gelenler kısa bir sessizlik
//! penceresi boyunca toplanır ve tek yenilemeye dönüşür; sürekli akışta yenileme
//! `max_delay` ile sınırlanır. Aynı anlık görüntü iki kez yayınlanmaz. Bir sorgu başarısız
//! olursa (zaman aşımı, kopuk cevap) bekleyen değişiklik kaybolmaz: artan beklemeyle
//! (`query_retry_min` → `query_retry_max`) yeniden sorgulanır, başarıda bekleme sıfırlanır.
//!
//! Thread'ler: `reader` socket2'yi okur ve kopunca artan beklemeyle yeniden bağlanır;
//! `refresher` sorguları yapar ve callback'i çağırır. [`HyprlandService`] düşürülünce
//! bloklayan okuma soket `shutdown` ile kesilir, iki thread de join edilir.

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::DeserializeOwned;

/// socket1 cevabının kabul edilen en büyük boyutu.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// socket2 olay satırının en büyük boyutu; uzun satır atlanır (pencere başlıkları uzun olabilir).
pub const MAX_EVENT_LINE_BYTES: usize = 64 * 1024;
/// Modeldeki metin alanlarının (başlık, sınıf, ad) en fazla karakter sayısı.
pub const MAX_TEXT_CHARS: usize = 512;
pub const MAX_MONITORS: usize = 16;
pub const MAX_WORKSPACES: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum HyprError {
    #[error("HYPRLAND_INSTANCE_SIGNATURE tanımlı değil (Hyprland altında çalışılmıyor)")]
    NoSignature,
    #[error("HYPRLAND_INSTANCE_SIGNATURE geçersiz karakter içeriyor")]
    BadSignature,
    #[error("XDG_RUNTIME_DIR tanımlı değil veya mutlak yol değil")]
    NoRuntimeDir,
    #[error("Hyprland IPC: {0}")]
    Io(#[from] io::Error),
    #[error("Hyprland cevabı çok büyük (sınır {MAX_RESPONSE_BYTES} bayt)")]
    TooLarge,
    // serde_json hatası hatalı değeri (pencere başlığı olabilir) içerebilir: yalnızca tür ve konum.
    #[error("Hyprland `{command}` cevabı ayrıştırılamadı: {}", json_error_summary(.source))]
    Json {
        command: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("Hyprland komutu reddetti: {0}")]
    Rejected(String),
    #[error("geçersiz workspace hedefi")]
    InvalidTarget,
}

/// serde_json hatasının içeriksiz özeti (tür, satır, sütun). Hata mesajı hatalı değeri
/// (ör. pencere başlığı) içerebileceği için loglara yalnızca bu yazılır.
pub fn json_error_summary(e: &serde_json::Error) -> String {
    format!("{:?}, satır {} sütun {}", e.classify(), e.line(), e.column()) // i18n: tanı ayrıştırma hatası biçimi (log/Display)
}

// ---------------------------------------------------------------------------------------
// Yol çözümü

/// İki soketin yolu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyprlandPaths {
    pub socket1: PathBuf,
    pub socket2: PathBuf,
}

impl HyprlandPaths {
    /// `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE`.
    pub fn from_env() -> Result<Self, HyprError> {
        Self::resolve(std::env::var_os("XDG_RUNTIME_DIR"), std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE"))
    }

    /// Ortam değişkeni değerlerinden çözer. İmza dizin ayırıcı veya `..` ile yol dışına
    /// çıkamaz; yalnızca `[A-Za-z0-9_-]` kabul edilir.
    pub fn resolve(runtime_dir: Option<OsString>, signature: Option<OsString>) -> Result<Self, HyprError> {
        let signature = signature.filter(|s| !s.is_empty()).ok_or(HyprError::NoSignature)?;
        let signature = signature.to_str().ok_or(HyprError::BadSignature)?;
        if signature.len() > 255 || !signature.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err(HyprError::BadSignature);
        }
        let runtime_dir = runtime_dir.map(PathBuf::from).filter(|p| p.is_absolute()).ok_or(HyprError::NoRuntimeDir)?;
        Ok(Self::in_instance_dir(&runtime_dir.join("hypr").join(signature)))
    }

    pub fn in_instance_dir(dir: &Path) -> Self {
        Self { socket1: dir.join(".socket.sock"), socket2: dir.join(".socket2.sock") }
    }
}

/// `hyprland.lock` okuma sınırı (iki kısa satır).
const MAX_LOCK_FILE: u64 = 4096;

/// `hyprland.lock`'un ilk satırı: compositor PID'i (1 ve 0 reddedilir).
pub fn parse_lock_pid(text: &str) -> Option<u32> {
    text.lines().next()?.trim().parse().ok().filter(|p| *p > 1)
}

/// Örnek dizinindeki `hyprland.lock`'tan compositor PID'i.
pub fn compositor_pid_in(instance_dir: &Path) -> Option<u32> {
    let mut s = String::new();
    std::fs::File::open(instance_dir.join("hyprland.lock")).ok()?.take(MAX_LOCK_FILE).read_to_string(&mut s).ok()?;
    parse_lock_pid(&s)
}

/// Ortamdaki Hyprland örneğinin compositor PID'i. Kabuk systemd kullanıcı servisi olarak oturum
/// kapsamı dışında çalışır; logind oturumu compositor'ın PID'inden kesin çözülür.
pub fn compositor_pid() -> Option<u32> {
    HyprlandPaths::from_env().ok()?.socket1.parent().and_then(compositor_pid_in)
}

// ---------------------------------------------------------------------------------------
// Veri tipleri (toleranslı: bilinmeyen alanlar yok sayılır, eksikler varsayılan)

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct WorkspaceRef {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    #[serde(rename = "activeWorkspace")]
    pub active_workspace: WorkspaceRef,
    /// `id == 0` ve boş ad: special workspace açık değil. [`Monitor::special`] kullan.
    #[serde(rename = "specialWorkspace")]
    pub special_workspace: WorkspaceRef,
    pub focused: bool,
    pub disabled: bool,
}

impl Monitor {
    /// Bu monitörde açık special workspace.
    pub fn special(&self) -> Option<&WorkspaceRef> {
        let s = &self.special_workspace;
        (s.id != 0 || !s.name.is_empty()).then_some(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    pub monitor: String,
    #[serde(rename = "monitorID")]
    pub monitor_id: Option<i64>,
    pub windows: u32,
    #[serde(rename = "hasfullscreen")]
    pub has_fullscreen: bool,
    /// Bu workspace'teki pencerelerin en yüksek tam ekran kipi (`j/clients`'tan):
    /// 0 yok, 1 maximize (rezerve alana saygılı), 2 gerçek tam ekran.
    #[serde(skip)]
    pub fullscreen_mode: u8,
    /// Temsilci pencerenin sınıfı (`j/clients`'tan; workspace göstergesinin uygulama ikonu):
    /// en büyük alanlı eşlenmiş, gizli olmayan pencere, eşitlikte en son odaklanan. Boş = yok.
    #[serde(skip)]
    pub app_class: String,
}

impl Workspace {
    /// Gerçek tam ekran pencere var mı (maximize sayılmaz).
    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen_mode >= 2
    }

    /// Special workspace: `special` / `special:<ad>` adlı ya da Hyprland'in special id
    /// aralığında (−99..=−2). Adlı workspace'ler (`name:foo`) de negatif id alır (≤ −1337) ama
    /// special değildir.
    pub fn is_special(&self) -> bool {
        is_special_workspace(self.id, &self.name)
    }
}

/// Bkz. [`Workspace::is_special`]; `WorkspaceRef` için de kullanılır.
pub fn is_special_workspace(id: i64, name: &str) -> bool {
    name == "special" || name.starts_with("special:") || (-99..=-2).contains(&id)
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Window {
    pub address: String,
    pub class: String,
    pub title: String,
    pub workspace: WorkspaceRef,
}

/// `j/clients` girdisinden gereken alanlar: tam ekran kipi ve workspace'in temsilci penceresi.
/// Yeni alanlar toleranslıdır: tür uyuşmazlığı tüm listeyi düşürmez.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ClientInfo {
    workspace: WorkspaceRef,
    #[serde(deserialize_with = "fullscreen_mode")]
    fullscreen: u8,
    #[serde(deserialize_with = "lenient")]
    class: String,
    #[serde(rename = "initialClass", deserialize_with = "lenient")]
    initial_class: String,
    #[serde(deserialize_with = "lenient")]
    size: [i64; 2],
    #[serde(rename = "focusHistoryID", deserialize_with = "lenient")]
    focus_history_id: i64,
    /// Eski sürümlerde alan yok: yalnızca açıkça `false` ise atlanır.
    #[serde(deserialize_with = "lenient")]
    mapped: Option<bool>,
    #[serde(deserialize_with = "lenient")]
    hidden: bool,
}

impl ClientInfo {
    /// Temsilci seçiminde sayılır mı (görünür, eşlenmiş, sınıfı olan pencere).
    fn is_candidate(&self) -> bool {
        self.mapped != Some(false) && !self.hidden && !self.app_class().is_empty()
    }

    fn app_class(&self) -> &str {
        if self.class.is_empty() { &self.initial_class } else { &self.class }
    }

    fn area(&self) -> i64 {
        self.size[0].clamp(0, 1 << 20) * self.size[1].clamp(0, 1 << 20)
    }

    /// Temsilci sıralama anahtarı: büyük alan, eşitlikte en son odaklanan (küçük geçmiş id'si).
    fn rank(&self) -> (i64, std::cmp::Reverse<i64>) {
        (self.area(), std::cmp::Reverse(self.focus_history_id))
    }
}

/// Tür uyuşmazlığında varsayılan değer.
fn lenient<'de, D: serde::Deserializer<'de>, T: DeserializeOwned + Default>(de: D) -> Result<T, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(de)?).unwrap_or_default())
}

/// Hyprland ≥ 0.42: `fullscreen` 0/1/2 sayısı; daha eski sürümlerde bool (true = tam ekran).
fn fullscreen_mode<'de, D: serde::Deserializer<'de>>(de: D) -> Result<u8, D::Error> {
    Ok(match serde_json::Value::deserialize(de)? {
        serde_json::Value::Number(n) => n.as_u64().map_or(0, |v| v.min(2) as u8),
        serde_json::Value::Bool(true) => 2,
        _ => 0,
    })
}

/// Hyprland durumunun anlık görüntüsü. Workspace'ler id'ye göre sıralıdır.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HyprSnapshot {
    pub monitors: Vec<Monitor>,
    pub workspaces: Vec<Workspace>,
    /// Odaklı pencere; yoksa (boş workspace, layer odakta) `None`.
    pub active_window: Option<Window>,
}

impl HyprSnapshot {
    pub fn focused_monitor(&self) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.focused)
    }

    pub fn monitor(&self, name: &str) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.name == name)
    }

    pub fn workspace(&self, id: i64) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    /// Dış girdiyi sınırlar ve kararlı sıraya sokar (karşılaştırma ve çizim için).
    fn normalize(mut self) -> Self {
        if self.monitors.len() > MAX_MONITORS || self.workspaces.len() > MAX_WORKSPACES {
            tracing::warn!(
                monitors = self.monitors.len(),
                workspaces = self.workspaces.len(),
                "Hyprland beklenmedik sayıda monitör/workspace bildirdi, kırpılıyor"
            );
        }
        self.monitors.truncate(MAX_MONITORS);
        self.monitors.sort_by_key(|m| m.id);
        self.workspaces.sort_by_key(|w| w.id);
        self.workspaces.truncate(MAX_WORKSPACES);
        for m in &mut self.monitors {
            clip_text(&mut m.name);
            clip_text(&mut m.description);
            clip_text(&mut m.active_workspace.name);
            clip_text(&mut m.special_workspace.name);
        }
        for w in &mut self.workspaces {
            clip_text(&mut w.name);
            clip_text(&mut w.monitor);
            clip_text(&mut w.app_class);
        }
        if let Some(w) = &mut self.active_window {
            clip_text(&mut w.class);
            clip_text(&mut w.title);
            clip_text(&mut w.address);
            clip_text(&mut w.workspace.name);
        }
        self
    }
}

fn clip_text(s: &mut String) {
    if let Some((i, _)) = s.char_indices().nth(MAX_TEXT_CHARS) {
        s.truncate(i);
    }
}

// ---------------------------------------------------------------------------------------
// socket1 istemcisi

/// `dispatch workspace` hedefi. Serbest metin kabul edilmez (komut enjeksiyonu yok).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceTarget {
    /// Mutlak workspace id'si (1 ve üzeri).
    Id(u32),
    /// Bu monitördeki açık workspace'ler arasında göreli (`e+1`, `e-1`).
    Relative(i32),
}

impl WorkspaceTarget {
    fn to_arg(self) -> Result<String, HyprError> {
        match self {
            WorkspaceTarget::Id(0) | WorkspaceTarget::Relative(0) => Err(HyprError::InvalidTarget),
            WorkspaceTarget::Id(id) => Ok(id.to_string()),
            WorkspaceTarget::Relative(d) => Ok(format!("e{d:+}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HyprlandClient {
    socket1: PathBuf,
    timeout: Duration,
}

impl HyprlandClient {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);

    pub fn new(socket1: PathBuf) -> Self {
        Self { socket1, timeout: Self::DEFAULT_TIMEOUT }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Ham istek. Yalnızca crate içinden sabit komutlarla çağrılır.
    /// Süre aşımı (`SO_RCVTIMEO`/`SO_SNDTIMEO`) Linux'ta `WouldBlock` (EAGAIN) olarak döner;
    /// `TimedOut`a çevrilir ki log "Resource temporarily unavailable" yerine zaman aşımı desin.
    pub(crate) fn request(&self, command: &str) -> Result<String, HyprError> {
        let mut stream = UnixStream::connect(&self.socket1)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;
        let timed_out = |e: io::Error| {
            if e.kind() == io::ErrorKind::WouldBlock {
                io::Error::from(io::ErrorKind::TimedOut)
            } else {
                e
            }
        };
        stream.write_all(command.as_bytes()).map_err(timed_out)?;
        let mut buf = Vec::new();
        (&mut stream).take(MAX_RESPONSE_BYTES as u64 + 1).read_to_end(&mut buf).map_err(timed_out)?;
        if buf.len() > MAX_RESPONSE_BYTES {
            return Err(HyprError::TooLarge);
        }
        Ok(match String::from_utf8(buf) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        })
    }

    fn json<T: DeserializeOwned>(&self, command: &'static str) -> Result<T, HyprError> {
        let text = self.request(command)?;
        serde_json::from_str(&text).map_err(|source| HyprError::Json { command, source })
    }

    pub fn monitors(&self) -> Result<Vec<Monitor>, HyprError> {
        self.json("j/monitors")
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>, HyprError> {
        self.json("j/workspaces")
    }

    pub fn active_workspace(&self) -> Result<Workspace, HyprError> {
        self.json("j/activeworkspace")
    }

    /// Odaklı pencere; Hyprland `{}` döndürürse `None`.
    pub fn active_window(&self) -> Result<Option<Window>, HyprError> {
        let w: Window = self.json("j/activewindow")?;
        Ok((!w.address.is_empty()).then_some(w))
    }

    /// Tüm durumu tek `[[BATCH]]` isteğiyle sorgular: meşgul compositor (ör. workspace
    /// animasyonu sırasında) her isteği bir kare kadar bekletebilir, tek tur gecikmeyi
    /// üçte birine indirir. Cevaplar art arda JSON değerleridir; ayırıcıya güvenmeden akış
    /// halinde ayrıştırılır. Monitörler ve workspace'ler zorunlu; odaklı pencere
    /// ayrıştırılamazsa `None` kabul edilir.
    pub fn snapshot(&self) -> Result<HyprSnapshot, HyprError> {
        const COMMAND: &str = SNAPSHOT_COMMAND;
        let text = self.request(COMMAND)?;
        parse_snapshot(&text)
    }

    pub fn dispatch_workspace(&self, target: WorkspaceTarget) -> Result<(), HyprError> {
        let reply = self.request(&format!("dispatch workspace {}", target.to_arg()?))?;
        expect_ok(reply)
    }

    /// Tüm monitörlerin gücü: `dispatch dpms on|off` (boşta kalma ekranı kapatırken). Hyprland
    /// girdiyle ekranı kendiliğinden açmaz (`misc:*_enables_dpms` varsayılan kapalı); açmak da
    /// çağıranın işi.
    pub fn dispatch_dpms(&self, on: bool) -> Result<(), HyprError> {
        expect_ok(self.request(if on { "dispatch dpms on" } else { "dispatch dpms off" })?)
    }

    /// Ana klavyenin düzenleri (`j/devices`); yanıt bozuksa `Ok(None)`.
    pub fn keyboard_layouts(&self) -> Result<Option<crate::services::keyboard_layouts::KeyboardLayouts>, HyprError> {
        Ok(crate::services::keyboard_layouts::parse_devices(&self.request("j/devices")?))
    }

    /// Ana klavyede sıradaki düzene geç (`switchxkblayout current next`).
    pub fn switch_layout_next(&self) -> Result<(), HyprError> {
        expect_ok(self.request("dispatch switchxkblayout current next")?)
    }

    /// Sabit bir `dispatch …` komutu (crate içi; cevap `ok` değilse ret).
    pub(crate) fn dispatch_raw(&self, command: &'static str) -> Result<(), HyprError> {
        expect_ok(self.request(command)?)
    }

    /// Ayar sağlayıcısı (`j/status` → `configProvider`: `hyprlang` | `lua`). Lua kipinde
    /// dispatcher'lar `hl.dsp.*` ifadeleridir.
    pub fn config_provider(&self) -> Result<String, HyprError> {
        #[derive(Deserialize)]
        struct Status {
            #[serde(rename = "configProvider", default)]
            config_provider: String,
        }
        let s: Status = self.json("j/status")?;
        let mut p = s.config_provider;
        clip_text(&mut p);
        Ok(p)
    }

    /// Örnek dizini (`$XDG_RUNTIME_DIR/hypr/<imza>`: soketler ve `hyprland.lock`).
    pub fn instance_dir(&self) -> Option<&Path> {
        self.socket1.parent()
    }
}

/// Komut cevabı `ok` değilse (kısaltılmış) ret.
fn expect_ok(reply: String) -> Result<(), HyprError> {
    if reply.trim() == "ok" {
        Ok(())
    } else {
        let mut reply = reply.trim().to_owned();
        clip_text(&mut reply);
        Err(HyprError::Rejected(reply))
    }
}

/// Tek turda tüm durum. `j/clients` yalnızca tam ekran kipi için.
pub const SNAPSHOT_COMMAND: &str = "[[BATCH]]j/monitors;j/workspaces;j/activewindow;j/clients";

/// [`SNAPSHOT_COMMAND`] cevabını ayrıştırır.
fn parse_snapshot(text: &str) -> Result<HyprSnapshot, HyprError> {
    let json = |command| move |source| HyprError::Json { command, source };
    let mut de = serde_json::Deserializer::from_str(text);
    let monitors = Vec::<Monitor>::deserialize(&mut de).map_err(json("j/monitors"))?;
    let mut workspaces = Vec::<Workspace>::deserialize(&mut de).map_err(json("j/workspaces"))?;
    let active_window = match Window::deserialize(&mut de) {
        Ok(w) => (!w.address.is_empty()).then_some(w),
        Err(e) => {
            tracing::warn!("odaklı pencere ayrıştırılamadı: {}", json_error_summary(&e));
            None
        }
    };
    // Pencere listesi ayrıştırılamazsa tam ekran kipi bilinmez: 0 kabul edilir (çerçeve
    // gizlenmez — yanlışlıkla bar'ı kaybetmekten iyidir).
    match Vec::<ClientInfo>::deserialize(&mut de) {
        Ok(clients) => {
            // Workspace başına seçilen temsilcinin sıralama anahtarı (`workspaces` ile aynı dizin).
            let mut best: Vec<Option<(i64, std::cmp::Reverse<i64>)>> = vec![None; workspaces.len()];
            for c in clients {
                if let Some(i) = workspaces.iter().position(|w| w.id == c.workspace.id) {
                    let w = &mut workspaces[i];
                    w.fullscreen_mode = w.fullscreen_mode.max(c.fullscreen);
                    if c.is_candidate() && best[i].is_none_or(|b| c.rank() > b) {
                        best[i] = Some(c.rank());
                        w.app_class = c.app_class().to_owned();
                    }
                }
            }
        }
        Err(e) => tracing::warn!("pencere listesi ayrıştırılamadı, tam ekran kipi bilinmiyor: {}", json_error_summary(&e)),
    }
    Ok(HyprSnapshot { monitors, workspaces, active_window }.normalize())
}

// ---------------------------------------------------------------------------------------
// socket2 olayları

/// Durumu etkileyen (yeniden sorgu gerektiren) olay mı?
pub fn is_relevant_event(name: &str) -> bool {
    let base = name.strip_suffix("v2").unwrap_or(name);
    matches!(
        base,
        "workspace"
            | "focusedmon"
            | "activewindow"
            | "activespecial"
            | "createworkspace"
            | "destroyworkspace"
            | "moveworkspace"
            | "renameworkspace"
            | "monitoradded"
            | "monitorremoved"
            | "openwindow"
            | "closewindow"
            | "movewindow"
            | "windowtitle"
            | "fullscreen"
            | "configreloaded"
    )
}

/// Klavye ve düzen adının en uzun hâli (xkb açıklamaları ~60 karakter).
const MAX_LAYOUT_NAME: usize = 128;

fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_LAYOUT_NAME && !s.chars().any(char::is_control)
}

/// `activelayout>>KLAVYE,DÜZEN` verisinden (klavye, düzen) adları. Düzen adında virgül olabilir ("English (US,
/// intl., with dead keys)"); klavye adında olmaz (Hyprland adları küçük harf ve tireli), bu
/// yüzden ilk virgülden bölünür. Boş, çok uzun ya da kontrol karakterli ad reddedilir. İkisine de aynı denetim.
pub fn parse_active_layout(data: &str) -> Option<(&str, &str)> {
    let (keyboard, layout) = data.split_once(',')?;
    let (keyboard, layout) = (keyboard.trim(), layout.trim());
    (valid_name(keyboard) && valid_name(layout)).then_some((keyboard, layout))
}

/// `screencast>>DURUM,SAHİP` verisi: bir istemci ekranı sürekli yakalamaya başladı (`1`) ya da
/// bıraktı (`0`). Hyprland bunu istemci başına yayar (portal paylaşımı, wf-recorder, OBS); sahip
/// alanı (0 monitör, 1 pencere) kullanılmaz. Tanınmayan veri `None`.
pub fn parse_screencast(data: &str) -> Option<bool> {
    // Biçim katı: `0|1,<tür>` (tür 0..=255 rakam). Hyprland olay verisindeki satır sonlarını boşluğa
    // çevirir (`EventManager::formatEvent`, 0.56.2 doğrulandı), başlıktan satır enjeksiyonu yok;
    // olay adı her zaman Hyprland'ın yazdığı ilk `>>`'den öncesidir.
    let (state, kind) = data.split_once(',')?;
    if kind.is_empty() || kind.len() > 3 || !kind.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match state {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// `EVENT>>DATA` satırını ayırır.
pub fn parse_event_line(line: &str) -> Option<(&str, &str)> {
    let (name, data) = line.split_once(">>")?;
    (!name.is_empty()).then_some((name, data))
}

#[derive(Debug, PartialEq, Eq)]
enum LineRead {
    Line,
    /// Satır sınırı aştı ve atlandı; akış senkron kalır.
    TooLong,
    Eof,
}

/// Sınırlı satır okuma: `max` bayttan uzun satır belleğe alınmadan sonuna kadar atlanır.
/// EOF'ta yarım kalan satır atılır (kesilmiş olay).
fn read_line_limited<R: BufRead>(reader: &mut R, buf: &mut Vec<u8>, max: usize) -> io::Result<LineRead> {
    buf.clear();
    let mut too_long = false;
    loop {
        let (used, done) = {
            let available = match reader.fill_buf() {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                return Ok(LineRead::Eof);
            }
            let (chunk, used, done) = match available.iter().position(|&b| b == b'\n') {
                Some(i) => (&available[..i], i + 1, true),
                None => (available, available.len(), false),
            };
            if !too_long {
                if buf.len() + chunk.len() > max {
                    too_long = true;
                    buf.clear();
                } else {
                    buf.extend_from_slice(chunk);
                }
            }
            (used, done)
        };
        reader.consume(used);
        if done {
            return Ok(if too_long { LineRead::TooLong } else { LineRead::Line });
        }
    }
}

// ---------------------------------------------------------------------------------------
// Birleştirme ve yeniden bağlanma (saf durum makineleri)

/// Olay patlaması birleştirici. Boştayken ilk olay hemen yenileme ister (önde kenar);
/// sonrakiler `quiet` sessizlik penceresi bitince tek yenilemeye dönüşür (arka kenar).
/// Bekleyen olay `max_delay`'den uzun bekletilmez.
#[derive(Debug)]
struct Coalescer {
    quiet: Duration,
    max_delay: Duration,
    state: CoalesceState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CoalesceState {
    Idle,
    Cooling { last_activity: Instant, pending_since: Option<Instant> },
}

impl Coalescer {
    fn new(quiet: Duration, max_delay: Duration) -> Self {
        Self { quiet, max_delay, state: CoalesceState::Idle }
    }

    /// Olay geldi. `true`: şimdi yenile.
    fn on_event(&mut self, now: Instant) -> bool {
        match &mut self.state {
            CoalesceState::Idle => {
                self.state = CoalesceState::Cooling { last_activity: now, pending_since: None };
                true
            }
            CoalesceState::Cooling { last_activity, pending_since } => {
                *last_activity = now;
                let since = *pending_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= self.max_delay {
                    *pending_since = None;
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Dışarıdan zorla yapılan yenilemeyi kaydeder (bekleyen olaylar karşılanmış sayılır).
    fn flushed(&mut self, now: Instant) {
        self.state = CoalesceState::Cooling { last_activity: now, pending_since: None };
    }

    fn reset(&mut self) {
        self.state = CoalesceState::Idle;
    }

    /// Bir sonraki uyanma zamanı; `None`: olay gelene kadar uyu.
    fn deadline(&self) -> Option<Instant> {
        match self.state {
            CoalesceState::Idle => None,
            CoalesceState::Cooling { last_activity, pending_since } => {
                let quiet_end = last_activity + self.quiet;
                Some(match pending_since {
                    Some(p) => quiet_end.min(p + self.max_delay),
                    None => quiet_end,
                })
            }
        }
    }

    /// Zaman aşımı. `true`: şimdi yenile.
    fn on_timeout(&mut self, now: Instant) -> bool {
        let Some(deadline) = self.deadline() else { return false };
        if now < deadline {
            return false;
        }
        match self.state {
            CoalesceState::Cooling { pending_since: Some(_), .. } => {
                self.flushed(now);
                true
            }
            _ => {
                self.state = CoalesceState::Idle;
                false
            }
        }
    }
}

/// Üstel yeniden bağlanma beklemesi.
#[derive(Debug)]
struct Backoff {
    min: Duration,
    max: Duration,
    current: Duration,
}

impl Backoff {
    fn new(min: Duration, max: Duration) -> Self {
        Self { min, max: max.max(min), current: min }
    }

    fn next_delay(&mut self) -> Duration {
        let d = self.current;
        self.current = (self.current * 2).min(self.max);
        d
    }

    fn reset(&mut self) {
        self.current = self.min;
    }
}

// ---------------------------------------------------------------------------------------
// Servis

/// Servisin dışarı verdiği olaylar.
#[derive(Clone, PartialEq)]
pub enum HyprlandEvent {
    /// Yeni (öncekinden farklı) durum. Her (yeniden) bağlanmada ilk durum da yayınlanır.
    Snapshot(HyprSnapshot),
    /// socket2 koptu; yeniden bağlanılıyor. Son durum bayat olabilir.
    Disconnected,
    /// Hyprland ayarı yeniden yüklendi (`configreloaded`): çalışma anında `keyword`/`eval` ile
    /// verilen değerler ayar dosyasındakine döndü (ör. büyüteç yeniden uygular). Durum
    /// yenilemesi ayrıca yapılır.
    ConfigReloaded,
    /// Bir klavyenin etkin düzeni değişti (`activelayout`): Hyprland cihaz adı ve xkb düzen adı
    /// ("Turkish", "English (US)"). Ekran klavyesi ve bar düzen widget'ı izler (cihaz adıyla
    /// sanal klavyeler süzülebilir); durum yenilemesi tetiklemez.
    ActiveLayout { keyboard: String, layout: String },
    /// Bir istemci ekranı sürekli yakalamaya başladı (`true`) ya da bıraktı (`false`)
    /// (`screencast`; istemci başına bir çift). Gizlilik göstergesi sayar; durum yenilemesi yok.
    Screencast(bool),
    /// Ana klavyenin düzenleri (`j/devices`; bağlanınca, `activelayout` ve ayar yeniden
    /// yüklemesinde, yalnız değişince). Okunamazsa `None`. Bar düzen göstergesi.
    Keyboard(Option<crate::services::keyboard_layouts::KeyboardLayouts>),
}

/// Debug redakte: anlık görüntü pencere başlıkları içerir; yalnızca sayılar yazılır.
impl std::fmt::Debug for HyprlandEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Snapshot(s) => f
                .debug_struct("Snapshot")
                .field("monitors", &s.monitors.len())
                .field("workspaces", &s.workspaces.len())
                .field("active_window", &s.active_window.is_some())
                .finish(),
            Self::Disconnected => f.write_str("Disconnected"),
            Self::ConfigReloaded => f.write_str("ConfigReloaded"),
            Self::ActiveLayout { keyboard, layout } => f.debug_struct("ActiveLayout").field("keyboard", keyboard).field("layout", layout).finish(),
            Self::Screencast(on) => f.debug_tuple("Screencast").field(on).finish(),
            Self::Keyboard(k) => f.debug_tuple("Keyboard").field(&k.as_ref().map(|k| (k.layouts.len(), k.active))).finish(),
        }
    }
}

/// Zamanlama ayarları. Varsayılanlar canlı Hyprland için seçildi; testler kısaltır.
#[derive(Debug, Clone)]
pub struct ServiceOptions {
    pub request_timeout: Duration,
    /// Patlama sonrası sessizlik penceresi.
    pub coalesce_quiet: Duration,
    /// Sürekli olay akışında iki yenileme arası en uzun süre.
    pub coalesce_max_delay: Duration,
    pub reconnect_min: Duration,
    pub reconnect_max: Duration,
    /// Bu kadar ayakta kalan bağlantı koparsa bekleme en baştan başlar.
    pub stable_after: Duration,
    /// Başarısız durum sorgusundan sonraki ilk yeniden deneme beklemesi; her hatada ikiye
    /// katlanır.
    pub query_retry_min: Duration,
    /// Yeniden deneme beklemesinin tavanı (kalıcı hatada sıcak döngü olmaz).
    pub query_retry_max: Duration,
}

impl Default for ServiceOptions {
    fn default() -> Self {
        Self {
            request_timeout: HyprlandClient::DEFAULT_TIMEOUT,
            coalesce_quiet: Duration::from_millis(35),
            coalesce_max_delay: Duration::from_millis(150),
            reconnect_min: Duration::from_millis(250),
            reconnect_max: Duration::from_secs(5),
            stable_after: Duration::from_secs(10),
            query_retry_min: Duration::from_millis(100),
            query_retry_max: Duration::from_secs(2),
        }
    }
}

/// reader → refresher sinyalleri.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Signal {
    Connected,
    Event,
    Disconnected,
    /// `configreloaded` (ayrıca `Event` de gönderilir).
    ConfigReloaded,
    /// `activelayout` (yalnızca bu; durum yenilenmez): (klavye, düzen).
    ActiveLayout(String, String),
    /// `screencast` (yalnızca bu; durum yenilenmez).
    Screencast(bool),
}

/// Kanal kapasitesi. `Event` sinyalleri eşgüdümlüdür; kanal doluysa yenisi atılır
/// (bekleyen yenileme zaten onu da kapsar), bellek sınırlı kalır.
const SIGNAL_CAPACITY: usize = 64;

#[derive(Debug, Default)]
struct Shared {
    stop: Mutex<bool>,
    wake: Condvar,
    /// Okunan socket2 bağlantısının kopyası; durdurmada `shutdown` edilir.
    stream: Mutex<Option<UnixStream>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Kilit altında panic olmaz; olsa bile veri (bool / Option<UnixStream>) tutarlı kalır.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn stopped(&self) -> bool {
        *lock(&self.stop)
    }

    fn stop(&self) {
        *lock(&self.stop) = true;
        self.wake.notify_all();
        if let Some(stream) = lock(&self.stream).take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    /// `timeout` kadar bekler; durdurulursa hemen `true` döner.
    fn wait_stop(&self, timeout: Duration) -> bool {
        let guard = lock(&self.stop);
        let (guard, _) = self.wake.wait_timeout_while(guard, timeout, |stop| !*stop).unwrap_or_else(PoisonError::into_inner);
        *guard
    }

    /// Bağlantıyı kaydeder ve okuma kopyasını döndürür. Durdurma başladıysa `None`.
    /// `stop()` ile yarış: ikisi de `stream` kilidi altında bayrağa bakar/ayarlar.
    fn register(&self, stream: UnixStream) -> io::Result<Option<UnixStream>> {
        let reader = stream.try_clone()?;
        let mut slot = lock(&self.stream);
        if self.stopped() {
            let _ = stream.shutdown(Shutdown::Both);
            return Ok(None);
        }
        *slot = Some(stream);
        Ok(Some(reader))
    }

    fn unregister(&self) {
        lock(&self.stream).take();
    }
}

/// Çalışan Hyprland servisi. Düşürülünce temiz kapanır.
#[derive(Debug)]
pub struct HyprlandService {
    client: HyprlandClient,
    shared: Arc<Shared>,
    reader: Option<JoinHandle<()>>,
    refresher: Option<JoinHandle<()>>,
}

impl HyprlandService {
    /// Ortamdan yolları çözer ve varsayılan ayarlarla başlatır.
    pub fn spawn_from_env<F>(sink: F) -> Result<Self, HyprError>
    where
        F: Fn(HyprlandEvent) + Send + 'static,
    {
        Ok(Self::spawn(HyprlandPaths::from_env()?, ServiceOptions::default(), sink)?)
    }

    /// Servisi başlatır. `sink` refresher thread'inden çağrılır; hızlı dönmeli (kanala
    /// göndermek gibi). Hyprland o an yoksa servis arka planda bağlanmayı dener.
    pub fn spawn<F>(paths: HyprlandPaths, options: ServiceOptions, sink: F) -> io::Result<Self>
    where
        F: Fn(HyprlandEvent) + Send + 'static,
    {
        let client = HyprlandClient::new(paths.socket1.clone()).with_timeout(options.request_timeout);
        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::sync_channel(SIGNAL_CAPACITY);

        let refresher = {
            let client = client.clone();
            let options = options.clone();
            std::thread::Builder::new()
                .name("nuxut-hypr-refresh".into())
                .spawn(move || refresh_loop(&client, &options, &rx, &sink))?
        };
        let reader = {
            let shared = Arc::clone(&shared);
            let spawned = std::thread::Builder::new()
                .name("nuxut-hypr-events".into())
                .spawn(move || reader_loop(&paths.socket2, &options, &shared, &tx));
            match spawned {
                Ok(handle) => handle,
                Err(e) => {
                    // tx düştü → refresher kendiliğinden çıkar.
                    let _ = refresher.join();
                    return Err(e);
                }
            }
        };
        Ok(Self { client, shared, reader: Some(reader), refresher: Some(refresher) })
    }

    /// Komut göndermek için istemci (ör. workspace değiştirme).
    pub fn client(&self) -> &HyprlandClient {
        &self.client
    }
}

/// Kapanışta refresher'ı bekleme süresi. Meşgul compositor'da süren bir socket1 isteği istek
/// zaman aşımına kadar sürebilir; kapanışı bununla bekletmeyiz.
const REFRESHER_JOIN_GRACE: Duration = Duration::from_millis(200);

impl Drop for HyprlandService {
    fn drop(&mut self) {
        self.shared.stop();
        // reader'ın bloklayan okuması soket kapatılarak hemen kesilir: join hızlı.
        if let Some(reader) = self.reader.take()
            && reader.join().is_err()
        {
            tracing::error!("Hyprland olay thread'i panikle sonlandı");
        }
        // reader çıkınca kanalın gönderen ucu düştü; refresher bir sonraki alımda çıkar. Süren
        // bir istek varsa en fazla REFRESHER_JOIN_GRACE beklenir, sonra thread bırakılır (kendi
        // zaman aşımıyla biter; sink'e yalnızca kanal üzerinden gönderir, güvenli).
        if let Some(refresher) = self.refresher.take() {
            let deadline = Instant::now() + REFRESHER_JOIN_GRACE;
            while !refresher.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if refresher.is_finished() {
                if refresher.join().is_err() {
                    tracing::error!("Hyprland sorgu thread'i panikle sonlandı");
                }
            } else {
                tracing::debug!("Hyprland sorgu thread'i süren istek yüzünden bırakıldı");
            }
        }
    }
}

fn reader_loop(socket2: &Path, options: &ServiceOptions, shared: &Shared, tx: &SyncSender<Signal>) {
    let mut backoff = Backoff::new(options.reconnect_min, options.reconnect_max);
    let mut failures = 0u32;
    while !shared.stopped() {
        match UnixStream::connect(socket2).and_then(|s| shared.register(s)) {
            Ok(Some(stream)) => {
                failures = 0;
                let started = Instant::now();
                tracing::info!(path = %socket2.display(), "Hyprland olay akışına bağlanıldı");
                if tx.send(Signal::Connected).is_err() {
                    return;
                }
                let end = read_events(stream, tx);
                shared.unregister();
                if shared.stopped() {
                    return;
                }
                match end {
                    Ok(()) => tracing::warn!("Hyprland olay akışı kapandı, yeniden bağlanılacak"),
                    Err(e) => tracing::warn!("Hyprland olay akışı okunamadı ({e}), yeniden bağlanılacak"),
                }
                if tx.send(Signal::Disconnected).is_err() {
                    return;
                }
                if started.elapsed() >= options.stable_after {
                    backoff.reset();
                }
            }
            Ok(None) => return,
            Err(e) => {
                failures += 1;
                if failures == 1 {
                    tracing::warn!(path = %socket2.display(), "Hyprland olay soketine bağlanılamadı: {e}");
                } else {
                    tracing::debug!(failures, "Hyprland olay soketine bağlanılamadı: {e}");
                }
            }
        }
        if shared.wait_stop(backoff.next_delay()) {
            return;
        }
    }
}

/// Akış bitene (EOF) veya hata olana kadar okur. Alıcı yoksa da döner.
fn read_events(stream: UnixStream, tx: &SyncSender<Signal>) -> io::Result<()> {
    let mut reader = BufReader::with_capacity(8 * 1024, stream);
    let mut line = Vec::with_capacity(256);
    loop {
        match read_line_limited(&mut reader, &mut line, MAX_EVENT_LINE_BYTES)? {
            LineRead::Eof => return Ok(()),
            LineRead::TooLong => {
                tracing::debug!("çok uzun Hyprland olay satırı atlandı");
                continue;
            }
            LineRead::Line => {}
        }
        // Olay adı ASCII'dir; veri kısmı geçersiz UTF-8 olsa bile ad çıkarılabilir.
        let text = String::from_utf8_lossy(&line);
        let Some((name, data)) = parse_event_line(&text) else { continue };
        if name == "activelayout" {
            if let Some((keyboard, layout)) = parse_active_layout(data)
                && matches!(tx.try_send(Signal::ActiveLayout(keyboard.to_owned(), layout.to_owned())), Err(TrySendError::Disconnected(_)))
            {
                return Ok(());
            }
            continue;
        }
        if name == "screencast" {
            // Sayaç olayı: düşürülmez (kanal doluysa okuyucu kısa süre bekler).
            if let Some(on) = parse_screencast(data)
                && tx.send(Signal::Screencast(on)).is_err()
            {
                return Ok(());
            }
            continue;
        }
        if !is_relevant_event(name) {
            continue;
        }
        tracing::trace!(event = name, "Hyprland olayı");
        if name == "configreloaded" && matches!(tx.try_send(Signal::ConfigReloaded), Err(TrySendError::Disconnected(_))) {
            return Ok(());
        }
        match tx.try_send(Signal::Event) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => return Ok(()),
        }
    }
}

/// Refresher'ın sorgu durumu: son yayınlanan anlık görüntü ve başarısız sorgu sonrası yeniden
/// deneme zamanlaması.
struct QueryState {
    last: Option<HyprSnapshot>,
    /// Bekleyen yeniden deneme zamanı. `None`: son sorgu başarılı; boşta ek timer yoktur.
    retry_at: Option<Instant>,
    backoff: Backoff,
    /// Art arda başarısız sorgu sayısı (log sesini kısmak için).
    failures: u32,
}

impl QueryState {
    fn new(options: &ServiceOptions) -> Self {
        Self {
            last: None,
            retry_at: None,
            backoff: Backoff::new(options.query_retry_min, options.query_retry_max),
            failures: 0,
        }
    }

    /// Durumu sorgular, değiştiyse yayınlar. Hata bekleyen değişikliği düşürmez: yeniden deneme
    /// artan beklemeyle zamanlanır (olaylar zaten "karşılanmış" sayıldığı için aksi hâlde son
    /// durum bir sonraki Hyprland olayına kadar yayınlanmazdı).
    fn refresh<F: Fn(HyprlandEvent)>(&mut self, client: &HyprlandClient, sink: &F) {
        let started = Instant::now();
        match client.snapshot() {
            Ok(snapshot) => {
                if self.failures > 0 {
                    tracing::info!(failures = self.failures, "Hyprland durumu yeniden sorgulanabildi");
                }
                self.cancel_retry();
                let changed = self.last.as_ref() != Some(&snapshot);
                tracing::trace!(changed, us = started.elapsed().as_micros() as u64, "Hyprland durumu sorgulandı");
                if changed {
                    self.last = Some(snapshot.clone());
                    sink(HyprlandEvent::Snapshot(snapshot));
                }
            }
            Err(e) => {
                self.failures = self.failures.saturating_add(1);
                let delay = self.backoff.next_delay();
                self.retry_at = Some(Instant::now() + delay);
                if self.failures == 1 {
                    tracing::warn!("Hyprland durumu sorgulanamadı ({e}); {delay:?} sonra yeniden denenecek");
                } else {
                    tracing::debug!(failures = self.failures, ?delay, "Hyprland durumu yine sorgulanamadı: {e}");
                }
            }
        }
    }

    fn cancel_retry(&mut self) {
        self.retry_at = None;
        self.failures = 0;
        self.backoff.reset();
    }

    fn retry_due(&self, now: Instant) -> bool {
        self.retry_at.is_some_and(|at| now >= at)
    }
}

fn refresh_loop<F: Fn(HyprlandEvent)>(
    client: &HyprlandClient,
    options: &ServiceOptions,
    rx: &Receiver<Signal>,
    sink: &F,
) {
    let mut coalescer = Coalescer::new(options.coalesce_quiet, options.coalesce_max_delay);
    let mut state = QueryState::new(options);
    let mut connected = false;
    // Son yayınlanan klavye düzenleri (`None`: henüz yayınlanmadı; bağlanınca yeniden).
    let mut keyboard: Option<Option<crate::services::keyboard_layouts::KeyboardLayouts>> = None;
    let publish_keyboard = |keyboard: &mut Option<_>| {
        let now = client.keyboard_layouts().ok().flatten();
        if keyboard.as_ref() != Some(&now) {
            *keyboard = Some(now.clone());
            sink(HyprlandEvent::Keyboard(now));
        }
    };

    loop {
        // En yakın uyanma: birleştirici ya da bekleyen yeniden deneme. İkisi de yoksa (boşta,
        // hata yok) yalnızca sinyal beklenir.
        let deadline = match (coalescer.deadline(), state.retry_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let received = match deadline {
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(deadline) => rx.recv_timeout(deadline.saturating_duration_since(Instant::now())),
        };
        let now = Instant::now();
        match received {
            Ok(Signal::Connected) => {
                connected = true;
                state.last = None;
                state.cancel_retry();
                state.refresh(client, sink);
                coalescer.flushed(Instant::now());
                keyboard = None;
                publish_keyboard(&mut keyboard);
            }
            Ok(Signal::Event) => {
                if connected && coalescer.on_event(now) {
                    state.refresh(client, sink);
                }
            }
            Ok(Signal::ConfigReloaded) => {
                sink(HyprlandEvent::ConfigReloaded);
                // `kb_layout` ayarda değişmiş olabilir.
                publish_keyboard(&mut keyboard);
            }
            Ok(Signal::ActiveLayout(kbd, layout)) => {
                sink(HyprlandEvent::ActiveLayout { keyboard: kbd, layout });
                publish_keyboard(&mut keyboard);
            }
            Ok(Signal::Screencast(on)) => sink(HyprlandEvent::Screencast(on)),
            Ok(Signal::Disconnected) => {
                connected = false;
                coalescer.reset();
                // Yeniden bağlanınca ilk durum zaten sorgulanır; kopukken denemek boşuna.
                state.cancel_retry();
                sink(HyprlandEvent::Disconnected);
            }
            Err(RecvTimeoutError::Timeout) => {
                if connected {
                    // Birleştirici her uyanmada ilerletilir; iki neden aynı anda dolarsa tek sorgu.
                    let coalesced = coalescer.on_timeout(now);
                    if coalesced || state.retry_due(now) {
                        state.refresh(client, sink);
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn lock_pid_parsing() {
        assert_eq!(parse_lock_pid("1234\nwayland-1\n"), Some(1234));
        assert_eq!(parse_lock_pid(" 77 "), Some(77));
        assert_eq!(parse_lock_pid("1\n"), None, "init değil");
        assert_eq!(parse_lock_pid("x\n"), None);
        assert_eq!(parse_lock_pid(""), None);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(compositor_pid_in(dir.path()), None, "dosya yok");
        std::fs::write(dir.path().join("hyprland.lock"), "4321\nwayland-1\n").unwrap();
        assert_eq!(compositor_pid_in(dir.path()), Some(4321));
    }

    const MONITORS: &str = include_str!("../../tests/fixtures/hyprland/monitors.json");
    const MONITORS_SPECIAL: &str = include_str!("../../tests/fixtures/hyprland/monitors-special.json");
    const WORKSPACES: &str = include_str!("../../tests/fixtures/hyprland/workspaces.json");
    const ACTIVE_WORKSPACE: &str = include_str!("../../tests/fixtures/hyprland/activeworkspace.json");
    const ACTIVE_WINDOW: &str = include_str!("../../tests/fixtures/hyprland/activewindow.json");
    const SOCKET2_SWITCH: &str = include_str!("../../tests/fixtures/hyprland/socket2-workspace-switch.txt");
    const SNAPSHOT_BATCH: &str = include_str!("../../tests/fixtures/hyprland/snapshot-batch.txt");

    #[test]
    fn parses_live_batch_snapshot() {
        let s = parse_snapshot(SNAPSHOT_BATCH).unwrap();
        assert_eq!(s.monitors.len(), 1);
        assert!(!s.workspaces.is_empty());
        assert!(s.workspaces.windows(2).all(|w| w[0].id < w[1].id));
        let active = s.focused_monitor().unwrap().active_workspace.id;
        assert!(s.workspace(active).is_some());
    }

    #[test]
    fn fullscreen_mode_comes_from_clients() {
        let clients = r#"[{"workspace": {"id": 3}, "fullscreen": 1}, {"workspace": {"id": 3}, "fullscreen": 0},
                          {"workspace": {"id": 4}, "fullscreen": 2}, {"workspace": {"id": 1}, "fullscreen": true},
                          {"workspace": {"id": 2}, "fullscreen": "garbage"}]"#;
        let s = parse_snapshot(&format!("{MONITORS}{WORKSPACES}{ACTIVE_WINDOW}{clients}")).unwrap();
        let mode = |id| s.workspace(id).unwrap().fullscreen_mode;
        assert_eq!((mode(1), mode(2), mode(3), mode(4)), (2, 0, 1, 2));
        assert!(!s.workspace(3).unwrap().is_fullscreen(), "maximize tam ekran sayılmaz");
        assert!(s.workspace(4).unwrap().is_fullscreen());
    }

    #[test]
    fn app_class_is_biggest_visible_window() {
        let clients = r#"[
            {"workspace": {"id": 1}, "class": "kitty", "size": [600, 1000], "focusHistoryID": 0},
            {"workspace": {"id": 1}, "class": "firefox", "size": [1200, 1000], "focusHistoryID": 3},
            {"workspace": {"id": 1}, "class": "huge", "size": [4000, 4000], "mapped": false},
            {"workspace": {"id": 1}, "class": "hidden", "size": [4000, 4000], "hidden": true},
            {"workspace": {"id": 2}, "class": "a", "size": [500, 500], "focusHistoryID": 5},
            {"workspace": {"id": 2}, "class": "b", "size": [500, 500], "focusHistoryID": 1},
            {"workspace": {"id": 3}, "class": "", "initialClass": "steam", "size": "bozuk"},
            {"workspace": {"id": 4}, "class": 7, "size": [10, 10]}
        ]"#;
        let s = parse_snapshot(&format!("{MONITORS}{WORKSPACES}{ACTIVE_WINDOW}{clients}")).unwrap();
        let class = |id| s.workspace(id).unwrap().app_class.as_str();
        assert_eq!(class(1), "firefox", "en büyük; gizli/eşlenmemiş sayılmaz");
        assert_eq!(class(2), "b", "eşitlikte en son odaklanan");
        assert_eq!(class(3), "steam", "sınıf boşsa ilk sınıf; bozuk boyut listeyi düşürmez");
        assert_eq!(class(4), "", "sınıf türü bozuk: temsilci yok");
    }

    #[test]
    fn special_vs_named_workspaces() {
        let ws = |id, name: &str| Workspace { id, name: name.into(), ..Default::default() };
        assert!(ws(-98, "special:magic").is_special());
        assert!(ws(-99, "special").is_special());
        assert!(ws(-50, "").is_special());
        assert!(!ws(-1337, "notes").is_special(), "adlı workspace special değil");
        assert!(!ws(3, "3").is_special());
    }

    #[test]
    fn batch_snapshot_tolerates_missing_or_bad_window_but_not_monitors() {
        let s = parse_snapshot(&format!("{MONITORS}\n\n\n{WORKSPACES}\n\n\n{{}}")).unwrap();
        assert_eq!(s.active_window, None);
        let s = parse_snapshot(&format!("{MONITORS}{WORKSPACES}")).unwrap();
        assert_eq!(s.active_window, None);
        let s = parse_snapshot(&format!("{MONITORS}{WORKSPACES}{ACTIVE_WINDOW}")).unwrap();
        assert_eq!(s.active_window.unwrap().class, "code-oss");
        assert!(matches!(parse_snapshot("unknown request"), Err(HyprError::Json { command: "j/monitors", .. })));
        assert!(matches!(
            parse_snapshot(&format!("{MONITORS}\n\n\nunknown request")),
            Err(HyprError::Json { command: "j/workspaces", .. })
        ));
    }

    #[test]
    fn parses_live_monitors_fixture() {
        let m: Vec<Monitor> = serde_json::from_str(MONITORS).unwrap();
        assert_eq!(m.len(), 1);
        let m = &m[0];
        assert_eq!(m.name, "Virtual-1");
        assert_eq!((m.width, m.height), (1920, 1080));
        assert_eq!(m.scale, 1.0);
        assert!(m.focused);
        assert_eq!(m.active_workspace, WorkspaceRef { id: 2, name: "2".into() });
        assert_eq!(m.special(), None);
    }

    #[test]
    fn parses_special_workspace() {
        let m: Vec<Monitor> = serde_json::from_str(MONITORS_SPECIAL).unwrap();
        assert_eq!(m[0].special(), Some(&WorkspaceRef { id: -98, name: "special:magic".into() }));
    }

    #[test]
    fn parses_live_workspaces_fixture() {
        let ws: Vec<Workspace> = serde_json::from_str(WORKSPACES).unwrap();
        assert_eq!(ws.len(), 4);
        let w3 = ws.iter().find(|w| w.id == 3).unwrap();
        assert!(w3.has_fullscreen);
        assert_eq!(w3.windows, 1);
        assert_eq!(w3.monitor, "Virtual-1");
        assert_eq!(w3.monitor_id, Some(0));
        let a: Workspace = serde_json::from_str(ACTIVE_WORKSPACE).unwrap();
        assert_eq!(a.id, 2);
    }

    #[test]
    fn parses_active_window_and_empty_object() {
        let w: Window = serde_json::from_str(ACTIVE_WINDOW).unwrap();
        assert_eq!(w.class, "code-oss");
        assert!(w.title.contains("Code - OSS"));
        assert_eq!(w.workspace.id, 2);
        let empty: Window = serde_json::from_str("{}").unwrap();
        assert!(empty.address.is_empty());
    }

    #[test]
    fn tolerates_unknown_fields_nulls_and_missing_fields() {
        let ws: Vec<Workspace> =
            serde_json::from_str(r#"[{"id": 7, "monitorID": null, "brandNew": {"x": [1]}}]"#).unwrap();
        assert_eq!(ws[0].id, 7);
        assert_eq!(ws[0].monitor_id, None);
        assert_eq!(ws[0].windows, 0);
    }

    #[test]
    fn normalize_sorts_and_clips() {
        let mut ws: Vec<Workspace> = serde_json::from_str(WORKSPACES).unwrap();
        ws[0].name = "x".repeat(MAX_TEXT_CHARS + 50);
        let s = HyprSnapshot { monitors: Vec::new(), workspaces: ws, active_window: None }.normalize();
        let ids: Vec<i64> = s.workspaces.iter().map(|w| w.id).collect();
        assert_eq!(ids, [1, 2, 3, 4]);
        assert!(s.workspaces.iter().all(|w| w.name.chars().count() <= MAX_TEXT_CHARS));
    }

    #[test]
    fn clip_text_respects_char_boundaries() {
        let mut s = "ğ".repeat(MAX_TEXT_CHARS + 1);
        clip_text(&mut s);
        assert_eq!(s.chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn resolves_paths_from_env_values() {
        let p = HyprlandPaths::resolve(Some("/run/user/1000".into()), Some("abc_123-x".into())).unwrap();
        assert_eq!(p.socket1, Path::new("/run/user/1000/hypr/abc_123-x/.socket.sock"));
        assert_eq!(p.socket2, Path::new("/run/user/1000/hypr/abc_123-x/.socket2.sock"));
    }

    #[test]
    fn rejects_bad_env_values() {
        let r = |dir: Option<&str>, sig: Option<&str>| HyprlandPaths::resolve(dir.map(Into::into), sig.map(Into::into));
        assert!(matches!(r(Some("/run"), None), Err(HyprError::NoSignature)));
        assert!(matches!(r(Some("/run"), Some("")), Err(HyprError::NoSignature)));
        assert!(matches!(r(Some("/run"), Some("../../etc")), Err(HyprError::BadSignature)));
        assert!(matches!(r(Some("/run"), Some("a/b")), Err(HyprError::BadSignature)));
        assert!(matches!(r(None, Some("abc")), Err(HyprError::NoRuntimeDir)));
        assert!(matches!(r(Some("relative/dir"), Some("abc")), Err(HyprError::NoRuntimeDir)));
    }

    #[test]
    fn workspace_target_arguments() {
        assert_eq!(WorkspaceTarget::Id(3).to_arg().unwrap(), "3");
        assert_eq!(WorkspaceTarget::Relative(1).to_arg().unwrap(), "e+1");
        assert_eq!(WorkspaceTarget::Relative(-2).to_arg().unwrap(), "e-2");
        assert!(WorkspaceTarget::Id(0).to_arg().is_err());
        assert!(WorkspaceTarget::Relative(0).to_arg().is_err());
    }

    #[test]
    fn event_lines_and_relevance() {
        let names: Vec<&str> = SOCKET2_SWITCH.lines().filter_map(parse_event_line).map(|(n, _)| n).collect();
        assert_eq!(names.len(), 8);
        assert!(names.iter().all(|n| is_relevant_event(n)));
        assert_eq!(parse_event_line("activewindow>>kitty,a>>b,c"), Some(("activewindow", "kitty,a>>b,c")));
        assert_eq!(parse_event_line("garbage"), None);
        assert_eq!(parse_event_line(">>x"), None);
        for ignored in ["openlayer", "closelayer", "submap", "activelayout", "urgent", "screencast", "v2"] {
            assert!(!is_relevant_event(ignored), "{ignored}");
        }
        assert_eq!(parse_active_layout("kbd,Turkish"), Some(("kbd", "Turkish")));
        assert_eq!(parse_active_layout("kbd,English (US, intl., with dead keys)"), Some(("kbd", "English (US, intl., with dead keys)")));
        assert_eq!(parse_active_layout(",Turkish"), None, "klavye adı boş");
        assert_eq!(parse_active_layout(&format!("{},Turkish", "k".repeat(200))), None);
        assert_eq!(parse_active_layout("k\u{1b}bd,Turkish"), None);
        assert_eq!(parse_active_layout("kbd,"), None);
        assert_eq!(parse_active_layout("Turkish"), None);
        assert_eq!(parse_active_layout(&format!("kbd,{}", "x".repeat(200))), None);
        assert_eq!(parse_active_layout("kbd,Tur\u{7}kish"), None);
        assert_eq!(parse_screencast("1,0"), Some(true));
        assert_eq!(parse_screencast("0,1"), Some(false));
        assert_eq!(parse_screencast("1"), None, "tür alanı zorunlu");
        assert_eq!(parse_screencast("0,2"), Some(false));
        assert_eq!(parse_screencast(" 0,1"), None);
        assert_eq!(parse_screencast("0,1 "), None);
        assert_eq!(parse_screencast("0,1,Pencere"), None);
        assert_eq!(parse_screencast("2,0"), None);
        assert_eq!(parse_screencast(""), None);
    }

    #[test]
    fn line_reader_handles_long_lines_and_eof() {
        let long = "x".repeat(100);
        let data = format!("a>>1\n{long}\nb>>2\ntruncated");
        let mut r = BufReader::with_capacity(16, Cursor::new(data.into_bytes()));
        let mut buf = Vec::new();
        assert_eq!(read_line_limited(&mut r, &mut buf, 32).unwrap(), LineRead::Line);
        assert_eq!(buf, b"a>>1");
        assert_eq!(read_line_limited(&mut r, &mut buf, 32).unwrap(), LineRead::TooLong);
        assert!(buf.is_empty());
        assert_eq!(read_line_limited(&mut r, &mut buf, 32).unwrap(), LineRead::Line);
        assert_eq!(buf, b"b>>2");
        assert_eq!(read_line_limited(&mut r, &mut buf, 32).unwrap(), LineRead::Eof);
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn coalescer_leading_edge_is_immediate() {
        let t0 = Instant::now();
        let mut c = Coalescer::new(ms(35), ms(150));
        assert_eq!(c.deadline(), None);
        assert!(c.on_event(t0));
        // Patlamanın geri kalanı bekletilir.
        for i in 1..8 {
            assert!(!c.on_event(t0 + ms(i)));
        }
        assert_eq!(c.deadline(), Some(t0 + ms(7) + ms(35)));
        assert!(!c.on_timeout(t0 + ms(20)), "erken uyanma yenilemez");
        assert!(c.on_timeout(t0 + ms(42)), "arka kenar tek yenileme");
        // Sessizlik sürerse boşa döner; sonraki olay yine gecikmesiz.
        assert!(!c.on_timeout(t0 + ms(80)));
        assert_eq!(c.deadline(), None);
        assert!(c.on_event(t0 + ms(500)));
    }

    #[test]
    fn coalescer_single_event_causes_single_refresh() {
        let t0 = Instant::now();
        let mut c = Coalescer::new(ms(35), ms(150));
        assert!(c.on_event(t0));
        assert!(!c.on_timeout(t0 + ms(35)));
        assert_eq!(c.deadline(), None);
    }

    #[test]
    fn coalescer_bounds_latency_under_continuous_events() {
        let t0 = Instant::now();
        let mut c = Coalescer::new(ms(35), ms(150));
        let mut refreshes = 0;
        // 1 saniye boyunca her 10 ms'de bir olay: sessizlik penceresi hiç dolmaz.
        let mut t = t0;
        while t < t0 + ms(1000) {
            if let Some(d) = c.deadline()
                && d <= t
                && c.on_timeout(d)
            {
                refreshes += 1;
            }
            if c.on_event(t) {
                refreshes += 1;
            }
            t += ms(10);
        }
        // önde kenar + her ~150 ms'de bir
        assert!((6..=9).contains(&refreshes), "{refreshes}");
    }

    #[test]
    fn backoff_grows_and_resets() {
        let mut b = Backoff::new(ms(250), ms(5000));
        let seq: Vec<u64> = (0..7).map(|_| b.next_delay().as_millis() as u64).collect();
        assert_eq!(seq, [250, 500, 1000, 2000, 4000, 5000, 5000]);
        b.reset();
        assert_eq!(b.next_delay(), ms(250));
    }
}
