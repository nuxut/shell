//! Diyalog süreci (`nuxut-polkit dialog`): framework layer-shell `Overlay` yüzeyi,
//! `KeyboardMode::Exclusive`, [`nuxut::ui::Dialog`] modal kartı. Parolayı yalnızca bu süreç
//! görür ve yalnızca yardımcıya yazar.
//!
//! Güvenlik kararı burada verilmez: her olay [`Session`]'a gider, eylemleri yürütülür.
//! Parola akışı: `TextInput` (parola kipi, `SecretBuffer`; sıfırlanır, mlock'suz) →
//! `with_secret` ile mlock'lu [`SecretBuf`] → [`HelperSession::respond`] → alan temizlenir.
//! Bu süreçte parola kopyalayan `String` yok; framework `KeyEvent::text` ve IME
//! `Preedit`/`Commit` `SecretText` taşır (Drop'ta sıfırlanır; kalan sınırlar framework
//! ARCHITECTURE "Gizli metin girişi").
//! log/Debug yok; IME'ye çevre metin gönderilmez; pano kapalı. İlk sunumdan sonra
//! [`InputGate::DELAY`] boyunca tuş/IME yok sayılır (yazı-önü).

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nuxut::i18n::t;
use nuxut::ui::{
    Dialog, DialogAction, DialogActionKind, DialogEvent, Icon, IconName, IconView, Label, Select,
    SelectEvent, TextInput, TextInputEvent, TextRole, Ui, text_style,
};
use nuxut::wayland::{Anchor, KeyboardMode, Layer, LayerSurfaceSpec};
use nuxut::{
    Animated, App, Context, ImeEvent, Key, KeyEvent, KeyState, Modifiers, OutputId, OutputInfo,
    Painter, Point, PointerEvent, PointerKind, Rect, Size, SurfaceId, TextInputState, TextPurpose,
    TimerId,
};
use nuxut_pam::SecretBuf;

use crate::helper::{HelperEvent, HelperPaths, HelperSession};
use crate::protocol::MAX_RESPONSE_BYTES;
use crate::request::{DialogOutcome, DialogRequest, Requester};
use crate::session::{Action, ErrorText, Event, Generation, Phase, Session};
use crate::text;

/// Gösterilebilir bir kimlik.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub uid: u32,
    /// Yardımcıya yazılan giriş adı.
    pub name: String,
    /// Gösterilen ad (GECOS, yoksa giriş adı).
    pub display: String,
}

impl Identity {
    /// `getpwuid_r`; bulunamayan uid atlanır (yardımcı adı ister).
    pub fn resolve(uids: &[u32]) -> Vec<Self> {
        uids.iter()
            .filter_map(|&uid| nuxut_pam::lookup_uid(uid).ok())
            .map(|u| Self {
                uid: u.uid,
                display: text::sanitize(u.display_name(), text::MAX_SHORT_CHARS),
                name: u.name,
            })
            .collect()
    }

    /// Gösterilen etiket. Giriş adı da (yardımcıya ham hâliyle gider) diğer dış metinler gibi
    /// denetim/yön karakterlerinden temizlenerek gösterilir.
    pub fn label(&self) -> String {
        // Giriş adı tanınması gereken kimliktir: ASCII dışı/karışabilir karakter kaçışlı
        // (`\u{…}`), uzunsa ortadan kırpılır. GECOS adı (kullanıcı `chfn` ile değiştirebilir)
        // yalnızca yanında, temizlenmiş olarak gösterilir; giriş adı her zaman görünür.
        let name = text::escape_middle(&self.name, text::MAX_SHORT_CHARS);
        if self.display == self.name || self.display == name || self.display.is_empty() {
            name
        } else {
            format!("{} ({})", self.display, name)
        }
    }
}

/// Yazı-önü (type-ahead) kapısı: diyalog ilk kez **sunulduktan** sonra [`InputGate::DELAY`]
/// boyunca tuşlar (Enter ve Esc dahil) ve IME metni yok sayılır. Diyalog `Exclusive` klavye
/// odağını aldığında kullanıcının başka pencerede yazmakta olduğu metnin kalanı parola alanına
/// düşüp Enter'la gönderilmesin (pam_faillock `deny=3`: birkaç yanlış deneme hesabı kilitler).
/// Saf: zaman dışarıdan verilir (test).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputGate {
    shown_at: Option<Instant>,
    delay: Duration,
}

impl InputGate {
    pub const DELAY: Duration = Duration::from_millis(400);

    pub fn new(delay: Duration) -> Self {
        Self { shown_at: None, delay }
    }

    /// Yüzey ilk kez sunuldu; yalnızca ilk çağrı sayılır.
    pub fn shown(&mut self, now: Instant) {
        self.shown_at.get_or_insert(now);
    }

    /// Girdi kabul edilir mi: sunulmadan önce hayır, sunulduktan sonra `delay` dolunca evet.
    pub fn allows(&self, now: Instant) -> bool {
        self.shown_at
            .is_some_and(|t| now.saturating_duration_since(t) >= self.delay)
    }
}

/// Arka plan thread'lerinden gelenler.
#[derive(Debug)]
pub enum Msg {
    Helper(Generation, HelperEvent),
    /// Ajan stdin'i kapattı (ajan öldü).
    StdinClosed,
}

/// Başlatma parametreleri.
pub struct Setup {
    pub request: DialogRequest,
    pub identities: Vec<Identity>,
    pub default_identity: usize,
    pub paths: HelperPaths,
    /// İstek satırından sonra stdin (EOF izlenir).
    pub stdin: Option<Box<dyn std::io::Read + Send>>,
    /// Sonuç buraya yazılır; `run` dönünce main stdout'a basar.
    pub result: Arc<Mutex<Option<DialogOutcome>>>,
}

impl std::fmt::Debug for Setup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Setup")
            .field("action", &self.request.action_id)
            .field("identities", &self.identities.len())
            .finish_non_exhaustive()
    }
}

/// Kartın gövdesi.
struct Body {
    icon: IconView,
    message: Label,
    detail: Label,
    identity_label: Label,
    select: Option<Select>,
    input: TextInput,
    echo: bool,
    status: Label,
    /// Hatalı denemede alanın yatay ofseti.
    shake: Animated<f32>,
    caps_lock: bool,
    /// Klavye odağı kimlik seçicide mi (Tab).
    select_focused: bool,
}

pub struct PolkitDialog {
    setup: Setup,
    session: Session,
    helper: Option<HelperSession>,
    surface: Option<SurfaceId>,
    size: Size,
    body: Option<Body>,
    dialog: Option<Dialog>,
    idle_timer: Option<TimerId>,
    started: bool,
    /// Yazı-önü kapısı (ilk sunumdan sonra kısa süre tuş/IME yok sayılır).
    gate: InputGate,
}

impl std::fmt::Debug for PolkitDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolkitDialog")
            .field("phase", self.session.phase())
            .finish_non_exhaustive()
    }
}

const NAMESPACE: &str = "nuxut-polkit";
/// Gövde genişliği (mantıksal px, tema boşluk birimiyle).
fn body_width(theme: &nuxut::Theme, bounds: Size) -> f32 {
    theme.space(100).min(bounds.width - theme.space(24)).max(theme.space(40))
}

impl PolkitDialog {
    pub fn new(setup: Setup) -> Self {
        let n = setup.identities.len();
        let session = Session::new(n, setup.default_identity);
        Self {
            setup,
            session,
            helper: None,
            surface: None,
            size: Size::ZERO,
            body: None,
            dialog: None,
            idle_timer: None,
            started: false,
            gate: InputGate::new(InputGate::DELAY),
        }
    }

    fn finish(&mut self, cx: &mut Context<Self>, outcome: DialogOutcome) {
        if let Ok(mut r) = self.setup.result.lock() {
            r.get_or_insert(outcome);
        }
        cx.quit();
    }

    fn feed(&mut self, cx: &mut Context<Self>, event: Event) {
        for action in self.session.handle(event) {
            self.perform(cx, action);
        }
        self.sync_input(cx);
        self.redraw(cx);
    }

    fn perform(&mut self, cx: &mut Context<Self>, action: Action) {
        match action {
            Action::StartHelper { generation, identity } => {
                self.helper = None;
                let Some(user) = self.setup.identities.get(identity) else {
                    self.feed(cx, Event::HelperStartFailed { generation });
                    return;
                };
                let sender = cx.sender();
                match HelperSession::start(&self.setup.paths, &user.name, &self.setup.request.cookie, move |e| {
                    let _ = sender.send(Msg::Helper(generation, e));
                }) {
                    Ok(h) => {
                        tracing::info!(transport = ?h.transport(), "yardımcı oturumu açıldı");
                        self.helper = Some(h);
                    }
                    Err(e) => {
                        tracing::warn!("yardımcıya ulaşılamadı: {:?}", e.kind());
                        self.feed(cx, Event::HelperStartFailed { generation });
                    }
                }
            }
            Action::StopHelper => self.helper = None,
            Action::SendResponse => self.send_response(cx),
            Action::ClearInput => {
                if let Some(b) = self.body.as_mut() {
                    b.input.set_text("");
                }
            }
            Action::Shake => {
                let theme = cx.theme().clone();
                if let Some(b) = self.body.as_mut() {
                    b.shake.kick(theme.space(1) * 400.0, Instant::now());
                }
            }
            Action::Finish(outcome) => {
                tracing::info!(?outcome, "diyalog bitti");
                self.helper = None;
                self.finish(cx, outcome);
            }
        }
    }

    fn send_response(&mut self, cx: &mut Context<Self>) {
        let generation = self.session.generation();
        let mut secret = SecretBuf::new();
        let copied = match self.body.as_mut() {
            Some(b) => {
                let r = b.input.with_secret(|s| secret.push_str(s));
                b.input.set_text("");
                r.is_ok()
            }
            None => false,
        };
        let written = copied
            && self
                .helper
                .as_mut()
                .is_some_and(|h| h.respond(&secret).is_ok());
        drop(secret);
        if !written {
            // Yanıt gitmediyse oturum bu nesilde kapanmış sayılır (yeniden denenir).
            self.helper = None;
            self.feed(
                cx,
                Event::Helper {
                    generation,
                    event: HelperEvent::Closed,
                },
            );
        }
    }

    fn reset_idle(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.idle_timer.take() {
            cx.cancel_timer(t);
        }
        let timeout = Duration::from_secs(self.setup.request.timeout_s.max(1));
        self.idle_timer = Some(cx.set_timer(timeout));
    }

    fn redraw(&self, cx: &mut Context<Self>) {
        if let Some(s) = self.surface {
            cx.request_frame(s);
        }
    }

    fn sync_input(&mut self, cx: &mut Context<Self>) {
        let accepts = self.session.accepts_input();
        let echo = matches!(self.session.phase(), Phase::Prompt { echo: true, .. });
        let theme = cx.theme().clone();
        let Some(b) = self.body.as_mut() else {
            return;
        };
        if echo != b.echo {
            // Görünür istem (çok adımlı PAM): alan maskesiz yeniden kurulur.
            b.echo = echo;
            b.input = new_input(&theme, echo);
            b.input.set_focused(true, Instant::now());
        }
        b.input.set_enabled(accepts);
        let placeholder = match self.session.phase() {
            Phase::Prompt { text, .. } if !text.trim().is_empty() => {
                text.trim().trim_end_matches(':').trim().to_owned()
            }
            _ => t!("polkit-password"),
        };
        b.input.set_placeholder(&placeholder);
        b.status.set_text(&status_text(&self.session, b.caps_lock));
        if let Some(surface) = self.surface {
            let ime = b.input.ime_state();
            cx.set_text_input(
                surface,
                (accepts && !b.select_focused).then_some(TextInputState {
                    cursor_rect: ime.cursor_rect,
                    // Gizli kipte çevre metin asla gönderilmez.
                    surrounding: None,
                    purpose: if echo { TextPurpose::Normal } else { TextPurpose::Password },
                    multiline: false,
                }),
            );
        }
    }

    fn build(&mut self, cx: &mut Context<Self>) {
        if self.body.is_some() || self.size.width <= 0.0 {
            return;
        }
        let theme = cx.theme().clone();
        let req = &self.setup.request;
        let width = body_width(&theme, self.size);
        let icon = IconView::with_role(&theme, Icon::named(IconName::Lock), TextRole::IconLarge);
        let mut message = Label::new(
            if req.message.is_empty() { t!("polkit-default-message") } else { req.message.clone() },
            text_style(&theme, TextRole::Body),
        );
        let home = crate::request::own_home();
        let mut detail = Label::new(detail_text(req, home.as_deref()), text_style(&theme, TextRole::Caption));
        let mut identity_label = Label::new("", text_style(&theme, TextRole::Body));
        let select = (self.setup.identities.len() > 1).then(|| {
            let options = self.setup.identities.iter().map(Identity::label).collect();
            Select::new(&theme, options, self.session.identity())
        });
        if let Some(id) = self.setup.identities.get(self.session.identity())
            && select.is_none()
        {
            identity_label.set_text(&id.label());
        }
        let mut status = Label::new("", text_style(&theme, TextRole::Body));
        let icon_w = icon.size().width;
        let text_w = width - icon_w - theme.space(4);
        message.set_max_width(Some(text_w));
        detail.set_max_width(Some(text_w));
        identity_label.set_max_width(Some(width));
        status.set_max_width(Some(width));
        let text = cx.text();
        let mut icon = icon;
        let icon_size = icon.measure(text);
        let header_h = message.measure(text).height + theme.space(1) + detail.measure(text).height;
        let line_h = Label::new(t!("polkit-password"), text_style(&theme, TextRole::Body)).measure(text).height;
        let mut input = new_input(&theme, false);
        let input_h = input.preferred_size(&theme, width).height;
        let identity_h = match &select {
            Some(s) => s.preferred_size(&theme, width).height,
            None => line_h,
        };
        // İkon | mesaj + ayrıntı; kimlik; parola; durum (iki satır yer ayrılır).
        let height = header_h.max(icon_size.height)
            + theme.space(5)
            + identity_h
            + theme.space(3)
            + input_h
            + theme.space(2)
            + line_h * 2.0;
        input.set_focused(true, Instant::now());
        let mut dialog = Dialog::new(&theme);
        dialog.set_dismissible(false);
        let bounds = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        dialog.open(
            cx.text(),
            &theme,
            &t!("polkit-title"),
            actions(),
            Size::new(width, height),
            bounds,
            Instant::now(),
        );
        self.dialog = Some(dialog);
        self.body = Some(Body {
            icon,
            message,
            detail,
            identity_label,
            select,
            input,
            echo: false,
            status,
            shake: Animated::new(0.0, theme.motion.spatial_fast),
            caps_lock: false,
            select_focused: false,
        });
        self.sync_input(cx);
    }

    /// Kartı mevcut yüzey boyutuna göre yeniden kurar; giriş alanı (içerik, kip, odak) ve
    /// klavye durumu eski gövdeden taşınır.
    fn relayout(&mut self, cx: &mut Context<Self>) {
        let old = self.body.take();
        self.dialog = None;
        self.build(cx);
        if let (Some(old), Some(b)) = (old, self.body.as_mut()) {
            b.input = old.input;
            b.echo = old.echo;
            b.caps_lock = old.caps_lock;
            b.select_focused = old.select_focused;
            if let Some(s) = b.select.as_mut() {
                s.set_focus_visible(old.select_focused);
            }
        }
        self.sync_input(cx);
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        self.feed(cx, Event::Submit);
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.feed(cx, Event::Cancel);
    }

    /// Doğrula/İptal düğmesi. `Dialog` eylemde kendini kapatır; Doğrula sonrası kart açık
    /// kalmalı (yanlış parola) — aynı karede yeniden açılır (görünür animasyon yok).
    fn dialog_action(&mut self, cx: &mut Context<Self>, index: usize) {
        match index {
            1 => {
                let theme = cx.theme().clone();
                if let Some(d) = self.dialog.as_mut() {
                    let content = d.content_rect(&theme).size();
                    let bounds = Rect::new(0.0, 0.0, self.size.width, self.size.height);
                    d.open(cx.text(), &theme, &t!("polkit-title"), actions(), content, bounds, Instant::now());
                }
                self.submit(cx);
            }
            _ => self.cancel(cx),
        }
    }

    fn key(&mut self, cx: &mut Context<Self>, event: &KeyEvent) {
        if event.state != KeyState::Pressed {
            return;
        }
        if !self.gate.allows(Instant::now()) {
            // Tuş içeriği loglanmaz.
            tracing::debug!("yazı-önü kapısı: tuş yok sayıldı");
            return;
        }
        self.reset_idle(cx);
        let now = Instant::now();
        let Some(b) = self.body.as_mut() else {
            if event.key == Key::Escape {
                self.cancel(cx);
            }
            return;
        };
        b.caps_lock = event.modifiers.caps_lock;
        if let Some(select) = b.select.as_mut()
            && (select.is_open() || b.select_focused)
            && !matches!(event.key, Key::Tab | Key::Escape)
        {
            let theme = cx.theme().clone();
            let r = select.key(cx.text(), &theme, event, now);
            if let Some(SelectEvent::Changed(i)) = r.event {
                self.feed(cx, Event::SelectIdentity(i));
            }
            self.redraw(cx);
            return;
        }
        match event.key {
            Key::Escape => {
                if let Some(select) = b.select.as_mut().filter(|s| s.is_open()) {
                    let theme = cx.theme().clone();
                    let _ = select.key(cx.text(), &theme, event, now);
                    self.redraw(cx);
                } else {
                    self.cancel(cx);
                }
                return;
            }
            Key::Tab if b.select.is_some() => {
                b.select_focused = !b.select_focused;
                let f = b.select_focused;
                if let Some(s) = b.select.as_mut() {
                    s.set_focus_visible(f);
                }
                b.input.set_focused(!f, now);
                self.sync_input(cx);
                self.redraw(cx);
                return;
            }
            Key::Enter => {
                self.submit(cx);
                return;
            }
            _ => {}
        }
        // Ctrl+U: alanı temizle. Pano yok (kopyala/kes/yapıştır yok sayılır).
        if event.modifiers.ctrl && matches!(event.key, Key::Char('u' | 'U')) {
            b.input.set_text("");
            self.redraw(cx);
            return;
        }
        let response = b.input.key(event, now);
        match response.event {
            Some(TextInputEvent::Submit) => self.submit(cx),
            Some(TextInputEvent::Cancel) => self.cancel(cx),
            Some(_) | None => {
                if response.repaint || response.event.is_some() {
                    self.redraw(cx);
                }
            }
        }
        self.sync_input(cx);
    }
}

fn new_input(theme: &nuxut::Theme, echo: bool) -> TextInput {
    TextInput::new(theme)
        .password(!echo)
        .with_max_bytes(MAX_RESPONSE_BYTES)
        .pill_shaped(true)
}

fn actions() -> Vec<DialogAction> {
    vec![
        DialogAction::new(t!("polkit-cancel"), DialogActionKind::Default),
        DialogAction::new(t!("polkit-authenticate"), DialogActionKind::Primary),
    ]
}

/// İsteyenin gösterim türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequesterKind {
    /// Doğrulanmış sistem yolu.
    Verified,
    /// Yol doğrulanmamış (kullanıcı yazılabilir yer, sahibi root değil, karışabilir karakter).
    UnverifiedPath,
    /// Yalnızca sürecin kendi seçtiği ad (`comm`).
    UnverifiedName,
}

/// Gösterilecek isteyen: kaçışlı metin ve türü. Yollar [`text::display_path`] ile (`~`,
/// ortadan kırpma, ASCII dışı kaçış). Sözleşmede `Exe` gelse bile düz ASCII değilse
/// doğrulanmamış sayılır (derinlemesine savunma).
pub fn requester_display(r: &Requester, home: Option<&str>) -> Option<(String, RequesterKind)> {
    let (text, kind) = match r {
        Requester::Exe(p) | Requester::UnverifiedExe(p) => {
            let plain = p.chars().all(text::is_plain_ascii);
            let kind = match r {
                Requester::Exe(_) if plain => RequesterKind::Verified,
                _ => RequesterKind::UnverifiedPath,
            };
            (text::display_path(p, home, text::MAX_SHORT_CHARS), kind)
        }
        Requester::Comm(c) => (text::escape_middle(c, text::MAX_SHORT_CHARS), RequesterKind::UnverifiedName),
    };
    (!text.is_empty()).then_some((text, kind))
}

/// Eylem kimliği gösterimi: polkit eylem kimlikleri düz ASCII'dir; dışındaki her karakter
/// kaçışlı, uzunsa ortadan kırpılır (anlamlı son kısım görünür kalır).
pub fn action_display(action_id: &str) -> String {
    text::escape_middle(action_id, text::MAX_SHORT_CHARS)
}

/// Ayrıntı satırı: isteyen süreç ve eylem kimliği (oltalamaya karşı bağlam).
pub fn detail_text(req: &DialogRequest, home: Option<&str>) -> String {
    let action = action_display(&req.action_id);
    match req.requester.as_ref().and_then(|r| requester_display(r, home)) {
        Some((app, RequesterKind::Verified)) => t!("polkit-detail-requester", app = app, action = action),
        Some((app, RequesterKind::UnverifiedPath)) => {
            t!("polkit-detail-requester-unverified-path", app = app, action = action)
        }
        Some((app, RequesterKind::UnverifiedName)) => {
            t!("polkit-detail-requester-unverified", app = app, action = action)
        }
        None => t!("polkit-detail", action = action),
    }
}

/// Durum satırı: hata > bilgi > doğrulanıyor > Caps Lock.
pub fn status_text(session: &Session, caps_lock: bool) -> String {
    if let Some(e) = session.error() {
        return match e {
            ErrorText::Pam(t) => t.clone(),
            ErrorText::WrongPassword => t!("polkit-wrong-password"),
        };
    }
    if let Some(info) = session.info() {
        return info.to_owned();
    }
    if matches!(session.phase(), Phase::Verifying) {
        return t!("polkit-verifying");
    }
    if caps_lock {
        return t!("polkit-caps-lock");
    }
    String::new()
}

impl App for PolkitDialog {
    type Message = Msg;

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if self.started {
            return;
        }
        self.started = true;
        // Çıkışı compositor seçer (odaklı/etkin çıkış): parola istenen yer kullanıcının baktığı ekran.
        // İlk çıkış yalnızca "en az bir çıkış var" işaretidir.
        let mut spec = LayerSurfaceSpec::on_focused_output(NAMESPACE);
        spec.layer = Layer::Overlay;
        spec.anchor = Anchor::ALL;
        spec.exclusive_zone = -1;
        spec.keyboard = KeyboardMode::Exclusive;
        self.surface = Some(cx.create_layer_surface(spec));
        if let Some(stdin) = self.setup.stdin.take() {
            let sender = cx.sender();
            let spawned = std::thread::Builder::new()
                .name("nuxut-polkit-stdin".into())
                .spawn(move || {
                    let mut stdin = stdin;
                    let mut buf = [0u8; 256];
                    // Ajan başka bir şey yazmaz; EOF ya da hata = ajan gitti.
                    while matches!(stdin.read(&mut buf), Ok(n) if n > 0) {}
                    let _ = sender.send(Msg::StdinClosed);
                });
            if let Err(e) = spawned {
                tracing::warn!("stdin izleyicisi başlatılamadı: {e}");
            }
        }
        self.reset_idle(cx);
        self.feed(cx, Event::Start);
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, _output: OutputId) {
        // Yüzeyin çıkışı gittiyse diyalog görünmez: iptal (fail-closed, kullanıcı yeniden dener).
        if let Some(s) = self.surface
            && cx.surface_size(s).is_none()
        {
            self.cancel(cx);
        }
    }

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, _scale: f32) {
        if Some(surface) != self.surface {
            return;
        }
        let changed = self.size != size;
        self.size = size;
        if changed && self.body.is_some() {
            // Çıkış boyutu/ölçeği değişti: kart yeni alana göre yeniden kurulur (Dialog ortalamayı
            // açılışta hesaplar). Parola alanı nesnesi taşınır: yazılan metin kopyalanmaz, kaybolmaz.
            self.relayout(cx);
        } else {
            self.build(cx);
        }
        cx.request_frame(surface);
    }

    fn surface_presented(&mut self, _cx: &mut Context<Self>, surface: SurfaceId) {
        if Some(surface) == self.surface {
            tracing::info!("diyalog gösterildi");
            self.gate.shown(Instant::now());
        }
    }

    fn close_requested(&mut self, cx: &mut Context<Self>, _surface: SurfaceId) {
        self.cancel(cx);
    }

    fn pointer(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, event: PointerEvent) {
        let now = Instant::now();
        if matches!(event.kind, PointerKind::Press(_)) {
            self.reset_idle(cx);
        }
        let theme = cx.theme().clone();
        let Some(b) = self.body.as_mut() else {
            return;
        };
        if let Some(select) = b.select.as_mut() {
            let was_open = select.is_open();
            let r = select.pointer(cx.text(), &theme, &event, now);
            if let Some(SelectEvent::Changed(i)) = r.event {
                self.feed(cx, Event::SelectIdentity(i));
                return;
            }
            if r.repaint {
                self.redraw(cx);
            }
            if was_open {
                return;
            }
        }
        if let Some(b) = self.body.as_mut() {
            let r = b.input.pointer(cx.text(), &theme, &event, now);
            if matches!(event.kind, PointerKind::Press(_)) && b.input.rect().contains(event.position) {
                b.select_focused = false;
                b.input.set_focused(true, now);
            }
            if r.repaint {
                self.redraw(cx);
            }
        }
        if let Some(d) = self.dialog.as_mut() {
            let r = d.pointer(&event, now);
            match r.event {
                Some(DialogEvent::Action(i)) => self.dialog_action(cx, i),
                Some(DialogEvent::Dismissed) => {}
                None if r.repaint => self.redraw(cx),
                None => {}
            }
        }
        self.sync_input(cx);
    }

    fn keyboard_focus(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, focused: bool) {
        if let Some(b) = self.body.as_mut() {
            b.input.set_focused(focused && !b.select_focused, Instant::now());
        }
        self.redraw(cx);
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        if Some(surface) != self.surface {
            return;
        }
        self.key(cx, &event);
    }

    fn modifiers_changed(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, modifiers: Modifiers) {
        if let Some(b) = self.body.as_mut()
            && b.caps_lock != modifiers.caps_lock
        {
            b.caps_lock = modifiers.caps_lock;
            self.sync_input(cx);
            self.redraw(cx);
        }
    }

    fn ime(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: ImeEvent) {
        if Some(surface) != self.surface
            || !self.session.accepts_input()
            || !self.gate.allows(Instant::now())
        {
            return;
        }
        let Some(b) = self.body.as_mut() else {
            return;
        };
        match event {
            ImeEvent::Preedit { text, cursor } => {
                let _ = b.input.ime_preedit(&text, cursor);
            }
            ImeEvent::Commit(text) => {
                let _ = b.input.ime_commit(&text);
            }
            ImeEvent::DeleteSurrounding { before, after } => {
                let _ = b.input.ime_delete_surrounding(before, after);
            }
        }
        self.reset_idle(cx);
        self.redraw(cx);
    }

    fn message(&mut self, cx: &mut Context<Self>, message: Msg) {
        match message {
            // Yalnızca tür loglanır; PAM metinleri ve istemler loglanmaz.
            Msg::Helper(generation, event) => {
                tracing::debug!(generation, final_ = event.is_final(), "yardımcı olayı");
                self.feed(cx, Event::Helper { generation, event });
            }
            Msg::StdinClosed => {
                tracing::warn!("ajan bağlantısı kapandı; iptal");
                self.cancel(cx);
            }
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.idle_timer == Some(id) {
            self.idle_timer = None;
            tracing::info!("etkileşimsiz süre doldu; iptal");
            self.cancel(cx);
        }
    }

    fn terminate(&mut self, cx: &mut Context<Self>) {
        // Ajan iptali (CancelAuthentication) ya da oturum kapanışı.
        self.cancel(cx);
        if !self.session.is_done() {
            cx.quit();
        }
    }

    fn theme_changed(&mut self, cx: &mut Context<Self>) {
        // Basitlik: gövde yeniden kurulur (parola alanı temizlenir; nadir olay).
        self.body = None;
        self.dialog = None;
        self.build(cx);
        self.redraw(cx);
    }

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        if Some(surface) != self.surface {
            return;
        }
        let theme = cx.theme();
        let (Some(dialog), Some(b)) = (self.dialog.as_mut(), self.body.as_mut()) else {
            return;
        };
        let mut ui = Ui::new(painter, theme, now);
        let c = &theme.colors;
        let status_error = self.session.error().is_some();
        dialog.draw(&mut ui, |ui, rect| {
            let icon_size = b.icon.size();
            b.icon.draw_centered(
                ui.painter,
                Point::new(rect.x + icon_size.width * 0.5, rect.y + icon_size.height * 0.5),
                ui.fade(c.primary),
            );
            let tx = rect.x + icon_size.width + theme.space(4);
            let m = b.message.measure_in(ui.painter);
            b.message.draw(ui.painter, Point::new(tx, rect.y), ui.fade(c.on_surface));
            let d = b.detail.measure_in(ui.painter);
            let dy = rect.y + m.height + theme.space(1);
            b.detail.draw(ui.painter, Point::new(tx, dy), ui.fade(c.on_surface_variant));
            let mut y = rect.y + (m.height + theme.space(1) + d.height).max(icon_size.height) + theme.space(5);
            match b.select.as_mut() {
                Some(s) => {
                    let h = s.preferred_size(theme, rect.width).height;
                    s.set_rect(theme, Rect::new(rect.x, y, rect.width, h));
                    s.set_bounds(ui.surface_rect());
                    s.draw(ui);
                    y += h;
                }
                None => {
                    let l = b.identity_label.measure_in(ui.painter);
                    b.identity_label.draw(ui.painter, Point::new(rect.x, y), ui.fade(c.on_surface));
                    y += l.height;
                }
            }
            y += theme.space(3);
            let h = b.input.preferred_size(theme, rect.width).height;
            let offset = b.shake.value(now);
            b.input.set_rect(Rect::new(rect.x + offset, y, rect.width, h));
            b.input.draw(ui);
            y += h + theme.space(2);
            let color = if status_error { c.error } else { c.on_surface_variant };
            b.status.draw(ui.painter, Point::new(rect.x, y), ui.fade(color));
            if let Some(s) = b.select.as_mut() {
                s.draw_overlay(ui);
            }
        });
        let more = ui.wants_frame() || !b.shake.is_settled(now);
        if more {
            cx.request_frame(surface);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_gate_blocks_typeahead() {
        let t0 = Instant::now();
        let mut g = InputGate::new(InputGate::DELAY);
        // Sunulmadan önce hiçbir tuş geçmez.
        assert!(!g.allows(t0));
        assert!(!g.allows(t0 + Duration::from_secs(60)));
        g.shown(t0);
        assert!(!g.allows(t0));
        assert!(!g.allows(t0 + Duration::from_millis(399)));
        assert!(g.allows(t0 + Duration::from_millis(400)));
        assert!(g.allows(t0 + Duration::from_secs(10)));
        // Yalnızca ilk sunum sayılır (sonraki çağrı kapıyı yeniden kapatmaz).
        g.shown(t0 + Duration::from_secs(5));
        assert!(g.allows(t0 + Duration::from_secs(5)));
        // Saat geri giderse (tekdüze saatte olmaz) kapı açılmaz, panik yok.
        let mut g = InputGate::new(InputGate::DELAY);
        g.shown(t0 + Duration::from_secs(1));
        assert!(!g.allows(t0));
    }

    #[test]
    fn identity_label_escapes_login_name() {
        // Giriş adı kaçışlı (yön/satır/homoglyph görünür); yardımcıya ham ad gider.
        let id = Identity {
            uid: 1000,
            name: "ev\u{202E}il\nroot".into(),
            display: "Ali".into(),
        };
        assert_eq!(id.label(), "Ali (ev\\u{202e}il\\u{a}root)");
        assert_eq!(id.name, "ev\u{202E}il\nroot");
        // GECOS "root" gibi görünse de gerçek giriş adı (Kiril "о") kaçışlı yanında.
        let id = Identity {
            uid: 1001,
            name: "r\u{43e}\u{43e}t".into(),
            display: "root".into(),
        };
        assert_eq!(id.label(), "root (r\\u{43e}\\u{43e}t)");
        let id = Identity {
            uid: 1000,
            name: "hakan".into(),
            display: "Hakan Göksu".into(),
        };
        assert_eq!(id.label(), "Hakan Göksu (hakan)");
        let id = Identity {
            uid: 0,
            name: "root".into(),
            display: String::new(),
        };
        assert_eq!(id.label(), "root");
        let id = Identity {
            uid: 0,
            name: "root".into(),
            display: "root".into(),
        };
        assert_eq!(id.label(), "root");
    }

    #[test]
    fn requester_display_kinds() {
        let home = Some("/home/u");
        assert_eq!(
            requester_display(&Requester::Exe("/usr/bin/pkexec".into()), home),
            Some(("/usr/bin/pkexec".into(), RequesterKind::Verified))
        );
        // Taklit: ev altı "usr/bin" — "~" ile ve doğrulanmamış.
        assert_eq!(
            requester_display(&Requester::UnverifiedExe("/home/u/.cache/x/usr/bin/gnome-software".into()), home),
            Some(("~/.cache/x/usr/bin/gnome-software".into(), RequesterKind::UnverifiedPath))
        );
        // Sözleşmede Exe gelse bile karışabilir karakter → doğrulanmamış + kaçışlı.
        assert_eq!(
            requester_display(&Requester::Exe("/usr/bin/gnom\u{435}".into()), home),
            Some(("/usr/bin/gnom\\u{435}".into(), RequesterKind::UnverifiedPath))
        );
        // Uzun taklit yol hiçbir zaman "/usr/bin/…" ya da "…/usr/bin/x" gibi başlamaz.
        let deep = format!("/home/u/{}usr/bin/gnome-software", "a/".repeat(80));
        let (s, kind) = requester_display(&Requester::UnverifiedExe(deep), home).unwrap();
        assert_eq!(kind, RequesterKind::UnverifiedPath);
        assert!(s.starts_with("~/") && s.ends_with("/gnome-software"), "{s}");
        assert!(s.chars().count() <= text::MAX_SHORT_CHARS, "{s}");
        // comm: kaçışlı, doğrulanmamış ad.
        assert_eq!(
            requester_display(&Requester::Comm("Sistem\u{202E}".into()), home),
            Some(("Sistem\\u{202e}".into(), RequesterKind::UnverifiedName))
        );
        assert_eq!(requester_display(&Requester::Comm(String::new()), home), None);
    }

    #[test]
    fn action_id_escaped_and_middle_truncated() {
        assert_eq!(action_display("org.freedesktop.timedate1.set-time"), "org.freedesktop.timedate1.set-time");
        assert_eq!(action_display("org.fr\u{435}edesktop.x"), "org.fr\\u{435}edesktop.x");
        let long = format!("org.{}.set-time", "x".repeat(200));
        let s = action_display(&long);
        assert!(s.starts_with("org.") && s.ends_with(".set-time") && s.contains('…'), "{s}");
        assert!(s.chars().count() <= text::MAX_SHORT_CHARS);
    }
}
