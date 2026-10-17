//! Arka planda bellek içi render ([`Renderer`], süreç başına bir tane) ve yüzey başına istek
//! durumu ([`Slot`]).
//!
//! Sonuç bir [`Frame`]'dir: paylaşılan (`Arc`) opak RGBA tamponu; tüketici onu framework
//! `Wallpaper::set_image`'a kopyasız verir. Disk önbelleği ve PNG turu yoktur: kaynak gömülü ve
//! güvenilir, kare süreç içinde üretilir (`docs/design/wallpapers.md` "Bellek içi kare").
//!
//! Aynı anahtar (kimlik, varyant, fiziksel boyut) için eşzamanlı istekler tek render'da
//! birleşir (aynı boyuttaki çıkışlar). İş varken tek bir thread çalışır, kuyruk boşalınca
//! biter: boşta thread yok.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use crate::{Builtin, ColorThemeId, RenderError, Variant};

/// Render boru hattının sürümü: render ya da dither davranışı değişince artırın ([`image_key`]).
pub const PIPELINE: u32 = 2;

/// Bir render'ın sonucu: opak düz RGBA8 (alfa her yerde 255; premultiplied = düz).
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    /// [`image_key`]: aynı içerik için aynı (framework `set_image` anahtarı).
    pub key: u64,
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bayt; yüzeyler ve bileşen kopyasız paylaşır.
    pub rgba: Arc<[u8]>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame").field("width", &self.width).field("height", &self.height).finish_non_exhaustive()
    }
}

/// Duvar kağıdı bileşenine kopyasız kare (`Wallpaper::set_image(frame.key, frame)`; anahtar
/// önce okunur).
#[cfg(feature = "frame")]
impl From<Frame> for nuxut_image::WallpaperFrame {
    fn from(f: Frame) -> Self {
        Self { width: f.width, height: f.height, pixels: f.rgba, animated_source: false }
    }
}

fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Yerleşik + varyant + renk temasının içerik anahtarı: FNV-1a 64(SVG ‖ kimlik ‖ [`PIPELINE`]
/// [‖ tema adı]). Nuxut mavisinde tema eklenmez (önceki anahtarlar aynı kalır). Boyut anahtarda
/// değil (framework aynı anahtarı kare boyutuyla birlikte karşılaştırır).
pub fn image_key(builtin: &Builtin, variant: Variant, theme: ColorThemeId) -> u64 {
    let h = fnv(builtin.svg(variant), 0xcbf2_9ce4_8422_2325);
    let h = fnv(&PIPELINE.to_le_bytes(), fnv(builtin.id.as_bytes(), h));
    if theme == ColorThemeId::Blue { h } else { fnv(theme.name().as_bytes(), h) }
}

type Done = Box<dyn FnOnce(Result<Frame, RenderError>) + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    builtin: &'static Builtin,
    variant: Variant,
    theme: ColorThemeId,
    size: (u32, u32),
}

impl std::hash::Hash for Builtin {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[derive(Default)]
struct State {
    queue: VecDeque<Key>,
    waiters: HashMap<Key, Vec<Done>>,
    running: bool,
    /// Tamamlanan render sayısı (birleşen istekler bir sayılır); tanı/test.
    completed: u64,
}

/// Süreç başına paylaşılan yerleşik duvar kağıdı render'ı.
#[derive(Default)]
pub struct Renderer {
    state: Arc<Mutex<State>>,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer").finish_non_exhaustive()
    }
}

impl Renderer {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    /// Tamamlanan render sayısı (birleşen istekler bir sayılır).
    pub fn completed(&self) -> u64 {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).completed
    }

    /// Kareyi arka planda render eder; `done` iş thread'inden çağrılır.
    pub fn request(
        &self,
        builtin: &'static Builtin,
        variant: Variant,
        theme: ColorThemeId,
        size: (u32, u32),
        done: impl FnOnce(Result<Frame, RenderError>) + Send + 'static,
    ) {
        let key = Key { builtin, variant, theme, size };
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match st.waiters.get_mut(&key) {
            Some(list) => list.push(Box::new(done)),
            None => {
                st.waiters.insert(key, vec![Box::new(done)]);
                st.queue.push_back(key);
            }
        }
        if st.running {
            return;
        }
        st.running = true;
        drop(st);
        let state = self.state.clone();
        let spawned = std::thread::Builder::new().name("nuxut-wp-builtin".into()).spawn(move || work(&state));
        if let Err(e) = spawned {
            let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            st.running = false;
            st.queue.clear();
            let waiters: Vec<Done> = st.waiters.drain().flat_map(|(_, v)| v).collect();
            drop(st);
            for w in waiters {
                w(Err(RenderError::Thread(e.to_string())));
            }
        }
    }
}

fn work(state: &Mutex<State>) {
    loop {
        let key = {
            let mut st = state.lock().unwrap_or_else(PoisonError::into_inner);
            match st.queue.pop_front() {
                Some(k) => k,
                None => {
                    st.running = false;
                    return;
                }
            }
        };
        let started = std::time::Instant::now();
        // Vec → Arc<[u8]> kopyası burada (iş thread'inde) bir kez; sonrası kopyasız paylaşım.
        let result = crate::render_themed(key.builtin, key.variant, key.theme, key.size.0, key.size.1).map(|r| Frame {
            key: image_key(key.builtin, key.variant, key.theme),
            width: r.width,
            height: r.height,
            rgba: Arc::from(r.rgba),
        });
        tracing::debug!(id = key.builtin.id, variant = key.variant.name(), theme = key.theme.name(), w = key.size.0, h = key.size.1, ms = started.elapsed().as_millis() as u64, ok = result.is_ok(), "yerleşik duvar kağıdı hazır");
        let waiters = {
            let mut st = state.lock().unwrap_or_else(PoisonError::into_inner);
            st.completed += 1;
            st.waiters.remove(&key).unwrap_or_default()
        };
        for w in waiters {
            w(result.clone());
        }
    }
}

/// Bir yüzeyin yerleşik duvar kağıdı isteği: istenen anahtar ve son sonucu. Yeni istek
/// eskisinin sonucunu geçersiz kılar (kuşak).
pub struct Slot {
    tx: Sender<(u64, Result<Frame, RenderError>)>,
    rx: Receiver<(u64, Result<Frame, RenderError>)>,
    generation: u64,
    wanted: Option<Key>,
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slot").field("generation", &self.generation).field("wanted", &self.wanted).finish()
    }
}

impl Default for Slot {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, generation: 0, wanted: None }
    }
}

impl Slot {
    /// Bu anahtar isteniyor. Öncekiyle aynıysa bir şey yapılmaz ve `false` döner; değilse
    /// render istenir (sonuç gelince `notify`; sonuç [`Slot::poll`] ile alınır).
    pub fn want(
        &mut self,
        renderer: &Renderer,
        builtin: &'static Builtin,
        variant: Variant,
        theme: ColorThemeId,
        size: (u32, u32),
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> bool {
        let key = Key { builtin, variant, theme, size };
        if self.wanted == Some(key) {
            return false;
        }
        self.wanted = Some(key);
        self.generation += 1;
        let generation = self.generation;
        let tx = self.tx.clone();
        renderer.request(builtin, variant, theme, size, move |r| {
            if tx.send((generation, r)).is_ok() {
                notify();
            }
        });
        true
    }

    /// Yerleşik kaynak artık istenmiyor (dosya yoluna geçildi): bekleyen sonuç atılır.
    pub fn clear(&mut self) {
        if self.wanted.take().is_some() {
            self.generation += 1;
        }
    }

    /// İstenen (kimlik, varyant, boyut) — tanı/test.
    pub fn wanted(&self) -> Option<(&'static str, Variant, (u32, u32))> {
        self.wanted.map(|k| (k.builtin.id, k.variant, k.size))
    }

    /// Güncel isteğin sonucu geldiyse döner (eski kuşaklar atılır).
    pub fn poll(&mut self) -> Option<Result<Frame, RenderError>> {
        let mut latest = None;
        while let Ok((g, r)) = self.rx.try_recv() {
            if g == self.generation && self.wanted.is_some() {
                latest = Some(r);
            }
        }
        latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn notifier() -> (Arc<dyn Fn() + Send + Sync>, Receiver<()>) {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        (
            Arc::new(move || {
                let _ = tx.lock().unwrap_or_else(PoisonError::into_inner).send(());
            }),
            rx,
        )
    }

    fn wait(slot: &mut Slot, rx: &Receiver<()>) -> Result<Frame, RenderError> {
        rx.recv_timeout(Duration::from_secs(20)).expect("bildirim");
        slot.poll().expect("sonuç")
    }

    #[test]
    fn same_key_coalesces_and_slots_share_buffer() {
        let r = Renderer::new();
        let (notify, rx) = notifier();
        let b = crate::default_builtin();
        let mut a = Slot::default();
        let mut c = Slot::default();
        assert!(a.want(&r, b, Variant::Dark, ColorThemeId::Blue, (96, 54), notify.clone()));
        assert!(!a.want(&r, b, Variant::Dark, ColorThemeId::Blue, (96, 54), notify.clone()), "aynı istek yinelenmez");
        assert!(c.want(&r, b, Variant::Dark, ColorThemeId::Blue, (96, 54), notify.clone()));
        let fa = wait(&mut a, &rx).unwrap();
        let fc = wait(&mut c, &rx).unwrap();
        assert_eq!((fa.width, fa.height, fa.rgba.len()), (96, 54, 96 * 54 * 4));
        assert_eq!(fa, fc);
        assert!(fa.rgba.chunks_exact(4).all(|p| p[3] == 255), "opak");
        assert_eq!(fa.key, image_key(b, Variant::Dark, ColorThemeId::Blue));
        // Birleştiyse tek tampon, birleşmediyse (ilki bitmişti) iki render; ikisi de aynı içerik.
        assert!(Arc::ptr_eq(&fa.rgba, &fc.rgba) || r.completed() == 2);
    }

    #[test]
    fn newer_request_supersedes_older() {
        let r = Renderer::new();
        let (notify, rx) = notifier();
        let b = crate::find("aurora").unwrap();
        let mut s = Slot::default();
        s.want(&r, b, Variant::Dark, ColorThemeId::Blue, (64, 36), notify.clone());
        s.want(&r, b, Variant::Light, ColorThemeId::Blue, (64, 36), notify.clone());
        rx.recv_timeout(Duration::from_secs(20)).unwrap();
        rx.recv_timeout(Duration::from_secs(20)).unwrap();
        let f = s.poll().expect("son istek").unwrap();
        assert_eq!(f.key, image_key(b, Variant::Light, ColorThemeId::Blue));
        assert_eq!(s.poll(), None);
        // Dosya kaynağına geçiş: bekleyen sonuç atılır.
        s.want(&r, b, Variant::Dark, ColorThemeId::Blue, (32, 18), notify);
        s.clear();
        rx.recv_timeout(Duration::from_secs(20)).unwrap();
        assert_eq!(s.poll(), None);
        assert_eq!(s.wanted(), None);
    }

    #[test]
    fn invalid_size_fails_without_panic() {
        let r = Renderer::new();
        let (tx, rx) = channel();
        r.request(crate::default_builtin(), Variant::Dark, ColorThemeId::Blue, (0, 8), move |res| tx.send(res).unwrap());
        let res = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        assert!(matches!(res, Err(RenderError::Size { .. })));
    }

    #[test]
    fn themed_frames_differ_and_keys_follow_theme() {
        let r = Renderer::new();
        let (notify, rx) = notifier();
        let b = crate::find("chase").unwrap();
        let mut blue = Slot::default();
        let mut sage = Slot::default();
        blue.want(&r, b, Variant::Dark, ColorThemeId::Blue, (64, 36), notify.clone());
        sage.want(&r, b, Variant::Dark, ColorThemeId::Sage, (64, 36), notify.clone());
        let fb = wait(&mut blue, &rx).unwrap();
        let fs = wait(&mut sage, &rx).unwrap();
        assert_eq!(fs.key, image_key(b, Variant::Dark, ColorThemeId::Sage));
        assert_ne!(fb.key, fs.key);
        assert_ne!(fb.rgba, fs.rgba, "tema renkleri karede");
        // Aynı slotta tema değişimi yeni istek sayılır.
        assert!(blue.want(&r, b, Variant::Dark, ColorThemeId::Rose, (64, 36), notify));
    }

    #[test]
    fn keys_differ_per_builtin_and_variant() {
        let mut keys: Vec<u64> = crate::BUILTINS.iter().flat_map(|b| Variant::ALL.map(|v| image_key(b, v, ColorThemeId::Blue))).collect();
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n);
    }
}
