//! Kurulu tema izleyicisi: ikon/imleç (`icons/`) ve GTK (`themes/`) arama dizinlerinde tema
//! eklenince, silinince ya da `index.theme` değişince [`ThemeWatcher::rev`] sayacını artırır.
//! `msg look status` bu sayacı `themes_rev` olarak verir; Ayarlar Görünüm ızgarasını yalnızca
//! sayaç değişince yeniden okur. Shell aynı anda birleşik görünüm turunu da tazeler (yeni kurulan
//! önerilen tema yedeğin yerini alır).
//!
//! Tek thread (`nuxut-theme-watch`), boşta tamamen uyur (poll; zamanlayıcı yok):
//! - **İzlenen:** her arama dizini (ekleme/silme/yeniden adlandırma) ve içindeki her tema dizininin
//!   **yalnızca üst düzeyi** ([`MARKERS`]: `index.theme`, `cursors/`, hyprcursor manifesti,
//!   `gtk-3.0/`, `gtk-4.0/`). Özyineleme yok; paketin binlerce ikon dosyası olay üretmez.
//! - **Var olmayan dizin:** en yakın var olan üst dizin izlenir ve yalnızca yoldaki sonraki
//!   bileşenin olayları dikkate alınır (ör. `~/.icons` yokken `$HOME`'daki diğer dosyalar
//!   uyandırmaz).
//! - **Birleştirme:** ilk ilgili olaydan sonra [`SETTLE`] sessizlik (en geç [`MAX_SETTLE`]); sonra
//!   izleme yeniden kurulur ve **ardından** parmak izi alınır (aradaki değişiklik kaçmaz). Sayaç
//!   yalnızca parmak izi (tema adları + işaret dosyalarının boy/zamanı) değiştiyse artar: ilgisiz
//!   dosya ya da aynı içerik yeniden yazımı artırmaz, bir paket kurulumu çoğunlukla tek artıştır.
//! - **Sınır:** en çok [`MAX_WATCHES`] izleme (aşan tema dizinleri izlenmez, arama dizinleri
//!   izlenmeye devam eder); çekirdek izleme limiti (`ENOSPC`) ya da inotify açılamazsa uyarı
//!   loglanır ve izleyici sessizce kapanır (sayaç donar, `msg look status` yine taze okur).

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

/// Olay patlamasından sonra beklenen sessizlik.
pub const SETTLE: Duration = Duration::from_millis(500);
/// Sürekli olayda en geç bu kadar sonra parmak izi alınır.
pub const MAX_SETTLE: Duration = Duration::from_secs(2);
/// En çok inotify izlemesi (arama dizinleri + tema dizinleri + üst dizinler).
pub const MAX_WATCHES: usize = 512;
/// Bir arama dizininde bakılan en çok girdi (framework tarayıcısıyla aynı).
pub const MAX_ENTRIES_PER_DIR: usize = 4096;
/// En çok arama dizini.
const MAX_ROOTS: usize = 40;
/// Tema dizininde izlenen üst düzey adlar (kurulu listeyi etkileyenler).
pub const MARKERS: [&str; 6] = ["index.theme", "cursors", "manifest.hl", "manifest.toml", "gtk-3.0", "gtk-4.0"];

/// Arama dizinleri: `~/.icons`, `$XDG_DATA_HOME/icons`, `$XDG_DATA_DIRS/*/icons`, `~/.themes`,
/// `$XDG_DATA_HOME/themes`, `$XDG_DATA_DIRS/*/themes` (boşsa `/usr/local/share:/usr/share`).
/// Göreli yollar atlanır, yinelenenler bir kez.
pub fn roots_from(home: Option<&Path>, data_home: Option<&Path>, data_dirs: Option<&str>) -> Vec<PathBuf> {
    let data_home = data_home.map(Path::to_path_buf).or_else(|| home.map(|h| h.join(".local/share")));
    let sys = data_dirs.filter(|s| !s.is_empty()).unwrap_or("/usr/local/share:/usr/share");
    let mut data: Vec<PathBuf> = data_home.into_iter().collect();
    data.extend(sys.split(':').map(PathBuf::from).take(16));
    let mut out: Vec<PathBuf> = Vec::new();
    for (dot, sub) in [(".icons", "icons"), (".themes", "themes")] {
        let candidates = home.map(|h| h.join(dot)).into_iter().chain(data.iter().map(|d| d.join(sub)));
        for p in candidates {
            if p.is_absolute() && !out.contains(&p) && out.len() < MAX_ROOTS {
                out.push(p);
            }
        }
    }
    out
}

/// Ortamdan ([`roots_from`]).
pub fn default_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok();
    roots_from(home.as_deref(), data_home.as_deref(), data_dirs.as_deref())
}

#[derive(Debug, Clone)]
pub struct WatchOptions {
    pub roots: Vec<PathBuf>,
    pub settle: Duration,
    pub max_settle: Duration,
}

impl WatchOptions {
    pub fn from_env() -> Self {
        Self { roots: default_roots(), settle: SETTLE, max_settle: MAX_SETTLE }
    }
}

/// Durdurma sinyali (eventfd).
#[derive(Debug)]
struct Wake {
    fd: OwnedFd,
    stop: AtomicBool,
    /// İlk izleme ve parmak izi hazır (testler bekler).
    armed: AtomicBool,
}

impl Wake {
    fn new() -> std::io::Result<Self> {
        use rustix::event::{EventfdFlags, eventfd};
        let fd = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        Ok(Self { fd, stop: AtomicBool::new(false), armed: AtomicBool::new(false) })
    }

    fn signal(&self) {
        let _ = rustix::io::write(&self.fd, &1u64.to_ne_bytes());
    }
}

/// Çalışan izleyici; düşürülünce thread durur ve join edilir.
#[derive(Debug)]
pub struct ThemeWatcher {
    rev: Arc<AtomicU64>,
    wake: Arc<Wake>,
    thread: Option<JoinHandle<()>>,
}

impl ThemeWatcher {
    /// `sink`: her artışta yeni sayaç (izleyici thread'inden çağrılır).
    pub fn spawn<F>(options: WatchOptions, sink: F) -> std::io::Result<Self>
    where
        F: Fn(u64) + Send + 'static,
    {
        let rev = Arc::new(AtomicU64::new(0));
        let wake = Arc::new(Wake::new()?);
        let thread = {
            let (rev, wake) = (rev.clone(), wake.clone());
            std::thread::Builder::new().name("nuxut-theme-watch".into()).spawn(move || run(&options, &wake, &rev, &sink))?
        };
        Ok(Self { rev, wake, thread: Some(thread) })
    }

    /// Kurulu tema listesinin sürümü (süreç başına 0'dan başlar).
    pub fn rev(&self) -> u64 {
        self.rev.load(Ordering::SeqCst)
    }
}

impl Drop for ThemeWatcher {
    fn drop(&mut self) {
        self.wake.stop.store(true, Ordering::SeqCst);
        self.wake.signal();
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("tema izleyici thread'i panikle sonlandı");
        }
    }
}

/// Bir izlemenin rolü.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// Arama dizini: her olay ilgili.
    Root,
    /// Tema dizini: yalnızca [`MARKERS`] adları ve kendi silinmesi.
    Theme,
    /// Var olmayan arama dizininin en yakın üst dizini: yalnızca bu ad.
    Ancestor(OsString),
}

/// Kurulmuş inotify nesnesi ve izleme rolleri.
struct Watch {
    fd: OwnedFd,
    kinds: HashMap<i32, Vec<Kind>>,
}

/// İzleme kurulamadı: izleyici kapanır.
#[derive(Debug)]
struct Disabled;

impl Watch {
    fn new(roots: &[PathBuf]) -> Result<Self, Disabled> {
        use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
        let fd = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK).map_err(|e| {
            tracing::warn!(error = %e, "inotify açılamadı; tema listesi canlı güncellenmeyecek");
            Disabled
        })?;
        let tree = WatchFlags::CREATE
            | WatchFlags::DELETE
            | WatchFlags::MOVED_FROM
            | WatchFlags::MOVED_TO
            | WatchFlags::DELETE_SELF
            | WatchFlags::MOVE_SELF
            | WatchFlags::ONLYDIR;
        // Tema dizininde `index.theme` içerik değişimi de.
        let theme = tree | WatchFlags::CLOSE_WRITE;
        // Üst dizinde yalnızca oluşma/taşınma; aynı dizin başka rolle de izleniyorsa bayraklar eklenir.
        let ancestor = WatchFlags::CREATE | WatchFlags::MOVED_TO | WatchFlags::ONLYDIR | WatchFlags::MASK_ADD;
        let mut w = Self { fd, kinds: HashMap::new() };
        let mut count = 0usize;
        let mut capped = false;
        let mut add = |w: &mut Self, path: &Path, flags: WatchFlags, kind: Kind| -> Result<(), Disabled> {
            if count >= MAX_WATCHES {
                capped = true;
                return Ok(());
            }
            match inotify::add_watch(&w.fd, path, flags) {
                Ok(wd) => {
                    count += 1;
                    w.kinds.entry(wd).or_default().push(kind);
                    Ok(())
                }
                Err(rustix::io::Errno::NOSPC) => {
                    tracing::warn!("inotify izleme limiti dolu; tema listesi canlı güncellenmeyecek");
                    Err(Disabled)
                }
                // Erişim yok, arada silindi vb.: o dizin atlanır (bir sonraki turda yeniden denenir).
                Err(_) => Ok(()),
            }
        };
        let mut missing: Vec<&PathBuf> = Vec::new();
        for root in roots {
            if !root.is_dir() {
                missing.push(root);
                continue;
            }
            add(&mut w, root, tree, Kind::Root)?;
            for name in theme_dirs(root) {
                add(&mut w, &root.join(&name), theme, Kind::Theme)?;
            }
        }
        // Üst dizinler en son: MASK_ADD ile var olan rolün bayraklarını ezmez.
        for root in missing {
            if let Some((parent, next)) = nearest_existing(root) {
                add(&mut w, &parent, ancestor, Kind::Ancestor(next))?;
            }
        }
        if capped {
            tracing::warn!(limit = MAX_WATCHES, "tema izleme sınırına ulaşıldı; bazı tema dizinleri izlenmiyor");
        }
        Ok(w)
    }

    /// Bekleyen olayları okur; en az biri ilgiliyse `true`.
    fn drain(&self) -> bool {
        use rustix::fs::inotify::{ReadFlags, Reader};
        let mut buf = [MaybeUninit::<u8>::uninit(); 4096];
        let mut reader = Reader::new(&self.fd, &mut buf);
        let mut relevant = false;
        // Okunacak olay kalmayınca `AGAIN` (NONBLOCK) ya da başka hata: döngü biter.
        while let Ok(event) = reader.next() {
            let flags = event.events();
            if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
                relevant = true;
                continue;
            }
            let Some(kinds) = self.kinds.get(&event.wd()) else { continue };
            let name = event.file_name().map(|n| OsStr::from_bytes(n.to_bytes()));
            let self_event = flags.intersects(ReadFlags::DELETE_SELF | ReadFlags::MOVE_SELF | ReadFlags::IGNORED);
            relevant |= kinds.iter().any(|k| match k {
                Kind::Root => true,
                Kind::Theme => self_event || name.is_some_and(|n| MARKERS.iter().any(|m| OsStr::new(m) == n)),
                Kind::Ancestor(next) => self_event || name == Some(next.as_os_str()),
            });
        }
        relevant
    }
}

/// Arama dizinindeki tema adayları: `.` ile başlamayan, `default` olmayan dizinler (symlink
/// izlenir), sıralı, sınırlı.
fn theme_dirs(root: &Path) -> Vec<OsString> {
    let Ok(read) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<OsString> = read
        .take(MAX_ENTRIES_PER_DIR)
        .flatten()
        .map(|e| e.file_name())
        .filter(|n| !n.as_bytes().starts_with(b".") && n != "default")
        .filter(|n| root.join(n).is_dir())
        .collect();
    out.sort();
    out
}

/// Var olmayan yolun en yakın var olan üst dizini ve yoldaki sonraki bileşen.
fn nearest_existing(path: &Path) -> Option<(PathBuf, OsString)> {
    let mut child = path;
    for parent in path.ancestors().skip(1) {
        if parent.is_dir() {
            return Some((parent.to_path_buf(), child.file_name()?.to_os_string()));
        }
        child = parent;
    }
    None
}

/// İşaret girdisinin özeti: dizin için yalnızca varlık, dosya için boy + değişim zamanı.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mark {
    Dir,
    File(u64, Option<SystemTime>),
}

/// Kurulu listeyi etkileyen her şey: arama dizini başına tema adları ve işaretleri.
type Fingerprint = Vec<Vec<(OsString, Vec<Option<Mark>>)>>;

fn fingerprint(roots: &[PathBuf]) -> Fingerprint {
    roots
        .iter()
        .map(|root| {
            theme_dirs(root)
                .into_iter()
                .map(|name| {
                    let dir = root.join(&name);
                    let marks = MARKERS
                        .iter()
                        .map(|m| {
                            let md = std::fs::metadata(dir.join(m)).ok()?;
                            Some(if md.is_dir() { Mark::Dir } else { Mark::File(md.len(), md.modified().ok()) })
                        })
                        .collect();
                    (name, marks)
                })
                .collect()
        })
        .collect()
}

/// `fd` okunabilir olana ya da durdurma sinyaline kadar bekler; `timeout` dolarsa `false`.
fn wait(fd: &OwnedFd, wake: &Wake, timeout: Option<Duration>) -> bool {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let ts = timeout.map(|d| Timespec { tv_sec: d.as_secs() as i64, tv_nsec: d.subsec_nanos() as i64 });
    let mut fds = [PollFd::new(&wake.fd, PollFlags::IN), PollFd::new(fd, PollFlags::IN)];
    loop {
        match poll(&mut fds, ts.as_ref()) {
            Ok(n) => return n > 0,
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return false,
        }
    }
}

fn run(options: &WatchOptions, wake: &Wake, rev: &AtomicU64, sink: &dyn Fn(u64)) {
    let mut last: Option<Fingerprint> = None;
    loop {
        // Önce izleme, sonra parmak izi: arada olan değişiklik bir sonraki olayla yakalanır.
        let Ok(watch) = Watch::new(&options.roots) else { return };
        let now = fingerprint(&options.roots);
        if last.as_ref().is_some_and(|l| *l != now) {
            let r = rev.fetch_add(1, Ordering::SeqCst) + 1;
            tracing::debug!(rev = r, "kurulu temalar değişti");
            sink(r);
        }
        last = Some(now);
        wake.armed.store(true, Ordering::SeqCst);

        // İlgili olaya kadar uyu.
        loop {
            let ready = wait(&watch.fd, wake, None);
            if wake.stop.load(Ordering::SeqCst) {
                return;
            }
            if ready && watch.drain() {
                break;
            }
        }
        // Birleştirme: `settle` sessizlik ya da `max_settle`.
        let first = Instant::now();
        loop {
            let left = options.max_settle.saturating_sub(first.elapsed());
            if left.is_zero() || !wait(&watch.fd, wake, Some(options.settle.min(left))) {
                break;
            }
            if wake.stop.load(Ordering::SeqCst) {
                return;
            }
            watch.drain();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    const TEST_SETTLE: Duration = Duration::from_millis(80);

    fn spawn(roots: Vec<PathBuf>) -> (ThemeWatcher, mpsc::Receiver<u64>) {
        let (tx, rx) = mpsc::channel();
        let options = WatchOptions { roots, settle: TEST_SETTLE, max_settle: Duration::from_millis(400) };
        let w = ThemeWatcher::spawn(options, move |r| {
            let _ = tx.send(r);
        })
        .unwrap();
        // İzleme kurulsun (ilk tur parmak izi alır, olay beklemeye geçer).
        let deadline = Instant::now() + Duration::from_secs(5);
        while !w.wake.armed.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        (w, rx)
    }

    fn icon_theme(root: &Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.theme"), format!("[Icon Theme]\nName={name}\nDirectories=16x16\n")).unwrap();
    }

    fn next(rx: &mpsc::Receiver<u64>) -> u64 {
        rx.recv_timeout(Duration::from_secs(5)).expect("sayaç artmadı")
    }

    fn quiet(rx: &mpsc::Receiver<u64>) {
        assert!(rx.recv_timeout(TEST_SETTLE * 6).is_err(), "fazladan artış");
    }

    #[test]
    fn roots_follow_xdg_order() {
        let r = roots_from(Some(Path::new("/h")), None, Some("/a:rel:/a"));
        let r: Vec<&str> = r.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(r, ["/h/.icons", "/h/.local/share/icons", "/a/icons", "/h/.themes", "/h/.local/share/themes", "/a/themes"]);
        let r = roots_from(None, Some(Path::new("/d")), None);
        assert!(r.contains(&PathBuf::from("/usr/share/icons")) && r.contains(&PathBuf::from("/d/themes")));
    }

    /// Ekleme, patlama, ilgisiz dosya, index.theme değişimi, silme.
    #[test]
    fn theme_changes_bump_rev() {
        let tmp = tempfile::tempdir().unwrap();
        let icons = tmp.path().join("icons");
        std::fs::create_dir_all(&icons).unwrap();
        icon_theme(&icons, "Old");
        let (w, rx) = spawn(vec![icons.clone()]);
        assert_eq!(w.rev(), 0);

        icon_theme(&icons, "New");
        assert_eq!(next(&rx), 1);
        quiet(&rx);

        // Paket kurulumu gibi patlama: tek artış.
        for i in 0..30 {
            icon_theme(&icons, &format!("Burst{i}"));
            std::fs::create_dir_all(icons.join(format!("Burst{i}/16x16/apps"))).unwrap();
        }
        assert_eq!(next(&rx), 2);
        quiet(&rx);

        // İlgisiz: arama dizininde dosya, tema içinde işaret olmayan dosya ve alt dizin.
        std::fs::write(icons.join("README"), "x").unwrap();
        std::fs::write(icons.join("New/notes.txt"), "x").unwrap();
        std::fs::create_dir_all(icons.join("New/32x32")).unwrap();
        std::fs::write(icons.join("New/32x32/a.svg"), "x").unwrap();
        quiet(&rx);

        // index.theme içeriği değişince.
        std::fs::write(icons.join("New/index.theme"), "[Icon Theme]\nName=Yeni ad\nDirectories=16x16,32x32\n").unwrap();
        assert_eq!(next(&rx), 3);

        // Silme ve yeniden adlandırma.
        std::fs::remove_dir_all(icons.join("Old")).unwrap();
        assert_eq!(next(&rx), 4);
        std::fs::rename(icons.join("New"), icons.join("Renamed")).unwrap();
        assert_eq!(next(&rx), 5);
        assert_eq!(w.rev(), 5);

        let started = Instant::now();
        drop(w);
        assert!(started.elapsed() < Duration::from_secs(1), "kapanış hızlı");
    }

    /// Yok olan dizin panik etmez; sonradan oluşan ve silinip yeniden oluşan dizin izlenir.
    #[test]
    fn missing_and_recreated_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let user = home.join(".local/share/icons"); // henüz yok (iki düzey eksik)
        let gone = tmp.path().join("hiç/yok/themes");
        let (w, rx) = spawn(vec![user.clone(), gone]);

        // Üst dizindeki ilgisiz dosya uyandırmaz/artırmaz.
        std::fs::write(home.join(".bash_history"), "x").unwrap();
        quiet(&rx);

        // Dizin zinciri sonradan oluşur, içine tema gelir.
        std::fs::create_dir_all(&user).unwrap();
        icon_theme(&user, "Papirus");
        next(&rx);
        // Zincir adım adım oluştuysa tema ikinci turda görünebilir: artışlar dinene kadar bekle.
        while rx.recv_timeout(TEST_SETTLE * 6).is_ok() {}
        let base = w.rev();
        assert!(base >= 1);

        // İzlenen dizin silinip yeniden oluşturulunca toparlanır.
        std::fs::remove_dir_all(&user).unwrap();
        assert_eq!(next(&rx), base + 1);
        std::fs::create_dir_all(&user).unwrap();
        quiet(&rx); // boş dizin: liste değişmedi
        icon_theme(&user, "Vimix");
        assert_eq!(next(&rx), base + 2);
        drop(w);
    }

    #[test]
    fn gtk_theme_dir_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let themes = tmp.path().join("themes");
        std::fs::create_dir_all(themes.join("adw-gtk3")).unwrap();
        let (w, rx) = spawn(vec![themes.clone()]);
        std::fs::create_dir_all(themes.join("adw-gtk3/gtk-3.0")).unwrap();
        assert_eq!(next(&rx), 1);
        std::fs::write(themes.join("adw-gtk3/gtk-3.0/gtk.css"), "x").unwrap();
        quiet(&rx);
        drop(w);
    }
}
