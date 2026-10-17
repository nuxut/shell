//! Oturumu kapat: önce compositor'a temiz çıkış, olmazsa logind `Session.Terminate`.
//! Tasarım: `docs/design/logout.md`.
//!
//! `Session.Terminate` oturumun bütün süreçlerini öldürür; Hyprland DRM'i/VT'yi geri vermeden
//! ölünce tty'ye dönüşte siyah ekran kalıyordu (2026-10-16, `start-hyprland` ile tty'den açılan
//! oturum). Doğru sıra: compositor'a çıkmasını söyle (uygulamalar kapanır, Hyprland VT'yi
//! bırakır, tty ya da DM geri gelir); compositor yanıt vermezse ya da süre içinde çıkmazsa
//! Terminate yedeği.
//!
//! Yol seçimi saf ([`plan`]); IO ([`request_exit`]) çağıranın thread'inde, bloklar.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::services::hyprland::{HyprError, HyprlandClient, compositor_pid_in};

/// Çıkış istendikten sonra kabuk hâlâ yaşıyorsa (compositor çıkmadı) Terminate yedeği.
pub const FALLBACK_AFTER: Duration = Duration::from_secs(10);
/// `uwsm stop`'un en uzun beklenen süresi (kendisi oturumu durdurur; biz de onunla ölürüz).
const UWSM_TIMEOUT: Duration = Duration::from_secs(5);
/// Okunan küçük sistem dosyalarının sınırı.
const MAX_SMALL_FILE: u64 = 64 * 1024;

/// Compositor'dan çıkış yolu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitPath {
    /// uwsm oturumu: `uwsm stop` (graphical-session birimleri sırayla durur, compositor biriminin
    /// ardından oturum kapanır; doğrudan `exit` uwsm'in temizliğini atlar).
    Uwsm,
    /// Hyprland `dispatch exit` (`lua`: Lua ayar kipinde `dispatch hl.dsp.exit()`).
    Hyprland { lua: bool },
}

/// Çıkış thread'inden ana döngüye.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogoutEvent {
    /// Compositor çıkışı kabul etti; kabuk birazdan kapanır.
    Requested(ExitPath),
    /// Hiçbir yol işlemedi: Terminate yedeği.
    Failed(String),
    /// Çıkış istendi ama [`FALLBACK_AFTER`] sonra hâlâ yaşıyoruz: Terminate yedeği.
    Stalled,
}

/// Hyprland çıkış komutu (ayar sağlayıcısına göre; `hyprctl -j status` `configProvider`).
pub fn exit_command(lua: bool) -> &'static str {
    if lua { "dispatch hl.dsp.exit()" } else { "dispatch exit" }
}

/// `/proc/<pid>/cgroup` içeriği uwsm'in compositor birimini (`wayland-wm@<id>.service`) mi
/// gösteriyor.
pub fn is_uwsm_cgroup(cgroup: &str) -> bool {
    cgroup.lines().any(|l| l.rsplit('/').any(|part| part.starts_with("wayland-wm@") && part.ends_with(".service")))
}

/// Denenecek yollar, sırayla. Hyprland istemcisi yoksa (compositor bilinmiyor) boş: çağıran
/// doğrudan Terminate'e düşer.
pub fn plan(have_hyprland: bool, uwsm: bool, lua: bool) -> Vec<ExitPath> {
    let mut out = Vec::new();
    if uwsm {
        out.push(ExitPath::Uwsm);
    }
    if have_hyprland {
        out.push(ExitPath::Hyprland { lua });
    }
    out
}

/// Compositor uwsm altında mı: Hyprland örnek dizinindeki `hyprland.lock` → PID → cgroup.
fn compositor_under_uwsm(instance_dir: &Path) -> bool {
    let Some(pid) = compositor_pid_in(instance_dir) else { return false };
    read_small(Path::new(&format!("/proc/{pid}/cgroup"))).is_some_and(|c| is_uwsm_cgroup(&c))
}

fn read_small(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut s = String::new();
    std::fs::File::open(path).ok()?.take(MAX_SMALL_FILE).read_to_string(&mut s).ok()?;
    Some(s)
}

/// Lua ayar kipi mi (`j/status` → `configProvider == "lua"`); sorulamazsa hyprlang varsayılır.
fn hyprland_is_lua(client: &HyprlandClient) -> bool {
    client.config_provider().is_ok_and(|p| p == "lua")
}

/// Hyprland'e çıkış komutu. Ret ya da yanıt yoksa hata.
pub fn hyprland_exit(client: &HyprlandClient, lua: bool) -> Result<(), HyprError> {
    client.dispatch_raw(exit_command(lua))
}

fn uwsm_stop() -> Result<(), String> {
    let mut child = Command::new("uwsm")
        .arg("stop")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("uwsm başlatılamadı: {e}"))?; // i18n: tanı (yalnız log)
    // `uwsm stop` oturumu durdururken bizi de öldürebilir; ancak hızlı biten bir hata yakalanır.
    let deadline = std::time::Instant::now() + UWSM_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(s)) if s.success() => return Ok(()),
            Ok(Some(s)) => return Err(format!("uwsm stop: {s}")),
            Ok(None) if std::time::Instant::now() >= deadline => return Ok(()),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("uwsm beklenemedi: {e}")),
        }
    }
}

/// Compositor'a çıkışı iste (bloklar). `Ok(yol)`: istek kabul edildi, compositor kapanıyor;
/// `Err`: hiçbir yol işlemedi, çağıran Terminate'e düşmeli.
pub fn request_exit(client: Option<&HyprlandClient>) -> Result<ExitPath, String> {
    let uwsm = client.and_then(HyprlandClient::instance_dir).is_some_and(compositor_under_uwsm);
    let lua = client.is_some_and(hyprland_is_lua);
    let mut errors = Vec::new();
    for path in plan(client.is_some(), uwsm, lua) {
        let result = match path {
            ExitPath::Uwsm => uwsm_stop(),
            ExitPath::Hyprland { lua } => match client {
                Some(c) => hyprland_exit(c, lua).map_err(|e| e.to_string()),
                None => continue,
            },
        };
        match result {
            Ok(()) => return Ok(path),
            Err(e) => {
                tracing::warn!(?path, "oturumu kapat: çıkış yolu başarısız: {e}");
                errors.push(e);
            }
        }
    }
    if errors.is_empty() {
        errors.push("compositor bilinmiyor (Hyprland yok)".into());
    }
    Err(errors.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_command_follows_config_provider() {
        assert_eq!(exit_command(false), "dispatch exit");
        assert_eq!(exit_command(true), "dispatch hl.dsp.exit()");
    }

    #[test]
    fn uwsm_cgroup_detection() {
        assert!(is_uwsm_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/session.slice/wayland-wm@hyprland.desktop.service\n"
        ));
        assert!(!is_uwsm_cgroup("0::/user.slice/user-1000.slice/session-2.scope\n"), "tty'den start-hyprland");
        assert!(!is_uwsm_cgroup("0::/user.slice/user-1000.slice/user@1000.service/app.slice/wayland-wm-env@x.service\n"));
        assert!(!is_uwsm_cgroup(""));
    }

    #[test]
    fn plan_order() {
        assert_eq!(plan(true, false, false), [ExitPath::Hyprland { lua: false }]);
        assert_eq!(plan(true, false, true), [ExitPath::Hyprland { lua: true }]);
        assert_eq!(plan(true, true, false), [ExitPath::Uwsm, ExitPath::Hyprland { lua: false }], "uwsm önce, olmazsa exit");
        assert!(plan(false, false, false).is_empty(), "Hyprland yok: doğrudan Terminate");
    }
}
