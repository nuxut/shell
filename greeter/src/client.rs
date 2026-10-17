//! greetd bağlantısı kendi thread'inde: protokol katı istek-yanıt olduğundan her komut bir
//! yanıt bekler (PAM saniyelerce sürebilir; UI donmaz). Sonuçlar `on_event` ile döner.
//!
//! Bağlantı tembel kurulur; koparsa olay [`ClientEvent::Disconnected`] olur ve sonraki komut
//! yeniden bağlanır (greetd yeniden başladıysa yeni oturum oradan açılır). Şifre
//! [`SecretBuf`] olarak thread'e taşınır, sıfırlanan tampona kodlanır ve düşer.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use nuxut_pam::SecretBuf;

use crate::protocol::{self, Request, Response};

/// greetd'ye giden komut.
#[derive(Debug)]
pub enum Command {
    Request(Request),
    /// İsteme yanıt; `None` bilgi/hata mesajının onayı.
    Respond(Option<SecretBuf>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    Response(Response),
    /// Bağlanılamadı ya da bağlantı koptu (ayrıntı yalnızca log için).
    Disconnected(String),
}

/// Yazma zaman aşımı: greetd okumuyorsa thread sonsuza dek takılmasın. Okuma sınırsız (PAM,
/// parmak izi okuyucusu kullanıcıyı bekleyebilir).
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct Client {
    tx: mpsc::Sender<Command>,
}

impl Client {
    pub fn spawn(
        socket: PathBuf,
        on_event: impl Fn(ClientEvent) + Send + 'static,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Command>();
        std::thread::Builder::new()
            .name("nuxut-greetd".into())
            .spawn(move || {
                let mut stream: Option<UnixStream> = None;
                for command in rx {
                    let event = match exchange(&socket, &mut stream, command) {
                        Ok(response) => ClientEvent::Response(response),
                        Err(e) => {
                            stream = None;
                            ClientEvent::Disconnected(e.to_string())
                        }
                    };
                    on_event(event);
                }
            })?;
        Ok(Self { tx })
    }

    /// Komutu sıraya koyar; thread öldüyse `false`.
    pub fn send(&self, command: Command) -> bool {
        self.tx.send(command).is_ok()
    }
}

fn exchange(
    socket: &Path,
    stream: &mut Option<UnixStream>,
    command: Command,
) -> io::Result<Response> {
    if stream.is_none() {
        let s = UnixStream::connect(socket)?;
        s.set_write_timeout(Some(WRITE_TIMEOUT))?;
        *stream = Some(s);
    }
    let Some(s) = stream.as_mut() else {
        return Err(io::Error::from(io::ErrorKind::NotConnected));
    };
    match command {
        Command::Request(request) => protocol::write_frame(s, &protocol::encode_request(&request)?)?,
        Command::Respond(secret) => {
            let body = protocol::encode_response(secret.as_ref().map(SecretBuf::as_bytes));
            drop(secret);
            protocol::write_frame(s, &body)?;
        }
    }
    protocol::decode_response(&protocol::read_frame(s)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;

    fn reply(stream: &mut UnixStream, json: &str) {
        protocol::write_frame(stream, json.as_bytes()).unwrap();
    }

    #[test]
    fn roundtrip_and_reconnect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("greetd.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (tx, rx) = mpsc::channel();
        let client = Client::spawn(path.clone(), move |e| tx.send(e).unwrap()).unwrap();

        client.send(Command::Request(Request::CreateSession { username: "u".into() }));
        let (mut conn, _) = listener.accept().unwrap();
        let body = protocol::read_frame(&mut conn).unwrap();
        assert_eq!(body, br#"{"type":"create_session","username":"u"}"#);
        reply(&mut conn, r#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password:"}"#);
        assert!(matches!(rx.recv().unwrap(), ClientEvent::Response(Response::AuthMessage { .. })));

        let mut s = SecretBuf::new();
        s.push_str("gizli").unwrap();
        client.send(Command::Respond(Some(s)));
        let body = protocol::read_frame(&mut conn).unwrap();
        assert_eq!(body, br#"{"type":"post_auth_message_response","response":"gizli"}"#);
        // Bağlantıyı kopar: olay Disconnected, sonraki komut yeniden bağlanır.
        drop(conn);
        assert!(matches!(rx.recv().unwrap(), ClientEvent::Disconnected(_)));
        client.send(Command::Request(Request::CancelSession));
        let (mut conn, _) = listener.accept().unwrap();
        assert_eq!(protocol::read_frame(&mut conn).unwrap(), br#"{"type":"cancel_session"}"#);
        reply(&mut conn, r#"{"type":"success"}"#);
        assert_eq!(rx.recv().unwrap(), ClientEvent::Response(Response::Success));
    }

    #[test]
    fn oversized_reply_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("greetd.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (tx, rx) = mpsc::channel();
        let client = Client::spawn(path, move |e| tx.send(e).unwrap()).unwrap();
        client.send(Command::Request(Request::CancelSession));
        let (mut conn, _) = listener.accept().unwrap();
        let _ = protocol::read_frame(&mut conn).unwrap();
        conn.write_all(&(u32::MAX).to_ne_bytes()).unwrap();
        assert!(matches!(rx.recv().unwrap(), ClientEvent::Disconnected(_)));
        let mut rest = Vec::new();
        let _ = conn.read_to_end(&mut rest);
    }

    #[test]
    fn missing_socket_is_disconnected() {
        let (tx, rx) = mpsc::channel();
        let client = Client::spawn("/nonexistent/greetd.sock".into(), move |e| tx.send(e).unwrap()).unwrap();
        client.send(Command::Request(Request::CancelSession));
        assert!(matches!(rx.recv().unwrap(), ClientEvent::Disconnected(_)));
    }
}
