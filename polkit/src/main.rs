//! `nuxut-polkit`: polkit kimlik doğrulama ajanı. Tasarım: `docs/design/polkit.md`.
//!
//! - `nuxut-polkit` — yerleşik ajan (sistem bus'ı; Wayland yok).
//! - `nuxut-polkit dialog` — ajanın istek başına başlattığı diyalog (stdin JSON, stdout sonuç).
//!
//! Loglar stderr'e gider: diyaloğun stdout'u yalnızca sonuç satırıdır.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};

use nuxut_polkit::agent::process::DialogCommand;
use nuxut_polkit::agent::{self, AgentOptions};
use nuxut_polkit::config::PolkitConfig;
use nuxut_polkit::dialog::{Identity, PolkitDialog, Setup};
use nuxut_polkit::helper::HelperPaths;
use nuxut_polkit::request::{self, DialogOutcome, DialogRequest, MAX_REQUEST_BYTES};

// Sahte bus/yardımcı/başsız diyalog yalnızca testler içindir; sürüm derlemesine asla girmez.
#[cfg(all(feature = "fake-polkit", not(debug_assertions)))]
compile_error!("fake-polkit yalnızca debug derlemesinde (test) kullanılabilir");

const USAGE: &str = "kullanım: nuxut-polkit | nuxut-polkit dialog";

fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    // Her iki kip: çekirdek dökümü ve aynı uid'den ptrace yok (diyalog parolayı tutar).
    if let Err(e) = nuxut_pam::process::set_non_dumpable() {
        tracing::warn!("PR_SET_DUMPABLE kapatılamadı: {e}");
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => daemon(),
        ["dialog"] => dialog(false),
        #[cfg(feature = "fake-polkit")]
        ["dialog", "--headless"] => dialog(true),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

fn daemon() -> i32 {
    let config = PolkitConfig::load();
    if !config.enabled {
        tracing::info!("shell.toml [polkit] enabled = false; ajan kaydolmadan çıkıyor");
        return 0;
    }
    let mut command = match DialogCommand::current_exe() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("kendi ikili yolu bulunamadı: {e}");
            return 2;
        }
    };
    fake::dialog_args(&mut command);
    let options = AgentOptions::new(command, config.timeout);
    let result = async_io::block_on(async {
        let conn = match fake::bus_address() {
            Some(addr) => zbus::connection::Builder::address(addr.as_str())?.build().await?,
            None => zbus::Connection::system().await?,
        };
        agent::run(conn, options).await
    });
    match result {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!("ajan: {e}");
            1
        }
    }
}

/// stdin'den tek satır istek (sınırlı). Kalan stdin EOF izlemesi için döner.
fn read_request() -> Result<(DialogRequest, BufReader<std::io::Stdin>), String> {
    let mut reader = BufReader::new(std::io::stdin());
    let mut line = Vec::new();
    let mut limited = (&mut reader).take(MAX_REQUEST_BYTES as u64 + 1);
    limited
        .read_until(b'\n', &mut line)
        .map_err(|e| format!("stdin: {e}"))?;
    if line.last() != Some(&b'\n') {
        return Err("istek satırı eksik ya da çok büyük".into());
    }
    line.pop();
    Ok((DialogRequest::from_line(&line)?, reader))
}


fn dialog(headless: bool) -> i32 {
    // Ajan ölürse diyalog da ölür (stdin EOF ayrıca izlenir: prctl'den önce ölmüşse).
    if let Err(e) =
        rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::TERM))
    {
        tracing::warn!("PR_SET_PDEATHSIG kurulamadı: {e}");
    }
    let outcome = match read_request() {
        Ok((request, stdin)) => run_dialog(request, stdin, headless),
        Err(e) => {
            tracing::error!("{e}");
            DialogOutcome::Failed
        }
    };
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(outcome.as_line().as_bytes());
    let _ = out.flush();
    0
}

fn run_dialog(request: DialogRequest, stdin: BufReader<std::io::Stdin>, headless: bool) -> DialogOutcome {
    let identities = Identity::resolve(&request.identities);
    if identities.is_empty() {
        tracing::error!("kimliklerin hiçbiri çözülemedi");
        return DialogOutcome::Failed;
    }
    let uids: Vec<u32> = identities.iter().map(|i| i.uid).collect();
    let default_identity = request::default_identity(&uids, rustix::process::getuid().as_raw());
    let paths = match fake::helper_paths() {
        Some(p) => p,
        None => HelperPaths::system(),
    };
    let result = Arc::new(Mutex::new(None));
    let setup = Setup {
        request,
        identities,
        default_identity,
        paths,
        stdin: Some(Box::new(stdin)),
        result: Arc::clone(&result),
    };
    if headless {
        #[cfg(feature = "fake-polkit")]
        return nuxut_polkit::headless::run(setup);
    }
    nuxut::i18n::init(&[nuxut_polkit::i18n::source()]);
    if let Err(e) = nuxut::run(PolkitDialog::new(setup)) {
        tracing::error!("çalışma zamanı: {e}");
    }
    let outcome = result.lock().ok().and_then(|mut r| r.take());
    outcome.unwrap_or(DialogOutcome::Failed)
}

/// Test kancaları: yalnızca `fake-polkit` derlemesinde; aksi hâlde hepsi kapalı ve ikilide
/// ortam değişkeni adları bile yok (`tests/no_fake_polkit.rs`).
#[cfg(feature = "fake-polkit")]
mod fake {
    use nuxut_polkit::agent::process::DialogCommand;
    use nuxut_polkit::helper::HelperPaths;

    /// Sistem bus'ı yerine bu adres (özel test bus'ı: sahte Authority + logind).
    pub fn bus_address() -> Option<String> {
        let a = std::env::var("NUXUT_POLKIT_FAKE_BUS").ok().filter(|s| !s.is_empty())?;
        tracing::warn!("SAHTE bus (yalnızca test derlemesi)");
        Some(a)
    }

    /// Sahte yardımcı soketi; setuid geri dönüşü kapalı (gerçek yardımcıya asla ulaşılmaz).
    pub fn helper_paths() -> Option<HelperPaths> {
        let s = std::env::var_os("NUXUT_POLKIT_FAKE_HELPER_SOCKET")?;
        tracing::warn!("SAHTE yardımcı soketi (yalnızca test derlemesi)");
        Some(HelperPaths {
            socket: s.into(),
            setuid: None,
        })
    }

    /// Ajanın başlattığı diyalog başsız olsun (e2e testleri).
    pub fn dialog_args(command: &mut DialogCommand) {
        if std::env::var_os("NUXUT_POLKIT_FAKE_HEADLESS").is_some() {
            command.args.push("--headless".into());
        }
    }
}

#[cfg(not(feature = "fake-polkit"))]
mod fake {
    use nuxut_polkit::agent::process::DialogCommand;
    use nuxut_polkit::helper::HelperPaths;

    pub fn bus_address() -> Option<String> {
        None
    }

    pub fn helper_paths() -> Option<HelperPaths> {
        None
    }

    pub fn dialog_args(_: &mut DialogCommand) {}
}
