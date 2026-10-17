//! Tepsi ikonlarını çizime hazır hale getirme. `IconName` ve `IconThemePath` **güvenilmeyen**
//! uygulamadan gelir; bu modül onları yalnızca dar, önceden belirli yollardan okur:
//!
//! - **Ad doğrulama** ([`IconSource::from_item`]): tema adı yalnızca `[A-Za-z0-9._+-]`,
//!   1..=128 karakter, `.` ile başlamaz, `..` içermez. Mutlak yol olarak verilen ad yalnızca
//!   `.png`/`.svg` uzantısıyla ve güvenli açıcıdan geçerek kabul edilir. Diğer her şey
//!   reddedilir (uygulamanın pixmap'i kullanılır).
//! - **Uygulama dizini** (`IconThemePath`): mutlak olmalı, kanonikleştirilir; sahte dosya
//!   sistemi reddedilir. Dizin **taranmaz**: yalnızca `<dir>/<ad>.png|svg` ve birkaç sabit
//!   `hicolor` alt yolu denenir; her aday kanonik olarak dizinin içinde kalmalıdır.
//! - **Okuma**: uygulama dosyası/dizini [`crate::services::safe_file::read_untrusted`]
//!   (O_NOFOLLOW, FIFO/aygıt/sahte dosya sistemi reddi, boyut sınırı); sistem teması adı
//!   `IconTheme::load_as` (dosyayı bir kez açar, güveni fd'den denetler).
//! - **Güven**: uygulamanın dosyası, dizini ve baytları `Untrusted`: shell'in paylaşılan
//!   Sandbox'ı kuruluysa (ADR-0007) ayrı, kısıtlı işçi süreçte çözülür (SVG dahil); işçi yoksa
//!   SVG reddedilir, raster süreç içinde (K2-b); Sandbox hiç verilmezse bugünkü davranış (SVG
//!   reddi). Yalnızca sistem temasında aranan ad `Trust::System` ister ve framework onu
//!   kullanıcının yazamadığı bir sistem dizininde bulduysa SVG süreç içinde çözülür. D-Bus
//!   pixmap'leri ham pikseldir, ayrıştırıcıya girmez. **Çözme**: `nuxut-image` Loader'ı,
//!   `Limits::ICON`, kaynak anahtarı öğe kimliği (`image_sandbox::source_key`). Hata → `None`
//!   → öğenin pixmap'i, o da yoksa widget yer tutucusu.
//! - **İş modeli**: tek arka plan thread'i, sınırlı kuyruk ([`QUEUE_CAPACITY`]); kuyruk doluysa
//!   iş reddedilir (pixmap yedeği). Öğe başına aynı anda en fazla bir iş (servis döngüsü
//!   sağlar); iptal bayrağı okumadan ve çözmeden önce bakılır. Servis döngüsü ve D-Bus
//!   tarafında hiçbir dosya IO'su yoktur.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

use nuxut_image::{
    DecodeOptions, Decoded, Fit, IconTheme, IsolatedDecoder, Limits, LoadRequest, Loader, Source, TargetSize, Trust,
};

use super::item::{Pixmap, TrayItem};
use super::menu::MenuNode;
use crate::services::safe_file::{self, FileError};

/// Kabul edilen hedef boyut aralığı (fiziksel piksel).
pub const MIN_ICON_PX: u32 = 8;
pub const MAX_ICON_PX: u32 = 256;
/// Tema ikon adının en fazla uzunluğu.
pub const MAX_ICON_NAME: usize = 128;
/// Okunan ikon dosyasının en büyük boyutu.
pub const MAX_ICON_FILE_BYTES: u64 = 2 << 20;
/// Bekleyen ikon işi sınırı (tüm öğeler).
pub const QUEUE_CAPACITY: usize = 32;
/// `IconThemePath` içinde denenen `hicolor` boyut dizinleri.
const HICOLOR_SIZES: [u32; 8] = [16, 22, 24, 32, 48, 64, 128, 256];

/// Tema adı güvenli mi?
pub fn valid_icon_name(name: &str) -> bool {
    (1..=MAX_ICON_NAME).contains(&name.len())
        && !name.starts_with('.')
        && !name.contains("..")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

fn has_image_extension(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("svg"))
}

/// Çözülecek ikonun doğrulanmış kaynağı.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum IconSource {
    /// Sistem ikon temasında ad.
    Theme { name: String },
    /// Uygulamanın kendi dizininde ad (önce orada, bulunamazsa sistem temasında).
    AppDir { dir: PathBuf, name: String },
    /// Mutlak dosya yolu (`.png`/`.svg`).
    File { path: PathBuf },
    /// dbusmenu `icon-data`: uygulamanın gönderdiği PNG baytları (imzası ve boyutu menü
    /// ayrıştırmasında sınırlandı); yalnızca çözülür.
    Png(Arc<[u8]>),
}

impl IconSource {
    /// Öğenin şu anki ikonu için kaynak; ad yoksa ya da geçersizse `None` (pixmap kullanılır).
    pub fn from_item(item: &TrayItem) -> Option<Self> {
        Self::from_name(&item.current_icon().name, &item.icon_theme_path)
    }

    /// Menü öğesinin ikonu: `icon-data` önceliklidir, yoksa `icon-name` (öğenin
    /// `IconThemePath`'iyle).
    pub fn for_menu_node(node: &MenuNode, theme_path: &str) -> Option<Self> {
        if let Some(png) = &node.icon_png {
            return Some(Self::Png(Arc::clone(png)));
        }
        Self::from_name(&node.icon_name, theme_path)
    }

    fn from_name(name: &str, theme_path: &str) -> Option<Self> {
        if name.starts_with('/') {
            let path = PathBuf::from(name);
            return (safe_file::check_path(&path).is_ok() && has_image_extension(&path)).then_some(Self::File { path });
        }
        if !valid_icon_name(name) {
            if !name.is_empty() {
                tracing::debug!("tepsi: geçersiz ikon adı reddedildi");
            }
            return None;
        }
        let dir = Path::new(theme_path);
        if !theme_path.is_empty() && safe_file::check_path(dir).is_ok() {
            return Some(Self::AppDir { dir: dir.to_path_buf(), name: name.to_owned() });
        }
        Some(Self::Theme { name: name.to_owned() })
    }
}

/// Bir öğenin o anki ikonunu ne üretir: aynı anahtar yeniden çözülmez.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct IconKey {
    pub source: IconSource,
    pub px: u32,
}

impl IconKey {
    pub fn for_item(item: &TrayItem, px: u32) -> Option<Self> {
        IconSource::from_item(item).map(|source| Self { source, px })
    }
}

/// Öğenin pixmap'lerinden hedef boyuta en uygunu (ad yoksa ya da çözülemezse).
pub(crate) fn pixmap_fallback(item: &TrayItem, px: u32) -> Option<Pixmap> {
    item.current_icon().pixmaps.best(px).cloned()
}

/// Çözme sonucunu modele çevirir (premultiplied → düz RGBA; ilk kare).
pub(crate) fn decoded_to_pixmap(decoded: Decoded) -> Option<Pixmap> {
    let image = decoded.into_first_frame();
    let (width, height) = image.size();
    (width > 0 && height > 0).then(|| Pixmap { width, height, rgba: image.to_straight().into() })
}

pub(crate) fn clamp_px(px: u32) -> u32 {
    px.clamp(MIN_ICON_PX, MAX_ICON_PX)
}

/// Uygulama dizininde denenecek sabit göreli yollar (tarama yok).
fn app_dir_candidates(name: &str, px: u32) -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from(format!("{name}.png")), PathBuf::from(format!("{name}.svg"))];
    out.push(PathBuf::from(format!("hicolor/scalable/apps/{name}.svg")));
    let size = HICOLOR_SIZES.iter().copied().find(|s| *s >= px).unwrap_or(256);
    out.push(PathBuf::from(format!("hicolor/{size}x{size}/apps/{name}.png")));
    out
}

/// Kaynağın Loader isteği. Bloklayan IO (uygulama dizini/dosya okuma): yalnızca ikon thread'inde.
///
/// Güven: uygulamanın verdiği dosya, dizin ve baytlar `Untrusted` (`Limits::ICON` varsayılanı:
/// Sandbox işçisinde çözülür; işçi/Sandbox yoksa SVG çözülmez). Yalnızca sistem temasında doğrulanmış adla aranan ikon `Trust::System` ister;
/// framework onu dosyanın kendisinden (fd) denetler, kullanıcının yazabildiği tema dizininde
/// bulunan dosya yine `Untrusted`'a düşer.
fn request(source: &IconSource, px: u32, system: &Arc<IconTheme>) -> Result<LoadRequest, FileError> {
    let bytes = |bytes: Vec<u8>| LoadRequest { source: Source::Bytes(bytes.into()), options: icon_options(px) };
    match source {
        IconSource::File { path } => safe_file::read_untrusted(path, MAX_ICON_FILE_BYTES).map(bytes),
        IconSource::Png(png) => Ok(LoadRequest { source: Source::Bytes(Arc::clone(png)), options: icon_options(px) }),
        IconSource::AppDir { dir, name } => {
            if let Ok(dir) = dir.canonicalize()
                && safe_file::check_path(&dir).is_ok()
            {
                for rel in app_dir_candidates(name, px) {
                    let Some(path) = safe_file::within_dir(&dir, &rel) else { continue };
                    match safe_file::read_untrusted(&path, MAX_ICON_FILE_BYTES) {
                        Ok(b) => return Ok(bytes(b)),
                        Err(FileError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e),
                    }
                }
            }
            request(&IconSource::Theme { name: name.clone() }, px, system)
        }
        IconSource::Theme { name } => Ok(LoadRequest::icon(Arc::clone(system), name.clone(), px, 1).with_trust(Trust::System)),
    }
}

fn icon_options(px: u32) -> DecodeOptions {
    DecodeOptions {
        limits: Limits { max_file_bytes: MAX_ICON_FILE_BYTES, ..Limits::ICON },
        target: Some(TargetSize::new(px, px, Fit::Contain)),
        animate: false,
    }
}

type Done = Box<dyn FnOnce(Option<Pixmap>) + Send>;

struct Job {
    /// Yalıtılmış çözümün kaynak anahtarı (öğe kimliğinin özeti; `image_sandbox::source_key`).
    source: u64,
    key: IconKey,
    cancelled: Arc<AtomicBool>,
    done: Done,
}

/// İş iptal bayrağı.
#[derive(Debug, Clone)]
pub(crate) struct JobHandle(Arc<AtomicBool>);

impl JobHandle {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Tek thread'li ikon çözücü. Düşürülünce kuyruk kapanır, thread mevcut işi bitirip katılır.
pub(crate) struct IconWorker {
    tx: Option<SyncSender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for IconWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IconWorker")
    }
}

impl IconWorker {
    /// `sandbox`: güvenilmeyen ikonların yalıtılmış çözücüsü (ADR-0007); `None` bugünkü davranış.
    pub fn new(sandbox: Option<Arc<dyn IsolatedDecoder>>) -> Self {
        let (tx, rx) = mpsc::sync_channel(QUEUE_CAPACITY);
        let thread = std::thread::Builder::new().name("nuxut-tray-icons".into()).spawn(move || worker(&rx, sandbox));
        match thread {
            Ok(thread) => Self { tx: Some(tx), thread: Some(thread) },
            Err(e) => {
                tracing::error!("tepsi ikon thread'i başlatılamadı: {e}");
                Self { tx: None, thread: None }
            }
        }
    }

    /// İşi kuyruğa ekler. Kuyruk doluysa ya da thread yoksa `None` (çağıran pixmap'e düşer).
    /// `done` her durumda en fazla bir kez, ikon thread'inden (ya da Loader'ın thread'inden)
    /// çağrılır; iptal edilen işte `None` ile.
    /// `source`: kaynak anahtarı (öğe kimliğinin özeti, sıfır değil).
    pub fn submit(&self, source: u64, key: IconKey, done: impl FnOnce(Option<Pixmap>) + Send + 'static) -> Option<JobHandle> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let job = Job { source, key, cancelled: Arc::clone(&cancelled), done: Box::new(done) };
        match self.tx.as_ref()?.try_send(job) {
            Ok(()) => Some(JobHandle(cancelled)),
            Err(TrySendError::Full(_)) => {
                tracing::warn!(capacity = QUEUE_CAPACITY, "tepsi: ikon kuyruğu dolu, iş atıldı");
                None
            }
            Err(TrySendError::Disconnected(_)) => None,
        }
    }
}

impl Drop for IconWorker {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("tepsi ikon thread'i panikle sonlandı");
        }
    }
}

fn worker(rx: &Receiver<Job>, sandbox: Option<Arc<dyn IsolatedDecoder>>) {
    let system = Arc::new(IconTheme::system());
    let mut loader = Loader::new(1);
    if let Some(sandbox) = sandbox {
        loader = loader.with_sandbox(sandbox);
    }
    while let Ok(job) = rx.recv() {
        if job.cancelled.load(Ordering::Relaxed) {
            (job.done)(None);
            continue;
        }
        let request = match request(&job.key.source, job.key.px, &system) {
            Ok(request) => request,
            Err(e) => {
                tracing::debug!(error = %e, "tepsi: ikon dosyası okunamadı/reddedildi");
                (job.done)(None);
                continue;
            }
        };
        if job.cancelled.load(Ordering::Relaxed) {
            (job.done)(None);
            continue;
        }
        let done = job.done;
        // Hata (yalıtım dahil: zaman aşımı, öldü, hız sınırı, bozuk) → `None`: servis öğenin
        // pixmap'ine, o da yoksa widget yer tutucusuna düşer.
        loader.load_keyed(job.source, request, move |result| {
            done(result.ok().and_then(decoded_to_pixmap));
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::time::Duration;

    use zbus::zvariant::{OwnedValue, Str};

    use super::super::item::ItemId;
    use super::*;

    /// Yalnızca canlılık sınırı (asılı kalan iş testi bitirsin diye): doğruluk hiçbir testte saate
    /// bağlı değil, bu yüzden cömert — yük altında (paralel derleme, bellek bekçisi) erken
    /// dolmasın.
    const LIVENESS: Duration = Duration::from_secs(120);

    const RED_SVG: &str =
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="red"/></svg>"#;

    /// 16×16 kırmızı RGBA PNG (sıkıştırmasız deflate bloğu; fikstür için kodlayıcı gerekmez).
    fn red_png() -> Vec<u8> {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = !0u32;
            for &b in data {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
                }
            }
            !crc
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let start = out.len();
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            let crc = crc32(&out[start..]);
            out.extend_from_slice(&crc.to_be_bytes());
        }
        const SIDE: u32 = 16;
        // Satır: filtre baytı (0 = yok) + pikseller.
        let row = [&[0u8][..], &[255, 0, 0, 255].repeat(SIDE as usize)].concat();
        let raw = row.repeat(SIDE as usize);
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + u32::from(x)) % 65_521;
            b = (b + a) % 65_521;
        }
        let len = raw.len() as u16;
        let mut zlib = vec![0x78, 0x01, 0x01];
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(&raw);
        zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());

        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&SIDE.to_be_bytes());
        ihdr.extend_from_slice(&SIDE.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &zlib);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    fn item(name: &str, theme_path: &str, status: &str) -> TrayItem {
        let mut props: HashMap<String, OwnedValue> = HashMap::new();
        for (k, v) in [("IconName", name), ("IconThemePath", theme_path), ("Status", status), ("AttentionIconName", "alert")] {
            props.insert(k.to_owned(), OwnedValue::from(Str::from(v.to_owned())));
        }
        TrayItem::from_properties(ItemId::new(":1.1", "/SNI"), 0, ":1.1", "/SNI", &props)
    }

    fn resolve(worker: &IconWorker, source: IconSource, px: u32) -> Option<Pixmap> {
        let (tx, rx) = mpsc::channel();
        worker.submit(1, IconKey { source, px }, move |p| {
            let _ = tx.send(p);
        })?;
        rx.recv_timeout(LIVENESS).expect("ikon işi bitmedi")
    }

    fn app(dir: &Path, name: &str) -> IconSource {
        IconSource::AppDir { dir: dir.to_path_buf(), name: name.into() }
    }

    #[test]
    fn names_are_validated() {
        for ok in ["nm-signal-75", "org.kde.foo", "a_b+c", "x"] {
            assert!(valid_icon_name(ok), "{ok}");
        }
        for bad in ["", "../../etc/passwd", "a/b", ".hidden", "a..b", "ä", "a b", &"x".repeat(129)] {
            assert!(!valid_icon_name(bad), "{bad}");
        }
        assert_eq!(IconKey::for_item(&item("../../etc/passwd", "", "Active"), 32), None);
        assert_eq!(IconKey::for_item(&item("/etc/passwd", "", "Active"), 32), None, "uzantısız mutlak yol");
        assert_eq!(IconKey::for_item(&item("/proc/self/fd/0.png", "", "Active"), 32), None);
        assert!(matches!(
            IconKey::for_item(&item("/opt/app/icon.png", "", "Active"), 32).unwrap().source,
            IconSource::File { .. }
        ));
        let k = IconKey::for_item(&item("nm-signal", "/opt/icons", "Active"), 32).unwrap();
        assert_eq!(k.source, app(Path::new("/opt/icons"), "nm-signal"));
        let k = IconKey::for_item(&item("nm-signal", "relative/dir", "NeedsAttention"), 32).unwrap();
        assert_eq!(k.source, IconSource::Theme { name: "alert".into() }, "göreli tema yolu yok sayılır");
        assert_eq!(clamp_px(0), MIN_ICON_PX);
        assert_eq!(clamp_px(10_000), MAX_ICON_PX);
    }

    #[test]
    fn resolves_flat_and_hicolor_icons_in_app_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("flat.png"), red_png()).unwrap();
        std::fs::create_dir_all(dir.path().join("hicolor/16x16/apps")).unwrap();
        std::fs::create_dir_all(dir.path().join("hicolor/scalable/apps")).unwrap();
        std::fs::write(dir.path().join("hicolor/16x16/apps/deep.png"), red_png()).unwrap();
        let w = IconWorker::new(None);
        let p = resolve(&w, app(dir.path(), "flat"), 16).expect("düz dosya");
        assert_eq!((p.width, p.height), (16, 16));
        assert_eq!(&p.rgba[..4], &[255, 0, 0, 255]);
        assert!(resolve(&w, app(dir.path(), "deep"), 16).is_some(), "hicolor alt yolu");
        assert!(resolve(&w, app(dir.path(), "nuxut-no-such-icon-xyz"), 16).is_none());
    }

    /// Sistem temasında yalnızca SVG olarak bulunabilen bir ad (hicolor/scalable) System
    /// güveniyle çözülür (tema daha uygun bir PNG bulursa onu çözer; ikisi de çözülmeli).
    /// Makinede hicolor'da SVG yoksa test bir şey iddia etmez.
    #[test]
    fn system_theme_svg_is_trusted() {
        let Ok(entries) = std::fs::read_dir("/usr/share/icons/hicolor/scalable/apps") else { return };
        let name = entries.filter_map(Result::ok).find_map(|e| {
            let n = e.file_name().into_string().ok()?;
            let stem = n.strip_suffix(".svg")?;
            (valid_icon_name(stem) && e.file_type().is_ok_and(|t| t.is_file())).then(|| stem.to_owned())
        });
        let Some(name) = name else { return };
        let w = IconWorker::new(None);
        assert!(resolve(&w, IconSource::Theme { name: name.clone() }, 32).is_some(), "{name}");
    }

    /// Uygulamanın verdiği SVG (dizinde, mutlak yolda ya da dbusmenu baytı olarak) güvenilmez:
    /// süreç içinde ayrıştırılmaz, pixmap yedeğine düşülür.
    #[test]
    fn app_supplied_svg_is_never_parsed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vec.svg"), RED_SVG).unwrap();
        std::fs::create_dir_all(dir.path().join("hicolor/scalable/apps")).unwrap();
        std::fs::write(dir.path().join("hicolor/scalable/apps/deepvec.svg"), RED_SVG).unwrap();
        // Uzantı yalan söylese de içerik belirler.
        std::fs::write(dir.path().join("liar.png"), RED_SVG).unwrap();
        let w = IconWorker::new(None);
        assert!(resolve(&w, app(dir.path(), "vec"), 16).is_none(), "app dizinindeki SVG");
        assert!(resolve(&w, app(dir.path(), "deepvec"), 16).is_none(), "hicolor/scalable SVG");
        assert!(resolve(&w, app(dir.path(), "liar"), 16).is_none(), ".png uzantılı SVG");
        let file = IconSource::File { path: dir.path().join("vec.svg") };
        assert!(resolve(&w, file, 16).is_none(), "mutlak yol SVG");
        assert!(resolve(&w, IconSource::Png(RED_SVG.as_bytes().into()), 16).is_none(), "icon-data SVG");
        // Karşılaştırma: aynı yollardan PNG çözülür.
        std::fs::write(dir.path().join("ras.png"), red_png()).unwrap();
        assert!(resolve(&w, app(dir.path(), "ras"), 16).is_some());
    }

    /// Sandbox kurulu ama işçi yok (`Unavailable`, ADR-0007 K2-b): uygulamanın SVG'si yine
    /// süreç içine düşmez (Err → `None` → pixmap/yer tutucu yedeği), temel raster süreç içinde
    /// çözülür, bozuk bayt yedeğe düşer. Gerçek işçiyle yol `tests/image_sandbox.rs`'te.
    #[test]
    fn unavailable_sandbox_keeps_svg_out_and_falls_back() {
        use nuxut_image_sandbox::{Sandbox, SandboxConfig};
        let config = SandboxConfig { worker: Some(PathBuf::from("/nonexistent/nuxut-image-worker")), ..SandboxConfig::default() };
        let sb = Arc::new(Sandbox::new(config));
        assert_eq!(sb.isolation(), nuxut_image::Isolation::Unavailable);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vec.svg"), RED_SVG).unwrap();
        std::fs::write(dir.path().join("ras.png"), red_png()).unwrap();
        std::fs::write(dir.path().join("broken.png"), b"\x89PNG\r\n\x1a\nbozuk").unwrap();
        let w = IconWorker::new(Some(sb.clone() as Arc<dyn IsolatedDecoder>));
        assert!(resolve(&w, app(dir.path(), "vec"), 16).is_none(), "SVG süreç içine düşmez");
        assert!(resolve(&w, IconSource::Png(RED_SVG.as_bytes().into()), 16).is_none(), "icon-data SVG");
        assert!(resolve(&w, app(dir.path(), "broken"), 16).is_none(), "bozuk → yedek");
        let p = resolve(&w, app(dir.path(), "ras"), 16).expect("K2-b: PNG süreç içinde");
        assert_eq!(&p.rgba[..4], &[255, 0, 0, 255]);
        let stats = sb.stats();
        assert_eq!(stats.spawned, 0, "işçi yok, süreç başlamaz");
        assert!(stats.in_process_fallbacks >= 1, "{stats:?}");
    }

    #[test]
    fn rejects_escapes_and_special_files() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("theme");
        std::fs::create_dir(&dir).unwrap();
        let secret = root.path().join("secret.png");
        std::fs::write(&secret, red_png()).unwrap();
        let w = IconWorker::new(None);

        // Dosya symlink'i dışarıyı gösteriyor.
        symlink(&secret, dir.join("link.png")).unwrap();
        assert!(resolve(&w, app(&dir, "link"), 16).is_none(), "symlink dosya");
        // Ara dizin symlink'i dışarı kaçıyor.
        std::fs::create_dir(root.path().join("elsewhere")).unwrap();
        std::fs::create_dir_all(root.path().join("elsewhere/16x16/apps")).unwrap();
        std::fs::write(root.path().join("elsewhere/16x16/apps/esc.png"), red_png()).unwrap();
        symlink(root.path().join("elsewhere"), dir.join("hicolor")).unwrap();
        assert!(resolve(&w, app(&dir, "esc"), 16).is_none(), "ara dizin symlink'i");
        // Tema dizininin kendisi symlink: kanonik hedefin içinde kalan dosya kabul edilir.
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("ok.png"), red_png()).unwrap();
        symlink(&real, root.path().join("alias")).unwrap();
        assert!(resolve(&w, app(&root.path().join("alias"), "ok"), 16).is_some());
        // /proc'a işaret eden tema yolu.
        assert!(resolve(&w, app(Path::new("/proc/self"), "environ"), 16).is_none());
        // FIFO beklemez: hiçbir yazar açmadığından bloklayan bir açılış hiç dönmezdi (canlılık
        // sınırı yakalar); saat ölçümü gerekmez.
        let fifo = dir.join("fifo.png");
        rustix::fs::mknodat(rustix::fs::CWD, &fifo, rustix::fs::FileType::Fifo, rustix::fs::Mode::from_raw_mode(0o600), 0)
            .unwrap();
        assert!(resolve(&w, app(&dir, "fifo"), 16).is_none());
        // Sınırı bir bayt aşan dosya okunmaz — içeriği geçerli PNG olsa bile (boyut kapısının
        // kanıtı zamana değil sonuca dayanır).
        let mut over = red_png();
        over.resize(usize::try_from(MAX_ICON_FILE_BYTES).unwrap() + 1, 0);
        std::fs::write(dir.join("over.png"), &over).unwrap();
        assert!(resolve(&w, app(&dir, "over"), 16).is_none(), "sınırı aşan dosya");
        // Devasa (seyrek) dosya okunmaz.
        let big = std::fs::File::create(dir.join("big.png")).unwrap();
        big.set_len(4 << 30).unwrap();
        assert!(resolve(&w, app(&dir, "big"), 16).is_none());
    }

    /// Dizin listelenemese bile (yalnızca `x` izni) bilinen ad çözülür: tarama yapılmadığının
    /// kanıtı.
    #[test]
    fn app_dir_is_never_listed() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("nolist");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("known.png"), red_png()).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o311)).unwrap();
        let w = IconWorker::new(None);
        let found = resolve(&w, app(&dir, "known"), 16);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(found.is_some());
    }

    /// Zamanlamadan bağımsız: işçi bir kapı işinde bloklanır, kuyruk tam kapasiteye dolar.
    #[test]
    fn queue_is_bounded_and_cancel_skips_work() {
        let w = IconWorker::new(None);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("i.png"), red_png()).unwrap();

        // Kapı işi: okunamayan dosya, `done` ikon thread'inde çağrılır ve serbest bırakılana
        // dek orada bekler. Doğruluk hiçbir saate bağlı değil: kapı yalnızca `release_tx`
        // düşünce açılır (zaman aşımı yok — yük altında ya da bellek bekçisi süreci
        // dondurduğunda erken açılıp kuyruğu boşaltamaz). Asılı kalmaz: `release_tx` `w`'den
        // sonra tanımlı, panikte önce o düşer ve `IconWorker::drop`'taki join beklemez.
        // Kalan zaman aşımları yalnızca canlılıktır ([`LIVENESS`]).
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let gate = IconKey { source: IconSource::File { path: dir.path().join("yok.png") }, px: 16 };
        w.submit(1, gate, move |_| {
            let _ = entered_tx.send(());
            let _ = release_rx.recv();
        })
        .expect("kapı işi");
        entered_rx.recv_timeout(LIVENESS).expect("işçi kapıya girmedi");

        let (tx, rx) = mpsc::channel();
        let mut accepted = 0;
        for _ in 0..QUEUE_CAPACITY * 4 {
            let tx = tx.clone();
            if let Some(h) = w.submit(2, IconKey { source: app(dir.path(), "i"), px: 16 }, move |p| {
                let _ = tx.send(p.is_some());
            }) {
                h.cancel();
                accepted += 1;
            }
        }
        assert_eq!(accepted, QUEUE_CAPACITY, "kuyruk sınırı");
        drop(release_tx);
        let results: Vec<bool> = (0..accepted).map(|_| rx.recv_timeout(LIVENESS).expect("iş bitmedi")).collect();
        assert!(results.iter().all(|r| !r), "iptal edilen işler çözülmez");
    }
}
