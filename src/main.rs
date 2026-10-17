//! nuxut-shell giriş noktası. Log düzeyi `RUST_LOG` ile.
//!
//! `nuxut-shell` shell'i çalıştırır; `nuxut-shell msg <komut…>` çalışan shell'e komut gönderir
//! (Hyprland bind'leri için, ör. `bind = SUPER, N, exec, nuxut-shell msg panel toggle right`).
//! `nuxut-shell schema` ayarlar uygulamasının okuduğu şemayı yazdırır (shell gerekmez).

use std::process::ExitCode;

use nuxut_shell::app::Shell;
use nuxut_shell::config::{LoadState, ShellConfig};
use nuxut_shell::config_migrate::WritePolicy;
use nuxut_shell::ipc::{self, Cli};

fn main() -> ExitCode {
    let cli = match ipc::parse_cli(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("nuxut-shell: {e}");
            return ExitCode::from(2);
        }
    };
    match cli {
        Cli::Help => {
            println!("{}\n\n{}", ipc::USAGE, ipc::HELP);
            ExitCode::SUCCESS
        }
        Cli::Version => {
            println!("nuxut-shell {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Cli::Msg(args) => send(&args),
        Cli::Schema => {
            // Başlıklar katalogdan (ADR-0010): Ayarlar şemayı kendi diliyle okur.
            nuxut::i18n::init(&[nuxut::i18n::embed!("shell")]);
            match nuxut_shell::schema::generate() {
                Ok(text) => {
                    print!("{text}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("nuxut-shell: şema yazılamadı: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Cli::DisplayGuard(args) => {
            use nuxut_shell::services::displays::guard::{GuardArgs, run_guard};
            match GuardArgs::parse(&args).and_then(|a| run_guard(&a)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("nuxut-shell display-guard: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Cli::LookApply => look_apply(),
        Cli::LookRevert => look_revert(),
        Cli::Doctor { json } => ExitCode::from(nuxut_shell::doctor::run(json)),
        Cli::HyprTakeover => hypr_oneshot("takeover", |env| {
            use nuxut_shell::services::hypr_config::takeover::{HyprlandVerifier, takeover};
            let runtime_dir = env.apply.state_home.clone().unwrap_or_else(std::env::temp_dir).join("nuxut/hypr-verify");
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            let verifier = HyprlandVerifier { runtime_dir, home: env.apply.home.clone(), config_home: env.apply.config_home.clone() };
            let report = takeover(env, &verifier, &exe, None);
            // Çalışan kabuk özet bildirimini ("Geri al" düğmeli) rapordan kurar; kabuk yoksa yalnız JSON.
            if report.result == "taken_over" && nuxut_shell::services::hypr_config::takeover::save_report(env, &report).is_ok() {
                let notify = ipc::app_name().ok().and_then(|app| nuxut::ipc::Client::new(&app).ok());
                if let Some(c) = notify {
                    let _ = c.with_timeout(ipc::REPLY_TIMEOUT).send_args(&["hypr".to_owned(), "takeover-done".to_owned()]);
                }
            }
            report
        }),
        Cli::HyprRestoreBackup => hypr_oneshot("restore-backup", |env| nuxut_shell::services::hypr_config::takeover::restore_backup(env, None)),
        Cli::SystemTheme(args) => nuxut_shell::services::system_theme::run_cli(&args),
        Cli::LoginScreen(args) => nuxut_shell::services::login_screen::run_cli(&args),
        Cli::Packages(args) => nuxut_shell::services::root_packages::run_cli(&args),
        Cli::Run => run(),
    }
}

/// `look apply`: birleşik görünümün tek turu (çalışan shell'e dokunmaz). JSON stdout'a; tam
/// başarıda 0, kısmi/hatada 1, okunamazsa 2.
fn look_apply() -> ExitCode {
    look_oneshot("apply", |instance| {
        nuxut_shell::services::look::oneshot::apply_once(instance).map(|o| (o.ok(), serde_json::to_string_pretty(&o)))
    })
}

/// `look revert`: birleşik görünümü Nuxut öncesine geri alır (ADR-0004 §8-3; framework.toml'a
/// bakmaz, çalışan shell'e dokunmaz). Çıkış kodları `look apply` ile aynı.
fn look_revert() -> ExitCode {
    look_oneshot("revert", |instance| {
        nuxut_shell::services::look::oneshot::revert_once(instance).map(|o| (o.ok(), serde_json::to_string_pretty(&o)))
    })
}

/// `look` tek seferlik komutlarının ortak kabuğu: günlük, i18n, örnek adı doğrulaması, JSON.
fn look_oneshot(
    name: &str,
    run: impl FnOnce(Option<&str>) -> Result<(bool, serde_json::Result<String>), String>,
) -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    // Yaratılan dosya başlıkları kullanıcının dilinde (t!).
    nuxut::i18n::init(&[nuxut::i18n::embed!("shell")]);
    // Örnek adı run() ile aynı kuralla doğrulanır; geçersizse hiçbir şey yazılmaz.
    let app = match ipc::app_name() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("nuxut-shell look {name}: {e}");
            return ExitCode::from(2);
        }
    };
    let instance = (app != ipc::APP_NAME).then(|| std::env::var(ipc::INSTANCE_ENV).unwrap_or_default());
    match run(instance.as_deref()) {
        Ok((ok, json)) => {
            match json {
                Ok(json) => println!("{json}"),
                Err(e) => eprintln!("nuxut-shell look {name}: {e}"),
            }
            if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
        Err(e) => {
            eprintln!("nuxut-shell look {name}: {e}");
            ExitCode::from(2)
        }
    }
}

/// `hypr takeover|restore-backup`: ana Hyprland ayarı üzerinde tek seferlik iş (ADR-0007); çalışan
/// shell'e dokunmaz. JSON stdout'a; başarıda 0, değilse 1, örnek adı geçersizse 2.
fn hypr_oneshot(name: &str, run: impl FnOnce(&nuxut_shell::services::hypr_config::sync::SyncEnv) -> nuxut_shell::services::hypr_config::takeover::TakeoverReport) -> ExitCode {
    let app = match ipc::app_name() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("nuxut-shell hypr {name}: {e}");
            return ExitCode::from(2);
        }
    };
    let instance = (app != ipc::APP_NAME).then(|| std::env::var(ipc::INSTANCE_ENV).unwrap_or_default());
    let env = nuxut_shell::services::hypr_config::sync::SyncEnv::from_process(nuxut_apply::Env::from_process(instance.as_deref().filter(|s| !s.is_empty())));
    let report = run(&env);
    println!("{}", report.to_json());
    if report.ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Komutu çalışan shell'e gönderir. Yanıt `error:` ile başlarsa çıkış kodu 1.
fn send(args: &[String]) -> ExitCode {
    let app = match ipc::app_name() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("nuxut-shell: {e}");
            return ExitCode::from(2);
        }
    };
    let sent = nuxut::ipc::Client::new(&app).and_then(|c| c.with_timeout(ipc::REPLY_TIMEOUT).send_args(args));
    match sent {
        Ok(reply) => {
            let failed = reply.starts_with("error:");
            if !reply.is_empty() {
                if failed { eprintln!("{reply}") } else { println!("{reply}") }
            }
            if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
        }
        Err(nuxut::ipc::IpcError::NotRunning) => {
            eprintln!("nuxut-shell çalışmıyor");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("nuxut-shell: komut gönderilemedi: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    // Katalog: framework.toml [locale] + ortam; metin kuran her şeyden önce (ADR-0010).
    nuxut::i18n::init(&[nuxut_shell::i18n::source(), nuxut_wallpapers::i18n_source()]);

    // Tek örnek: ikinci bir shell başlamaz. IPC kurulamazsa (güvensiz dizin vb.) shell yine
    // çalışır, yalnızca komutlar devre dışı kalır. Config bu karardan SONRA yüklenir: göç/damga
    // yalnızca kilidi almış asıl örnekte diske yazılır (ikinci çağrı ve test örneği kullanıcının
    // dosyasını değiştirip çıkmasın, göç kaydı kaybolmasın).
    let app = match ipc::app_name() {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("{e}");
            return ExitCode::from(2);
        }
    };
    if app != ipc::APP_NAME {
        tracing::info!(ipc = %app, "ayrı örnek adıyla çalışıyor ({}); shell.toml'a yazılmaz", ipc::INSTANCE_ENV);
    }
    let (options, primary) = match nuxut::ipc::try_single_instance(&app) {
        Ok(nuxut::ipc::Instance::Primary(listener)) => {
            (nuxut::RunOptions { ipc: Some(listener.with_reply_timeout(ipc::REPLY_TIMEOUT)) }, true)
        }
        Ok(nuxut::ipc::Instance::Secondary(_)) => {
            eprintln!("nuxut-shell zaten çalışıyor (komut göndermek için: nuxut-shell msg …)");
            // systemd birimi (Restart=on-failure) başka yoldan başlamış bir örnekle karşılaşırsa
            // yeniden başlatma döngüsüne girmesin (ADR-0003 Karar 5).
            let under_systemd = std::env::var_os("INVOCATION_ID").is_some();
            return if under_systemd { ExitCode::SUCCESS } else { ExitCode::FAILURE };
        }
        Err(e) => {
            tracing::warn!("komut IPC'si kurulamadı, shell komutsuz çalışacak: {e}");
            (nuxut::RunOptions::default(), false)
        }
    };

    // Tek dosya yazım ortamı (nuxut-apply, ADR-0001): test örneği (NUXUT_*_INSTANCE) yalnızca
    // kendi geçici XDG kökleri altına yazar; kilidi alamamış örnek hiçbir şey yazmaz.
    let instance = (app != ipc::APP_NAME).then_some(app.as_str());
    let apply_env = nuxut_apply::Env { disabled: !primary, ..nuxut_apply::Env::from_process(instance) };
    // Açılış onarımı diske yazar (asıl örnek; önce yedek). Sonraki yeniden yüklemeler yalnızca
    // bellekte onarır (docs/design/config-migrations.md).
    let policy = WritePolicy::for_instance(primary, &app);
    let mut state = LoadState { policy, unversioned_is_current: false, heal_write: true };
    let mut heal_reports = Vec::new();
    let (config, migration) = match ShellConfig::load(state, &apply_env) {
        Ok(loaded) => {
            for w in &loaded.warnings {
                tracing::warn!("shell.toml: {w}");
            }
            state.unversioned_is_current = loaded.unversioned_is_current;
            heal_reports.extend(loaded.heal);
            (loaded.config, loaded.migration)
        }
        Err(e) => {
            tracing::error!("shell.toml yüklenemedi, varsayılanlar kullanılıyor: {e}");
            (ShellConfig::validated_default(), None)
        }
    };
    state.heal_write = false;
    // framework.toml: yalnızca sözdizimi (şeması framework'te); tema run_with içinde okunur.
    heal_reports.extend(nuxut_shell::config_heal::heal_framework(&apply_env, policy == WritePolicy::Disk));

    // Kurulum dizini PATH'te değilse bir kez uyar (kabuk kendi ikililerini mutlak yolla çağırır).
    nuxut_shell::self_exe::warn_if_not_in_path();

    let mut shell = Shell::new(config);
    shell.set_config_load_state(state);
    // setup.toml, Hyprland eşitlemesi, Ekranlar ve `msg snapshot` aynı ortamdan (aynı kapı).
    shell.set_apply_env(apply_env);
    if let Some(record) = &migration {
        shell.on_config_migrated(record);
    }
    shell.on_config_healed(&heal_reports);
    match nuxut::run_with(shell, options) {
        Ok(()) => ExitCode::SUCCESS,
        // Compositor kapandı (oturum sonu): normal çıkış. systemd birimi `Restart=on-failure`
        // olduğundan hata kodu yeniden başlatma döngüsü yaratırdı (ADR-0003 Karar 5).
        Err(nuxut::wayland::RunError::Disconnected(reason)) => {
            tracing::info!("compositor bağlantısı kapandı, çıkılıyor: {reason}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("nuxut-shell: {e}");
            ExitCode::FAILURE
        }
    }
}
