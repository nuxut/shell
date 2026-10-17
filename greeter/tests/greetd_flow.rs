//! Uçtan uca (UI'sız): durum makinesi + soket thread'i + sahte greetd. Kullanıcının yazdığı
//! metin `SendInput` eyleminde verilir; eylemler uygulamadaki gibi yürütülür.

mod common;

use std::os::unix::net::UnixListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::fake_greetd::{self, Logged};
use nuxut_greeter::client::{Client, ClientEvent, Command};
use nuxut_greeter::flow::{Action, Event, Machine, Phase};
use nuxut_greeter::protocol::Request;
use nuxut_lock::state::AuthStatus;
use nuxut_pam::{Outcome, SecretBuf};

struct Rig {
    machine: Machine,
    client: Client,
    events: mpsc::Receiver<ClientEvent>,
    log: mpsc::Receiver<Logged>,
    exited: bool,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("greetd.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (log_tx, log) = mpsc::channel();
        std::thread::spawn(move || fake_greetd::serve(listener, move |l| { let _ = log_tx.send(l); }));
        let (tx, events) = mpsc::channel();
        let client = Client::spawn(path, move |e| { let _ = tx.send(e); }).unwrap();
        Self { machine: Machine::new(), client, events, log, exited: false, _dir: dir }
    }

    /// Olayı verir, eylemleri yürütür ve makine yanıt beklemeyene dek greetd yanıtlarını işler.
    fn feed(&mut self, event: Event, typed: &str) {
        let mut queue = vec![event];
        while let Some(ev) = queue.pop() {
            for action in self.machine.handle(ev, Instant::now()) {
                match action {
                    Action::Send(r) => assert!(self.client.send(Command::Request(r))),
                    Action::SendInput => {
                        let mut s = SecretBuf::new();
                        s.push_str(typed).unwrap();
                        assert!(self.client.send(Command::Respond(Some(s))));
                    }
                    Action::Acknowledge => assert!(self.client.send(Command::Respond(None))),
                    Action::StartSession => assert!(self.client.send(Command::Request(Request::StartSession {
                        cmd: vec!["start-hyprland".into()],
                        env: vec!["XDG_SESSION_DESKTOP=nuxut".into()],
                    }))),
                    Action::Exit => self.exited = true,
                    Action::ClearInput | Action::Shake | Action::WakeAfter(_) => {}
                }
            }
            if self.machine.in_flight() {
                match self.events.recv_timeout(Duration::from_secs(10)).unwrap() {
                    ClientEvent::Response(r) => queue.push(Event::Response(r)),
                    ClientEvent::Disconnected(e) => panic!("bağlantı koptu: {e}"),
                }
            }
        }
    }

    fn logged(&self) -> Vec<Logged> {
        self.log.try_iter().collect()
    }
}

#[test]
fn password_login_starts_session() {
    let mut r = Rig::new();
    r.feed(Event::SelectUser("ayse".into()), "");
    assert!(matches!(r.machine.phase(), Phase::Prompt { secret: true, .. }));
    r.feed(Event::Submit, "nuxut");
    assert!(r.exited);
    assert_eq!(r.machine.phase(), &Phase::Done);
    let log = r.logged();
    assert_eq!(log[0], Logged::Create("ayse".into()));
    assert!(matches!(&log[1], Logged::Start { cmd, env } if cmd == &["start-hyprland"] && env.len() == 1));
}

#[test]
fn wrong_password_retries_on_fresh_session() {
    let mut r = Rig::new();
    r.feed(Event::SelectUser("ayse".into()), "");
    r.feed(Event::Submit, "yanlis");
    assert!(matches!(r.machine.status(), AuthStatus::Failed { outcome: Outcome::Denied, .. }));
    assert!(matches!(r.machine.phase(), Phase::Prompt { .. }), "yeni istem: {:?}", r.machine.phase());
    r.feed(Event::Submit, "nuxut");
    assert!(r.exited);
    let log = r.logged();
    assert_eq!(
        log[..4],
        [Logged::Create("ayse".into()), Logged::Denied("ayse".into()), Logged::Cancel, Logged::Create("ayse".into())]
    );
}

#[test]
fn fingerprint_info_then_password() {
    let mut r = Rig::new();
    r.feed(Event::SelectUser("parmak".into()), "");
    // Bilgi mesajı onaylandı, şimdi şifre istemi; mesaj hâlâ gösteriliyor.
    assert!(matches!(r.machine.phase(), Phase::Prompt { secret: true, .. }));
    assert_eq!(r.machine.notices()[0].text, "Parmağınızı okuyucuya koyun");
    r.feed(Event::Submit, "nuxut");
    assert!(r.exited);
}

#[test]
fn two_step_visible_code() {
    let mut r = Rig::new();
    r.feed(Event::SelectUser("kod".into()), "");
    r.feed(Event::Submit, "nuxut");
    assert!(matches!(r.machine.phase(), Phase::Prompt { secret: false, text } if text == "Doğrulama kodu:"));
    r.feed(Event::Submit, "123456");
    assert!(r.exited);
}

#[test]
fn locked_account_shows_pam_message() {
    let mut r = Rig::new();
    r.feed(Event::SelectUser("kilitli".into()), "");
    r.feed(Event::Submit, "nuxut");
    let AuthStatus::Failed { messages, .. } = r.machine.status() else { panic!("{:?}", r.machine.status()) };
    assert_eq!(messages[0].text, "Hesap 10 dakika kilitli");
    assert!(!r.exited);
}

#[test]
fn switching_user_cancels_previous_session() {
    let mut r = Rig::new();
    r.feed(Event::SelectUser("ayse".into()), "");
    r.feed(Event::SelectUser("kod".into()), "");
    assert_eq!(r.machine.user(), Some("kod"));
    r.feed(Event::Submit, "nuxut");
    r.feed(Event::Submit, "123456");
    assert!(r.exited);
    let log = r.logged();
    assert_eq!(log[..3], [Logged::Create("ayse".into()), Logged::Cancel, Logged::Create("kod".into())]);
}
