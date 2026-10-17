//! Testler için özel `dbus-daemon` (kullanıcının bus'larına dokunulmaz).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

// Atla/panik kararı shell testleriyle aynı dosyadan (tek kaynak).
#[path = "../../../tests/support/skip.rs"]
pub mod skip;

pub struct PrivateBus {
    child: Child,
    pub address: String,
    _dir: tempfile::TempDir,
}

impl PrivateBus {
    /// Özel `dbus-daemon` başlatır. Başlatılamazsa panikler; yalnızca
    /// `NUXUT_SKIP_DBUS_TESTS=1` ise uyarıp `None` döner (test atlanır).
    pub fn start() -> Option<Self> {
        let dir = tempfile::tempdir().expect("geçici dizin oluşturulamadı");
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
        .expect("bus.conf yazılamadı");
        let mut child = match Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return skip::skip_or_panic("dbus-daemon", skip::SKIP_DBUS, e),
        };
        let mut line = String::new();
        let _ = BufReader::new(child.stdout.take().unwrap()).read_line(&mut line);
        let address = line.trim().to_owned();
        if address.is_empty() {
            let _ = child.kill();
            let _ = child.wait();
            return skip::skip_or_panic("dbus-daemon", skip::SKIP_DBUS, "adres yazmadı");
        }
        Some(Self {
            child,
            address,
            _dir: dir,
        })
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Sonuç bekleyen testlerin üst sınırı: yalnızca asılı kalmayı keser. Sonuç gelir gelmez test
/// sürer; yük altında (nuxut-build.slice MemoryHigh kısması, CPUWeight=30, mem-watch dondurması)
/// saniyelerce süren duraklamalar yanlış alarm üretmesin diye geniş (bkz. shell 5552ba1).
pub const LIVENESS: std::time::Duration = std::time::Duration::from_secs(120);
