//! Beyaz liste denetimli yazma katmanı ve işlem (transaction) günlüğü.
//!
//! Tüm değiştirici işlemler buradan geçer: önce [`policy::allowed`], sonra [`safefs`].
//! [`Tx`] bir hedefin uygulanması/geri alınması boyunca yapılanları tutar: başarıda geçici
//! yedekleri temizler, hatada her adımı ters sırayla geri alır.

use nuxut_boot_theme::ThemeFiles;
use nuxut_boot_theme::detect::{Probe, Read};

use crate::policy;
use crate::safefs::{self, Dir, FsError, Root};

/// İşlem hatası.
#[derive(Debug)]
pub enum OpError {
    Fs(FsError),
    /// Beyaz liste dışı yol (programlama hatası ya da oynanmış girdi).
    Denied(String),
    Other(String),
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fs(e) => e.fmt(f),
            Self::Denied(p) => write!(f, "/{p}: yardımcının yazma listesinde değil"),
            Self::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for OpError {}

impl From<FsError> for OpError {
    fn from(e: FsError) -> Self {
        Self::Fs(e)
    }
}

impl From<String> for OpError {
    fn from(e: String) -> Self {
        Self::Other(e)
    }
}

pub type Result<T> = std::result::Result<T, OpError>;

fn check(rel: &str) -> Result<()> {
    if policy::allowed(rel) { Ok(()) } else { Err(OpError::Denied(rel.to_owned())) }
}

/// Saatten `YYYYmmddHHMMSS` (UTC; chrono'suz, Howard Hinnant'ın sivil tarih algoritması).
pub fn timestamp(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}{:02}{:02}{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

fn now() -> String {
    timestamp(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") }
}

/// Kök üstünde denetimli işlemler.
#[derive(Debug)]
pub struct Ops {
    pub root: Root,
}

impl Ops {
    pub fn new(root: Root) -> Self {
        Self { root }
    }

    fn parent(&self, rel: &str) -> Result<(Dir, String)> {
        let (dir, name) = safefs::split(rel)?;
        Ok((self.root.dir(dir)?, name.to_owned()))
    }

    pub fn read(&self, rel: &str, max: usize) -> Result<Option<Vec<u8>>> {
        let (dir, name) = safefs::split(rel)?;
        match self.root.dir(dir) {
            Ok(d) => Ok(d.read(name, max)?),
            Err(FsError::Io { err, .. }) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.root.exists(rel)
    }

    pub fn size(&self, rel: &str) -> Result<Option<u64>> {
        let (d, name) = self.parent(rel)?;
        Ok(d.file_size(&name)?)
    }

    pub fn free_bytes(&self, dir: &str) -> Result<u64> {
        Ok(self.root.dir(dir)?.free_bytes()?)
    }

    pub fn write(&self, rel: &str, bytes: &[u8]) -> Result<()> {
        check(rel)?;
        let (d, name) = self.parent(rel)?;
        Ok(d.write_atomic(&name, bytes, 0o644)?)
    }

    /// Yedek kopyası (`<ad>.nuxut-<zaman>[-n].bak`, var olanın üstüne asla yazılmaz). Dosya
    /// yoksa `None`.
    pub fn backup(&self, rel: &str) -> Result<Option<String>> {
        check(rel)?;
        let (d, name) = self.parent(rel)?;
        if !d.is_file(&name) {
            return Ok(None);
        }
        let stamp = now();
        for n in 0..100 {
            let bname = if n == 0 { format!("{name}.nuxut-{stamp}.bak") } else { format!("{name}.nuxut-{stamp}-{n}.bak") };
            if d.exists(&bname) {
                continue;
            }
            let brel = join(&d.rel, &bname);
            check(&brel)?;
            d.copy(&name, &bname)?;
            return Ok(Some(brel));
        }
        Err(OpError::Other(format!("/{rel}: yedek adı bulunamadı")))
    }

    /// Aynı dizinde yeniden adlandırma (yedeği yerine koyma vb.).
    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        check(from)?;
        check(to)?;
        let (fd, fname) = safefs::split(from)?;
        let (td, tname) = safefs::split(to)?;
        if fd != td {
            return Err(OpError::Other(format!("/{from} → /{to}: farklı dizin")));
        }
        Ok(self.root.dir(fd)?.rename(fname, tname)?)
    }

    pub fn remove_file(&self, rel: &str) -> Result<()> {
        check(rel)?;
        match self.parent(rel) {
            Ok((d, name)) => Ok(d.remove_file(&name)?),
            Err(OpError::Fs(FsError::Io { err, .. })) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn remove_dir(&self, rel: &str) -> Result<()> {
        check(rel)?;
        let (dir, name) = safefs::split(rel)?;
        match self.root.dir(dir) {
            Ok(d) => Ok(d.remove_flat_dir(name)?),
            Err(FsError::Io { err, .. }) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn ensure_dir(&self, rel: &str) -> Result<()> {
        self.root.ensure_dir(rel, 0o755)?;
        Ok(())
    }

    /// Tema dizinini `rel`'in geçici kardeşi `.<ad>.nuxut-new` olarak kurar ve dosyaları yazar.
    /// Dönen: geçici dizinin yolu.
    pub fn stage_dir(&self, rel: &str, files: &ThemeFiles) -> Result<String> {
        check(rel)?;
        let (parent, name) = safefs::split(rel)?;
        let tmp = join(parent, &format!(".{name}.nuxut-new"));
        check(&tmp)?;
        let pd = self.root.dir(parent)?;
        pd.remove_flat_dir(&format!(".{name}.nuxut-new"))?;
        let d = pd.mkdir(&format!(".{name}.nuxut-new"), 0o755)?;
        for (fname, bytes) in &files.files {
            check(&join(&tmp, fname))?;
            d.write_atomic(fname, bytes, 0o644)?;
        }
        Ok(tmp)
    }
}

/// [`Ops`] üstünde algılama erişimi (doğrulanmış yürüyüşle okur).
#[derive(Debug)]
pub struct OpsProbe<'a>(pub &'a Ops);

impl Probe for OpsProbe<'_> {
    fn read(&self, path: &str, max: usize) -> Option<Read> {
        match self.0.read(path, max) {
            Ok(Some(d)) => Some(Read::Data(d)),
            Ok(None) => Some(Read::Missing),
            Err(OpError::Fs(FsError::TooLarge { .. })) => None,
            Err(_) => Some(Read::Denied),
        }
    }

    fn exists(&self, path: &str) -> bool {
        self.0.exists(path)
    }

    fn list(&self, dir: &str, max: usize) -> Vec<String> {
        self.0.root.dir(dir).and_then(|d| d.list(max)).unwrap_or_default()
    }
}

/// Geri alınabilir adım.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Undo {
    /// Bu işlemde kurulan dizini sil.
    RemoveDir(String),
    /// Bu işlemde kurulan dosyayı sil.
    RemoveFile(String),
    /// `from` → `to` geri adlandır (eski dizini/yedeği yerine koy).
    Rename { from: String, to: String },
    /// İçeriği geri yaz (küçük dosyalar).
    Write { path: String, bytes: Vec<u8> },
}

/// Bir hedefin işlemi.
#[derive(Debug, Default)]
pub struct Tx {
    undo: Vec<Undo>,
    /// Başarıda silinecekler (işleme özel yedekler, eski dizinler).
    cleanup_files: Vec<String>,
    cleanup_dirs: Vec<String>,
}

impl Tx {
    pub fn on_fail_remove_dir(&mut self, rel: &str) {
        self.undo.push(Undo::RemoveDir(rel.to_owned()));
    }

    pub fn on_fail_remove_file(&mut self, rel: &str) {
        self.undo.push(Undo::RemoveFile(rel.to_owned()));
    }

    pub fn on_fail_rename(&mut self, from: &str, to: &str) {
        self.undo.push(Undo::Rename { from: from.to_owned(), to: to.to_owned() });
    }

    pub fn on_fail_write(&mut self, path: &str, bytes: Vec<u8>) {
        self.undo.push(Undo::Write { path: path.to_owned(), bytes });
    }

    pub fn on_success_remove_file(&mut self, rel: &str) {
        self.cleanup_files.push(rel.to_owned());
    }

    pub fn on_success_remove_dir(&mut self, rel: &str) {
        self.cleanup_dirs.push(rel.to_owned());
    }

    /// Başarı: temizlik. Temizlik hataları yalnız uyarıdır (sistem doğru durumda).
    pub fn commit(self, ops: &Ops) -> Vec<String> {
        let mut warnings = Vec::new();
        for f in &self.cleanup_files {
            if let Err(e) = ops.remove_file(f) {
                warnings.push(e.to_string());
            }
        }
        for d in &self.cleanup_dirs {
            if let Err(e) = ops.remove_dir(d) {
                warnings.push(e.to_string());
            }
        }
        warnings
    }

    /// Hata: her adımı ters sırayla geri al. Dönen: geri alınamayan adımların hataları
    /// (boşsa sistem işlem öncesi durumda).
    pub fn rollback(self, ops: &Ops) -> Vec<String> {
        let mut errors = Vec::new();
        for u in self.undo.into_iter().rev() {
            let r = match &u {
                Undo::RemoveDir(d) => ops.remove_dir(d),
                Undo::RemoveFile(f) => ops.remove_file(f),
                Undo::Rename { from, to } => ops.rename(from, to),
                Undo::Write { path, bytes } => ops.write(path, bytes),
            };
            if let Err(e) = r {
                errors.push(e.to_string());
            }
        }
        errors
    }
}

/// Tema dizinini `rel` yerine koyar (geçici dizinden). Var olan dizin: `keep_previous` ise kalıcı
/// yedek adına (dönen `Some`), değilse `.<ad>.nuxut-old`'a alınır ve başarıda silinir.
pub fn swap_dir(ops: &Ops, tx: &mut Tx, rel: &str, staged: &str, keep_previous: bool) -> Result<Option<String>> {
    let (parent, name) = safefs::split(rel)?;
    tx.on_fail_remove_dir(staged);
    let mut previous = None;
    if ops.exists(rel) {
        let aside = if keep_previous {
            join(parent, &format!("{name}.nuxut-{}.bak", now()))
        } else {
            let old = join(parent, &format!(".{name}.nuxut-old"));
            ops.remove_dir(&old)?;
            old
        };
        ops.rename(rel, &aside)?;
        tx.on_fail_rename(&aside, rel);
        if keep_previous {
            previous = Some(aside);
        } else {
            tx.on_success_remove_dir(&aside);
        }
    }
    ops.rename(staged, rel)?;
    // Yerine konan dizin artık `rel`'de: hata olursa önce o silinir (ters sırada en son eklenen
    // ilk çalışır), sonra eski dizin geri adlandırılır.
    tx.undo.retain(|u| *u != Undo::RemoveDir(staged.to_owned()));
    tx.undo.push(Undo::RemoveDir(rel.to_owned()));
    Ok(previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn timestamps() {
        assert_eq!(timestamp(0), "19700101000000");
        assert_eq!(timestamp(1_792_152_000), "20261016120000");
        assert_eq!(timestamp(951_782_400), "20000229000000");
    }

    fn root() -> (tempfile::TempDir, Ops) {
        let t = tempfile::tempdir().unwrap();
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        for d in ["etc/plymouth", "usr/share/plymouth/themes", "boot/grub"] {
            std::fs::create_dir_all(t.path().join(d)).unwrap();
        }
        let r = Root::open(t.path().to_str().unwrap(), rustix::process::getuid().as_raw()).unwrap();
        (t, Ops::new(r))
    }

    #[test]
    fn writes_are_whitelisted() {
        let (_t, ops) = root();
        assert!(matches!(ops.write("etc/passwd", b"x"), Err(OpError::Denied(_))));
        ops.write("etc/plymouth/plymouthd.conf", b"[Daemon]\n").unwrap();
        let b = ops.backup("etc/plymouth/plymouthd.conf").unwrap().unwrap();
        assert!(b.starts_with("etc/plymouth/plymouthd.conf.nuxut-") && b.ends_with(".bak"));
        let b2 = ops.backup("etc/plymouth/plymouthd.conf").unwrap().unwrap();
        assert_ne!(b, b2, "ikinci yedek ayrı ad alır");
        assert_eq!(ops.backup("boot/grub/grub.cfg").unwrap(), None);
    }

    #[test]
    fn swap_dir_commit_and_rollback() {
        let (t, ops) = root();
        let theme = "usr/share/plymouth/themes/nuxut";
        let mut files = ThemeFiles::default();
        files.push("a.png", b"1".to_vec());
        // Bizim olmayan eski dizin: kalıcı yedeğe alınır.
        std::fs::create_dir(t.path().join(theme)).unwrap();
        std::fs::write(t.path().join(theme).join("eski.png"), b"0").unwrap();
        let staged = ops.stage_dir(theme, &files).unwrap();
        let mut tx = Tx::default();
        let prev = swap_dir(&ops, &mut tx, theme, &staged, true).unwrap().unwrap();
        assert!(ops.exists(&format!("{theme}/a.png")) && ops.exists(&format!("{prev}/eski.png")));
        // Hata: yeni dizin silinir, eski geri gelir.
        assert!(tx.rollback(&ops).is_empty());
        assert!(ops.exists(&format!("{theme}/eski.png")) && !ops.exists(&format!("{theme}/a.png")));
        assert!(!ops.exists(&prev));
        // Başarı (bizim önceki dizinimiz): eski silinir.
        let staged = ops.stage_dir(theme, &files).unwrap();
        let mut tx = Tx::default();
        assert_eq!(swap_dir(&ops, &mut tx, theme, &staged, false).unwrap(), None);
        assert!(tx.commit(&ops).is_empty());
        assert!(ops.exists(&format!("{theme}/a.png")));
        assert!(!ops.exists("usr/share/plymouth/themes/.nuxut.nuxut-old"));
    }
}
