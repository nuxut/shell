//! Ölçüm geçmişi: CPU, bellek ve GPU kullanımının son [`HISTORY_LEN`] örneği (%).
//!
//! Servis thread'inde tutulur (panel kapalıyken de birikir; açılınca son dakikalar hemen
//! görünür) ve her ölçümle [`Snapshot::history`](super::Snapshot) içinde yayınlanır. UI'dan
//! bağımsızdır: sabit kapasiteli, bir kez ayrılan tampon; sonlu olmayan örnek NaN (boşluk)
//! olarak saklanır. `seq` itilen ölçüm sayısıdır: görünüm son gördüğü `seq`'in bir fazlasını
//! alırsa yalnızca en yeni örneği iter (kayma animasyonu), aksi hâlde tüm geçmişi yükler.

use std::collections::VecDeque;

/// Saklanan örnek sayısı (varsayılan 2 sn aralıkla iki dakika).
pub const HISTORY_LEN: usize = 60;

/// Sabit kapasiteli örnek kuyruğu (eskiden yeniye).
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    samples: VecDeque<f32>,
}

impl Default for Series {
    fn default() -> Self {
        Self { samples: VecDeque::with_capacity(HISTORY_LEN) }
    }
}

impl Series {
    /// Örneği ekler; dolunca en eski düşer. Sonlu olmayan değer NaN (boşluk).
    pub fn push(&mut self, value: f32) {
        if self.samples.len() == HISTORY_LEN {
            self.samples.pop_front();
        }
        self.samples.push_back(if value.is_finite() { value } else { f32::NAN });
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// En yeni örnek.
    pub fn latest(&self) -> Option<f32> {
        self.samples.back().copied()
    }

    /// Eskiden yeniye.
    pub fn iter(&self) -> impl Iterator<Item = f32> + '_ {
        self.samples.iter().copied()
    }
}

/// Hangi ölçümün geçmişi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Cpu,
    Memory,
    Gpu,
}

/// CPU, bellek ve (ilk) GPU kullanım geçmişi.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct History {
    cpu: Series,
    memory: Series,
    gpu: Series,
    /// İtilen ölçüm sayısı (yalnızca artar).
    seq: u64,
}

impl History {
    /// Bir ölçümün değerleri (%); bilinmeyen ölçüm `None` → boşluk.
    pub fn push(&mut self, cpu: Option<f32>, memory: Option<f32>, gpu: Option<f32>) {
        let v = |x: Option<f32>| x.unwrap_or(f32::NAN);
        self.cpu.push(v(cpu));
        self.memory.push(v(memory));
        self.gpu.push(v(gpu));
        self.seq = self.seq.wrapping_add(1);
    }

    pub fn series(&self, metric: Metric) -> &Series {
        match metric {
            Metric::Cpu => &self.cpu,
            Metric::Memory => &self.memory,
            Metric::Gpu => &self.gpu,
        }
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_is_bounded_and_ordered() {
        let mut s = Series::default();
        assert!(s.is_empty() && s.latest().is_none());
        for i in 0..(HISTORY_LEN * 3) {
            s.push(i as f32);
        }
        assert_eq!(s.len(), HISTORY_LEN, "kapasite aşılmaz");
        let v: Vec<f32> = s.iter().collect();
        assert_eq!(v.first().copied(), Some((HISTORY_LEN * 2) as f32), "en eskiler düştü");
        assert_eq!(s.latest(), Some((HISTORY_LEN * 3 - 1) as f32));
        assert!(v.windows(2).all(|w| w[0] < w[1]), "eskiden yeniye");
    }

    #[test]
    fn non_finite_is_gap() {
        let mut s = Series::default();
        s.push(f32::INFINITY);
        s.push(f32::NAN);
        s.push(5.0);
        let v: Vec<f32> = s.iter().collect();
        assert!(v[0].is_nan() && v[1].is_nan());
        assert_eq!(v[2], 5.0);
    }

    #[test]
    fn history_tracks_metrics_and_seq() {
        let mut h = History::default();
        assert_eq!(h.seq(), 0);
        h.push(Some(10.0), Some(50.0), None);
        h.push(Some(20.0), None, Some(70.0));
        assert_eq!(h.seq(), 2);
        assert_eq!(h.series(Metric::Cpu).iter().collect::<Vec<_>>(), [10.0, 20.0]);
        assert_eq!(h.series(Metric::Memory).latest().map(f32::is_nan), Some(true), "bilinmeyen = boşluk");
        assert_eq!(h.series(Metric::Gpu).latest(), Some(70.0));
        for _ in 0..1000 {
            h.push(Some(1.0), Some(1.0), Some(1.0));
        }
        assert!([Metric::Cpu, Metric::Memory, Metric::Gpu].iter().all(|&m| h.series(m).len() == HISTORY_LEN));
        assert_eq!(h.seq(), 1002);
    }
}
