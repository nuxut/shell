//! Sahte `polkit-agent-helper-1` (YALNIZCA `fake-polkit` testleri): Unix soketinde dinler, her
//! bağlantıda polkit 127 soket protokolünü konuşur (bkz. `nuxut_polkit::protocol`).
//!
//! `nuxut-polkit-fake-helper --socket <yol> --password <parola> [--bus <adres>] [--mode <kip>]`
//!
//! Başarıda (ve `--bus` verildiyse) sahte Authority'ye `AuthenticationAgentResponse2(uid,
//! çerez, unix-user)` gönderir, ancak sonra `SUCCESS` yazar — gerçek yardımcı gibi. Kipler:
//! `normal`, `garbage` (istem yerine bilinmeyen satır), `long` (sınır üstü satır), `info`
//! (istemden önce PAM_TEXT_INFO + PAM_ERROR_MSG), `eof` (istemden sonra kapat), `partial`
//! (yanıttan sonra satır sonusuz `SUCC` yazıp bağlantıyı keser; Authority'ye yanıt yok).

#[cfg(not(debug_assertions))]
compile_error!("sahte yardımcı yalnızca debug derlemesinde");

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};

use nuxut_polkit::protocol::escape;
use zbus::zvariant::Value;

#[derive(Clone)]
struct Opts {
    password: String,
    bus: Option<String>,
    mode: String,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let (Some(socket), Some(password)) = (get("--socket"), get("--password")) else {
        eprintln!("kullanım: --socket <yol> --password <parola> [--bus <adres>] [--mode <kip>]");
        std::process::exit(2);
    };
    let opts = Opts {
        password,
        bus: get("--bus"),
        mode: get("--mode").unwrap_or_else(|| "normal".into()),
    };
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("soket");
    // Hazır işareti (testler bekler).
    println!("ready");
    for stream in listener.incoming().flatten() {
        let opts = opts.clone();
        std::thread::spawn(move || {
            let _ = serve(stream, &opts);
        });
    }
}

fn line(w: &mut UnixStream, key: &str, text: &str) -> std::io::Result<()> {
    w.write_all(format!("{key} {}\n", escape(text.as_bytes())).as_bytes())
}

fn serve(stream: UnixStream, opts: &Opts) -> std::io::Result<()> {
    let mut r = BufReader::new(stream.try_clone()?);
    let mut w = stream;
    let mut user = String::new();
    let mut cookie = String::new();
    r.read_line(&mut user)?;
    r.read_line(&mut cookie)?;
    let (user, cookie) = (user.trim_end().to_owned(), cookie.trim_end().to_owned());
    eprintln!("fake-helper: kullanıcı={user}");
    match opts.mode.as_str() {
        "garbage" => return w.write_all(b"HELLO WORLD\n"),
        "long" => {
            let long = format!("PAM_TEXT_INFO {}\n", "x".repeat(5000));
            return w.write_all(long.as_bytes());
        }
        "info" => {
            line(&mut w, "PAM_TEXT_INFO", "Parmağınızı okutun ya da parolayı girin")?;
            line(&mut w, "PAM_ERROR_MSG", "Uyarı: \u{202E}deneme")?;
        }
        _ => {}
    }
    line(&mut w, "PAM_PROMPT_ECHO_OFF", "Password: ")?;
    if opts.mode == "eof" {
        return Ok(());
    }
    let mut pw = String::new();
    if r.read_line(&mut pw)? == 0 {
        return Ok(());
    }
    let pw = pw.strip_suffix('\n').unwrap_or(&pw);
    if opts.mode == "partial" {
        // Yarım satır (satır sonu yok) ve bağlantı kesilir; polkitd'ye yanıt YOK.
        w.write_all(b"SUCC")?;
        return w.shutdown(std::net::Shutdown::Both);
    }
    if pw != opts.password {
        std::thread::sleep(std::time::Duration::from_millis(200));
        return w.write_all(b"FAILURE\n");
    }
    let ok = match &opts.bus {
        Some(bus) => respond(bus, &cookie).map_err(|e| eprintln!("fake-helper: {e}")).is_ok(),
        None => true,
    };
    w.write_all(if ok { b"SUCCESS\n" } else { b"FAILURE\n" })
}

fn respond(bus: &str, cookie: &str) -> zbus::Result<()> {
    let conn = zbus::blocking::connection::Builder::address(bus)?.build()?;
    let uid = rustix::process::getuid().as_raw();
    let mut props: HashMap<&str, Value> = HashMap::new();
    props.insert("uid", Value::U32(uid));
    conn.call_method(
        Some("org.freedesktop.PolicyKit1"),
        "/org/freedesktop/PolicyKit1/Authority",
        Some("org.freedesktop.PolicyKit1.Authority"),
        "AuthenticationAgentResponse2",
        &(uid, cookie, ("unix-user", props)),
    )?;
    Ok(())
}
