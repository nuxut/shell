//! GTK yapılandırma dosyaları izleyicisi (kendini onarma, kullanıcı isteği 2026-10-17): birleşik
//! görünüm açıkken `~/.config/gtk-3.0` ve `~/.config/gtk-4.0` altındaki Nuxut dosyaları
//! ([`NAMES`]: `gtk.css`, `gtk-dark.css`, `assets`, `settings.ini`, `nuxut.css`) başka bir araç
//! (tema yöneticisi, nwg-look, kurulum betiği) tarafından değiştirilince ya da yeniden bağlanınca
//! kabuk bir doğrulama turu ister: `nuxut-apply` aynı planı yeniden uygular, değişmemişse `NoOp`
//! (yazım yok), değişmişse yedekleyip yeniden devralır. Kendi yazımlarımız da olay üretir; onların
//! turu `NoOp` döner ve döngü kurmaz.
//!
//! Tek thread (`nuxut-gtk-watch`), boşta uyur. Dizin yoksa en yakın üst dizin izlenir (yalnız
//! yoldaki sonraki bileşen). Olaylar [`SETTLE`] sessizlikle birleştirilir; parmak izi (her adın
//! türü, bağ hedefi, inode, boy, zaman) değişmediyse çağrılmaz.

use std::ffi::OsStr;
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

/// İzlenen adlar (iki dizinde de).
pub const NAMES: [&str; 5] = ["gtk.css", "gtk-dark.css", "assets", "settings.ini", "nuxut.css"];
/// İzlenen dizinler (`$XDG_CONFIG_HOME` altında).
pub const DIRS: [&str; 2] = ["gtk-3.0", "gtk-4.0"];
/// Olay patlamasından sonra beklenen sessizlik.
pub const SETTLE: Duration = Duration::from_millis(700);

#[derive(Debug)]
struct Wake {
    fd: OwnedFd,
    stop: AtomicBool,
    armed: AtomicBool,
}

/// Çalışan izleyici; düşürülünce thread durur.
#[derive(Debug)]
pub struct GtkWatcher {
    wake: Arc<Wake>,
    thread: Option<JoinHandle<()>>,
}

impl GtkWatcher {
    /// `config`: `$XDG_CONFIG_HOME`. `sink`: dosyalar değişince (izleyici thread'inden).
    pub fn spawn<F>(config: PathBuf, settle: Duration, sink: F) -> std::io::Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        use rustix::event::{EventfdFlags, eventfd};
        let fd = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        let wake = Arc::new(Wake { fd, stop: AtomicBool::new(false), armed: AtomicBool::new(false) });
        let thread = {
            let wake = wake.clone();
            std::thread::Builder::new().name("nuxut-gtk-watch".into()).spawn(move || run(&config, settle, &wake, &sink))?
        };
        Ok(Self { wake, thread: Some(thread) })
    }
}

impl Drop for GtkWatcher {
    fn drop(&mut self) {
        self.wake.stop.store(true, Ordering::SeqCst);
        let _ = rustix::io::write(&self.wake.fd, &1u64.to_ne_bytes());
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("GTK izleyici thread'i panikle sonlandı");
        }
    }
}

/// Bir adın özeti (`lstat`; bağ izlenmez).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mark {
    Missing,
    Link(PathBuf),
    Entry { ino: u64, len: u64, modified: Option<SystemTime>, dir: bool },
}

fn mark(path: &Path) -> Mark {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(path) {
        Err(_) => Mark::Missing,
        Ok(m) if m.file_type().is_symlink() => Mark::Link(std::fs::read_link(path).unwrap_or_default()),
        Ok(m) => Mark::Entry { ino: m.ino(), len: m.len(), modified: m.modified().ok(), dir: m.is_dir() },
    }
}

fn fingerprint(config: &Path) -> Vec<Mark> {
    DIRS.iter().flat_map(|d| NAMES.iter().map(move |n| mark(&config.join(d).join(n)))).collect()
}

struct Watch {
    fd: OwnedFd,
    /// (izleme, ilgili adlar: `None` = [`NAMES`], `Some(ad)` = üst dizinde yalnız o alt dizin).
    roles: Vec<(i32, Option<&'static str>)>,
}

impl Watch {
    fn new(config: &Path) -> Option<Self> {
        use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
        let fd = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK).ok()?;
        let files = WatchFlags::CREATE
            | WatchFlags::DELETE
            | WatchFlags::MOVED_FROM
            | WatchFlags::MOVED_TO
            | WatchFlags::CLOSE_WRITE
            | WatchFlags::ATTRIB
            | WatchFlags::DELETE_SELF
            | WatchFlags::MOVE_SELF
            | WatchFlags::ONLYDIR;
        let parent = WatchFlags::CREATE | WatchFlags::MOVED_TO | WatchFlags::ONLYDIR | WatchFlags::MASK_ADD;
        let mut roles = Vec::new();
        for d in DIRS {
            let dir = config.join(d);
            if dir.is_dir() {
                if let Ok(wd) = inotify::add_watch(&fd, &dir, files) {
                    roles.push((wd, None));
                }
            } else if config.is_dir()
                && let Ok(wd) = inotify::add_watch(&fd, config, parent)
            {
                roles.push((wd, Some(d)));
            }
        }
        Some(Self { fd, roles })
    }

    fn drain(&self) -> bool {
        use rustix::fs::inotify::{ReadFlags, Reader};
        let mut buf = [MaybeUninit::<u8>::uninit(); 4096];
        let mut reader = Reader::new(&self.fd, &mut buf);
        let mut relevant = false;
        while let Ok(event) = reader.next() {
            let flags = event.events();
            if flags.contains(ReadFlags::QUEUE_OVERFLOW) {
                relevant = true;
                continue;
            }
            let name = event.file_name().map(|n| OsStr::from_bytes(n.to_bytes()));
            let self_event = flags.intersects(ReadFlags::DELETE_SELF | ReadFlags::MOVE_SELF | ReadFlags::IGNORED);
            relevant |= self.roles.iter().filter(|(wd, _)| *wd == event.wd()).any(|(_, role)| match role {
                None => self_event || name.is_some_and(|n| NAMES.iter().any(|m| OsStr::new(m) == n)),
                Some(dir) => name == Some(OsStr::new(dir)),
            });
        }
        relevant
    }
}

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

fn run(config: &Path, settle: Duration, wake: &Wake, sink: &dyn Fn()) {
    let mut last: Option<Vec<Mark>> = None;
    loop {
        let Some(watch) = Watch::new(config) else {
            tracing::warn!("GTK dosya izleyicisi kurulamadı; dış değişiklikler yalnız görünüm değişince onarılır");
            return;
        };
        let now = fingerprint(config);
        if last.as_ref().is_some_and(|l| *l != now) {
            tracing::debug!("GTK yapılandırma dosyaları değişti");
            sink();
        }
        last = Some(now);
        wake.armed.store(true, Ordering::SeqCst);
        loop {
            let ready = wait(&watch.fd, wake, None);
            if wake.stop.load(Ordering::SeqCst) {
                return;
            }
            if ready && watch.drain() {
                break;
            }
        }
        // Birleştirme: `settle` sessizlik.
        while wait(&watch.fd, wake, Some(settle)) {
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
    use std::time::Instant;

    fn spawn(config: &Path) -> (GtkWatcher, mpsc::Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        let w = GtkWatcher::spawn(config.to_path_buf(), Duration::from_millis(60), move || {
            let _ = tx.send(());
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !w.wake.armed.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        (w, rx)
    }

    fn fired(rx: &mpsc::Receiver<()>) -> bool {
        rx.recv_timeout(Duration::from_secs(3)).is_ok()
    }

    fn quiet(rx: &mpsc::Receiver<()>) -> bool {
        rx.recv_timeout(Duration::from_millis(400)).is_err()
    }

    /// Bir aracın Nuxut dosyasını bağla değiştirmesi, içerik yazması ve dizinin sonradan
    /// yaratılması çağrı üretir; ilgisiz dosya ve aynı içerik üretmez.
    #[test]
    fn relink_and_rewrite_fire_unrelated_does_not() {
        let t = tempfile::tempdir().unwrap();
        let cfg = t.path();
        let (_w, rx) = spawn(cfg);
        // gtk-4.0 sonradan yaratılır (üst dizin izlenir).
        std::fs::create_dir(cfg.join("gtk-4.0")).unwrap();
        std::fs::write(cfg.join("gtk-4.0/gtk.css"), "/* nuxut */").unwrap();
        assert!(fired(&rx), "dizin + dosya");
        while rx.try_recv().is_ok() {}
        std::fs::write(cfg.join("gtk-4.0/notlar.txt"), "x").unwrap();
        assert!(quiet(&rx), "ilgisiz dosya");
        // Başka bir araç bağı yeniden kurar.
        std::fs::remove_file(cfg.join("gtk-4.0/gtk.css")).unwrap();
        std::os::unix::fs::symlink("/usr/share/themes/Dracula/gtk-4.0/gtk.css", cfg.join("gtk-4.0/gtk.css")).unwrap();
        assert!(fired(&rx), "yeniden bağlama");
    }
}
