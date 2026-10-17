//! Başsız diyalog (YALNIZCA `fake-polkit` test derlemesi): arayüz yerine
//! `NUXUT_POLKIT_TEST_PASSWORDS` (virgülle ayrık) sırasıyla yanıt verir; liste bitince istemde
//! bekler (iptal/öldürme senaryoları). Aynı [`Session`] makinesi ve [`HelperSession`] kullanılır;
//! yalnızca girdi kaynağı farklıdır.
//!
//! Gerçek yardımcıya asla bağlanmaz: yollar sahte soket değilse (`setuid` açık ya da sistem
//! soketi) hemen `Failed`.

use std::io::Read;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nuxut_pam::SecretBuf;

use crate::dialog::Setup;
use crate::helper::{HelperEvent, HelperSession, SYSTEM_SOCKET};
use crate::request::DialogOutcome;
use crate::session::{Action, Event, Phase, Session};

enum In {
    Helper(u64, HelperEvent),
    StdinClosed,
}

pub fn run(mut setup: Setup) -> DialogOutcome {
    if setup.paths.setuid.is_some() || setup.paths.socket == std::path::Path::new(SYSTEM_SOCKET) {
        tracing::error!("başsız diyalog yalnızca sahte yardımcıyla çalışır");
        return DialogOutcome::Failed;
    }
    let mut passwords: std::collections::VecDeque<String> = std::env::var("NUXUT_POLKIT_TEST_PASSWORDS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let (tx, rx) = mpsc::channel::<In>();
    if let Some(mut stdin) = setup.stdin.take() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            while matches!(stdin.read(&mut buf), Ok(n) if n > 0) {}
            let _ = tx.send(In::StdinClosed);
        });
    }
    let timeout = Duration::from_secs(setup.request.timeout_s.max(1));
    let mut session = Session::new(setup.identities.len(), setup.default_identity);
    let mut helper: Option<HelperSession> = None;
    let mut queue = session.handle(Event::Start);
    let mut deadline = Instant::now() + timeout;
    loop {
        while let Some(action) = (!queue.is_empty()).then(|| queue.remove(0)) {
            match action {
                Action::StartHelper { generation, identity } => {
                    helper = None;
                    let user = &setup.identities[identity].name;
                    let tx = tx.clone();
                    match HelperSession::start(&setup.paths, user, &setup.request.cookie, move |e| {
                        let _ = tx.send(In::Helper(generation, e));
                    }) {
                        Ok(h) => helper = Some(h),
                        Err(_) => queue.extend(session.handle(Event::HelperStartFailed { generation })),
                    }
                }
                Action::StopHelper => helper = None,
                Action::SendResponse => {
                    let generation = session.generation();
                    let ok = passwords.pop_front().is_some_and(|pw| {
                        let mut s = SecretBuf::new();
                        s.push_str(&pw).is_ok() && helper.as_mut().is_some_and(|h| h.respond(&s).is_ok())
                    });
                    if !ok {
                        queue.extend(session.handle(Event::Helper {
                            generation,
                            event: HelperEvent::Closed,
                        }));
                    }
                }
                Action::ClearInput | Action::Shake => {}
                Action::Finish(outcome) => return outcome,
            }
        }
        // İstemde ve sırada parola varsa "kullanıcı" gönderir.
        if matches!(session.phase(), Phase::Prompt { .. }) && !passwords.is_empty() {
            deadline = Instant::now() + timeout;
            queue.extend(session.handle(Event::Submit));
            continue;
        }
        let wait = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(In::Helper(generation, event)) => queue.extend(session.handle(Event::Helper { generation, event })),
            Ok(In::StdinClosed) => queue.extend(session.handle(Event::Cancel)),
            Err(mpsc::RecvTimeoutError::Timeout) => queue.extend(session.handle(Event::Cancel)),
            Err(mpsc::RecvTimeoutError::Disconnected) => return DialogOutcome::Failed,
        }
    }
}
