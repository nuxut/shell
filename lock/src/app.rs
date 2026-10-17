//! Wayland uygulaması: durum makinesini ([`state::Machine`]) runtime'ın oturum kilidine
//! (ext-session-lock-v1), PAM thread'ine, fprintd'ye ve görünüme bağlar.
//!
//! Burada güvenlik kararı verilmez: her olay makineye gider, makinenin eylemleri yürütülür.
//! Kilidi açan tek çağrı [`Action::Unlock`] → `cx.unlock_session()` (runtime yalnızca `Locked`
//! iken kabul eder). Kilitli değilken `unlock_session` çağrılmaz; çökme ya da sinyalde runtime
//! oturumu kilitli bırakır.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nuxut::ui::TextInputEvent;
use nuxut::wayland::{LockEnd, LockError, LockState};
use nuxut::{
    App, Context, ImeEvent, Key, KeyEvent, KeyState, Modifiers, OutputId, OutputInfo, Painter,
    PointerEvent, PointerKind, Size, SurfaceId, TextInputState, TextPurpose, TimerId, ipc,
};
use nuxut_pam::{AuthResult, SecretBuf};

use crate::auth::{Authenticator, spawn_attempt};
use crate::config::LockConfig;
use crate::fingerprint::{Fingerprint, FingerprintOptions, FpEvent};
use crate::images::LockImages;
use crate::logind::HintWorker;
use crate::state::{
    Action, AttemptId, EXIT_NOT_LOCKED, Event, Machine, PRESENT_TIMEOUT, Phase, UNLOCK_TIMEOUT,
    PresentTracker,
};
use crate::view::{OutputView, Shared};

/// Arka plan thread'lerinden gelenler.
#[derive(Debug)]
pub enum Msg {
    Auth(AttemptId, Option<AuthResult>),
    Fingerprint(FpEvent),
    /// Avatar çözüldü.
    Avatar(nuxut::render::ImageData),
    /// Duvar kağıdı yüklendi ya da kare istedi.
    Redraw,
    /// Ev sahibi görünümün açma animasyonu bitti (draw içinden; yapısal iş orada yapılmaz).
    UnlockFaded,
}

/// Başlatma parametreleri (main'de, kilitlemeden önce hazırlanır).
pub struct Setup {
    pub config: LockConfig,
    pub user: String,
    pub display_name: String,
    pub home: std::path::PathBuf,
    pub auth: Arc<dyn Authenticator>,
    /// logind `SetLockedHint` ve fprintd (sistem bus'ı). Yalnızca sahte test kipinde kapalı.
    pub system_bus: bool,
    /// `--ready-fd`: `locked` + tüm kilit yüzeylerinin ilk karesinden sonra "locked\n".
    pub ready_fd: Option<std::os::fd::OwnedFd>,
    pub wallpaper: Option<crate::assets::WallpaperSource>,
    /// `run` uygulamayı düşürünce çıkış kodu buraya yazılır ([`LockApp::exit_code`]).
    pub exit: Arc<std::sync::atomic::AtomicI32>,
    /// Canlı test kancası; main yalnızca fake-auth derlemesinde ve sahte doğrulayıcı
    /// etkinken (adlandırılmış örnek) doldurur, aksi hâlde hep boş.
    pub test_hold: TestHold,
}

/// İkincil örnek yollarını deterministik sınamak için evre bekletme (yalnızca canlı test).
#[derive(Debug, Default, Clone, Copy)]
pub struct TestHold {
    /// `lock_session` bu kadar geciktirilir: evre `Pending` kalır (`status` → `pending`).
    pub pending: Option<Duration>,
    /// Makine bittikten sonra çıkış bu kadar geciktirilir (`status` → `done`).
    pub done: Option<Duration>,
}

/// Kancanın beklettiği iş.
#[derive(Debug, Clone, Copy)]
enum Held {
    Lock,
    Exit,
}

impl std::fmt::Debug for Setup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Setup").finish_non_exhaustive()
    }
}

pub struct LockApp {
    setup: Setup,
    machine: Machine,
    /// Süreç bu kodla çıkacak.
    exit_code: Option<i32>,
    shared: Option<Shared>,
    /// Duvar kağıdı ve avatarın yalıtılmış çözücüleri (süreç başına tek; görünümler paylaşır).
    images: LockImages,
    views: HashMap<SurfaceId, OutputView>,
    /// Giriş alanının çizildiği kilit yüzeyi.
    host: Option<SurfaceId>,
    /// Klavye odağı bir kez geldiyse işaretçi ev sahibini değiştirmez.
    keyboard_seen: bool,
    fingerprint: Option<Fingerprint>,
    hint: Option<HintWorker>,
    started: bool,
    clock_timer: Option<TimerId>,
    backoff_timer: Option<TimerId>,
    unlock_timer: Option<TimerId>,
    hold_timer: Option<(TimerId, Held)>,
    present_timer: Option<TimerId>,
    /// Kilit yüzeylerinin ilk karesi compositor'da sunuldu mu (`surface_presented`).
    presented: PresentTracker<SurfaceId>,
}

impl Drop for LockApp {
    fn drop(&mut self) {
        self.setup
            .exit
            .store(self.exit_code(), std::sync::atomic::Ordering::SeqCst);
    }
}

impl std::fmt::Debug for LockApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockApp")
            .field("phase", &self.machine.phase())
            .field("views", &self.views.len())
            .finish_non_exhaustive()
    }
}

impl LockApp {
    pub fn new(setup: Setup) -> Self {
        Self {
            setup,
            machine: Machine::new(),
            exit_code: None,
            shared: None,
            views: HashMap::new(),
            images: LockImages::new(),
            host: None,
            keyboard_seen: false,
            fingerprint: None,
            hint: None,
            started: false,
            clock_timer: None,
            backoff_timer: None,
            unlock_timer: None,
            hold_timer: None,
            present_timer: None,
            presented: PresentTracker::new(),
        }
    }

    /// `run` döndükten sonra süreç çıkış kodu. Makine bitmeden döndüyse (bağlantı koptu)
    /// başarı sayılmaz.
    pub fn exit_code(&self) -> i32 {
        self.exit_code.unwrap_or(match self.machine.phase() {
            Phase::Done(code) => code,
            _ => 2,
        })
    }

    fn feed(&mut self, cx: &mut Context<Self>, event: Event) {
        let now = Instant::now();
        let actions = self.machine.handle(event, now);
        for action in actions {
            self.perform(cx, action, now);
        }
        self.sync_input(cx);
        self.redraw_all(cx);
    }

    fn perform(&mut self, cx: &mut Context<Self>, action: Action, now: Instant) {
        match action {
            Action::LockSession => {
                if let Some(delay) = self.setup.test_hold.pending.take() {
                    tracing::warn!(?delay, "test kancası: kilit isteği bekletiliyor (pending)");
                    self.hold_timer = Some((cx.set_timer(delay), Held::Lock));
                    return;
                }
                self.request_lock(cx, now);
            }
            Action::NotifyLocked => {
                if let Some(t) = self.present_timer.take() {
                    cx.cancel_timer(t);
                }
                tracing::info!("kilit kuruldu: ilk kareler sunuldu");
                self.on_locked(cx);
            }
            Action::WaitPresented(delay) => {
                self.present_timer = Some(cx.set_timer(delay));
            }
            Action::PresentTimedOut => {
                tracing::error!(
                    timeout = ?PRESENT_TIMEOUT,
                    "ilk kareler süresinde sunulmadı: kilit kurulu bildirilmedi (ready-fd, \
                     LockedHint yok); oturum kilitli kalır"
                );
            }
            Action::StartAuth(id) => {
                tracing::info!(id, "doğrulama başladı");
                self.start_auth(cx, id);
            }
            Action::ClearInput => {
                if let Some(shared) = self.shared.as_mut() {
                    shared.input.set_text("");
                }
            }
            Action::Shake => {
                let theme = cx.theme();
                if let Some(view) = self.host.and_then(|h| self.views.get_mut(&h)) {
                    view.shake(theme, now);
                }
            }
            Action::StartUnlockAnimation => {
                for view in self.views.values_mut() {
                    view.begin_unlock(now);
                }
                // Kare gelmezse (ekran kapalı) yine de aç.
                self.unlock_timer = Some(cx.set_timer(UNLOCK_TIMEOUT));
            }
            Action::Unlock => {
                // `LockedHint=false` yalnızca açma gerçekten başarılıysa: reddedilen açmada oturum
                // kilitli kalır ve dinleyiciler kilidin kalktığını sanmamalı.
                if cx.lock_state() != LockState::Locked {
                    return;
                }
                // Runtime `unlock_and_destroy`'u compositor işleyene dek bekler.
                match cx.unlock_session() {
                    Ok(()) => {
                        if let Some(hint) = &self.hint {
                            hint.set_and_wait(false, Duration::from_secs(1));
                        }
                    }
                    Err(e) => tracing::error!(error = ?e, "unlock_session reddedildi"),
                }
            }
            // Runtime, `locked` gelmemiş kilidi kendisi `destroy` eder (lock_finished/quit).
            Action::DestroyLock => {}
            Action::WakeAfter(delay) => {
                self.arm_backoff_timer(cx, delay);
            }
            Action::StopFingerprint => {
                if let Some(mut fp) = self.fingerprint.take() {
                    fp.stop();
                }
                if let Some(shared) = self.shared.as_mut() {
                    shared.fingerprint = None;
                }
            }
            Action::Exit(code) => {
                tracing::info!(code, "çıkılıyor");
                self.exit_code = Some(code);
                if let Some(delay) = self.setup.test_hold.done.take() {
                    tracing::warn!(?delay, "test kancası: çıkış bekletiliyor (done)");
                    self.hold_timer = Some((cx.set_timer(delay), Held::Exit));
                    return;
                }
                cx.quit();
            }
        }
    }

    fn request_lock(&mut self, cx: &mut Context<Self>, now: Instant) {
        // Kilit yüzeyleri ekran okuyucuya açık (ADR-0011 madde 8): AT yalnız okur
        // (`OutputView::access`, `accessibility_action`).
        cx.set_a11y_lock_surfaces(true);
        if let Err(e) = cx.lock_session() {
            tracing::error!(error = ?e, "oturum kilitlenemedi");
            let event = match e {
                // Zaten bizim kilidimiz varsa yapacak bir şey yok.
                LockError::AlreadyActive => return,
                // InDraw: perform draw dışından çağrılır; olursa kilit hiç olmadı, çıkış 1.
                LockError::Unsupported | LockError::NotLocked | LockError::InDraw => {
                    Event::CompositorFinished
                }
            };
            let actions = self.machine.handle(event, now);
            for a in actions {
                self.perform(cx, a, now);
            }
        }
    }

    fn on_locked(&mut self, cx: &mut Context<Self>) {
        if let Some(fd) = self.setup.ready_fd.take()
            && let Err(e) = nuxut_pam::process::write_ready(fd)
        {
            tracing::warn!("hazır fd'sine yazılamadı: {}", e.kind());
        }
        if !self.setup.system_bus {
            return;
        }
        match HintWorker::spawn(None, Duration::from_secs(2)) {
            Ok(hint) => {
                hint.set(true);
                self.hint = Some(hint);
            }
            Err(e) => tracing::warn!("logind işçisi başlatılamadı: {e}"),
        }
        if self.setup.config.fingerprint {
            let sender = cx.sender();
            let options = FingerprintOptions {
                bus_address: None,
                user: self.setup.user.clone(),
                tries: self.setup.config.fingerprint_tries,
                call_timeout: Duration::from_secs(5),
            };
            match Fingerprint::spawn(options, move |e| {
                let _ = sender.send(Msg::Fingerprint(e));
            }) {
                Ok(fp) => self.fingerprint = Some(fp),
                Err(e) => tracing::warn!("parmak izi thread'i başlatılamadı: {e}"),
            }
        }
    }

    fn start_auth(&mut self, cx: &mut Context<Self>, id: AttemptId) {
        let mut secret = SecretBuf::new();
        // Alan içeriği doğrudan sabit kapasiteli, mlock'lu tampona; arada String yok.
        let copied = match self.shared.as_mut() {
            Some(shared) => {
                let r = shared.input.with_secret(|s| secret.push_str(s));
                shared.input.set_text("");
                r.is_ok()
            }
            None => false,
        };
        if !copied {
            drop(secret);
            self.feed(cx, Event::AuthFinished { id, result: None });
            return;
        }
        let sender = cx.sender();
        spawn_attempt(
            Arc::clone(&self.setup.auth),
            id,
            secret,
            move |id, result| {
                let _ = sender.send(Msg::Auth(id, result));
            },
        );
    }

    fn arm_backoff_timer(&mut self, cx: &mut Context<Self>, left: Duration) {
        if let Some(t) = self.backoff_timer.take() {
            cx.cancel_timer(t);
        }
        // Geri sayım saniyede bir güncellenir; bitişte `Tick`.
        let step = left
            .min(Duration::from_secs(1))
            .max(Duration::from_millis(10));
        self.backoff_timer = Some(cx.set_timer(step));
    }

    fn arm_clock(&mut self, cx: &mut Context<Self>) {
        use chrono::Timelike;
        let now = chrono::Local::now();
        let into = u64::from(now.second()) * 1000 + u64::from(now.timestamp_subsec_millis() % 1000);
        let left = 60_000u64.saturating_sub(into).max(50);
        self.clock_timer = Some(cx.set_timer(Duration::from_millis(left)));
    }

    /// Saat ve tarih dizgileri, saat/tarih locale'iyle (`nuxut::i18n::time_locale`:
    /// `framework.toml [locale] formats` + `hour_cycle`, yoksa `LC_TIME`). `clock_format`
    /// yazılmadıysa saat `hour_cycle`'a uyar ("13:05" / "1:05 PM"); yazıldıysa kalıp kazanır
    /// (t119). Kalıp config'te doğrulandı; yine de biçimlenemezse boş dizgi (kilit asla panik
    /// etmez).
    fn time_strings(&self) -> (String, String) {
        let now = chrono::Local::now();
        let locale = nuxut::i18n::time_locale();
        (
            crate::config::clock_text(&locale, self.setup.config.clock_format.as_deref(), &now),
            locale.format_datetime(&now, &self.setup.config.date_format).unwrap_or_default(),
        )
    }

    fn update_clock(&mut self) {
        let (clock, date) = self.time_strings();
        for view in self.views.values_mut() {
            view.set_time(&clock, &date);
        }
    }

    /// Alanın etkinliği ve IME durumu (yalnızca ev sahibinde, parola amacıyla).
    fn sync_input(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let accepts = self.machine.accepts_input(now);
        let Some(shared) = self.shared.as_mut() else {
            return;
        };
        shared.input.set_enabled(accepts);
        if let Some(host) = self.host {
            let ime = shared.input.ime_state();
            cx.set_text_input(
                host,
                accepts.then_some(TextInputState {
                    cursor_rect: ime.cursor_rect,
                    // Gizli kipte çevre metin asla gönderilmez.
                    surrounding: None,
                    purpose: TextPurpose::Password,
                    multiline: false,
                }),
            );
        }
    }

    fn set_host(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {
        if self.host == Some(surface) || !self.views.contains_key(&surface) {
            return;
        }
        if let Some(old) = self.host.replace(surface) {
            cx.set_text_input(old, None);
        }
        self.sync_input(cx);
        self.redraw_all(cx);
    }

    fn redraw_all(&self, cx: &mut Context<Self>) {
        for id in self.views.keys() {
            cx.request_frame(*id);
        }
    }

    fn key(&mut self, cx: &mut Context<Self>, event: &KeyEvent) {
        if event.state != KeyState::Pressed {
            return;
        }
        let now = Instant::now();
        let Some(shared) = self.shared.as_mut() else {
            return;
        };
        shared.caps_lock = event.modifiers.caps_lock;
        // Ctrl+U: alanı temizle (yapıştırma/kopyalama yok).
        if event.modifiers.ctrl && matches!(event.key, Key::Char('u' | 'U')) {
            shared.input.set_text("");
            self.redraw_all(cx);
            return;
        }
        let response = shared.input.key(event, now);
        match response.event {
            // Boş parola PAM denemesi başlatmaz: faillock sayacı boşa artmasın (ADR-0011 madde 8,
            // 2026-10-17). Parmak izi kendiliğinden çalışır; Enter'a gerek yok.
            Some(TextInputEvent::Submit) if shared.input.is_empty() => {}
            Some(TextInputEvent::Submit) => self.feed(cx, Event::Submit),
            Some(TextInputEvent::Cancel) => {
                shared.input.set_text("");
                self.redraw_all(cx);
            }
            // Pano yok: kopyalama, kesme ve yapıştırma gizli kipte yok sayılır.
            Some(_) => self.redraw_all(cx),
            None if response.repaint => self.redraw_all(cx),
            None => {}
        }
    }

    fn ensure_shared(&mut self, cx: &Context<Self>) {
        if self.shared.is_none() {
            self.shared = Some(Shared::new(
                cx.theme(),
                self.setup.display_name.clone(),
                nuxut_pam::MAX_SECRET_BYTES,
            ));
        }
    }

    /// Avatar yalıtılmış işçide çözülür (`images`): kilit süreci kullanıcı dosyasını kendi
    /// adres alanında ayrıştırmaz.
    fn spawn_avatar_loader(&self, cx: &Context<Self>) {
        let candidates = crate::assets::avatar_candidates(&self.setup.home, &self.setup.user);
        let sender = cx.sender();
        let images = self.images.clone();
        let spawned = std::thread::Builder::new()
            .name("nuxut-avatar".into())
            .spawn(move || {
                if let Some(image) = images.load_avatar(&candidates) {
                    let _ = sender.send(Msg::Avatar(image.into()));
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("avatar thread'i başlatılamadı: {e}");
        }
    }
}

impl App for LockApp {
    type Message = Msg;

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if self.started {
            // Kilitliyken eklenen çıkışa yüzeyi runtime açar (`lock_surface_added`).
            return;
        }
        self.started = true;
        // Arka plan thread'leri burada: `run` sinyal maskesini kurduktan sonra.
        self.spawn_avatar_loader(cx);
        self.feed(cx, Event::Start);
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, _output: OutputId) {
        // Runtime çıkışın kilit yüzeyini bu çağrıdan önce kaldırır: artık kilit yüzeyi
        // olmayan görünümler atılır. Sunulmamış bir yüzey gittiyse kalanlar sunulmuş olabilir.
        let gone: Vec<SurfaceId> = self
            .views
            .keys()
            .copied()
            .filter(|s| !cx.is_lock_surface(*s))
            .collect();
        if gone.is_empty() {
            return;
        }
        let mut all = false;
        for surface in gone {
            self.views.remove(&surface);
            all = self.presented.removed(surface);
        }
        if self.host.is_some_and(|h| !self.views.contains_key(&h)) {
            // Eski ev sahibinin yüzeyi yok; metin girişi onunla gitti. Odak gelince
            // keyboard_focus yeniden seçer; o zamana dek kalan bir yüzey taşır.
            self.host = None;
            if let Some(next) = self.views.keys().min().copied() {
                self.set_host(cx, next);
            }
        }
        tracing::info!(surfaces = self.views.len(), "kilit yüzeyi kaldırıldı");
        if all {
            self.feed(cx, Event::Presented);
        }
    }

    fn surface_presented(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {
        if !cx.is_lock_surface(surface) {
            return;
        }
        tracing::debug!(?surface, "kilit yüzeyi sunuldu");
        if self.presented.presented(surface) {
            self.feed(cx, Event::Presented);
        }
    }

    fn lock_surface_added(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        _output: OutputId,
    ) {
        self.ensure_shared(cx);
        let sender = cx.sender();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = sender.send(Msg::Redraw);
        });
        let mut view = OutputView::new(
            cx.theme(),
            &self.setup.display_name,
            self.setup.config.blur,
            self.setup.config.dim,
            self.images.wallpaper().clone(),
            self.images.builtins().clone(),
            notify,
        );
        let (clock, date) = self.time_strings();
        view.set_time(&clock, &date);
        if self.machine.phase() == Phase::Unlocking {
            view.begin_unlock(Instant::now());
        }
        self.views.insert(surface, view);
        // Kilitliyken eklenen çıkışın yüzeyi de sunulana kadar sayılır (onaydan önceyse
        // bildirimi bekletir; sonra makine ikinci `Presented`'ı yok sayar).
        self.presented.added(surface);
        if self.host.is_none() {
            self.host = Some(surface);
            self.sync_input(cx);
        }
        if self.clock_timer.is_none() {
            self.arm_clock(cx);
        }
        tracing::info!(surfaces = self.views.len(), "kilit yüzeyi");
    }

    fn session_locked(&mut self, cx: &mut Context<Self>) {
        tracing::info!(surfaces = self.views.len(), "oturum kilitlendi");
        self.feed(cx, Event::CompositorLocked);
    }

    fn lock_finished(&mut self, cx: &mut Context<Self>, end: LockEnd) {
        tracing::info!(?end, "kilit bitti");
        self.views.clear();
        self.presented.clear();
        self.host = None;
        match end {
            // Bizim açmamız: makine zaten `Done`.
            LockEnd::Unlocked => {
                if self.exit_code.is_none() {
                    self.exit_code = Some(crate::state::EXIT_UNLOCKED);
                }
                // Test kancası çıkışı bekletiyorsa zamanlayıcı bitirir.
                if !matches!(self.hold_timer, Some((_, Held::Exit))) {
                    cx.quit();
                }
            }
            LockEnd::Refused | LockEnd::CompositorEnded => {
                self.feed(cx, Event::CompositorFinished);
                if self.exit_code.is_none() {
                    // Makine olayı yok saydıysa da süreç burada biter.
                    self.exit_code = Some(EXIT_NOT_LOCKED);
                    cx.quit();
                }
            }
        }
    }

    fn surface_resized(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        size: Size,
        scale: f32,
    ) {
        let Some(view) = self.views.get_mut(&surface) else {
            return;
        };
        view.set_size(size, scale, self.setup.wallpaper.as_ref());
        cx.request_frame(surface);
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) {
        // Klavye odağı gelmediyse giriş alanı işaretçinin olduğu çıkışa taşınır.
        if !self.keyboard_seen && matches!(event.kind, PointerKind::Enter | PointerKind::Motion) {
            self.set_host(cx, surface);
        }
    }

    // AT_POLICY: ağaçta yalnız saat, tarih, ad, parola alanı (dolu/boş; değer ve uzunluk yok) ve
    // arındırılmış durum metni; `view.rs` `OutputView::access`.
    fn accessibility(&mut self, _cx: &mut Context<Self>, surface: SurfaceId, tree: &mut nuxut::access::AccessTree) {
        let (Some(view), Some(shared)) = (self.views.get(&surface), self.shared.as_ref()) else { return };
        let waiting = self.machine.backoff_remaining(Instant::now()).is_some();
        view.access(tree, shared, self.machine.status(), waiting, self.host == Some(surface));
    }

    // AT_POLICY: kilit ekranında hiçbir AT eylemi uygulanmaz (ağaçta da sunulmaz). Odak hep
    // parola alanında (gerçek klavye odağı); kilidi açma ve parola yalnız gerçek klavyeyle.
    // a11y bus'ındaki herhangi bir süreç (Flatpak dahil) eylem gönderebilir: AT eylemi kullanıcı
    // niyeti değildir. Reddedilen eylem loglanmaz (kilitliyken AT olayı günlüğe yazılmaz).
    fn accessibility_action(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _request: nuxut::access::AccessRequest) {}

    fn keyboard_focus(&mut self, cx: &mut Context<Self>, surface: SurfaceId, focused: bool) {
        tracing::debug!(focused, lock = cx.is_lock_surface(surface), "klavye odağı");
        if focused && cx.is_lock_surface(surface) {
            self.keyboard_seen = true;
            self.set_host(cx, surface);
        }
        if let Some(shared) = self.shared.as_mut() {
            shared.input.set_focused(true, Instant::now());
        }
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        if !cx.is_lock_surface(surface) {
            return;
        }
        if let Some(shared) = self.shared.as_mut()
            && !shared.input.is_focused()
        {
            shared.input.set_focused(true, Instant::now());
        }
        self.key(cx, &event);
        self.sync_input(cx);
    }

    fn modifiers_changed(
        &mut self,
        cx: &mut Context<Self>,
        _surface: SurfaceId,
        modifiers: Modifiers,
    ) {
        tracing::debug!(caps_lock = modifiers.caps_lock, "değiştiriciler");
        if let Some(shared) = self.shared.as_mut()
            && shared.caps_lock != modifiers.caps_lock
        {
            shared.caps_lock = modifiers.caps_lock;
            self.redraw_all(cx);
        }
    }

    fn ime(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: ImeEvent) {
        if Some(surface) != self.host || !self.machine.accepts_input(Instant::now()) {
            return;
        }
        let Some(shared) = self.shared.as_mut() else {
            return;
        };
        match event {
            ImeEvent::Preedit { text, cursor } => {
                let _ = shared.input.ime_preedit(&text, cursor);
            }
            ImeEvent::Commit(text) => {
                let _ = shared.input.ime_commit(&text);
            }
            ImeEvent::DeleteSurrounding { before, after } => {
                let _ = shared.input.ime_delete_surrounding(before, after);
            }
        }
        self.redraw_all(cx);
    }

    fn message(&mut self, cx: &mut Context<Self>, message: Msg) {
        match message {
            Msg::Auth(id, result) => {
                // Yalnızca sonuç türü; şifre, uzunluğu ve PAM mesaj metni loglanmaz.
                tracing::info!(id, outcome = ?result.as_ref().map(|r| r.outcome), "doğrulama bitti");
                self.feed(cx, Event::AuthFinished { id, result });
            }
            Msg::Fingerprint(FpEvent::Matched) => {
                tracing::info!("parmak izi eşleşti");
                self.feed(cx, Event::FingerprintMatched);
            }
            Msg::Fingerprint(e) => {
                if let Some(shared) = self.shared.as_mut() {
                    shared.fingerprint = Some(e);
                }
                self.redraw_all(cx);
            }
            Msg::Avatar(data) => {
                self.ensure_shared(cx);
                if let Some(shared) = self.shared.as_mut() {
                    shared.avatar_image = Some(data);
                }
                self.redraw_all(cx);
            }
            Msg::Redraw => self.redraw_all(cx),
            Msg::UnlockFaded => {
                if self.machine.phase() == Phase::Unlocking {
                    self.feed(cx, Event::UnlockAnimationDone);
                }
            }
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.clock_timer == Some(id) {
            self.update_clock();
            self.arm_clock(cx);
            self.redraw_all(cx);
        } else if self.backoff_timer == Some(id) {
            self.backoff_timer = None;
            let now = Instant::now();
            match self.machine.backoff_remaining(now) {
                Some(left) => self.arm_backoff_timer(cx, left),
                None => self.feed(cx, Event::Tick),
            }
            self.redraw_all(cx);
        } else if self.unlock_timer == Some(id) {
            self.unlock_timer = None;
            self.feed(cx, Event::UnlockAnimationDone);
        } else if self.present_timer == Some(id) {
            self.present_timer = None;
            self.feed(cx, Event::PresentTimeout);
        } else if let Some((timer, held)) = self.hold_timer
            && timer == id
        {
            self.hold_timer = None;
            match held {
                Held::Lock if self.machine.phase() == Phase::Pending => {
                    self.request_lock(cx, Instant::now());
                }
                Held::Lock => {}
                Held::Exit => cx.quit(),
            }
        }
    }

    fn terminate(&mut self, cx: &mut Context<Self>) {
        // Kilitliyken makine sinyali yok sayar; runtime da kilidi açmaz.
        if matches!(self.machine.phase(), Phase::Locked | Phase::Unlocking) {
            tracing::warn!("kilitliyken sonlandırma sinyali yok sayıldı");
        }
        self.feed(cx, Event::Terminate);
    }

    fn command(&mut self, _cx: &mut Context<Self>, args: Vec<String>, reply: ipc::Reply) {
        match args.first().map(String::as_str) {
            Some("status") | None => reply.send(match self.machine.phase() {
                Phase::Starting | Phase::Pending => "pending",
                // İlk kareler sunulmadan "locked" denmez: ikincil örnek erken çıkmasın.
                Phase::Locked | Phase::Unlocking if self.machine.is_confirmed() => "locked",
                Phase::Locked | Phase::Unlocking => "pending",
                Phase::Done(_) => "done",
            }),
            _ => reply.send("error: bilinmeyen komut"),
        }
    }

    fn draw(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        now: Instant,
        painter: &mut Painter,
    ) {
        let host = self.host == Some(surface);
        let status = self.machine.status().clone();
        let backoff = self.machine.backoff_remaining(now);
        let (Some(view), Some(shared)) = (self.views.get_mut(&surface), self.shared.as_mut())
        else {
            // Görünüm yoksa bile tampon opak olmalı.
            let size = cx.surface_size(surface).unwrap_or(Size::ZERO);
            painter.rect(
                nuxut::Rect::new(0.0, 0.0, size.width, size.height),
                &nuxut::RectStyle::filled(
                    cx.theme().colors.surface.with_alpha(1.0),
                    nuxut::Corners::ZERO,
                ),
            );
            return;
        };
        let more = view.draw(painter, cx.theme(), now, shared, &status, backoff, host);
        let faded = host && view.faded_out(now);
        if more {
            cx.request_frame(surface);
        }
        if faded && self.machine.phase() == Phase::Unlocking {
            let _ = cx.sender().send(Msg::UnlockFaded);
        }
    }
}
