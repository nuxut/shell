//! Bekleyen görsel çözmelerinin bayt bütçesi (güvenlik: bellek şişirme).
//!
//! Sunucu `image-path`/`app_icon` dosyasını D-Bus çağrısında okur; kodlanmış baytlar çözme
//! kuyruğunda bekler. Bütçe olmadan herhangi bir uygulama art arda bildirimle kuyruğu
//! (`MAX_LIVE` × 2 × 16 MiB'a kadar) şişirebilirdi. Her iş kuyruğa girmeden önce bayt ayırır
//! ([`Reservation`]); ayırma iş bitince (çözüldü, hata, iptal ya da kuyruk kapandı — iş nasıl
//! biterse bitsin `Drop` ile) geri verilir. Sığmayan görsel düşer, bildirim görselsiz kalır;
//! uyarı bütçe boşalana dek bir kez loglanır.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::images::{ImageSource, PendingImage, Slot};

/// Bekleyen çözmelerin toplam kodlanmış bayt sınırı (varsayılan).
pub const MAX_PENDING_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// Tek bildirimin bekleyen görsellerinin (simge + içerik) toplam sınırı (varsayılan).
pub const MAX_PENDING_IMAGE_BYTES_PER_NOTIFICATION: usize = 16 * 1024 * 1024;

/// Paylaşılan bayt bütçesi.
#[derive(Debug)]
pub struct DecodeBudget {
    used: AtomicUsize,
    total: usize,
    per_notification: usize,
    /// Bu doluluk döneminde uyarı verildi mi (bütçe sıfıra inince sıfırlanır).
    warned: AtomicBool,
}

/// Ayrılmış baytlar; düşünce bütçeye geri döner.
#[derive(Debug)]
pub struct Reservation {
    budget: Arc<DecodeBudget>,
    bytes: usize,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let before = self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
        if before == self.bytes {
            self.budget.warned.store(false, Ordering::Relaxed);
        }
    }
}

impl DecodeBudget {
    pub fn new(total: usize, per_notification: usize) -> Arc<Self> {
        Arc::new(Self { used: AtomicUsize::new(0), total, per_notification, warned: AtomicBool::new(false) })
    }

    /// Şu an ayrılmış bayt.
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    /// `bytes` sığarsa ayırır (hep ya da hiç).
    fn try_reserve(self: &Arc<Self>, bytes: usize) -> Option<Reservation> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|&next| next <= self.total)
            })
            .ok()
            .map(|_| Reservation { budget: Arc::clone(self), bytes })
    }

    /// Bir bildirimin işlerini bütçeden geçirir: önce içerik görseli, sonra simge. Kodlanmış
    /// baytı olmayan işler (tema ikon adı) bayt ayırmaz. Sığmayanlar atılır; dönen listedeki
    /// her iş kendi ayırmasıyla gelir. İkinci değer atılan iş sayısı.
    pub fn admit(self: &Arc<Self>, mut jobs: Vec<PendingImage>) -> (Vec<(PendingImage, Option<Reservation>)>, usize) {
        jobs.sort_by_key(|j| j.slot != Slot::Image);
        let mut out = Vec::with_capacity(jobs.len());
        let mut dropped = 0;
        let mut local = 0usize;
        for job in jobs {
            let bytes = match &job.source {
                ImageSource::Bytes(b) => b.len(),
                ImageSource::IconName(_) => {
                    out.push((job, None));
                    continue;
                }
            };
            let fits_local = local.checked_add(bytes).is_some_and(|n| n <= self.per_notification);
            match fits_local.then(|| self.try_reserve(bytes)).flatten() {
                Some(r) => {
                    local += bytes;
                    out.push((job, Some(r)));
                }
                None => dropped += 1,
            }
        }
        if dropped > 0 && !self.warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                used = self.used(),
                total = self.total,
                "bekleyen bildirim görselleri bayt bütçesini aştı; sığmayan görseller atılıyor"
            );
        }
        (out, dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(slot: Slot, n: usize) -> PendingImage {
        PendingImage { slot, source: ImageSource::Bytes(vec![0u8; n].into()) }
    }

    #[test]
    fn over_budget_drops_and_release_returns_bytes() {
        let b = DecodeBudget::new(100, 80);
        let (a, d) = b.admit(vec![bytes(Slot::Image, 60)]);
        assert_eq!((a.len(), d, b.used()), (1, 0, 60));
        // Toplam bütçe aşımı: görsel atılır.
        let (c, d) = b.admit(vec![bytes(Slot::Image, 50)]);
        assert_eq!((c.len(), d, b.used()), (0, 1, 60));
        // Çözme bitince (ayırma düşer) bütçe geri verilir ve aynı görsel artık sığar.
        drop(a);
        assert_eq!(b.used(), 0);
        let (e, d) = b.admit(vec![bytes(Slot::Image, 50)]);
        assert_eq!((e.len(), d, b.used()), (1, 0, 50));
        drop(e);
        assert_eq!(b.used(), 0);
    }

    #[test]
    fn per_notification_limit_prefers_content_image() {
        let b = DecodeBudget::new(1000, 80);
        let jobs = vec![bytes(Slot::AppIcon, 40), bytes(Slot::Image, 60)];
        let (a, d) = b.admit(jobs);
        assert_eq!(d, 1, "bildirim başına sınır: simge atılır");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].0.slot, Slot::Image);
        assert_eq!(b.used(), 60);
        // Tek başına sınırı aşan görsel hiç girmez.
        let (c, d) = b.admit(vec![bytes(Slot::Image, 81)]);
        assert_eq!((c.len(), d), (0, 1));
        drop(a);
        assert_eq!(b.used(), 0);
    }

    #[test]
    fn icon_names_do_not_use_budget() {
        let b = DecodeBudget::new(0, 0);
        let (a, d) = b.admit(vec![PendingImage { slot: Slot::AppIcon, source: ImageSource::IconName("firefox".into()) }]);
        assert_eq!((a.len(), d, b.used()), (1, 0, 0));
        assert!(a[0].1.is_none());
    }
}
