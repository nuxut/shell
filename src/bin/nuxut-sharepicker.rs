//! `nuxut-sharepicker`: xdg-desktop-portal-hyprland'in ekran paylaşım seçicisi
//! (`~/.config/hypr/xdph.conf`: `screencopy { custom_picker_binary = <bu ikili> }`).
//! Tasarım ve sözleşme: `docs/design/screenshare-picker.md`.
//!
//! stdout YALNIZCA sözleşme satırıdır (`[SELECTION]…`); loglar stderr'e gider. Çıkış kodu:
//! 0 seçim yazıldı, 1 iptal, 2 hata (xdph yalnızca stdout'a bakar).

use std::io::Write;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use nuxut_shell::services::sharepicker::{MAX_LIST_BYTES, PickerArgs, WINDOW_LIST_ENV, WindowList, selection_line};
use nuxut_shell::sharepicker_view::{PickerSetup, SharePicker};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args = PickerArgs::parse(std::env::args().skip(1));
    if !args.unknown.is_empty() {
        tracing::warn!(args = ?args.unknown, "tanınmayan argümanlar yok sayıldı");
    }
    let (windows, windows_available) = read_window_list();
    nuxut::i18n::init(&[nuxut_shell::i18n::source()]);

    let result = Arc::new(Mutex::new(None));
    let setup = PickerSetup { windows, windows_available, allow_token: args.allow_token, result: Arc::clone(&result) };
    if let Err(e) = nuxut::run(SharePicker::new(setup)) {
        tracing::error!("çalışma zamanı: {e}");
        return ExitCode::from(2);
    }
    let Some((choice, remember)) = result.lock().ok().and_then(|mut r| r.take()) else {
        return ExitCode::from(1);
    };
    let line = match selection_line(&choice, remember) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("{e}");
            return ExitCode::from(2);
        }
    };
    let mut out = std::io::stdout().lock();
    if out.write_all(line.as_bytes()).and_then(|()| out.flush()).is_err() {
        tracing::error!("seçim stdout'a yazılamadı");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}

/// Ortamdaki pencere listesi (boyut sınırlı). Başlıklar loglanmaz; yalnızca sayılar.
fn read_window_list() -> (WindowList, bool) {
    let Some(raw) = std::env::var_os(WINDOW_LIST_ENV) else {
        tracing::info!("{WINDOW_LIST_ENV} yok: pencere paylaşımı listelenemez");
        return (WindowList::default(), false);
    };
    if raw.len() > MAX_LIST_BYTES {
        tracing::warn!(bytes = raw.len(), "pencere listesi çok uzun; yok sayıldı");
        return (WindowList { truncated: true, ..WindowList::default() }, true);
    }
    let list = WindowList::parse(&raw.to_string_lossy());
    tracing::info!(windows = list.windows.len(), malformed = list.malformed, truncated = list.truncated, "pencere listesi");
    (list, true)
}
