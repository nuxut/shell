//! Kök dosyalarının (sistem yardımcısı, polkit eylemi, giriş ekranı) kaynak kurulumunda yerine
//! konması: `nuxut-shell system-theme install-helper` (ADR-0008 "Sonuçlar", en az ayrıcalık kararı
//! 2026-10-17).
//!
//! `install.sh` normal kurulumda kök dosyalarına dokunmaz (sudo yok): bu dosyaları kullanıcının
//! kendi dizinine, kurulacakları yolun aynısıyla hazırlar (`<önek>/lib/nuxut/system/usr/…`). Kök
//! yetkisi yalnız sistem görünümü ya da giriş ekranı istendiğinde ve bir düğmeyle istenir
//! (Hoş geldin, Ayarlar, `install.sh --system-theme`):
//!
//! 1. Yalnız [`FILES`]'taki yollar okunur (hazırlık dizinine sonradan konan başka bir dosya kök
//!    dosya sistemine girmez); her biri `safe_file::read_untrusted` ile (son bileşen bağ olamaz,
//!    düz dosya, boyut sınırlı). Hazırlık dizini kullanıcının olmalı ve başkasınca yazılamamalı.
//! 2. İçerik, sahibi ve izni açıkça yazılmış (uid/gid 0, 0755/0644) bir ustar arşivine konur;
//!    arşiv rastgele adlı 0700 geçici dizinde.
//! 3. `pkexec /usr/bin/bsdtar` arşivi `/` altına açar (kökün aracı; libarchive pacman'ın
//!    bağımlılığı, Arch'ta hep kurulu). bsdtar varsayılanı mutlak yol, `..` ve ara dizin bağını
//!    reddeder; `-U` var olan dosyayı silip yeniden yaratır (çalışan ikili ETXTBSY vermez).
//!    Kullanıcı Nuxut polkit penceresinde tek bir yönetici onayı verir.
//!
//! Sınır: hazırlanan dosyalar kullanıcının yazabildiği yerden gelir; kaynak kurulumunda kök
//! yardımcının içeriği zaten kullanıcının derlediğidir (sudo ile kurmakla aynı güven). Paketli
//! kurulumda dosyalar paketle `/usr`'dedir, hazırlık dizini yoktur ve bu yol kullanılmaz.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::services::safe_file;

/// Kurulan kök dosyası: `/`'e göre yol ve izin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleFile {
    pub path: &'static str,
    pub mode: u32,
}

const fn file(path: &'static str, mode: u32) -> BundleFile {
    BundleFile { path, mode }
}

/// Kurulan dosyalar (beyaz liste). Giriş ekranı dosyaları `nuxut_system_helper::greeter::REQUIRED`
/// ile aynı yollar (test denetler).
pub const FILES: [BundleFile; 8] = [
    file("usr/lib/nuxut/nuxut-system-helper", 0o755),
    file("usr/share/polkit-1/actions/org.nuxut.system-helper.policy", 0o644),
    file("usr/bin/nuxut-greeter", 0o755),
    file("usr/lib/nuxut/nuxut-greeter-session", 0o755),
    file("usr/lib/nuxut/nuxut-session", 0o755),
    file("usr/share/nuxut/greeter/hyprland.lua", 0o644),
    file("usr/lib/tmpfiles.d/nuxut-greeter.conf", 0o644),
    file("usr/share/wayland-sessions/nuxut.desktop", 0o644),
];

/// Nuxut'un kendi dizinleri (yoksa 0755 yaratılır). Sistemin dizinleri (`usr/bin` …) arşive
/// girmez: izinleri ve zamanları değişmez.
pub const DIRS: [&str; 3] = ["usr/lib/nuxut", "usr/share/nuxut", "usr/share/nuxut/greeter"];

/// Tek dosya üst sınırı (giriş ekranı ikilisi çerçeveyle birlikte onlarca MB).
const MAX_FILE: u64 = 256 * 1024 * 1024;

/// Hazırlık dizini: çalışan ikilinin önekinde `lib/nuxut/system` (`~/.local/bin/nuxut-shell` →
/// `~/.local/lib/nuxut/system`).
pub fn stage_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("lib/nuxut/system"))
}

/// Hazırlık dizininin durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Tüm dosyalar var.
    Ready,
    /// Hazırlık yok (paketli kurulum ya da eski install.sh).
    Missing,
    /// Var ama güvensiz (başkasının ya da başkasınca yazılabilir) ya da eksik.
    Unusable,
}

pub fn stage_state(stage: &Path) -> Stage {
    use std::os::unix::fs::MetadataExt;
    let Ok(m) = stage.symlink_metadata() else { return Stage::Missing };
    let me = rustix::process::geteuid().as_raw();
    if !m.is_dir() || m.uid() != me || m.mode() & 0o022 != 0 {
        return Stage::Unusable;
    }
    if FILES.iter().all(|f| stage.join(f.path).symlink_metadata().is_ok_and(|m| m.is_file())) { Stage::Ready } else { Stage::Unusable }
}

/// Kurulu dosyalar hazırlıktakilerle aynı mı? Hazırlık yoksa ya da kurulu yardımcı yoksa `false`
/// (güncellenecek bir şey yok). Önce boyut, sonra içerik (yalnız okuma).
pub fn update_available(stage: &Path, root: &Path) -> bool {
    if stage_state(stage) != Stage::Ready || !root.join(FILES[0].path).exists() {
        return false;
    }
    FILES.iter().any(|f| !same_file(&stage.join(f.path), &root.join(f.path)))
}

fn same_file(a: &Path, b: &Path) -> bool {
    use std::io::Read;
    let (Ok(fa), Ok(fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else { return false };
    let (Ok(ma), Ok(mb)) = (fa.metadata(), fb.metadata()) else { return false };
    if ma.len() != mb.len() {
        return false;
    }
    let (mut ra, mut rb) = (std::io::BufReader::new(fa), std::io::BufReader::new(fb));
    let (mut ba, mut bb) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let n = match ra.read(&mut ba) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if n == 0 {
            return true;
        }
        if rb.read_exact(&mut bb[..n]).is_err() || ba[..n] != bb[..n] {
            return false;
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("hazırlık dizini yok ya da eksik: {0}")]
    Stage(String),
    #[error("{path}: {err}")]
    File { path: &'static str, err: safe_file::FileError },
    #[error("arşiv yazılamadı: {0}")]
    Io(#[from] std::io::Error),
}

/// [`FILES`]'ı hazırlık dizininden okur ve ustar arşivi olarak `out`'a yazar.
pub fn write_archive(stage: &Path, out: &mut impl Write, mtime: u64) -> Result<(), BundleError> {
    if stage_state(stage) != Stage::Ready {
        return Err(BundleError::Stage(stage.display().to_string()));
    }
    for d in DIRS {
        out.write_all(&header(&format!("{d}/"), 0o755, 0, mtime, b'5'))?;
    }
    for f in FILES {
        let bytes = safe_file::read_untrusted(&stage.join(f.path), MAX_FILE).map_err(|err| BundleError::File { path: f.path, err })?;
        out.write_all(&header(f.path, f.mode, bytes.len() as u64, mtime, b'0'))?;
        out.write_all(&bytes)?;
        let pad = (512 - bytes.len() % 512) % 512;
        out.write_all(&vec![0u8; pad])?;
    }
    out.write_all(&[0u8; 1024])?;
    Ok(())
}

/// ustar başlığı: kök sahibi, verilen izin. Yollar 100 baytı aşmaz (sabit liste; test denetler).
fn header(name: &str, mode: u32, size: u64, mtime: u64, kind: u8) -> [u8; 512] {
    let mut h = [0u8; 512];
    let put = |h: &mut [u8; 512], at: usize, s: &[u8]| h[at..at + s.len()].copy_from_slice(s);
    let octal = |h: &mut [u8; 512], at: usize, width: usize, v: u64| {
        let s = format!("{v:0w$o}", w = width - 1);
        put(h, at, s.as_bytes());
    };
    put(&mut h, 0, name.as_bytes());
    octal(&mut h, 100, 8, u64::from(mode));
    octal(&mut h, 108, 8, 0);
    octal(&mut h, 116, 8, 0);
    octal(&mut h, 124, 12, size);
    octal(&mut h, 136, 12, mtime);
    h[156] = kind;
    put(&mut h, 257, b"ustar\x0000");
    put(&mut h, 265, b"root");
    put(&mut h, 297, b"root");
    put(&mut h, 148, b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    put(&mut h, 148, format!("{sum:06o}\0 ").as_bytes());
    h
}

/// bsdtar'ın açma argümanları (pkexec'ten sonra).
pub fn extract_args(archive: &Path) -> Vec<std::ffi::OsString> {
    let mut v: Vec<std::ffi::OsString> = ["/usr/bin/bsdtar", "-x", "-p", "-U", "--no-acls", "--no-xattrs", "--no-fflags", "-C", "/", "-f"].iter().map(Into::into).collect();
    v.push(archive.into());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn stage_with_files() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        for f in FILES {
            let p = t.path().join(f.path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, format!("içerik {}", f.path)).unwrap();
        }
        t
    }

    #[test]
    fn greeter_files_are_in_the_bundle() {
        for r in nuxut_system_helper::greeter::REQUIRED {
            assert!(FILES.iter().any(|f| f.path == r), "{r}");
        }
        assert_eq!(FILES[0].path, nuxut_system_helper::INSTALL_PATH.trim_start_matches('/'));
        for f in FILES {
            assert!(f.path.len() < 100 && !f.path.starts_with('/') && !f.path.contains(".."));
        }
    }

    #[test]
    fn stage_must_be_complete_and_private() {
        let t = stage_with_files();
        assert_eq!(stage_state(t.path()), Stage::Ready);
        assert_eq!(stage_state(&t.path().join("yok")), Stage::Missing);
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(stage_state(t.path()), Stage::Unusable, "herkesçe yazılabilir");
        std::fs::set_permissions(t.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_file(t.path().join(FILES[3].path)).unwrap();
        assert_eq!(stage_state(t.path()), Stage::Unusable, "eksik dosya");
    }

    /// Hazırlıktaki bağ (ör. `/etc/shadow`'a) arşive girmez; liste dışı dosya hiç okunmaz.
    #[test]
    fn archive_rejects_links_and_ignores_extra_files() {
        let t = stage_with_files();
        std::fs::write(t.path().join("usr/bin/fazladan"), "x").unwrap();
        let mut out = Vec::new();
        write_archive(t.path(), &mut out, 0).unwrap();
        assert!(!out.windows(8).any(|w| w == b"fazladan"));
        let p = t.path().join(FILES[1].path);
        std::fs::remove_file(&p).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", &p).unwrap();
        assert!(matches!(write_archive(t.path(), &mut Vec::new(), 0), Err(BundleError::Stage(_))));
    }

    /// Arşiv geçerli ustar: başlık sağlaması, kök sahibi, izinler; sistem bsdtar'ı (varsa) listeler.
    #[test]
    fn archive_is_valid_ustar_owned_by_root() {
        let t = stage_with_files();
        let mut out = Vec::new();
        write_archive(t.path(), &mut out, 1_700_000_000).unwrap();
        assert_eq!(out.len() % 512, 0);
        let h = &out[..512];
        let stored = u32::from_str_radix(std::str::from_utf8(&h[148..154]).unwrap(), 8).unwrap();
        let mut copy = h.to_vec();
        copy[148..156].copy_from_slice(b"        ");
        assert_eq!(stored, copy.iter().map(|&b| u32::from(b)).sum::<u32>());
        let bsdtar = Path::new("/usr/bin/bsdtar");
        if !bsdtar.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let ar = dir.path().join("a.tar");
        std::fs::write(&ar, &out).unwrap();
        let o = std::process::Command::new(bsdtar).args(["-t", "-v", "--numeric-owner", "-f"]).arg(&ar).output().unwrap();
        assert!(o.status.success());
        let list = String::from_utf8_lossy(&o.stdout);
        for f in FILES {
            let line = list.lines().find(|l| l.ends_with(f.path)).unwrap_or_else(|| panic!("{} yok:\n{list}", f.path));
            let perm = if f.mode == 0o755 { "-rwxr-xr-x" } else { "-rw-r--r--" };
            assert!(line.starts_with(perm) && line.contains(" 0 ") && line.contains(" 0 "), "{line}");
        }
        // Kendi kökümüze açılış (yetkisiz): içerik aynı.
        let root = tempfile::tempdir().unwrap();
        let o = std::process::Command::new(bsdtar).args(["-x", "-U", "--no-acls", "--no-xattrs", "--no-fflags", "-C"]).arg(root.path()).arg("-f").arg(&ar).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        for f in FILES {
            assert_eq!(std::fs::read(root.path().join(f.path)).unwrap(), std::fs::read(t.path().join(f.path)).unwrap());
        }
        assert!(!update_available(t.path(), root.path()), "aynı");
        std::fs::write(root.path().join(FILES[2].path), "eski").unwrap();
        assert!(update_available(t.path(), root.path()), "farklı");
        assert!(!update_available(t.path(), &root.path().join("kurulu-değil")));
    }
}
