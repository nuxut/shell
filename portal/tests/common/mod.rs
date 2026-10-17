//! Testler için özel `dbus-daemon` (polkit testlerindeki kalıp). Kullanıcının bus'larına
//! dokunulmaz; gerçek xdg-desktop-portal'a kayıt yok.
#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use zbus::blocking::Connection;

/// `=1` ise özel `dbus-daemon` gerektiren testler bilerek atlanır (yalnızca `dbus-daemon`
/// olmayan derleme ortamları için; varsayılan: dbus-daemon yoksa test başarısız).
pub const SKIP_DBUS_ENV: &str = "NUXUT_PORTAL_SKIP_DBUS_TESTS";

pub struct PrivateBus {
    child: Child,
    pub address: String,
    _dir: tempfile::TempDir,
}

impl PrivateBus {
    /// Özel bus'ı başlatır. `dbus-daemon` yoksa ya da başlamazsa test **başarısız olur**.
    /// Bilinçli atlama yalnızca [`SKIP_DBUS_ENV`]`=1` ile: `None` döner, çağıran test atlar.
    pub fn start() -> Option<Self> {
        if std::env::var_os(SKIP_DBUS_ENV).is_some_and(|v| v == "1") {
            eprintln!("{SKIP_DBUS_ENV}=1: D-Bus testi bilerek atlandı");
            return None;
        }
        match Self::try_start() {
            Ok(bus) => Some(bus),
            Err(e) => panic!(
                "özel dbus-daemon başlatılamadı: {e}. dbus-daemon kurulu olmalı \
                 (Arch: dbus); bilerek atlamak için {SKIP_DBUS_ENV}=1"
            ),
        }
    }

    fn try_start() -> Result<Self, String> {
        let dir = tempfile::tempdir().map_err(|e| format!("geçici dizin: {e}"))?;
        let config = dir.path().join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
                dir.path().display()
            ),
        )
        .map_err(|e| format!("bus yapılandırması yazılamadı: {e}"))?;
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("dbus-daemon çalıştırılamadı ({e})"))?;
        let mut line = String::new();
        let stdout = child.stdout.take().ok_or("dbus-daemon stdout yok")?;
        let read = BufReader::new(stdout).read_line(&mut line);
        if read.is_err() || line.trim().is_empty() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("dbus-daemon adres yazmadı".into());
        }
        Ok(Self { child, address: line.trim().to_owned(), _dir: dir })
    }

    pub fn builder(&self) -> zbus::blocking::connection::Builder<'static> {
        zbus::blocking::connection::Builder::address(self.address.clone().as_str()).unwrap()
    }

    pub fn connect(&self) -> Connection {
        self.builder().build().unwrap()
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
