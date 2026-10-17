//! `nuxut-lock`: oturum kilidi. Tasarım: `docs/ARCHITECTURE.md` "Kilit ekranı".
//!
//! Çıkış kodları: 0 kilitlendi ve açıldı / zaten kilitli; 1 compositor reddetti ya da protokol
//! yok ya da çalışan birincil örnek sınırlı sürede kilitlemedi; 2 başlatma hatası.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Instant;

use nuxut::{RunOptions, ipc};
use nuxut_lock::app::{LockApp, Setup, TestHold};
use nuxut_lock::auth::{Authenticator, PamAuthenticator};
use nuxut_lock::cli::{self, Command};
use nuxut_lock::config::LockConfig;
use nuxut_lock::instance;
use nuxut_lock::state::EXIT_NOT_LOCKED;

// Sahte doğrulayıcı yalnızca canlı test içindir; sürüm derlemesine asla girmez.
#[cfg(all(feature = "fake-auth", not(debug_assertions)))]
compile_error!("fake-auth yalnızca debug derlemesinde (canlı test) kullanılabilir");

const EXIT_START_FAILED: i32 = 2;

/// AT olayları ve ağaç bilgisi hiçbir seviyede loglanmaz (kilitliyken AT ayrıntısı günlüğe
/// sızmasın, 2026-10-17): RUST_LOG ne derse desin a11y modülleri kapalı.
const LOG_SILENCED: [&str; 3] = ["nuxut_wayland::a11y=off", "accesskit_unix=off", "accesskit_atspi_common=off"];

fn log_filter() -> tracing_subscriber::EnvFilter {
    let mut filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    for d in LOG_SILENCED {
        filter = filter.add_directive(d.parse().expect("sabit yönerge"));
    }
    filter
}

fn main() {
    tracing_subscriber::fmt().with_env_filter(log_filter()).init();
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    if let Err(e) = nuxut_pam::process::set_non_dumpable() {
        tracing::warn!("PR_SET_DUMPABLE kapatılamadı: {e}");
    }
    // Dil süreç başında bir kez (`framework.toml [locale]`); dökülemezlik ayarından sonra.
    nuxut::i18n::init(&[nuxut_lock::i18n::source()]);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_START_FAILED;
        }
    };
    let name = match cli::app_name(std::env::var(cli::INSTANCE_ENV).ok().as_deref()) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_START_FAILED;
        }
    };
    match command {
        Command::Msg(args) => match ipc::send_args(&name, &args) {
            Ok(reply) => {
                println!("{reply}");
                0
            }
            Err(e) => {
                eprintln!("nuxut-lock çalışmıyor: {e}");
                1
            }
        },
        Command::Lock { ready_fd } => lock(&name, ready_fd),
    }
}

fn lock(name: &str, ready_fd: Option<i32>) -> i32 {
    // Hazır fd'si en başta sahiplenilir (geçersizse kilit yine de yapılır).
    let ready_fd = ready_fd.and_then(|fd| {
        nuxut_pam::process::take_inherited_fd(fd)
            .inspect_err(|e| tracing::warn!("--ready-fd kullanılamıyor: {}", e.kind()))
            .ok()
    });
    // Erken 0 yalnızca birincil gerçekten kilitliyken; bitmekte/beklemedeyse sınırlı bekleyip
    // tek örnek boşalınca kendimiz kilitleriz (instance.rs: fail-open değil).
    let started = Instant::now();
    let listener = loop {
        match ipc::try_single_instance(name) {
            Ok(ipc::Instance::Primary(listener)) => break Some(listener),
            Ok(ipc::Instance::Secondary(client)) => {
                let reply = client.send("status").ok();
                let peer = instance::parse_status(reply.as_deref());
                match instance::decide(peer, started.elapsed()) {
                    instance::Step::AlreadyLocked => {
                        tracing::info!("zaten bir kilit örneği çalışıyor (kilitli)");
                        if let Some(fd) = ready_fd {
                            let _ = nuxut_pam::process::write_ready(fd);
                        }
                        return 0;
                    }
                    instance::Step::Retry(delay) => {
                        tracing::debug!(?peer, "birincil kilitli değil; bekleniyor");
                        std::thread::sleep(delay);
                    }
                    instance::Step::GiveUp => {
                        tracing::error!(?peer, "birincil örnek kilitlemedi ve çıkmadı");
                        return EXIT_NOT_LOCKED;
                    }
                }
            }
            // Tek örnek kurulamasa da kilitlenir: ikinci kilidi compositor zaten reddeder.
            Err(e) => {
                tracing::warn!("tek örnek IPC'si kurulamadı: {e}");
                break None;
            }
        }
    };

    let user = match nuxut_pam::current_user() {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("kullanıcı bilgisi alınamadı: {e}");
            return EXIT_START_FAILED;
        }
    };
    let config = LockConfig::load();
    let mut shell_warn = |w: String| tracing::warn!("shell.toml duvar kağıdı: {w}");
    let wallpaper = nuxut_lock::assets::resolve_wallpaper(
        config.wallpaper.as_ref(),
        nuxut_lock::assets::shell_wallpaper(nuxut_lock::assets::shell_toml().as_deref(), &mut shell_warn),
        nuxut_lock::assets::palette_image().as_deref(),
        &mut |w: String| tracing::warn!("palette.toml [meta] image: {w}"),
    );
    let exit = Arc::new(AtomicI32::new(EXIT_START_FAILED));
    let (auth, system_bus) = authenticator(&user.name, name);
    // Sistem bus'ı yalnızca sahte doğrulayıcı etkinken kapalıdır.
    let fake_mode = !system_bus;
    let setup = Setup {
        auth,
        system_bus,
        display_name: user.display_name().to_owned(),
        user: user.name,
        home: user.home,
        config,
        ready_fd,
        wallpaper,
        exit: Arc::clone(&exit),
        test_hold: test_hold(fake_mode),
    };
    let result = nuxut::run_with(LockApp::new(setup), RunOptions { ipc: listener });
    let code = exit.load(Ordering::SeqCst);
    match result {
        Ok(()) => code,
        Err(e) => {
            // Protokol/bağlantı hatası: kilit olmadıysa 1; olduysa compositor kilitli tutar.
            tracing::error!("çalışma zamanı: {e}");
            if code == 0 { 0 } else { 1 }
        }
    }
}

/// PAM servisi kilitlemeden önce seçilir (`nuxut-lock` yoksa `login`). İkinci değer: sistem
/// bus'ı (logind ipucu, fprintd) kullanılsın mı — sahte test kipinde gerçek oturuma dokunulmaz.
/// Sahte kip `NUXUT_LOCK_INSTANCE` de ister; yoksa gerçek PAM kullanılır.
fn authenticator(user: &str, app_name: &str) -> (Arc<dyn Authenticator>, bool) {
    #[cfg(not(feature = "fake-auth"))]
    let _ = app_name;
    #[cfg(feature = "fake-auth")]
    if let Some(secret) = std::env::var_os("NUXUT_LOCK_FAKE_SECRET") {
        if !cli::fake_auth_allowed(app_name) {
            tracing::error!(
                "NUXUT_LOCK_FAKE_SECRET {} olmadan yok sayıldı",
                cli::INSTANCE_ENV
            );
        } else {
            tracing::warn!("SAHTE doğrulayıcı (yalnızca test derlemesi); logind ve fprintd kapalı");
            let secret = std::os::unix::ffi::OsStrExt::as_bytes(secret.as_os_str()).to_vec();
            return (Arc::new(fake::Fake(secret)), false);
        }
    }
    let service = nuxut_pam::Service::system();
    if service.is_fallback() {
        tracing::warn!("/etc/pam.d/nuxut-lock yok; `login` servisi kullanılıyor");
    }
    (
        Arc::new(PamAuthenticator::new(service, user.to_owned())),
        true,
    )
}

#[cfg(feature = "fake-auth")]
mod fake {
    use nuxut_lock::auth::Authenticator;
    use nuxut_pam::{AuthResult, Outcome, SecretBuf};

    pub struct Fake(pub Vec<u8>);

    impl Authenticator for Fake {
        fn authenticate(&self, secret: &SecretBuf) -> AuthResult {
            // PAM'in hata gecikmesine benzer.
            std::thread::sleep(std::time::Duration::from_millis(300));
            let outcome = if secret.as_bytes() == self.0.as_slice() {
                Outcome::Success
            } else {
                Outcome::Denied
            };
            AuthResult {
                outcome,
                messages: Vec::new(),
            }
        }
    }
}

/// Canlı test kancası (ikincil örnek yolları): yalnızca fake-auth derlemesinde ve sahte
/// doğrulayıcı etkinken (o da `NUXUT_LOCK_INSTANCE` ister). Değer ms, en çok 10 sn.
#[cfg(feature = "fake-auth")]
fn test_hold(fake_mode: bool) -> TestHold {
    if !fake_mode {
        return TestHold::default();
    }
    let ms = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|ms| std::time::Duration::from_millis(ms.min(10_000)))
    };
    TestHold {
        pending: ms("NUXUT_LOCK_TEST_HOLD_PENDING_MS"),
        done: ms("NUXUT_LOCK_TEST_HOLD_DONE_MS"),
    }
}

#[cfg(not(feature = "fake-auth"))]
fn test_hold(_fake_mode: bool) -> TestHold {
    TestHold::default()
}
