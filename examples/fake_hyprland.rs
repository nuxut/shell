//! Sahte Hyprland IPC'si (yalnızca izole test compositor'ında görsel test için): workspace
//! göstergesini gerçek Hyprland olmadan, bilinen bir durumla çizdirir.
//!
//! ```sh
//! cargo run --example fake_hyprland -- <örnek-dizini> <monitör-adı>
//! HYPRLAND_INSTANCE_SIGNATURE=<dizin adı> nuxut-shell   # aynı XDG_RUNTIME_DIR/hypr altında
//! ```
//!
//! `<örnek-dizini>` = `$XDG_RUNTIME_DIR/hypr/<imza>`; kullanıcının Hyprland örneğinden farklı bir
//! imza seçilmelidir. Durum: aşağıdaki pencereler; `<monitör-adı>` 2. workspace'te, sahte
//! ikinci monitör `FAKE-2` 4. workspace'te. `dispatch workspace N` aktif workspace'i değiştirir
//! ve socket2'ye `workspace>>N` olayı yollar (gösterge yeniden sorgular).
//!
//! Büyüteç (t124): `j/getoption cursor:zoom_factor`, `j/status` (hyprlang), `keyword
//! cursor:zoom_factor <f>` ve `eval hl.config({cursor={zoom_factor=<f>}})` taklit edilir; her
//! büyüteç isteği stderr'e `zoom: …` olarak yazılır (test betiği logdan doğrular).
//!
//! Klavye düzeni: `j/devices` ana klavyede `tr,us` döner; `dispatch switchxkblayout current
//! next` düzeni değiştirip socket2'ye `activelayout` yollar (`keyboard: …` stderr).

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::json;

/// (workspace, sınıf, genişlik, yükseklik)
const WINDOWS: &[(i64, &str, i64, i64)] = &[
    (1, "firefox", 1900, 1000),
    (2, "kitty", 900, 1000),
    (2, "code-oss", 1000, 1000),
    (3, "org.gnome.Nautilus", 1900, 1000),
    (5, "discord", 1900, 1000),
    (7, "mystery-app", 1900, 1000),
    (9, "org.telegram.desktop", 1900, 1000),
    (14, "Spotify", 1900, 1000),
    (-1337, "obsidian", 1900, 1000),
];

struct State {
    monitor: String,
    active: i64,
    /// Special workspace açık mı.
    special: bool,
    events: Vec<UnixStream>,
    /// Ana klavyenin etkin düzeni (`tr,us`; `switchxkblayout current next` değiştirir).
    layout: usize,
    /// `cursor:zoom_factor`.
    zoom: f64,
}

fn snapshot(s: &State) -> String {
    let special = if s.special { json!({"id": -98, "name": "special:magic"}) } else { json!({"id": 0, "name": ""}) };
    let monitors = json!([
        {"id": 0, "name": s.monitor, "width": 1280, "height": 720, "scale": 1.0, "focused": true,
         "activeWorkspace": {"id": s.active, "name": ws_name(s.active)},
         "specialWorkspace": special},
        {"id": 1, "name": "FAKE-2", "x": 1280, "width": 1280, "height": 720, "scale": 1.0, "focused": false,
         "activeWorkspace": {"id": 4, "name": "4"}, "specialWorkspace": {"id": 0, "name": ""}},
    ]);
    let mut ids: Vec<i64> = WINDOWS.iter().map(|w| w.0).chain([s.active, 4]).collect();
    ids.sort_unstable();
    ids.dedup();
    let workspaces: Vec<_> = ids
        .iter()
        .map(|&id| {
            let n = WINDOWS.iter().filter(|w| w.0 == id).count();
            let mon = if id == 4 { "FAKE-2" } else { s.monitor.as_str() };
            json!({"id": id, "name": ws_name(id), "monitor": mon, "windows": n, "hasfullscreen": false})
        })
        .collect();
    let clients: Vec<_> = WINDOWS
        .iter()
        .enumerate()
        .map(|(i, &(ws, class, w, h))| {
            json!({"address": format!("0x{i:x}"), "mapped": true, "hidden": false, "size": [w, h],
                   "workspace": {"id": ws, "name": ws_name(ws)}, "class": class, "initialClass": class,
                   "title": "", "fullscreen": 0, "focusHistoryID": i})
        })
        .collect();
    format!("{monitors}\n\n\n{}\n\n\n{{}}\n\n\n{}", json!(workspaces), json!(clients))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(dir), Some(monitor)) = (args.next(), args.next()) else {
        eprintln!("kullanım: fake_hyprland <örnek-dizini> <monitör-adı> [named]");
        std::process::exit(2);
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("dizin");
    let (s1, s2) = (dir.join(".socket.sock"), dir.join(".socket2.sock"));
    let _ = std::fs::remove_file(&s1);
    let _ = std::fs::remove_file(&s2);
    // Üçüncü argüman `named`: adlı workspace (`notes`) aktif ve special workspace açık başlar.
    let named = args.next().as_deref() == Some("named");
    let active = if named { -1337 } else { 2 };
    let state = Arc::new(Mutex::new(State { monitor, active, special: named, events: Vec::new(), zoom: 1.0, layout: 0 }));

    let listener2 = UnixListener::bind(&s2).expect("socket2");
    let st = Arc::clone(&state);
    thread::spawn(move || {
        for conn in listener2.incoming().flatten() {
            st.lock().unwrap().events.push(conn);
        }
    });

    let listener1 = UnixListener::bind(&s1).expect("socket1");
    eprintln!("sahte Hyprland hazır: {}", dir.display());
    for mut conn in listener1.incoming().flatten() {
        let mut buf = [0u8; 4096];
        let n = conn.read(&mut buf).unwrap_or(0);
        let cmd = String::from_utf8_lossy(&buf[..n]).into_owned();
        let mut s = state.lock().unwrap();
        let reply = if let Some(id) = cmd.strip_prefix("dispatch workspace ").and_then(|a| a.trim().parse::<i64>().ok()) {
            eprintln!("dispatch workspace {id}");
            s.active = id;
            s.events.retain_mut(|e| e.write_all(format!("workspace>>{id}\n").as_bytes()).is_ok());
            "ok".to_owned()
        } else if let Some(reply) = keyboard_request(&mut s, &cmd) {
            reply
        } else if let Some(reply) = zoom_request(&mut s, &cmd) {
            reply
        } else if cmd.starts_with("[[BATCH]]") {
            snapshot(&s)
        } else {
            "unknown request".to_owned()
        };
        let _ = conn.write_all(reply.as_bytes());
    }
}

fn ws_name(id: i64) -> String {
    if id == -1337 { "notes".to_owned() } else { id.to_string() }
}

/// Büyüteç istekleri; başka istekse `None`.
fn zoom_request(s: &mut State, cmd: &str) -> Option<String> {
    let reply = if cmd == "j/getoption cursor:zoom_factor" {
        format!("{{\"option\": \"cursor:zoom_factor\", \"float\": {:.6}, \"set\": false }}", s.zoom)
    } else if cmd == "j/status" {
        "{\"configProvider\": \"hyprlang\", \"backend\": \"headless\"}".to_owned()
    } else if let Some(v) = cmd
        .strip_prefix("keyword cursor:zoom_factor ")
        .or_else(|| cmd.strip_prefix("eval hl.config({cursor={zoom_factor=").and_then(|v| v.strip_suffix("}})")))
    {
        match v.trim().parse::<f64>() {
            Ok(f) if (1.0..=10.0).contains(&f) => {
                s.zoom = f;
                "ok".to_owned()
            }
            _ => "error: invalid value".to_owned(),
        }
    } else {
        return None;
    };
    eprintln!("zoom: {cmd} -> {reply}");
    Some(reply)
}

/// Klavye düzeni istekleri (bar düzen göstergesi): `j/devices` (ana klavye `tr,us`) ve
/// `dispatch switchxkblayout current next` (sıradaki düzen + `activelayout` olayı).
fn keyboard_request(s: &mut State, cmd: &str) -> Option<String> {
    const NAMES: [&str; 2] = ["Turkish", "English (US)"];
    let reply = if cmd == "j/devices" {
        json!({"mice": [], "keyboards": [
            {"address": "0x1", "name": "fake-keyboard", "rules": "", "model": "", "layout": "tr,us", "variant": "",
             "options": "", "active_layout_index": s.layout, "active_keymap": NAMES[s.layout], "main": true}
        ]})
        .to_string()
    } else if cmd == "dispatch switchxkblayout current next" {
        s.layout = (s.layout + 1) % NAMES.len();
        let event = format!("activelayout>>fake-keyboard,{}\n", NAMES[s.layout]);
        s.events.retain_mut(|e| e.write_all(event.as_bytes()).is_ok());
        "ok".to_owned()
    } else {
        return None;
    };
    eprintln!("keyboard: {cmd}");
    Some(reply)
}
