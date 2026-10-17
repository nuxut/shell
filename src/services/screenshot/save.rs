//! Çıktı: PNG kodlama, kayıt dizini ve dosya adı, güvenli yazım, pano, dosyayı açma.
//!
//! Kayıt kuralları (`safe_file` ruhu): dizin `create_dir_all` ile kurulur ve bir kez açılır;
//! dosya o dizin tanıtıcısına göre `O_CREAT | O_EXCL | O_NOFOLLOW` ile yaratılır — var olan
//! dosyanın (ya da oraya konmuş bir sembolik bağlantının) üzerine asla yazılmaz, bağlantı
//! izlenmez. Ad çakışırsa `-2`, `-3`… denenir. Ara dizinlerdeki bağlantılar izlenir
//! (`~/Pictures` → başka disk gibi meşru kurulumlar).

use std::io::Write;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rustix::fs::{Mode, OFlags};

use super::geometry::RgbaImage;

/// Çakışmada denenecek en çok ad.
const MAX_NAME_TRIES: u32 = 100;
/// Dosya adı sınırı (bayt; `.png` ve sonek hariç). ext4 sınırı 255.
const MAX_STEM_BYTES: usize = 200;
/// `wl-copy` ön sürecinin (stdin'i okuyup arka plana çatallanana dek) en uzun süresi.
const COPY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Kullanıcı dizin adı yerelleştirilemezse.
pub const FALLBACK_SUBDIR: &str = "Screenshots";
pub const FALLBACK_STEM: &str = "Screenshot";

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("PNG kodlanamadı: {0}")]
    Encode(String),
    #[error("dizin hazırlanamadı: {0}")]
    Dir(std::io::Error),
    #[error("dosya yazılamadı: {0}")]
    Write(std::io::Error),
    #[error("boş bir ad bulunamadı")]
    NoFreeName,
    #[error("wl-copy bulunamadı (wl-clipboard paketi kurulu mu?)")]
    NoWlCopy,
    #[error("panoya kopyalanamadı: {0}")]
    Copy(String),
}

/// PNG baytları. Opak görüntü RGB (alfa kanalı atlanır), değilse RGBA. Hızlı sıkıştırma
/// (fdeflate): 4K ekran görüntüsü onlarca ms'de, boyut varsayılana yakın.
pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, SaveError> {
    let mut out = Vec::new();
    let opaque = image.is_opaque();
    {
        let mut enc = png::Encoder::new(&mut out, image.width, image.height);
        enc.set_color(if opaque { png::ColorType::Rgb } else { png::ColorType::Rgba });
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().map_err(|e| SaveError::Encode(e.to_string()))?;
        let result = if opaque {
            let rgb: Vec<u8> = image.pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            writer.write_image_data(&rgb)
        } else {
            writer.write_image_data(&image.pixels)
        };
        result.map_err(|e| SaveError::Encode(e.to_string()))?;
        writer.finish().map_err(|e| SaveError::Encode(e.to_string()))?;
    }
    Ok(out)
}

/// `user-dirs.dirs` içeriğinden `key` (`XDG_PICTURES_DIR`) dizini. Biçim (xdg-user-dirs):
/// `KEY="$HOME/yol"` ya da `KEY="/mutlak/yol"`. `$HOME`'un kendisi "kapalı" demektir (`None`).
pub fn parse_user_dirs(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let value = text.lines().rev().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix(key)?.trim_start().strip_prefix('=')?.trim();
        Some(rest.strip_prefix('"')?.strip_suffix('"')?.to_owned())
    })?;
    let path = if let Some(rel) = value.strip_prefix("$HOME") {
        let rel = rel.trim_start_matches('/');
        if rel.is_empty() {
            return None;
        }
        home.join(rel)
    } else if value.starts_with('/') {
        PathBuf::from(value)
    } else {
        return None;
    };
    (path != home && !path.components().any(|c| matches!(c, std::path::Component::ParentDir))).then_some(path)
}

/// Resim dizini: `$XDG_CONFIG_HOME/user-dirs.dirs`'teki `XDG_PICTURES_DIR`, yoksa `~/Pictures`.
pub fn pictures_dir(home: &Path, config_home: &Path) -> PathBuf {
    std::fs::read_to_string(config_home.join("user-dirs.dirs"))
        .ok()
        .and_then(|t| parse_user_dirs(&t, "XDG_PICTURES_DIR", home))
        .unwrap_or_else(|| home.join("Pictures"))
}

/// Ortamdan ev ve config dizini (`$HOME`, `$XDG_CONFIG_HOME` yoksa `~/.config`).
pub fn home_and_config() -> Option<(PathBuf, PathBuf)> {
    let home = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute())?;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    Some((home, config))
}

/// Kayıt dizini. `configured`: `[screenshot] dir` (boş = varsayılan; `~/` ev dizinine göre;
/// doğrulama göreli yolu zaten reddetti). Varsayılan: resim dizini / yerelleştirilmiş alt dizin.
pub fn save_dir(configured: &str, home: &Path, pictures: &Path, subdir: &str) -> PathBuf {
    let c = configured.trim();
    if let Some(rest) = c.strip_prefix("~/") {
        return home.join(rest);
    }
    if c.starts_with('/') {
        return PathBuf::from(c);
    }
    pictures.join(sanitize_name(subdir, MAX_STEM_BYTES).unwrap_or_else(|| FALLBACK_SUBDIR.to_owned()))
}

/// Ad bileşeni: `/` ve kontrol karakterleri `-`/atılır, baş/son boşluk ve noktalar kırpılır,
/// bayt sınırında karakter sınırından kesilir. Boş kalırsa `None`.
pub fn sanitize_name(name: &str, max_bytes: usize) -> Option<String> {
    let mut out = String::new();
    for c in name.chars() {
        let c = match c {
            '/' | '\\' => '-',
            c if c.is_control() => continue,
            c => c,
        };
        if out.len() + c.len_utf8() > max_bytes {
            break;
        }
        out.push(c);
    }
    let trimmed = out.trim().trim_matches('.').trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Zaman damgası (`YYYY-MM-DD HH-MM-SS`; `:` dosya yöneticilerinde/FAT'te sorun çıkarır).
pub fn timestamp(at: chrono::DateTime<chrono::Local>) -> String {
    at.format("%Y-%m-%d %H-%M-%S").to_string()
}

/// Denenecek `n`. ad (`n` 1'den): `stem.png`, `stem-2.png`…
pub fn candidate_name(stem: &str, n: u32) -> String {
    if n <= 1 { format!("{stem}.png") } else { format!("{stem}-{n}.png") }
}

/// PNG'yi `dir/stem(.png|-N.png)` olarak yazar (bkz. modül belgesi). Dönen: yazılan yol.
pub fn save_png(dir: &Path, stem: &str, bytes: &[u8]) -> Result<PathBuf, SaveError> {
    let stem = sanitize_name(stem, MAX_STEM_BYTES).unwrap_or_else(|| FALLBACK_STEM.to_owned());
    std::fs::DirBuilder::new().recursive(true).create(dir).map_err(SaveError::Dir)?;
    let dir_fd = rustix::fs::open(dir, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())
        .map_err(|e| SaveError::Dir(e.into()))?;
    for n in 1..=MAX_NAME_TRIES {
        let name = candidate_name(&stem, n);
        let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let fd = match rustix::fs::openat(dir_fd.as_fd(), name.as_str(), flags, Mode::from_raw_mode(0o644)) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::EXIST) => continue,
            Err(e) => return Err(SaveError::Write(e.into())),
        };
        let mut file = std::fs::File::from(fd);
        if let Err(e) = file.write_all(bytes).and_then(|()| file.flush()) {
            // Yarım dosya kalmasın (yalnızca bizim az önce yarattığımız ad).
            let _ = rustix::fs::unlinkat(dir_fd.as_fd(), name.as_str(), rustix::fs::AtFlags::empty());
            return Err(SaveError::Write(e));
        }
        return Ok(dir.join(name));
    }
    Err(SaveError::NoFreeName)
}

/// `wl-copy` argümanları (program adı hariç).
pub fn wl_copy_args() -> [&'static str; 2] {
    ["--type", "image/png"]
}

/// PNG'yi panoya verir. `wl-copy` arka plana çatallanıp seçimi sunar; ön süreç stdin bitince
/// çıkar ve beklenir.
pub fn copy_png(bytes: &[u8]) -> Result<(), SaveError> {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new("wl-copy")
        .args(wl_copy_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { SaveError::NoWlCopy } else { SaveError::Copy(e.to_string()) })?;
    let (written, timed_out) = super::capture::with_deadline(&mut child, COPY_TIMEOUT, |child| {
        let written = child.stdin.take().map(|mut s| s.write_all(bytes));
        super::capture::wait_exit_no_reap(child);
        written
    });
    let status = child.wait().map_err(|e| SaveError::Copy(e.to_string()))?;
    if timed_out {
        return Err(SaveError::Copy(format!("{} sn içinde bitmedi", COPY_TIMEOUT.as_secs())));
    }
    if let Some(Err(e)) = written {
        return Err(SaveError::Copy(e.to_string()));
    }
    if !status.success() {
        return Err(SaveError::Copy(status.to_string()));
    }
    Ok(())
}

/// Dosyayı varsayılan uygulamayla açar (`xdg-open`, oturumdan ayrık; `setsid -f`).
pub fn open_path(path: &Path) {
    spawn_detached(&["xdg-open".to_owned(), path.to_string_lossy().into_owned()]);
}

/// Dosyayı dosya yöneticisinde seçili gösterir (`org.freedesktop.FileManager1.ShowItems`);
/// yoksa içinde bulunduğu dizini açar. Bloklar: arka plan thread'inde çağrılır.
pub fn reveal_path(path: &Path) {
    let shown = url::Url::from_file_path(path).ok().is_some_and(|uri| {
        zbus::blocking::Connection::session()
            .and_then(|c| {
                c.call_method(
                    Some("org.freedesktop.FileManager1"),
                    "/org/freedesktop/FileManager1",
                    Some("org.freedesktop.FileManager1"),
                    "ShowItems",
                    &(vec![uri.as_str()], ""),
                )
            })
            .inspect_err(|e| tracing::debug!("FileManager1.ShowItems yok: {e}"))
            .is_ok()
    });
    if !shown && let Some(dir) = path.parent() {
        open_path(dir);
    }
}

fn spawn_detached(argv: &[String]) {
    use std::os::unix::process::CommandExt;
    let tools = crate::services::launcher::launch::Tools::from_env();
    let argv = crate::services::launcher::launch::wrap(argv, &tools, None);
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    for (key, _) in std::env::vars_os().filter(|(k, _)| k.to_string_lossy().starts_with("NUXUT_")) {
        cmd.env_remove(key);
    }
    match cmd.spawn() {
        Ok(mut child) => {
            let _ = std::thread::Builder::new().name("nuxut-shot-open".into()).spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!("ekran görüntüsü açılamadı: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_parsing() {
        let home = Path::new("/home/u");
        let text = "# yorum\nXDG_DESKTOP_DIR=\"$HOME/Masaüstü\"\nXDG_PICTURES_DIR=\"$HOME/Resimler\"\n";
        assert_eq!(parse_user_dirs(text, "XDG_PICTURES_DIR", home), Some(PathBuf::from("/home/u/Resimler")));
        assert_eq!(parse_user_dirs("XDG_PICTURES_DIR=\"/data/pics\"", "XDG_PICTURES_DIR", home), Some(PathBuf::from("/data/pics")));
        assert_eq!(parse_user_dirs("XDG_PICTURES_DIR=\"$HOME/\"", "XDG_PICTURES_DIR", home), None, "kapalı");
        assert_eq!(parse_user_dirs("XDG_PICTURES_DIR=\"$HOME\"", "XDG_PICTURES_DIR", home), None);
        assert_eq!(parse_user_dirs("XDG_PICTURES_DIR=relative", "XDG_PICTURES_DIR", home), None);
        assert_eq!(parse_user_dirs("XDG_PICTURES_DIR=\"$HOME/../etc\"", "XDG_PICTURES_DIR", home), None);
        assert_eq!(parse_user_dirs("XDG_PICTURES_DIRX=\"/x\"", "XDG_PICTURES_DIR", home), None);
        // Son tanım geçerli.
        let twice = "XDG_PICTURES_DIR=\"/a\"\nXDG_PICTURES_DIR=\"/b\"";
        assert_eq!(parse_user_dirs(twice, "XDG_PICTURES_DIR", home), Some(PathBuf::from("/b")));
    }

    #[test]
    fn pictures_dir_falls_back_to_home_pictures() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let config = tmp.path().join("config");
        assert_eq!(pictures_dir(&home, &config), home.join("Pictures"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("user-dirs.dirs"), "XDG_PICTURES_DIR=\"$HOME/Resimler\"\n").unwrap();
        assert_eq!(pictures_dir(&home, &config), home.join("Resimler"));
    }

    #[test]
    fn save_dir_resolution() {
        let home = Path::new("/home/u");
        let pics = Path::new("/home/u/Resimler");
        assert_eq!(save_dir("", home, pics, "Ekran Görüntüleri"), PathBuf::from("/home/u/Resimler/Ekran Görüntüleri"));
        assert_eq!(save_dir("~/shots", home, pics, "x"), PathBuf::from("/home/u/shots"));
        assert_eq!(save_dir("/tmp/s", home, pics, "x"), PathBuf::from("/tmp/s"));
        assert_eq!(save_dir("", home, pics, "../a/b"), PathBuf::from("/home/u/Resimler/-a-b"), "alt dizin tek bileşen");
        assert_eq!(save_dir("", home, pics, " \u{7} "), PathBuf::from("/home/u/Resimler/Screenshots"));
    }

    #[test]
    fn names_and_timestamps() {
        use chrono::TimeZone;
        let at = chrono::Local.with_ymd_and_hms(2026, 10, 12, 9, 5, 7).unwrap();
        assert_eq!(timestamp(at), "2026-10-12 09-05-07");
        assert_eq!(candidate_name("Ekran görüntüsü 2026-10-12 09-05-07", 1), "Ekran görüntüsü 2026-10-12 09-05-07.png");
        assert_eq!(candidate_name("a", 2), "a-2.png");
        assert_eq!(sanitize_name("a/b\nc", 50).as_deref(), Some("a-bc"));
        assert_eq!(sanitize_name("..", 50), None);
        assert_eq!(sanitize_name("ğğğ", 5).as_deref(), Some("ğğ"), "karakter sınırında kesilir");
    }

    #[test]
    fn save_never_overwrites_or_follows_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/b");
        let p1 = save_png(&dir, "shot", b"one").unwrap();
        assert_eq!(p1, dir.join("shot.png"));
        let p2 = save_png(&dir, "shot", b"two").unwrap();
        assert_eq!(p2, dir.join("shot-2.png"));
        assert_eq!(std::fs::read(&p1).unwrap(), b"one", "üzerine yazılmadı");
        // Sonraki ada konmuş bir bağlantı izlenmez, atlanır.
        let target = tmp.path().join("target");
        std::fs::write(&target, b"secret").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("shot-3.png")).unwrap();
        let p4 = save_png(&dir, "shot", b"four").unwrap();
        assert_eq!(p4, dir.join("shot-4.png"));
        assert_eq!(std::fs::read(&target).unwrap(), b"secret");
        // Kırık bağlantı da izlenmez (O_CREAT onu yaratmaz).
        std::os::unix::fs::symlink(tmp.path().join("nowhere"), dir.join("new.png")).unwrap();
        assert_eq!(save_png(&dir, "new", b"x").unwrap(), dir.join("new-2.png"));
        assert!(!tmp.path().join("nowhere").exists());
        // Boş ad yedeğe düşer.
        assert_eq!(save_png(&dir, "/", b"x").unwrap(), dir.join("-.png"));
    }

    #[test]
    fn png_roundtrip_rgb_and_rgba() {
        let opaque = RgbaImage::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
        let bytes = encode_png(&opaque).unwrap();
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut buf = vec![0; dec.output_buffer_size().unwrap()];
        let info = dec.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height, info.color_type), (2, 1, png::ColorType::Rgb));
        assert_eq!(&buf[..6], &[1, 2, 3, 4, 5, 6]);
        let alpha = RgbaImage::new(1, 1, vec![9, 8, 7, 0]).unwrap();
        let bytes = encode_png(&alpha).unwrap();
        let dec = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        assert_eq!(dec.info().color_type, png::ColorType::Rgba);
    }

    #[test]
    fn wl_copy_command_line() {
        assert_eq!(wl_copy_args(), ["--type", "image/png"]);
    }
}
