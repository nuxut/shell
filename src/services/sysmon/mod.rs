//! Sistem izleyicisi servisi: CPU, bellek, GPU, sıcaklık, ağ hızı, disk.
//!
//! Örnekleme kendi thread'inde (`nuxut-sysmon`), `[sysmon] interval` aralığıyla; her ölçüm
//! [`SysmonEvent::Sample`] olarak yayınlanır (aynısı iki kez gönderilmez). Her aralık ölçümü
//! CPU/bellek/GPU kullanımını sınırlı [`History`]'ye de iter (panel kapalıyken de; ayrıntı
//! kipine geçişteki ara ölçüm itmez, aralık düzenli kalır). Pahalı ölçümler
//! yalnızca **ayrıntı kipinde** ([`SysmonService::set_detail`], panel açıkken): süreç listesi,
//! tüm sensörler, bağlama noktaları, yük ortalaması, frekans. Okumalar `sampler.rs`'te
//! (açık tanıtıcı + `pread`, sınırlı tampon), ayrıştırıcılar `parse.rs`'te (saf, düşmanca
//! girdi testli), görünüm metinleri `format.rs`'te.

pub mod format;
pub mod history;
pub mod parse;
pub mod sampler;

use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Deserialize;

use nuxut::i18n::t;
use crate::schema::SettingSchema;

pub use history::{History, Metric};
pub use parse::Memory;
use sampler::{Roots, Sampler};

/// Yüksek kullanım (%) ve sıcaklık (°C) uyarı eşikleri (bar değeri, panel çubukları ve grafikleri).
pub const ALERT_PERCENT: f32 = 90.0;
pub const ALERT_CELSIUS: f32 = 85.0;

/// Örnekleme aralığı sınırları (sn).
pub const MIN_INTERVAL: f32 = 0.5;
pub const MAX_INTERVAL: f32 = 10.0;
/// Panelde gösterilecek en çok süreç.
pub const MAX_PROCESSES: u32 = 15;
/// Ayrıntı kipi açılınca süreç CPU payları için ikinci ölçüme kadar bekleme.
const DETAIL_WARMUP: Duration = Duration::from_millis(500);
/// Servis açılınca ilk (taban) ölçümden sonra ilk yayına kadar bekleme.
const FIRST_SAMPLE: Duration = Duration::from_millis(1000);

/// `[sysmon]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct SysmonConfig {
    /// Örnekleme aralığı (sn, [`MIN_INTERVAL`]..=[`MAX_INTERVAL`]).
    pub interval: f32,
    /// Panelde en çok CPU kullanan süreç sayısı (0 = liste yok; süreçler hiç okunmaz).
    pub processes: u32,
}

impl Default for SysmonConfig {
    fn default() -> Self {
        Self { interval: 2.0, processes: 5 }
    }
}

impl SysmonConfig {
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(
            SettingSchema::float("sysmon.interval", t!("shell-sysmon-setting-interval-label"), d.interval, MIN_INTERVAL, MAX_INTERVAL)
                .step(0.5)
                .unit("s"),
        );
        out.push(SettingSchema::int("sysmon.processes", t!("shell-sysmon-setting-processes-label"), d.processes.into(), 0, MAX_PROCESSES.into()));
    }

    pub fn validate(&mut self, w: &mut Vec<String>) {
        let d = Self::default();
        if !self.interval.is_finite() || !(MIN_INTERVAL..=MAX_INTERVAL).contains(&self.interval) {
            let fixed = if self.interval.is_finite() { self.interval.clamp(MIN_INTERVAL, MAX_INTERVAL) } else { d.interval };
            w.push(format!("sysmon.interval geçersiz, {fixed} kullanılıyor ({MIN_INTERVAL}..={MAX_INTERVAL})"));
            self.interval = fixed;
        }
        if self.processes > MAX_PROCESSES {
            w.push(format!("sysmon.processes = {} aralık dışı, {MAX_PROCESSES} kullanılıyor (0..={MAX_PROCESSES})", self.processes));
            self.processes = MAX_PROCESSES;
        }
    }

    pub fn interval(&self) -> Duration {
        let s = if self.interval.is_finite() { self.interval.clamp(MIN_INTERVAL, MAX_INTERVAL) } else { 2.0 };
        Duration::from_secs_f32(s)
    }
}

/// Kullanılan / toplam (bayt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub used: u64,
    pub total: u64,
}

impl Usage {
    /// 0..=100.
    pub fn percent(&self) -> f32 {
        if self.total == 0 { 0.0 } else { (self.used.min(self.total) as f64 / self.total as f64 * 100.0) as f32 }
    }
}

/// Saniye başına bayt: ağda (alınan, gönderilen), diskte (okunan, yazılan).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rates {
    pub down: f64,
    pub up: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    /// Sürücü adı (`amdgpu`).
    pub driver: String,
    /// Aygıt güç tasarrufunda (okunmadı; uyandırılmaz).
    pub suspended: bool,
    /// Kullanım (%).
    pub busy: Option<f32>,
    pub vram: Option<Usage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Temperature {
    /// "k10temp Tctl", "amdgpu edge", "acpitz".
    pub label: String,
    pub celsius: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    /// Tüm makineye göre CPU payı (%); ilk ölçümde bilinmez.
    pub cpu: Option<f32>,
    /// Yerleşik bellek (bayt).
    pub rss: u64,
}

/// Bir ölçüm. Bilinmeyen/olmayan ölçüm `None` ya da boş liste (UI gizler).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    /// Toplam CPU kullanımı (%).
    pub cpu: Option<f32>,
    /// Çekirdek başına (%).
    pub cores: Vec<f32>,
    pub memory: Option<Memory>,
    pub gpus: Vec<Gpu>,
    /// Seçilen CPU sensörü (°C).
    pub cpu_temp: Option<f32>,
    pub net: Option<Rates>,
    pub disk_io: Option<Rates>,
    /// Kök dosya sistemi.
    pub root: Option<Usage>,
    /// Ayrıntı kipinde alındı (aşağıdakiler yalnızca o zaman dolu).
    pub detail: bool,
    pub load: Option<[f32; 3]>,
    pub freq_mhz: Option<u32>,
    pub temps: Vec<Temperature>,
    pub mounts: Vec<(String, Usage)>,
    pub processes: Vec<Process>,
    /// Kullanım geçmişi (servis thread'inde birikir).
    pub history: History,
}

impl Snapshot {
    /// Bellek kullanımı (%).
    pub fn memory_percent(&self) -> Option<f32> {
        self.memory.map(|m| Usage { used: m.used, total: m.total }.percent())
    }

    /// İlk GPU'nun kullanımı (%); uykudaysa ya da okunamıyorsa `None`.
    pub fn gpu_busy(&self) -> Option<f32> {
        self.gpus.first().filter(|g| !g.suspended).and_then(|g| g.busy)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SysmonEvent {
    Sample(Arc<Snapshot>),
}

#[derive(Debug)]
enum Command {
    Detail(bool),
    Config(SysmonConfig),
}

/// Servis tutamağı; düşürülünce thread durur ve join edilir.
#[derive(Debug)]
pub struct SysmonService {
    tx: Option<mpsc::Sender<Command>>,
    thread: Option<JoinHandle<()>>,
    detail: bool,
    config: SysmonConfig,
}

impl SysmonService {
    pub fn spawn<F>(config: SysmonConfig, sink: F) -> std::io::Result<Self>
    where
        F: Fn(SysmonEvent) + Send + 'static,
    {
        Self::spawn_with(Roots::system(), config, sink)
    }

    fn spawn_with<F>(roots: Roots, config: SysmonConfig, sink: F) -> std::io::Result<Self>
    where
        F: Fn(SysmonEvent) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let c = config.clone();
        let thread = std::thread::Builder::new().name("nuxut-sysmon".into()).spawn(move || run(roots, c, &rx, &sink))?;
        Ok(Self { tx: Some(tx), thread: Some(thread), detail: false, config })
    }

    /// Ayrıntı kipi (panel açık). Açılınca hemen ölçer; değişmediyse bir şey göndermez.
    pub fn set_detail(&mut self, on: bool) {
        if self.detail != on {
            self.detail = on;
            self.send(Command::Detail(on));
        }
    }

    pub fn detail(&self) -> bool {
        self.detail
    }

    pub fn set_config(&mut self, config: &SysmonConfig) {
        if self.config != *config {
            self.config = config.clone();
            self.send(Command::Config(config.clone()));
        }
    }

    fn send(&self, c: Command) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(c);
        }
    }
}

impl Drop for SysmonService {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::warn!("sistem izleyici thread'i panikle bitti");
        }
    }
}

fn run(roots: Roots, mut config: SysmonConfig, rx: &mpsc::Receiver<Command>, sink: &dyn Fn(SysmonEvent)) {
    let mut sampler = Sampler::new(roots);
    let mut detail = false;
    let mut last: Option<Arc<Snapshot>> = None;
    let mut history = History::default();
    // Taban ölçüm (CPU ve hızlar fark ister); ilk yayın kısa süre sonra.
    sampler.sample(false, 0);
    let mut next = Instant::now() + FIRST_SAMPLE.min(config.interval());
    loop {
        let wait = next.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Command::Detail(on)) => {
                if on == detail {
                    continue;
                }
                detail = on;
                if on {
                    // Süreç tıkları için taban; ölçüm kısa süre sonra (CPU payları dolu gelir).
                    sampler.reset_processes();
                    let mut base = sampler.sample(true, config.processes as usize);
                    base.history = history.clone();
                    publish(&mut last, Arc::new(base), sink);
                    next = Instant::now() + DETAIL_WARMUP;
                }
            }
            Ok(Command::Config(c)) => {
                config = c;
                next = next.min(Instant::now() + config.interval());
            }
            Err(RecvTimeoutError::Timeout) => {
                let mut s = sampler.sample(detail, config.processes as usize);
                history.push(s.cpu, s.memory_percent(), s.gpu_busy());
                s.history = history.clone();
                publish(&mut last, Arc::new(s), sink);
                next = Instant::now() + config.interval();
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn publish(last: &mut Option<Arc<Snapshot>>, s: Arc<Snapshot>, sink: &dyn Fn(SysmonEvent)) {
    if last.as_deref() == Some(&*s) {
        return;
    }
    *last = Some(s.clone());
    sink(SysmonEvent::Sample(s));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_is_bounded() {
        let mut w = Vec::new();
        let mut c = SysmonConfig { interval: 0.01, processes: 99 };
        c.validate(&mut w);
        assert_eq!((c.interval, c.processes), (MIN_INTERVAL, MAX_PROCESSES));
        assert_eq!(w.len(), 2);
        let mut c = SysmonConfig { interval: f32::NAN, processes: 0 };
        c.validate(&mut w);
        assert_eq!(c.interval, 2.0);
        let mut ok = SysmonConfig::default();
        let before = w.len();
        ok.validate(&mut w);
        assert_eq!(w.len(), before);
        assert_eq!(SysmonConfig { interval: 1e9, processes: 0 }.interval(), Duration::from_secs(10));
    }

    #[test]
    fn usage_percent() {
        assert_eq!(Usage { used: 1, total: 4 }.percent(), 25.0);
        assert_eq!(Usage { used: 9, total: 4 }.percent(), 100.0);
        assert_eq!(Usage::default().percent(), 0.0);
    }

    #[test]
    fn service_publishes_and_stops_quickly() {
        let dir = tempfile::tempdir().unwrap();
        let proc = dir.path().join("proc");
        std::fs::create_dir_all(&proc).unwrap();
        std::fs::write(proc.join("meminfo"), "MemTotal: 100 kB\nMemAvailable: 50 kB\n").unwrap();
        let roots = Roots { proc, sys: dir.path().join("sys") };
        let (tx, rx) = mpsc::channel();
        let cfg = SysmonConfig { interval: MIN_INTERVAL, processes: 3 };
        let mut svc = SysmonService::spawn_with(roots, cfg, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let SysmonEvent::Sample(s) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(s.memory.unwrap().total, 100 * 1024);
        assert!(!s.detail);
        assert_eq!(s.history.seq(), 1);
        assert_eq!(s.history.series(Metric::Memory).latest(), Some(50.0));
        assert_eq!(s.history.series(Metric::Gpu).latest().map(f32::is_nan), Some(true), "GPU yok: boşluk");
        svc.set_detail(true);
        let SysmonEvent::Sample(s) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(s.detail, "ayrıntı açılınca hemen ölçer");
        assert_eq!(s.history.seq(), 1, "ara ölçüm geçmişe itmez");
        // Sahte ağaçta değer değişmese de aralık ölçümü geçmişi ilerletir (grafik kayar).
        let SysmonEvent::Sample(s) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(s.history.seq(), 2);
        assert_eq!(s.history.series(Metric::Memory).len(), 2);
        let t = Instant::now();
        drop(svc);
        assert!(t.elapsed() < Duration::from_secs(1));
    }
}
