//! Oturumu kapat sahte Hyprland soketiyle: `request_exit` ayar sağlayıcısına göre doğru çıkış
//! komutunu yazar (hyprlang `dispatch exit`; Lua `dispatch hl.dsp.exit()` — Hyprland 0.56.2
//! `dispatchRequest` Lua'da girdiyi `hl.dispatch(…)` içine sarar), ret ve ulaşılamayan compositor
//! Terminate yedeğine düşer (Err).

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use nuxut_shell::services::hyprland::HyprlandClient;
use nuxut_shell::services::logout::{ExitPath, request_exit};

/// socket1: istek başına bağlantı; istekleri kaydeder. `accept_exit = false`: çıkışı reddeder.
fn fake_hypr(dir: &Path, provider: &'static str, accept_exit: bool) -> Arc<Mutex<Vec<String>>> {
    let listener = UnixListener::bind(dir.join(".socket.sock")).unwrap();
    // Kendi PID'imiz: cgroup'u uwsm birimi değil (tty'den start-hyprland gibi).
    std::fs::write(dir.join("hyprland.lock"), format!("{}\nwayland-1\n", std::process::id())).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let r = Arc::clone(&requests);
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { return };
            let mut buf = [0u8; 4096];
            let n = conn.read(&mut buf).unwrap_or(0);
            let cmd = String::from_utf8_lossy(&buf[..n]).into_owned();
            let reply = match cmd.as_str() {
                "j/status" => format!("{{\"configProvider\": \"{provider}\", \"backend\": \"drm\"}}"),
                "dispatch exit" if provider == "hyprlang" && accept_exit => "ok".to_owned(),
                "dispatch hl.dsp.exit()" if provider == "lua" && accept_exit => "ok".to_owned(),
                _ => "Invalid dispatcher".to_owned(),
            };
            r.lock().unwrap().push(cmd);
            let _ = conn.write_all(reply.as_bytes());
        }
    });
    requests
}

fn client(dir: &Path) -> HyprlandClient {
    HyprlandClient::new(dir.join(".socket.sock")).with_timeout(Duration::from_secs(30))
}

#[test]
fn hyprlang_exit() {
    let dir = tempfile::tempdir().unwrap();
    let req = fake_hypr(dir.path(), "hyprlang", true);
    assert_eq!(request_exit(Some(&client(dir.path()))), Ok(ExitPath::Hyprland { lua: false }));
    assert_eq!(*req.lock().unwrap(), ["j/status", "dispatch exit"]);
}

#[test]
fn lua_exit() {
    let dir = tempfile::tempdir().unwrap();
    let req = fake_hypr(dir.path(), "lua", true);
    assert_eq!(request_exit(Some(&client(dir.path()))), Ok(ExitPath::Hyprland { lua: true }));
    assert_eq!(*req.lock().unwrap(), ["j/status", "dispatch hl.dsp.exit()"]);
}

#[test]
fn refused_or_unreachable_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let _req = fake_hypr(dir.path(), "hyprlang", false);
    assert!(request_exit(Some(&client(dir.path()))).is_err(), "ret: Terminate yedeği");

    let empty = tempfile::tempdir().unwrap();
    let c = HyprlandClient::new(empty.path().join(".socket.sock")).with_timeout(Duration::from_millis(500));
    assert!(request_exit(Some(&c)).is_err(), "soket yok: Terminate yedeği");
    assert!(request_exit(None).is_err(), "Hyprland yok: Terminate yedeği");
}
