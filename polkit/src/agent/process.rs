//! Diyalog çocuk süreci: başlatma, istek satırı, sonuç satırı, sonlandırma.
//!
//! - İstek stdin'e tek satır JSON olarak yazılır; stdin çocuk çıkana dek açık tutulur. Ajan
//!   ölürse çekirdek stdin'i kapatır → diyalog EOF görüp iptal eder (ayrıca PDEATHSIG).
//! - Sonuç stdout'tan en çok [`MAX_OUTPUT`] bayt okunur; tam olarak bilinen tek satır ve
//!   çıkış kodu 0 değilse `Failed` (çökme/öldürülme fail-closed).
//! - Sinyaller pidfd ile gönderilir: biçilmiş bir sürecin pid'i yeniden kullanılsa bile yanlış
//!   sürece sinyal gitmez.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use rustix::process::{PidfdFlags, Signal};

use crate::request::{DialogOutcome, DialogRequest};

/// Diyalog stdout'unun okunacak üst sınırı.
pub const MAX_OUTPUT: u64 = 64;

/// Diyaloğu başlatan komut (üretimde `nuxut-polkit dialog`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

impl DialogCommand {
    /// Bu ikilinin kendisi (`/proc/self/exe`) + `dialog`.
    pub fn current_exe() -> io::Result<Self> {
        Ok(Self {
            program: std::env::current_exe()?,
            args: vec!["dialog".into()],
        })
    }
}

/// Çalışan diyalog.
#[derive(Debug)]
pub struct DialogProcess {
    pidfd: OwnedFd,
    pid: u32,
}

impl DialogProcess {
    /// Başlatır; süreç çıkınca `on_exit(sonuç)` okuma thread'inden çağrılır (tam bir kez).
    pub fn spawn(
        command: &DialogCommand,
        request: &DialogRequest,
        on_exit: impl FnOnce(DialogOutcome) + Send + 'static,
    ) -> io::Result<Self> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let pid = child.id();
        // Çocuk henüz biçilmedi (biçen thread aşağıda başlıyor): pidfd bu sürece aittir.
        let pidfd = i32::try_from(pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .ok_or_else(|| io::Error::other("geçersiz pid"))
            .and_then(|p| rustix::process::pidfd_open(p, PidfdFlags::empty()).map_err(io::Error::from));
        let pidfd = match pidfd {
            Ok(fd) => fd,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("diyalog boruları yok"));
        };
        let line = request.to_line();
        let spawned = std::thread::Builder::new()
            .name("nuxut-polkit-dialog".into())
            .spawn(move || {
                // Yazma hatası (çocuk hemen öldü) okuma tarafında EOF olarak görünür.
                let _ = stdin.write_all(line.as_bytes()).and_then(|()| stdin.flush());
                let mut out = Vec::new();
                let read = stdout.take(MAX_OUTPUT + 1).read_to_end(&mut out);
                // Fazlası okunmaz; çocuk yazmaya devam ederse boru dolar, sonlandırılınca biter.
                let status = child.wait();
                drop(stdin);
                let outcome = match (read, status) {
                    (Ok(_), Ok(s)) if s.success() => DialogOutcome::parse(&out),
                    _ => DialogOutcome::Failed,
                };
                on_exit(outcome);
            });
        match spawned {
            Ok(_) => Ok(Self { pidfd, pid }),
            Err(e) => {
                let _ = rustix::process::pidfd_send_signal(&pidfd, Signal::KILL);
                Err(e)
            }
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// SIGTERM (diyalog yardımcı bağlantısını kapatıp `cancelled` yazar).
    pub fn terminate(&self) {
        let _ = rustix::process::pidfd_send_signal(&self.pidfd, Signal::TERM);
    }

    pub fn kill(&self) {
        let _ = rustix::process::pidfd_send_signal(&self.pidfd, Signal::KILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn request() -> DialogRequest {
        DialogRequest {
            cookie: "c-1".into(),
            action_id: "a".into(),
            message: "m".into(),
            icon_name: String::new(),
            identities: vec![1000],
            requester: None,
            timeout_s: 5,
        }
    }

    fn run(script: &str) -> (DialogProcess, mpsc::Receiver<DialogOutcome>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cmd = DialogCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.replace("$D", &dir.path().display().to_string()).into()],
        };
        let (tx, rx) = mpsc::channel();
        let p = DialogProcess::spawn(&cmd, &request(), move |o| tx.send(o).unwrap()).unwrap();
        (p, rx, dir)
    }

    const T: Duration = Duration::from_secs(10);

    #[test]
    fn outcomes_from_output_and_status() {
        let (_p, rx, dir) = run("read -r l; printf '%s\\n' \"$l\" > $D/req; echo success");
        assert_eq!(rx.recv_timeout(T).unwrap(), DialogOutcome::Success);
        let req = std::fs::read(dir.path().join("req")).unwrap();
        assert_eq!(DialogRequest::from_line(req.trim_ascii_end()).unwrap(), request());
        let (_p, rx, _d) = run("echo cancelled");
        assert_eq!(rx.recv_timeout(T).unwrap(), DialogOutcome::Cancelled);
        let (_p, rx, _d) = run("echo success; exit 3");
        assert_eq!(rx.recv_timeout(T).unwrap(), DialogOutcome::Failed);
        let (_p, rx, _d) = run("exit 0");
        assert_eq!(rx.recv_timeout(T).unwrap(), DialogOutcome::Failed);
        let (_p, rx, _d) = run("yes success");
        // Sınırsız çıktı: okuma sınırda kesilir; süreç SIGPIPE ile ölür → Failed.
        assert_eq!(rx.recv_timeout(T).unwrap(), DialogOutcome::Failed);
    }

    #[test]
    fn terminate_and_kill() {
        let (p, rx, _d) = run("exec sleep 30");
        p.terminate();
        assert_eq!(rx.recv_timeout(T).unwrap(), DialogOutcome::Failed);
        // Biçilmiş sürece sinyal zararsız (pidfd: ESRCH).
        p.terminate();
        p.kill();
        let (p, rx, _d) = run("trap '' TERM; read -r l; while :; do sleep 0.1; done");
        std::thread::sleep(Duration::from_millis(200));
        p.terminate();
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err(), "TERM yok sayıldı");
        p.kill();
        assert_eq!(rx.recv_timeout(T).unwrap(), DialogOutcome::Failed);
    }
}
