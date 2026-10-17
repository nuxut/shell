//! Güvenilmeyen kaynaktan (D-Bus istemcisi) gelen yoldaki dosyayı güvenle okuma. Bildirim
//! görselleri (`image-path`) ve tepsi ikonları (`IconThemePath`, mutlak `IconName`) kullanır.
//!
//! 1. Mutlak yol, `..` yok; `/proc`, `/sys`, `/dev` önekleri reddedilir.
//! 2. `O_NOFOLLOW | O_NONBLOCK | O_NOCTTY | O_CLOEXEC` ile **bir kez** açılır: son bileşen
//!    symlink ise açılmaz, FIFO açılışı beklemez.
//! 3. Açık tanıtıcı üzerinde `fstat` (normal dosya, boyut `1..=max_bytes`) ve `fstatfs` (sahte
//!    dosya sistemi değil) kontrol edilir.
//! 4. İçerik aynı tanıtıcıdan sınırla okunur (dosya bu arada büyürse reddedilir).
//!
//! Ara dizinlerdeki symlink'ler izlenir (`/home` → `/var/home` gibi meşru kurulumlar); bunlar
//! üzerinden sahte dosya sistemine varmak 3. adımda yakalanır. Bir dizinin *içinde* kalma
//! şartı gereken yerde çağıran ayrıca kanonik önek denetler ([`within_dir`]).
//!
//! Okuma bloklayan IO'dur: yalnızca arka plan thread'lerinde çağrılır.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use rustix::fs::{FileType, Mode, OFlags};

/// Dosya neden reddedildi (log için; yol log'a yazılmaz).
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("yol mutlak değil ya da `..` içeriyor")]
    Path,
    #[error("sahte/çekirdek dosya sistemi (/proc, /sys, /dev ...)")]
    PseudoFs,
    #[error("normal dosya değil (symlink, FIFO, aygıt, dizin)")]
    NotRegular,
    #[error("dosya boş ya da boyut sınırını aşıyor")]
    Size,
    #[error("açılamadı/okunamadı: {0}")]
    Io(#[from] std::io::Error),
}

/// Çekirdeğin sahte dosya sistemlerinin `statfs` sihirli sayıları: içerikleri görsel değildir,
/// okumak bilgi sızdırabilir (`/proc/<pid>/environ`) veya yan etki yapabilir.
const PSEUDO_FS_MAGIC: &[u32] = &[
    0x9fa0,      // proc
    0x6265_6572, // sysfs
    0x6462_6720, // debugfs
    0x7472_6163, // tracefs
    0x7363_6673, // securityfs
    0x0027_e0eb, // cgroup
    0x6367_7270, // cgroup2
    0x1cd1,      // devpts
    0xcafe_4a11, // bpf
    0x6265_6570, // configfs
    0xde5e_81e4, // efivarfs
    0x6165_676c, // pstore
    0x6573_5546, // fusectl
    0x4249_4e4d, // binfmt_misc
];

const PSEUDO_PREFIXES: [&str; 3] = ["/proc", "/sys", "/dev"];

/// Yol biçimi uygun mu (mutlak, `..` yok, sahte dosya sistemi öneki yok)?
pub fn check_path(path: &Path) -> Result<(), FileError> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(FileError::Path);
    }
    if PSEUDO_PREFIXES.iter().any(|p| path.starts_with(p)) {
        return Err(FileError::PseudoFs);
    }
    Ok(())
}

/// Dosyayı yukarıdaki kurallarla okur; içerik en fazla `max_bytes`.
pub fn read_untrusted(path: &Path, max_bytes: u64) -> Result<Vec<u8>, FileError> {
    check_path(path)?;
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC;
    let fd = rustix::fs::open(path, flags, Mode::empty()).map_err(|e| {
        // ELOOP: son bileşen symlink.
        if e == rustix::io::Errno::LOOP { FileError::NotRegular } else { FileError::Io(e.into()) }
    })?;
    let stat = rustix::fs::fstat(&fd).map_err(std::io::Error::from)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(FileError::NotRegular);
    }
    let statfs = rustix::fs::fstatfs(&fd).map_err(std::io::Error::from)?;
    // f_type mimariye göre farklı genişlikte; sihirli sayılar 32 bite sığar.
    if PSEUDO_FS_MAGIC.contains(&((statfs.f_type as u64 & 0xffff_ffff) as u32)) {
        return Err(FileError::PseudoFs);
    }
    let size = u64::try_from(stat.st_size).unwrap_or(u64::MAX);
    if size == 0 || size > max_bytes {
        return Err(FileError::Size);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    std::fs::File::from(fd).take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > max_bytes {
        return Err(FileError::Size);
    }
    Ok(bytes)
}

/// `dir` (kanonik) içindeki `relative` dosyasının yolu; ara dizinler kanonikleştirilir ve
/// sonuç `dir`'in içinde kalmalıdır (ara dizin symlink'iyle dışarı kaçış yok). Son bileşen
/// kanonikleştirilmez: [`read_untrusted`] onu `O_NOFOLLOW` ile açar. Bulunamazsa `None`.
pub fn within_dir(dir: &Path, relative: &Path) -> Option<PathBuf> {
    if relative.is_absolute() || relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    let file_name = relative.file_name()?;
    let parent = match relative.parent() {
        Some(p) if !p.as_os_str().is_empty() => dir.join(p).canonicalize().ok()?,
        _ => dir.to_path_buf(),
    };
    parent.starts_with(dir).then(|| parent.join(file_name))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn rejects_bad_paths_and_special_files() {
        assert!(matches!(read_untrusted(Path::new("relative.png"), 10), Err(FileError::Path)));
        assert!(matches!(read_untrusted(Path::new("/tmp/../etc/passwd"), 10), Err(FileError::Path)));
        assert!(matches!(read_untrusted(Path::new("/proc/self/environ"), 1 << 20), Err(FileError::PseudoFs)));
        assert!(matches!(read_untrusted(Path::new("/dev/zero"), 10), Err(FileError::PseudoFs)));

        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.png");
        std::fs::write(&real, b"data").unwrap();
        assert_eq!(read_untrusted(&real, 10).unwrap(), b"data");
        assert!(matches!(read_untrusted(&real, 3), Err(FileError::Size)));
        let link = dir.path().join("link.png");
        symlink(&real, &link).unwrap();
        assert!(matches!(read_untrusted(&link, 10), Err(FileError::NotRegular)), "son bileşen symlink");
        let fifo = dir.path().join("fifo.png");
        rustix::fs::mknodat(rustix::fs::CWD, &fifo, FileType::Fifo, Mode::from_raw_mode(0o600), 0).unwrap();
        let started = std::time::Instant::now();
        assert!(matches!(read_untrusted(&fifo, 10), Err(FileError::NotRegular)));
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "FIFO beklemez");
        assert!(matches!(read_untrusted(dir.path(), 10), Err(FileError::NotRegular)), "dizin");
    }

    #[test]
    fn within_dir_blocks_escape() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("theme")).unwrap();
        let theme = root.path().join("theme").canonicalize().unwrap();
        std::fs::create_dir_all(theme.join("hicolor/scalable/apps")).unwrap();
        let outside = root.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, theme.join("evil")).unwrap();

        assert_eq!(within_dir(&theme, Path::new("a.png")), Some(theme.join("a.png")));
        assert_eq!(
            within_dir(&theme, Path::new("hicolor/scalable/apps/a.svg")),
            Some(theme.join("hicolor/scalable/apps/a.svg"))
        );
        assert_eq!(within_dir(&theme, Path::new("evil/a.png")), None, "ara dizin symlink'i dışarı çıkıyor");
        assert_eq!(within_dir(&theme, Path::new("../outside/a.png")), None);
        assert_eq!(within_dir(&theme, Path::new("/etc/passwd")), None);
        assert_eq!(within_dir(&theme, Path::new("missing/a.png")), None);
    }
}
