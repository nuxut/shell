//! Doğrulanmış dosya işlemleri (kök yetkiyle çalışırken sembolik bağ ve TOCTOU savunması).
//!
//! Kurallar:
//! - Her yol kökten bileşen bileşen `openat(O_DIRECTORY | O_NOFOLLOW)` ile yürünür; her dizin
//!   beklenen sahibe (üretimde root) ait olmalı ve grup/diğerlerine yazılamaz olmalı (sudo ve
//!   OpenSSH'ın denetimi). Sembolik bağ bileşeni = hata; sonradan değiştirilen bir yol eski
//!   tanıtıcıyı etkilemez (işlemler tanıtıcıya göre: `openat`/`renameat`/`unlinkat`).
//! - Dosya okuma `O_NOFOLLOW`, yalnız düzenli dosya, boyut sınırlı.
//! - Yazma: aynı dizinde `O_CREAT | O_EXCL | O_NOFOLLOW` geçici dosya → yaz → fsync →
//!   `renameat` → dizin fsync. Yarım dosya hiçbir zaman hedef adda görünmez.
//! - Silme yalnız çağıranın verdiği dizin tanıtıcısında, derinlik sınırlı, bağ izlemeden.
//!
//! Hangi yolların yazılabileceğine bu katman değil [`crate::policy`] karar verir.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;

use rustix::fs::{AtFlags, CWD, FileType, Mode, OFlags};

/// Dosya işlemi hatası (yol köke göre).
#[derive(Debug)]
pub enum FsError {
    Io { path: String, err: io::Error },
    /// Güvensiz yol: sembolik bağ, yanlış sahip, herkese yazılabilir dizin, düzenli olmayan dosya.
    Unsafe { path: String, why: &'static str },
    TooLarge { path: String },
    BadName(String),
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, err } => write!(f, "/{path}: {err}"),
            Self::Unsafe { path, why } => write!(f, "/{path}: güvensiz ({why})"),
            Self::TooLarge { path } => write!(f, "/{path}: çok büyük"),
            Self::BadName(n) => write!(f, "geçersiz ad: {n:?}"),
        }
    }
}

impl std::error::Error for FsError {}

pub type Result<T> = std::result::Result<T, FsError>;

fn io_err(path: &str, err: impl Into<io::Error>) -> FsError {
    FsError::Io { path: path.to_owned(), err: err.into() }
}

/// Tek bileşen ad denetimi: boş değil, `/` yok, `.`/`..` değil, NUL yok, en çok 255 bayt.
pub fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 255 || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(FsError::BadName(name.to_owned()));
    }
    Ok(())
}

/// Köke göre yolu bileşenlerine ayırır (`a/b/c`; baştaki/sondaki `/` yok, `..` yok).
pub fn components(rel: &str) -> Result<Vec<&str>> {
    if rel.is_empty() {
        return Ok(vec![]);
    }
    rel.split('/').map(|c| check_name(c).map(|()| c)).collect()
}

/// Yolu `(dizin, ad)` olarak böler.
pub fn split(rel: &str) -> Result<(&str, &str)> {
    let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
    check_name(name)?;
    Ok((dir, name))
}

/// Kök: üretimde `/`, testte geçici bir dizin. `owner`: dizinlerin beklenen sahibi.
#[derive(Debug)]
pub struct Root {
    fd: OwnedFd,
    owner: u32,
    /// Dış komutlara verilecek mutlak yolların öneki (üretimde boş).
    prefix: String,
}

/// Doğrulanmış dizin tanıtıcısı.
#[derive(Debug)]
pub struct Dir {
    fd: OwnedFd,
    /// Köke göre yol (hata metinleri ve politika için).
    pub rel: String,
    owner: u32,
}

fn verify_dir(fd: &OwnedFd, owner: u32, path: &str) -> Result<()> {
    let st = rustix::fs::fstat(fd).map_err(|e| io_err(path, e))?;
    if FileType::from_raw_mode(st.st_mode) != FileType::Directory {
        return Err(FsError::Unsafe { path: path.to_owned(), why: "dizin değil" });
    }
    if st.st_uid != owner {
        return Err(FsError::Unsafe { path: path.to_owned(), why: "beklenmeyen sahip" });
    }
    if st.st_mode & 0o022 != 0 {
        return Err(FsError::Unsafe { path: path.to_owned(), why: "grup/diğerleri yazabiliyor" });
    }
    Ok(())
}

const DIR_FLAGS: OFlags = OFlags::DIRECTORY.union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC).union(OFlags::RDONLY);

impl Root {
    /// `path` mutlak kök dizini. Kökün kendisi de doğrulanır.
    pub fn open(path: &str, owner: u32) -> Result<Self> {
        let fd = rustix::fs::openat(CWD, path, DIR_FLAGS, Mode::empty()).map_err(|e| io_err(path, e))?;
        verify_dir(&fd, owner, "")?;
        let prefix = path.trim_end_matches('/').to_owned();
        Ok(Self { fd, owner, prefix })
    }

    pub fn owner(&self) -> u32 {
        self.owner
    }

    /// Köke göre yolun dış komutlara verilecek mutlak hâli.
    pub fn abs(&self, rel: &str) -> String {
        format!("{}/{rel}", self.prefix)
    }

    /// Kök dizinin kendisi.
    pub fn root_dir(&self) -> Result<Dir> {
        let fd = self.fd.try_clone().map_err(|e| io_err("", e))?;
        Ok(Dir { fd, rel: String::new(), owner: self.owner })
    }

    /// Var olan dizini doğrulayarak açar.
    pub fn dir(&self, rel: &str) -> Result<Dir> {
        self.walk(rel, None)
    }

    /// Dizini açar; eksik bileşenleri `mode` ile kurar.
    pub fn ensure_dir(&self, rel: &str, mode: u32) -> Result<Dir> {
        self.walk(rel, Some(mode))
    }

    fn walk(&self, rel: &str, create: Option<u32>) -> Result<Dir> {
        let mut cur = self.fd.try_clone().map_err(|e| io_err("", e))?;
        let mut path = String::new();
        for comp in components(rel)? {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(comp);
            let next = match rustix::fs::openat(&cur, comp, DIR_FLAGS, Mode::empty()) {
                Ok(fd) => fd,
                Err(e) if e == rustix::io::Errno::NOENT && create.is_some() => {
                    let mode = Mode::from_raw_mode(create.unwrap_or(0o755));
                    match rustix::fs::mkdirat(&cur, comp, mode) {
                        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                        Err(e) => return Err(io_err(&path, e)),
                    }
                    rustix::fs::openat(&cur, comp, DIR_FLAGS, Mode::empty()).map_err(|e| io_err(&path, e))?
                }
                Err(rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) => {
                    return Err(FsError::Unsafe { path, why: "sembolik bağ ya da dizin değil" });
                }
                Err(e) => return Err(io_err(&path, e)),
            };
            verify_dir(&next, self.owner, &path)?;
            cur = next;
        }
        Ok(Dir { fd: cur, rel: rel.to_owned(), owner: self.owner })
    }

    /// Yol var mı (bağ izlenmez; ara dizinler doğrulanmaz — yalnız algılama için).
    pub fn exists(&self, rel: &str) -> bool {
        components(rel).is_ok() && rustix::fs::statat(&self.fd, rel, AtFlags::SYMLINK_NOFOLLOW).is_ok()
    }
}

impl Dir {
    fn path(&self, name: &str) -> String {
        if self.rel.is_empty() { name.to_owned() } else { format!("{}/{name}", self.rel) }
    }

    /// Düzenli dosya mı (bağ izlenmez).
    pub fn is_file(&self, name: &str) -> bool {
        check_name(name).is_ok()
            && rustix::fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .is_ok_and(|st| FileType::from_raw_mode(st.st_mode) == FileType::RegularFile)
    }

    pub fn exists(&self, name: &str) -> bool {
        check_name(name).is_ok() && rustix::fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW).is_ok()
    }

    /// Alt dizini doğrulayarak açar.
    pub fn subdir(&self, name: &str) -> Result<Dir> {
        check_name(name)?;
        let path = self.path(name);
        let fd = rustix::fs::openat(&self.fd, name, DIR_FLAGS, Mode::empty()).map_err(|e| match e {
            rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR => FsError::Unsafe { path: path.clone(), why: "sembolik bağ ya da dizin değil" },
            e => io_err(&path, e),
        })?;
        verify_dir(&fd, self.owner, &path)?;
        Ok(Dir { fd, rel: path, owner: self.owner })
    }

    fn open_file(&self, name: &str) -> Result<Option<File>> {
        check_name(name)?;
        let path = self.path(name);
        match rustix::fs::openat(&self.fd, name, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK, Mode::empty()) {
            Ok(fd) => {
                let st = rustix::fs::fstat(&fd).map_err(|e| io_err(&path, e))?;
                if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
                    return Err(FsError::Unsafe { path, why: "düzenli dosya değil" });
                }
                Ok(Some(File::from(fd)))
            }
            Err(rustix::io::Errno::NOENT) => Ok(None),
            Err(rustix::io::Errno::LOOP) => Err(FsError::Unsafe { path, why: "sembolik bağ" }),
            Err(e) => Err(io_err(&path, e)),
        }
    }

    /// Dosyanın tamamı (en çok `max` bayt); yoksa `None`.
    pub fn read(&self, name: &str, max: usize) -> Result<Option<Vec<u8>>> {
        let Some(f) = self.open_file(name)? else { return Ok(None) };
        let mut buf = Vec::new();
        f.take(max as u64 + 1).read_to_end(&mut buf).map_err(|e| io_err(&self.path(name), e))?;
        if buf.len() > max {
            return Err(FsError::TooLarge { path: self.path(name) });
        }
        Ok(Some(buf))
    }

    /// Dosyanın sahibi beklenen sahip mi (değiştireceğimiz dosyalar için).
    pub fn owned_file(&self, name: &str) -> Result<bool> {
        let Some(f) = self.open_file(name)? else { return Ok(false) };
        let st = rustix::fs::fstat(f.as_fd()).map_err(|e| io_err(&self.path(name), e))?;
        Ok(st.st_uid == self.owner)
    }

    /// Dosya boyutu (yoksa `None`).
    pub fn file_size(&self, name: &str) -> Result<Option<u64>> {
        let Some(f) = self.open_file(name)? else { return Ok(None) };
        let st = rustix::fs::fstat(f.as_fd()).map_err(|e| io_err(&self.path(name), e))?;
        Ok(Some(st.st_size as u64))
    }

    fn create_excl(&self, name: &str, mode: u32) -> Result<File> {
        let fd = rustix::fs::openat(
            &self.fd,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(mode),
        )
        .map_err(|e| io_err(&self.path(name), e))?;
        Ok(File::from(fd))
    }

    fn temp_name(name: &str) -> String {
        format!(".{name}.nuxut-tmp-{}", std::process::id())
    }

    /// Atomik yazma (bkz. modül belgesi). Hedef bir dizin ya da bağsa hata.
    pub fn write_atomic(&self, name: &str, bytes: &[u8], mode: u32) -> Result<()> {
        check_name(name)?;
        if self.exists(name) && !self.is_file(name) {
            return Err(FsError::Unsafe { path: self.path(name), why: "düzenli dosya değil" });
        }
        let tmp = Self::temp_name(name);
        let _ = rustix::fs::unlinkat(&self.fd, tmp.as_str(), AtFlags::empty());
        let res = (|| {
            let mut f = self.create_excl(&tmp, mode)?;
            f.write_all(bytes).map_err(|e| io_err(&self.path(&tmp), e))?;
            f.sync_all().map_err(|e| io_err(&self.path(&tmp), e))?;
            drop(f);
            rustix::fs::renameat(&self.fd, tmp.as_str(), &self.fd, name).map_err(|e| io_err(&self.path(name), e))?;
            self.sync()
        })();
        if res.is_err() {
            let _ = rustix::fs::unlinkat(&self.fd, tmp.as_str(), AtFlags::empty());
        }
        res
    }

    /// `name`'i aynı dizinde `to` adıyla kopyalar (akışla; `to` yoksa kurulur, varsa hata).
    pub fn copy(&self, name: &str, to: &str) -> Result<()> {
        check_name(to)?;
        let Some(mut src) = self.open_file(name)? else {
            return Err(io_err(&self.path(name), io::Error::from(io::ErrorKind::NotFound)));
        };
        let mode = rustix::fs::fstat(src.as_fd()).map(|st| st.st_mode & 0o777).unwrap_or(0o600);
        let mut dst = self.create_excl(to, mode)?;
        let res = io::copy(&mut src, &mut dst).and_then(|_| dst.sync_all()).map_err(|e| io_err(&self.path(to), e));
        if res.is_err() {
            let _ = rustix::fs::unlinkat(&self.fd, to, AtFlags::empty());
        }
        res?;
        self.sync()
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        check_name(from)?;
        check_name(to)?;
        rustix::fs::renameat(&self.fd, from, &self.fd, to).map_err(|e| io_err(&self.path(to), e))?;
        self.sync()
    }

    /// Dosya siler (yoksa sorun değil).
    pub fn remove_file(&self, name: &str) -> Result<()> {
        check_name(name)?;
        match rustix::fs::unlinkat(&self.fd, name, AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => Ok(()),
            Err(e) => Err(io_err(&self.path(name), e)),
        }
    }

    pub fn mkdir(&self, name: &str, mode: u32) -> Result<Dir> {
        check_name(name)?;
        rustix::fs::mkdirat(&self.fd, name, Mode::from_raw_mode(mode)).map_err(|e| io_err(&self.path(name), e))?;
        self.subdir(name)
    }

    /// Düz bir dizini (yalnız düzenli dosyalar; alt dizin yok) siler. Bağ izlenmez; dizinde
    /// dosya olmayan bir şey varsa durur.
    pub fn remove_flat_dir(&self, name: &str) -> Result<()> {
        let sub = match self.subdir(name) {
            Ok(d) => d,
            Err(FsError::Io { err, .. }) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for entry in sub.list(4096)? {
            let st = rustix::fs::statat(&sub.fd, entry.as_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(|e| io_err(&sub.path(&entry), e))?;
            match FileType::from_raw_mode(st.st_mode) {
                FileType::RegularFile | FileType::Symlink => sub.remove_file(&entry)?,
                _ => return Err(FsError::Unsafe { path: sub.path(&entry), why: "beklenmeyen alt dizin" }),
            }
        }
        rustix::fs::unlinkat(&self.fd, name, AtFlags::REMOVEDIR).map_err(|e| io_err(&self.path(name), e))?;
        self.sync()
    }

    /// Dizindeki adlar (sıralı, `.`/`..` hariç, en çok `max`).
    pub fn list(&self, max: usize) -> Result<Vec<String>> {
        let fd = rustix::fs::openat(&self.fd, ".", DIR_FLAGS, Mode::empty()).map_err(|e| io_err(&self.rel, e))?;
        let mut dir = rustix::fs::Dir::read_from(&fd).map_err(|e| io_err(&self.rel, e))?;
        let mut out = Vec::new();
        while let Some(entry) = dir.read() {
            let entry = entry.map_err(|e| io_err(&self.rel, e))?;
            let name = OsStr::from_bytes(entry.file_name().to_bytes());
            let Some(name) = name.to_str() else { continue };
            if name == "." || name == ".." {
                continue;
            }
            out.push(name.to_owned());
            if out.len() >= max {
                break;
            }
        }
        out.sort();
        Ok(out)
    }

    /// Dizin girdilerini diske indirir.
    pub fn sync(&self) -> Result<()> {
        rustix::fs::fsync(&self.fd).map_err(|e| io_err(&self.rel, e))
    }

    /// Bu dizinin dosya sistemindeki boş alan (bayt; ayrıcalıksız kullanılabilir).
    pub fn free_bytes(&self) -> Result<u64> {
        let st = rustix::fs::fstatvfs(&self.fd).map_err(|e| io_err(&self.rel, e))?;
        Ok(st.f_bavail.saturating_mul(st.f_frsize))
    }

    /// `flock` tanıtıcısı (süreç boyunca tutulur).
    pub fn lock_file(&self, name: &str) -> Result<OwnedFd> {
        check_name(name)?;
        let fd = rustix::fs::openat(
            &self.fd,
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|e| io_err(&self.path(name), e))?;
        rustix::fs::flock(&fd, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| io_err(&self.path(name), io::Error::new(io::ErrorKind::WouldBlock, "başka bir Nuxut sistem işlemi sürüyor")))?;
        Ok(fd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn me() -> u32 {
        rustix::process::getuid().as_raw()
    }

    fn tree() -> (tempfile::TempDir, Root) {
        let t = tempfile::tempdir().unwrap();
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir_all(t.path().join("etc/x")).unwrap();
        let root = Root::open(t.path().to_str().unwrap(), me()).unwrap();
        (t, root)
    }

    #[test]
    fn names_are_checked() {
        assert!(components("a/b").is_ok());
        for bad in ["/a", "a/../b", "a//b", "a/", ".", "a\0b"] {
            assert!(components(bad).is_err(), "{bad:?}");
        }
        assert_eq!(split("a/b/c").unwrap(), ("a/b", "c"));
        assert_eq!(split("c").unwrap(), ("", "c"));
    }

    #[test]
    fn write_read_copy_remove() {
        let (_t, root) = tree();
        let d = root.dir("etc/x").unwrap();
        d.write_atomic("f", b"bir", 0o644).unwrap();
        d.write_atomic("f", b"iki", 0o644).unwrap();
        assert_eq!(d.read("f", 10).unwrap().unwrap(), b"iki");
        assert!(matches!(d.read("f", 2), Err(FsError::TooLarge { .. })));
        assert_eq!(d.read("yok", 10).unwrap(), None);
        d.copy("f", "f.bak").unwrap();
        assert!(d.copy("f", "f.bak").is_err(), "var olan hedefin üstüne kopyalanmaz");
        assert_eq!(d.list(10).unwrap(), vec!["f", "f.bak"]);
        let sub = d.mkdir("tema", 0o755).unwrap();
        sub.write_atomic("a.png", b"x", 0o644).unwrap();
        d.remove_flat_dir("tema").unwrap();
        assert!(!d.exists("tema"));
        d.remove_flat_dir("tema").unwrap();
    }

    #[test]
    fn symlinks_are_refused() {
        let (t, root) = tree();
        std::os::unix::fs::symlink("/etc", t.path().join("etc/link")).unwrap();
        assert!(matches!(root.dir("etc/link"), Err(FsError::Unsafe { .. })));
        std::os::unix::fs::symlink("/etc/passwd", t.path().join("etc/x/pw")).unwrap();
        let d = root.dir("etc/x").unwrap();
        assert!(matches!(d.read("pw", 1 << 20), Err(FsError::Unsafe { .. })));
        assert!(matches!(d.write_atomic("pw", b"x", 0o644), Err(FsError::Unsafe { .. })), "bağın üstüne yazılmaz");
        assert_ne!(std::fs::read("/etc/passwd").unwrap(), b"x");
        assert!(!d.is_file("pw"));
        assert!(d.copy("pw", "pw2").is_err(), "bağ kopyalanmaz");
    }

    #[test]
    fn world_writable_dir_is_refused() {
        let (t, root) = tree();
        std::fs::set_permissions(t.path().join("etc/x"), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(matches!(root.dir("etc/x"), Err(FsError::Unsafe { .. })));
    }

    #[test]
    fn ensure_dir_creates_missing() {
        let (_t, root) = tree();
        let d = root.ensure_dir("var/lib/nuxut/system-theme", 0o755).unwrap();
        d.write_atomic("m", b"{}", 0o644).unwrap();
        assert!(root.exists("var/lib/nuxut/system-theme/m"));
    }

    #[test]
    fn lock_is_exclusive() {
        let (_t, root) = tree();
        let d = root.dir("etc").unwrap();
        let _a = d.lock_file("l").unwrap();
        assert!(d.lock_file("l").is_err());
    }
}
