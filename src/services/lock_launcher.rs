//! Kilit ekranı tetikleyicisi: `nuxut-lock` sürecini başlatır. Tasarım:
//! `docs/design/lock-trigger.md`.
//!
//! Tek yol: Kilitle düğmesi, `msg lock`, SUPER+L ve boşta kilidi logind'e `Session.Lock()` der;
//! logind oturumun `Lock` sinyalini yayınlar; kabuk ([`crate::services::idle::IdleLogind`])
//! sinyali doğrular ve burada `nuxut-lock`'u başlatır. logind'e ulaşılamazsa ya da sinyali
//! dinleyecek bağlantı hazır değilse kilit doğrudan başlatılır ([`decide`]; kilitlenemeyen kilit
//! düğmesi olmaz). `Unlock` sinyali hiçbir şey yapmaz: kilidi yalnızca kimlik doğrulama açar.
//!
//! İkili sabit yoldan (`current_exe` kardeşi), sabit argv ile, kabuksuz başlatılır; PATH'e
//! güvenilmez. Kendi çocuğumuz çalışırken ikincisi başlatılmaz; başka biri (hypridle) başlattıysa
//! `nuxut-lock`'un tek örnek IPC'si ikinciyi hemen 0 ile çıkarır. Çocuk ayrı bir thread'de
//! beklenir (zombi kalmaz), çıkış kodu loglanır.

use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nuxut::i18n::t;
use serde::Deserialize;

use crate::schema::SettingSchema;

/// Kilit ikilisinin adı (kabuk ikilisinin yanında kurulur).
pub const LOCK_BIN: &str = "nuxut-lock";
/// `nuxut-lock`'un tek örnek adı değişkeni (`lock/src/cli.rs`).
const LOCK_INSTANCE_ENV: &str = "NUXUT_LOCK_INSTANCE";
/// `nuxut-lock` çıkış kodu: compositor kilidi reddetti (başka bir kilitleyici etkin) ya da
/// birincil örnek kilitlemedi.
const EXIT_NOT_LOCKED: i32 = 1;

/// `[lock]`: kabuğun kilit tetikleyicisi.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct LockConfig {
    /// logind'in kilit isteğinde (Kilitle, SUPER+L, `loginctl lock-session`, boşta kilit)
    /// `nuxut-lock`'u başlat. Kendi kilitleyicisini (hypridle `lock_cmd = hyprlock`, swayidle)
    /// kullanan kapatır; logind'e ulaşılamazken Kilitle düğmesi yine `nuxut-lock` açar.
    pub listen_logind: bool,
}

impl Default for LockConfig {
    fn default() -> Self {
        Self { listen_logind: true }
    }
}

impl LockConfig {
    pub fn schema(out: &mut Vec<SettingSchema>) {
        out.push(
            SettingSchema::bool("lock.listen_logind", t!("shell-config-lock-listen-logind-label"), Self::default().listen_logind)
                .describe(t!("shell-config-lock-listen-logind-description")),
        );
    }
}

/// Kilidin nereden istendiği.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockTrigger {
    /// Kendi oturumumuzun logind `Lock` sinyali (doğrulanmış).
    LogindSignal,
    /// `Session.Lock()` logind'e iletildi. `listener_ready`: sinyali dinleyen bağlantı oturumu
    /// çözmüş ve bağlı; değilse sinyal bize ulaşmaz.
    LockSent { listener_ready: bool },
    /// `Session.Lock()` başarısız ya da logind yok.
    LockFailed,
}

/// Bu tetikte `nuxut-lock` başlatılmalı mı? Dinleyici kapalıysa (kullanıcının kendi
/// kilitleyicisi) yalnızca logind'e hiç ulaşılamadığında başlatılır: kilitlenemeyen kilit
/// düğmesi olmasın.
pub fn decide(trigger: LockTrigger, listen_logind: bool) -> bool {
    match trigger {
        LockTrigger::LogindSignal => listen_logind,
        LockTrigger::LockSent { listener_ready } => listen_logind && !listener_ready,
        LockTrigger::LockFailed => true,
    }
}

/// `Session.Lock` iletildikten sonra kendi oturumumuzun `Lock` sinyali bu süre içinde gelmezse
/// kilit doğrudan başlatılır (sinyal başka oturuma gitmiş olabilir; 2026-10-16 "Kilitle
/// çalışmıyor"). Sinyal normalde milisaniyeler içinde gelir.
pub const LOCK_SIGNAL_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Bu tetikten sonra sinyal bekçisi kurulmalı mı (sinyal bekleniyor ve `nuxut-lock` onu açacak).
pub fn needs_signal_watchdog(trigger: LockTrigger, listen_logind: bool) -> bool {
    listen_logind && trigger == LockTrigger::LockSent { listener_ready: true }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    Started,
    /// Kendi başlattığımız kilit hâlâ çalışıyor.
    AlreadyRunning,
}

#[derive(Debug)]
pub enum LaunchError {
    /// Kabuk ikilisinin yanında `nuxut-lock` yok.
    NotFound(Option<PathBuf>),
    /// Test örneği (`NUXUT_SHELL_INSTANCE`) kendi `NUXUT_LOCK_INSTANCE`'ı olmadan kilit açmaz:
    /// kullanıcının kilidine IPC gitmesin.
    TestInstance,
    Spawn(io::Error),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::NotFound(Some(p)) => write!(f, "{} bulunamadı", p.display()),
            LaunchError::NotFound(None) => write!(f, "kabuk ikilisinin dizini bulunamadı"),
            LaunchError::TestInstance => write!(f, "test örneği {LOCK_INSTANCE_ENV} olmadan kilit başlatmaz"),
            LaunchError::Spawn(e) => write!(f, "başlatılamadı: {e}"),
        }
    }
}

/// Test örneği kilidi yalnızca kendi kilit örnek adıyla başlatır.
fn instance_allowed(shell_instance: Option<&str>, lock_instance: Option<&str>) -> bool {
    shell_instance.is_none_or(str::is_empty) || lock_instance.is_some_and(|s| !s.is_empty())
}

/// Kendi başlattığımız `nuxut-lock`'un çıkışı (bekleme thread'inden).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockExit {
    pub pid: u32,
    /// Çıkış kodu; sinyalle öldüyse `None`.
    pub code: Option<i32>,
    /// Öldüren sinyal.
    pub signal: Option<i32>,
}

impl LockExit {
    fn from_status(pid: u32, s: std::process::ExitStatus) -> Self {
        use std::os::unix::process::ExitStatusExt;
        Self { pid, code: s.code(), signal: s.signal() }
    }

    /// Kilit ekranı başarısız oldu (kullanıcıya gösterilir). 0: kilit açıldı ya da başka
    /// örnek zaten kilitliydi.
    pub fn failed(&self) -> bool {
        self.code != Some(0)
    }

    /// Compositor kilidi reddetti (başka bir ext-session-lock kilitleyicisi etkin).
    pub fn refused(&self) -> bool {
        self.code == Some(EXIT_NOT_LOCKED)
    }
}

type ExitSink = Arc<dyn Fn(LockExit) + Send + Sync>;

pub struct LockLauncher {
    program: Option<PathBuf>,
    /// Kendi başlattığımız çocuk çalışıyor (bekleme thread'i çıkınca `false`).
    running: Arc<AtomicBool>,
    /// Çocuk çıkınca (bekleme thread'inde) çağrılır: başarısız kilit sessiz kalmasın.
    on_exit: Option<ExitSink>,
}

impl std::fmt::Debug for LockLauncher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockLauncher").field("program", &self.program).field("running", &self.is_running()).finish()
    }
}

impl LockLauncher {
    pub fn new(program: Option<PathBuf>) -> Self {
        Self { program, running: Arc::new(AtomicBool::new(false)), on_exit: None }
    }

    /// Çocuk çıkışını bildir (thread'den çağrılır; ana döngüye mesaj göndermeli).
    #[must_use]
    pub fn with_exit_sink(mut self, sink: impl Fn(LockExit) + Send + Sync + 'static) -> Self {
        self.on_exit = Some(Arc::new(sink));
        self
    }

    /// Kurulu kilidin beklenen yolu (hata iletisi için).
    pub fn program(&self) -> Option<&std::path::Path> {
        self.program.as_deref()
    }

    /// Kurulu kilit: kabuk ikilisinin yanındaki `nuxut-lock`.
    pub fn installed() -> Self {
        Self::new(crate::self_exe::exe_dir().map(|d| d.join(LOCK_BIN)))
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// `nuxut-lock`'u başlatır (argümansız: kilitle; zaten kilitliyse kendisi 0 ile çıkar).
    pub fn launch(&self) -> Result<Launch, LaunchError> {
        let shell_instance = std::env::var(crate::ipc::INSTANCE_ENV).ok();
        let lock_instance = std::env::var(LOCK_INSTANCE_ENV).ok();
        if !instance_allowed(shell_instance.as_deref(), lock_instance.as_deref()) {
            return Err(LaunchError::TestInstance);
        }
        self.start()
    }

    fn start(&self) -> Result<Launch, LaunchError> {
        let program = match &self.program {
            Some(p) if p.is_file() => p,
            other => return Err(LaunchError::NotFound(other.clone())),
        };
        // Bayrak başlatmadan önce alınır: aynı anda iki tetik iki çocuk açmaz.
        if self.running.swap(true, Ordering::AcqRel) {
            return Ok(Launch::AlreadyRunning);
        }
        let child = Command::new(program).stdin(Stdio::null()).spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                self.running.store(false, Ordering::Release);
                return Err(LaunchError::Spawn(e));
            }
        };
        let pid = child.id();
        let running = self.running.clone();
        let on_exit = self.on_exit.clone();
        let waiter = std::thread::Builder::new().name("nuxut-lock-wait".into()).spawn(move || {
            let exit = match child.wait() {
                Ok(s) => Some(LockExit::from_status(pid, s)),
                Err(e) => {
                    tracing::warn!(pid, "kilit ekranı beklenemedi: {e}");
                    None
                }
            };
            match exit {
                Some(e) if !e.failed() => tracing::info!(pid, "kilit ekranı kapandı"),
                Some(e) if e.refused() => tracing::warn!(
                    pid,
                    "kilit kurulamadı: compositor reddetti (başka bir kilitleyici etkin olabilir; \
                     kendi kilitleyicini kullanıyorsan shell.toml [lock] listen_logind = false)"
                ),
                Some(e) => tracing::error!(pid, code = ?e.code, signal = ?e.signal, "kilit ekranı hatayla çıktı"),
                None => {}
            }
            // Bayrak bildirimden önce iner: bildirimi gören yeniden deneyebilsin.
            running.store(false, Ordering::Release);
            if let (Some(e), Some(sink)) = (exit, on_exit) {
                sink(e);
            }
        });
        if let Err(e) = waiter {
            // Bekleyen yoksa bayrak hiç inmezdi: bir sonraki istek yine başlatabilsin.
            tracing::warn!("kilit bekleme thread'i açılamadı: {e}");
            self.running.store(false, Ordering::Release);
        }
        tracing::info!(pid, "kilit ekranı başlatıldı");
        Ok(Launch::Started)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn decide_matrix() {
        use LockTrigger::*;
        assert!(decide(LogindSignal, true));
        assert!(!decide(LogindSignal, false), "kullanıcının kendi kilitleyicisi");
        assert!(!decide(LockSent { listener_ready: true }, true), "sinyal gelecek");
        assert!(decide(LockSent { listener_ready: false }, true), "sinyal bize ulaşmaz: doğrudan");
        assert!(!decide(LockSent { listener_ready: false }, false));
        assert!(decide(LockFailed, true) && decide(LockFailed, false), "logind yok: her zaman kilitle");
        // Bekçi yalnız sinyalin beklendiği durumda: hiçbir kombinasyonda kilit sessizce düşmez.
        assert!(needs_signal_watchdog(LockSent { listener_ready: true }, true));
        assert!(!needs_signal_watchdog(LockSent { listener_ready: true }, false), "kendi kilitleyicisi");
        assert!(!needs_signal_watchdog(LockSent { listener_ready: false }, true), "zaten doğrudan açıldı");
        assert!(!needs_signal_watchdog(LogindSignal, true) && !needs_signal_watchdog(LockFailed, true));
    }

    #[test]
    fn test_instance_needs_own_lock_instance() {
        assert!(instance_allowed(None, None));
        assert!(instance_allowed(Some(""), None));
        assert!(!instance_allowed(Some("test-x"), None));
        assert!(!instance_allowed(Some("test-x"), Some("")));
        assert!(instance_allowed(Some("test-x"), Some("test-x")));
    }

    #[test]
    fn config_default_listens() {
        assert!(LockConfig::default().listen_logind);
        let c: LockConfig = toml::from_str("listen_logind = false").unwrap();
        assert!(!c.listen_logind);
    }

    fn wait_until(what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < end, "zaman aşımı: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Sahte `nuxut-lock`: çağrıyı sayar, `gate` dosyası silinene dek çalışır.
    fn fake_lock(dir: &std::path::Path) -> PathBuf {
        let path = dir.join("nuxut-lock");
        let script = format!(
            "#!/bin/sh\necho run >> '{0}/calls'\nwhile [ -e '{0}/gate' ]; do sleep 0.02; done\n",
            dir.display()
        );
        crate::test_exec::write_executable(&path, &script);
        path
    }

    fn calls(dir: &std::path::Path) -> usize {
        std::fs::read_to_string(dir.join("calls")).map(|s| s.lines().count()).unwrap_or(0)
    }

    #[test]
    fn launches_once_while_running_then_again() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = LockLauncher::new(Some(fake_lock(dir.path())));
        std::fs::write(dir.path().join("gate"), "").unwrap();
        assert_eq!(launcher.start().unwrap(), Launch::Started);
        assert_eq!(launcher.start().unwrap(), Launch::AlreadyRunning, "çift başlatma yok");
        wait_until("çocuk çalıştı", || calls(dir.path()) == 1);
        assert!(launcher.is_running());
        std::fs::remove_file(dir.path().join("gate")).unwrap();
        wait_until("çocuk beklendi", || !launcher.is_running());
        assert_eq!(launcher.start().unwrap(), Launch::Started, "kilit açıldıktan sonra yeniden");
        wait_until("ikinci çalışma", || calls(dir.path()) == 2);
        wait_until("ikinci beklendi", || !launcher.is_running());
    }

    #[test]
    fn exit_is_reported_with_code_and_signal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nuxut-lock");
        crate::test_exec::write_executable(&path, "#!/bin/sh\nexit \"$(cat \"$0.code\")\"\n");
        let (tx, rx) = std::sync::mpsc::channel();
        let launcher = LockLauncher::new(Some(path.clone())).with_exit_sink(move |e| {
            let _ = tx.send(e);
        });
        for (code, failed, refused) in [(0, false, false), (1, true, true), (101, true, false)] {
            std::fs::write(path.with_extension("code"), code.to_string()).unwrap();
            assert_eq!(launcher.start().unwrap(), Launch::Started);
            let e = rx.recv_timeout(Duration::from_secs(5)).expect("çıkış bildirilmedi");
            assert_eq!((e.code, e.failed(), e.refused()), (Some(code), failed, refused));
            assert!(!launcher.is_running(), "bildirimden önce bayrak iner");
        }
        let crash = LockExit { pid: 1, code: None, signal: Some(11) };
        assert!(crash.failed() && !crash.refused());
    }

    #[test]
    fn missing_binary_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = LockLauncher::new(Some(dir.path().join("nuxut-lock")));
        assert!(matches!(launcher.start(), Err(LaunchError::NotFound(Some(_)))));
        assert!(matches!(LockLauncher::new(None).start(), Err(LaunchError::NotFound(None))));
        assert!(!launcher.is_running(), "başarısız başlatma bayrağı bırakır");
    }
}
