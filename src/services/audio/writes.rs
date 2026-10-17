//! Yazım kuyruğu (saf): düğüm başına en çok bir yazım yolda; yoldayken gelenler tek bekleyen
//! yazımda birleşir (kaydırma patlamasında yalnızca son değer gider). Gösterilen seviye, yazım
//! sunucuya ulaşıp yankısı gelene dek hedef değerdir: ardışık göreli adımlar yankıyı beklemeden
//! birikir, gecikmiş bir ara yankı gösterilen değeri geri çekmez.
//!
//! Yaşam döngüsü: [`Writes::request`] → (hemen gönderilecekse) [`Writes::sent`] → sunucu
//! gidiş-dönüşü bitince [`Writes::done`] (bekleyen varsa onu döndürür) → sunucudaki değer hedefe
//! ulaşınca ya da [`SETTLE`] dolunca hedef bırakılır ([`Writes::reconcile`], [`Writes::expire`]).
//! Route listesi gelmemiş kart düğümü için yazım [`Writes::park`] ile bekletilir.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::model::{VolumeWrite, peak};

/// Yazım bittikten sonra hedefin sunucudan teyidi için beklenen en uzun süre.
pub(crate) const SETTLE: Duration = Duration::from_millis(1000);
/// Route listesi beklenirken bekletilen yazımın en uzun bekleme süresi.
pub(crate) const PARK: Duration = Duration::from_millis(1500);
/// Hedef ile sunucu değeri arasındaki tolerans (doğrusal).
const EPSILON: f32 = 1e-3;

#[derive(Debug, Default, Clone, PartialEq)]
struct Slot {
    target_channels: Option<Vec<f32>>,
    target_mute: Option<bool>,
    /// Gönderilmemiş (birleştirilmiş) yazım.
    pending: Option<VolumeWrite>,
    /// Yoldaki yazımın jetonu.
    in_flight: Option<u64>,
    /// Route listesi bekleniyor; bu ana dek.
    parked_until: Option<Instant>,
    /// Yazım bitti; hedef en geç bu anda bırakılır.
    settle_at: Option<Instant>,
}

impl Slot {
    fn idle(&self) -> bool {
        self.pending.is_none() && self.in_flight.is_none() && self.parked_until.is_none()
    }
}

/// Bir düğümün yazım durumu (tanı).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteState {
    InFlight { pending: bool },
    Parked,
    Settling,
}

#[derive(Debug, Default)]
pub(crate) struct Writes {
    slots: BTreeMap<u32, Slot>,
    next: u64,
    /// Gönderilen yazım sayısı (tanı).
    pub sent: u64,
    /// Birleştirilen (gönderilmeden yerini yenisine bırakan) istek sayısı (tanı).
    pub merged: u64,
}

impl Writes {
    /// Yeni istek. Hemen gönderilmesi gerekiyorsa yazımı döndürür (çağıran gönderip
    /// [`Self::sent`] çağırır); yolda bir yazım varsa bekleyene birleştirir ve `None`.
    pub fn request(&mut self, id: u32, w: VolumeWrite) -> Option<VolumeWrite> {
        let s = self.slots.entry(id).or_default();
        if let Some(c) = &w.channels {
            s.target_channels = Some(c.clone());
        }
        if let Some(m) = w.mute {
            s.target_mute = Some(m);
        }
        s.settle_at = None;
        if s.in_flight.is_some() || s.parked_until.is_some() {
            if s.pending.is_some() {
                self.merged += 1;
            }
            s.pending = Some(VolumeWrite::merged(s.pending.take(), w));
            None
        } else {
            Some(VolumeWrite::merged(s.pending.take(), w))
        }
    }

    /// Yazım gönderildi; gidiş-dönüş jetonu.
    pub fn sent(&mut self, id: u32) -> u64 {
        self.next += 1;
        self.sent += 1;
        let s = self.slots.entry(id).or_default();
        s.in_flight = Some(self.next);
        s.parked_until = None;
        self.next
    }

    /// Yazım şimdi gönderilemiyor (route listesi bekleniyor): bekletilir.
    pub fn park(&mut self, id: u32, w: VolumeWrite, now: Instant) {
        let s = self.slots.entry(id).or_default();
        s.pending = Some(VolumeWrite::merged(s.pending.take(), w));
        s.parked_until.get_or_insert(now + PARK);
    }

    /// Bekletilen düğümler.
    pub fn parked(&self) -> Vec<u32> {
        self.slots.iter().filter(|(_, s)| s.parked_until.is_some()).map(|(id, _)| *id).collect()
    }

    /// Bekletilen yazımı yeniden göndermek için alır.
    pub fn take_parked(&mut self, id: u32) -> Option<VolumeWrite> {
        let s = self.slots.get_mut(&id)?;
        s.parked_until.take()?;
        s.pending.take()
    }

    /// Süresi dolan bekletilmiş yazımlar (çağıran son çare yoluna gönderir).
    pub fn parked_expired(&mut self, now: Instant) -> Vec<(u32, VolumeWrite)> {
        let mut out = Vec::new();
        for (id, s) in &mut self.slots {
            if s.parked_until.is_some_and(|t| t <= now) {
                s.parked_until = None;
                if let Some(w) = s.pending.take() {
                    out.push((*id, w));
                }
            }
        }
        out
    }

    /// Yoldaki yazım sunucuda işlendi. Bekleyen varsa döndürür (çağıran gönderir).
    pub fn done(&mut self, id: u32, token: u64, now: Instant) -> Option<VolumeWrite> {
        let s = self.slots.get_mut(&id)?;
        if s.in_flight != Some(token) {
            return None;
        }
        s.in_flight = None;
        match s.pending.take() {
            Some(p) => Some(p),
            None => {
                s.settle_at = Some(now + SETTLE);
                None
            }
        }
    }

    /// Alt süreçle (wpctl) yazıldı: gidiş-dönüş izlenmez, hedef yalnızca teyit/süre bekler.
    pub fn optimistic(&mut self, id: u32, w: &VolumeWrite, now: Instant) {
        let s = self.slots.entry(id).or_default();
        if let Some(c) = &w.channels {
            s.target_channels = Some(c.clone());
        }
        if let Some(m) = w.mute {
            s.target_mute = Some(m);
        }
        s.settle_at = Some(now + SETTLE);
    }

    /// Yazım reddedildi ya da düğüm gitti: hedef bırakılır, sunucudaki değer gösterilir.
    pub fn forget(&mut self, id: u32) {
        self.slots.remove(&id);
    }

    /// Sunucudaki değer hedefe ulaştıysa ve iş kalmadıysa hedefi bırakır; bıraktıysa `true`.
    pub fn reconcile(&mut self, id: u32, channels: &[f32], mute: bool) -> bool {
        let Some(s) = self.slots.get(&id) else { return false };
        if !s.idle() || s.settle_at.is_none() {
            return false;
        }
        let channels_ok = s.target_channels.as_ref().is_none_or(|t| {
            t.len() == channels.len() && t.iter().zip(channels).all(|(a, b)| (a - b).abs() <= EPSILON.max(peak(t) * EPSILON))
        });
        let mute_ok = s.target_mute.is_none_or(|m| m == mute);
        if channels_ok && mute_ok {
            self.slots.remove(&id);
            return true;
        }
        false
    }

    /// Teyit süresi dolan hedefleri bırakır; bıraktıysa `true`.
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.slots.len();
        self.slots.retain(|_, s| !(s.idle() && s.settle_at.is_some_and(|t| t <= now)));
        before != self.slots.len()
    }

    /// Bir sonraki zaman sınırı (teyit ya da bekletme).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.slots.values().flat_map(|s| [s.settle_at.filter(|_| s.idle()), s.parked_until]).flatten().min()
    }

    pub fn ids(&self) -> Vec<u32> {
        self.slots.keys().copied().collect()
    }

    /// Gösterilecek hedef (kanallar, sessiz); yoksa sunucudaki değer gösterilir.
    pub fn overlay(&self, id: u32) -> (Option<&[f32]>, Option<bool>) {
        match self.slots.get(&id) {
            Some(s) => (s.target_channels.as_deref(), s.target_mute),
            None => (None, None),
        }
    }

    pub fn state(&self, id: u32) -> Option<WriteState> {
        let s = self.slots.get(&id)?;
        Some(if s.in_flight.is_some() {
            WriteState::InFlight { pending: s.pending.is_some() }
        } else if s.parked_until.is_some() {
            WriteState::Parked
        } else {
            WriteState::Settling
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(c: f32) -> VolumeWrite {
        VolumeWrite { channels: Some(vec![c, c]), mute: None }
    }

    fn mute(m: bool) -> VolumeWrite {
        VolumeWrite { channels: None, mute: Some(m) }
    }

    #[test]
    fn burst_sends_first_and_coalesces_rest_into_last() {
        let now = Instant::now();
        let mut q = Writes::default();
        assert_eq!(q.request(1, w(0.1)), Some(w(0.1)), "boşta: hemen gönder");
        let t1 = q.sent(1);
        // Yoldayken gelen 10 adım tek bekleyen yazımda birleşir.
        for i in 2..=11 {
            assert_eq!(q.request(1, w(i as f32 / 100.0)), None);
        }
        assert_eq!(q.merged, 9);
        assert_eq!(q.overlay(1).0, Some(&[0.11, 0.11][..]), "gösterilen = son hedef");
        // Gidiş-dönüş bitti: yalnızca son değer gider.
        assert_eq!(q.done(1, t1, now), Some(w(0.11)));
        let t2 = q.sent(1);
        assert_eq!(q.done(1, t2, now), None);
        assert_eq!(q.sent, 2, "11 istek, 2 yazım");
        assert_eq!(q.state(1), Some(WriteState::Settling));
    }

    #[test]
    fn mute_and_volume_merge_without_losing_either() {
        let mut q = Writes::default();
        q.request(1, w(0.2));
        let t = q.sent(1);
        q.request(1, mute(true));
        q.request(1, w(0.3));
        let next = q.done(1, t, Instant::now()).unwrap();
        assert_eq!(next, VolumeWrite { channels: Some(vec![0.3, 0.3]), mute: Some(true) });
    }

    #[test]
    fn stale_echo_does_not_pull_back_shown_value() {
        // Yankı yarışı: yolda 0.3 varken 0.2'nin gecikmiş yankısı gelir; gösterilen hedef kalır.
        let now = Instant::now();
        let mut q = Writes::default();
        q.request(1, w(0.2));
        let t = q.sent(1);
        q.request(1, w(0.3));
        assert!(!q.reconcile(1, &[0.2, 0.2], false), "iş sürerken hedef bırakılmaz");
        assert_eq!(q.overlay(1).0, Some(&[0.3, 0.3][..]));
        let t2 = q.done(1, t, now).map(|_| q.sent(1)).unwrap();
        q.done(1, t2, now);
        assert!(!q.reconcile(1, &[0.2, 0.2], false), "eski yankı teyit sayılmaz");
        assert!(q.reconcile(1, &[0.3, 0.3], false), "hedefe ulaşınca bırakılır");
        assert_eq!(q.overlay(1), (None, None));
    }

    #[test]
    fn settle_timeout_releases_target_when_server_differs() {
        // Donanım adımı ya da başka bir uygulama: değer hedefe hiç gelmezse süre dolunca sunucu kazanır.
        let now = Instant::now();
        let mut q = Writes::default();
        q.request(1, w(0.5));
        let t = q.sent(1);
        q.done(1, t, now);
        assert_eq!(q.next_deadline(), Some(now + SETTLE));
        assert!(!q.expire(now + SETTLE / 2));
        assert!(q.expire(now + SETTLE));
        assert_eq!(q.overlay(1), (None, None));
        assert_eq!(q.next_deadline(), None);
    }

    #[test]
    fn stale_token_is_ignored() {
        let mut q = Writes::default();
        q.request(1, w(0.2));
        let t = q.sent(1);
        q.forget(1);
        q.request(1, w(0.4));
        let t2 = q.sent(1);
        assert_ne!(t, t2);
        assert_eq!(q.done(1, t, Instant::now()), None, "eski oturumun jetonu");
        assert_eq!(q.state(1), Some(WriteState::InFlight { pending: false }));
    }

    #[test]
    fn parked_until_routes_arrive_or_timeout() {
        let now = Instant::now();
        let mut q = Writes::default();
        let first = q.request(1, w(0.2)).unwrap();
        q.park(1, first, now);
        assert_eq!(q.request(1, w(0.3)), None, "bekletilirken birleşir");
        assert_eq!(q.parked(), [1]);
        assert_eq!(q.state(1), Some(WriteState::Parked));
        assert_eq!(q.take_parked(1), Some(w(0.3)));
        assert!(q.parked().is_empty());
        // Route hiç gelmezse süre dolunca son çare yoluna verilir.
        let again = q.request(1, w(0.4)).unwrap();
        q.park(1, again, now);
        assert!(q.parked_expired(now + PARK / 2).is_empty());
        assert_eq!(q.parked_expired(now + PARK), vec![(1, w(0.4))]);
    }

    #[test]
    fn optimistic_wpctl_writes_settle() {
        let now = Instant::now();
        let mut q = Writes::default();
        q.optimistic(1, &mute(true), now);
        assert_eq!(q.overlay(1), (None, Some(true)));
        assert!(q.reconcile(1, &[0.5], true));
    }
}
