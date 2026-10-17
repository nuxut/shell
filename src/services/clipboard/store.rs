//! İsteğe bağlı disk geçmişi (`[clipboard] persist = true`; varsayılan kapalı).
//!
//! Dosya: `$XDG_STATE_HOME/nuxut/clipboard/history.bin`, izin `0600`, dizin `0700`. Yazım
//! atomik (geçici dosya + fsync + rename). Biçim sürümlü ve kendi kendini sınırlar:
//!
//! ```text
//! "NUXUTCLP" | sürüm u16 LE | 0u16 | öğe sayısı u32 LE
//! her öğe: tür u8 (1 metin, 2 png, 3 jpeg) | zaman u64 LE (unix sn) | uzunluk u32 LE | bayt
//! ```
//!
//! Okumada tüm sınırlar yeniden uygulanır (öğe sayısı, öğe ve toplam bayt, UTF-8/imza); bozuk ya
//! da büyük dosya hata verir (çağıran tek uyarıyla boş geçmişle başlar). Ara tamponlar sıfırlanır.
//! Bilinen sınır: SSD'de silinen dosyanın blokları fiziksel olarak kalabilir.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use zeroize::Zeroizing;

use super::filter::ImageMime;
use super::history::{Content, MAX_IMAGE_BYTES, MAX_ITEMS, MAX_TEXT_BYTES, MAX_TOTAL_BYTES};

const MAGIC: &[u8; 8] = b"NUXUTCLP";
pub const VERSION: u16 = 1;
const HEADER: usize = 8 + 2 + 2 + 4;
const ITEM_HEADER: usize = 1 + 8 + 4;
/// Dosyanın olabilecek en büyük boyu.
pub const MAX_FILE_BYTES: usize = HEADER + MAX_ITEMS * ITEM_HEADER + MAX_TOTAL_BYTES;

const KIND_TEXT: u8 = 1;
const KIND_PNG: u8 = 2;
const KIND_JPEG: u8 = 3;

/// Disk biçimi hatası (içerik taşımaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("geçmiş dosyası tanınmadı")]
    Magic,
    #[error("desteklenmeyen geçmiş dosyası sürümü {0}")]
    Version(u16),
    #[error("geçmiş dosyası sınırları aşıyor")]
    Limits,
    #[error("geçmiş dosyası bozuk")]
    Corrupt,
    #[error("geçmiş dosyası okunamadı/yazılamadı")]
    Io,
}

/// Yüklenen öğe (en yeni başta).
pub type Loaded = Vec<(Content, SystemTime)>;

/// `<state>/nuxut/clipboard/history.bin` (`state`: `Env::state_home`).
pub fn path_in(state_home: &Path) -> PathBuf {
    state_home.join("nuxut/clipboard/history.bin")
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Öğeleri (en yeni başta) dosya baytlarına çevirir.
pub fn encode<'a>(items: impl ExactSizeIterator<Item = (&'a Content, SystemTime)>) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(Vec::new());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let count = items.len().min(MAX_ITEMS);
    out.extend_from_slice(&(count as u32).to_le_bytes());
    for (content, at) in items.take(count) {
        let kind = match content {
            Content::Text(_) => KIND_TEXT,
            Content::Image { mime: ImageMime::Png, .. } => KIND_PNG,
            Content::Image { mime: ImageMime::Jpeg, .. } => KIND_JPEG,
        };
        let bytes = content.blob().as_bytes();
        out.push(kind);
        out.extend_from_slice(&unix(at).to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

/// Sınırlı okuyucu: her alan sınır denetimiyle.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], StoreError> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.bytes.len()).ok_or(StoreError::Corrupt)?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], StoreError> {
        self.take(N)?.try_into().map_err(|_| StoreError::Corrupt)
    }
}

/// Dosya baytlarını çözer; her sınır yeniden uygulanır.
pub fn decode(bytes: &[u8]) -> Result<Loaded, StoreError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(StoreError::Limits);
    }
    let mut r = Reader { bytes, pos: 0 };
    if r.take(8).map_err(|_| StoreError::Magic)? != MAGIC {
        return Err(StoreError::Magic);
    }
    let version = u16::from_le_bytes(r.array()?);
    if version != VERSION {
        return Err(StoreError::Version(version));
    }
    let _reserved = r.array::<2>()?;
    let count = u32::from_le_bytes(r.array()?) as usize;
    if count > MAX_ITEMS {
        return Err(StoreError::Limits);
    }
    let mut out = Vec::with_capacity(count);
    let mut total = 0usize;
    for _ in 0..count {
        let [kind] = r.array::<1>()?;
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::from_le_bytes(r.array()?));
        let len = u32::from_le_bytes(r.array()?) as usize;
        let max = match kind {
            KIND_TEXT => MAX_TEXT_BYTES,
            KIND_PNG | KIND_JPEG => MAX_IMAGE_BYTES,
            _ => return Err(StoreError::Corrupt),
        };
        total = total.saturating_add(len);
        if len > max || total > MAX_TOTAL_BYTES {
            return Err(StoreError::Limits);
        }
        let data = Zeroizing::new(r.take(len)?.to_vec());
        let content = match kind {
            KIND_TEXT => Content::text(data),
            KIND_PNG => Content::image(ImageMime::Png, data),
            _ => Content::image(ImageMime::Jpeg, data),
        }
        .map_err(|_| StoreError::Corrupt)?;
        out.push((content, at));
    }
    if r.pos != bytes.len() {
        return Err(StoreError::Corrupt);
    }
    Ok(out)
}

/// Dosyayı okur. Yoksa boş; bozuk/büyükse hata.
pub fn load(path: &Path) -> Result<Loaded, StoreError> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(StoreError::Io),
        Ok(_) => {}
    }
    let bytes = crate::services::safe_file::read_untrusted(path, MAX_FILE_BYTES as u64).map_err(|e| match e {
        crate::services::safe_file::FileError::Size => StoreError::Limits,
        _ => StoreError::Io,
    })?;
    let bytes = Zeroizing::new(bytes);
    decode(&bytes)
}

/// Atomik yazım: dizin `0700`, dosya `0600`.
pub fn save(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    let dir = path.parent().ok_or(StoreError::Io)?;
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|_| StoreError::Io)?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Var olan dizin: symlink ya da başka türse yazılmaz; izni daraltılır.
            let meta = std::fs::symlink_metadata(dir).map_err(|_| StoreError::Io)?;
            if !meta.is_dir() {
                return Err(StoreError::Io);
            }
            if meta.permissions().mode() & 0o077 != 0 {
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|_| StoreError::Io)?;
            }
        }
        Err(_) => return Err(StoreError::Io),
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("history.bin");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    // Çökmüş önceki yazımın aynı PID'li artığı `create_new`'ü sonsuza dek kırmasın (dizin
    // 0700 ve bizim; `remove_file` symlink'i izlemez).
    let _ = std::fs::remove_file(&tmp);
    let result = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc_nofollow()).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(StoreError::Io);
    }
    Ok(())
}

/// `O_NOFOLLOW` bayrağı (geçici dosya symlink olamaz; `create_new` zaten var olanı reddeder).
fn libc_nofollow() -> i32 {
    rustix::fs::OFlags::NOFOLLOW.bits() as i32
}

/// Dosyayı siler (yoksa sorun değil).
pub fn remove(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(kind = ?e.kind(), "pano geçmişi dosyası silinemedi"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Content {
        Content::text(Zeroizing::new(s.as_bytes().to_vec())).unwrap()
    }

    fn png() -> Content {
        Content::image(ImageMime::Png, Zeroizing::new(b"\x89PNG\r\n\x1a\nxyz".to_vec())).unwrap()
    }

    fn at(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(s)
    }

    fn sample() -> Zeroizing<Vec<u8>> {
        let items = [(text("merhaba"), at(100)), (png(), at(50))];
        encode(items.iter().map(|(c, t)| (c, *t)))
    }

    #[test]
    fn roundtrip() {
        let loaded = decode(&sample()).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].0.as_text(), Some("merhaba"));
        assert_eq!(loaded[0].1, at(100));
        assert!(matches!(loaded[1].0, Content::Image { mime: ImageMime::Png, .. }));
        assert_eq!(loaded[1].0.blob().as_bytes(), b"\x89PNG\r\n\x1a\nxyz");
        let empty = encode(std::iter::empty::<(&Content, SystemTime)>());
        assert!(decode(&empty).unwrap().is_empty());
    }

    #[test]
    fn corrupt_and_version() {
        let good = sample();
        assert_eq!(decode(b"").unwrap_err(), StoreError::Magic);
        assert_eq!(decode(b"NOTCLIP!xxxxxxxx").unwrap_err(), StoreError::Magic);
        let mut v = good.to_vec();
        v[8] = 2;
        assert_eq!(decode(&v).unwrap_err(), StoreError::Version(2));
        // Her kırpma bozuk sayılır (panik yok).
        for n in 0..good.len() {
            assert!(decode(&good[..n]).is_err(), "kırpık {n}");
        }
        let mut v = good.to_vec();
        v.push(0);
        assert_eq!(decode(&v).unwrap_err(), StoreError::Corrupt, "artık bayt");
        // Bilinmeyen tür.
        let mut v = good.to_vec();
        v[HEADER] = 9;
        assert_eq!(decode(&v).unwrap_err(), StoreError::Corrupt);
        // Geçersiz UTF-8 metin.
        let mut v = good.to_vec();
        v[HEADER + ITEM_HEADER] = 0xFF;
        assert_eq!(decode(&v).unwrap_err(), StoreError::Corrupt);
    }

    #[test]
    fn limits_enforced_on_read() {
        // Öğe sayısı başlıkta sınırı aşıyor.
        let mut v = sample().to_vec();
        v[12..16].copy_from_slice(&((MAX_ITEMS + 1) as u32).to_le_bytes());
        assert_eq!(decode(&v).unwrap_err(), StoreError::Limits);
        // Metin uzunluğu sınırı aşıyor (bayt gelmeden reddedilir).
        let mut v = sample().to_vec();
        let len_at = HEADER + 1 + 8;
        v[len_at..len_at + 4].copy_from_slice(&((MAX_TEXT_BYTES + 1) as u32).to_le_bytes());
        assert_eq!(decode(&v).unwrap_err(), StoreError::Limits);
        // Uzunluk dosyadan büyük: bozuk.
        let mut v = sample().to_vec();
        v[len_at..len_at + 4].copy_from_slice(&1000u32.to_le_bytes());
        assert_eq!(decode(&v).unwrap_err(), StoreError::Corrupt);
        assert_eq!(decode(&vec![0u8; MAX_FILE_BYTES + 1]).unwrap_err(), StoreError::Limits);
    }

    #[test]
    fn save_load_permissions_and_remove() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        assert!(load(&path).unwrap().is_empty(), "dosya yoksa boş");
        save(&path, &sample()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(load(&path).unwrap().len(), 2);
        // Üzerine yazım atomik; geçici dosya kalmaz.
        save(&path, &sample()).unwrap();
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1);
        // Gevşek izinli var olan dizin daraltılır.
        std::fs::set_permissions(path.parent().unwrap(), std::fs::Permissions::from_mode(0o755)).unwrap();
        save(&path, &sample()).unwrap();
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        // Bozuk dosya hata verir.
        std::fs::write(&path, b"garbage").unwrap();
        assert_eq!(load(&path).unwrap_err(), StoreError::Magic);
        remove(&path);
        assert!(!path.exists());
        remove(&path);
    }

    #[test]
    fn stale_temp_file_does_not_block_saving() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        save(&path, &sample()).unwrap();
        let tmp = path.parent().unwrap().join(format!(".history.bin.{}.tmp", std::process::id()));
        std::fs::write(&tmp, b"yarim").unwrap();
        save(&path, &sample()).unwrap();
        assert!(!tmp.exists());
        assert_eq!(load(&path).unwrap().len(), 2);
    }

    #[test]
    fn symlinked_file_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        save(&path, &sample()).unwrap();
        let target = dir.path().join("elsewhere.bin");
        std::fs::rename(&path, &target).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert_eq!(load(&path).unwrap_err(), StoreError::Io);
    }
}
