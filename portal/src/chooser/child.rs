//! Seçici sürecini (`nuxut-filechooser`) çalıştırma: istek stdin'e, sonuç stdout'tan.
//!
//! - stdin isteği yazdıktan sonra AÇIK kalır; kapanması iptaldir ([`Running::cancel`], ya da portal
//!   ölünce çekirdek kapatır) — seçici yetim kalmaz.
//! - Süreç yalnızca tek bir kilit altında biçilir (`try_wait`/`wait`) ve öldürülür (`kill` yalnızca
//!   aynı kilit altında `try_wait` `None` verdiyse): biçilmiş PID'e sinyal gitmez.
//! - stdout EOF'tan ya da iptalden sonra [`Grace`] süresinde çıkmayan süreç `SIGKILL` alır.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::protocol::{ChooserRequest, ChooserResult, EXIT_CANCELLED, EXIT_SELECTED, MAX_RESULT_BYTES};

/// İptal ya da stdout kapandıktan sonra çıkış için tanınan süre.
pub const GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);

/// Seçicinin sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Selected(ChooserResult),
    Cancelled,
    /// Neden (yol/ad içermez).
    Failed(&'static str),
}

#[derive(Debug)]
struct Proc {
    child: Child,
    status: Option<ExitStatus>,
}

impl Proc {
    /// Çıktıysa biçer; çıkmadıysa `None`.
    fn reap(&mut self) -> Option<ExitStatus> {
        if self.status.is_none() {
            self.status = self.child.try_wait().ok().flatten();
        }
        self.status
    }

    fn kill(&mut self) {
        if self.reap().is_none() {
            let _ = self.child.kill();
        }
    }
}

#[derive(Debug)]
struct Shared {
    proc: Mutex<Proc>,
    stdin: Mutex<Option<ChildStdin>>,
    cancelled: Mutex<bool>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Çalışan seçici.
#[derive(Debug, Clone)]
pub struct Running {
    shared: Arc<Shared>,
}

impl Running {
    /// İptal: stdin kapanır (seçici penceresini kapatıp çıkar); [`GRACE`] içinde çıkmazsa öldürülür.
    /// Sonuç yine [`spawn`]'ın alıcısına gelir. Tekrar çağrı zararsız.
    pub fn cancel(&self) {
        {
            let mut c = lock(&self.shared.cancelled);
            if *c {
                return;
            }
            *c = true;
        }
        drop(lock(&self.shared.stdin).take());
        let shared = Arc::clone(&self.shared);
        let _ = std::thread::Builder::new().name("chooser-kill".into()).spawn(move || {
            let deadline = Instant::now() + GRACE;
            while Instant::now() < deadline {
                if lock(&shared.proc).reap().is_some() {
                    return;
                }
                std::thread::sleep(POLL);
            }
            lock(&shared.proc).kill();
        });
    }
}

/// Seçiciyi başlatır. Sonuç tam bir kez `done`'a verilir (çalıştırıcı thread'inden).
pub fn spawn(bin: &Path, request: &ChooserRequest, done: impl FnOnce(Outcome) + Send + 'static) -> std::io::Result<Running> {
    let mut child = Command::new(bin).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let shared = Arc::new(Shared { proc: Mutex::new(Proc { child, status: None }), stdin: Mutex::new(None), cancelled: Mutex::new(false) });
    let running = Running { shared: Arc::clone(&shared) };
    let line = request.to_line();
    let spawned = std::thread::Builder::new().name("chooser".into()).spawn(move || {
        let outcome = drive(&shared, stdin, stdout, &line);
        done(outcome);
    });
    if let Err(e) = spawned {
        lock(&running.shared.proc).kill();
        let _ = lock(&running.shared.proc).child.wait();
        return Err(e);
    }
    Ok(running)
}

fn drive(shared: &Shared, stdin: Option<ChildStdin>, stdout: Option<std::process::ChildStdout>, line: &str) -> Outcome {
    if let Some(mut stdin) = stdin {
        // Seçici okumadan ölürse yazma EPIPE alır; sonuç yine çıkış kodundan.
        let _ = stdin.write_all(line.as_bytes()).and_then(|()| stdin.flush());
        if !*lock(&shared.cancelled) {
            *lock(&shared.stdin) = Some(stdin);
        }
    }
    let mut out = Vec::new();
    let mut too_long = false;
    if let Some(stdout) = stdout {
        let mut limited = stdout.take(MAX_RESULT_BYTES as u64 + 1);
        if limited.read_to_end(&mut out).is_err() {
            out.clear();
        }
        too_long = out.len() > MAX_RESULT_BYTES;
    }
    // stdout kapandı: seçici çıkıyor. Tanınan süre içinde çıkmazsa öldürülür.
    let deadline = Instant::now() + GRACE;
    let status = loop {
        if let Some(s) = lock(&shared.proc).reap() {
            break s;
        }
        if Instant::now() >= deadline {
            let mut p = lock(&shared.proc);
            p.kill();
            let s = p.child.wait();
            break match s {
                Ok(s) => s,
                Err(_) => return Outcome::Failed("seçici biçilemedi"),
            };
        }
        std::thread::sleep(POLL);
    };
    drop(lock(&shared.stdin).take());
    if *lock(&shared.cancelled) {
        return Outcome::Cancelled;
    }
    match status.code() {
        Some(EXIT_SELECTED) if too_long => Outcome::Failed("sonuç çok uzun"),
        Some(EXIT_SELECTED) => match ChooserResult::from_bytes(&out) {
            Ok(r) => Outcome::Selected(r),
            Err(e) => Outcome::Failed(e),
        },
        Some(EXIT_CANCELLED) => Outcome::Cancelled,
        Some(_) => Outcome::Failed("seçici hatayla çıktı"),
        None => Outcome::Failed("seçici sinyalle öldü"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chooser::protocol::Mode;
    use std::sync::mpsc;

    fn request() -> ChooserRequest {
        ChooserRequest {
            mode: Mode::Open,
            title: String::new(),
            app_id: String::new(),
            parent: String::new(),
            accept_label: None,
            modal: true,
            multiple: false,
            directory: false,
            filters: vec![],
            current_filter: None,
            choices: vec![],
            current_folder: None,
            current_name: None,
            files: vec![],
        }
    }

    /// Betiği alt süreçte yazar (bu süreçte yazma fd'si açılırsa paralel testin çocuğu onu miras
    /// alır, exec ETXTBSY ile düşer; 3003a95).
    fn script(dir: &Path, body: &str) -> std::path::PathBuf {
        let src = dir.join("s.src");
        std::fs::write(&src, format!("#!/bin/sh\n{body}\n")).unwrap();
        let bin = dir.join("s.sh");
        assert!(Command::new("cp").arg(&src).arg(&bin).status().unwrap().success());
        assert!(Command::new("chmod").arg("+x").arg(&bin).status().unwrap().success());
        bin
    }

    fn run(body: &str, cancel_after: Option<Duration>) -> (Outcome, Duration) {
        let dir = tempfile::tempdir().unwrap();
        let bin = script(dir.path(), body);
        let (tx, rx) = mpsc::channel();
        let start = Instant::now();
        let r = spawn(&bin, &request(), move |o| tx.send(o).unwrap()).unwrap();
        if let Some(d) = cancel_after {
            std::thread::sleep(d);
            r.cancel();
            r.cancel();
        }
        let o = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        (o, start.elapsed())
    }

    #[test]
    fn selected_cancelled_failed() {
        let (o, _) = run(r#"read -r req; case "$req" in *'"mode":"open"'*) printf '{"paths":["/tmp"],"choices":[],"filter":null}\n' ;; esac"#, None);
        assert_eq!(o, Outcome::Selected(ChooserResult { paths: vec!["/tmp".into()], ..Default::default() }));
        assert_eq!(run("read -r req; exit 1", None).0, Outcome::Cancelled);
        assert!(matches!(run("read -r req; echo '{bozuk'", None).0, Outcome::Failed(_)));
        assert!(matches!(run("read -r req; exit 2", None).0, Outcome::Failed(_)));
        assert!(matches!(run("read -r req; echo '{\"paths\":[]}'; kill -9 $$", None).0, Outcome::Failed(_)));
    }

    #[test]
    fn cancel_closes_stdin_and_child_exits() {
        // stdin kapanınca `cat` biter, betik 1 ile çıkar: zarif iptal (öldürme gerekmez).
        let (o, t) = run("read -r req; cat >/dev/null; exit 1", Some(Duration::from_millis(100)));
        assert_eq!(o, Outcome::Cancelled);
        assert!(t < GRACE, "{t:?}");
    }

    #[test]
    fn stubborn_child_is_killed_after_grace() {
        // stdin'i yok sayan seçici: iptalden GRACE sonra SIGKILL.
        let (o, t) = run("trap '' TERM; read -r req; exec sleep 30", Some(Duration::from_millis(50)));
        assert_eq!(o, Outcome::Cancelled);
        assert!(t >= GRACE && t < GRACE + Duration::from_secs(3), "{t:?}");
    }

    #[test]
    fn closed_stdout_but_alive_is_killed() {
        let (o, t) = run("read -r req; exec >&-; sleep 30", None);
        assert!(matches!(o, Outcome::Failed(_)));
        assert!(t < GRACE + Duration::from_secs(3), "{t:?}");
    }

    #[test]
    fn oversized_result_is_rejected() {
        let (o, _) = run(&format!("read -r req; head -c {} /dev/zero | tr '\\0' a", MAX_RESULT_BYTES + 10), None);
        assert!(matches!(o, Outcome::Failed(_)));
    }

    #[test]
    fn missing_binary_is_an_error() {
        assert!(spawn(Path::new("/yok/nuxut-filechooser"), &request(), |_| {}).is_err());
    }
}
