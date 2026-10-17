//! `nuxut-greeter`: greetd'nin `default_session`'ı içinde, küçük bir compositor'da çalışır
//! (`dist/greeter/`). Oturum başlatılınca 0 ile çıkar; greetd oturumu ondan sonra açar.
//!
//! Çıkış kodları: 0 oturum başlatıldı ya da sonlandırıldı; 1 çalışma zamanı hatası; 2 başlatma
//! hatası (`GREETD_SOCK` yok, geçersiz argüman).

use nuxut::{RunOptions, ipc};
use nuxut_greeter::app::{GreeterApp, Setup};
use nuxut_greeter::cli::{self, Command};
use nuxut_greeter::{remember, sessions, users};
use nuxut_lock::config::LockConfig;

const EXIT_START_FAILED: i32 = 2;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    // Şifre bu süreçte de tutulur (istem yanıtı): çekirdek dökümü ve ptrace kapalı.
    if let Err(e) = nuxut_pam::process::set_non_dumpable() {
        tracing::warn!("PR_SET_DUMPABLE kapatılamadı: {e}");
    }
    nuxut::i18n::init(&nuxut_greeter::i18n::sources());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_START_FAILED;
        }
    };
    let instance = std::env::var(cli::INSTANCE_ENV).ok();
    let name = match cli::app_name(instance.as_deref()) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}");
            return EXIT_START_FAILED;
        }
    };
    let test_mode = name != cli::APP_NAME;
    match command {
        Command::Msg(args) => match ipc::send_args(&name, &args) {
            Ok(reply) => {
                println!("{reply}");
                0
            }
            Err(e) => {
                eprintln!("nuxut-greeter çalışmıyor: {e}");
                1
            }
        },
        Command::Run => run(&name, test_mode),
    }
}

fn run(name: &str, test_mode: bool) -> i32 {
    let Some(socket) = std::env::var_os("GREETD_SOCK").map(std::path::PathBuf::from) else {
        eprintln!("GREETD_SOCK yok: nuxut-greeter greetd tarafından başlatılmalı");
        return EXIT_START_FAILED;
    };
    let env = |k: &str| std::env::var(k).ok();
    let passwd = cli::test_path(test_mode, env("NUXUT_GREETER_PASSWD")).unwrap_or_else(|| "/etc/passwd".into());
    let session_dirs = match cli::test_path(test_mode, env("NUXUT_GREETER_SESSIONS")) {
        Some(dir) => vec![dir],
        None => sessions::search_dirs(env("XDG_DATA_DIRS").as_deref()),
    };
    let icons_dir = cli::test_path(test_mode, env("NUXUT_GREETER_ICONS")).unwrap_or_else(|| users::ICONS_DIR.into());
    let users = users::load(&passwd, std::path::Path::new("/etc/login.defs"));
    let sessions = sessions::load(&session_dirs);
    if sessions.is_empty() {
        tracing::error!("hiç Wayland oturumu bulunamadı ({} dizin)", session_dirs.len());
    }
    let state_dir = remember::state_dir();
    let config = LockConfig::load();
    let mut shell_warn = |w: String| tracing::warn!("shell.toml duvar kağıdı: {w}");
    let wallpaper = nuxut_lock::assets::resolve_wallpaper(
        config.wallpaper.as_ref(),
        nuxut_lock::assets::shell_wallpaper(nuxut_lock::assets::shell_toml().as_deref(), &mut shell_warn),
        nuxut_lock::assets::palette_image().as_deref(),
        &mut |w: String| tracing::warn!("palette.toml [meta] image: {w}"),
    );
    tracing::info!(users = users.len(), sessions = sessions.len(), test_mode, "giriş ekranı");
    let listener = match ipc::try_single_instance(name) {
        Ok(ipc::Instance::Primary(l)) => Some(l),
        Ok(ipc::Instance::Secondary(_)) => {
            tracing::error!("başka bir nuxut-greeter çalışıyor");
            return EXIT_START_FAILED;
        }
        Err(e) => {
            tracing::warn!("tek örnek IPC'si kurulamadı: {e}");
            None
        }
    };
    let setup = Setup {
        socket,
        users,
        sessions,
        remembered: remember::load(&state_dir),
        state_dir,
        config,
        wallpaper,
        system_bus: !test_mode,
        icons_dir,
        layouts: nuxut_greeter::keyboard::Layouts::from_env(),
        fake_power: test_mode
            .then(|| env("NUXUT_GREETER_POWER"))
            .flatten()
            .map(|v| nuxut_greeter::power::parse_list(&v)),
    };
    match nuxut::run_with(GreeterApp::new(setup), RunOptions { ipc: listener }) {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("çalışma zamanı: {e}");
            1
        }
    }
}
