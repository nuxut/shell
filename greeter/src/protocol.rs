//! greetd IPC'si (`greetd-ipc(7)`): her ileti 4 baytlık yerel bayt sıralı uzunluk + JSON.
//!
//! Dış girdi: greetd root çalışır ama soket yolu ortamdan gelir; yanıt boyutu
//! [`MAX_MESSAGE`] ile sınırlı, metinler [`MAX_TEXT`] bayta kırpılır ve kontrol karakterleri
//! atılır. Bilinmeyen ileti türü hata olur (panik yok).
//!
//! Şifre yanıtı serde'den geçmez: [`encode_response`] JSON'u baştan yeterli kapasiteyle
//! ayrılmış, düşerken sıfırlanan tek tampona yazar (yeniden tahsis yok, ara `String` yok).

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Bir iletinin en büyük gövdesi (greetd'nin yanıtları birkaç yüz bayttır).
pub const MAX_MESSAGE: usize = 64 * 1024;
/// greetd'den gelen tek metnin (PAM mesajı, hata açıklaması) bayt üst sınırı.
pub const MAX_TEXT: usize = 512;
/// `start_session` komut ve ortam öğesi sayısı üst sınırı.
pub const MAX_ARGS: usize = 64;

/// greeter → greetd. `post_auth_message_response` burada yok: [`encode_response`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    CreateSession { username: String },
    StartSession { cmd: Vec<String>, env: Vec<String> },
    CancelSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    /// Kimlik doğrulama başarısız (yanlış şifre, hesap kilitli).
    AuthError,
    /// Diğer hatalar (oturum zaten var, komut çalıştırılamadı…).
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMessageType {
    /// Yankılı istem (ör. doğrulama kodu).
    Visible,
    /// Gizli istem (şifre).
    Secret,
    Info,
    Error,
}

/// greetd → greeter.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Success,
    Error {
        error_type: ErrorType,
        description: String,
    },
    AuthMessage {
        auth_message_type: AuthMessageType,
        auth_message: String,
    },
}

impl Response {
    /// Metinleri [`sanitize`]'dan geçirir.
    fn sanitized(self) -> Self {
        match self {
            Self::Success => Self::Success,
            Self::Error { error_type, description } => Self::Error {
                error_type,
                description: sanitize(&description),
            },
            Self::AuthMessage { auth_message_type, auth_message } => Self::AuthMessage {
                auth_message_type,
                auth_message: sanitize(&auth_message),
            },
        }
    }
}

/// Kontrol karakterlerini atar (satır sonu boşluğa döner), baştaki/sondaki boşluğu kırpar,
/// [`MAX_TEXT`] bayta karakter sınırında kısaltır.
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_TEXT));
    for ch in text.chars() {
        let ch = match ch {
            '\n' | '\t' => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        if out.len() + ch.len_utf8() > MAX_TEXT {
            break;
        }
        out.push(ch);
    }
    out.trim().to_owned()
}

/// Çerçeveyi yazar: uzunluk + gövde.
pub fn write_frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "ileti çok büyük"));
    }
    let len = u32::try_from(body.len()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    w.write_all(&len.to_ne_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// Bir çerçeve okur; [`MAX_MESSAGE`]'dan büyükse `InvalidData` (gövde okunmaz).
pub fn read_frame(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "greetd iletisi çok büyük"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(body)
}

pub fn encode_request(request: &Request) -> io::Result<Vec<u8>> {
    if let Request::StartSession { cmd, env } = request
        && (cmd.is_empty() || cmd.len() > MAX_ARGS || env.len() > MAX_ARGS)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "geçersiz oturum komutu"));
    }
    serde_json::to_vec(request).map_err(io::Error::other)
}

pub fn decode_response(body: &[u8]) -> io::Result<Response> {
    serde_json::from_slice::<Response>(body)
        .map(Response::sanitized)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

const RESPONSE_HEAD: &[u8] = br#"{"type":"post_auth_message_response","response":"#;

/// `post_auth_message_response` gövdesi; `None` → `"response":null` (bilgi/hata mesajının
/// onayı). Tampon en kötü kaçış (`\u00XX`, 6 kat) için baştan ayrılır: hiç büyümez.
pub fn encode_response(answer: Option<&[u8]>) -> Zeroizing<Vec<u8>> {
    let cap = RESPONSE_HEAD.len() + answer.map_or(4, |a| a.len() * 6 + 2) + 1;
    let mut out = Zeroizing::new(Vec::with_capacity(cap));
    out.extend_from_slice(RESPONSE_HEAD);
    match answer {
        None => out.extend_from_slice(b"null"),
        Some(bytes) => {
            out.push(b'"');
            for &b in bytes {
                match b {
                    b'"' => out.extend_from_slice(b"\\\""),
                    b'\\' => out.extend_from_slice(b"\\\\"),
                    0x00..=0x1f | 0x7f => {
                        const HEX: &[u8; 16] = b"0123456789abcdef";
                        out.extend_from_slice(b"\\u00");
                        out.push(HEX[usize::from(b >> 4)]);
                        out.push(HEX[usize::from(b & 0xf)]);
                    }
                    // UTF-8 baytları olduğu gibi (girdi `&str`'den gelir).
                    _ => out.push(b),
                }
            }
            out.push(b'"');
        }
    }
    out.push(b'}');
    debug_assert!(out.len() <= cap);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_match_greetd_schema() {
        let create = encode_request(&Request::CreateSession { username: "ayşe".into() }).unwrap();
        assert_eq!(
            std::str::from_utf8(&create).unwrap(),
            r#"{"type":"create_session","username":"ayşe"}"#
        );
        let start = encode_request(&Request::StartSession {
            cmd: vec!["start-hyprland".into()],
            env: vec!["XDG_SESSION_DESKTOP=nuxut".into()],
        })
        .unwrap();
        assert_eq!(
            std::str::from_utf8(&start).unwrap(),
            r#"{"type":"start_session","cmd":["start-hyprland"],"env":["XDG_SESSION_DESKTOP=nuxut"]}"#
        );
        assert_eq!(encode_request(&Request::CancelSession).unwrap(), br#"{"type":"cancel_session"}"#);
        assert!(encode_request(&Request::StartSession { cmd: vec![], env: vec![] }).is_err());
    }

    #[test]
    fn secret_response_is_valid_json() {
        let pw = "a\"b\\c\u{1}şı".as_bytes();
        let body = encode_response(Some(pw));
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["type"], "post_auth_message_response");
        assert_eq!(v["response"], "a\"b\\c\u{1}şı");
        let ack = encode_response(None);
        let v: serde_json::Value = serde_json::from_slice(&ack).unwrap();
        assert!(v["response"].is_null());
    }

    #[test]
    fn secret_buffer_never_grows() {
        // Her bayt en kötü kaçışta: kapasite yetmeli (yeniden tahsis eski kopyayı bırakırdı).
        let worst = vec![0x01u8; nuxut_pam::MAX_SECRET_BYTES];
        let body = encode_response(Some(&worst));
        assert!(body.len() <= body.capacity());
        let expected_cap = RESPONSE_HEAD.len() + worst.len() * 6 + 3;
        assert_eq!(body.capacity(), expected_cap);
    }

    #[test]
    fn responses_parse_and_sanitize() {
        assert_eq!(decode_response(br#"{"type":"success"}"#).unwrap(), Response::Success);
        let r = decode_response(
            br#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password: \u001b[31m\n"}"#,
        )
        .unwrap();
        assert_eq!(
            r,
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                auth_message: "Password: [31m".into()
            }
        );
        let r = decode_response(br#"{"type":"error","error_type":"auth_error","description":"pam"}"#).unwrap();
        assert!(matches!(r, Response::Error { error_type: ErrorType::AuthError, .. }));
        assert!(decode_response(br#"{"type":"bogus"}"#).is_err());
        assert!(decode_response(b"\xff").is_err());
        let long = format!(r#"{{"type":"error","error_type":"error","description":"{}"}}"#, "ş".repeat(1000));
        let Response::Error { description, .. } = decode_response(long.as_bytes()).unwrap() else { panic!() };
        assert!(description.len() <= MAX_TEXT && description.chars().all(|c| c == 'ş'));
    }

    #[test]
    fn frames_roundtrip_and_limit() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"{}").unwrap();
        assert_eq!(&buf[..4], &2u32.to_ne_bytes());
        assert_eq!(read_frame(&mut buf.as_slice()).unwrap(), b"{}");
        let mut huge = (MAX_MESSAGE as u32 + 1).to_ne_bytes().to_vec();
        huge.extend_from_slice(b"x");
        assert_eq!(read_frame(&mut huge.as_slice()).unwrap_err().kind(), io::ErrorKind::InvalidData);
        // Kesik gövde: UnexpectedEof (panik yok).
        let mut short = 10u32.to_ne_bytes().to_vec();
        short.extend_from_slice(b"abc");
        assert_eq!(read_frame(&mut short.as_slice()).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }
}
