//! Oturum bağlantısı: kilit/çıkış/güç eylemleri, kullanıcı bilgisi, kilit ekranı tetikleyicisi
//! (`services::lock_launcher`, `docs/design/lock-trigger.md`) ve oturumu kapatma
//! (`services::logout`, `docs/design/logout.md`).

use nuxut::Context;
use nuxut::i18n::t;

use super::{Shell, ShellMessage};
use crate::services::lock_launcher::{
    LOCK_SIGNAL_GRACE, Launch, LaunchError, LockExit, LockLauncher, LockTrigger, decide, needs_signal_watchdog,
};
use crate::services::logout::{self, LogoutEvent};
use crate::services::notifications::{InternalAction, InternalNotice};
use crate::services::session::{SessionAction, SessionEvent, SessionOptions, SessionService, UserInfo};

/// Kilit ret bildiriminin düğmesi: Ayarlar › Güç (Kilit kartı, `lock.listen_logind`).
pub(super) const LOCK_SETTINGS_COMMAND: &str = "settings open power";

impl Shell {
    pub(super) fn start_session(&mut self, cx: &mut Context<Self>) {
        self.model.user = UserInfo::current();
        let sender = cx.sender();
        self.lock = LockLauncher::installed().with_exit_sink(move |e| {
            let _ = sender.send(ShellMessage::LockExited(e));
        });
        // Kurulumda eksik kalan kilit ikilisi (2026-10-16 kök nedeni) Kilitle'ye basılınca değil
        // açılışta görünsün.
        if let Some(missing) = self.lock.program().filter(|p| !p.is_file()).map(|p| p.display().to_string()) {
            tracing::warn!(path = %missing, "kilit ekranı programı yok; Kilitle çalışmayacak (install.sh ile yeniden kurun)");
            let body = t!("shell-lock-missing-body", path = missing);
            let _ = cx.sender().send(ShellMessage::Notice(InternalNotice::popup(t!("shell-lock-missing-title"), body, Vec::new())));
        }
        // Test örneği gerçek logind'e kilit/çıkış göndermez (boşta servisiyle aynı kural): Kilitle
        // doğrudan (kendi kilit örnek adıyla) nuxut-lock'a düşer, diğer eylemler yapılmaz.
        if !super::idle::system_logind_allowed() {
            tracing::info!("oturum: test örneği, gerçek logind'e bağlanılmıyor");
            return;
        }
        let sender = cx.sender();
        match SessionService::spawn(SessionOptions::default(), move |e| {
            let _ = sender.send(ShellMessage::Session(e));
        }) {
            Ok(service) => self.session = Some(service),
            Err(e) => tracing::warn!("oturum servisi başlatılamadı: {e}"),
        }
    }

    pub(super) fn on_session(&mut self, cx: &mut Context<Self>, e: SessionEvent) {
        match e {
            SessionEvent::Done(SessionAction::Lock) => {
                self.lock_trigger(cx, LockTrigger::LockSent { listener_ready: self.idle.lock_listener_ready() });
            }
            SessionEvent::Failed { action: SessionAction::Lock, .. } => self.lock_trigger(cx, LockTrigger::LockFailed),
            _ => {}
        }
        if self.model.apply_session(e) {
            self.model_changed(cx);
        }
    }

    pub(super) fn session_action(&mut self, cx: &mut Context<Self>, a: SessionAction) {
        if a == SessionAction::Logout {
            self.logout(cx);
            return;
        }
        match &self.session {
            Some(s) => s.perform(a),
            None if a == SessionAction::Lock => {
                tracing::warn!("oturum servisi yok, kilit doğrudan başlatılıyor");
                self.lock_trigger(cx, LockTrigger::LockFailed);
            }
            None => tracing::warn!("oturum servisi yok, {a:?} yapılamadı"),
        }
    }

    /// Kilit tetiği: logind sinyali ya da (logind'e ulaşılamazken) doğrudan. Başlatılamazsa
    /// kullanıcıya görünür bildirim (sessizce hiçbir şey olmaması kabul edilmez).
    pub(super) fn lock_trigger(&mut self, cx: &mut Context<Self>, trigger: LockTrigger) {
        let listen = self.model.config.lock.listen_logind;
        if trigger == LockTrigger::LogindSignal {
            self.lock_signal_pending = false;
        }
        if needs_signal_watchdog(trigger, listen) {
            self.lock_signal_pending = true;
            let sender = cx.sender();
            let spawned = std::thread::Builder::new().name("nuxut-lock-grace".into()).spawn(move || {
                std::thread::sleep(LOCK_SIGNAL_GRACE);
                let _ = sender.send(ShellMessage::LockSignalTimeout);
            });
            if let Err(e) = spawned {
                tracing::warn!("kilit bekçisi açılamadı ({e}); kilit doğrudan başlatılıyor");
                self.lock_signal_pending = false;
                self.lock_trigger(cx, LockTrigger::LockFailed);
            }
            return;
        }
        if !decide(trigger, listen) {
            tracing::debug!(?trigger, "kilit: nuxut-lock başlatılmıyor (dinleyici kapalı ya da sinyal bekleniyor)");
            return;
        }
        match self.lock.launch() {
            Ok(Launch::Started) => tracing::info!(?trigger, "kilit ekranı istendi"),
            Ok(Launch::AlreadyRunning) => tracing::debug!(?trigger, "kilit ekranı zaten çalışıyor"),
            Err(e) => {
                tracing::error!(?trigger, "kilit ekranı başlatılamadı: {e}");
                let body = match &e {
                    LaunchError::NotFound(p) => t!(
                        "shell-lock-missing-body",
                        path = p.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "nuxut-lock".into())
                    ),
                    LaunchError::TestInstance | LaunchError::Spawn(_) => t!("shell-lock-spawn-body", error = e.to_string()),
                };
                self.notice(cx, InternalNotice::popup(t!("shell-lock-failed-title"), body, Vec::new()));
            }
        }
    }

    /// `Session.Lock` iletildi ama kendi oturumumuzun `Lock` sinyali [`LOCK_SIGNAL_GRACE`] içinde
    /// gelmedi (yanlış oturuma gitmiş olabilir): oturum zaten kilitli değilse doğrudan aç.
    pub(super) fn on_lock_signal_timeout(&mut self, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.lock_signal_pending) {
            return;
        }
        if self.idle.session_locked() || self.lock.is_running() {
            return;
        }
        tracing::warn!("kilit: logind Lock sinyali {LOCK_SIGNAL_GRACE:?} içinde gelmedi; nuxut-lock doğrudan başlatılıyor");
        self.lock_trigger(cx, LockTrigger::LockFailed);
    }

    /// Kendi başlattığımız `nuxut-lock` çıktı (`LockLauncher` bekleme thread'i).
    pub(super) fn on_lock_exited(&mut self, cx: &mut Context<Self>, e: LockExit) {
        if !e.failed() {
            return;
        }
        let (body, actions) = if e.refused() {
            (t!("shell-lock-refused-body"), vec![InternalAction::new(t!("shell-lock-action-settings"), LOCK_SETTINGS_COMMAND)])
        } else {
            let reason = match (e.code, e.signal) {
                (Some(c), _) => format!("exit {c}"),
                (None, Some(s)) => format!("signal {s}"),
                (None, None) => "?".into(),
            };
            (t!("shell-lock-crashed-body", reason = reason), Vec::new())
        };
        self.notice(cx, InternalNotice::popup(t!("shell-lock-failed-title"), body, actions));
    }

    /// Oturumu kapat: önce compositor'a temiz çıkış (uwsm altında `uwsm stop`, yoksa Hyprland
    /// `exit`), olmazsa ya da [`logout::FALLBACK_AFTER`] içinde çıkılmazsa logind `Terminate`.
    fn logout(&mut self, cx: &mut Context<Self>) {
        if !super::idle::system_logind_allowed() {
            // Test örneği kullanıcının compositor'ını kapatmasın.
            tracing::warn!("oturum: test örneği, oturumu kapat yapılmadı");
            return;
        }
        let client = self.client.clone();
        let sender = cx.sender();
        let spawned = std::thread::Builder::new().name("nuxut-logout".into()).spawn(move || {
            match logout::request_exit(client.as_ref()) {
                Ok(path) => {
                    let _ = sender.send(ShellMessage::Logout(LogoutEvent::Requested(path)));
                    std::thread::sleep(logout::FALLBACK_AFTER);
                    let _ = sender.send(ShellMessage::Logout(LogoutEvent::Stalled));
                }
                Err(e) => {
                    let _ = sender.send(ShellMessage::Logout(LogoutEvent::Failed(e)));
                }
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("oturumu kapat thread'i açılamadı: {e}");
            self.terminate_session();
        }
    }

    pub(super) fn on_logout(&mut self, e: LogoutEvent) {
        match e {
            LogoutEvent::Requested(path) => tracing::info!(?path, "oturumu kapat: compositor çıkıyor"),
            LogoutEvent::Failed(reason) => {
                tracing::warn!("oturumu kapat: compositor çıkışı istenemedi ({reason}); logind Terminate");
                self.terminate_session();
            }
            LogoutEvent::Stalled => {
                tracing::warn!("oturumu kapat: compositor {:?} içinde çıkmadı; logind Terminate", logout::FALLBACK_AFTER);
                self.terminate_session();
            }
        }
    }

    fn terminate_session(&self) {
        match &self.session {
            Some(s) => s.perform(SessionAction::Logout),
            None => tracing::warn!("oturum servisi yok, oturum kapatılamadı"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_settings_action_opens_power_page() {
        assert_eq!(crate::ipc::parse(LOCK_SETTINGS_COMMAND), Ok(crate::ipc::Command::OpenSettings("power".into())));
    }
}
