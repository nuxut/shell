//! Wayland uygulaması: her çıkışta tam ekran katman yüzeyi (overlay, özel klavye); arka plan
//! kilit ekranının görünümü, form yalnızca ev sahibi çıkışta. Olaylar durum makinesine
//! ([`flow::Machine`]) gider, eylemleri burada yürütülür; greetd soketi ayrı thread'de.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nuxut::wayland::KeyboardMode;
use nuxut::{
    Anchor, Animated, App, Context, ImeEvent, KeyEvent, KeyState, Layer, LayerSurfaceSpec, Modifiers,
    OutputId, OutputInfo, Painter, PointerEvent, PointerKind, Size, SurfaceId, TextInputState, TextPurpose,
    TimerId, ipc,
};
use nuxut_lock::assets::WallpaperSource;
use nuxut_lock::config::LockConfig;
use nuxut_lock::images::LockImages;
use nuxut_lock::view::{OutputView, Shared};
use nuxut_pam::SecretBuf;

use crate::client::{Client, ClientEvent, Command};
use crate::flow::{Action, Event, Machine, Phase};
use crate::power::{self, PowerAction};
use crate::protocol::Request;
use crate::remember::{self, Remembered};
use crate::sessions::Session;
use crate::users::User;
use crate::view::{Form, FormState, Intent, placeholder_for};

/// Oturum başlarken içerik bu sürede solmazsa (kare gelmiyor) yine de çıkılır.
const EXIT_TIMEOUT: Duration = Duration::from_millis(400);

#[derive(Debug)]
pub enum Msg {
    Greetd(ClientEvent),
    Avatar(String, nuxut::render::ImageData),
    Power(Vec<PowerAction>),
    Redraw,
}

/// Başlatma parametreleri (main'de hazırlanır).
#[derive(Debug)]
pub struct Setup {
    pub socket: PathBuf,
    pub users: Vec<User>,
    pub sessions: Vec<Session>,
    pub remembered: Remembered,
    pub state_dir: PathBuf,
    pub config: LockConfig,
    pub wallpaper: Option<WallpaperSource>,
    /// logind güç düğmeleri (sistem bus'ı). Test kipinde kapalı.
    pub system_bus: bool,
    pub icons_dir: PathBuf,
    /// Sistem klavye düzenleri (başlatıcıdan); `None` → düğme yok.
    pub layouts: Option<crate::keyboard::Layouts>,
    /// Test kipi: logind yerine bu güç düğmeleri gösterilir, basılınca yalnız loglanır.
    pub fake_power: Option<Vec<PowerAction>>,
}

pub struct GreeterApp {
    setup: Setup,
    machine: Machine,
    client: Option<Client>,
    images: LockImages,
    views: HashMap<SurfaceId, OutputView>,
    surfaces: HashMap<OutputId, SurfaceId>,
    host: Option<SurfaceId>,
    keyboard_seen: bool,
    form: Option<Form>,
    /// Arka plan görünümünün istediği (çizilmeyen) giriş alanı.
    background: Option<Shared>,
    user: Option<usize>,
    session: usize,
    power: Vec<PowerAction>,
    layouts: Option<crate::keyboard::Layouts>,
    caps_lock: bool,
    shake: Option<Animated<f32>>,
    exiting: bool,
    loaded_avatars: Vec<String>,
    clock_timer: Option<TimerId>,
    backoff_timer: Option<TimerId>,
    exit_timer: Option<TimerId>,
    started: bool,
}

impl std::fmt::Debug for GreeterApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GreeterApp")
            .field("phase", self.machine.phase())
            .field("views", &self.views.len())
            .finish_non_exhaustive()
    }
}

impl GreeterApp {
    pub fn new(setup: Setup) -> Self {
        let user = setup
            .remembered
            .user
            .as_ref()
            .and_then(|u| setup.users.iter().position(|x| &x.name == u))
            .or((!setup.users.is_empty()).then_some(0));
        let session = setup
            .remembered
            .session
            .as_ref()
            .and_then(|s| setup.sessions.iter().position(|x| &x.id == s))
            .unwrap_or(0);
        let layouts = setup.layouts.clone();
        Self {
            setup,
            machine: Machine::new(),
            client: None,
            images: LockImages::new(),
            views: HashMap::new(),
            surfaces: HashMap::new(),
            host: None,
            keyboard_seen: false,
            form: None,
            background: None,
            user,
            session,
            power: Vec::new(),
            layouts,
            caps_lock: false,
            shake: None,
            exiting: false,
            loaded_avatars: Vec::new(),
            clock_timer: None,
            backoff_timer: None,
            exit_timer: None,
            started: false,
        }
    }

    fn current_user(&self) -> Option<&User> {
        self.user.and_then(|i| self.setup.users.get(i))
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

    fn send(&mut self, command: Command) {
        let sent = self.client.as_ref().is_some_and(|c| c.send(command));
        if !sent {
            tracing::error!("greetd thread'i yok");
        }
    }

    fn perform(&mut self, cx: &mut Context<Self>, action: Action, now: Instant) {
        match action {
            Action::Send(request) => {
                if let Request::CreateSession { .. } = &request {
                    tracing::info!("oturum isteniyor");
                }
                self.send(Command::Request(request));
            }
            Action::SendInput => {
                let mut secret = SecretBuf::new();
                // Alan içeriği doğrudan mlock'lu, sabit kapasiteli tampona; arada String yok.
                let ok = self.form.as_ref().is_some_and(|f| f.with_text(|s| secret.push_str(s)).is_ok());
                if !ok {
                    // Sığmayan/NUL'lu girdi: boş yanıt (PAM reddeder), kopya kalmaz.
                    secret.clear();
                }
                self.send(Command::Respond(Some(secret)));
            }
            Action::Acknowledge => self.send(Command::Respond(None)),
            Action::StartSession => match self.setup.sessions.get(self.session) {
                Some(s) => {
                    tracing::info!(session = %s.id, "oturum başlatılıyor");
                    let request = Request::StartSession { cmd: s.cmd.clone(), env: s.env() };
                    self.send(Command::Request(request));
                }
                None => {
                    tracing::error!("başlatılacak oturum yok");
                    // Makineyi durdurmak için hata yanıtı gibi davran.
                    let e = crate::protocol::Response::Error {
                        error_type: crate::protocol::ErrorType::Error,
                        description: "oturum yok".into(),
                    };
                    let actions = self.machine.handle(Event::Response(e), now);
                    for a in actions {
                        self.perform(cx, a, now);
                    }
                }
            },
            Action::ClearInput => {
                if let Some(f) = self.form.as_mut() {
                    f.clear();
                }
            }
            Action::Shake => {
                if let Some(s) = self.shake.as_mut() {
                    s.kick(cx.theme().space(1) * 400.0, now);
                }
            }
            Action::WakeAfter(delay) => self.arm_backoff_timer(cx, delay),
            Action::Exit => {
                self.remember();
                self.exiting = true;
                for v in self.views.values_mut() {
                    v.begin_unlock(now);
                }
                self.exit_timer = Some(cx.set_timer(EXIT_TIMEOUT));
            }
        }
    }

    fn remember(&self) {
        let value = Remembered {
            user: self.current_user().map(|u| u.name.clone()),
            session: self.setup.sessions.get(self.session).map(|s| s.id.clone()),
        };
        if let Err(e) = remember::save(&self.setup.state_dir, &value) {
            tracing::debug!("son seçim kaydedilemedi: {}", e.kind());
        }
    }

    fn arm_backoff_timer(&mut self, cx: &mut Context<Self>, left: Duration) {
        if let Some(t) = self.backoff_timer.take() {
            cx.cancel_timer(t);
        }
        let step = left.min(Duration::from_secs(1)).max(Duration::from_millis(10));
        self.backoff_timer = Some(cx.set_timer(step));
    }

    fn arm_clock(&mut self, cx: &mut Context<Self>) {
        use chrono::Timelike;
        let now = chrono::Local::now();
        let into = u64::from(now.second()) * 1000 + u64::from(now.timestamp_subsec_millis() % 1000);
        let left = 60_000u64.saturating_sub(into).max(50);
        self.clock_timer = Some(cx.set_timer(Duration::from_millis(left)));
    }

    fn time_strings(&self) -> (String, String) {
        let now = chrono::Local::now();
        let locale = nuxut::i18n::time_locale();
        (
            nuxut_lock::config::clock_text(&locale, self.setup.config.clock_format.as_deref(), &now),
            locale.format_datetime(&now, &self.setup.config.date_format).unwrap_or_default(),
        )
    }

    fn update_clock(&mut self) {
        let (clock, date) = self.time_strings();
        for view in self.views.values_mut() {
            view.set_time(&clock, &date);
        }
    }

    fn sync_input(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let (enabled, secret) = match self.machine.phase() {
            Phase::Prompt { secret, .. } => (self.machine.accepts_input(now), *secret),
            // Kullanıcı listesi boşsa ad elle girilir.
            Phase::Idle => (self.user.is_none(), false),
            _ => (false, true),
        };
        let Some(form) = self.form.as_mut() else { return };
        form.set_secret_mode(secret, now);
        form.set_enabled(enabled, now);
        if let Some(host) = self.host {
            let ime = form.ime_state();
            cx.set_text_input(
                host,
                enabled.then_some(TextInputState {
                    cursor_rect: ime.cursor_rect,
                    surrounding: None,
                    purpose: if secret { TextPurpose::Password } else { TextPurpose::Normal },
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

    fn select_user(&mut self, cx: &mut Context<Self>, index: usize) {
        let Some(user) = self.setup.users.get(index) else { return };
        self.user = Some(index);
        let name = user.name.clone();
        self.load_avatar(cx, index);
        self.feed(cx, Event::SelectUser(name));
    }

    fn load_avatar(&mut self, cx: &Context<Self>, index: usize) {
        let Some(user) = self.setup.users.get(index) else { return };
        if self.loaded_avatars.contains(&user.name) {
            return;
        }
        self.loaded_avatars.push(user.name.clone());
        let Some(path) = user.avatar(&self.setup.icons_dir) else { return };
        let name = user.name.clone();
        let images = self.images.clone();
        let sender = cx.sender();
        let spawned = std::thread::Builder::new().name("nuxut-avatar".into()).spawn(move || {
            if let Some(image) = images.load_avatar(&[path]) {
                let _ = sender.send(Msg::Avatar(name, image.into()));
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("avatar thread'i başlatılamadı: {e}");
        }
    }

    fn intent(&mut self, cx: &mut Context<Self>, intent: Intent) {
        let n = self.setup.users.len();
        match intent {
            Intent::Submit => {
                if self.user.is_none() && *self.machine.phase() == Phase::Idle {
                    // Elle girilen kullanıcı adı.
                    let name = self.form.as_ref().map(|f| f.with_text(|s| s.trim().to_owned())).unwrap_or_default();
                    if crate::users::valid_name(&name) {
                        self.feed(cx, Event::SelectUser(name));
                    }
                } else {
                    self.feed(cx, Event::Submit);
                }
            }
            Intent::PreviousUser if n > 1 => {
                let i = self.user.map_or(0, |i| (i + n - 1) % n);
                self.select_user(cx, i);
            }
            Intent::NextUser if n > 1 => {
                let i = self.user.map_or(0, |i| (i + 1) % n);
                self.select_user(cx, i);
            }
            Intent::NextSession if !self.setup.sessions.is_empty() => {
                self.session = (self.session + 1) % self.setup.sessions.len();
                self.redraw_all(cx);
            }
            Intent::NextLayout if self.layouts.as_ref().is_some_and(crate::keyboard::Layouts::many) => {
                if let Some(index) = self.layouts.as_mut().map(crate::keyboard::Layouts::advance) {
                    let _ = std::thread::Builder::new()
                        .name("nuxut-xkb".into())
                        .spawn(move || crate::keyboard::apply(index));
                }
                self.redraw_all(cx);
            }
            Intent::Power(action) if self.setup.fake_power.is_some() => {
                tracing::info!(?action, "test kipi: güç eylemi yok sayıldı");
            }
            Intent::Power(action) if self.power.contains(&action) && !self.exiting => {
                tracing::info!(?action, "güç eylemi");
                let _ = std::thread::Builder::new().name("nuxut-power".into()).spawn(move || {
                    if let Err(e) = power::request(None, action) {
                        tracing::error!("güç eylemi reddedildi: {e}");
                    }
                });
            }
            _ => self.redraw_all(cx),
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let sender = cx.sender();
        match Client::spawn(self.setup.socket.clone(), move |e| {
            let _ = sender.send(Msg::Greetd(e));
        }) {
            Ok(c) => self.client = Some(c),
            Err(e) => tracing::error!("greetd thread'i başlatılamadı: {e}"),
        }
        if let Some(fake) = &self.setup.fake_power {
            self.power = fake.clone();
        } else if self.setup.system_bus {
            let sender = cx.sender();
            let _ = std::thread::Builder::new().name("nuxut-power".into()).spawn(move || {
                let _ = sender.send(Msg::Power(power::query(None)));
            });
        }
        if let Some(i) = self.user {
            self.select_user(cx, i);
        }
    }
}

impl App for GreeterApp {
    type Message = Msg;

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        let mut spec = LayerSurfaceSpec::new("nuxut-greeter", output.id);
        spec.layer = Layer::Overlay;
        spec.anchor = Anchor::ALL;
        spec.exclusive_zone = -1;
        spec.keyboard = KeyboardMode::Exclusive;
        let surface = cx.create_layer_surface(spec);
        let theme = cx.theme();
        if self.form.is_none() {
            self.form = Some(Form::new(theme));
            self.background = Some(Shared::new(theme, String::new(), 1));
            self.shake = Some(Animated::new(0.0, theme.motion.spatial_fast));
        }
        let sender = cx.sender();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = sender.send(Msg::Redraw);
        });
        let mut view = OutputView::new(
            cx.theme(),
            "",
            self.setup.config.blur,
            self.setup.config.dim,
            self.images.wallpaper().clone(),
            self.images.builtins().clone(),
            notify,
        );
        let (clock, date) = self.time_strings();
        view.set_time(&clock, &date);
        self.views.insert(surface, view);
        self.surfaces.insert(output.id, surface);
        if self.host.is_none() {
            self.host = Some(surface);
        }
        if self.clock_timer.is_none() {
            self.arm_clock(cx);
        }
        if !self.started {
            self.started = true;
            self.start(cx);
        }
        self.sync_input(cx);
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        let Some(surface) = self.surfaces.remove(&output) else { return };
        self.views.remove(&surface);
        cx.destroy_surface(surface);
        if self.host == Some(surface) {
            self.host = None;
            if let Some(next) = self.views.keys().min().copied() {
                self.set_host(cx, next);
            }
        }
    }

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, scale: f32) {
        let Some(view) = self.views.get_mut(&surface) else { return };
        view.set_size(size, scale, self.setup.wallpaper.as_ref());
        cx.request_frame(surface);
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) {
        if !self.keyboard_seen && matches!(event.kind, PointerKind::Enter | PointerKind::Motion) {
            self.set_host(cx, surface);
        }
        if Some(surface) != self.host || self.exiting {
            return;
        }
        let now = Instant::now();
        let theme = cx.theme().clone();
        let Some(form) = self.form.as_mut() else { return };
        if let Some(intent) = form.pointer(&theme, cx.text(), &event, now) {
            self.intent(cx, intent);
            self.sync_input(cx);
        }
    }

    fn keyboard_focus(&mut self, cx: &mut Context<Self>, surface: SurfaceId, focused: bool) {
        if focused && self.views.contains_key(&surface) {
            self.keyboard_seen = true;
            self.set_host(cx, surface);
        }
        if let Some(form) = self.form.as_mut() {
            form.set_focused(true, Instant::now());
        }
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, event: KeyEvent) {
        if event.state != KeyState::Pressed || self.exiting {
            return;
        }
        self.caps_lock = event.modifiers.caps_lock;
        let Some(form) = self.form.as_mut() else { return };
        if let Some(intent) = form.key(&event, Instant::now()) {
            self.intent(cx, intent);
        }
        self.sync_input(cx);
        self.redraw_all(cx);
    }

    fn modifiers_changed(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, modifiers: Modifiers) {
        if self.caps_lock != modifiers.caps_lock {
            self.caps_lock = modifiers.caps_lock;
            self.redraw_all(cx);
        }
    }

    fn ime(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: ImeEvent) {
        if Some(surface) != self.host || self.exiting {
            return;
        }
        let Some(form) = self.form.as_mut() else { return };
        let input = form.input_mut();
        match event {
            ImeEvent::Preedit { text, cursor } => {
                let _ = input.ime_preedit(&text, cursor);
            }
            ImeEvent::Commit(text) => {
                let _ = input.ime_commit(&text);
            }
            ImeEvent::DeleteSurrounding { before, after } => {
                let _ = input.ime_delete_surrounding(before, after);
            }
        }
        self.redraw_all(cx);
    }

    fn message(&mut self, cx: &mut Context<Self>, message: Msg) {
        match message {
            Msg::Greetd(ClientEvent::Response(r)) => {
                // Yalnızca tür; mesaj metni ve kullanıcı adı loglanmaz.
                tracing::debug!(kind = response_kind(&r), "greetd yanıtı");
                self.feed(cx, Event::Response(r));
            }
            Msg::Greetd(ClientEvent::Disconnected(e)) => {
                tracing::warn!("greetd bağlantısı yok: {e}");
                self.feed(cx, Event::Disconnected);
            }
            Msg::Avatar(user, data) => {
                if let Some(f) = self.form.as_mut() {
                    f.add_avatar(user, data);
                }
                self.redraw_all(cx);
            }
            Msg::Power(actions) => {
                self.power = actions;
                self.redraw_all(cx);
            }
            Msg::Redraw => self.redraw_all(cx),
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.clock_timer == Some(id) {
            self.update_clock();
            self.arm_clock(cx);
            self.redraw_all(cx);
        } else if self.backoff_timer == Some(id) {
            self.backoff_timer = None;
            match self.machine.backoff_remaining(Instant::now()) {
                Some(left) => self.arm_backoff_timer(cx, left),
                None => self.feed(cx, Event::Tick),
            }
            self.redraw_all(cx);
        } else if self.exit_timer == Some(id) {
            cx.quit();
        }
    }

    fn command(&mut self, _cx: &mut Context<Self>, args: Vec<String>, reply: ipc::Reply) {
        // Yalnızca test/teşhis: evre adı (kullanıcı adı ve metin yok).
        match args.first().map(String::as_str) {
            Some("status") | None => reply.send(match self.machine.phase() {
                Phase::Idle => "idle",
                Phase::Busy => "busy",
                Phase::Prompt { .. } => "prompt",
                Phase::Starting => "starting",
                Phase::Stopped => "stopped",
                Phase::Done => "done",
            }),
            _ => reply.send("error: bilinmeyen komut"),
        }
    }

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        let host = self.host == Some(surface);
        let status = self.machine.status().clone();
        let backoff = self.machine.backoff_remaining(now);
        let size = cx.surface_size(surface).unwrap_or(Size::ZERO);
        let theme = cx.theme();
        let (Some(view), Some(background)) = (self.views.get_mut(&surface), self.background.as_mut()) else {
            return;
        };
        // Kilidin arka planı (giriş alanı çizilmez: ev sahibi değil).
        let mut more = view.draw(painter, theme, now, background, &status, backoff, false);
        if host
            && let Some(form) = self.form.as_mut()
        {
            let user = self.user.and_then(|i| self.setup.users.get(i));
            let prompt = match self.machine.phase() {
                Phase::Prompt { secret, text } => Some((*secret, placeholder_for(*secret, text))),
                _ => None,
            };
            let label = self.layouts.as_ref().map(crate::keyboard::Layouts::label);
            let state = FormState {
                display_name: user.map(|u| u.display_name.as_str()),
                user_name: user.map(|u| u.name.as_str()),
                many_users: self.setup.users.len() > 1,
                prompt,
                status: &status,
                notices: self.machine.notices(),
                backoff,
                caps_lock: self.caps_lock,
                busy: matches!(self.machine.phase(), Phase::Busy | Phase::Starting) && user.is_some(),
                session: self.setup.sessions.get(self.session).map(|s| s.name.as_str()),
                power: &self.power,
                layout: label.as_deref(),
            };
            let shake = self.shake.as_ref().map_or(0.0, |s| s.value(now));
            let alpha = if self.exiting { 0.0 } else { 1.0 };
            let mut ui = nuxut::ui::Ui::new(painter, theme, now);
            ui.with_alpha(alpha, |ui| {
                more |= form.draw(ui, size, shake, &state);
            });
            more |= self.shake.as_mut().is_some_and(|s| !s.is_settled(now));
        }
        let faded = view_faded(&self.views, surface, now);
        if more {
            cx.request_frame(surface);
        }
        if self.exiting && faded && host {
            cx.quit();
        }
    }

    fn terminate(&mut self, cx: &mut Context<Self>) {
        // greetd greeter'ı sonlandırırsa (ör. yeniden başlatma) temiz çık; oturum açılmaz.
        cx.quit();
    }
}

fn view_faded(views: &HashMap<SurfaceId, OutputView>, surface: SurfaceId, now: Instant) -> bool {
    views.get(&surface).is_some_and(|v| v.faded_out(now))
}

fn response_kind(r: &crate::protocol::Response) -> &'static str {
    use crate::protocol::{AuthMessageType, Response};
    match r {
        Response::Success => "success",
        Response::Error { .. } => "error",
        Response::AuthMessage { auth_message_type, .. } => match auth_message_type {
            AuthMessageType::Secret => "secret",
            AuthMessageType::Visible => "visible",
            AuthMessageType::Info => "info",
            AuthMessageType::Error => "error-message",
        },
    }
}
