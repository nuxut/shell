//! Girişin saf durum makinesi: kullanıcı/greetd olayı → eylemler. Soket, Wayland ve
//! zamanlayıcı bilmez; testlidir.
//!
//! greetd protokolü katı istek-yanıttır: aynı anda tek istek uçuşta olur
//! ([`Machine::in_flight`]). Kullanıcı değişimi uçuştaki yanıtı bekler; yanıt yok sayılır,
//! oturum iptal edilir ve yeni kullanıcı için açılır.
//!
//! Değişmezler:
//! - [`Action::StartSession`] yalnızca bir yanıtın (`post_auth_message_response` ya da
//!   istemsiz geçen `create_session`) `success` dönmesiyle, iptal beklenmiyorken üretilir.
//! - Şifre yalnızca açık bir istem varken ve gecikme dolmuşken gönderilir.
//! - Kimlik hatası sonrası oturum iptal edilip aynı kullanıcı için yeniden açılır; greetd'nin
//!   açıklaması gösterilmez (`pam_authenticate: AUTH_ERR` gibi iç metin), yalnızca PAM'in
//!   kendi mesajı (faillock) ya da "Yanlış şifre".

use std::time::{Duration, Instant};

use nuxut_lock::state::{AuthStatus, backoff_for};
use nuxut_pam::{MessageKind, Outcome, PamMessage};

use crate::protocol::{AuthMessageType, ErrorType, Request, Response};

/// Bir deneme boyunca biriken PAM mesajı sayısı üst sınırı.
pub const MAX_MESSAGES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Henüz kullanıcı seçilmedi.
    Idle,
    /// greetd'den yanıt bekleniyor (oturum açılıyor, yanıt doğrulanıyor, parmak izi okuyucusu
    /// bekleniyor…).
    Busy,
    /// PAM bir yanıt istiyor.
    Prompt { secret: bool, text: String },
    /// Doğrulandı; `start_session` gönderildi.
    Starting,
    /// greetd'ye ulaşılamadı ya da iç hata; Enter yeniden dener.
    Stopped,
    /// Oturum başlatılacak: greeter çıkmalı.
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Kullanıcı seçildi (ilk seçim de).
    SelectUser(String),
    /// Enter: açık isteme yanıt ya da durmuşsa yeniden dene.
    Submit,
    Response(Response),
    Disconnected,
    /// Gecikme zamanlayıcısı.
    Tick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Send(Request),
    /// Alanın içeriğini isteme yanıt olarak gönder (gizli ya da yankılı).
    SendInput,
    /// Bilgi/hata mesajını onayla (`response: null`).
    Acknowledge,
    /// Seçili oturumun komutuyla `start_session` gönder.
    StartSession,
    ClearInput,
    Shake,
    WakeAfter(Duration),
    /// Oturum başlıyor: greeter çıkar (greetd oturumu greeter bitince başlatır).
    Exit,
}

#[derive(Debug)]
pub struct Machine {
    phase: Phase,
    user: Option<String>,
    /// Bu kullanıcıya geçilecek (uçuştaki yanıt ya da iptal bitince).
    switch_to: Option<String>,
    /// Gönderilmiş, yanıtı beklenen istek var.
    in_flight: bool,
    /// Uçuştaki istek `cancel_session`.
    cancelling: bool,
    status: AuthStatus,
    /// Bu denemenin PAM bilgi/hata mesajları (en çok [`MAX_MESSAGES`]).
    messages: Vec<PamMessage>,
    failures: u32,
    backoff_until: Option<Instant>,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            user: None,
            switch_to: None,
            in_flight: false,
            cancelling: false,
            status: AuthStatus::Idle,
            messages: Vec::new(),
            failures: 0,
            backoff_until: None,
        }
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    pub fn status(&self) -> &AuthStatus {
        &self.status
    }

    /// Bilgi mesajları (başarısızlık dışında gösterilenler; ör. parmak izi istemi).
    pub fn notices(&self) -> &[PamMessage] {
        if matches!(self.status, AuthStatus::Failed { .. }) { &[] } else { &self.messages }
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }

    pub fn backoff_remaining(&self, now: Instant) -> Option<Duration> {
        self.backoff_until
            .and_then(|t| t.checked_duration_since(now))
            .filter(|d| !d.is_zero())
    }

    /// Alan yazılabilir mi: açık istem var, iptal/geçiş beklenmiyor, gecikme yok.
    pub fn accepts_input(&self, now: Instant) -> bool {
        matches!(self.phase, Phase::Prompt { .. })
            && !self.in_flight
            && self.switch_to.is_none()
            && self.backoff_remaining(now).is_none()
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Action> {
        match event {
            Event::SelectUser(user) => self.select(user),
            Event::Submit => self.submit(now),
            Event::Response(r) => self.response(r, now),
            Event::Disconnected => {
                self.in_flight = false;
                self.cancelling = false;
                if self.phase == Phase::Done {
                    return vec![];
                }
                // Kopan bağlantıyla oturum da gitti: geçiş bekleniyorsa yeni bağlantıda aç.
                if let Some(user) = self.switch_to.take() {
                    return self.create(user);
                }
                self.phase = Phase::Stopped;
                self.status = AuthStatus::Failed { outcome: Outcome::Unavailable, messages: vec![] };
                vec![Action::ClearInput]
            }
            Event::Tick => vec![],
        }
    }

    fn select(&mut self, user: String) -> Vec<Action> {
        if matches!(self.phase, Phase::Starting | Phase::Done) {
            return vec![];
        }
        if self.user.as_deref() == Some(&user) && self.switch_to.is_none() && self.phase != Phase::Stopped {
            return vec![];
        }
        self.status = AuthStatus::Idle;
        self.messages.clear();
        // Kaba kuvvet sayacı kullanıcıdan bağımsız (makinenin başındaki kişiye uygulanır).
        if self.in_flight {
            // Yanıt gelince iptal edilip yeni kullanıcı açılır.
            self.switch_to = Some(user);
            self.phase = Phase::Busy;
            return vec![Action::ClearInput];
        }
        match self.phase {
            Phase::Idle | Phase::Stopped => {
                let mut v = self.create(user);
                v.insert(0, Action::ClearInput);
                v
            }
            _ => {
                self.switch_to = Some(user);
                self.phase = Phase::Busy;
                vec![Action::ClearInput, self.cancel()]
            }
        }
    }

    /// Yeni oturum: önceki denemenin mesajları atılır (istemden önce gelen faillock mesajı bu
    /// denemeye aittir ve hata olursa durum satırında gösterilir).
    fn create(&mut self, user: String) -> Vec<Action> {
        self.messages.clear();
        self.user = Some(user.clone());
        self.phase = Phase::Busy;
        self.in_flight = true;
        self.cancelling = false;
        vec![Action::Send(Request::CreateSession { username: user })]
    }

    fn cancel(&mut self) -> Action {
        self.in_flight = true;
        self.cancelling = true;
        Action::Send(Request::CancelSession)
    }

    fn submit(&mut self, now: Instant) -> Vec<Action> {
        match self.phase {
            Phase::Prompt { .. } if self.accepts_input(now) => {
                self.phase = Phase::Busy;
                self.in_flight = true;
                self.status = AuthStatus::Verifying;
                vec![Action::SendInput, Action::ClearInput]
            }
            Phase::Stopped if !self.in_flight => match self.user.clone() {
                Some(user) => {
                    self.status = AuthStatus::Idle;
                    // Yarım kalmış oturum varsa greetd "zaten var" der: önce iptal.
                    self.switch_to = Some(user);
                    self.phase = Phase::Busy;
                    vec![self.cancel()]
                }
                None => vec![],
            },
            _ => vec![],
        }
    }

    fn response(&mut self, response: Response, now: Instant) -> Vec<Action> {
        if !self.in_flight {
            tracing::warn!("istenmemiş greetd yanıtı yok sayıldı");
            return vec![];
        }
        self.in_flight = false;
        if self.cancelling {
            // İptalin sonucu ne olursa olsun (oturum yoksa hata döner) yeni oturum açılır.
            self.cancelling = false;
            return match self.switch_to.take().or_else(|| self.user.clone()) {
                Some(user) => self.create(user),
                None => {
                    self.phase = Phase::Idle;
                    vec![]
                }
            };
        }
        if self.switch_to.is_some() {
            // Eski kullanıcının yanıtı: içerik yok sayılır, oturum iptal edilir.
            return vec![self.cancel()];
        }
        match response {
            Response::AuthMessage { auth_message_type, auth_message } => match auth_message_type {
                AuthMessageType::Secret | AuthMessageType::Visible => {
                    self.phase = Phase::Prompt {
                        secret: auth_message_type == AuthMessageType::Secret,
                        text: auth_message,
                    };
                    // Doğrulama sürerken yeni istem: önceki deneme bitti (ör. OTP adımı).
                    if self.status == AuthStatus::Verifying {
                        self.status = AuthStatus::Idle;
                    }
                    vec![]
                }
                AuthMessageType::Info | AuthMessageType::Error => {
                    if !auth_message.is_empty() && self.messages.len() < MAX_MESSAGES {
                        self.messages.push(PamMessage {
                            kind: if auth_message_type == AuthMessageType::Error {
                                MessageKind::Error
                            } else {
                                MessageKind::Info
                            },
                            text: auth_message,
                        });
                    }
                    self.phase = Phase::Busy;
                    self.in_flight = true;
                    vec![Action::Acknowledge]
                }
            },
            Response::Success => match self.phase {
                Phase::Starting => {
                    self.phase = Phase::Done;
                    vec![Action::Exit]
                }
                _ => {
                    self.phase = Phase::Starting;
                    self.in_flight = true;
                    self.status = AuthStatus::Verifying;
                    self.failures = 0;
                    vec![Action::StartSession]
                }
            },
            Response::Error { error_type, description } => {
                let messages = std::mem::take(&mut self.messages);
                let mut actions = vec![Action::ClearInput];
                match error_type {
                    ErrorType::AuthError => {
                        self.failures = self.failures.saturating_add(1);
                        self.status = AuthStatus::Failed { outcome: Outcome::Denied, messages };
                        actions.push(Action::Shake);
                        if let Some(delay) = backoff_for(self.failures) {
                            self.backoff_until = Some(now + delay);
                            actions.push(Action::WakeAfter(delay));
                        }
                        // Aynı kullanıcı için yeni oturum (taze istem).
                        self.switch_to = self.user.clone();
                        self.phase = Phase::Busy;
                        actions.push(self.cancel());
                    }
                    ErrorType::Error => {
                        // İç metin yalnızca log'a (kullanıcı adı/yol taşıyabilir).
                        tracing::warn!(%description, "greetd hatası");
                        self.status = AuthStatus::Failed { outcome: Outcome::Error(0), messages };
                        self.phase = Phase::Stopped;
                    }
                }
                actions
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(text: &str) -> Response {
        Response::AuthMessage { auth_message_type: AuthMessageType::Secret, auth_message: text.into() }
    }
    fn info(text: &str) -> Response {
        Response::AuthMessage { auth_message_type: AuthMessageType::Info, auth_message: text.into() }
    }
    fn auth_error() -> Response {
        Response::Error { error_type: ErrorType::AuthError, description: "pam_authenticate: AUTH_ERR".into() }
    }
    fn create(user: &str) -> Action {
        Action::Send(Request::CreateSession { username: user.into() })
    }

    fn prompted(user: &str) -> (Machine, Instant) {
        let now = Instant::now();
        let mut m = Machine::new();
        assert_eq!(m.handle(Event::SelectUser(user.into()), now), [Action::ClearInput, create(user)]);
        assert!(!m.accepts_input(now));
        assert!(m.handle(Event::Response(secret("Password: ")), now).is_empty());
        assert!(m.accepts_input(now));
        (m, now)
    }

    #[test]
    fn happy_path_starts_session_then_exits() {
        let (mut m, now) = prompted("ayse");
        assert_eq!(m.handle(Event::Submit, now), [Action::SendInput, Action::ClearInput]);
        assert_eq!(m.status(), &AuthStatus::Verifying);
        // Uçuşta ikinci Enter şifreyi yeniden göndermez.
        assert!(m.handle(Event::Submit, now).is_empty());
        assert_eq!(m.handle(Event::Response(Response::Success), now), [Action::StartSession]);
        assert_eq!(m.phase(), &Phase::Starting);
        assert_eq!(m.handle(Event::Response(Response::Success), now), [Action::Exit]);
        assert_eq!(m.phase(), &Phase::Done);
        assert!(m.handle(Event::SelectUser("x".into()), now).is_empty());
    }

    #[test]
    fn passwordless_account_starts_directly() {
        let now = Instant::now();
        let mut m = Machine::new();
        m.handle(Event::SelectUser("misafir".into()), now);
        assert_eq!(m.handle(Event::Response(Response::Success), now), [Action::StartSession]);
    }

    #[test]
    fn auth_error_shakes_cancels_and_recreates_for_same_user() {
        let (mut m, now) = prompted("ayse");
        m.handle(Event::Submit, now);
        let a = m.handle(Event::Response(auth_error()), now);
        assert_eq!(a, [Action::ClearInput, Action::Shake, Action::Send(Request::CancelSession)]);
        assert!(matches!(m.status(), AuthStatus::Failed { outcome: Outcome::Denied, messages } if messages.is_empty()));
        // İptal sonucu hata olsa da aynı kullanıcı için yeni oturum.
        let a = m.handle(
            Event::Response(Response::Error { error_type: ErrorType::Error, description: "no session".into() }),
            now,
        );
        assert_eq!(a, [create("ayse")]);
        m.handle(Event::Response(secret("Password: ")), now);
        assert!(m.accepts_input(now));
        // Hata satırı yeni istemde de kalır (kullanıcı neden yeniden sorulduğunu görür).
        assert!(matches!(m.status(), AuthStatus::Failed { .. }));
    }

    #[test]
    fn faillock_message_replaces_generic_error() {
        let (mut m, now) = prompted("ayse");
        m.handle(Event::Submit, now);
        assert_eq!(m.handle(Event::Response(Response::AuthMessage {
            auth_message_type: AuthMessageType::Error,
            auth_message: "Hesap kilitli".into(),
        }), now), [Action::Acknowledge]);
        m.handle(Event::Response(auth_error()), now);
        let AuthStatus::Failed { messages, .. } = m.status() else { panic!() };
        assert_eq!(messages[0].text, "Hesap kilitli");
    }

    #[test]
    fn backoff_blocks_submit_until_expired() {
        let (mut m, now) = prompted("ayse");
        let mut t = now;
        for i in 0..4 {
            m.handle(Event::Submit, t);
            let a = m.handle(Event::Response(auth_error()), t);
            assert_eq!(a.iter().any(|a| matches!(a, Action::WakeAfter(_))), i >= 2, "deneme {i}");
            m.handle(Event::Response(Response::Success), t); // iptal yanıtı
            m.handle(Event::Response(secret("Password: ")), t);
            if i >= 2 {
                assert!(!m.accepts_input(t));
                assert!(m.handle(Event::Submit, t).is_empty());
                t += m.backoff_remaining(t).unwrap();
            }
            assert!(m.accepts_input(t));
        }
    }

    #[test]
    fn info_messages_are_acknowledged_and_shown() {
        let now = Instant::now();
        let mut m = Machine::new();
        m.handle(Event::SelectUser("ayse".into()), now);
        assert_eq!(m.handle(Event::Response(info("Parmağınızı okuyucuya koyun")), now), [Action::Acknowledge]);
        assert!(m.in_flight() && !m.accepts_input(now));
        assert_eq!(m.notices().len(), 1);
        for _ in 0..10 {
            m.handle(Event::Response(info("tekrar")), now);
        }
        assert_eq!(m.notices().len(), MAX_MESSAGES);
    }

    #[test]
    fn switching_user_waits_for_in_flight_reply() {
        let now = Instant::now();
        let mut m = Machine::new();
        m.handle(Event::SelectUser("ayse".into()), now);
        // create_session uçuşta: geçiş bekler.
        assert_eq!(m.handle(Event::SelectUser("mehmet".into()), now), [Action::ClearInput]);
        // Eski kullanıcının istemi yok sayılır, iptal gönderilir.
        assert_eq!(m.handle(Event::Response(secret("Password: ")), now), [Action::Send(Request::CancelSession)]);
        assert!(!m.accepts_input(now));
        assert_eq!(m.handle(Event::Response(Response::Success), now), [create("mehmet")]);
        assert_eq!(m.user(), Some("mehmet"));
        // Eski kullanıcının "başarı"sı oturum başlatmaz: SelectUser→başarı yarışı.
        let (mut m, now) = prompted("ayse");
        m.handle(Event::Submit, now);
        m.handle(Event::SelectUser("mehmet".into()), now);
        assert_eq!(m.handle(Event::Response(Response::Success), now), [Action::Send(Request::CancelSession)]);
        assert_eq!(m.handle(Event::Response(Response::Success), now), [create("mehmet")]);
    }

    #[test]
    fn switching_user_at_prompt_cancels_first() {
        let (mut m, now) = prompted("ayse");
        assert_eq!(
            m.handle(Event::SelectUser("mehmet".into()), now),
            [Action::ClearInput, Action::Send(Request::CancelSession)]
        );
        assert!(m.handle(Event::Submit, now).is_empty(), "iptal sürerken şifre gitmez");
        assert_eq!(m.handle(Event::Response(Response::Success), now), [create("mehmet")]);
        // Aynı kullanıcıyı yeniden seçmek bir şey yapmaz.
        m.handle(Event::Response(secret("Password: ")), now);
        assert!(m.handle(Event::SelectUser("mehmet".into()), now).is_empty());
    }

    #[test]
    fn disconnect_stops_and_enter_retries_with_cancel() {
        let (mut m, now) = prompted("ayse");
        m.handle(Event::Submit, now);
        assert_eq!(m.handle(Event::Disconnected, now), [Action::ClearInput]);
        assert_eq!(m.phase(), &Phase::Stopped);
        assert!(matches!(m.status(), AuthStatus::Failed { outcome: Outcome::Unavailable, .. }));
        assert_eq!(m.handle(Event::Submit, now), [Action::Send(Request::CancelSession)]);
        assert_eq!(m.handle(Event::Response(Response::Success), now), [create("ayse")]);
    }

    #[test]
    fn start_failure_stops_without_exit() {
        let (mut m, now) = prompted("ayse");
        m.handle(Event::Submit, now);
        m.handle(Event::Response(Response::Success), now);
        let a = m.handle(
            Event::Response(Response::Error { error_type: ErrorType::Error, description: "exec".into() }),
            now,
        );
        assert_eq!(a, [Action::ClearInput]);
        assert_eq!(m.phase(), &Phase::Stopped);
        assert!(!a.contains(&Action::Exit));
    }

    #[test]
    fn unsolicited_reply_ignored() {
        let mut m = Machine::new();
        assert!(m.handle(Event::Response(Response::Success), Instant::now()).is_empty());
        assert_eq!(m.phase(), &Phase::Idle);
    }
}
