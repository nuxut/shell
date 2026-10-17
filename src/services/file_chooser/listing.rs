//! Dizin okuma: sınırlı, sıralı, sembolik bağ politikalı. Bloklar (her girdi için `lstat`, bağda
//! `stat`; ağ dosya sisteminde uzun sürebilir) → yalnızca işçi thread'inde ([`Lister`]).
//!
//! Politika (docs/design/file-chooser.md "Güvenlik"): girdi `lstat` ile; bağ kendi adıyla
//! görünür, türü hedefin `stat`'ından (klasöre bağ gezilebilir; kırık bağ `Broken`). Dönen yol
//! kullanıcının gördüğü yoldur, kanonikleştirilmez.

use std::cmp::Ordering;
use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::SystemTime;

/// Bir klasörde okunan en çok girdi; fazlası atlanır ([`Listing::truncated`]).
pub const MAX_ENTRIES: usize = 50_000;
/// Görünen adın en büyük boyu (bayt); dosya adı zaten ≤ 255.
const MAX_DISPLAY: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    /// Aygıt, soket, FIFO: listelenir ama seçilemez/açılamaz.
    Special,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    No,
    /// Bağ; tür hedeften.
    Yes,
    /// Kırık bağ (hedef yok ya da döngü).
    Broken,
}

#[derive(Debug, Clone)]
pub struct Entry {
    /// Ham dosya adı (UTF-8 olmayabilir).
    pub name: OsString,
    /// Görünen ad: kayıplı UTF-8, denetim/yön karakterleri atılmış.
    pub display: String,
    pub kind: Kind,
    pub link: Link,
    /// Bayt (klasörde 0).
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Entry {
    pub fn is_hidden(&self) -> bool {
        self.name.as_bytes().first() == Some(&b'.')
    }

    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }

    /// Seçilebilir dosya mı (kırık bağ ve özel dosya değil).
    pub fn is_regular(&self) -> bool {
        self.kind == Kind::File && self.link != Link::Broken
    }
}

#[derive(Debug, Clone)]
pub struct Listing {
    pub dir: PathBuf,
    /// Sıralı ([`compare`]).
    pub entries: Vec<Entry>,
    /// [`MAX_ENTRIES`]'e ulaşıldı.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListError {
    NotFound,
    PermissionDenied,
    NotADirectory,
    Other,
}

impl ListError {
    fn from_io(e: &io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::NotADirectory => Self::NotADirectory,
            _ => Self::Other,
        }
    }
}

/// Görünen ad: kayıplı UTF-8, denetim/yön/sıfır genişlikli karakterler atılmış.
pub fn display_name(name: &std::ffi::OsStr) -> String {
    nuxut_portal_backend::chooser::protocol::clean_text(&name.to_string_lossy(), MAX_DISPLAY)
}

fn entry(dir: &Path, name: OsString) -> Entry {
    let path = dir.join(&name);
    let display = display_name(&name);
    let lmeta = std::fs::symlink_metadata(&path);
    let (meta, link) = match &lmeta {
        Ok(m) if m.file_type().is_symlink() => match std::fs::metadata(&path) {
            Ok(t) => (Some(t), Link::Yes),
            Err(_) => (None, Link::Broken),
        },
        Ok(m) => (Some(m.clone()), Link::No),
        Err(_) => (None, Link::No),
    };
    let (kind, size, modified) = match &meta {
        Some(m) if m.is_dir() => (Kind::Dir, 0, m.modified().ok()),
        Some(m) if m.is_file() => (Kind::File, m.len(), m.modified().ok()),
        Some(m) => (Kind::Special, 0, m.modified().ok()),
        // Kırık bağ: bağın kendi zamanı; okunamayan girdi dosya gibi görünür.
        None => (Kind::File, 0, lmeta.ok().and_then(|m| m.modified().ok())),
    };
    Entry { name, display, kind, link, size, modified }
}

/// Klasörü okur ve sıralar. `cancel`: yeni okuma başladıysa erken bırakır (`None`).
pub fn read_dir(dir: &Path, max: usize, cancel: &dyn Fn() -> bool) -> Result<Option<Listing>, ListError> {
    let iter = std::fs::read_dir(dir).map_err(|e| ListError::from_io(&e))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for item in iter {
        if cancel() {
            return Ok(None);
        }
        let Ok(item) = item else { continue };
        if entries.len() >= max {
            truncated = true;
            break;
        }
        entries.push(entry(dir, item.file_name()));
    }
    entries.sort_by(compare);
    Ok(Some(Listing { dir: dir.to_path_buf(), entries, truncated }))
}

/// Klasörler önce, sonra doğal sıra (`dosya2` < `dosya10`), harf duyarsız; eşitlikte ham bayt.
pub fn compare(a: &Entry, b: &Entry) -> Ordering {
    b.is_dir().cmp(&a.is_dir()).then_with(|| natural(&a.display, &b.display)).then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
}

/// Doğal sıralama: rakam dizileri sayı olarak (öndeki sıfırlar yok sayılır, sonra uzunluk),
/// diğer karakterler küçük harfe çevrilerek karşılaştırılır.
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (n, m) = (take(&mut x), take(&mut y));
                let (nt, mt) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = nt.len().cmp(&mt.len()).then_with(|| nt.cmp(mt)).then_with(|| n.len().cmp(&m.len()));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Okuma sonucu.
#[derive(Debug)]
pub struct ListResult {
    pub generation: u64,
    pub dir: PathBuf,
    pub result: Result<Listing, ListError>,
}

/// Arka plan okuyucu: her [`Lister::list`] yeni bir nesil başlatır; eski okumalar iptal edilir,
/// sonuçları gelmez. Okuma başına kısa ömürlü bir thread (boşta thread yok).
#[derive(Debug, Clone, Default)]
pub struct Lister {
    generation: Arc<AtomicU64>,
}

impl Lister {
    pub fn new() -> Self {
        Self::default()
    }

    /// Okumayı başlatır; nesli döner. `sink` işçi thread'inden çağrılır (eski nesiller çağırmaz).
    pub fn list(&self, dir: PathBuf, sink: impl FnOnce(ListResult) + Send + 'static) -> u64 {
        let generation = self.generation.fetch_add(1, AtomicOrdering::SeqCst) + 1;
        let current = Arc::clone(&self.generation);
        let spawned = std::thread::Builder::new().name("nuxut-fc-list".into()).spawn(move || {
            let stale = || current.load(AtomicOrdering::SeqCst) != generation;
            let result = match read_dir(&dir, MAX_ENTRIES, &stale) {
                Ok(Some(l)) => Ok(l),
                Ok(None) => return,
                Err(e) => Err(e),
            };
            if !stale() {
                sink(ListResult { generation, dir, result });
            }
        });
        if spawned.is_err() {
            tracing::warn!("dizin okuma thread'i başlatılamadı");
        }
        generation
    }

    pub fn current(&self) -> u64 {
        self.generation.load(AtomicOrdering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    use std::sync::mpsc;
    use std::time::Duration;

    fn names(l: &Listing) -> Vec<String> {
        l.entries.iter().map(|e| e.display.clone()).collect()
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["dosya10", "Dosya2", "dosya1", "a", "B", "dosya02", "z"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, ["a", "B", "dosya1", "Dosya2", "dosya02", "dosya10", "z"]);
        assert_eq!(natural("x99999999999999999999999", "x100000000000000000000000"), Ordering::Less);
    }

    #[test]
    fn reads_sorts_and_classifies() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        std::fs::create_dir(p.join("klasör")).unwrap();
        std::fs::write(p.join("b10.txt"), "xyz").unwrap();
        std::fs::write(p.join("b9.txt"), "").unwrap();
        std::fs::write(p.join(".gizli"), "").unwrap();
        std::os::unix::fs::symlink(p.join("klasör"), p.join("bağ")).unwrap();
        std::os::unix::fs::symlink(p.join("yok"), p.join("kırık")).unwrap();
        std::fs::write(p.join(OsString::from_vec(b"r\xe9\xe2\x80\xaeb".to_vec())), "").unwrap();
        let l = read_dir(p, MAX_ENTRIES, &|| false).unwrap().unwrap();
        assert_eq!(names(&l), ["bağ", "klasör", ".gizli", "b9.txt", "b10.txt", "kırık", "r\u{FFFD}b"]);
        let by = |n: &str| l.entries.iter().find(|e| e.display == n).unwrap();
        assert_eq!((by("bağ").kind, by("bağ").link), (Kind::Dir, Link::Yes));
        assert_eq!(by("kırık").link, Link::Broken);
        assert!(!by("kırık").is_regular());
        assert_eq!(by("b10.txt").size, 3);
        assert!(by(".gizli").is_hidden());
        // Ham ad korunur (yön karakteri yalnız görünen adda atıldı).
        assert_eq!(by("r\u{FFFD}b").name.as_bytes(), b"r\xe9\xe2\x80\xaeb");
        assert!(!l.truncated);
    }

    #[test]
    fn limit_and_errors() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(d.path().join(format!("f{i}")), "").unwrap();
        }
        let l = read_dir(d.path(), 3, &|| false).unwrap().unwrap();
        assert_eq!(l.entries.len(), 3);
        assert!(l.truncated);
        assert!(read_dir(d.path(), 3, &|| true).unwrap().is_none());
        assert_eq!(read_dir(&d.path().join("yok"), 3, &|| false).unwrap_err(), ListError::NotFound);
        assert_eq!(read_dir(&d.path().join("f0"), 3, &|| false).unwrap_err(), ListError::NotADirectory);
    }

    #[test]
    fn lister_drops_stale_generations() {
        let d = tempfile::tempdir().unwrap();
        let lister = Lister::new();
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        let g1 = lister.list(d.path().to_path_buf(), move |r| tx.send(r.generation).unwrap());
        let g2 = lister.list(d.path().to_path_buf(), move |r| tx2.send(r.generation).unwrap());
        assert!(g2 > g1);
        assert_eq!(lister.current(), g2);
        // İlk okuma ikinci başlamadan bitmiş olabilir; ama ikinciden sonra eski nesil gelmez.
        let mut got = Vec::new();
        while let Ok(g) = rx.recv_timeout(Duration::from_millis(300)) {
            got.push(g);
        }
        assert_eq!(got.last(), Some(&g2));
        assert!(got.iter().all(|&g| g == g1 || g == g2) && got.len() <= 2);
    }
}
