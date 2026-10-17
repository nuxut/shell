//! Kilidin saf durum makinesi: olay → eylemler. Wayland, PAM ve zamanlayıcı bilmez; tüm
//! güvenlik kuralları burada ve testlidir.
//!
//! Değişmezler:
//! - [`Action::Unlock`] yalnızca `Unlocking` evresinden (açma animasyonu bitince ya da zaman
//!   aşımında) ya da compositor kilidi kendisi bitirince (`finished`, `Locked` iken) üretilir.
//! - `Unlocking`'e yalnızca `Locked` iken ve yalnızca başarılı PAM sonucu
//!   ([`nuxut_pam::Outcome::Success`]) ya da parmak izi eşleşmesiyle girilir.
//! - [`Action::NotifyLocked`] (ready-fd, LockedHint) yalnızca `locked` geldikten VE tüm kilit
//!   yüzeylerinin ilk karesi sunulduktan sonra, bir kez üretilir; `finished` önce gelirse çıkış
//!   kodu [`EXIT_NOT_LOCKED`].
//! - `Locked`/`Unlocking`'te sonlandırma sinyali yok sayılır.

use std::time::{Duration, Instant};

use nuxut_pam::{AuthResult, Outcome, PamMessage};

/// Kilitlendi ve açıldı (ya da compositor kilidi kendisi bitirdi).
pub const EXIT_UNLOCKED: i32 = 0;
/// Kilit hiç olmadı: compositor reddetti ya da `locked`'tan önce sinyal geldi.
pub const EXIT_NOT_LOCKED: i32 = 1;

/// Bu kadar başarısız denemeden sonra artan gecikme başlar.
pub const FREE_ATTEMPTS: u32 = 3;
/// Gecikme üst sınırı.
pub const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Açma animasyonu bu sürede bitmezse (kare gelmiyor: DPMS kapalı) yine de açılır.
pub const UNLOCK_TIMEOUT: Duration = Duration::from_millis(400);
/// `locked`'tan sonra tüm kilit yüzeylerinin ilk karesi bu sürede sunulmazsa kilit "kurulu"
/// bildirilmez (ready-fd/LockedHint yazılmaz); oturum yine kilitli kalır.
pub const PRESENT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Henüz kilit istenmedi.
    Starting,
    /// `lock_session` gönderildi, `locked`/`finished` bekleniyor.
    Pending,
    Locked,
    /// Doğrulandı; açma animasyonu oynuyor.
    Unlocking,
    /// Süreç bu kodla çıkmalı.
    Done(i32),
}

/// Doğrulama denemesi kimliği; geç gelen sonuçları eşleştirmek için.
pub type AttemptId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Başlangıç: kilit iste.
    Start,
    CompositorLocked,
    CompositorFinished,
    /// Kullanıcı Enter'a bastı (şifre alanı boş da olabilir: PAM `nullok` politikası).
    Submit,
    /// Auth thread'inin sonucu. `None`: thread öldü/sonuç kayboldu → başarısız.
    AuthFinished {
        id: AttemptId,
        result: Option<AuthResult>,
    },
    FingerprintMatched,
    /// Açma animasyonu bitti ya da [`UNLOCK_TIMEOUT`] doldu.
    UnlockAnimationDone,
    /// SIGTERM/SIGINT/SIGHUP.
    Terminate,
    /// Zamanlayıcı (gecikme bitişi); `now` her olayda ayrıca verilir.
    Tick,
    /// O anki tüm kilit yüzeylerinin ilk karesi sunuldu ([`PresentTracker`], runtime `surface_presented`).
    Presented,
    /// [`Action::WaitPresented`] süresi doldu.
    PresentTimeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `ext_session_lock_manager_v1.lock`.
    LockSession,
    /// `locked` geldi VE tüm kilit yüzeylerinin ilk karesi sunuldu: ready-fd'ye yaz, logind
    /// `SetLockedHint(true)`.
    NotifyLocked,
    /// `locked` geldi ama ilk kareler henüz sunulmadı: bu süre sonra `PresentTimeout` gönder.
    WaitPresented(Duration),
    /// İlk kareler süresinde sunulmadı: kilit "kurulu" bildirilmedi (yalnızca log). Oturum
    /// kilitli kalır; kare sonradan gelirse bildirim o zaman yapılır.
    PresentTimedOut,
    /// Şifre alanının içeriğini [`nuxut_pam::SecretBuf`]'a al ve bu kimlikle doğrulamayı başlat.
    StartAuth(AttemptId),
    /// Şifre alanını sıfırla.
    ClearInput,
    /// Hatalı şifre geri bildirimi (sarsılma).
    Shake,
    /// Açma animasyonunu başlat; [`UNLOCK_TIMEOUT`] sonra `UnlockAnimationDone` gönder.
    StartUnlockAnimation,
    /// `unlock_and_destroy` + flush, logind `SetLockedHint(false)`.
    Unlock,
    /// Kilit nesnesini `destroy` et (yalnızca `locked` hiç gelmediyse).
    DestroyLock,
    /// Bu süre sonra `Tick` gönder.
    WakeAfter(Duration),
    /// Parmak izi doğrulamasını durdur (fprintd `VerifyStop` + `Release`).
    StopFingerprint,
    Exit(i32),
}

/// Arayüzün göstereceği doğrulama durumu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatus {
    Idle,
    Verifying,
    /// Son deneme başarısız; PAM mesajı varsa o gösterilir, yoksa "Yanlış şifre".
    Failed {
        outcome: Outcome,
        messages: Vec<PamMessage>,
    },
}

#[derive(Debug)]
pub struct Machine {
    phase: Phase,
    status: AuthStatus,
    verifying: Option<AttemptId>,
    next_id: AttemptId,
    failures: u32,
    backoff_until: Option<Instant>,
    /// İlk kareler sunuldu (`locked`'tan önce de gelebilir).
    presented: bool,
    /// `NotifyLocked` üretildi: kilit dışarıya "kurulu" bildirildi.
    confirmed: bool,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub fn new() -> Self {
        Self {
            phase: Phase::Starting,
            status: AuthStatus::Idle,
            verifying: None,
            next_id: 1,
            failures: 0,
            backoff_until: None,
            presented: false,
            confirmed: false,
        }
    }

    /// Kilit dışarıya "kurulu" bildirildi mi (`locked` + ilk kareler sunuldu)? Tek örnek
    /// `status` cevabı buna dayanır: ikincil örnek ancak bu doğruyken erken çıkar.
    pub fn is_confirmed(&self) -> bool {
        self.confirmed
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn status(&self) -> &AuthStatus {
        &self.status
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// Gecikmede kalan süre; yoksa `None`.
    pub fn backoff_remaining(&self, now: Instant) -> Option<Duration> {
        self.backoff_until
            .and_then(|t| t.checked_duration_since(now))
            .filter(|d| !d.is_zero())
    }

    /// Şifre girilebilir mi (alan etkin)?
    pub fn accepts_input(&self, now: Instant) -> bool {
        self.phase == Phase::Locked
            && self.verifying.is_none()
            && self.backoff_remaining(now).is_none()
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Action> {
        match (self.phase, event) {
            (Phase::Starting, Event::Start) => {
                self.phase = Phase::Pending;
                vec![Action::LockSession]
            }
            (Phase::Starting, Event::Terminate) => self.finish(EXIT_NOT_LOCKED, vec![]),

            (Phase::Pending, Event::CompositorLocked) => {
                self.phase = Phase::Locked;
                if self.presented {
                    self.confirm()
                } else {
                    vec![Action::WaitPresented(PRESENT_TIMEOUT)]
                }
            }
            (Phase::Starting | Phase::Pending, Event::Presented) => {
                self.presented = true;
                vec![]
            }
            (Phase::Locked | Phase::Unlocking, Event::Presented) => {
                self.presented = true;
                if self.confirmed { vec![] } else { self.confirm() }
            }
            (Phase::Locked | Phase::Unlocking, Event::PresentTimeout) if !self.confirmed => {
                vec![Action::PresentTimedOut]
            }
            (Phase::Pending, Event::CompositorFinished) => self.finish(
                EXIT_NOT_LOCKED,
                vec![Action::DestroyLock, Action::StopFingerprint],
            ),
            (Phase::Pending, Event::Terminate) => self.finish(
                EXIT_NOT_LOCKED,
                vec![Action::DestroyLock, Action::StopFingerprint],
            ),

            // Compositor kilidi kendisi bitirdi (kendi güvenli açma yolu vb.): protokol
            // `unlock_and_destroy` ister. Bu bizim açmamız değil, compositor'un kararı.
            (Phase::Locked | Phase::Unlocking, Event::CompositorFinished) => self.finish(
                EXIT_UNLOCKED,
                vec![Action::ClearInput, Action::StopFingerprint, Action::Unlock],
            ),

            (Phase::Locked, Event::Submit) => self.submit(now),
            (Phase::Locked, Event::AuthFinished { id, result }) => {
                self.auth_finished(id, result, now)
            }
            (Phase::Locked, Event::FingerprintMatched) => self.begin_unlock(),
            (Phase::Locked, Event::Tick) => {
                if self.backoff_until.is_some_and(|t| t <= now) {
                    self.backoff_until = None;
                }
                vec![]
            }

            (Phase::Unlocking, Event::UnlockAnimationDone) => {
                self.finish(EXIT_UNLOCKED, vec![Action::Unlock])
            }

            // Kilitliyken sinyal: yok say (log çağıranın işi). Diğer her olay bu evrede anlamsız.
            _ => vec![],
        }
    }

    fn confirm(&mut self) -> Vec<Action> {
        self.confirmed = true;
        vec![Action::NotifyLocked]
    }

    fn submit(&mut self, now: Instant) -> Vec<Action> {
        if !self.accepts_input(now) {
            return vec![];
        }
        let id = self.next_id;
        self.next_id += 1;
        self.verifying = Some(id);
        self.status = AuthStatus::Verifying;
        vec![Action::StartAuth(id)]
    }

    fn auth_finished(
        &mut self,
        id: AttemptId,
        result: Option<AuthResult>,
        now: Instant,
    ) -> Vec<Action> {
        if self.verifying != Some(id) {
            return vec![];
        }
        self.verifying = None;
        match result {
            Some(r) if r.outcome.is_success() => {
                self.failures = 0;
                self.backoff_until = None;
                self.begin_unlock()
            }
            other => {
                let (outcome, messages) = match other {
                    Some(r) => (r.outcome, r.messages),
                    None => (Outcome::Error(-1), Vec::new()),
                };
                self.failures = self.failures.saturating_add(1);
                self.status = AuthStatus::Failed { outcome, messages };
                let mut actions = vec![Action::ClearInput, Action::Shake];
                if let Some(delay) = backoff_for(self.failures) {
                    self.backoff_until = Some(now + delay);
                    actions.push(Action::WakeAfter(delay));
                }
                actions
            }
        }
    }

    fn begin_unlock(&mut self) -> Vec<Action> {
        self.phase = Phase::Unlocking;
        self.status = AuthStatus::Idle;
        self.verifying = None;
        vec![
            Action::ClearInput,
            Action::StopFingerprint,
            Action::StartUnlockAnimation,
        ]
    }

    fn finish(&mut self, code: i32, mut actions: Vec<Action>) -> Vec<Action> {
        self.phase = Phase::Done(code);
        self.verifying = None;
        actions.push(Action::Exit(code));
        actions
    }
}

/// Kilit yüzeylerinin sunum durumu (saf). Runtime `surface_presented`'i yüzey başına bir kez,
/// yeniden yaratılan yüzey için (çıkış çıkarıp eklenince yeni kimlikle) yeniden bildirir.
/// Bilinmeyen (eklenmemiş ya da kaldırılmış) yüzeyin sunumu yok sayılır.
#[derive(Debug)]
pub struct PresentTracker<T> {
    surfaces: Vec<(T, bool)>,
}

impl<T: PartialEq + Copy> Default for PresentTracker<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: PartialEq + Copy> PresentTracker<T> {
    pub fn new() -> Self {
        Self {
            surfaces: Vec::new(),
        }
    }

    /// Yeni kilit yüzeyi: henüz sunulmadı.
    pub fn added(&mut self, surface: T) {
        match self.surfaces.iter_mut().find(|(s, _)| *s == surface) {
            Some(entry) => entry.1 = false,
            None => self.surfaces.push((surface, false)),
        }
    }

    /// Yüzeyin ilk karesi sunuldu. Dönüş: artık tüm yüzeyler sunuldu mu ([`Self::all`]).
    pub fn presented(&mut self, surface: T) -> bool {
        if let Some(entry) = self.surfaces.iter_mut().find(|(s, _)| *s == surface) {
            entry.1 = true;
        }
        self.all()
    }

    /// Yüzey yok edildi (çıkışı kaldırıldı). Dönüş: [`Self::all`].
    pub fn removed(&mut self, surface: T) -> bool {
        self.surfaces.retain(|(s, _)| *s != surface);
        self.all()
    }

    pub fn clear(&mut self) {
        self.surfaces.clear();
    }

    /// Tüm kilit yüzeyleri ilk karesini sundu mu? Yüzey yoksa hayır (kaplanmamış çıkış
    /// "kilitli" sayılmaz).
    pub fn all(&self) -> bool {
        !self.surfaces.is_empty() && self.surfaces.iter().all(|(_, p)| *p)
    }
}

/// `failures` başarısızdan sonraki gecikme: ilk [`FREE_ATTEMPTS`] serbest, sonra 2, 4, 8, 16,
/// 30 sn (üst sınır).
pub fn backoff_for(failures: u32) -> Option<Duration> {
    let over = failures.checked_sub(FREE_ATTEMPTS)?;
    let secs = 2u64.saturating_pow(over.saturating_add(1).min(16));
    Some(Duration::from_secs(secs).min(MAX_BACKOFF))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_pam::MessageKind;

    fn locked() -> (Machine, Instant) {
        let now = Instant::now();
        let mut m = Machine::new();
        assert_eq!(m.handle(Event::Start, now), vec![Action::LockSession]);
        assert_eq!(
            m.handle(Event::CompositorLocked, now),
            vec![Action::WaitPresented(PRESENT_TIMEOUT)]
        );
        assert_eq!(m.handle(Event::Presented, now), vec![Action::NotifyLocked]);
        (m, now)
    }

    #[test]
    fn locked_is_announced_only_after_first_frames() {
        let now = Instant::now();
        // locked önce, kareler sonra.
        let mut m = Machine::new();
        m.handle(Event::Start, now);
        let a = m.handle(Event::CompositorLocked, now);
        assert!(!a.contains(&Action::NotifyLocked) && !m.is_confirmed());
        assert_eq!(m.phase(), Phase::Locked);
        assert_eq!(m.handle(Event::Presented, now), vec![Action::NotifyLocked]);
        assert!(m.is_confirmed());
        // İkinci kez bildirilmez.
        assert!(m.handle(Event::Presented, now).is_empty());
        assert!(m.handle(Event::PresentTimeout, now).is_empty());

        // Kareler önce (compositor `locked`'ı geciktirdi), locked sonra: hemen bildirilir.
        let mut m = Machine::new();
        m.handle(Event::Start, now);
        assert!(m.handle(Event::Presented, now).is_empty());
        assert!(!m.is_confirmed());
        assert_eq!(
            m.handle(Event::CompositorLocked, now),
            vec![Action::NotifyLocked]
        );
    }

    #[test]
    fn frames_without_locked_never_announce() {
        let now = Instant::now();
        let mut m = Machine::new();
        m.handle(Event::Start, now);
        for e in [Event::Presented, Event::PresentTimeout, Event::Presented] {
            assert!(m.handle(e, now).is_empty());
        }
        let a = m.handle(Event::CompositorFinished, now);
        assert!(!a.contains(&Action::NotifyLocked));
        assert!(!m.is_confirmed());
    }

    #[test]
    fn present_timeout_does_not_announce_and_keeps_lock() {
        let now = Instant::now();
        let mut m = Machine::new();
        m.handle(Event::Start, now);
        m.handle(Event::CompositorLocked, now);
        assert_eq!(
            m.handle(Event::PresentTimeout, now),
            vec![Action::PresentTimedOut]
        );
        // Kilitli kalır: açma ya da çıkış yok, sinyal yine yok sayılır.
        assert_eq!(m.phase(), Phase::Locked);
        assert!(!m.is_confirmed());
        assert!(m.handle(Event::Terminate, now).is_empty());
        // Kare geç gelirse bildirim o zaman.
        assert_eq!(m.handle(Event::Presented, now), vec![Action::NotifyLocked]);
    }

    #[test]
    fn tracker_needs_every_surface() {
        let mut t = PresentTracker::new();
        assert!(!t.all());
        // Bilinmeyen yüzeyin sunumu sayılmaz.
        assert!(!t.presented(9u32));
        t.added(1);
        t.added(2);
        assert!(!t.presented(1));
        assert!(!t.presented(3));
        assert!(t.presented(2));
        t.clear();
        assert!(!t.all());
    }

    #[test]
    fn tracker_handles_recreated_surfaces() {
        let mut t = PresentTracker::new();
        t.added(1u32);
        t.added(2);
        assert!(!t.presented(1));
        // Sunulmamış yüzeyin çıkışı kaldırıldı: kalanlar sunulduysa tamam.
        assert!(t.removed(2));
        // Çıkış geri geldi: yeni yüzey sunulana kadar tamam değil.
        t.added(3);
        assert!(!t.all());
        assert!(t.presented(3));
        // Kaldırılan yüzeyin geç olayı yok sayılır; hepsi kalkarsa tamam değil.
        assert!(t.presented(2));
        assert!(t.removed(1));
        assert!(!t.removed(3));
        assert!(!t.all());
    }

    /// Uygulamanın yaptığı gibi izleyici + makine: hotplug onaydan önce yeni yüzeyi bekletir,
    /// sonra yeniden gelen sunum ikinci bildirim üretmez.
    #[test]
    fn hotplug_before_and_after_confirmation() {
        let now = Instant::now();
        let mut m = Machine::new();
        let mut t = PresentTracker::new();
        m.handle(Event::Start, now);
        t.added(1u32);
        t.added(2);
        assert_eq!(
            m.handle(Event::CompositorLocked, now),
            vec![Action::WaitPresented(PRESENT_TIMEOUT)]
        );
        assert!(!t.presented(1));
        // Onaydan önce 2 kaldırılıp 3 eklendi: 3 sunulana kadar bildirim yok.
        assert!(t.removed(2));
        t.added(3);
        assert!(!t.all());
        assert!(t.presented(3));
        assert_eq!(m.handle(Event::Presented, now), vec![Action::NotifyLocked]);
        // Onaydan sonra yeni çıkış: yeni yüzeyin sunumu ikinci kez bildirmez.
        t.added(4);
        assert!(t.presented(4));
        assert!(m.handle(Event::Presented, now).is_empty());
        assert!(m.is_confirmed());
    }

    fn result(outcome: Outcome) -> Option<AuthResult> {
        Some(AuthResult {
            outcome,
            messages: vec![],
        })
    }

    fn submit(m: &mut Machine, now: Instant) -> AttemptId {
        match m.handle(Event::Submit, now).as_slice() {
            [Action::StartAuth(id)] => *id,
            other => panic!("deneme başlamadı: {other:?}"),
        }
    }

    fn has_unlock(actions: &[Action]) -> bool {
        actions
            .iter()
            .any(|a| matches!(a, Action::Unlock | Action::StartUnlockAnimation))
    }

    #[test]
    fn happy_path() {
        let (mut m, now) = locked();
        let id = submit(&mut m, now);
        assert_eq!(m.status(), &AuthStatus::Verifying);
        let a = m.handle(
            Event::AuthFinished {
                id,
                result: result(Outcome::Success),
            },
            now,
        );
        assert_eq!(
            a,
            vec![
                Action::ClearInput,
                Action::StopFingerprint,
                Action::StartUnlockAnimation
            ]
        );
        assert_eq!(m.phase(), Phase::Unlocking);
        let a = m.handle(Event::UnlockAnimationDone, now);
        assert_eq!(a, vec![Action::Unlock, Action::Exit(EXIT_UNLOCKED)]);
        assert_eq!(m.phase(), Phase::Done(EXIT_UNLOCKED));
    }

    #[test]
    fn every_failure_outcome_keeps_locked() {
        for outcome in [
            Outcome::Denied,
            Outcome::MaxTries,
            Outcome::Unavailable,
            Outcome::UnsupportedPrompt,
            Outcome::Error(4),
            Outcome::Error(-1),
        ] {
            let (mut m, now) = locked();
            let id = submit(&mut m, now);
            let a = m.handle(
                Event::AuthFinished {
                    id,
                    result: result(outcome),
                },
                now,
            );
            assert!(!has_unlock(&a), "{outcome:?}");
            assert_eq!(m.phase(), Phase::Locked);
            assert!(a.contains(&Action::ClearInput) && a.contains(&Action::Shake));
            assert!(matches!(m.status(), AuthStatus::Failed { .. }));
        }
    }

    #[test]
    fn lost_auth_thread_is_failure() {
        let (mut m, now) = locked();
        let id = submit(&mut m, now);
        let a = m.handle(Event::AuthFinished { id, result: None }, now);
        assert!(!has_unlock(&a));
        assert_eq!(m.phase(), Phase::Locked);
        assert_eq!(m.failures(), 1);
    }

    #[test]
    fn stale_or_foreign_results_are_ignored() {
        let (mut m, now) = locked();
        // Deneme yokken gelen "başarı".
        assert!(
            m.handle(
                Event::AuthFinished {
                    id: 1,
                    result: result(Outcome::Success)
                },
                now
            )
            .is_empty()
        );
        let id = submit(&mut m, now);
        // Yanlış kimlik.
        assert!(
            m.handle(
                Event::AuthFinished {
                    id: id + 1,
                    result: result(Outcome::Success)
                },
                now
            )
            .is_empty()
        );
        assert_eq!(m.phase(), Phase::Locked);
        // Aynı kimlik ikinci kez (önce başarısız, sonra "başarı").
        m.handle(
            Event::AuthFinished {
                id,
                result: result(Outcome::Denied),
            },
            now,
        );
        assert!(
            m.handle(
                Event::AuthFinished {
                    id,
                    result: result(Outcome::Success)
                },
                now
            )
            .is_empty()
        );
        assert_eq!(m.phase(), Phase::Locked);
    }

    #[test]
    fn no_unlock_before_locked() {
        let now = Instant::now();
        let mut m = Machine::new();
        // Starting'de hiçbir giriş işe yaramaz.
        for e in [
            Event::Submit,
            Event::FingerprintMatched,
            Event::UnlockAnimationDone,
            Event::CompositorLocked,
        ] {
            assert!(m.handle(e, now).is_empty());
        }
        m.handle(Event::Start, now);
        for e in [
            Event::Submit,
            Event::FingerprintMatched,
            Event::UnlockAnimationDone,
            Event::AuthFinished {
                id: 1,
                result: result(Outcome::Success),
            },
        ] {
            let a = m.handle(e, now);
            assert!(!has_unlock(&a) && !a.contains(&Action::NotifyLocked));
            assert_eq!(m.phase(), Phase::Pending);
        }
    }

    #[test]
    fn finished_before_locked_is_failure_exit() {
        let now = Instant::now();
        let mut m = Machine::new();
        m.handle(Event::Start, now);
        let a = m.handle(Event::CompositorFinished, now);
        assert!(!a.contains(&Action::NotifyLocked) && !a.contains(&Action::Unlock));
        assert!(a.contains(&Action::DestroyLock));
        assert_eq!(a.last(), Some(&Action::Exit(EXIT_NOT_LOCKED)));
        // Sonradan gelen locked bir şey yapmaz.
        assert!(m.handle(Event::CompositorLocked, now).is_empty());
    }

    #[test]
    fn finished_while_locked_unlocks_by_protocol() {
        let (mut m, now) = locked();
        let a = m.handle(Event::CompositorFinished, now);
        assert!(a.contains(&Action::Unlock) && !a.contains(&Action::DestroyLock));
        assert_eq!(m.phase(), Phase::Done(EXIT_UNLOCKED));
    }

    #[test]
    fn terminate_ignored_while_locked() {
        let (mut m, now) = locked();
        assert!(m.handle(Event::Terminate, now).is_empty());
        assert_eq!(m.phase(), Phase::Locked);
        let id = submit(&mut m, now);
        m.handle(
            Event::AuthFinished {
                id,
                result: result(Outcome::Success),
            },
            now,
        );
        assert!(m.handle(Event::Terminate, now).is_empty());
        assert_eq!(m.phase(), Phase::Unlocking);
    }

    #[test]
    fn terminate_while_pending_destroys_without_unlock() {
        let now = Instant::now();
        let mut m = Machine::new();
        m.handle(Event::Start, now);
        let a = m.handle(Event::Terminate, now);
        assert_eq!(
            a,
            vec![
                Action::DestroyLock,
                Action::StopFingerprint,
                Action::Exit(EXIT_NOT_LOCKED)
            ]
        );
    }

    #[test]
    fn one_attempt_at_a_time() {
        let (mut m, now) = locked();
        submit(&mut m, now);
        assert!(m.handle(Event::Submit, now).is_empty());
        assert!(!m.accepts_input(now));
    }

    #[test]
    fn fingerprint_unlocks_only_when_locked_and_supersedes_pending_password() {
        let (mut m, now) = locked();
        let id = submit(&mut m, now);
        let a = m.handle(Event::FingerprintMatched, now);
        assert!(a.contains(&Action::StartUnlockAnimation));
        // Şifre sonucu artık önemsiz.
        assert!(
            m.handle(
                Event::AuthFinished {
                    id,
                    result: result(Outcome::Denied)
                },
                now
            )
            .is_empty()
        );
        assert_eq!(m.phase(), Phase::Unlocking);
    }

    /// PAM ve parmak izi aynı tek açma kapısından geçer: yarışta açma tek kez.
    #[test]
    fn pam_and_fingerprint_share_one_unlock_gate() {
        let count = |all: &[Vec<Action>]| {
            let flat: Vec<&Action> = all.iter().flatten().collect();
            (
                flat.iter()
                    .filter(|a| ***a == Action::StartUnlockAnimation)
                    .count(),
                flat.iter().filter(|a| ***a == Action::Unlock).count(),
            )
        };
        // PAM başarısı + parmak izi eşleşmesi aynı anda.
        let (mut m, now) = locked();
        let id = submit(&mut m, now);
        let mut all = vec![
            m.handle(
                Event::AuthFinished {
                    id,
                    result: result(Outcome::Success),
                },
                now,
            ),
            m.handle(Event::FingerprintMatched, now),
            m.handle(Event::FingerprintMatched, now),
            m.handle(Event::UnlockAnimationDone, now),
            m.handle(Event::UnlockAnimationDone, now),
        ];
        assert_eq!(count(&all), (1, 1));
        // PAM başarısız, sonra parmak izi: bir kez açar.
        let (mut m, now) = locked();
        let id = submit(&mut m, now);
        all = vec![
            m.handle(
                Event::AuthFinished {
                    id,
                    result: result(Outcome::Denied),
                },
                now,
            ),
            m.handle(Event::FingerprintMatched, now),
            m.handle(Event::UnlockAnimationDone, now),
        ];
        assert_eq!(count(&all), (1, 1));
        // Parmak izi önce, sonra geç gelen PAM başarısı: yine tek.
        let (mut m, now) = locked();
        let id = submit(&mut m, now);
        all = vec![
            m.handle(Event::FingerprintMatched, now),
            m.handle(
                Event::AuthFinished {
                    id,
                    result: result(Outcome::Success),
                },
                now,
            ),
            m.handle(Event::UnlockAnimationDone, now),
        ];
        assert_eq!(count(&all), (1, 1));
    }

    #[test]
    fn backoff_sequence() {
        assert_eq!(backoff_for(0), None);
        assert_eq!(backoff_for(2), None);
        let secs: Vec<u64> = (3..=9).map(|f| backoff_for(f).unwrap().as_secs()).collect();
        assert_eq!(secs, [2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(backoff_for(u32::MAX), Some(MAX_BACKOFF));
    }

    #[test]
    fn backoff_blocks_submit_until_tick() {
        let (mut m, t0) = locked();
        for _ in 0..FREE_ATTEMPTS {
            let id = submit(&mut m, t0);
            m.handle(
                Event::AuthFinished {
                    id,
                    result: result(Outcome::Denied),
                },
                t0,
            );
        }
        assert_eq!(m.backoff_remaining(t0), Some(Duration::from_secs(2)));
        assert!(
            m.handle(Event::Submit, t0 + Duration::from_secs(1))
                .is_empty()
        );
        let t1 = t0 + Duration::from_secs(2);
        m.handle(Event::Tick, t1);
        assert!(m.accepts_input(t1));
        submit(&mut m, t1);
    }

    #[test]
    fn third_failure_schedules_wake() {
        let (mut m, now) = locked();
        let mut last = vec![];
        for _ in 0..FREE_ATTEMPTS {
            let id = submit(&mut m, now);
            last = m.handle(
                Event::AuthFinished {
                    id,
                    result: result(Outcome::Denied),
                },
                now,
            );
        }
        assert!(last.contains(&Action::WakeAfter(Duration::from_secs(2))));
    }

    #[test]
    fn success_resets_failures() {
        let (mut m, now) = locked();
        let id = submit(&mut m, now);
        m.handle(
            Event::AuthFinished {
                id,
                result: result(Outcome::Denied),
            },
            now,
        );
        let id = submit(&mut m, now);
        m.handle(
            Event::AuthFinished {
                id,
                result: result(Outcome::Success),
            },
            now,
        );
        assert_eq!(m.failures(), 0);
    }

    #[test]
    fn pam_messages_are_kept_for_display() {
        let (mut m, now) = locked();
        let id = submit(&mut m, now);
        let msg = PamMessage {
            kind: MessageKind::Error,
            text: "Hesap 10 dk kilitli".into(),
        };
        m.handle(
            Event::AuthFinished {
                id,
                result: Some(AuthResult {
                    outcome: Outcome::Denied,
                    messages: vec![msg.clone()],
                }),
            },
            now,
        );
        assert_eq!(
            m.status(),
            &AuthStatus::Failed {
                outcome: Outcome::Denied,
                messages: vec![msg]
            }
        );
    }

    /// Rastgele olay dizileri: açma eylemleri yalnızca izinli geçişlerde görülür.
    #[test]
    fn random_sequences_respect_unlock_invariants() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut rand = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let outcomes = [
            Outcome::Success,
            Outcome::Denied,
            Outcome::MaxTries,
            Outcome::Error(4),
            Outcome::UnsupportedPrompt,
        ];
        let mut password_unlocks = 0;
        for _ in 0..5_000 {
            let mut m = Machine::new();
            let mut now = Instant::now();
            let mut open_attempts: Vec<AttemptId> = vec![];
            let mut animating = false;
            let (mut saw_locked, mut saw_presented, mut notified) = (false, false, 0);
            for _ in 0..40 {
                now += Duration::from_millis(rand(5_000));
                let event = match rand(12) {
                    0 => Event::Start,
                    1 => Event::CompositorLocked,
                    2 => Event::CompositorFinished,
                    3 | 4 => Event::Submit,
                    5 | 6 => {
                        let id = open_attempts
                            .last()
                            .copied()
                            .filter(|_| rand(4) != 0)
                            .unwrap_or(rand(8));
                        let result = (rand(6) != 0).then(|| AuthResult {
                            outcome: outcomes[rand(outcomes.len() as u64) as usize],
                            messages: vec![],
                        });
                        Event::AuthFinished { id, result }
                    }
                    7 => Event::FingerprintMatched,
                    8 => Event::UnlockAnimationDone,
                    9 => {
                        if rand(2) == 0 {
                            Event::Terminate
                        } else {
                            Event::Tick
                        }
                    }
                    10 => Event::Presented,
                    _ => Event::PresentTimeout,
                };
                let before = m.phase();
                if before == Phase::Pending && event == Event::CompositorLocked {
                    saw_locked = true;
                }
                if !matches!(before, Phase::Done(_)) && event == Event::Presented {
                    saw_presented = true;
                }
                let actions = m.handle(event.clone(), now);
                for a in &actions {
                    match a {
                        Action::StartAuth(id) => {
                            assert_eq!(before, Phase::Locked);
                            open_attempts.push(*id);
                        }
                        Action::StartUnlockAnimation => {
                            assert_eq!(before, Phase::Locked);
                            let ok = match &event {
                                Event::FingerprintMatched => true,
                                Event::AuthFinished {
                                    id,
                                    result: Some(r),
                                } => r.outcome.is_success() && open_attempts.last() == Some(id),
                                _ => false,
                            };
                            assert!(ok, "izinsiz açma: {event:?}");
                            animating = true;
                        }
                        Action::Unlock => {
                            let by_compositor = matches!(event, Event::CompositorFinished)
                                && matches!(before, Phase::Locked | Phase::Unlocking);
                            let by_animation =
                                matches!(event, Event::UnlockAnimationDone) && animating;
                            if by_animation {
                                password_unlocks += 1;
                            }
                            assert!(
                                by_compositor || by_animation,
                                "izinsiz Unlock: {event:?} {before:?}"
                            );
                        }
                        Action::NotifyLocked => {
                            // Yalnızca bir kez, `locked` VE ilk kareler görüldükten sonra.
                            assert!(saw_locked && saw_presented, "erken NotifyLocked: {event:?}");
                            assert!(matches!(
                                event,
                                Event::CompositorLocked | Event::Presented
                            ));
                            notified += 1;
                            assert_eq!(notified, 1, "NotifyLocked iki kez");
                        }
                        Action::DestroyLock => assert_eq!(before, Phase::Pending),
                        _ => {}
                    }
                }
                if matches!(event, Event::AuthFinished { .. }) && !actions.is_empty() {
                    open_attempts.clear();
                }
                if matches!(m.phase(), Phase::Done(_)) {
                    break;
                }
            }
        }
        assert!(
            password_unlocks > 50,
            "dizi kapsamı zayıf: {password_unlocks}"
        );
    }

    #[test]
    fn done_is_terminal() {
        let (mut m, now) = locked();
        m.handle(Event::CompositorFinished, now);
        for e in [
            Event::Start,
            Event::Submit,
            Event::CompositorLocked,
            Event::UnlockAnimationDone,
            Event::Terminate,
        ] {
            assert!(m.handle(e, now).is_empty());
        }
    }
}
