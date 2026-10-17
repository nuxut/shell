//! Girdi: input bölgesi ve klavye modu eşleme, klavye olayları, odak kaybı/odak alma.

use std::time::{Duration, Instant};

use nuxut::{Context, FocusGrabId, Key, KeyEvent, KeyState, KeyboardMode, SurfaceId, TimerId};

use super::Shell;
use crate::config::ShellConfig;
use crate::panels::{self, GrabStep};
use crate::screen::Screen;
use crate::widgets::Action;

/// Panel açılınca `Exclusive` ile istenen odağın en geç gelmesi beklenen süre.
const FOCUS_GRAB_TIMEOUT: Duration = Duration::from_millis(300);
/// Odak kaybında overlay/yaprak kapatılmadan önceki bekleme: bazı compositor'lar (sway,
/// `OnDemand`) yüzeyin içine tıklanınca aynı anda leave+enter gönderir; bu süre içinde odak
/// geri gelirse kayıp geçici sayılır.
const UNFOCUS_GRACE: Duration = Duration::from_millis(50);

impl Shell {
    /// Input bölgesini ve klavye modunu yalnızca değiştiyse uygular (compositor'a gereksiz
    /// commit yok).
    /// Ardından odak kapmayı geçici yaprağa eşler ([`Shell::sync_focus_grab`]).
    pub(super) fn apply_input(cx: &mut Context<Self>, screen: &mut Screen, config: &ShellConfig) {
        let before = screen.applied_keyboard_mode();
        let mut to_on_demand = false;
        if let Some((rects, mode)) = screen.take_input_change(config) {
            tracing::debug!(output = %screen.name, ?rects, ?mode, "input bölgesi");
            cx.set_input_region(screen.main, &rects);
            cx.set_keyboard_mode(screen.main, mode);
            to_on_demand = before == Some(KeyboardMode::Exclusive) && mode == KeyboardMode::OnDemand;
        }
        Self::sync_focus_grab(cx, screen, to_on_demand);
    }

    /// Odak kapma (`hyprland_focus_grab_v1`): geçici yaprak açıkken ana yüzey için grab tutulur
    /// (başka uygulamanın açık menüsü kapanır; liste dışına tıklama `cleared` → yaprak kapanır).
    /// Kapanırken önce bırakılır (`cleared` gelmez). Klavye `Exclusive` → `OnDemand` inince önce
    /// mod (bu çağrıdan önce gönderildi), sonra yeni grab. Protokol yoksa (sway) hiçbir şey
    /// gönderilmez.
    fn sync_focus_grab(cx: &mut Context<Self>, screen: &mut Screen, to_on_demand: bool) {
        if !cx.focus_grab_supported() && screen.focus_grab.is_none() {
            return;
        }
        let step = panels::focus_grab_step(screen.wants_focus_grab(), screen.focus_grab.is_some(), to_on_demand);
        match step {
            GrabStep::Keep => return,
            GrabStep::Release | GrabStep::Reacquire => {
                if let Some(id) = screen.focus_grab.take() {
                    cx.focus_grab_release(id);
                }
            }
            GrabStep::Acquire => {}
        }
        if matches!(step, GrabStep::Acquire | GrabStep::Reacquire) {
            let mut surfaces = vec![screen.main];
            surfaces.extend(screen.focus_grab_extra);
            screen.focus_grab = cx.focus_grab(&surfaces);
        }
        tracing::debug!(output = %screen.name, ?step, held = screen.focus_grab.is_some(), "odak kapma");
    }

    /// Compositor grab'ı bitirdi (başka uygulamaya/liste dışına tıklama): örtük kapatma —
    /// overlay ve geçici yaprak kapanır, sabit panel kalır. Kendi bıraktığımız grab için gelmez.
    pub(super) fn focus_grab_cleared_inner(&mut self, cx: &mut Context<Self>, id: FocusGrabId) {
        cx.focus_grab_release(id); // kapalı grab: istek gitmez, defter temizlenir
        let Some(s) = self.screens.iter_mut().find(|s| s.focus_grab == Some(id)) else { return };
        s.focus_grab = None;
        let surface = s.main;
        tracing::debug!(output = %s.name, "odak kapma bitti, geçici yaprak kapanıyor");
        self.dismiss_transient(cx, surface);
    }

    /// Tüm ekranlarda input bölgesi/klavye modu eşlenir: overlay (tepsi menüsü) ve bildirim
    /// popup'ları boyutlarını çizimde öğrenir, menü kendi kendine kapanabilir. Olay
    /// işleyicilerinin sonunda ve çizimden sonraki tek atımlık timer'da çağrılır. Klavye
    /// `Exclusive` istendiyse odak için yedek timer kurulur.
    pub(super) fn refresh_inputs(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.input_timer.take() {
            cx.cancel_timer(t);
        }
        let config = self.model.config.clone();
        let mut awaiting = false;
        for s in &mut self.screens {
            Self::apply_input(cx, s, &config);
            awaiting |= s.awaiting_focus();
        }
        if awaiting && self.grab_timer.is_none() {
            self.grab_timer = Some(cx.set_timer(FOCUS_GRAB_TIMEOUT));
        }
    }

    /// Yeni yaprak açıldı: odak için yedek süre baştan başlar.
    pub(super) fn restart_focus_grab(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.grab_timer.take() {
            cx.cancel_timer(t);
        }
    }

    /// Girdi timer'larından biriyse (odak bekleme, çizim sonrası eşleme, odak kaybı) işler
    /// ve `true` döner.
    pub(super) fn input_timer_fired(&mut self, cx: &mut Context<Self>, id: TimerId) -> bool {
        if self.grab_timer == Some(id) {
            self.grab_timer = None;
            let config = self.model.config.clone();
            for s in &mut self.screens {
                if s.awaiting_focus() && s.give_up_focus_grab() {
                    tracing::debug!(output = %s.name, "odak gelmedi, OnDemand'e inildi");
                    Self::apply_input(cx, s, &config);
                }
            }
            return true;
        }
        if self.input_timer == Some(id) {
            self.input_timer = None;
            self.refresh_inputs(cx);
            return true;
        }
        if let Some(surface) = self.unfocus_timers.fired(&id) {
            self.focus_lost(cx, surface);
            return true;
        }
        false
    }

    pub(super) fn keyboard_inner(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        let theme = cx.theme().clone();
        let config = self.model.config.clone();
        let now = Instant::now();
        // Önce overlay ve panel widget'ları (menüde ok tuşları, Enter…).
        let consumed = {
            let text = cx.text();
            let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) else { return };
            s.key(text, &config, &theme, &event, now)
        };
        if let Some((redraw, action)) = consumed {
            if redraw {
                cx.request_frame(surface);
            }
            if let Some(a) = action {
                self.act(cx, surface, a);
            }
            return;
        }
        // Klavye gezinmesi (Tab/oklar/Enter; bar ve açık yapraklar; screen/access.rs).
        if self.nav_key(cx, surface, &event) {
            return;
        }
        if event.state != KeyState::Pressed || event.key != Key::Escape {
            return;
        }
        // Escape: önce açık overlay, sonra geçici yaprak (sabit panel kalır).
        let Some(idx) = self.screens.iter().position(|s| s.main == surface) else { return };
        if self.screens[idx].close_overlays(now) {
            Self::apply_input(cx, &mut self.screens[idx], &config);
            cx.request_frame(surface);
        } else if let Some(open) = self.screens[idx].transient_panel() {
            self.act(cx, surface, Action::ClosePanel(open));
        }
    }

    pub(super) fn keyboard_focus_inner(&mut self, cx: &mut Context<Self>, surface: SurfaceId, focused: bool) {
        tracing::debug!(?surface, focused, "klavye odağı");
        if !focused {
            if !self.unfocus_timers.is_pending(&surface) {
                let t = cx.set_timer(UNFOCUS_GRACE);
                self.unfocus_timers.push(surface, t);
            }
            return;
        }
        let (own, others) = self.unfocus_timers.gained(&surface);
        if let Some(t) = own {
            // Geçici kayıp (yüzeyin içine tıklama): hiçbir şey kapanmaz.
            cx.cancel_timer(t);
        }
        // Odak başka bir ekrana geçti: oradaki kayıp kesin, beklemeden kapanır.
        for (other, t) in others {
            cx.cancel_timer(t);
            self.focus_lost(cx, other);
        }
        let config = self.model.config.clone();
        let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) else { return };
        if s.focus_changed(focused) {
            Self::apply_input(cx, s, &config);
        }
    }

    /// Odak geri gelmedi (başka yere tıklandı): overlay (tepsi menüsü) ve geçici yaprak kapanır;
    /// sabit panel kalır. Etkin odak kapma varken kapanışı `cleared` sürer (çift kapanış yok).
    fn focus_lost(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {
        let Some(s) = self.screens.iter().find(|s| s.main == surface) else { return };
        if s.focus_grab.is_some_and(|id| cx.focus_grab_active(id)) {
            tracing::debug!(output = %s.name, "odak kaybı: odak kapma etkin, kapanış cleared'a bırakıldı");
            return;
        }
        self.dismiss_transient(cx, surface);
    }

    /// Örtük kapatma: overlay (tepsi menüsü) ve geçici yaprak; sabit panel kalır.
    fn dismiss_transient(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {
        let config = self.model.config.clone();
        let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) else { return };
        // Klavye gezinmesi odakla biter (başka yere tıklandı).
        let nav_ended = s.end_nav();
        if s.close_overlays(Instant::now()) || nav_ended {
            Self::apply_input(cx, s, &config);
            cx.request_frame(surface);
        }
        if let Some(open) = s.transient_panel() {
            self.act(cx, surface, Action::ClosePanel(open));
        }
    }
}

/// Odağını kaybetmiş, kapanmayı bekleyen ana yüzeyler ([`UNFOCUS_GRACE`]). Yüzey ve timer
/// kimliği üzerinden genel: saf durum, birim testli.
#[derive(Debug)]
pub(super) struct UnfocusTimers<S, T> {
    pending: Vec<(S, T)>,
}

impl<S, T> Default for UnfocusTimers<S, T> {
    fn default() -> Self {
        Self { pending: Vec::new() }
    }
}

impl<S: PartialEq, T: PartialEq> UnfocusTimers<S, T> {
    fn is_pending(&self, surface: &S) -> bool {
        self.pending.iter().any(|(s, _)| s == surface)
    }

    fn push(&mut self, surface: S, timer: T) {
        self.pending.push((surface, timer));
    }

    /// `surface` odak aldı: kendi bekleyen timer'ı (geçici kayıp, iptal edilir) ve diğer
    /// yüzeylerinkiler (kayıp kesinleşti, hemen kapatılır). Hepsi listeden çıkar.
    fn gained(&mut self, surface: &S) -> (Option<T>, Vec<(S, T)>) {
        let (own, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending).into_iter().partition(|(s, _)| s == surface);
        (own.into_iter().next().map(|(_, t)| t), others)
    }

    /// Timer doldu: bekleyen yüzeyse çıkarılır ve döner.
    fn fired(&mut self, timer: &T) -> Option<S> {
        let i = self.pending.iter().position(|(_, t)| t == timer)?;
        Some(self.pending.swap_remove(i).0)
    }
}

#[cfg(test)]
mod tests {
    use super::UnfocusTimers;

    #[test]
    fn unfocus_timers_transient_and_cross_screen() {
        let mut u = UnfocusTimers::<u8, u32>::default();
        // Aynı yüzeye leave+enter (sway tıklaması): kendi timer'ı iptal, kapanan yok.
        u.push(1, 10);
        assert!(u.is_pending(&1));
        assert_eq!(u.gained(&1), (Some(10), vec![]));
        assert!(!u.is_pending(&1));

        // Odak 1. ekrandan 2.'ye: 1'in bekleyen kaybı hemen kesinleşir.
        u.push(1, 11);
        assert_eq!(u.gained(&2), (None, vec![(1, 11)]));
        assert_eq!(u.fired(&11), None, "iptal edilen timer artık eşleşmez");

        // Odak hiçbir yüzeye gelmedi: timer dolunca kapanır.
        u.push(1, 12);
        u.push(3, 13);
        assert_eq!(u.fired(&12), Some(1));
        assert_eq!(u.gained(&3), (Some(13), vec![]));
    }
}
