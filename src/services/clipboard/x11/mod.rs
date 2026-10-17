//! VM pano köprüsü: Xwayland `CLIPBOARD` seçimi ↔ Wayland panosu (tasarım
//! `docs/design/vm-clipboard.md`, araştırma `docs/research/vm-clipboard.md`).
//!
//! Neden: VM konuk ajanları (spice-vdagent, open-vm-tools, VirtualBox) panoyu yalnızca X11
//! üzerinden konuşur. Hyprland'in XWM'i iki yönü de **klavye odağı bir X11 penceresindeyken**
//! eşitler (gizlilik koruması); ajanın penceresi yoktur, odak hiç ona geçmez, pano VM'e/VM'den
//! akmaz. Mutter odak koşulu olmadan eşitler; KWin Wayland → X11 yönünde odak ister.
//!
//! Nasıl (olay tabanlı; boşta iş yok):
//! - **X11 → Wayland:** XFixes `SelectionNotify` (sahibi değişti) → `TARGETS` → metin + tek
//!   görsel + en çok [`policy::MAX_EXTRA`] ek tür okunur (INCR dahil, [`policy::MAX_BYTES`]
//!   sınırı) → framework data-control ile Wayland seçimi olur ([`policy::BRIDGE_MIME`] işaretli).
//! - **Wayland → X11:** data-control teklifi → X11 `CLIPBOARD`'ı alırız ve hedefleri **tembel**
//!   sunarız: içerik yalnızca bir X11 istemcisi istediğinde Wayland'den okunur.
//! - **Döngü önleme:** işaret türü (iki yönde), kendi penceremiz, compositor'ın XWM istemcisi
//!   (`WM_S<n>` sahibiyle aynı X11 istemcisi), compositor'ın X11 → Wayland yansıtması
//!   ([`policy::MIRROR_WINDOW`]) ve aynı içeriğin yeniden konması (özet).
//! - **Gizlilik:** hassas teklif (parola yöneticisi ipucu, `secret`/`password` türleri,
//!   framework `sensitive`) iki yönde de taşınmaz ve okunmaz. Oturum kilitliyken ya da hariç
//!   tutulan uygulama etkinken ([`Gate`]) içerik iki yönde de okunmaz ve taşınmaz (kilit ekranı
//!   ayrı süreç olduğu için framework'ün kilit kuralı kabukta devreye girmez). İçerik
//!   loglanmaz; tamponlar `Zeroizing`.
//!
//! Ne zaman ([`BridgeMode`]): `auto` yalnızca hipervizör altında ve bir konuk ajanı süreci
//! çalışırken (ajan oturumla başladığı için birkaç dakika boyunca seyrek taranır, sonra tarama
//! durur). Köprü, Hyprland'in "odakta olmayan X11 istemcisi panoyu okuyamaz" korumasını
//! kaldırır; bu yüzden VM dışında varsayılan kapalıdır (`on` ile her zaman açılabilir).

mod conn;
pub mod policy;

use std::ffi::OsString;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use policy::BridgeMode;

use super::{ClipboardSource, Gate, OfferId, filter};

/// Köprüden ana döngüye.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeEvent {
    /// Köprü devreye girdi (`auto`'da konuk ajanı bulundu): pano izlemesi gerekir.
    Active,
    /// Köprü Xwayland'e bağlandı (kullanıcı betiğinin çalışma anı devralması bunu bekler).
    Connected,
}

/// Ana thread'den köprü thread'ine.
#[derive(Debug)]
enum Cmd {
    Offered(Offer),
    Cleared,
    Stop,
}

/// Wayland pano teklifi (yalnızca `Selection::Clipboard`).
#[derive(Debug, Clone)]
struct Offer {
    id: OfferId,
    mimes: Vec<String>,
    flags: filter::Flags,
    /// Ana thread'de alındığı an (yansıtma sezgisi).
    at: Instant,
    /// Kopyalama anında kapı kapalıydı (oturum kilitli ya da hariç tutulan uygulama etkin): X11'e
    /// taşınmaz.
    blocked: bool,
}

/// Komut kuyruğu + uyandırma soketi (X11 bağlantısıyla birlikte `poll` edilir).
struct Inbox {
    rx: Receiver<Cmd>,
    wake: UnixStream,
}

impl Inbox {
    fn drain_wake(&self) {
        let mut buf = [0u8; 64];
        while let Ok(n) = std::io::Read::read(&mut &self.wake, &mut buf) {
            if n < buf.len() {
                break;
            }
        }
    }

    /// `timeout` boyunca (ya da komut gelene dek) bekler; `Stop` / kapanmış kanal → `false`.
    /// Teklifler `latest`'e yazılır (bağlantı yokken yalnızca sonuncusu önemli).
    fn idle(&self, timeout: Duration, latest: &mut Option<Offer>) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain_wake();
            loop {
                match self.rx.try_recv() {
                    Ok(Cmd::Offered(o)) => *latest = Some(o),
                    Ok(Cmd::Cleared) => *latest = None,
                    Ok(Cmd::Stop) | Err(mpsc::TryRecvError::Disconnected) => return false,
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            conn::wait_readable(&[&self.wake], Some(left));
        }
    }
}

/// Çalışan köprü. Düşürülünce thread durur (X11 seçimi bırakılır) ve join edilir.
#[derive(Debug)]
pub struct X11Bridge {
    tx: Sender<Cmd>,
    gate: Arc<Gate>,
    wake: UnixStream,
    thread: Option<JoinHandle<()>>,
}

impl X11Bridge {
    /// `display`: Xwayland'in `DISPLAY`'i (yoksa köprü hiçbir şey yapmaz). `sink` köprü
    /// thread'inden çağrılır.
    pub fn spawn<F>(
        mode: BridgeMode,
        display: Option<OsString>,
        source: Arc<dyn ClipboardSource>,
        gate: Arc<Gate>,
        sink: F,
    ) -> std::io::Result<Self>
    where
        F: Fn(BridgeEvent) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let (wake, wake_rx) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        wake_rx.set_nonblocking(true)?;
        let inbox = Inbox { rx, wake: wake_rx };
        let thread = std::thread::Builder::new()
            .name("nuxut-x11-clip".into())
            .spawn({
                let gate = gate.clone();
                move || run(mode, display, source, gate, &inbox, &sink)
            })?;
        Ok(Self { tx, gate, wake, thread: Some(thread) })
    }

    /// Wayland pano teklifi (`Selection::Clipboard`).
    pub fn offered(&self, id: OfferId, mimes: Vec<String>, flags: filter::Flags) {
        // Kapı ana thread'de, teklif anında okunur (kopyalayan pencere hâlâ etkin).
        let blocked = self.gate.blocks_transfer();
        self.send(Cmd::Offered(Offer { id, mimes, flags, at: Instant::now(), blocked }));
    }

    /// Wayland panosu boşaldı.
    pub fn cleared(&self) {
        self.send(Cmd::Cleared);
    }

    fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_ok() {
            // Dolu soket: thread zaten uyanacak.
            let _ = (&self.wake).write(&[1]);
        }
    }
}

impl Drop for X11Bridge {
    fn drop(&mut self) {
        self.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// `auto`: hipervizör + konuk ajanı. Durdurulursa `None`.
fn wait_for_agent(inbox: &Inbox, latest: &mut Option<Offer>) -> Option<bool> {
    let hyper = std::fs::read_to_string("/proc/cpuinfo").is_ok_and(|c| policy::cpuinfo_has_hypervisor(&c));
    // x86 dışında bayrak yok: ajan taraması yine yapılır.
    if !hyper && cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
        tracing::debug!("pano köprüsü: sanal makine değil; kapalı");
        return Some(false);
    }
    let start = Instant::now();
    for &at in policy::AGENT_SCAN_AT {
        if !inbox.idle(at.saturating_sub(start.elapsed()), latest) {
            return None;
        }
        if let Some(agent) = find_agent() {
            tracing::info!(agent, "pano köprüsü: VM konuk ajanı bulundu; X11 ↔ Wayland eşitleme açık");
            return Some(true);
        }
    }
    tracing::debug!("pano köprüsü: konuk ajanı yok; kapalı");
    Some(false)
}

/// Çalışan bir konuk ajanının adı.
fn find_agent() -> Option<&'static str> {
    let dir = std::fs::read_dir("/proc").ok()?;
    for e in dir.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if let Some(agent) = own_guest_agent(pid) {
            return Some(agent);
        }
    }
    None
}

/// `pid` bu kullanıcının bir konuk ajanı süreci mi; öyleyse adı. Sahip `/proc/<pid>` dizininin
/// uid'i (süreç gerçek kimliği); başka kullanıcının aynı adlı süreci sayılmaz.
pub(super) fn own_guest_agent(pid: u32) -> Option<&'static str> {
    use std::os::unix::fs::MetadataExt as _;
    let dir = std::path::PathBuf::from(format!("/proc/{pid}"));
    let owner = std::fs::metadata(&dir).ok()?.uid();
    let comm = std::fs::read_to_string(dir.join("comm")).ok()?;
    let ours = rustix::process::getuid().as_raw();
    if !policy::is_own_guest_agent(&comm, owner, ours) {
        return None;
    }
    policy::GUEST_AGENTS.iter().copied().find(|a| *a == comm.trim_end())
}

/// Yeniden bağlanma gecikmesi (Xwayland yeniden başlayabilir ya da tembel başlar).
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(300);
/// Bu kadar çalışan bağlantı kopunca gecikme sıfırdan başlar.
const STABLE: Duration = Duration::from_secs(60);

fn run(mode: BridgeMode, display: Option<OsString>, source: Arc<dyn ClipboardSource>, gate: Arc<Gate>, inbox: &Inbox, sink: &dyn Fn(BridgeEvent)) {
    let mut latest = None;
    match mode {
        BridgeMode::Off => return,
        BridgeMode::On => {}
        BridgeMode::Auto => match wait_for_agent(inbox, &mut latest) {
            Some(true) => {}
            Some(false) | None => return,
        },
    }
    let Some(display) = display.and_then(|d| d.into_string().ok()).filter(|d| !d.is_empty()) else {
        tracing::info!("pano köprüsü: DISPLAY yok (Xwayland kapalı); köprü çalışmıyor");
        return;
    };
    // Yalnız yerel X sunucusu (TCP / SSH yönlendirmesi değil); sahibi ayrıca bağlanınca doğrulanır.
    if policy::local_display(&display).is_none() {
        tracing::warn!("pano köprüsü: DISPLAY yerel bir X sunucusu değil; köprü çalışmıyor");
        return;
    }
    sink(BridgeEvent::Active);
    let mut delay = RETRY_FIRST;
    loop {
        let started = Instant::now();
        match conn::Session::connect(&display, mode, source.clone(), gate.clone()) {
            Ok(mut s) => {
                tracing::info!("pano köprüsü: Xwayland'e bağlandı");
                sink(BridgeEvent::Connected);
                match s.run(inbox, latest.take()) {
                    conn::Exit::Stop => return,
                    conn::Exit::Lost(e) => tracing::info!("pano köprüsü: X11 bağlantısı koptu ({e}); yeniden denenecek"),
                }
                latest = s.into_latest();
            }
            Err(e) => tracing::debug!("pano köprüsü: Xwayland'e bağlanılamadı ({e})"),
        }
        if started.elapsed() >= STABLE {
            delay = RETRY_FIRST;
        }
        if !inbox.idle(delay, &mut latest) {
            return;
        }
        delay = (delay * 2).min(RETRY_MAX);
    }
}
