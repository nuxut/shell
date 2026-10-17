//! Dış komutlar (grub-mkconfig, grub-script-check, grub-mkfont, mkinitcpio; giriş ekranı için
//! systemctl, localectl, systemd-tmpfiles): mutlak yol, temiz
//! ortam, stdin kapalı, çıktı satır satır olay olarak, süre sınırlı (aşılırsa öldürülür).

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::events::{Event, Events, sanitize};

/// Çalıştırılabilir dış araçlar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    GrubMkconfig,
    GrubScriptCheck,
    GrubMkfont,
    Mkinitcpio,
    // Giriş ekranı alanı.
    Systemctl,
    Localectl,
    SystemdTmpfiles,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Self::GrubMkconfig => "grub-mkconfig",
            Self::GrubScriptCheck => "grub-script-check",
            Self::GrubMkfont => "grub-mkfont",
            Self::Mkinitcpio => "mkinitcpio",
            Self::Systemctl => "systemctl",
            Self::Localectl => "localectl",
            Self::SystemdTmpfiles => "systemd-tmpfiles",
        }
    }

    /// Süre sınırı: grub-mkconfig os-prober ile diskleri tarayabilir; mkinitcpio birden çok
    /// ön ayar ve sıkıştırma yapar.
    pub fn timeout(self) -> Duration {
        match self {
            Self::GrubMkconfig => Duration::from_secs(300),
            Self::Mkinitcpio => Duration::from_secs(900),
            Self::GrubScriptCheck | Self::GrubMkfont => Duration::from_secs(60),
            Self::Systemctl | Self::Localectl | Self::SystemdTmpfiles => Duration::from_secs(60),
        }
    }
}

/// Araç yolları: üretimde `/usr/bin/<ad>`; test kipinde verilen dizin.
#[derive(Debug, Clone)]
pub struct Bins {
    dir: String,
    /// Test kipinde komutlara geçirilen ek ortam (sahte araçlar test kökünü bilsin).
    extra_env: Vec<(String, String)>,
}

impl Bins {
    pub fn system() -> Self {
        Self { dir: "/usr/bin".into(), extra_env: vec![] }
    }

    pub fn at(dir: impl Into<String>, extra_env: Vec<(String, String)>) -> Self {
        Self { dir: dir.into(), extra_env }
    }

    pub fn path(&self, t: Tool) -> String {
        format!("{}/{}", self.dir, t.name())
    }
}

/// Komut hatası.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmdError {
    Spawn(String),
    Failed { tool: &'static str, code: Option<i32> },
    Timeout(&'static str),
}

impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "komut başlatılamadı: {e}"),
            Self::Failed { tool, code: Some(c) } => write!(f, "{tool} başarısız (çıkış {c})"),
            Self::Failed { tool, code: None } => write!(f, "{tool} sinyalle sonlandı"),
            Self::Timeout(t) => write!(f, "{t} süre sınırını aştı, durduruldu"),
        }
    }
}

impl std::error::Error for CmdError {}

/// Komutu çalıştırır, çıktıyı `Log` olayı olarak akıtır.
pub fn run(bins: &Bins, tool: Tool, args: &[&str], events: &Events) -> Result<(), CmdError> {
    let mut cmd = Command::new(bins.path(tool));
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/bin:/usr/sbin")
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir("/");
    for (k, v) in &bins.extra_env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| CmdError::Spawn(format!("{}: {e}", bins.path(tool))))?;
    let mut readers = Vec::new();
    for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>)].into_iter().flatten() {
        let ev = events.clone();
        let source = tool.name().to_owned();
        readers.push(std::thread::spawn(move || {
            let mut r = BufReader::new(stream);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                // Satır başına en çok 64 KiB okunur (taşan kısım bir sonraki "satır" olur).
                match std::io::Read::take(&mut r, 64 * 1024).read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = sanitize(buf.strip_suffix(b"\n").unwrap_or(&buf));
                        if !line.trim().is_empty() {
                            ev.emit(Event::Log { source: source.clone(), line });
                        }
                    }
                }
            }
        }));
    }
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Ok(s),
            Ok(None) if start.elapsed() > tool.timeout() => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(CmdError::Timeout(tool.name()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => break Err(CmdError::Spawn(e.to_string())),
        }
    };
    // Süre aşımında okuyucular beklenmez: öldürülen sürecin torunları boruyu açık tutabilir.
    if status.is_ok() {
        for r in readers {
            let _ = r.join();
        }
    }
    let status = status?;
    if status.success() { Ok(()) } else { Err(CmdError::Failed { tool: tool.name(), code: status.code() }) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake(dir: &std::path::Path, tool: Tool, body: &str) {
        let p = dir.join(tool.name());
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn streams_output_and_reports_failure() {
        let t = tempfile::tempdir().unwrap();
        fake(t.path(), Tool::GrubScriptCheck, "echo satır1; echo hata >&2; printf '\\033[31mrenk\\n'; exit 3");
        let bins = Bins::at(t.path().to_str().unwrap(), vec![]);
        let (ev, buf) = Events::memory();
        let r = run(&bins, Tool::GrubScriptCheck, &[], &ev);
        assert_eq!(r, Err(CmdError::Failed { tool: "grub-script-check", code: Some(3) }));
        let logs: Vec<String> = buf.lock().unwrap().iter().filter_map(|e| match e {
            Event::Log { line, .. } => Some(line.clone()),
            _ => None,
        }).collect();
        assert!(logs.contains(&"satır1".to_owned()) && logs.contains(&"hata".to_owned()));
        assert!(logs.contains(&"[31mrenk".to_owned()), "kaçış dizisi ayıklandı");
    }

    #[test]
    fn environment_is_clean() {
        let t = tempfile::tempdir().unwrap();
        fake(t.path(), Tool::GrubMkfont, "[ -z \"$HOME\" ] && [ \"$PATH\" = /usr/local/sbin:/usr/local/bin:/usr/bin:/usr/sbin ] && [ \"$(pwd)\" = / ]");
        let bins = Bins::at(t.path().to_str().unwrap(), vec![]);
        assert_eq!(run(&bins, Tool::GrubMkfont, &[], &Events::memory().0), Ok(()));
    }

    #[test]
    fn missing_tool_is_spawn_error() {
        let bins = Bins::at("/nonexistent-nuxut", vec![]);
        assert!(matches!(run(&bins, Tool::Mkinitcpio, &[], &Events::memory().0), Err(CmdError::Spawn(_))));
    }
}
