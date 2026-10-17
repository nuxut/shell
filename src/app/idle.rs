//! Boşta kalma bağlantısı (t061): durum makinesi (`services::idle`), logind, karartma örtüsü,
//! ekran gücü (Hyprland DPMS), `msg idle`. Tasarım: `docs/design/idle.md`.
//!
//! "Boşta" bildirimi compositor'dan gelir (ext-idle-notify-v1, framework `cx.idle_watch`):
//! aşama başına bir izleme, Wayland idle inhibitor'larına saygılı ([`Shell::arm_idle_watches`]).
//! Compositor desteklemiyorsa sessiz yedek yok: `msg idle status` `source: "unsupported"`,
//! boşta aşamaları çalışmaz; uyku öncesi kilit, engeller ve `msg idle` yine çalışır.

use std::time::{Duration, Instant};

use nuxut::{
    Anchor, App, Context, Corners, IdleSupport, IdleWatchError, IdleWatchId, KeyboardMode, Layer, LayerSurfaceSpec,
    OutputId, Painter, Rect, RectStyle, SurfaceId, TimerId,
};

use super::{Shell, ShellMessage};
use crate::ipc::{IdleCommand, Toggle};
use crate::services::idle::{
    Effect, IdleLogind, IdleMachine, IdleWatcher, Inhibitor, LogindAction, LogindEvent, LogindOptions, LogindState, Stage,
    WatchSource,
    Watches,
};
use crate::services::lock_launcher::LockTrigger;
use crate::services::media::PlaybackStatus;

pub const DIM_NAMESPACE: &str = "nuxut-dim";
/// Karartma örtüsünün son opaklığı (temanın gölge rengi üstünde; framework diyalog scrim'i gibi).
const DIM_ALPHA: f32 = 0.5;
/// Örtünün belirme süresi (kalkışı anlıktır: kullanıcı geri döndü).
const DIM_FADE: Duration = Duration::from_secs(1);

/// Boşta kalma durumu (Shell alanı).
#[derive(Debug)]
pub struct IdleState {
    machine: IdleMachine,
    logind: Option<IdleLogind>,
    logind_state: LogindState,
    /// logind'de gerçekten tutulan uyku gecikme inhibitörü (`SleepDelayHeld`).
    delay_held: bool,
    /// Compositor'da izlenen aşamalar (izleme kimliği ↔ aşama).
    watches: Watches<IdleWatchId>,
    /// Kilit bekleme (uyku öncesi) timer'ı ve anı.
    timer: Option<(TimerId, Instant)>,
    /// Karartma örtüleri (monitör başına) ve belirmeye başladığı an.
    dims: Vec<(OutputId, SurfaceId)>,
    dim_since: Option<Instant>,
    /// Sıralı ekran gücü thread'inin kuyruğu (ilk istekte açılır).
    dpms: Option<std::sync::mpsc::Sender<bool>>,
}

impl IdleState {
    /// Oturum kilitli mi (logind `LockedHint`; kilit ekranı bunu ayarlar). logind yoksa `false`.
    pub(super) fn session_locked(&self) -> bool {
        self.logind_state.locked_hint
    }

    /// logind `Lock` sinyali bize ulaşır mı (bağlı ve oturum çözülmüş)? Değilse `Session.Lock`
    /// başarılı olsa da kilit doğrudan başlatılır.
    pub(super) fn lock_listener_ready(&self) -> bool {
        self.logind.is_some() && self.logind_state.available && self.logind_state.has_session
    }
}

impl Default for IdleState {
    fn default() -> Self {
        Self {
            machine: IdleMachine::new(crate::services::idle::IdleSettings::default()),
            logind: None,
            logind_state: LogindState::default(),
            delay_held: false,
            watches: Watches::default(),
            timer: None,
            dims: Vec::new(),
            dim_since: None,
            dpms: None,
        }
    }
}

/// Test örneği (`NUXUT_SHELL_INSTANCE`) gerçek sistem bus'ındaki logind'e dokunmaz: inhibitör
/// almaz, kilitlemez, uyutmaz. Yalnızca `DBUS_SYSTEM_BUS_ADDRESS` açıkça (sahte bir bus'a)
/// yönlendirilmişse bağlanır.
fn logind_allowed(instance: Option<&str>, system_bus: Option<&str>) -> bool {
    instance.is_none_or(str::is_empty) || system_bus.is_some_and(|a| !a.is_empty())
}

/// [`logind_allowed`] bu sürecin ortamıyla (oturum servisi de kullanır).
pub(super) fn system_logind_allowed() -> bool {
    let instance = std::env::var(crate::ipc::INSTANCE_ENV).ok();
    let system_bus = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").ok();
    logind_allowed(instance.as_deref(), system_bus.as_deref())
}

/// `Context` üzerinden izleme arka ucu ([`Watches::sync`] için).
struct CxWatcher<'a, A: App>(&'a mut Context<A>);

impl<A: App> IdleWatcher for CxWatcher<'_, A> {
    type Id = IdleWatchId;
    type Error = IdleWatchError;
    fn watch(&mut self, timeout: Duration) -> Result<IdleWatchId, IdleWatchError> {
        self.0.idle_watch(timeout)
    }
    fn set_timeout(&mut self, id: IdleWatchId, timeout: Duration) -> bool {
        self.0.idle_set_timeout(id, timeout)
    }
    fn unwatch(&mut self, id: IdleWatchId) {
        self.0.idle_unwatch(id)
    }
}

/// `msg idle status` → `source`. İzleme kurulduysa ya da kurulamadıysa o; hiç aşama
/// istenmediyse (varsayılan) compositor'ın o anki desteği.
fn status_source(watches: WatchSource, support: IdleSupport) -> &'static str {
    match (watches, support) {
        (WatchSource::Compositor, _) | (WatchSource::Idle, IdleSupport::Supported { .. }) => "compositor",
        (WatchSource::Unsupported, _) | (WatchSource::Idle, IdleSupport::Unsupported) => "unsupported",
    }
}

impl Shell {
    pub(super) fn start_idle(&mut self, cx: &mut Context<Self>) {
        self.idle.machine = IdleMachine::new(self.model.config.idle.settings());
        if system_logind_allowed() {
            let sender = cx.sender();
            match IdleLogind::spawn(LogindOptions::default(), move |e| {
                let _ = sender.send(ShellMessage::IdleLogind(e));
            }) {
                Ok(s) => self.idle.logind = Some(s),
                Err(e) => tracing::warn!("boşta: logind servisi başlatılamadı: {e}"),
            }
        } else {
            tracing::info!("boşta: test örneği, gerçek logind'e bağlanılmıyor");
        }
        let fx = self.idle.machine.start();
        self.sync_idle_system();
        self.apply_idle(cx, fx);
    }

    pub(super) fn on_idle_logind(&mut self, cx: &mut Context<Self>, e: LogindEvent) {
        let fx = match e {
            LogindEvent::State(s) => {
                let old = std::mem::replace(&mut self.idle.logind_state, s);
                self.idle.machine.set_delay_max(s.delay_max);
                let mut fx = Vec::new();
                if old.idle_inhibited != s.idle_inhibited {
                    fx.extend(self.idle.machine.set_inhibitor(Inhibitor::Logind, s.idle_inhibited));
                }
                if old.locked_hint != s.locked_hint {
                    fx.extend(self.idle.machine.set_locked(s.locked_hint));
                }
                // Ekran okuyucu (framework ADR-0011 tehdit modeli): kilitliyken a11y bus'ındaki
                // bir süreç kabuğun ağacını (bildirim, pano) okuyamaz, düğmelerine bastıramaz.
                cx.set_a11y_suspended(s.locked_hint);
                fx
            }
            LogindEvent::PrepareForSleep(start) => {
                tracing::info!(start, "logind: uyku");
                if !start {
                    // Uyanma: uykuda duran monoton timer'a güvenilmez; gece ışığı hemen yeniden hesaplar.
                    self.night_light_update(cx);
                }
                self.idle.machine.prepare_for_sleep(start, Instant::now())
            }
            LogindEvent::SleepDelayHeld(held) => {
                tracing::debug!(held, "uyku gecikme inhibitörü");
                self.idle.delay_held = held;
                Vec::new()
            }
            LogindEvent::LockRequested => {
                tracing::info!("logind: kilit isteği");
                self.lock_trigger(cx, LockTrigger::LogindSignal);
                Vec::new()
            }
            LogindEvent::Failed { action, reason } => {
                tracing::warn!(?action, ?reason, "boşta: logind eylemi başarısız");
                if action == LogindAction::Lock {
                    self.lock_trigger(cx, LockTrigger::LockFailed);
                }
                Vec::new()
            }
        };
        self.apply_idle(cx, fx);
    }

    /// Sistem modeli değişti: medya çalıyor mu, AC'de mi.
    pub(super) fn sync_idle_system_and_apply(&mut self, cx: &mut Context<Self>) {
        let fx = self.sync_idle_system();
        self.apply_idle(cx, fx);
    }

    fn sync_idle_system(&mut self) -> Vec<Effect> {
        let playing = self.model.system.players.iter().any(|p| p.status == PlaybackStatus::Playing);
        let on_ac = self.model.system.power.ac_online();
        let m = &mut self.idle.machine;
        let mut fx = m.set_inhibitor(Inhibitor::Media, playing);
        if m.on_ac() != on_ac {
            fx.extend(m.set_on_ac(on_ac));
        }
        fx
    }

    /// shell.toml yeniden yüklendi.
    pub(super) fn sync_idle_config(&mut self, cx: &mut Context<Self>) {
        let fx = self.idle.machine.set_settings(self.model.config.idle.settings());
        self.apply_idle(cx, fx);
    }

    pub(super) fn idle_command(&mut self, cx: &mut Context<Self>, c: IdleCommand) -> Result<String, String> {
        match c {
            IdleCommand::Status => Ok(self.idle_status_json(cx.idle_support())),
            IdleCommand::Inhibit(t) => {
                let on = match t {
                    Toggle::On => true,
                    Toggle::Off => false,
                    Toggle::Toggle => !self.idle.machine.manual_inhibit(),
                };
                tracing::info!(on, "kahve modu");
                let fx = self.idle.machine.set_inhibitor(Inhibitor::Manual, on);
                self.apply_idle(cx, fx);
                if std::mem::replace(&mut self.model.idle_inhibit, on) != on {
                    self.model_changed(cx);
                }
                Ok(if on { "inhibited".into() } else { "uninhibited".into() })
            }
        }
    }

    fn idle_status_json(&self, support: IdleSupport) -> String {
        let mut v = serde_json::to_value(self.idle.machine.status()).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.insert("source".into(), status_source(self.idle.watches.source(), support).into());
            let watching: Vec<&str> = self.idle.watches.stages().map(|(s, _)| s.name()).collect();
            o.insert("watching".into(), watching.into());
            o.insert("logind".into(), self.idle.logind_state.available.into());
            o.insert("manual".into(), self.idle.machine.manual_inhibit().into());
            // Makinenin istediği ile logind'de gerçekten tutulan ayrı: logind yoksa tutulmaz.
            if let Some(wanted) = o.insert("sleep_delay".into(), self.idle.delay_held.into()) {
                o.insert("sleep_delay_wanted".into(), wanted);
            }
        }
        v.to_string()
    }

    fn apply_idle(&mut self, cx: &mut Context<Self>, effects: Vec<Effect>) {
        for e in effects {
            tracing::debug!(?e, "boşta etkisi");
            match e {
                Effect::Watch(stages) => self.arm_idle_watches(cx, &stages),
                Effect::Dim(on) => self.set_dim(cx, on),
                Effect::Screens(on) => self.dispatch_dpms(on),
                Effect::Lock => match &self.idle.logind {
                    Some(l) if self.idle.lock_listener_ready() => drop(l.lock()),
                    _ => {
                        tracing::warn!("boşta: logind kilidi iletemez, kilit doğrudan başlatılıyor");
                        self.lock_trigger(cx, LockTrigger::LockFailed);
                    }
                },
                Effect::Suspend => match &self.idle.logind {
                    Some(l) => drop(l.suspend()),
                    None => tracing::warn!("boşta: logind yok, uyunamadı"),
                },
                Effect::IdleHint(idle) => {
                    if let Some(l) = &self.idle.logind {
                        l.set_idle_hint(idle);
                    }
                }
                Effect::SleepDelay(hold) => {
                    if let Some(l) = &self.idle.logind {
                        l.set_sleep_delay(hold);
                    }
                }
            }
        }
        self.rearm_idle_timer(cx);
    }

    /// Compositor izlemelerini aşamalarla eşler (hepsi `Respect`: tam ekran video gibi Wayland
    /// idle inhibitor'ları karartma/kilit/uykuyu durdurur). Kalan aşama yeniden kaydedilir
    /// (sayaç şimdiden), kalkan bırakılır; eşleme [`Watches`]'ta.
    fn arm_idle_watches(&mut self, cx: &mut Context<Self>, stages: &[(Stage, Duration)]) {
        self.idle.watches.sync(&mut CxWatcher(cx), stages);
        tracing::debug!(?stages, source = ?self.idle.watches.source(), "boşta: izlemeler kuruldu");
    }

    /// `App::idle`: izleme durum değiştirdi.
    pub(super) fn on_idle_watch(&mut self, cx: &mut Context<Self>, id: IdleWatchId, idle: bool) {
        let Some(stage) = self.idle.watches.stage(id) else {
            tracing::debug!(?id, idle, "boşta: bırakılmış izlemenin olayı");
            return;
        };
        tracing::debug!(?stage, idle, "boşta: compositor");
        let fx = self.idle.machine.idle(stage, idle);
        self.apply_idle(cx, fx);
    }

    /// Ekran gücü isteği tek bir sıralı thread'e gider (socket1 zaman aşımına kadar bloklayabilir:
    /// ana döngü dışında). İstek başına thread açılsaydı hızlı kapat→aç sırası tersine dönüp
    /// ekranlar kapalı kalabilirdi; thread bekleyenleri son isteğe indirger.
    fn dispatch_dpms(&mut self, on: bool) {
        let Some(client) = self.client.clone() else {
            tracing::debug!(on, "boşta: Hyprland yok, ekran gücü değiştirilmedi");
            return;
        };
        if let Some(tx) = &self.idle.dpms
            && tx.send(on).is_ok()
        {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        let spawned = std::thread::Builder::new().name("nuxut-dpms".into()).spawn(move || {
            while let Ok(mut on) = rx.recv() {
                while let Ok(next) = rx.try_recv() {
                    on = next;
                }
                if let Err(e) = client.dispatch_dpms(on) {
                    tracing::warn!(on, "ekran gücü değiştirilemedi: {e}");
                }
            }
        });
        match spawned {
            Ok(_) => {
                // Yeni kanal az önce yaratıldı ve alıcı thread'de: gönderim başarısız olamaz.
                let _ = tx.send(on);
                self.idle.dpms = Some(tx);
            }
            Err(e) => tracing::warn!("dpms thread'i açılamadı: {e}"),
        }
    }

    fn rearm_idle_timer(&mut self, cx: &mut Context<Self>) {
        let want = self.idle.machine.deadline();
        if self.idle.timer.map(|(_, at)| at) == want {
            return;
        }
        if let Some((id, _)) = self.idle.timer.take() {
            cx.cancel_timer(id);
        }
        if let Some(at) = want {
            let id = cx.set_timer(at.saturating_duration_since(Instant::now()));
            self.idle.timer = Some((id, at));
        }
    }

    pub(super) fn idle_timer_fired(&mut self, cx: &mut Context<Self>, id: TimerId) -> bool {
        if self.idle.timer.is_none_or(|(t, _)| t != id) {
            return false;
        }
        self.idle.timer = None;
        let fx = self.idle.machine.tick(Instant::now());
        self.apply_idle(cx, fx);
        true
    }

    // --- karartma örtüsü

    fn set_dim(&mut self, cx: &mut Context<Self>, on: bool) {
        for (_, id) in self.idle.dims.drain(..) {
            cx.destroy_surface(id);
        }
        self.idle.dim_since = None;
        if !on {
            return;
        }
        for s in &self.screens {
            let spec = LayerSurfaceSpec {
                layer: Layer::Overlay,
                anchor: Anchor::ALL,
                exclusive_zone: -1,
                keyboard: KeyboardMode::None,
                ..LayerSurfaceSpec::new(DIM_NAMESPACE, s.output)
            };
            let id = cx.create_layer_surface(spec);
            cx.a11y_exclude(id); // karartma örtüsü: içeriksiz
            // Girdi örtünün altına geçer: compositor onu etkinlik sayar ve `resumed` gönderir.
            cx.set_input_region(id, &[]);
            self.idle.dims.push((s.output, id));
        }
        self.idle.dim_since = Some(Instant::now());
    }

    pub(super) fn idle_output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        self.idle.dims.retain(|&(o, id)| {
            if o == output {
                cx.destroy_surface(id);
            }
            o != output
        });
    }

    /// Karartma örtüsünü çizer; bu yüzey örtü değilse `false`.
    pub(super) fn draw_idle(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) -> bool {
        if !self.idle.dims.iter().any(|&(_, id)| id == surface) {
            return false;
        }
        let Some(size) = cx.surface_size(surface) else { return true };
        let since = self.idle.dim_since.unwrap_or(now);
        let t = (now.saturating_duration_since(since).as_secs_f32() / DIM_FADE.as_secs_f32()).clamp(0.0, 1.0);
        let color = cx.theme().colors.shadow.with_alpha(DIM_ALPHA * t);
        painter.rect(Rect::new(0.0, 0.0, size.width, size.height), &RectStyle::filled(color, Corners::ZERO));
        if t < 1.0 {
            cx.request_frame(surface);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instance_never_reaches_real_logind() {
        assert!(logind_allowed(None, None), "kullanıcının shell'i");
        assert!(logind_allowed(Some(""), None));
        assert!(!logind_allowed(Some("test-idle"), None), "test örneği gerçek sistem bus'ında");
        assert!(!logind_allowed(Some("test-idle"), Some("")));
        assert!(logind_allowed(Some("test-idle"), Some("unix:path=/tmp/x/bus")), "sahte bus'a yönlendirilmiş");
    }

    #[test]
    fn status_source_names() {
        let yes = IdleSupport::Supported { ignore_inhibitors: true };
        assert_eq!(status_source(WatchSource::Idle, yes), "compositor", "eylem kapalı, destek var");
        assert_eq!(status_source(WatchSource::Idle, IdleSupport::Unsupported), "unsupported");
        assert_eq!(status_source(WatchSource::Compositor, yes), "compositor");
        assert_eq!(status_source(WatchSource::Unsupported, yes), "unsupported", "kurulamadıysa görünür");
    }
}
