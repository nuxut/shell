//! Dosya seçici ızgara görünümünün küçük resimleri (UI'sız; `docs/design/file-chooser.md`).
//!
//! - **Yalnız yalıtılmış çözüm (ADR-0007):** dosya [`nuxut_image::open_user_file`] ile açılır,
//!   çözüm [`IsolatedDecoder::decode_file`]'dadır (görünüm `nuxut-image-sandbox`'ı duvar kağıdı
//!   profilinde, `in_process_fallback = false` ile kurar: işçi yoksa önizleme yok, süreç içinde
//!   hiçbir bayt ayrıştırılmaz). Biçim yalnızca addan seçilir (çağıran); içerik koklanmaz.
//! - **Yalnız görünen hücreler:** [`Thumbnailer::request`] kuyruğu her çağrıda DEĞİŞTİRİR
//!   (kaydırınca eski istekler düşer); en çok [`MAX_QUEUE`] iş. Nesil değişince (yeni klasör,
//!   yeni liste) süren çözüm iptal edilir, eski nesil sonucu gönderilmez.
//! - Tek işçi thread'i; dosya açma (ağ dosya sistemi bloklayabilir) ana thread'de değil.
//! - Yol ve ad loglanmaz.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

use nuxut_image::{CancelFlag, DecodeOptions, Fit, Image, IsolatedDecoder, Limits, TargetSize};

/// Kuyruktaki en fazla iş (görünen hücreler + önden yükleme payı bundan azdır).
pub const MAX_QUEUE: usize = 256;

/// Tek küçük resim isteği: ızgaradaki indis ve dosyanın yolu.
#[derive(Clone, PartialEq, Eq)]
pub struct ThumbJob {
    pub index: usize,
    pub path: PathBuf,
}

impl std::fmt::Debug for ThumbJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThumbJob").field("index", &self.index).finish_non_exhaustive()
    }
}

/// Sonuç. `image == None`: çözülemedi (biçim, sınır, işçi yok, iptal değil); ikon kalır.
pub struct Thumb {
    pub generation: u64,
    pub index: usize,
    pub image: Option<Image>,
}

impl std::fmt::Debug for Thumb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Thumb").field("generation", &self.generation).field("index", &self.index).field("ok", &self.image.is_some()).finish()
    }
}

#[derive(Default)]
struct State {
    generation: u64,
    queue: VecDeque<ThumbJob>,
    /// Süren işin nesli, indisi ve iptal bayrağı.
    running: Option<(u64, usize, CancelFlag)>,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Küçük resim kuyruğu ve işçi thread'i. Düşünce süren çözüm iptal edilir ve thread beklenir.
pub struct Thumbnailer {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Thumbnailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.shared.lock();
        f.debug_struct("Thumbnailer").field("generation", &st.generation).field("queued", &st.queue.len()).finish()
    }
}

impl Thumbnailer {
    /// `edge_px`: küçük resmin en uzun kenarı (fiziksel piksel; `Contain`).
    pub fn new(decoder: Arc<dyn IsolatedDecoder>, edge_px: u32, sink: impl Fn(Thumb) + Send + 'static) -> std::io::Result<Self> {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), cv: Condvar::new() });
        let s = Arc::clone(&shared);
        let edge = edge_px.clamp(16, 1024);
        let thread = std::thread::Builder::new().name("nuxut-fc-thumbs".into()).spawn(move || run(&s, decoder.as_ref(), edge, &sink))?;
        Ok(Self { shared, thread: Some(thread) })
    }

    /// Kuyruğu `jobs` ile değiştirir (sıra korunur, en çok [`MAX_QUEUE`]). `generation` öncekinden
    /// farklıysa süren çözüm iptal edilir. Aynı nesilde süren işin indisi yeniden kuyruğa alınmaz.
    pub fn request(&self, generation: u64, jobs: Vec<ThumbJob>) {
        let mut st = self.shared.lock();
        if st.generation != generation {
            st.generation = generation;
            if let Some((_, _, cancel)) = &st.running {
                cancel.cancel();
            }
        }
        let running = st.running.as_ref().filter(|(g, ..)| *g == generation).map(|(_, i, _)| *i);
        st.queue = jobs.into_iter().filter(|j| Some(j.index) != running).take(MAX_QUEUE).collect();
        drop(st);
        self.shared.cv.notify_one();
    }

    /// Bekleyen iş sayısı (testler).
    pub fn queued(&self) -> usize {
        self.shared.lock().queue.len()
    }
}

impl Drop for Thumbnailer {
    fn drop(&mut self) {
        {
            let mut st = self.shared.lock();
            st.stop = true;
            st.queue.clear();
            if let Some((_, _, cancel)) = &st.running {
                cancel.cancel();
            }
        }
        self.shared.cv.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(shared: &Shared, decoder: &dyn IsolatedDecoder, edge: u32, sink: &dyn Fn(Thumb)) {
    let opts = DecodeOptions { limits: Limits::WALLPAPER, target: Some(TargetSize::new(edge, edge, Fit::Contain)), animate: false };
    loop {
        let (generation, job, cancel) = {
            let mut st = shared.lock();
            loop {
                if st.stop {
                    return;
                }
                if let Some(job) = st.queue.pop_front() {
                    let cancel = CancelFlag::new();
                    let g = st.generation;
                    st.running = Some((g, job.index, cancel.clone()));
                    break (g, job, cancel);
                }
                st = shared.cv.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };
        let image = nuxut_image::open_user_file(&job.path)
            .and_then(|file| decoder.decode_file(nuxut_image::file_key("filechooser", &job.path), file, &opts, &cancel))
            .map(|d| d.image);
        let current = {
            let mut st = shared.lock();
            st.running = None;
            !st.stop && st.generation == generation
        };
        match image {
            Err(nuxut_image::ImageError::Cancelled) => {}
            _ if !current || cancel.is_cancelled() => {}
            Ok(image) => sink(Thumb { generation, index: job.index, image: Some(image) }),
            Err(_) => {
                // Hata ayrıntısı yol/ad içerebilir: loglanmaz.
                tracing::debug!("küçük resim çözülemedi");
                sink(Thumb { generation, index: job.index, image: None });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_image::{ImageError, Isolation};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Sahte yalıtılmış çözücü: içerik ilk baytı `b'x'` ise hata, değilse 2×2 görsel; `slow`
    /// ise iptal edilene dek bekler.
    #[derive(Debug, Default)]
    struct Fake {
        calls: AtomicUsize,
        slow: bool,
    }

    impl IsolatedDecoder for Fake {
        fn isolation(&self) -> Isolation {
            Isolation::Process
        }
        fn decode(&self, _key: u64, bytes: Arc<[u8]>, _opts: &DecodeOptions) -> Result<Image, ImageError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if bytes.first() == Some(&b'x') {
                return Err(ImageError::Unsupported("test"));
            }
            Image::from_premultiplied(2, 2, vec![255; 16])
        }
        fn decode_file(&self, key: u64, file: std::fs::File, opts: &DecodeOptions, cancel: &CancelFlag) -> Result<nuxut_image::DecodedFile, ImageError> {
            if self.slow {
                while !cancel.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                return Err(ImageError::Cancelled);
            }
            use std::io::Read;
            let mut bytes = Vec::new();
            let _ = file.take(16).read_to_end(&mut bytes);
            let image = self.decode(key, bytes.into(), opts)?;
            Ok(nuxut_image::DecodedFile { image, animated_source: false, animation: None })
        }
    }

    fn files(dir: &std::path::Path) -> (PathBuf, PathBuf) {
        let ok = dir.join("a.png");
        let bad = dir.join("b.png");
        std::fs::write(&ok, b"png").unwrap();
        std::fs::write(&bad, b"xxx").unwrap();
        (ok, bad)
    }

    #[test]
    fn decodes_requested_and_reports_failures() {
        let dir = tempfile::tempdir().unwrap();
        let (ok, bad) = files(dir.path());
        let (tx, rx) = mpsc::channel();
        let t = Thumbnailer::new(Arc::new(Fake::default()), 128, move |th| {
            let _ = tx.send(th);
        })
        .unwrap();
        t.request(1, vec![ThumbJob { index: 0, path: ok }, ThumbJob { index: 3, path: bad }, ThumbJob { index: 4, path: dir.path().join("yok") }]);
        let mut got: Vec<(usize, bool)> = (0..3).map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap()).map(|th| (th.index, th.image.is_some())).collect();
        got.sort_unstable();
        assert_eq!(got, [(0, true), (3, false), (4, false)]);
        // Dizin ve özel dosya açılmaz (open_user_file yalnız düzenli dosya).
        t.request(1, vec![ThumbJob { index: 5, path: dir.path().to_path_buf() }]);
        let th = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(th.index == 5 && th.image.is_none());
    }

    #[test]
    fn request_replaces_queue_and_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let (ok, _) = files(dir.path());
        let fake = Arc::new(Fake { slow: true, ..Fake::default() });
        let t = Thumbnailer::new(fake, 64, |_| {}).unwrap();
        let jobs = |n: usize| (0..n).map(|i| ThumbJob { index: i, path: ok.clone() }).collect::<Vec<_>>();
        t.request(1, jobs(MAX_QUEUE + 50));
        std::thread::sleep(Duration::from_millis(50));
        // Biri sürüyor (yavaş), kalan sınırlı.
        assert!(t.queued() <= MAX_QUEUE);
        t.request(1, jobs(3));
        assert!(t.queued() <= 3, "kaydırma eski istekleri düşürür");
        // Düşürmek takılmaz: süren çözüm iptal edilir.
        let started = std::time::Instant::now();
        drop(t);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn new_generation_cancels_and_drops_stale_results() {
        let dir = tempfile::tempdir().unwrap();
        let (ok, _) = files(dir.path());
        let (tx, rx) = mpsc::channel();
        let fake = Arc::new(Fake { slow: true, ..Fake::default() });
        let t = Thumbnailer::new(fake, 64, move |th| {
            let _ = tx.send(th);
        })
        .unwrap();
        t.request(1, vec![ThumbJob { index: 0, path: ok.clone() }]);
        std::thread::sleep(Duration::from_millis(50));
        // Yeni klasör: süren iş iptal, sonucu gönderilmez.
        t.request(2, Vec::new());
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "eski nesil sonucu yok");
    }
}
