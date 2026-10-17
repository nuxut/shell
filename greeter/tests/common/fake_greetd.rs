//! Sahte greetd (yalnızca test ve canlı test örneği): gerçek kimlik doğrulama yok, betikli
//! PAM konuşması. `examples/fake_greetd.rs` de bunu kullanır.
//!
//! Kullanıcılar:
//! - `ayse` / şifre `nuxut`: tek gizli istem.
//! - `parmak`: önce bilgi mesajı (parmak izi istemi; pam_fprintd benzeri), sonra şifre `nuxut`.
//! - `kod`: şifre `nuxut`, sonra yankılı "Doğrulama kodu:" (`123456`).
//! - `kilitli`: önce hata mesajı (pam_faillock benzeri), sonra şifre ne olursa olsun `auth_error`.
//! - diğerleri: gizli istem, sonra `auth_error` (PAM bilinmeyen kullanıcıyı da istemle karşılar).

#![allow(dead_code)]

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};

/// greetd'nin çerçevesi: yerel bayt sıralı u32 uzunluk + JSON.
fn read(stream: &mut UnixStream) -> io::Result<serde_json::Value> {
    use std::io::Read;
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_ne_bytes(len) as usize;
    if len > 64 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "büyük"));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn write(stream: &mut UnixStream, value: serde_json::Value) -> io::Result<()> {
    use std::io::Write;
    let body = serde_json::to_vec(&value)?;
    stream.write_all(&(body.len() as u32).to_ne_bytes())?;
    stream.write_all(&body)
}

fn secret(text: &str) -> serde_json::Value {
    serde_json::json!({"type": "auth_message", "auth_message_type": "secret", "auth_message": text})
}
fn visible(text: &str) -> serde_json::Value {
    serde_json::json!({"type": "auth_message", "auth_message_type": "visible", "auth_message": text})
}
fn info(kind: &str, text: &str) -> serde_json::Value {
    serde_json::json!({"type": "auth_message", "auth_message_type": kind, "auth_message": text})
}
fn success() -> serde_json::Value {
    serde_json::json!({"type": "success"})
}
fn error(kind: &str, description: &str) -> serde_json::Value {
    serde_json::json!({"type": "error", "error_type": kind, "description": description})
}

/// Oturumun konuşma adımları.
#[derive(Debug, Clone, PartialEq)]
enum Step {
    Ack(serde_json::Value),
    Answer { prompt: serde_json::Value, expected: Option<&'static str> },
}

fn script(user: &str) -> Vec<Step> {
    let pw = |expected| Step::Answer { prompt: secret("Password: "), expected };
    match user {
        "ayse" => vec![pw(Some("nuxut"))],
        "parmak" => vec![Step::Ack(info("info", "Parmağınızı okuyucuya koyun")), pw(Some("nuxut"))],
        "kod" => vec![pw(Some("nuxut")), Step::Answer { prompt: visible("Doğrulama kodu:"), expected: Some("123456") }],
        "kilitli" => vec![Step::Ack(info("error", "Hesap 10 dakika kilitli")), pw(None)],
        _ => vec![pw(None)],
    }
}

/// Olay günlüğü (test doğrular; örnek stderr'e basar). Şifre yazılmaz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Logged {
    Create(String),
    Cancel,
    Start { cmd: Vec<String>, env: Vec<String> },
    Denied(String),
}

/// Tek bağlantıya hizmet eder (greetd her greeter bağlantısı için ayrı durum tutar).
pub fn serve_connection(mut stream: UnixStream, log: &mut dyn FnMut(Logged)) -> io::Result<()> {
    let mut session: Option<(String, Vec<Step>, bool)> = None; // (kullanıcı, kalan adımlar, doğrulandı)
    loop {
        let req = match read(&mut stream) {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let reply = match req["type"].as_str() {
            Some("create_session") => {
                let user = req["username"].as_str().unwrap_or("").to_owned();
                if session.is_some() {
                    error("error", "a session is already being configured")
                } else {
                    log(Logged::Create(user.clone()));
                    let steps = script(&user);
                    let first = match &steps[0] {
                        Step::Ack(v) => v.clone(),
                        Step::Answer { prompt, .. } => prompt.clone(),
                    };
                    session = Some((user, steps, false));
                    first
                }
            }
            Some("post_auth_message_response") => match session.as_mut() {
                None => error("error", "no session active"),
                Some((user, steps, done)) => {
                    let step = steps.remove(0);
                    let ok = match step {
                        Step::Ack(_) => true,
                        Step::Answer { expected, .. } => expected.is_some_and(|e| req["response"].as_str() == Some(e)),
                    };
                    if !ok {
                        log(Logged::Denied(user.clone()));
                        session = None;
                        error("auth_error", "pam_authenticate: AUTH_ERR")
                    } else if let Some(next) = steps.first() {
                        match next {
                            Step::Ack(v) => v.clone(),
                            Step::Answer { prompt, .. } => prompt.clone(),
                        }
                    } else {
                        *done = true;
                        success()
                    }
                }
            },
            Some("start_session") => match &session {
                Some((_, _, true)) => {
                    let list = |k: &str| -> Vec<String> {
                        req[k].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default()
                    };
                    log(Logged::Start { cmd: list("cmd"), env: list("env") });
                    success()
                }
                _ => error("error", "session not yet authenticated"),
            },
            Some("cancel_session") => {
                log(Logged::Cancel);
                match session.take() {
                    Some(_) => success(),
                    None => error("error", "no session active"),
                }
            }
            _ => error("error", "unknown request"),
        };
        write(&mut stream, reply)?;
    }
}

/// Dinleyicideki bağlantılara sırayla hizmet eder (sonsuza dek).
pub fn serve(listener: UnixListener, mut log: impl FnMut(Logged)) {
    for stream in listener.incoming().flatten() {
        let _ = serve_connection(stream, &mut log);
    }
}
