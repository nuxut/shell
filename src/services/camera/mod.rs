//! Kamera kullanım izleyicisi (gizlilik göstergesi): hangi süreçler bir V4L2 aygıtını
//! (`/dev/video*`) açık tutuyor.
//!
//! **Karar kuralı** (2026-10-16; `docs/design/privacy-indicator.md`): gösterge açık =
//! denge > 0 **veya** okunabilir sahip var **veya** dengesi belirsiz bir düğüm varken aygıtı
//! açabilecek okunamayan süreç var. Gerçeği hiçbir zaman eksik göstermez (fail-closed).
//!
//! - **Denge** ([`Tracker`]): `/dev` dizini (aygıt takıldı/çıktı) ve her `video*` düğümü inotify
//!   ile izlenir; düğüm (inode) başına `IN_OPEN` − `IN_CLOSE_*`. Başlangıç dengesi **yarışsız** bir
//!   taramayla kurulur (tarama sırasında o düğüme olay gelirse tarama tekrarlanır); sonra yalnız
//!   olaylarla değişir, negatife inmez. **Tam** taramada ([`Scan::complete`]) taramaya eşitlenir.
//!   Kapanış fazlası (denge 0'ken kapanış: denge eksikti) loglanır ve yeniden tarama tetikler.
//!   Aygıt çıkarılıp takılırsa yeni düğümün dengesi yine taramayla kurulur.
//! - **Tarama = atıf:** `/proc/<pid>/fd` bağlantıları **yalnızca `readlink`** ile; hedef `video*`
//!   ise sürecin adı (`/proc/<pid>/comm`). Aygıtın içeriği ya da süreç belleği okunmaz.
//! - **Okunamayan süreç** (`fd` dizini okunamıyor: başka uid, dumpable=0): kimliği
//!   (`/proc/<pid>/status`) düğümün izinleriyle (mod + POSIX ACL) karşılaştırılır; aygıtı
//!   açabiliyorsa **şüpheli** sayılır ([`Scan::suspects`]). root tehdit modeli dışı (kabuğu
//!   ptrace/kill edebilir); oturumun `(sd-pam)`'i (kendi `systemd --user`'ının ilk çocuğu) muaf.
//!   Şüpheli yalnız dengesi belirsiz bir düğüm varken göstergeyi yakar: belirsizlik, düğümün
//!   başlangıç dengesi şüpheli varken kurulduysa (izleme öncesi açmasını göremeyiz) ya da kuyruk
//!   taştıysa doğar, şüphelisiz tam taramada kalkar. Kabuktan sonra başlayan tarayıcı sandbox'ı
//!   göstergeyi boşuna yakmaz: onun açmaları zaten dengede.
//! - Boşta thread `poll`'da uyur; gösterge açıkken 5 sn'de bir doğrulama taraması.
//! - Kabuğun kendi süreci sayılmaz. Servis yalnızca bar'da `privacy` widget'ı kamerayı gösterirken
//!   çalışır (`app::privacy`).

use std::collections::{BTreeSet, HashMap};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Taranan en fazla süreç (bozuk/dev `/proc`'a karşı sınır).
const MAX_PROCESSES: usize = 65_536;
/// Süreç başına en fazla bakılan fd.
const MAX_FDS: usize = 16_384;
/// Bildirilen en fazla süreç adı.
pub const MAX_USERS: usize = 8;
/// Süreç adının en uzun hâli (çekirdek `comm` 15 bayt).
const MAX_NAME: usize = 32;
/// İzlenen en fazla `video*` düğümü.
const MAX_DEVICES: usize = 64;
/// Birleştirmenin en uzun süresi (sürekli açılıp kapanan aygıtta da sonuç gecikmesin).
const MAX_SETTLE: Duration = Duration::from_secs(2);
/// `/proc/<pid>/status|stat|cmdline` ve ACL okumasının üst sınırı.
const MAX_PROC_FILE: usize = 8192;
/// Okunan en fazla çocuk süreç (`(sd-pam)` denetimi).
const MAX_CHILDREN: usize = 4096;

#[derive(Debug, Clone)]
pub struct CameraOptions {
    /// Aygıt düğümlerinin dizini (`/dev`; testler geçici dizin verir).
    pub dev_dir: PathBuf,
    /// procfs kökü (`/proc`; testler sahte ağaç verir).
    pub proc_dir: PathBuf,
    /// Son olaydan sonra bu kadar sessizlik beklenir.
    pub settle: Duration,
    /// Gösterge açıkken kaçmış olaya karşı bu aralıkla yeniden doğrulanır (boşta yoklama yok).
    pub recheck: Duration,
}

impl Default for CameraOptions {
    fn default() -> Self {
        Self { dev_dir: "/dev".into(), proc_dir: "/proc".into(), settle: Duration::from_millis(300), recheck: Duration::from_secs(5) }
    }
}

/// Kamera kullanım durumu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CameraState {
    /// Kamerayı açık tuttuğu görülen süreçlerin adları (sıralı, tekil, en çok [`MAX_USERS`]).
    pub users: Vec<String>,
    /// Kullanımı taramanın gösteremediği bir kaynak var: dengede sahipsiz açma, kuyruk taşması ya
    /// da belirsiz denge + şüpheli okunamayan süreç. **Fail-closed:** gösterge yine yanar, ad
    /// "bilinmeyen".
    pub unattributed: bool,
}

impl CameraState {
    pub fn in_use(&self) -> bool {
        !self.users.is_empty() || self.unattributed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraEvent {
    /// Durum değişti (ve başlangıçta bir kez).
    State(CameraState),
}

/// Thread'i uyandıran/durduran eventfd.
#[derive(Debug)]
struct Wake {
    fd: OwnedFd,
    stop: AtomicBool,
}

impl Wake {
    fn new() -> std::io::Result<Self> {
        use rustix::event::{EventfdFlags, eventfd};
        Ok(Self { fd: eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?, stop: AtomicBool::new(false) })
    }
}

/// Çalışan izleyici; düşürülünce thread durur ve join edilir.
#[derive(Debug)]
pub struct CameraService {
    wake: Arc<Wake>,
    thread: Option<JoinHandle<()>>,
}

impl CameraService {
    pub fn spawn<F>(options: CameraOptions, sink: F) -> std::io::Result<Self>
    where
        F: Fn(CameraEvent) + Send + 'static,
    {
        let wake = Arc::new(Wake::new()?);
        let w = Arc::clone(&wake);
        let thread = std::thread::Builder::new().name("nuxut-camera".into()).spawn(move || run(&options, &w, &sink))?;
        Ok(Self { wake, thread: Some(thread) })
    }
}

impl Drop for CameraService {
    fn drop(&mut self) {
        self.wake.stop.store(true, Ordering::SeqCst);
        let _ = rustix::io::write(&self.wake.fd, &1u64.to_ne_bytes());
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("kamera izleyici thread'i panikle sonlandı");
        }
    }
}

/// `video` + rakamlar.
fn is_video_node(name: &str) -> bool {
    name.strip_prefix("video").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// inotify kurar: dizin (CREATE/DELETE/MOVED_*) + var olan düğümler (OPEN/CLOSE). Yeni düğüm
/// [`Tracker::watch`] ile yerleşmemiş olarak girer; dengesi bir sonraki yarışsız taramada kurulur.
/// Var olan düğüm için `add_watch` aynı wd'yi döndürür (yeniden adlandırmada yol güncellenir).
/// İzlenemeyen düğüm (izin) uyarıyla atlanır; tarama yine görür. `/dev`'de artık olmayan düğüm
/// izleyiciden düşer ([`Tracker::retain`]).
fn add_watches(fd: &OwnedFd, dev_dir: &Path, tracker: &mut Tracker) {
    use rustix::fs::inotify::{self, WatchFlags};
    let dir_flags = WatchFlags::CREATE | WatchFlags::DELETE | WatchFlags::MOVED_FROM | WatchFlags::MOVED_TO | WatchFlags::ONLYDIR;
    let _ = inotify::add_watch(fd, dev_dir, dir_flags);
    let Ok(read) = std::fs::read_dir(dev_dir) else { return };
    let node_flags = WatchFlags::OPEN | WatchFlags::CLOSE_WRITE | WatchFlags::CLOSE_NOWRITE;
    let mut seen = Vec::new();
    for item in read.flatten().filter(|e| e.file_name().to_str().is_some_and(is_video_node)).take(MAX_DEVICES) {
        let path = item.path();
        match inotify::add_watch(fd, &path, node_flags) {
            Ok(wd) => {
                tracker.watch(wd, &path);
                seen.push(wd);
            }
            Err(e) => tracing::warn!(error = %e, "kamera düğümü izlenemiyor; yalnız taramayla görülür"),
        }
    }
    tracker.retain(&seen);
}

/// Bir düğümün inotify olayı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeEvent {
    Open,
    Close,
    /// İzleme bitti (`IN_IGNORED`: düğüm silindi, aygıt çıkarıldı).
    Gone,
}

#[derive(Debug)]
struct Node {
    /// Düğümün güncel yolu (taramanın `per_device` anahtarı).
    path: PathBuf,
    /// Açık dosya açıklaması dengesi. Sınırsız (u64, 2026-10-16): üst sınırda kırpmak bilgi
    /// kaybettirir — sınırdan fazla açıp kapanışlarla sayacı sıfırlayan süreç, fd'leri hâlâ açıkken
    /// göstergeyi söndürürdü. Çekirdek açılış başına tek kapanış ürettiği için denge sınırsız da kesin.
    open: u64,
    /// Başlangıç dengesi yarışsız bir taramayla kuruldu.
    settled: bool,
    /// Taramanın başından beri bu düğüme olay geldi: tarama bu düğüm için yarışlı.
    dirty: bool,
    /// Denge eksik olabilir: başlangıç dengesi şüpheli (okunamayan, aygıtı açabilen süreç) varken
    /// ya da yarım taramayla kuruldu. Şüphelisiz tam taramada kalkar.
    uncertain: bool,
    /// Denge sayacı tavana dayandı (u64; pratikte erişilemez, savunma): o andan sonraki açmalar
    /// sayılamadı, denge artık alt sınır bile değil. Yapışkan: yalnız bu düğüm için yarışsız, tam
    /// ve şüphelisiz bir taramada kalkar (denge o taramadan kurulur); o zamana dek gösterge açık.
    saturated: bool,
}

/// Düğüm (inode) başına açık dosya açıklaması dengesi ve karar (2026-10-16). Çekirdek
/// her açılış için tek kapanış üretir (dup/fork/SCM_RIGHTS aynı açıklamayı paylaşır); bu yüzden
/// denge izleme başladıktan sonraki açmalar için kesindir. IO'suz: olaylar ve taramalar dışarıdan
/// verilir (birim/özellik testleri doğrudan sürer).
#[derive(Debug, Default)]
struct Tracker {
    nodes: HashMap<i32, Node>,
    /// Kuyruk taştı (`IN_Q_OVERFLOW`): olaylar kayboldu. Fail-closed: gösterge bir sonraki yarışsız
    /// tam taramaya dek açık kalır; o taramada denge taramadan kurulur. Taşma ~16 bin bekleyen olay
    /// ister: pratikte yalnız kasıtlı aç-kapa seli (gizleme girişimi).
    overflowed: bool,
    /// Taramanın başından beri kuyruk taştı (tarama hiçbir düğüm için yarışsız değil).
    overflow_in_scan: bool,
    /// Görülen kapanış fazlası sayısı (tanı ve test).
    excess_closes: u64,
    /// Kapanış fazlası görüldü: sonraki tur beklemeden tarar.
    rescan: bool,
}

impl Tracker {
    /// Düğüm izleniyor. Yeni wd yerleşmemiş girer; bilinen wd'nin yolu güncellenir (yeniden adlandırma
    /// inode'u değiştirmez, denge korunur).
    fn watch(&mut self, wd: i32, path: &Path) {
        match self.nodes.get_mut(&wd) {
            Some(n) if n.path != path => n.path = path.to_owned(),
            Some(_) => {}
            None => {
                self.nodes.insert(wd, Node { path: path.to_owned(), open: 0, settled: false, dirty: false, uncertain: false, saturated: false });
            }
        }
    }

    /// `/dev`'de görülen düğümler dışındakiler düşer: aygıt çıkarıldı (`IN_IGNORED` taşmada
    /// kaybolmuş ya da düğüm silinmiş ama eski fd'ler hâlâ açık olabilir). Aygıt yoksa yakalama
    /// da yok; bayat denge göstergeyi takılı bırakmasın.
    fn retain(&mut self, seen: &[i32]) {
        self.nodes.retain(|wd, _| seen.contains(wd));
    }

    fn event(&mut self, wd: i32, event: NodeEvent) {
        let Some(n) = self.nodes.get_mut(&wd) else { return };
        n.dirty = true;
        match event {
            // Aygıt yoksa yakalama da yok; yeniden takılırsa yeni düğüm taramayla kurulur.
            NodeEvent::Gone => {
                self.nodes.remove(&wd);
            }
            NodeEvent::Open => match n.open.checked_add(1) {
                Some(v) => n.open = v,
                None => {
                    if !std::mem::replace(&mut n.saturated, true) {
                        tracing::warn!(path = %n.path.display(), "kamera: denge sayacı doydu; tam taramaya dek gösterge açık");
                    }
                }
            },
            NodeEvent::Close if n.open > 0 => n.open -= 1,
            NodeEvent::Close => {
                // Denge eksikti: izleme öncesi açılmış, taramanın görmediği bir fd kapandı. 0'ın
                // altına inilmez; yerleşmiş düğümde loglanır ve hemen yeniden taranır.
                if n.settled {
                    self.excess_closes += 1;
                    self.rescan = true;
                    tracing::warn!(path = %n.path.display(), "kamera: kapanış fazlası (denge eksikti); yeniden taranıyor");
                }
            }
        }
    }

    fn overflow(&mut self) {
        if !std::mem::replace(&mut self.overflowed, true) {
            tracing::warn!("kamera inotify kuyruğu taştı; kullanım doğrulanamıyor, gösterge açık kalacak");
        }
        self.overflow_in_scan = true;
    }

    /// Taramadan hemen önce: bundan sonra gelen olaylar taramayı o düğüm için yarışlı yapar.
    fn begin_scan(&mut self) {
        for n in self.nodes.values_mut() {
            n.dirty = false;
        }
        self.overflow_in_scan = false;
        self.rescan = false;
    }

    /// Taramadan (ve tarama sırasında gelen olaylar uygulandıktan) sonra. Yarışsız düğümde:
    /// yerleşmemişse denge taramadan kurulur; tarama tamsa her düğüm taramaya eşitlenir, belirsizlik
    /// ve taşma kalkar. Yarışlı düğüm dokunulmadan kalır (olaylar uygulanmıştır). Döner: yerleşmemiş
    /// düğüm kaldı (tarama tekrarlanmalı).
    fn reconcile(&mut self, scan: &Scan) -> bool {
        let complete = scan.complete();
        let mut all_clean = !self.overflow_in_scan;
        for n in self.nodes.values_mut() {
            if self.overflow_in_scan || n.dirty {
                all_clean = false;
                continue;
            }
            if complete {
                // Tam tarama dengeyi yükseltebilir; düşürmesi yalnız taşmadan sonra (kapanışlar
                // kaybolmuş olabilir). Aksi hâlde olaylar kesindir ve taramanın hiç göremediği
                // sahip (`hidepid`, kabuğun kendisi) dengeden silinmez.
                let found = scan.found(&n.path);
                // Doymuş denge kaybolan açmaları sayamadı: taşmadaki gibi tarama esastır.
                n.open = if self.overflowed || n.saturated { found } else { n.open.max(found) };
                n.uncertain = false;
                n.saturated = false;
            } else if !n.settled {
                // İzlemeden beri biriken (0'da kırpılmış) denge, izleme sonrası açılıp hâlâ açık
                // fd'lerden fazla olamaz; tarama da gerçekten fazlasını görmez: büyüğü alt sınırdır
                // (taramanın göremediği okunamayan açmalar korunur).
                n.open = n.open.max(scan.found(&n.path));
                n.uncertain = true;
            } else {
                continue;
            }
            n.settled = true;
        }
        if complete && all_clean && self.overflowed {
            self.overflowed = false;
            tracing::info!("kamera: taşmadan sonra yarışsız tam tarama; denge yeniden kuruldu");
        }
        self.nodes.values().any(|n| !n.settled)
    }

    /// Karar: denge > 0, okunabilir sahip, belirsiz denge + şüpheli ya da taşma.
    fn state(&self, scan: &Scan) -> CameraState {
        let unaccounted = self.nodes.values().any(|n| n.open > scan.found(&n.path));
        let uncertain = scan.suspects > 0 && self.nodes.values().any(|n| n.uncertain || !n.settled);
        let saturated = self.nodes.values().any(|n| n.saturated);
        CameraState { users: scan.users.clone(), unattributed: self.overflowed || unaccounted || uncertain || saturated }
    }

    /// Bekleyen inotify olaylarını uygular.
    fn drain(&mut self, fd: &OwnedFd) {
        use rustix::fs::inotify::{ReadFlags, Reader};
        let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); 4096];
        let mut reader = Reader::new(fd, &mut buf);
        while let Ok(e) = reader.next() {
            let flags = e.events();
            let event = if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
                self.overflow();
                continue;
            } else if flags.contains(ReadFlags::IGNORED) {
                NodeEvent::Gone
            } else if flags.contains(ReadFlags::OPEN) {
                NodeEvent::Open
            } else if flags.intersects(ReadFlags::CLOSE_WRITE | ReadFlags::CLOSE_NOWRITE) {
                NodeEvent::Close
            } else {
                // Dizin olayları (CREATE/DELETE/MOVED_*): yalnız uyandırır, izleme turun başında.
                continue;
            };
            self.event(e.wd(), event);
        }
    }
}

fn wait(fds: &[&OwnedFd], timeout: Option<Duration>) -> bool {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let ts = timeout.map(|d| Timespec { tv_sec: d.as_secs() as i64, tv_nsec: d.subsec_nanos() as i64 });
    let mut polls: Vec<PollFd<'_>> = fds.iter().map(|fd| PollFd::new(*fd, PollFlags::IN)).collect();
    loop {
        match poll(&mut polls, ts.as_ref()) {
            Ok(n) => return n > 0,
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return false,
        }
    }
}

fn run(options: &CameraOptions, wake: &Wake, sink: &dyn Fn(CameraEvent)) {
    use rustix::fs::inotify::{self, CreateFlags};
    let inotify = match inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK) {
        Ok(fd) => fd,
        Err(e) => {
            tracing::warn!(error = %e, "inotify açılamadı; kamera göstergesi çalışmayacak");
            return;
        }
    };
    let me = std::process::id();
    let mut tracker = Tracker::default();
    let mut last: Option<CameraState> = None;
    // Üst üste yarışlı tarama sayısı (yerleşmemiş düğüm varken geri çekilme).
    let mut retries = 0u32;
    loop {
        add_watches(&inotify, &options.dev_dir, &mut tracker);
        tracker.begin_scan();
        let found = scan(&options.dev_dir, &options.proc_dir, me);
        // Tarama sırasında gelen olaylar: uygulanır ve o düğümü bu tur için yarışlı yapar.
        tracker.drain(&inotify);
        let pending = tracker.reconcile(&found);
        let state = tracker.state(&found);
        if last.as_ref() != Some(&state) {
            tracing::debug!(users = state.users.len(), unattributed = state.unattributed, suspects = found.suspects, "kamera kullanımı değişti");
            sink(CameraEvent::State(state.clone()));
        }
        // Boşta tamamen uyur; gösterge açıkken seyrek doğrulama; yerleşmemiş düğüm ya da kapanış
        // fazlası varsa kısa aralıkla (geri çekilerek) yeniden tarar.
        retries = if pending { retries.saturating_add(1) } else { 0 };
        let timeout = if pending || tracker.rescan {
            Some(options.settle.saturating_mul(1 << retries.min(4)).min(options.recheck))
        } else {
            state.in_use().then_some(options.recheck)
        };
        last = Some(state);
        wait(&[&wake.fd, &inotify], timeout);
        // Birleştirme: `settle` sessizlik ya da en çok MAX_SETTLE.
        let first = Instant::now();
        loop {
            if wake.stop.load(Ordering::SeqCst) {
                return;
            }
            tracker.drain(&inotify);
            let left = MAX_SETTLE.saturating_sub(first.elapsed());
            if left.is_zero() || !wait(&[&wake.fd, &inotify], Some(options.settle.min(left))) {
                break;
            }
        }
        if wake.stop.load(Ordering::SeqCst) {
            return;
        }
    }
}

/// Tarama sonucu (atıf ve dengenin kurulması; karar [`Tracker`]'da).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Scan {
    /// Kamerayı açık tutan (okunabilen) süreçlerin adları.
    pub users: Vec<String>,
    /// Bulunan `video*` fd'si sayısı.
    pub fds: u32,
    /// Düğüm başına bulunan fd sayısı.
    pub per_device: HashMap<PathBuf, u32>,
    /// fd dizini okunamayan süreçler (başka uid, dumpable=0); tanı için.
    pub unreadable: u32,
    /// Okunamayan ve bir `video*` düğümünü açabilecek süreçler (root ve oturumun `(sd-pam)`'i
    /// hariç; kimliği okunamayan da şüphelidir). Kamerayı tutuyor olabilirler.
    pub suspects: u32,
    /// Tarama yarım kaldı: procfs okunamadı ya da süreç/fd sınırı aşıldı.
    pub partial: bool,
}

impl Scan {
    /// Kamerayı tutabilecek her sürecin her fd'si görüldü: sonuç gerçeğin kendisi.
    pub fn complete(&self) -> bool {
        !self.partial && self.suspects == 0
    }

    fn found(&self, path: &Path) -> u64 {
        self.per_device.get(path).copied().map_or(0, u64::from)
    }
}

/// `/proc/<pid>/fd` bağlantılarından `dev_dir/video*` açık tutan süreçler. Yalnız `readlink`;
/// içerik okunmaz.
pub fn scan(dev_dir: &Path, proc_dir: &Path, skip_pid: u32) -> Scan {
    let Ok(procs) = std::fs::read_dir(proc_dir) else { return Scan { partial: true, ..Scan::default() } };
    let nodes = node_access(dev_dir);
    let mut out = Scan::default();
    let mut names = BTreeSet::new();
    let mut procs = procs.flatten();
    for entry in procs.by_ref().take(MAX_PROCESSES) {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if pid == skip_pid {
            continue;
        }
        let fds = match std::fs::read_dir(entry.path().join("fd")) {
            Ok(fds) => fds,
            // Süreç bu arada çıktı.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                out.unreadable = out.unreadable.saturating_add(1);
                if is_suspect(proc_dir, pid, &nodes) {
                    out.suspects = out.suspects.saturating_add(1);
                }
                continue;
            }
        };
        let mut held = 0u32;
        let mut fds = fds.flatten();
        for fd in fds.by_ref().take(MAX_FDS) {
            let Ok(target) = std::fs::read_link(fd.path()) else { continue };
            if target.parent() == Some(dev_dir) && target.file_name().and_then(|n| n.to_str()).is_some_and(is_video_node) {
                held = held.saturating_add(1);
                let n = out.per_device.entry(target).or_insert(0);
                *n = n.saturating_add(1);
            }
        }
        out.partial |= fds.next().is_some();
        if held > 0 {
            out.fds = out.fds.saturating_add(held);
            names.insert(process_name(&entry.path()).unwrap_or_else(|| pid.to_string()));
        }
    }
    out.partial |= procs.next().is_some();
    out.users = names.into_iter().take(MAX_USERS).collect();
    out
}

/// Okunamayan süreç kamerayı tutuyor olabilir mi: kimliği okunamazsa evet (fail-closed); root
/// hayır (tehdit modeli dışı); oturumun `(sd-pam)`'i hayır; aksi hâlde bir düğümü açabiliyorsa.
fn is_suspect(proc_dir: &Path, pid: u32, nodes: &[NodeAccess]) -> bool {
    if nodes.is_empty() {
        return false;
    }
    let dir = proc_dir.join(pid.to_string());
    let Some(creds) = read_small(&dir.join("status")).and_then(|s| Creds::parse(&s)) else { return true };
    if creds.fsuid == 0 || is_session_pam(proc_dir, pid, &creds) {
        return false;
    }
    nodes.iter().any(|n| n.allows(&creds))
}

/// Oturumun `(sd-pam)`'i: adı `(sd-pam)`, ebeveyni aynı uid'in `systemd --user`'ı ve o ebeveynin
/// **en erken başlamış** çocuğu (`(sd-pam)`, ebeveyn `systemd`'yi exec etmeden önce çatallanır;
/// sonradan başlayan bir birim adını taklit etse de ilk çocuk olamaz). Bilgi okunamazsa muaf değil.
fn is_session_pam(proc_dir: &Path, pid: u32, creds: &Creds) -> bool {
    let dir = proc_dir.join(pid.to_string());
    if read_small(&dir.join("comm")).is_none_or(|c| c.trim_end() != "(sd-pam)") {
        return false;
    }
    let Some((ppid, start)) = read_small(&dir.join("stat")).and_then(|s| parse_stat(&s)) else { return false };
    let parent = proc_dir.join(ppid.to_string());
    let is_user_manager = read_small(&parent.join("comm")).is_some_and(|c| c.trim_end() == "systemd")
        && read_small(&parent.join("cmdline")).is_some_and(|c| c.split('\0').any(|a| a == "--user"))
        && read_small(&parent.join("status")).and_then(|s| Creds::parse(&s)).is_some_and(|p| p.ruid == creds.ruid);
    if !is_user_manager {
        return false;
    }
    let Some(children) = read_small(&parent.join("task").join(ppid.to_string()).join("children")) else { return false };
    let mut earliest: Option<(u64, u32)> = None;
    for child in children.split_whitespace().take(MAX_CHILDREN).filter_map(|c| c.parse::<u32>().ok()) {
        if let Some((_, s)) = read_small(&proc_dir.join(child.to_string()).join("stat")).and_then(|s| parse_stat(&s)) {
            earliest = Some(earliest.map_or((s, child), |e| e.min((s, child))));
        }
    }
    earliest == Some((start, pid))
}

/// Küçük procfs/sysfs dosyası (sınırlı, kayıplı UTF-8).
fn read_small(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_PROC_FILE as u64).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// `/proc/<pid>/stat` → (ppid, starttime). `comm` parantez/boşluk içerebilir: son `)`'dan sonrası.
fn parse_stat(stat: &str) -> Option<(u32, u64)> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    Some((fields.get(1)?.parse().ok()?, fields.get(19)?.parse().ok()?))
}

/// Sürecin dosya erişim kimliği (`/proc/<pid>/status`: herkese okunur).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Creds {
    ruid: u32,
    fsuid: u32,
    fsgid: u32,
    groups: Vec<u32>,
}

impl Creds {
    fn parse(status: &str) -> Option<Self> {
        let ids = |key: &str| -> Option<Vec<u32>> {
            let line = status.lines().find_map(|l| l.strip_prefix(key))?;
            line.split_whitespace().take(64).map(|v| v.parse().ok()).collect()
        };
        let uid = ids("Uid:")?;
        let gid = ids("Gid:")?;
        Some(Self { ruid: *uid.first()?, fsuid: *uid.get(3)?, fsgid: *gid.get(3)?, groups: ids("Groups:").unwrap_or_default() })
    }

    fn in_group(&self, gid: u32) -> bool {
        self.fsgid == gid || self.groups.contains(&gid)
    }
}

/// POSIX ACL girdisi (`system.posix_acl_access`, sürüm 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AclEntry {
    tag: u16,
    perm: u16,
    id: u32,
}

const ACL_USER_OBJ: u16 = 0x01;
const ACL_USER: u16 = 0x02;
const ACL_GROUP_OBJ: u16 = 0x04;
const ACL_GROUP: u16 = 0x08;
const ACL_MASK: u16 = 0x10;
const ACL_OTHER: u16 = 0x20;
/// Okuma ya da yazma: ikisinden biri aygıtı açmaya yeter.
const RW: u16 = 0o6;

fn parse_acl(raw: &[u8]) -> Option<Vec<AclEntry>> {
    let (head, body) = raw.split_at_checked(4)?;
    if u32::from_le_bytes(head.try_into().ok()?) != 2 || body.len() % 8 != 0 {
        return None;
    }
    Some(
        body.chunks_exact(8)
            .map(|c| AclEntry { tag: u16::from_le_bytes([c[0], c[1]]), perm: u16::from_le_bytes([c[2], c[3]]), id: u32::from_le_bytes([c[4], c[5], c[6], c[7]]) })
            .collect(),
    )
}

/// Bir `video*` düğümünün erişim bilgisi.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeAccess {
    uid: u32,
    gid: u32,
    mode: u32,
    /// Boş: ACL yok, yalnız mod bitleri. `None`: ACL okunamadı/bozuk (herkes açabilir sayılır).
    acl: Option<Vec<AclEntry>>,
}

impl NodeAccess {
    /// Bu kimlik düğümü (okuma ya da yazma için) açabilir mi (POSIX ACL erişim algoritması).
    fn allows(&self, c: &Creds) -> bool {
        let Some(acl) = &self.acl else { return true };
        let mode = self.mode as u16;
        if acl.is_empty() {
            let perm = if c.fsuid == self.uid {
                mode >> 6
            } else if c.in_group(self.gid) {
                mode >> 3
            } else {
                mode
            };
            return perm & RW != 0;
        }
        let entry = |tag: u16| acl.iter().find(|e| e.tag == tag).map(|e| e.perm);
        let mask = entry(ACL_MASK).unwrap_or(0o7);
        if c.fsuid == self.uid {
            return entry(ACL_USER_OBJ).unwrap_or(mode >> 6) & RW != 0;
        }
        if let Some(e) = acl.iter().find(|e| e.tag == ACL_USER && e.id == c.fsuid) {
            return e.perm & mask & RW != 0;
        }
        let groups: Vec<u16> =
            acl.iter().filter(|e| (e.tag == ACL_GROUP_OBJ && c.in_group(self.gid)) || (e.tag == ACL_GROUP && c.in_group(e.id))).map(|e| e.perm).collect();
        if !groups.is_empty() {
            return groups.iter().any(|p| p & mask & RW != 0);
        }
        entry(ACL_OTHER).unwrap_or(mode) & RW != 0
    }
}

/// `dev_dir`'deki `video*` düğümlerinin erişim bilgisi (yalnız okunamayan süreç çıkınca okunur
/// diye tembel değil: düğüm sayısı küçük, tarama olay güdümlü).
fn node_access(dev_dir: &Path) -> Vec<NodeAccess> {
    use std::os::unix::fs::MetadataExt;
    let Ok(read) = std::fs::read_dir(dev_dir) else { return Vec::new() };
    read.flatten()
        .filter(|e| e.file_name().to_str().is_some_and(is_video_node))
        .take(MAX_DEVICES)
        .filter_map(|e| {
            let path = e.path();
            let meta = std::fs::metadata(&path).ok()?;
            let mut buf = vec![0u8; MAX_PROC_FILE];
            let acl = match rustix::fs::getxattr(&path, "system.posix_acl_access", &mut buf[..]) {
                Ok(n) => parse_acl(&buf[..n]),
                Err(rustix::io::Errno::NODATA | rustix::io::Errno::NOTSUP) => Some(Vec::new()),
                Err(_) => None,
            };
            Some(NodeAccess { uid: meta.uid(), gid: meta.gid(), mode: meta.mode(), acl })
        })
        .collect()
}

/// `comm` (kısa süreç adı): kontrol karakterleri atılır, uzunluk sınırlı.
fn process_name(proc_pid: &Path) -> Option<String> {
    let raw = std::fs::read(proc_pid.join("comm")).ok()?;
    let text = String::from_utf8_lossy(&raw[..raw.len().min(MAX_NAME * 4)]);
    let clean: String = text.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    (!clean.is_empty()).then_some(clean)
}

#[cfg(test)]
mod tests;
