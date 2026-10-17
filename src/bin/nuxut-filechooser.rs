//! `nuxut-filechooser`: portal `FileChooser` isteği başına `xdg-desktop-portal-nuxut`'un başlattığı
//! seçici. Tasarım ve sözleşme: `docs/design/file-chooser.md`.
//!
//! stdin: tek satır JSON istek; sonra açık kalır, kapanması iptaldir. stdout YALNIZCA tek satır
//! JSON sonuçtur; loglar stderr'e. Çıkış kodu: 0 seçim, 1 vazgeçildi, 2 hata.

use std::io::{BufRead, Read, Write};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use nuxut_portal_backend::chooser::protocol::{ChooserRequest, EXIT_CANCELLED, EXIT_ERROR, MAX_REQUEST_BYTES};
use nuxut_shell::filechooser_view::{ChooserSetup, FileChooser};

fn code(c: i32) -> ExitCode {
    ExitCode::from(u8::try_from(c).unwrap_or(2))
}

/// İlk satırı (≤ [`MAX_REQUEST_BYTES`]) okur; stdin açık kalır.
fn read_request() -> Result<ChooserRequest, &'static str> {
    let mut line = Vec::new();
    let stdin = std::io::stdin();
    let mut limited = stdin.lock().take(MAX_REQUEST_BYTES as u64 + 1);
    limited.read_until(b'\n', &mut line).map_err(|_| "stdin okunamadı")?;
    if line.len() > MAX_REQUEST_BYTES {
        return Err("istek çok uzun");
    }
    let text = std::str::from_utf8(&line).map_err(|_| "istek UTF-8 değil")?;
    ChooserRequest::from_line(text)
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")))
        .init();
    let request = match read_request() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("{e}");
            return code(EXIT_ERROR);
        }
    };
    tracing::info!(?request, "istek");
    nuxut::i18n::init(&[nuxut_shell::i18n::source()]);
    let result = Arc::new(Mutex::new(None));
    let setup = ChooserSetup { request, result: Arc::clone(&result), watch_stdin: true };
    if let Err(e) = nuxut::run(FileChooser::new(setup)) {
        tracing::error!("çalışma zamanı: {e}");
        return code(EXIT_ERROR);
    }
    let Some(result) = result.lock().ok().and_then(|mut r| r.take()) else {
        return code(EXIT_CANCELLED);
    };
    let mut out = std::io::stdout().lock();
    if out.write_all(result.to_line().as_bytes()).and_then(|()| out.flush()).is_err() {
        tracing::error!("sonuç stdout'a yazılamadı");
        return code(EXIT_ERROR);
    }
    ExitCode::SUCCESS
}
