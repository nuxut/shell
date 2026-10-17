//! Gerçek görsel işçisiyle (ADR-0007) testler için ortak yardımcılar.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use nuxut_shell::services::image_sandbox::{Sandbox, SandboxConfig};

// `#[path]` ile yüklenen dosya mod-rs sayılır: yol bu dosyanın dizinine (tests/support) göredir.
#[path = "repo.rs"]
mod repo;

/// Gerçek işçi ikilisi. İşçi framework'teki ayrı paket (`nuxut-image-worker`; shell [[bin]]
/// tanımlamaz): `NUXUT_IMAGE_WORKER` verilmişse o, yoksa test başına bir kez aynı `../framework`
/// kaynağından `CARGO_TARGET_TMPDIR` altındaki ayrı hedef dizinine derlenir (el sıkışma aynı
/// framework sürümünü ister; ayrı hedef dizini dıştaki `cargo test`in kilidiyle çakışmaz).
fn worker_binary() -> PathBuf {
    static WORKER: OnceLock<PathBuf> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            if let Some(p) = std::env::var_os("NUXUT_IMAGE_WORKER").filter(|p| !p.is_empty()) {
                return p.into();
            }
            // Framework yolu çalışma zamanında çözülür (`repo`); `CARGO_TARGET_TMPDIR` yalnızca
            // derleme anında verilir ama hedef dizini ikiliyle birlikte taşınır/silinir.
            let manifest = repo::repo_path("../framework/Cargo.toml");
            let target = concat!(env!("CARGO_TARGET_TMPDIR"), "/image-worker");
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let status = std::process::Command::new(cargo)
                .args(["build", "--quiet", "-p", "nuxut-image-worker", "--target-dir", target])
                .arg("--manifest-path")
                .arg(&manifest)
                .status()
                .expect("cargo çalıştırılamadı");
            assert!(status.success(), "nuxut-image-worker derlenemedi ({status})");
            PathBuf::from(target).join("debug/nuxut-image-worker")
        })
        .clone()
}

/// Gerçek işçi ikilisiyle Sandbox (arama sırasına bırakılmaz). Kilit aynı test ikilisindeki
/// gerçek işçili testleri sıralar: [`worker_children`] ve [`assert_no_workers_left`] sürecin
/// tüm çocuklarını sayar, paralel testin işçileri sayımı bozardı. Kilit test sonuna dek tutulur.
///
/// Süre sınırları canlılık sınırına genişletilir ([`patient`]): bu testler kapıların SONUCA
/// dayalı davranışını (gönderen başına susturma, negatif önbellek, yedek) doğrular; zaman aşımı
/// ve susturma süresinin kendisi framework'ün birim testlerinde sınanır.
pub fn real_sandbox() -> (MutexGuard<'static, ()>, Arc<Sandbox>) {
    static SERIAL: Mutex<()> = Mutex::new(());
    let serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    (serial, Arc::new(Sandbox::new(patient(SandboxConfig { worker: Some(worker_binary()), ..SandboxConfig::default() }))))
}

/// Derleme yükü altında (debug işçi, düşük öncelik, aynı makinede eşzamanlı derlemeler) üretim varsayılanları
/// (çözme 1.5 sn, başlatma 3 sn) GEÇERLİ görseli zaman aşımına düşürüyordu: zaman aşımı negatif
/// önbelleğe girip aynı görseli ve gönderenin anahtar sayacını zehirliyor, test 20 sn sonra
/// "görsel kayboldu" diye düşüyordu. Çözme/başlatma sınırı canlılık sınırıdır (sonuç gelince
/// döner); susturma süresi testin en uzun yolundan uzun tutulur ki susturma sonucu süre dolması
/// yüzünden kaybolmasın. Kapıların kendisi (eşik, kova, negatif önbellek) değişmez.
pub fn patient(cfg: SandboxConfig) -> SandboxConfig {
    SandboxConfig {
        timeout: Duration::from_secs(60),
        spawn_timeout: Duration::from_secs(60),
        key_mute: Duration::from_secs(3600),
        ..cfg
    }
}

/// Bu test sürecinin canlı (zombi dahil) `nuxut-image-worker` çocukları.
pub fn worker_children() -> usize {
    let me = std::process::id();
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{me}/task")) else { return 0 };
    tasks
        .filter_map(Result::ok)
        .filter_map(|t| std::fs::read_to_string(t.path().join("children")).ok())
        .flat_map(|c| c.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
        .filter(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim_end().starts_with("nuxut-image-wor")))
        .count()
}

/// İşçi çocuğu kalmayana dek bekler (öldürme + `wait` eşzamanlı değil); kalırsa panik. Sınır
/// canlılık sınırıdır (yük altında `wait` gecikebilir); çocuk kalmayınca hemen döner.
pub fn assert_no_workers_left() {
    let deadline = Instant::now() + Duration::from_secs(60);
    while worker_children() > 0 {
        assert!(Instant::now() < deadline, "işçi süreci kaldı: {}", worker_children());
        std::thread::sleep(Duration::from_millis(20));
    }
}
