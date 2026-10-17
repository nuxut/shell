//! Uygulama başlatıcısı servisi: XDG masaüstü girdileri, canlı güncelleme, bulanık arama,
//! kullanım sıklığı, hesap makinesi, emoji verisi/araması (`emoji`, emoji kipi) ve başlatma.
//!
//! İki thread:
//! - **`nuxut-launcher-scan`**: `applications/` dizinlerini inotify ile izler, değişiklik
//!   patlamasını [`SETTLE`] sessizliğe (en geç [`MAX_SETTLE`]) birleştirip yeniden tarar
//!   (değişmeyen dosyalar önbellekten), her taramada [`LauncherEvent::Catalog`] yayınlar. Yeni
//!   inotify nesnesi taramadan **önce** kurulur: tarama sırasındaki değişiklik kaçmaz. Var
//!   olmayan dizin için en yakın var olan üst dizin izlenir (dizin sonradan oluşunca yakalanır).
//!   Boşta tamamen uyur (poll; zamanlayıcı yok); durdurma eventfd ile.
//! - **`nuxut-launcher-run`**: başlatma kuyruğu ([`LauncherService::launch`]); kullanım
//!   dosyasının sahibi: açılışta okur, her başarılı başlatmada ve emoji seçiminde kaydeder,
//!   atomik yazar ve [`LauncherEvent::Usage`] yayınlar.
//!
//! Arama UI thread'inde senkron yapılır ([`Searcher`]): katalog değişmez `Arc` olarak gelir.

pub mod calc;
pub mod catalog;
pub mod desktop;
pub mod emoji;
pub mod exec;
pub mod launch;
pub mod search;
pub mod usage;

use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use catalog::{Catalog, ScanConfig, Scanner};
pub use desktop::{DesktopAction, DesktopEntry};
pub use launch::{LaunchError, LaunchOptions, Tools};
pub use search::{Hit, Searcher};
pub use usage::Usage;
pub use emoji::{EmojiHit, EmojiSearcher};

/// Değişiklik patlamasından sonra yeniden taramadan önce beklenen sessizlik.
pub const SETTLE: Duration = Duration::from_millis(200);
/// Sürekli değişiklikte en geç bu kadar sonra taranır.
pub const MAX_SETTLE: Duration = Duration::from_secs(2);
/// En çok izlenen dizin sayısı.
pub const MAX_WATCHES: usize = 512;
/// Bekleyen başlatma sınırı.
const QUEUE: usize = 8;

#[derive(Debug, Clone)]
pub enum LauncherEvent {
    /// Yeni katalog (ilk tarama ve her değişiklik).
    Catalog(Arc<Catalog>),
    /// Kullanım kayıtları (açılışta ve her başarılı başlatmadan sonra).
    Usage(Arc<Usage>),
    /// Başlatma başarısız (UI kısa bir hata gösterebilir).
    LaunchFailed(LaunchError),
}

#[derive(Debug, Clone)]
pub struct LauncherOptions {
    pub scan: ScanConfig,
    /// `None`: kullanım kaydedilmez.
    pub usage_path: Option<PathBuf>,
    pub launch: LaunchOptions,
    pub tools: Tools,
}

impl LauncherOptions {
    /// Ortamdan (XDG dizinleri, `$PATH`, yerel ayar, durum dosyası).
    pub fn from_env(launch: LaunchOptions) -> Self {
        Self { scan: ScanConfig::from_env(), usage_path: Usage::default_path(), launch, tools: Tools::from_env() }
    }
}

/// `[launcher]` (shell.toml).
///
/// ```toml
/// [launcher]
/// enabled       = true
/// terminal      = []       # Terminal=true uygulamalar için önek argv; boş: otomatik
/// systemd_scope = true     # her uygulama kendi systemd kapsamında
/// calculator    = true     # ifade yazınca hesap satırı
/// width         = 560.0    # 360..=900
/// rows          = 8        # görünen sonuç satırı 4..=16
/// ```
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct LauncherConfig {
    pub enabled: bool,
    pub terminal: Vec<String>,
    pub systemd_scope: bool,
    pub calculator: bool,
    pub width: f32,
    pub rows: u32,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self { enabled: true, terminal: Vec::new(), systemd_scope: true, calculator: true, width: 560.0, rows: 8 }
    }
}

/// Terminal önek argümanlarının sınırları.
const MAX_TERMINAL_ARGS: usize = 16;
const MAX_TERMINAL_ARG: usize = 256;

impl LauncherConfig {
    /// Ayarlar şeması. `terminal` (argv listesi) şemada yok: liste türü şemada henüz yok,
    /// boşken otomatik seçim yapılır.
    pub fn schema(out: &mut Vec<crate::schema::SettingSchema>) {
        use nuxut::i18n::t;
        use crate::schema::SettingSchema;
        let d = Self::default();
        out.push(SettingSchema::bool("launcher.enabled", t!("shell-launcher-enabled-label"), d.enabled));
        out.push(SettingSchema::bool("launcher.calculator", t!("shell-launcher-calculator-label"), d.calculator));
        out.push(SettingSchema::bool("launcher.systemd_scope", t!("shell-launcher-systemd-scope-label"), d.systemd_scope));
        out.push(SettingSchema::float("launcher.width", t!("shell-launcher-width-label"), d.width, 360.0, 900.0).unit("px"));
        out.push(SettingSchema::int("launcher.rows", t!("shell-launcher-rows-label"), d.rows.into(), 4, 16));
    }

    /// Değerleri sınırlar; her düzeltme için uyarı (değerler uyarıya yazılmaz).
    pub fn validate(&mut self) -> Vec<String> {
        let mut w = Vec::new();
        let d = Self::default();
        if !self.width.is_finite() || !(360.0..=900.0).contains(&self.width) {
            w.push(format!("launcher.width aralık dışı (360..=900), {} kullanılıyor", d.width));
            self.width = if self.width.is_finite() { self.width.clamp(360.0, 900.0) } else { d.width };
        }
        if !(4..=16).contains(&self.rows) {
            self.rows = self.rows.clamp(4, 16);
            w.push(format!("launcher.rows aralık dışı (4..=16), {} kullanılıyor", self.rows));
        }
        let bad = self.terminal.len() > MAX_TERMINAL_ARGS
            || self.terminal.iter().any(|a| a.is_empty() || a.len() > MAX_TERMINAL_ARG || a.contains('\0'));
        if bad {
            w.push("launcher.terminal geçersiz (boş/çok uzun argüman ya da çok fazla öğe); otomatik seçim".into());
            self.terminal.clear();
        }
        w
    }

    pub fn launch_options(&self) -> LaunchOptions {
        LaunchOptions { terminal: self.terminal.clone(), systemd_scope: self.systemd_scope }
    }
}

/// Başlatma isteği. Debug'ı yalnızca yapıyı gösterir (kimlik, eylem ve belirteç gizli).
#[derive(Clone)]
pub struct LaunchRequest {
    pub entry: Arc<DesktopEntry>,
    /// Masaüstü eylemi kimliği (`new-window`).
    pub action: Option<String>,
    /// xdg-activation token'ı (compositor'dan; içerik loglanmaz).
    pub activation_token: Option<String>,
}

impl std::fmt::Debug for LaunchRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchRequest")
            .field("action", &self.action.is_some())
            .field("activation_token", &self.activation_token.is_some())
            .finish_non_exhaustive()
    }
}

enum Job {
    Launch(LaunchRequest),
    Options(LaunchOptions),
    /// Emoji kipinde seçim: kullanım kaydına (dosya yazımı bu thread'de).
    RecordEmoji(&'static str),
}

/// Paylaşılan durdurma sinyali (eventfd).
#[derive(Debug)]
struct Wake {
    fd: OwnedFd,
    stop: AtomicBool,
    rescan: AtomicBool,
}

impl Wake {
    fn new() -> std::io::Result<Self> {
        use rustix::event::{EventfdFlags, eventfd};
        let fd = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        Ok(Self { fd, stop: AtomicBool::new(false), rescan: AtomicBool::new(false) })
    }

    fn signal(&self) {
        let _ = rustix::io::write(&self.fd, &1u64.to_ne_bytes());
    }

    fn drain(&self) {
        let mut buf = [0u8; 8];
        let _ = rustix::io::read(&self.fd, &mut buf);
    }
}

/// Çalışan servis. Düşürülünce iki thread de durur ve join edilir.
#[derive(Debug)]
pub struct LauncherService {
    wake: Arc<Wake>,
    tx: Option<SyncSender<Job>>,
    scan_thread: Option<JoinHandle<()>>,
    run_thread: Option<JoinHandle<()>>,
}

impl LauncherService {
    pub fn spawn<F>(options: LauncherOptions, sink: F) -> std::io::Result<Self>
    where
        F: Fn(LauncherEvent) + Send + Sync + 'static,
    {
        let sink = Arc::new(sink);
        let wake = Arc::new(Wake::new()?);
        let scan_thread = {
            let (wake, sink, scan) = (wake.clone(), sink.clone(), options.scan.clone());
            std::thread::Builder::new().name("nuxut-launcher-scan".into()).spawn(move || scan_loop(&scan, &wake, &*sink))?
        };
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        let run_thread = {
            let sink = sink.clone();
            std::thread::Builder::new()
                .name("nuxut-launcher-run".into())
                .spawn(move || run_loop(options, rx, &*sink))
        };
        let run_thread = match run_thread {
            Ok(t) => t,
            Err(e) => {
                wake.stop.store(true, Ordering::SeqCst);
                wake.signal();
                let _ = scan_thread.join();
                return Err(e);
            }
        };
        Ok(Self { wake, tx: Some(tx), scan_thread: Some(scan_thread), run_thread: Some(run_thread) })
    }

    /// Başlatma kuyruğa alınır (bloklamaz; kuyruk doluysa reddedilir).
    pub fn launch(&self, request: LaunchRequest) {
        self.send(Job::Launch(request));
    }

    /// Başlatma ayarlarını günceller (config yeniden yüklenince).
    pub fn set_options(&self, options: LaunchOptions) {
        self.send(Job::Options(options));
    }

    /// Emoji kipinde seçilen emojiyi kullanım kaydına ekler (sık kullanılanlar; bloklamaz).
    pub fn record_emoji(&self, emoji: &'static str) {
        self.send(Job::RecordEmoji(emoji));
    }

    /// Dizinleri hemen yeniden tarar.
    pub fn rescan(&self) {
        self.wake.rescan.store(true, Ordering::SeqCst);
        self.wake.signal();
    }

    fn send(&self, job: Job) {
        if let Some(tx) = &self.tx {
            match tx.try_send(job) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => tracing::warn!("başlatıcı kuyruğu dolu; istek atıldı"),
                Err(TrySendError::Disconnected(_)) => tracing::warn!("başlatıcı thread'i kapalı"),
            }
        }
    }
}

impl Drop for LauncherService {
    fn drop(&mut self) {
        self.wake.stop.store(true, Ordering::SeqCst);
        self.wake.signal();
        self.tx.take();
        for t in [self.scan_thread.take(), self.run_thread.take()].into_iter().flatten() {
            if t.join().is_err() {
                tracing::error!("başlatıcı thread'i panikle sonlandı");
            }
        }
    }
}

/// Dizinleri izleyen inotify nesnesi.
fn watch(config: &ScanConfig) -> Option<OwnedFd> {
    use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
    let fd = match inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK) {
        Ok(fd) => fd,
        Err(e) => {
            tracing::warn!(error = %e, "inotify açılamadı; uygulama listesi canlı güncellenmeyecek");
            return None;
        }
    };
    let flags = WatchFlags::CREATE
        | WatchFlags::DELETE
        | WatchFlags::CLOSE_WRITE
        | WatchFlags::MOVED_FROM
        | WatchFlags::MOVED_TO
        | WatchFlags::ATTRIB
        | WatchFlags::DELETE_SELF
        | WatchFlags::MOVE_SELF
        | WatchFlags::ONLYDIR;
    let mut count = 0usize;
    let mut add = |dir: &Path| {
        if count < MAX_WATCHES && inotify::add_watch(&fd, dir, flags).is_ok() {
            count += 1;
        }
    };
    for apps in config.application_dirs() {
        if apps.is_dir() {
            add_tree(&apps, 0, &mut add);
        } else if let Some(parent) = apps.ancestors().skip(1).find(|p| p.is_dir()) {
            add(parent); // dizin sonradan oluşursa CREATE gelir
        }
    }
    if count >= MAX_WATCHES {
        tracing::warn!(limit = MAX_WATCHES, "izlenen dizin sınırına ulaşıldı");
    }
    Some(fd)
}

fn add_tree(dir: &Path, depth: usize, add: &mut impl FnMut(&Path)) {
    add(dir);
    if depth >= catalog::MAX_DEPTH {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for item in read.flatten() {
        if item.file_type().is_ok_and(|t| t.is_dir()) {
            add_tree(&item.path(), depth + 1, add);
        }
    }
}

/// `fd` okunabilir olana ya da `wake` sinyaline kadar bekler; `timeout` dolarsa `false`.
fn wait(fd: Option<&OwnedFd>, wake: &Wake, timeout: Option<Duration>) -> bool {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let ts = timeout.map(|d| Timespec { tv_sec: d.as_secs() as i64, tv_nsec: d.subsec_nanos() as i64 });
    let mut fds = vec![PollFd::new(&wake.fd, PollFlags::IN)];
    if let Some(fd) = fd {
        fds.push(PollFd::new(fd, PollFlags::IN));
    }
    loop {
        match poll(&mut fds, ts.as_ref()) {
            Ok(n) => return n > 0,
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return false,
        }
    }
}

fn drain(fd: &impl AsFd) {
    let mut buf = [0u8; 4096];
    while matches!(rustix::io::read(fd, &mut buf), Ok(n) if n > 0) {}
}

fn scan_loop(config: &ScanConfig, wake: &Wake, sink: &(dyn Fn(LauncherEvent) + Send + Sync)) {
    let mut scanner = Scanner::new();
    loop {
        let inotify = watch(config);
        let started = Instant::now();
        let catalog = scanner.scan(config);
        tracing::debug!(
            apps = catalog.len(),
            parsed = scanner.last_parsed,
            ms = started.elapsed().as_millis() as u64,
            "uygulama kataloğu tarandı"
        );
        sink(LauncherEvent::Catalog(Arc::new(catalog)));

        // Olay bekle (boşta tamamen uyur).
        wait(inotify.as_ref(), wake, None);
        if wake.stop.load(Ordering::SeqCst) {
            return;
        }
        // Birleştirme: SETTLE sessizlik ya da MAX_SETTLE.
        let first = Instant::now();
        loop {
            wake.drain();
            if let Some(fd) = &inotify {
                drain(fd);
            }
            if wake.stop.load(Ordering::SeqCst) {
                return;
            }
            if wake.rescan.swap(false, Ordering::SeqCst) {
                break;
            }
            let left = MAX_SETTLE.saturating_sub(first.elapsed());
            if left.is_zero() || !wait(inotify.as_ref(), wake, Some(SETTLE.min(left))) {
                break;
            }
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Kullanım kaydını atomik yazar ve yayınlar.
fn persist_usage(usage: &Usage, options: &LauncherOptions, sink: &(dyn Fn(LauncherEvent) + Send + Sync)) {
    if let Some(path) = &options.usage_path
        && let Err(e) = usage.save(path)
    {
        tracing::warn!(kind = ?e.kind(), "başlatıcı kullanım dosyası yazılamadı");
    }
    sink(LauncherEvent::Usage(Arc::new(usage.clone())));
}

fn run_loop(mut options: LauncherOptions, rx: Receiver<Job>, sink: &(dyn Fn(LauncherEvent) + Send + Sync)) {
    // Gömülü emoji verisi bu thread'de bir kez ayrıştırılır: emoji kipinin ilk açılışı UI
    // thread'ini beklemez (kullanım dosyasının emoji anahtarları da buna karşı doğrulanır).
    emoji::all();
    let mut usage = options.usage_path.as_deref().map(Usage::load).unwrap_or_default();
    sink(LauncherEvent::Usage(Arc::new(usage.clone())));
    let mut dbus: Option<zbus::blocking::Connection> = None;
    while let Ok(job) = rx.recv() {
        let request = match job {
            Job::Options(o) => {
                options.launch = o;
                continue;
            }
            Job::RecordEmoji(e) => {
                usage.record_emoji(e, unix_now());
                persist_usage(&usage, &options, sink);
                continue;
            }
            Job::Launch(r) => r,
        };
        match launch_one(&request, &options, &mut dbus) {
            Ok(()) => {
                usage.record(&request.entry.id, unix_now());
                persist_usage(&usage, &options, sink);
            }
            Err(e) => {
                tracing::warn!(error = %e, "uygulama başlatılamadı");
                sink(LauncherEvent::LaunchFailed(e));
            }
        }
    }
}

fn launch_one(
    request: &LaunchRequest,
    options: &LauncherOptions,
    dbus: &mut Option<zbus::blocking::Connection>,
) -> Result<(), LaunchError> {
    let entry = &*request.entry;
    let action = request.action.as_deref();
    let token = request.activation_token.as_deref();
    let action_exec = match action {
        Some(a) => entry.actions.iter().find(|x| x.id == a).ok_or(LaunchError::UnknownAction)?.exec.is_some(),
        None => entry.exec.is_some(),
    };
    if entry.dbus_activatable {
        match launch::dbus_activate(dbus, entry, action, token) {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!(error = %e, "D-Bus etkinleştirme başarısız");
                if matches!(e, zbus::Error::InputOutput(_)) {
                    *dbus = None; // bağlantı koptu: sonraki istekte yeniden kurulur
                }
                if !action_exec {
                    return Err(LaunchError::NoExec);
                }
            }
        }
    }
    let plan = launch::plan(entry, action, &options.launch, &options.tools)?;
    launch::spawn(entry, &plan, &options.launch, &options.tools, token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn app(name: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec=true\n")
    }

    /// Canlı güncelleme: dosya ekleme/silme, sonradan oluşan kullanıcı dizini, birleştirme.
    #[test]
    fn live_updates_via_inotify() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home"); // henüz yok
        let sys = tmp.path().join("sys");
        write(&sys.join("applications/a.desktop"), &app("A"));
        let options = LauncherOptions {
            scan: ScanConfig { data_dirs: vec![home.clone(), sys.clone()], ..Default::default() },
            usage_path: None,
            launch: LaunchOptions::default(),
            tools: Tools::default(),
        };
        let (tx, rx) = mpsc::channel();
        let service = LauncherService::spawn(options, move |e| {
            if let LauncherEvent::Catalog(c) = e {
                let mut names: Vec<String> = c.apps.iter().map(|a| a.entry.name.clone()).collect();
                names.sort();
                let _ = tx.send(names);
            }
        })
        .unwrap();
        let next = || rx.recv_timeout(Duration::from_secs(5)).expect("katalog gelmedi");
        assert_eq!(next(), ["A"]);

        // Patlama tek taramaya iner.
        for i in 0..20 {
            write(&sys.join(format!("applications/sub/b{i}.desktop")), &app(&format!("B{i:02}")));
        }
        let mut names = next();
        while names.len() < 21 {
            names = next();
        }
        assert_eq!(names.len(), 21);
        assert!(rx.recv_timeout(SETTLE * 3).is_err(), "patlama sonrası fazladan tarama yok");

        // Var olmayan kullanıcı dizini oluşup içine dosya gelince.
        write(&home.join("applications/a.desktop"), &app("A Kullanıcı"));
        let mut names = next();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !names.contains(&"A Kullanıcı".to_owned()) && Instant::now() < deadline {
            names = next();
        }
        assert!(names.contains(&"A Kullanıcı".to_owned()));
        assert!(!names.contains(&"A".to_owned()), "kullanıcı dosyası gölgeler");

        std::fs::remove_file(home.join("applications/a.desktop")).unwrap();
        let mut names = next();
        while !names.contains(&"A".to_owned()) {
            names = next();
        }

        service.rescan();
        assert_eq!(next().len(), 21);
        let started = Instant::now();
        drop(service);
        assert!(started.elapsed() < Duration::from_secs(1), "kapanış hızlı");
    }

    #[test]
    fn config_validation() {
        let mut c = LauncherConfig { width: f32::NAN, rows: 100, terminal: vec![String::new()], ..Default::default() };
        assert_eq!(c.validate().len(), 3);
        assert_eq!((c.width, c.rows, c.terminal.len()), (560.0, 16, 0));
        let mut ok = LauncherConfig { terminal: vec!["kitty".into(), "-e".into()], ..Default::default() };
        assert!(ok.validate().is_empty());
        assert_eq!(ok.launch_options().terminal, ["kitty", "-e"]);
    }

    #[test]
    fn failed_launch_reports_and_usage_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let usage_path = tmp.path().join("state/launcher.toml");
        let options = LauncherOptions {
            scan: ScanConfig::default(),
            usage_path: Some(usage_path.clone()),
            launch: LaunchOptions::default(),
            tools: Tools { path: vec![tmp.path().to_path_buf()], ..Default::default() },
        };
        let (tx, rx) = mpsc::channel();
        let service = LauncherService::spawn(options, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let entry = desktop::parse(
            "x.desktop",
            Path::new("/x"),
            "[Desktop Entry]\nType=Application\nName=X\nExec=surely-not-installed",
            &desktop::Locale::default(),
        )
        .unwrap();
        service.launch(LaunchRequest { entry: Arc::new(entry), action: None, activation_token: None });
        let mut failed = None;
        while let Ok(e) = rx.recv_timeout(Duration::from_secs(5)) {
            if let LauncherEvent::LaunchFailed(err) = e {
                failed = Some(err);
                break;
            }
        }
        assert_eq!(failed, Some(LaunchError::NotFound));
        assert!(!usage_path.exists());
    }
}
