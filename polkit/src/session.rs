//! Diyaloğun saf durum makinesi: yardımcı olayları ve kullanıcı eylemleri → eylemler.
//! Wayland, soket ve zamanlayıcı bilmez; tüm kurallar burada ve testli.
//!
//! Değişmezler:
//! - [`Action::Finish`]`(Success)` yalnızca güncel yardımcı oturumu `SUCCESS` yazınca üretilir
//!   (karar polkitd'de; `SUCCESS` yardımcının yanıtı polkitd'ye ilettiği anlamına gelir).
//! - Eski nesil yardımcının (kimlik değişti, yeniden deneme) olayları yok sayılır.
//! - [`Action::SendResponse`] yalnızca `Prompt` evresinde ve oturum başına istem başına bir kez.
//! - Kurallara uymayan satır → bağlantı kapanır, sonuç `Failed` (fail-closed).
//! - Kullanıcı parola gönderdikten sonra `FAILURE`/EOF → hata gösterilir, alan temizlenir,
//!   yeni yardımcı oturumu açılır (polkit ajanlarının davranışı). Parola sorulmadan art arda
//!   [`MAX_UNPROMPTED_FAILURES`] başarısızlık → `Failed` (bozuk kurulumda döngü yok).

use crate::helper::HelperEvent;
use crate::protocol::HelperMessage;
use crate::request::DialogOutcome;
use crate::text;

/// Kullanıcı yanıtı olmadan biten ardışık yardımcı oturumu sınırı.
pub const MAX_UNPROMPTED_FAILURES: u32 = 2;

/// Yardımcı oturumu nesli.
pub type Generation = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Yardımcı çalışıyor, istem bekleniyor.
    Waiting,
    /// Yardımcı yanıt istiyor; giriş açık.
    Prompt { text: String, echo: bool },
    /// Yanıt gönderildi, sonuç bekleniyor.
    Verifying,
    Done(DialogOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Start,
    Helper { generation: Generation, event: HelperEvent },
    /// Yardımcıya bağlanılamadı / başlatılamadı.
    HelperStartFailed { generation: Generation },
    /// Enter ya da Doğrula.
    Submit,
    /// Esc, İptal, zaman aşımı, ajan iptali (SIGTERM), stdin EOF.
    Cancel,
    SelectIdentity(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bu nesille, bu kimlik için yeni yardımcı oturumu (eskisi varsa önce kapatılır).
    StartHelper { generation: Generation, identity: usize },
    StopHelper,
    /// Giriş alanının içeriğini güncel yardımcıya yanıt olarak yaz, sonra alanı temizle.
    SendResponse,
    ClearInput,
    Shake,
    Finish(DialogOutcome),
}

/// Hata satırı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorText {
    /// PAM modülünün metni (temizlenmiş).
    Pam(String),
    /// Genel "parola yanlış".
    WrongPassword,
}

#[derive(Debug)]
pub struct Session {
    phase: Phase,
    identities: usize,
    identity: usize,
    generation: Generation,
    /// Bu yardımcı oturumunda yanıt gönderildi mi.
    responded: bool,
    unprompted_failures: u32,
    /// Bu oturumda gelen son PAM hata metni.
    pam_error: Option<String>,
    error: Option<ErrorText>,
    info: Option<String>,
    failed_attempts: u32,
}

impl Session {
    pub fn new(identities: usize, identity: usize) -> Self {
        Self {
            phase: Phase::Waiting,
            identities,
            identity: identity.min(identities.saturating_sub(1)),
            generation: 0,
            responded: false,
            unprompted_failures: 0,
            pam_error: None,
            error: None,
            info: None,
            failed_attempts: 0,
        }
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn identity(&self) -> usize {
        self.identity
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }

    pub fn error(&self) -> Option<&ErrorText> {
        self.error.as_ref()
    }

    pub fn info(&self) -> Option<&str> {
        self.info.as_deref()
    }

    pub fn failed_attempts(&self) -> u32 {
        self.failed_attempts
    }

    /// Giriş alanı yazılabilir mi.
    pub fn accepts_input(&self) -> bool {
        matches!(self.phase, Phase::Prompt { .. })
    }

    pub fn is_done(&self) -> bool {
        matches!(self.phase, Phase::Done(_))
    }

    fn restart(&mut self) -> Action {
        self.generation += 1;
        self.responded = false;
        self.pam_error = None;
        self.phase = Phase::Waiting;
        Action::StartHelper {
            generation: self.generation,
            identity: self.identity,
        }
    }

    fn finish(&mut self, outcome: DialogOutcome, stop: bool) -> Vec<Action> {
        self.phase = Phase::Done(outcome);
        let mut v = Vec::new();
        if stop {
            v.push(Action::StopHelper);
        }
        v.push(Action::ClearInput);
        v.push(Action::Finish(outcome));
        v
    }

    /// Yardımcı oturumu kullanıcı yanıtı olmadan bitti.
    fn unprompted_failure(&mut self) -> Vec<Action> {
        self.unprompted_failures += 1;
        if self.unprompted_failures >= MAX_UNPROMPTED_FAILURES {
            return self.finish(DialogOutcome::Failed, true);
        }
        vec![Action::StopHelper, self.restart()]
    }

    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        if self.is_done() {
            return Vec::new();
        }
        match event {
            Event::Start if self.generation == 0 => vec![self.restart()],
            Event::Start => Vec::new(),
            Event::Cancel => self.finish(DialogOutcome::Cancelled, true),
            Event::SelectIdentity(i) => {
                if i >= self.identities || i == self.identity {
                    return Vec::new();
                }
                self.identity = i;
                self.error = None;
                self.info = None;
                self.unprompted_failures = 0;
                vec![Action::StopHelper, Action::ClearInput, self.restart()]
            }
            Event::Submit => {
                if !matches!(self.phase, Phase::Prompt { .. }) {
                    return Vec::new();
                }
                self.phase = Phase::Verifying;
                self.responded = true;
                self.error = None;
                vec![Action::SendResponse]
            }
            Event::HelperStartFailed { generation } => {
                if generation != self.generation {
                    return Vec::new();
                }
                self.unprompted_failure()
            }
            Event::Helper { generation, event } => {
                if generation != self.generation {
                    return Vec::new();
                }
                self.helper(event)
            }
        }
    }

    fn helper(&mut self, event: HelperEvent) -> Vec<Action> {
        match event {
            HelperEvent::Message(HelperMessage::Prompt { text: t, echo }) => {
                // Yeni istem: önceki yanıt kabul edildi (çok adımlı PAM) ya da ilk istem.
                self.phase = Phase::Prompt {
                    text: text::sanitize(&t, text::MAX_PAM_CHARS),
                    echo,
                };
                vec![Action::ClearInput]
            }
            HelperEvent::Message(HelperMessage::ErrorMsg(t)) => {
                let t = text::sanitize(&t, text::MAX_PAM_CHARS);
                if !t.is_empty() {
                    self.pam_error = Some(t.clone());
                    self.error = Some(ErrorText::Pam(t));
                }
                Vec::new()
            }
            HelperEvent::Message(HelperMessage::TextInfo(t)) => {
                let t = text::sanitize(&t, text::MAX_PAM_CHARS);
                self.info = (!t.is_empty()).then_some(t);
                Vec::new()
            }
            HelperEvent::Message(HelperMessage::Success) => {
                self.finish(DialogOutcome::Success, false)
            }
            HelperEvent::Protocol(e) => {
                tracing::warn!(?e, "yardımcı kurallara uymayan satır yazdı");
                self.finish(DialogOutcome::Failed, true)
            }
            HelperEvent::Message(HelperMessage::Failure) | HelperEvent::Closed => {
                if !self.responded {
                    return self.unprompted_failure();
                }
                self.unprompted_failures = 0;
                self.failed_attempts += 1;
                self.error = Some(match self.pam_error.take() {
                    Some(t) => ErrorText::Pam(t),
                    None => ErrorText::WrongPassword,
                });
                self.info = None;
                vec![
                    Action::StopHelper,
                    Action::ClearInput,
                    Action::Shake,
                    self.restart(),
                ]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ParseError;

    fn msg(generation: Generation, m: HelperMessage) -> Event {
        Event::Helper {
            generation,
            event: HelperEvent::Message(m),
        }
    }

    fn prompt() -> HelperMessage {
        HelperMessage::Prompt {
            text: "Password: ".into(),
            echo: false,
        }
    }

    fn started(n: usize, id: usize) -> Session {
        let mut s = Session::new(n, id);
        assert_eq!(
            s.handle(Event::Start),
            [Action::StartHelper {
                generation: 1,
                identity: id
            }]
        );
        assert!(s.handle(Event::Start).is_empty());
        s
    }

    #[test]
    fn success_path() {
        let mut s = started(1, 0);
        assert!(!s.accepts_input());
        assert!(s.handle(Event::Submit).is_empty(), "istem yokken gönderilmez");
        assert_eq!(s.handle(msg(1, prompt())), [Action::ClearInput]);
        assert!(s.accepts_input());
        assert_eq!(s.handle(Event::Submit), [Action::SendResponse]);
        assert!(s.handle(Event::Submit).is_empty(), "doğrularken ikinci yanıt yok");
        assert_eq!(s.phase(), &Phase::Verifying);
        assert_eq!(
            s.handle(msg(1, HelperMessage::Success)),
            [Action::ClearInput, Action::Finish(DialogOutcome::Success)]
        );
        assert!(s.handle(Event::Cancel).is_empty());
        assert!(s.handle(msg(1, prompt())).is_empty());
    }

    #[test]
    fn wrong_password_retries_with_new_helper() {
        let mut s = started(1, 0);
        s.handle(msg(1, prompt()));
        s.handle(Event::Submit);
        assert_eq!(
            s.handle(msg(1, HelperMessage::Failure)),
            [
                Action::StopHelper,
                Action::ClearInput,
                Action::Shake,
                Action::StartHelper {
                    generation: 2,
                    identity: 0
                }
            ]
        );
        assert_eq!(s.error(), Some(&ErrorText::WrongPassword));
        assert_eq!(s.failed_attempts(), 1);
        // Eski oturumun geç olayları yok sayılır (SUCCESS dahil).
        assert!(s.handle(msg(1, HelperMessage::Success)).is_empty());
        assert!(!s.is_done());
        s.handle(msg(2, prompt()));
        s.handle(Event::Submit);
        assert_eq!(s.error(), None, "yeni denemede hata satırı silinir");
        assert!(s.handle(msg(2, HelperMessage::Success)).contains(&Action::Finish(DialogOutcome::Success)));
    }

    #[test]
    fn pam_error_text_wins_and_eof_after_response_is_failure() {
        let mut s = started(1, 0);
        s.handle(msg(1, prompt()));
        s.handle(Event::Submit);
        s.handle(msg(1, HelperMessage::ErrorMsg("Hesap\u{202E} kilitli\n".into())));
        let a = s.handle(Event::Helper {
            generation: 1,
            event: HelperEvent::Closed,
        });
        assert!(a.contains(&Action::Shake));
        assert_eq!(s.error(), Some(&ErrorText::Pam("Hesap kilitli".into())));
    }

    #[test]
    fn info_and_multi_step_prompts() {
        let mut s = started(1, 0);
        s.handle(msg(1, HelperMessage::TextInfo("Parmağınızı okutun".into())));
        assert_eq!(s.info(), Some("Parmağınızı okutun"));
        s.handle(msg(1, prompt()));
        s.handle(Event::Submit);
        // İkinci adım (ör. OTP, görünür).
        s.handle(msg(
            1,
            HelperMessage::Prompt {
                text: "Kod:".into(),
                echo: true,
            },
        ));
        assert_eq!(
            s.phase(),
            &Phase::Prompt {
                text: "Kod:".into(),
                echo: true
            }
        );
        assert_eq!(s.handle(Event::Submit), [Action::SendResponse]);
        // Parmak izi gibi istemsiz başarı da geçerli.
        let mut s = started(1, 0);
        assert!(s.handle(msg(1, HelperMessage::Success)).contains(&Action::Finish(DialogOutcome::Success)));
    }

    #[test]
    fn protocol_error_fails_closed() {
        let mut s = started(1, 0);
        s.handle(msg(1, prompt()));
        s.handle(Event::Submit);
        let a = s.handle(Event::Helper {
            generation: 1,
            event: HelperEvent::Protocol(ParseError::Unknown),
        });
        assert_eq!(a, [Action::StopHelper, Action::ClearInput, Action::Finish(DialogOutcome::Failed)]);
    }

    #[test]
    fn unprompted_failures_are_bounded() {
        let mut s = started(1, 0);
        assert_eq!(
            s.handle(Event::HelperStartFailed { generation: 1 }),
            [Action::StopHelper, Action::StartHelper { generation: 2, identity: 0 }]
        );
        assert_eq!(
            s.handle(Event::Helper {
                generation: 2,
                event: HelperEvent::Message(HelperMessage::Failure)
            }),
            [Action::StopHelper, Action::ClearInput, Action::Finish(DialogOutcome::Failed)]
        );
        // Eski nesilin başlatma hatası sayılmaz.
        let mut s = started(1, 0);
        assert!(s.handle(Event::HelperStartFailed { generation: 7 }).is_empty());
    }

    #[test]
    fn cancel_and_identity_switch() {
        let mut s = started(3, 1);
        s.handle(msg(1, prompt()));
        assert!(s.handle(Event::SelectIdentity(1)).is_empty(), "aynı kimlik");
        assert!(s.handle(Event::SelectIdentity(3)).is_empty(), "aralık dışı");
        assert_eq!(
            s.handle(Event::SelectIdentity(2)),
            [
                Action::StopHelper,
                Action::ClearInput,
                Action::StartHelper {
                    generation: 2,
                    identity: 2
                }
            ]
        );
        assert!(!s.accepts_input());
        assert!(s.handle(msg(1, prompt())).is_empty());
        assert_eq!(
            s.handle(Event::Cancel),
            [Action::StopHelper, Action::ClearInput, Action::Finish(DialogOutcome::Cancelled)]
        );
        assert_eq!(s.phase(), &Phase::Done(DialogOutcome::Cancelled));
        assert!(s.handle(Event::Submit).is_empty());
    }

    #[test]
    fn identity_clamped() {
        assert_eq!(Session::new(2, 9).identity(), 1);
        assert_eq!(Session::new(0, 0).identity(), 0);
    }
}
