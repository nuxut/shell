//! `nuxut-shell msg displays apply <JSON>` uçtan uca: gerçek ikili (istemci tarafı doğrulama +
//! framework `send_args` kodlaması) → geçici `XDG_RUNTIME_DIR` altında sahte IPC sunucusu →
//! shell'in ayrıştırıcısı (`ipc::parse_args` + `protocol::parse_apply`). Kullanıcının çalışan
//! shell'ine dokunmaz (ayrı örnek adı, geçici dizin).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixListener;
use std::process::Command;

use nuxut_shell::ipc::{self, DisplaysCommand};
use nuxut_shell::services::displays::protocol::parse_apply;

/// Geçerli bir `apply` JSON'u: `n` monitör, uzun kimliklerle (boşluk içerir).
fn apply_json(n: usize, id_len: usize) -> String {
    let displays: Vec<String> = (0..n)
        .map(|i| {
            format!(
                r#"{{"id":"Maker {i}|{}|S{i}","enabled":true,"mode":{{"w":1920,"h":1080,"mhz":60000}},"pos":[{},0],"scale_120":120,"transform":0,"vrr":"off","mirror_of":null}}"#,
                "M ".repeat(id_len / 2),
                i * 1920
            )
        })
        .collect();
    format!(r#"{{"v":1,"base_gen":3,"primary":null,"displays":[{}]}}"#, displays.join(","))
}

/// Tek bağlantılık sahte sunucu: satırı okur, sekmeyle ayırır, shell'in ayrıştırıcısından geçirir.
fn serve_once(listener: UnixListener) -> std::thread::JoinHandle<Option<Vec<String>>> {
    std::thread::spawn(move || {
        listener.set_nonblocking(false).ok()?;
        let (mut stream, _) = listener.accept().ok()?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).ok()?;
        let args: Vec<String> = line.trim_end_matches('\n').split('\t').map(str::to_owned).collect();
        let reply = match ipc::parse_args(&args) {
            Ok(ipc::Command::Displays(DisplaysCommand::Apply(json))) => match parse_apply(&json) {
                Ok(req) => format!("ok {}", req.layout.displays.len()),
                Err(e) => e.line(),
            },
            Ok(other) => format!("ok {other:?}"),
            Err(e) => format!("error: {e}"),
        };
        stream.write_all(reply.as_bytes()).ok()?;
        Some(args)
    })
}

fn msg(runtime: &std::path::Path, args: &[&str]) -> std::process::Output {
    // İkili yolu çalışma zamanında (cargo test verir); derleme anındaki değer yedek (bkz. support::repo).
    let exe = std::env::var_os("CARGO_BIN_EXE_nuxut-shell")
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_nuxut-shell").into());
    Command::new(exe)
        .arg("msg")
        .args(args)
        .env("XDG_RUNTIME_DIR", runtime)
        .env("NUXUT_SHELL_INSTANCE", "displays-e2e")
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .output()
        .unwrap()
}

#[test]
fn large_apply_travels_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nuxut");
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let sock = dir.join("nuxut-shell-displays-e2e.sock");

    // ~3,9 KB geçerli istek: ayrışır.
    let mut n = 1;
    let json = loop {
        let j = apply_json(n, 160);
        if j.len() > 3700 {
            break j;
        }
        n += 1;
    };
    assert!(json.len() > 3700 && json.len() <= ipc::MAX_DISPLAYS_ARG, "{}", json.len());
    let listener = UnixListener::bind(&sock).unwrap();
    let server = serve_once(listener);
    let out = msg(tmp.path(), &["displays", "apply", &json]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout} {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout.trim(), format!("ok {n}"));
    let args = server.join().unwrap().unwrap();
    assert_eq!(args, ["displays", "apply", json.as_str()]);

    // ~4,1 KB: istemci göndermeden reddeder (sunucu yok; bağlanılmaz).
    std::fs::remove_file(&sock).unwrap();
    let big = apply_json(n + 2, 160);
    assert!(big.len() > 4000);
    let out = msg(tmp.path(), &["displays", "apply", &big]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("too_large"), "{}", String::from_utf8_lossy(&out.stderr));

    // Başka komut 512 bayt sınırında kalır.
    let long = "x".repeat(600);
    let out = msg(tmp.path(), &["panel", "toggle", &long]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("çok uzun"), "{}", String::from_utf8_lossy(&out.stderr));
}
