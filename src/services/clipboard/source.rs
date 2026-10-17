//! Framework data-control adaptörü (`nuxut-wayland` `DataControl`; sözleşme framework
//! `docs/ARCHITECTURE.md` "Pano izleme") ve izleme durunca yeniden başlatma politikası.
//!
//! Olaylar ana thread'de [`DataControlSource::translate`] ile servis olayına çevrilir; okuma ve
//! seçimi koyma servis thread'inden tutamaçla yapılır (bloklayan çağrı ana thread'de
//! `MainThread` döner). Framework'ün teklif kimliği opak olduğu için servis kendi
//! [`OfferId`]'sini görür; eşleme yalnızca son birkaç teklifi tutar (framework de yalnızca
//! güncel teklifi okur, eskisi `Gone`).
//!
//! İçerik burada da loglanmaz: yalnızca hata türü.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nuxut::{DataControlEvent, DataControlHandle, DataOfferId, DataReadError, SetSelectionError, Zeroizing};
use nuxut::clipboard::Selection;

use super::{ClipboardSource, OfferId, ReadError, SetError, SourceEvent, filter};

/// Eşlemede tutulan en çok teklif (pano + birincil ve araya giren bir yenisi).
const OFFERS_KEPT: usize = 4;

/// Servis kimliği ↔ kaynak kimliği eşlemesi (kaynak kimliği opak; testte `u32`).
#[derive(Debug)]
pub struct OfferMap<T> {
    next: u64,
    /// En yeni sonda; en çok [`OFFERS_KEPT`].
    recent: Vec<(OfferId, T)>,
}

impl<T> Default for OfferMap<T> {
    fn default() -> Self {
        Self { next: 1, recent: Vec::new() }
    }
}

impl<T: Copy + PartialEq> OfferMap<T> {
    /// Yeni teklif; servis kimliğini döner. En eskisi düşer.
    pub fn insert(&mut self, offer: T) -> OfferId {
        let id = OfferId(self.next);
        self.next += 1;
        if self.recent.len() == OFFERS_KEPT {
            self.recent.remove(0);
        }
        self.recent.push((id, offer));
        id
    }

    pub fn get(&self, id: OfferId) -> Option<T> {
        self.recent.iter().find(|(i, _)| *i == id).map(|&(_, o)| o)
    }

    /// İzleme durdu: tüm teklifler geçersiz (kimlik sayacı sürer; eski kimlik yeniden çözülmez).
    pub fn clear(&mut self) {
        self.recent.clear();
    }
}

/// Framework olayının servis karşılığı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Translated {
    Event(SourceEvent),
    /// Servisin işine yaramaz (birincil seçimin boşalması): kuyruğa girmez, son teklifi
    /// gölgelemez.
    Ignored,
    /// İzleme durdu: çağıran [`DataControlSource::detach`] eder ve yeniden başlatmayı planlar.
    Stopped,
}

/// Teklif (ya da `None`: seçim boşaldı) → servis olayı. Pano ve birincil aynı akışa girer
/// (birincil yalnızca `clipboard.primary` açıkken gelir); birincil seçimin boşalması yok sayılır.
pub fn translate<T: Copy + PartialEq>(
    map: &mut OfferMap<T>,
    selection: Selection,
    offered: Option<(T, Vec<String>, filter::Flags)>,
) -> Translated {
    match (offered, selection) {
        (Some((offer, mimes, flags)), _) => Translated::Event(SourceEvent::Offered { id: map.insert(offer), mimes, flags }),
        (None, Selection::Clipboard) => Translated::Event(SourceEvent::Cleared),
        (None, _) => Translated::Ignored,
    }
}
#[derive(Debug, Default)]
struct State {
    handle: Option<DataControlHandle>,
    offers: OfferMap<DataOfferId>,
}

/// `ClipboardSource` olarak framework data-control tutamacı. İzleme durunca tutamaç bırakılır
/// ([`detach`](Self::detach)); yeniden başlayınca yenisi verilir ([`attach`](Self::attach)).
#[derive(Debug, Default)]
pub struct DataControlSource {
    state: Mutex<State>,
}

impl DataControlSource {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // Zehirlenmiş kilit: içerik (tutamaç + eşleme) yine tutarlı.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn attach(&self, handle: DataControlHandle) {
        let mut s = self.state();
        s.handle = Some(handle);
        s.offers.clear();
    }

    pub fn detach(&self) {
        let mut s = self.state();
        s.handle = None;
        s.offers.clear();
    }

    pub fn attached(&self) -> bool {
        self.state().handle.is_some()
    }

    /// Framework olayı → servis olayı.
    pub fn translate(&self, event: DataControlEvent) -> Translated {
        let mut s = self.state();
        match event {
            DataControlEvent::SelectionOffered { selection, id, mimes, own, sensitive, truncated } => {
                translate(&mut s.offers, selection, Some((id, mimes, filter::Flags { own, sensitive, truncated })))
            }
            DataControlEvent::SelectionCleared { selection } => translate::<DataOfferId>(&mut s.offers, selection, None),
            DataControlEvent::Stopped => Translated::Stopped,
        }
    }
}

/// Framework okuma hatası → servis okuma hatası (içerik taşımaz).
pub fn read_error(e: DataReadError) -> ReadError {
    match e {
        DataReadError::Gone | DataReadError::NotOffered => ReadError::Gone,
        DataReadError::TooLarge => ReadError::TooLarge,
        DataReadError::Timeout => ReadError::Timeout,
        // Sensitive/Locked: filtre zaten okumaz; framework yine de reddederse saklanmaz.
        DataReadError::Sensitive
        | DataReadError::Locked
        | DataReadError::Io
        | DataReadError::MainThread
        | DataReadError::Closed => ReadError::Failed,
    }
}

/// Framework seçim koyma hatası → servis hatası.
pub fn set_error(e: SetSelectionError) -> SetError {
    match e {
        SetSelectionError::NotStarted | SetSelectionError::Unsupported | SetSelectionError::Closed => SetError::Unsupported,
        SetSelectionError::Invalid | SetSelectionError::Locked | SetSelectionError::MainThread => SetError::Failed,
    }
}

impl ClipboardSource for DataControlSource {
    fn read(&self, id: OfferId, mime: &str, max_bytes: usize, timeout: Duration) -> Result<Zeroizing<Vec<u8>>, ReadError> {
        let (handle, offer) = {
            let s = self.state();
            let handle = s.handle.clone().ok_or(ReadError::Gone)?;
            (handle, s.offers.get(id).ok_or(ReadError::Gone)?)
        };
        // Kilit dışında: okuma bloklar (en çok `timeout`).
        handle.read(offer, mime, max_bytes, timeout).map_err(read_error)
    }

    fn set_selection(&self, data: Vec<(String, Arc<[u8]>)>) -> Result<(), SetError> {
        let handle = self.state().handle.clone().ok_or(SetError::Unsupported)?;
        handle.set_selection(Selection::Clipboard, data).map_err(set_error)
    }
}

/// İzleme durunca (`Stopped`, ya da `NoSeat` ile başlayamayınca) yeniden deneme: üstel geri
/// çekilme 1 sn → 5 dk. Son başlangıçtan beri [`STABLE`] boyunca çalıştıysa sayaç sıfırlanır
/// (ara sıra düşen bağlantı hızlı döner; hemen düşen aygıt compositor'ı dövmez).
#[derive(Debug, Default, Clone)]
pub struct Restart {
    attempt: u32,
    started: Option<Instant>,
}

/// İlk yeniden deneme gecikmesi.
pub const RETRY_FIRST: Duration = Duration::from_secs(1);
/// En uzun gecikme.
pub const RETRY_MAX: Duration = Duration::from_secs(300);
/// Bu kadar çalışan izleme "kararlı" sayılır (durunca sayaç sıfırdan başlar).
pub const STABLE: Duration = Duration::from_secs(60);

impl Restart {
    /// İzleme başladı.
    pub fn started(&mut self, now: Instant) {
        self.started = Some(now);
    }

    /// İzleme durdu ya da başlatılamadı: bir sonraki denemeye kadar beklenecek süre.
    pub fn failed(&mut self, now: Instant) -> Duration {
        if self.started.take().is_some_and(|at| now.saturating_duration_since(at) >= STABLE) {
            self.attempt = 0;
        }
        let delay = RETRY_FIRST.saturating_mul(1u32 << self.attempt.min(16)).min(RETRY_MAX);
        self.attempt = self.attempt.saturating_add(1);
        delay
    }

    /// Kullanıcı kapattı/açtı: geçmiş denemeler unutulur.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_map_keeps_only_recent() {
        let mut m = OfferMap::<u32>::default();
        let ids: Vec<_> = (0..6).map(|o| m.insert(o)).collect();
        assert_eq!(m.get(ids[5]), Some(5));
        assert_eq!(m.get(ids[2]), Some(2));
        assert_eq!(m.get(ids[1]), None, "en eski düştü");
        m.clear();
        assert_eq!(m.get(ids[5]), None);
        // Kimlik yeniden kullanılmaz: eski servis kimliği yeni teklife çözülmez.
        let next = m.insert(9);
        assert!(!ids.contains(&next));
    }

    #[test]
    fn translate_offers_and_clear() {
        let mut m = OfferMap::<u32>::default();
        let flags = filter::Flags { sensitive: true, ..filter::Flags::NONE };
        let e = translate(&mut m, Selection::Clipboard, Some((7, vec!["text/plain".into()], flags)));
        let Translated::Event(SourceEvent::Offered { id, mimes, flags: f }) = e else { panic!("teklif değil") };
        assert_eq!(m.get(id), Some(7));
        assert_eq!(mimes, ["text/plain"]);
        assert_eq!(f, flags, "işaretler servise aynen geçer (filtre atlar)");
        // Birincil teklif de aynı akışa girer; boşalması yok sayılır (son panoyu gölgelemez).
        let e = translate(&mut m, Selection::Primary, Some((8, vec!["text/plain".into()], filter::Flags::NONE)));
        assert!(matches!(e, Translated::Event(SourceEvent::Offered { .. })));
        assert_eq!(translate::<u32>(&mut m, Selection::Clipboard, None), Translated::Event(SourceEvent::Cleared));
        assert_eq!(translate::<u32>(&mut m, Selection::Primary, None), Translated::Ignored);
    }

    #[test]
    fn error_mapping_never_carries_content() {
        assert_eq!(read_error(DataReadError::Gone), ReadError::Gone);
        assert_eq!(read_error(DataReadError::NotOffered), ReadError::Gone);
        assert_eq!(read_error(DataReadError::TooLarge), ReadError::TooLarge);
        assert_eq!(read_error(DataReadError::Timeout), ReadError::Timeout);
        assert_eq!(read_error(DataReadError::Sensitive), ReadError::Failed);
        assert_eq!(read_error(DataReadError::MainThread), ReadError::Failed);
        assert_eq!(set_error(SetSelectionError::NotStarted), SetError::Unsupported);
        assert_eq!(set_error(SetSelectionError::Locked), SetError::Failed);
    }

    #[test]
    fn detached_source_reads_nothing_and_cannot_set() {
        let s = DataControlSource::default();
        assert!(!s.attached());
        assert_eq!(s.read(OfferId(1), "text/plain", 10, Duration::from_millis(1)), Err(ReadError::Gone));
        assert_eq!(s.set_selection(vec![("text/plain".into(), Arc::from(&b"x"[..]))]), Err(SetError::Unsupported));
        assert_eq!(s.translate(DataControlEvent::Stopped), Translated::Stopped);
    }

    #[test]
    fn restart_backs_off_and_resets_after_stable_run() {
        let t0 = Instant::now();
        let mut r = Restart::default();
        let delays: Vec<_> = (0..12).map(|_| r.failed(t0).as_secs()).collect();
        assert_eq!(&delays[..5], [1, 2, 4, 8, 16]);
        assert!(delays.iter().all(|&d| d <= RETRY_MAX.as_secs()));
        assert_eq!(*delays.last().unwrap(), RETRY_MAX.as_secs());
        // Hemen düşen başlangıç geri çekilmeyi sıfırlamaz.
        r.started(t0);
        assert_eq!(r.failed(t0 + Duration::from_secs(1)), RETRY_MAX);
        // Kararlı çalışma sonrası hızlı döner.
        r.started(t0);
        assert_eq!(r.failed(t0 + STABLE), RETRY_FIRST);
        assert_eq!(r.failed(t0 + STABLE), RETRY_FIRST * 2);
        r.reset();
        assert_eq!(r.failed(t0), RETRY_FIRST);
    }
}
