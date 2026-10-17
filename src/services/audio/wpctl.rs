//! `wpctl` yazım kuyruğu: seviye/sessiz yazımları TEK bir thread'de, SIRAYLA çalışan `wpctl`
//! alt süreçleriyle uygulanır.
//!
//! Neden sıra: her `wpctl` çağrısı PipeWire'a ayrı bir bağlantıdır; aynı anda başlatılan
//! süreçlerin `set_param` istekleri sunucuya başlatılma sırasıyla ulaşmaz (bağlanma, modül
//! yükleme ve zamanlayıcıya bağlı). Hızlı kaydırmada eski bir seviye en son yazılıp kalabilirdi.
//! Kuyrukta aynı düğümün aynı türden (seviye / sessiz) bekleyen işi yenisiyle değiştirilir:
//! patlama tek yazıma iner, son değer her zaman en son uygulanır.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Tek bir `wpctl` sürecinin ilk denemedeki en uzun süresi; aşılırsa öldürülür.
const RUN_TIMEOUT: Duration = Duration::from_secs(3);
/// Zaman aşımında aynı yazım (daha yenisi kuyruğa gelmediyse) bu kadar denemeye dek, her
/// seferinde iki kat süreyle yeniden denenir (3 → 6 → 12 sn). Yük altında `wpctl`'in açılışı
/// 3 sn'yi aşabiliyor; eskiden yazım sessizce kayboluyordu (seviye/sessiz hiç uygulanmıyordu,
/// audio_service testleri yük altında böyle düşüyordu, 2026-10-16). Yazım eş güçlüdür: tekrarı
/// güvenli.
const ATTEMPTS: u32 = 3;
/// Kuyrukta en çok iş (düğüm × tür başına bir tane olduğundan normalde birkaç tane).
const MAX_QUEUE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Volume,
    Mute,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Job {
    pub node: u32,
    pub kind: Kind,
    pub args: Vec<String>,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    closed: bool,
}

impl Queue {
    /// Aynı (düğüm, tür) bekleyen iş yerinde güncellenir; yoksa sona eklenir.
    fn push(&mut self, job: Job) {
        if let Some(slot) = self.jobs.iter_mut().find(|j| j.node == job.node && j.kind == job.kind) {
            *slot = job;
            return;
        }
        if self.jobs.len() >= MAX_QUEUE {
            self.jobs.pop_front();
        }
        self.jobs.push_back(job);
    }
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    /// Süreç başlatılamadı (wpctl kalktı vb.): çağıran süreç içi yazıma döner.
    broken: AtomicBool,
}

/// Sıralı `wpctl` yazıcısı. Düşürülünce kuyruktakiler bitirilip thread çıkar (beklenmez).
pub(crate) struct Writer {
    shared: Arc<Shared>,
}

impl Writer {
    /// `wpctl` `$PATH`'te yoksa ya da thread başlatılamazsa `None`.
    pub fn spawn(remote: Option<String>) -> Option<Self> {
        let path: Vec<PathBuf> =
            std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        crate::services::launcher::catalog::find_executable("wpctl", &path)?;
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            ready: Condvar::new(),
            broken: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("nuxut-wpctl".into())
            .spawn(move || run(&worker, remote.as_deref()))
            .ok()?;
        Some(Self { shared })
    }

    /// Kullanılabilir mi (son süreç başlatılabildi mi)?
    pub fn usable(&self) -> bool {
        !self.shared.broken.load(Ordering::Relaxed)
    }

    pub fn push(&self, job: Job) {
        self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner).push(job);
        self.shared.ready.notify_one();
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner).closed = true;
        self.shared.ready.notify_one();
    }
}

fn run(shared: &Shared, remote: Option<&str>) {
    loop {
        let job = {
            let mut q = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if let Some(job) = q.jobs.pop_front() {
                    break job;
                }
                if q.closed {
                    return;
                }
                q = shared.ready.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let mut attempt = 0;
        loop {
            let Some(child) = spawn(&job.args, remote) else {
                shared.broken.store(true, Ordering::Relaxed);
                break;
            };
            if wait(child, Instant::now() + attempt_timeout(attempt)) == Outcome::Exited {
                break;
            }
            attempt += 1;
            if attempt >= ATTEMPTS {
                tracing::warn!(node = job.node, kind = ?job.kind, "ses: wpctl {ATTEMPTS} denemede de yanıt vermedi, yazım bırakıldı");
                break;
            }
            if superseded(shared, &job) {
                break; // daha yeni değer kuyrukta: o uygulanacak
            }
            tracing::debug!(node = job.node, attempt, "ses: wpctl zaman aşımı, yeniden deneniyor");
        }
    }
}

/// `attempt`. denemenin süresi (0'dan; her denemede iki kat).
fn attempt_timeout(attempt: u32) -> Duration {
    RUN_TIMEOUT * 2u32.saturating_pow(attempt)
}

/// Aynı (düğüm, tür) için daha yeni bir iş kuyrukta bekliyor mu.
fn superseded(shared: &Shared, job: &Job) -> bool {
    let q = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
    q.jobs.iter().any(|j| j.node == job.node && j.kind == job.kind)
}

/// Kabuk yok, çıktı yok. Ayrı bir PipeWire örneğine bağlıysak (testler, özel oturum) wpctl de
/// oraya gider; yoksa başka sunucudaki aynı id'li düğümü değiştirebilir.
fn spawn(args: &[String], remote: Option<&str>) -> Option<Child> {
    let mut cmd = Command::new("wpctl");
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(r) = remote {
        cmd.env("PIPEWIRE_REMOTE", r);
    }
    match cmd.spawn() {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::debug!("ses: wpctl başlatılamadı: {e}");
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Süreç kendisi çıktı (çıkış kodu ne olursa olsun: hedef yoksa yeniden denemek boşuna).
    Exited,
    /// Süre doldu ya da beklenemedi; öldürüldü.
    Killed,
}

/// Süreci süre sınırıyla bekler; aşılırsa öldürür (sıradaki iş takılmasın).
fn wait(mut child: Child, deadline: Instant) -> Outcome {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    tracing::debug!(%status, "ses: wpctl başarısız çıktı");
                }
                return Outcome::Exited;
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::Killed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(node: u32, kind: Kind, v: &str) -> Job {
        Job { node, kind, args: vec![v.into()] }
    }

    #[test]
    fn burst_coalesces_to_latest_in_place() {
        let mut q = Queue::default();
        q.push(job(1, Kind::Volume, "0.1"));
        q.push(job(1, Kind::Mute, "1"));
        q.push(job(2, Kind::Volume, "0.5"));
        q.push(job(1, Kind::Volume, "0.2"));
        q.push(job(1, Kind::Volume, "0.3"));
        let got: Vec<_> = q.jobs.iter().cloned().collect();
        assert_eq!(got, vec![job(1, Kind::Volume, "0.3"), job(1, Kind::Mute, "1"), job(2, Kind::Volume, "0.5")]);
    }

    #[test]
    fn wait_reports_exit_or_kill() {
        let quick = Command::new("true").spawn().unwrap();
        assert_eq!(wait(quick, Instant::now() + Duration::from_secs(30)), Outcome::Exited);
        let failing = Command::new("false").spawn().unwrap();
        assert_eq!(wait(failing, Instant::now() + Duration::from_secs(30)), Outcome::Exited, "çıkış kodu yeniden deneme sebebi değil");
        let hung = Command::new("sleep").arg("30").spawn().unwrap();
        assert_eq!(wait(hung, Instant::now() + Duration::from_millis(50)), Outcome::Killed);
    }

    #[test]
    fn retries_back_off_and_yield_to_newer_value() {
        assert_eq!(attempt_timeout(0), RUN_TIMEOUT);
        assert_eq!(attempt_timeout(2), RUN_TIMEOUT * 4);
        let shared = Shared { queue: Mutex::new(Queue::default()), ready: Condvar::new(), broken: AtomicBool::new(false) };
        let old = job(1, Kind::Volume, "0.2");
        assert!(!superseded(&shared, &old));
        shared.queue.lock().unwrap().push(job(1, Kind::Mute, "1"));
        assert!(!superseded(&shared, &old), "başka tür");
        shared.queue.lock().unwrap().push(job(1, Kind::Volume, "0.3"));
        assert!(superseded(&shared, &old));
    }

    #[test]
    fn queue_is_bounded() {
        let mut q = Queue::default();
        for n in 0..(MAX_QUEUE as u32 + 10) {
            q.push(job(n, Kind::Volume, "1"));
        }
        assert_eq!(q.jobs.len(), MAX_QUEUE);
        assert_eq!(q.jobs.back().map(|j| j.node), Some(MAX_QUEUE as u32 + 9));
    }
}
