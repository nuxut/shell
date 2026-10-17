//! Yardımcı bağlantısı (diyalog sürecinde): soket ya da setuid yardımcı, okuma thread'i,
//! parola yanıtı. Protokol ve kaynak: [`crate::protocol`].
//!
//! - Okuma ayrı thread'de, satır başına [`protocol::MAX_LINE_BYTES`] sınırıyla; her mesaj
//!   `on_event` ile verilir. `SUCCESS`/`FAILURE`/ayrıştırma hatası/EOF'ta thread biter.
//! - Yanıt mlock'lu, düşünce sıfırlanan [`SecretBuf`]'ta `parola + \n` olarak kurulur ve tek
//!   `write_all` ile yazılır; arada `String`/`Vec` kopyası yok.
//! - [`HelperSession`] düşünce bağlantı kapanır (soket `shutdown`; setuid yardımcıya SIGTERM,
//!   okuma thread'i çocuğu biçer). polkit `kill_helper` ile aynı.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

use nuxut_pam::SecretBuf;

use crate::protocol::{self, HelperMessage, ParseError};

/// polkit 127 soket yolu (`polkitagentsession.c`).
pub const SYSTEM_SOCKET: &str = "/run/polkit/agent-helper.socket";
/// setuid yardımcı (Arch: `PACKAGE_PREFIX "/lib/polkit-1/polkit-agent-helper-1"`).
pub const SYSTEM_SETUID_HELPER: &str = "/usr/lib/polkit-1/polkit-agent-helper-1";

/// Yardımcıya ulaşma yolları.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperPaths {
    pub socket: PathBuf,
    /// `None`: setuid geri dönüşü kapalı (testler).
    pub setuid: Option<PathBuf>,
}

impl HelperPaths {
    pub fn system() -> Self {
        Self {
            socket: PathBuf::from(SYSTEM_SOCKET),
            setuid: Some(PathBuf::from(SYSTEM_SETUID_HELPER)),
        }
    }
}

/// Okuma thread'inin olayları. Her oturumda tam olarak bir son olay ([`HelperEvent::is_final`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperEvent {
    Message(HelperMessage),
    /// Kurallara uymayan satır: oturum başarısız (bağlantı kapatılır).
    Protocol(ParseError),
    /// EOF ya da okuma hatası (`SUCCESS`/`FAILURE` gelmeden).
    Closed,
}

impl HelperEvent {
    pub fn is_final(&self) -> bool {
        !matches!(
            self,
            Self::Message(
                HelperMessage::Prompt { .. } | HelperMessage::ErrorMsg(_) | HelperMessage::TextInfo(_)
            )
        )
    }
}

/// Bağlantı türü (log için).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Socket,
    Setuid,
}

enum Writer {
    Socket(UnixStream),
    Child(ChildStdin),
}

impl Writer {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Self::Socket(s) => s.write_all(bytes),
            Self::Child(s) => s.write_all(bytes),
        }
    }
}

/// Bir yardımcı oturumu (tek kimlik, tek çerez).
pub struct HelperSession {
    writer: Option<Writer>,
    /// Soket: okuma ucunu da kapatmak için.
    socket: Option<UnixStream>,
    /// setuid: okuma thread'i EOF'ta biçer; o zamana dek SIGTERM bu kilit altında gönderilir
    /// (biçilmiş pid'e sinyal gitmesin).
    child: Option<Arc<Mutex<Option<Child>>>>,
    transport: Transport,
}

impl std::fmt::Debug for HelperSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HelperSession")
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

fn valid_field(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.bytes().any(|b| b == b'\n' || b == 0)
}

impl HelperSession {
    /// Bağlanır, kullanıcı adını ve çerezi yazar, okuma thread'ini başlatır.
    pub fn start(
        paths: &HelperPaths,
        user: &str,
        cookie: &str,
        on_event: impl FnMut(HelperEvent) + Send + 'static,
    ) -> io::Result<Self> {
        if !valid_field(user) || !valid_field(cookie) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "geçersiz kullanıcı/çerez"));
        }
        // polkit: soket dosyası varsa önce o; bağlanamazsa setuid'e düşülür.
        if paths.socket.exists() {
            match Self::start_socket(&paths.socket, user, cookie) {
                Ok((stream, reader)) => {
                    spawn_reader(reader, None, on_event)?;
                    return Ok(Self {
                        writer: Some(Writer::Socket(stream.try_clone()?)),
                        socket: Some(stream),
                        child: None,
                        transport: Transport::Socket,
                    });
                }
                Err(e) => tracing::warn!(
                    "yardımcı soketine bağlanılamadı ({:?}); setuid yardımcı deneniyor",
                    e.kind()
                ),
            }
        }
        let Some(setuid) = paths.setuid.as_deref() else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "yardımcı yok"));
        };
        Self::start_setuid(setuid, user, cookie, on_event)
    }

    fn start_socket(path: &Path, user: &str, cookie: &str) -> io::Result<(UnixStream, UnixStream)> {
        let mut stream = UnixStream::connect(path)?;
        // Tek yazım: kullanıcı adı ve çerez satırları (polkitagentsession.c sırası).
        let mut head = Vec::with_capacity(user.len() + cookie.len() + 2);
        head.extend_from_slice(user.as_bytes());
        head.push(b'\n');
        head.extend_from_slice(cookie.as_bytes());
        head.push(b'\n');
        stream.write_all(&head)?;
        let reader = stream.try_clone()?;
        Ok((stream, reader))
    }

    fn start_setuid(
        helper: &Path,
        user: &str,
        cookie: &str,
        on_event: impl FnMut(HelperEvent) + Send + 'static,
    ) -> io::Result<Self> {
        let mut cmd = Command::new(helper);
        cmd.arg(user)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Yardımcı ortamı zaten temizler; yalnızca dil değişkenlerini korur.
        for k in ["LANG", "LANGUAGE"] {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        let mut child = cmd.spawn()?;
        let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("yardımcı boruları yok"));
        };
        let mut line = Vec::with_capacity(cookie.len() + 1);
        line.extend_from_slice(cookie.as_bytes());
        line.push(b'\n');
        if let Err(e) = stdin.write_all(&line) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        let child = Arc::new(Mutex::new(Some(child)));
        spawn_reader(stdout, Some(Arc::clone(&child)), on_event)?;
        Ok(Self {
            writer: Some(Writer::Child(stdin)),
            socket: None,
            child: Some(child),
            transport: Transport::Setuid,
        })
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// İsteme yanıt verir: `secret + \n`. Yanıt [`protocol::response_allowed`] değilse
    /// yazılmaz (`InvalidInput`).
    pub fn respond(&mut self, secret: &SecretBuf) -> io::Result<()> {
        if !protocol::response_allowed(secret.as_bytes()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "yanıt yazılamaz"));
        }
        let mut line = SecretBuf::new();
        line.push_bytes(secret.as_bytes())
            .and_then(|()| line.push_bytes(b"\n"))
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "yanıt çok uzun"))?;
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "bağlantı kapalı"))?;
        let r = writer.write_all(line.as_bytes());
        drop(line);
        r
    }

    /// Bağlantıyı kapatır (idempotent).
    pub fn stop(&mut self) {
        self.writer = None;
        if let Some(s) = self.socket.take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        if let Some(child) = self.child.take()
            && let Ok(guard) = child.lock()
            && let Some(c) = guard.as_ref()
            && let Ok(pid) = i32::try_from(c.id())
            && let Some(pid) = rustix::process::Pid::from_raw(pid)
        {
            // Kilit altında: okuma thread'i henüz biçmedi, pid hâlâ bu çocuğun.
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
        }
    }
}

impl Drop for HelperSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_reader<R: Read + Send + 'static>(
    reader: R,
    child: Option<Arc<Mutex<Option<Child>>>>,
    mut on_event: impl FnMut(HelperEvent) + Send + 'static,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("nuxut-polkit-helper".into())
        .spawn(move || {
            read_events(BufReader::new(reader), &mut on_event);
            if let Some(child) = child {
                let taken = child.lock().ok().and_then(|mut g| g.take());
                if let Some(mut c) = taken {
                    // Son olaydan sonra yardımcı kendisi çıkar; kalmışsa sonlandır.
                    match c.try_wait() {
                        Ok(Some(_)) => {}
                        _ => {
                            let _ = c.kill();
                            let _ = c.wait();
                        }
                    }
                }
            }
        })
        .map(|_| ())
}

/// Satırları okuyup olaylara çevirir; ilk son olayda döner.
pub fn read_events(mut reader: impl BufRead, on_event: &mut impl FnMut(HelperEvent)) {
    loop {
        let event = match read_line_bounded(&mut reader, protocol::MAX_LINE_BYTES) {
            Ok(Line::Complete(bytes)) => match protocol::parse_line(&bytes) {
                Ok(m) => HelperEvent::Message(m),
                Err(e) => HelperEvent::Protocol(e),
            },
            Ok(Line::TooLong) => HelperEvent::Protocol(ParseError::TooLong),
            Ok(Line::Eof) | Err(_) => HelperEvent::Closed,
        };
        let last = event.is_final();
        on_event(event);
        if last {
            return;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Line {
    Complete(Vec<u8>),
    /// Sınır aşıldı (geri kalanı okunmaz; bağlantı kapatılacak).
    TooLong,
    /// Satır sonu gelmeden bağlantı bitti (yarım satır da EOF sayılır).
    Eof,
}

/// `\n`'e kadar okur; `max` bayttan uzun satır `TooLong`. Bellek `max + 1`'i aşmaz.
fn read_line_bounded(reader: &mut impl BufRead, max: usize) -> io::Result<Line> {
    let mut out = Vec::new();
    loop {
        let buf = match reader.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buf.is_empty() {
            return Ok(Line::Eof);
        }
        match buf.iter().position(|&b| b == b'\n') {
            Some(i) => {
                if out.len() + i > max {
                    return Ok(Line::TooLong);
                }
                out.extend_from_slice(&buf[..i]);
                reader.consume(i + 1);
                return Ok(Line::Complete(out));
            }
            None => {
                let n = buf.len();
                if out.len() + n > max {
                    return Ok(Line::TooLong);
                }
                out.extend_from_slice(buf);
                reader.consume(n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(input: &[u8]) -> Vec<HelperEvent> {
        let mut out = Vec::new();
        read_events(BufReader::with_capacity(7, input), &mut |e| out.push(e));
        out
    }

    #[test]
    fn conversation_until_final() {
        let ev = events(b"PAM_TEXT_INFO hi\nPAM_PROMPT_ECHO_OFF Password: \nSUCCESS\nFAILURE\n");
        assert_eq!(
            ev,
            [
                HelperEvent::Message(HelperMessage::TextInfo("hi".into())),
                HelperEvent::Message(HelperMessage::Prompt {
                    text: "Password: ".into(),
                    echo: false
                }),
                HelperEvent::Message(HelperMessage::Success),
            ]
        );
    }

    #[test]
    fn eof_and_partial_line_are_closed() {
        assert_eq!(events(b""), [HelperEvent::Closed]);
        assert_eq!(events(b"SUCCESS"), [HelperEvent::Closed]);
        assert_eq!(
            events(b"PAM_TEXT_INFO a\nSUCC"),
            [
                HelperEvent::Message(HelperMessage::TextInfo("a".into())),
                HelperEvent::Closed
            ]
        );
    }

    #[test]
    fn long_and_bad_lines_end_session() {
        let mut long = vec![b'a'; protocol::MAX_LINE_BYTES + 1];
        long.extend_from_slice(b"\nSUCCESS\n");
        assert_eq!(events(&long), [HelperEvent::Protocol(ParseError::TooLong)]);
        // Satır sonu hiç gelmeyen sonsuz akış da sınırda kesilir.
        let endless = vec![b'b'; protocol::MAX_LINE_BYTES * 3];
        assert_eq!(events(&endless), [HelperEvent::Protocol(ParseError::TooLong)]);
        let exact = [vec![b'P'; protocol::MAX_LINE_BYTES], b"\n".to_vec()].concat();
        assert_eq!(events(&exact), [HelperEvent::Protocol(ParseError::Unknown)]);
        assert_eq!(
            events(b"\xff\nSUCCESS\n"),
            [HelperEvent::Protocol(ParseError::InvalidUtf8)]
        );
        assert_eq!(
            events(b"GARBAGE\nSUCCESS\n"),
            [HelperEvent::Protocol(ParseError::Unknown)]
        );
    }

    #[test]
    fn socket_session_writes_user_cookie_and_response() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("helper.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut w = s;
            let mut lines = Vec::new();
            for _ in 0..2 {
                let mut l = String::new();
                r.read_line(&mut l).unwrap();
                lines.push(l);
            }
            w.write_all(b"PAM_PROMPT_ECHO_OFF Password: \n").unwrap();
            let mut pw = String::new();
            r.read_line(&mut pw).unwrap();
            lines.push(pw);
            w.write_all(b"SUCCESS\n").unwrap();
            lines
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let paths = HelperPaths {
            socket: path,
            setuid: None,
        };
        let mut session = HelperSession::start(&paths, "hakan", "c-1", move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        assert_eq!(session.transport(), Transport::Socket);
        let t = std::time::Duration::from_secs(5);
        assert!(matches!(rx.recv_timeout(t).unwrap(), HelperEvent::Message(HelperMessage::Prompt { .. })));
        let mut secret = SecretBuf::new();
        secret.push_str("şifre").unwrap();
        session.respond(&secret).unwrap();
        assert_eq!(rx.recv_timeout(t).unwrap(), HelperEvent::Message(HelperMessage::Success));
        assert_eq!(server.join().unwrap(), ["hakan\n", "c-1\n", "şifre\n"]);
        // Satır sonu içeren yanıt yazılmaz.
        let mut bad = SecretBuf::new();
        bad.push_str("a\nSUCCESS").unwrap();
        assert_eq!(session.respond(&bad).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn missing_socket_without_setuid_fails_and_bad_fields_rejected() {
        let paths = HelperPaths {
            socket: PathBuf::from("/nonexistent/nuxut-polkit-test.sock"),
            setuid: None,
        };
        assert!(HelperSession::start(&paths, "u", "c", |_| {}).is_err());
        assert!(HelperSession::start(&paths, "u\nx", "c", |_| {}).is_err());
        assert!(HelperSession::start(&paths, "u", "", |_| {}).is_err());
    }

    #[test]
    fn setuid_path_with_script_helper() {
        // setuid yolunun boru düzeni: argv[1] kullanıcı, stdin çerez (sahte betik).
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("helper.src");
        std::fs::write(
            &source,
            "#!/bin/sh\nread cookie\n[ \"$1\" = u ] && [ \"$cookie\" = c-9 ] || { echo FAILURE; exit 1; }\necho 'PAM_PROMPT_ECHO_OFF Password: '\nread pw\n[ \"$pw\" = ok ] && echo SUCCESS || echo FAILURE\n",
        )
        .unwrap();
        // Çalıştırılacak dosyayı alt süreç yazar: bu süreçte yazma fd'si açılsaydı,
        // aynı anda fork eden başka bir test iş parçacığının çocuğu onu exec'e dek
        // miras alır ve exec ETXTBSY ("Text file busy") ile düşerdi.
        let script = dir.path().join("helper.sh");
        let copied = std::process::Command::new("cp").arg(&source).arg(&script).status().unwrap();
        assert!(copied.success());
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
        let paths = HelperPaths {
            socket: dir.path().join("none.sock"),
            setuid: Some(script),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut s = HelperSession::start(&paths, "u", "c-9", move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        assert_eq!(s.transport(), Transport::Setuid);
        let t = std::time::Duration::from_secs(5);
        assert!(matches!(rx.recv_timeout(t).unwrap(), HelperEvent::Message(HelperMessage::Prompt { .. })));
        let mut secret = SecretBuf::new();
        secret.push_str("ok").unwrap();
        s.respond(&secret).unwrap();
        assert_eq!(rx.recv_timeout(t).unwrap(), HelperEvent::Message(HelperMessage::Success));
    }
}
