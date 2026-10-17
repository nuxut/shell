//! `xdg-desktop-portal-nuxut`: oturum bus'ında `org.freedesktop.impl.portal.desktop.nuxut`.
//! xdg-desktop-portal tarafından D-Bus ile etkinleştirilir (docs/design/portal.md).

use std::process::ExitCode;

use nuxut_portal::service::{Paths, Portal};
use zbus::blocking::{Connection, MessageIterator};

/// Bağlantı kapanınca ya da `NameLost` gelince süreci sonlandıran bekçi thread'i.
fn exit_with_bus(conn: &Connection) -> zbus::Result<()> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameLost")?
        .arg(0, nuxut_portal::BUS_NAME)?
        .build();
    let iter = MessageIterator::for_match_rule(rule, conn, Some(4))?;
    std::thread::Builder::new()
        .name("bus-watch".into())
        .spawn(move || {
            // Akış yalnızca bağlantı kapanınca biter; ilk NameLost de çıkış nedenidir.
            let reason = match iter.into_iter().next() {
                Some(Ok(_)) => "ad kaybedildi",
                _ => "bus bağlantısı kapandı",
            };
            tracing::info!("{reason}; çıkılıyor");
            std::process::exit(0);
        })
        .map_err(|e| zbus::Error::Failure(format!("thread: {e}")))?;
    Ok(())
}

/// `--write-portals-conf`: `nuxut-portals.conf`'u kullanıcının Hyprland portal seçiminden üretir
/// (portals_conf modülü). `dry_run`: içerik stdout'a, hiçbir şey yazılmaz; ne olacağı stderr'e.
/// `file_chooser`: `None` → var olan Nuxut dosyasındaki tercih (kapalılık işareti yoksa açık).
fn write_portals_conf(dry_run: bool, file_chooser: Option<bool>) -> ExitCode {
    use nuxut_portal::portals_conf::{self as pc, Outcome};
    let Some(dirs) = pc::SearchDirs::from_env() else {
        eprintln!("HOME ya da XDG_CONFIG_HOME/XDG_DATA_HOME yok; hedef belirlenemiyor");
        return ExitCode::FAILURE;
    };
    let target = dirs.target();
    let file_chooser = file_chooser.unwrap_or_else(|| {
        !(pc::is_ours(&target).unwrap_or(false) && std::fs::read_to_string(&target).is_ok_and(|t| pc::file_chooser_disabled(&t)))
    });
    let source = pc::find_source(&dirs, pc::HOST_DESKTOPS);
    let regen = std::env::current_exe()
        .map(|p| format!("{} --write-portals-conf", p.display()))
        .unwrap_or_else(|_| pc::DEFAULT_REGEN.to_owned());
    let content = pc::render_opts(source.as_ref(), &regen, file_chooser);
    let from = source.as_ref().map_or_else(|| "kaynak yok (Hyprland varsayılanı)".to_owned(), |s| s.path.display().to_string());
    if dry_run {
        print!("{content}");
        let plan = match pc::is_ours(&target) {
            Ok(true) => "yazılırdı",
            Ok(false) => "dokunulmazdı (ilk satır işareti yok: sizin)",
            Err(e) => {
                eprintln!("{}: {e}", target.display());
                return ExitCode::FAILURE;
            }
        };
        eprintln!("[deneme] {} {plan}; kaynak: {from}", target.display());
        return ExitCode::SUCCESS;
    }
    match pc::write(&target, &content) {
        Ok(Outcome::Written) => println!("{} yazıldı (kaynak: {from}); etkisi sonraki girişte.", target.display()),
        Ok(Outcome::Unchanged) => println!("{} güncel (kaynak: {from}).", target.display()),
        Ok(Outcome::UserOwned) => println!(
            "{} sizin (ilk satır işareti yok); dokunulmadı. Nuxut'a geri vermek için silin ve komutu yeniden çalıştırın.",
            target.display()
        ),
        Err(e) => {
            eprintln!("{} yazılamadı: {e}", target.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || {
        eprintln!("kullanım: {} [--version | --write-portals-conf [--dry-run] [--no-filechooser | --filechooser]]", env!("CARGO_PKG_NAME"));
        ExitCode::from(2)
    };
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => {}
        ["--version"] => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        flags if flags.contains(&"--write-portals-conf") => {
            let known = ["--write-portals-conf", "--dry-run", "--no-filechooser", "--filechooser"];
            let both = flags.contains(&"--no-filechooser") && flags.contains(&"--filechooser");
            if both || flags.iter().any(|f| !known.contains(f)) || flags.len() > known.len() {
                return usage();
            }
            // Açık bayrak yoksa önceki tercih (dosyadaki kapalılık işareti) korunur.
            let file_chooser = if flags.contains(&"--filechooser") {
                Some(true)
            } else if flags.contains(&"--no-filechooser") {
                Some(false)
            } else {
                None
            };
            return write_portals_conf(flags.contains(&"--dry-run"), file_chooser);
        }
        _ => return usage(),
    }

    let builder = match zbus::blocking::connection::Builder::session() {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("oturum bus'ı yok: {e}");
            return ExitCode::FAILURE;
        }
    };
    let portal = match Portal::start(builder, Paths::from_env()) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("{} alınamadı: {e}", nuxut_portal::BUS_NAME);
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(state = ?portal.current(), "hazır");
    // Ömür bus'a bağlı: oturum bus'ı kapanınca (çıkış; systemd'siz etkinleştirmede birim yok) ya da
    // ad kaybedilince süreç biter — ana thread inotify'da beklerken artık süreç kalmaz.
    if let Err(e) = exit_with_bus(portal.connection()) {
        tracing::error!("bus izlenemiyor: {e}");
        return ExitCode::FAILURE;
    }
    match portal.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("config izleme durdu: {e}");
            ExitCode::FAILURE
        }
    }
}
