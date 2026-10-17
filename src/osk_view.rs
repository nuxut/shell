//! Ekran klavyesinin görünümü (t125): tuş yerleşimi, tutamaç, isabet testi ve çizim. Durum
//! (değiştiriciler, düzen, gönderilecek tuşlar) `services::osk::Osk`'ta, yer
//! `services::osk::place`'tadır; bu dosya yalnızca gösterir ve işaretçiyi basma/bırakma ve
//! sürükleme olaylarına çevirir.
//!
//! Klavye yüzeyin içinde bir dikdörtgendir (`frame`; çevresinde gölge payı). Zemin kabuğun panel
//! geometrisiyle çizilir (framework `BlobField`, ADR-0008: boşluğa açılan köşe `radius_large`);
//! yanaşıkken alt köşeler kenara gömülüdür. Tuş kapakları framework parçalarından kurulur:
//! `Interaction` (state layer), `Label`, `IconView`, tema renk/şekil token'ları. Koda gömülü renk yok.

use std::time::{Duration, Instant};

use nuxut::access::{AccessTree, Role};
use nuxut::i18n::t;
use nuxut::render::{BlobField, BlobShape, RectStyle, ShadowStyle};
use nuxut::ui::{Icon, IconName, IconView, Interaction, Label, TextRole, Ui, style, text_style};
use nuxut::{Button as MouseButton, Color, Corners, Point, PointerEvent, PointerKind, Rect, Size, Theme, TouchEvent, TouchId, TouchPhase};

use crate::services::osk::keys::{self, ROW_UNITS};
use crate::services::osk::{KeyAction, KeySpec, Osk, Special};

/// Satır sayısı.
const ROWS: usize = 5;
/// Bir tuş biriminin genişliği / yüksekliği.
const UNIT_ASPECT: f32 = 1.25;

/// Dokunmada tuş bu süre basılı kalırsa basılır ve basılı tutulur (istemci tekrarlar); daha kısa
/// dokunuş kalkışta yazılır (parmağı kaydırıp düzeltmek mümkün).
pub const TOUCH_HOLD: Duration = Duration::from_millis(400);

/// Dokunmatik ekrandan çıkan olay (temas başına).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchInput {
    /// Parmak tuştan kalktı: bas + bırak.
    Commit(KeyAction),
    /// Parmak tuşta [`TOUCH_HOLD`] kaldı: bas ve basılı tut.
    Hold(KeyAction),
    /// Bu temasın basılı tuttuğu tuş bırakıldı.
    Release,
}

/// Bir dokunma temasının durumu.
#[derive(Debug, Clone, Copy)]
struct Contact {
    id: TouchId,
    key: Option<usize>,
    /// Tuş için bekleme başladı (kaydırınca yeniden başlar).
    since: Instant,
    /// `Hold` gönderildi: tuş basılı.
    held: bool,
}

/// Aynı anda izlenen en çok temas.
const MAX_CONTACTS: usize = 10;

/// İşaretçiden çıkan olay (konumlar yüzey koordinatı).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyInput {
    Press(KeyAction),
    /// Basılı tuş bırakıldı (ya da işaretçi tuştan/yüzeyden çıktı).
    Release,
    /// Tutamaçtan sürükleme başladı / sürdü / bitti.
    DragStart(Point),
    DragMove(Point),
    DragEnd(Point),
}

fn key_icon(action: KeyAction) -> Option<IconName> {
    Some(match action {
        KeyAction::Special(Special::Backspace) => IconName::Backspace,
        KeyAction::Special(Special::Tab) => IconName::KeyboardTab,
        KeyAction::Special(Special::Enter) => IconName::KeyboardReturn,
        KeyAction::Special(Special::Left) => IconName::ArrowBack,
        KeyAction::Special(Special::Right) => IconName::ArrowForward,
        KeyAction::Special(Special::Up) => IconName::ArrowUpward,
        KeyAction::Special(Special::Down) => IconName::ArrowDownward,
        KeyAction::Shift => IconName::Shift,
        KeyAction::Caps => IconName::KeyboardCapslock,
        KeyAction::Hide => IconName::KeyboardHide,
        _ => return None,
    })
}

/// Tuşun erişilebilir adı (katalogdan; sembol değil).
pub fn accessible_name(osk: &Osk, action: KeyAction) -> String {
    match action {
        KeyAction::Char(code) => osk.labels().label(code, osk.level()).unwrap_or("").to_owned(),
        KeyAction::Special(s) => match s {
            Special::Backspace => t!("shell-osk-key-backspace"),
            Special::Tab => t!("shell-osk-key-tab"),
            Special::Enter => t!("shell-osk-key-enter"),
            Special::Space => t!("shell-osk-key-space"),
            Special::Left => t!("shell-osk-key-left"),
            Special::Right => t!("shell-osk-key-right"),
            Special::Up => t!("shell-osk-key-up"),
            Special::Down => t!("shell-osk-key-down"),
        },
        KeyAction::Shift => t!("shell-osk-key-shift"),
        KeyAction::Caps => t!("shell-osk-key-caps"),
        KeyAction::AltGr => t!("shell-osk-key-altgr"),
        KeyAction::Ctrl => t!("shell-osk-key-ctrl"),
        KeyAction::SwitchLayout => t!("shell-osk-key-switch-layout", layout = osk.layout().name().to_uppercase()),
        KeyAction::Hide => t!("shell-osk-key-hide"),
    }
}

fn label_role(action: KeyAction) -> TextRole {
    match action {
        KeyAction::Char(_) => TextRole::Title,
        _ => TextRole::Label,
    }
}

#[derive(Debug)]
struct Cap {
    spec: KeySpec,
    rect: Rect,
    interaction: Interaction,
    label: Label,
    icon: Option<IconView>,
    /// Değiştirici etkin (vurgulu çizilir).
    active: bool,
    enabled: bool,
}

impl Cap {
    fn new(theme: &Theme, spec: KeySpec) -> Self {
        let icon = key_icon(spec.action).map(|n| IconView::with_role(theme, Icon::named(n), TextRole::Icon));
        Self {
            spec,
            rect: Rect::ZERO,
            interaction: Interaction::new(theme),
            label: Label::new("", text_style(theme, label_role(spec.action))),
            icon,
            active: false,
            enabled: true,
        }
    }

    /// Yazı tuşu mu (açık kap rengi); değilse işlev tuşu (tonal).
    fn is_char(&self) -> bool {
        matches!(self.spec.action, KeyAction::Char(_) | KeyAction::Special(Special::Space))
    }
}

/// Klavyenin çizim biçimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Alta yanaşık: üst köşeler yuvarlak, alt kenar yüzeyin altına gömülü, gölge yok.
    Docked,
    /// Yüzen: dört köşe yuvarlak, tema gölgesi.
    Floating,
}

/// Klavyenin içeriği.
#[derive(Debug)]
pub struct KeyboardView {
    caps: Vec<Cap>,
    /// Klavye dikdörtgeni (yüzey koordinatı).
    frame: Rect,
    shape: Shape,
    key_height: f32,
    /// İşaretçiyle basılı tutulan tuşun dizini.
    pressed: Option<usize>,
    /// Tutamaçtan sürükleniyor.
    dragging: bool,
    handle: Interaction,
    /// Dokunmatik ekran temasları (çoklu dokunma; işaretçiden bağımsız).
    contacts: Vec<Contact>,
}

/// Dış boşluk, tuş arası ve tutamaç şeridinin yüksekliği.
fn metrics(theme: &Theme) -> (f32, f32, f32) {
    (theme.space(2), theme.space(1), theme.space(5))
}

/// Gölgenin yüzeyde kapladığı pay (yüzen klavyenin her yanında).
pub fn shadow_margin(theme: &Theme) -> f32 {
    if theme.shadow.enabled { (theme.shadow.blur * 1.5 + theme.shadow.offset_y.abs()).ceil() } else { 0.0 }
}

impl KeyboardView {
    pub fn new(theme: &Theme, key_height: f32) -> Self {
        let caps = keys::rows().into_iter().flatten().map(|spec| Cap::new(theme, spec)).collect();
        Self {
            caps,
            frame: Rect::ZERO,
            shape: Shape::Docked,
            key_height,
            pressed: None,
            dragging: false,
            handle: Interaction::new(theme),
            contacts: Vec::new(),
        }
    }

    /// Klavyenin (gölgesiz) boyutu: tuş yüksekliğinden; genişlik `max_width`'e sığar (dar ekranda
    /// tuşlar daralır, yükseklik aynı kalır). Üstte tutamaç şeridi.
    pub fn preferred_size(theme: &Theme, key_height: f32, max_width: f32) -> Size {
        let (pad, gap, handle) = metrics(theme);
        let width = ROW_UNITS * key_height * UNIT_ASPECT + 2.0 * pad;
        let height = handle + ROWS as f32 * key_height + (ROWS - 1) as f32 * gap + pad;
        Size::new(width.min(max_width.max(0.0)).floor(), height.ceil())
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for c in &mut self.caps {
            c.interaction.set_theme(theme, now);
            c.label.set_style(text_style(theme, label_role(c.spec.action)));
            if let Some(i) = &mut c.icon {
                i.set_role(theme, TextRole::Icon);
            }
        }
        self.handle.set_theme(theme, now);
        self.layout(theme);
    }

    pub fn frame(&self) -> Rect {
        self.frame
    }

    /// Klavyenin yüzeydeki yeri ve biçimi.
    pub fn set_frame(&mut self, theme: &Theme, frame: Rect, shape: Shape) {
        if frame != self.frame || shape != self.shape {
            self.frame = frame;
            self.shape = shape;
            self.layout(theme);
        }
    }

    fn handle_rect(&self, theme: &Theme) -> Rect {
        let (_, _, handle) = metrics(theme);
        Rect::new(self.frame.x, self.frame.y, self.frame.width, handle)
    }

    /// Tuş dikdörtgenlerini klavye dikdörtgenine göre yerleştirir.
    fn layout(&mut self, theme: &Theme) {
        let (pad, gap, handle) = metrics(theme);
        let inner_w = (self.frame.width - 2.0 * pad).max(0.0);
        let unit = inner_w / ROW_UNITS;
        let rows = keys::rows();
        let mut i = 0;
        for (r, row) in rows.iter().enumerate() {
            let y = self.frame.y + handle + r as f32 * (self.key_height + gap);
            let mut x = self.frame.x + pad;
            for spec in row {
                let w = spec.width * unit;
                // Tuş arası boşluk iki yana yarı yarıya; satır kenarları dış boşlukla hizalı.
                self.caps[i].rect = Rect::new(x + gap * 0.5, y, (w - gap).max(0.0), self.key_height);
                x += w;
                i += 1;
            }
        }
    }

    /// Etiketleri, etkin değiştiricileri ve etkisiz tuşları durumdan yeniler.
    pub fn sync(&mut self, osk: &Osk, now: Instant) {
        let labels = osk.labels();
        let level = osk.level();
        let layout_name = osk.layout().name().to_uppercase();
        for c in &mut self.caps {
            let text = match c.spec.action {
                KeyAction::Char(code) => labels.label(code, level).unwrap_or("").to_owned(),
                KeyAction::AltGr => t!("shell-osk-key-altgr"),
                KeyAction::Ctrl => t!("shell-osk-key-ctrl"),
                KeyAction::SwitchLayout => layout_name.clone(),
                _ => String::new(),
            };
            c.label.set_text(&text);
            c.active = osk.is_active(c.spec.action);
            c.enabled = osk.is_enabled(c.spec.action);
            if let Some(icon) = &mut c.icon {
                icon.set_filled(c.active, now);
            }
        }
    }

    /// Tuşların erişilebilir adları (yüzey koordinatlı dikdörtgenle). Özel tuşlarda sembol değil
    /// katalog metni; yazı tuşunda görünen karakter (seviyesi olmayan tuşta boş).
    pub fn accessible_keys(&self, osk: &Osk) -> Vec<(Rect, String)> {
        self.caps.iter().map(|c| (c.rect, accessible_name(osk, c.spec.action))).collect()
    }

    /// Ekran okuyucu ağacı (framework ADR-0011): klavye `Group`, tuş başına `Button` (ad
    /// [`accessible_name`], kutu, etkinlik; değiştiricide basılı durum). Tuşlar **eylemsiz**:
    /// AT'den sanal klavyeyle odaklı pencereye metin yazılamaz (ADR-0011 karar 6 ile aynı ilke;
    /// `docs/design/osk.md`). Anahtar tuş sırası: düzen değişse de tuş yerleri sabit.
    pub fn access(&self, tree: &mut AccessTree, osk: &Osk) {
        let bounds = self.caps.iter().map(|c| c.rect).reduce(|a, b| a.union(&b)).unwrap_or_default();
        let group = tree.push("osk", Role::Group).label(&t!("shell-a11y-osk")).bounds(bounds).id();
        tree.within(group, |tree| {
            for (i, (c, (rect, name))) in self.caps.iter().zip(self.accessible_keys(osk)).enumerate() {
                if name.is_empty() {
                    continue;
                }
                let node = tree.push(i, Role::Button).label(&name).bounds(rect).enabled(c.enabled);
                if matches!(c.spec.action, KeyAction::Shift | KeyAction::Caps | KeyAction::AltGr | KeyAction::Ctrl) {
                    node.toggled(c.active);
                }
            }
        });
    }

    fn key_at(&self, p: Point) -> Option<usize> {
        self.caps.iter().position(|c| c.rect.contains(p))
    }

    /// İşaretçi olayı → (yeniden çiz, klavye olayı). Sol düğme tuşa basar ve basılı tutar;
    /// bırakmak, tuştan kaymak ya da yüzeyden çıkmak bırakır. Tutamaçta basmak sürükleme başlatır;
    /// sürükleme bırakılana dek sürer (örtük kapma: yüzey dışındaki konumlar da gelir).
    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> (bool, Option<KeyInput>) {
        let p = event.position;
        if self.dragging {
            let input = match event.kind {
                PointerKind::Motion => Some(KeyInput::DragMove(p)),
                PointerKind::Release(MouseButton::Left) | PointerKind::Leave => {
                    self.dragging = false;
                    self.handle.set_state(false, false, now);
                    Some(KeyInput::DragEnd(p))
                }
                _ => None,
            };
            return (true, input);
        }
        let on_handle = !matches!(event.kind, PointerKind::Leave) && self.handle_rect(theme).contains(p);
        let hit = match event.kind {
            PointerKind::Leave => None,
            _ => self.key_at(p),
        };
        let mut input = None;
        match event.kind {
            PointerKind::Press(MouseButton::Left) if on_handle && self.pressed.is_none() => {
                self.dragging = true;
                input = Some(KeyInput::DragStart(p));
            }
            PointerKind::Press(MouseButton::Left) => {
                if let Some(i) = hit.filter(|&i| self.caps[i].enabled) {
                    self.pressed = Some(i);
                    input = Some(KeyInput::Press(self.caps[i].spec.action));
                }
            }
            PointerKind::Release(MouseButton::Left) | PointerKind::Leave if self.pressed.take().is_some() => {
                input = Some(KeyInput::Release);
            }
            // Tuştan kayınca bırakılır.
            PointerKind::Motion if self.pressed.is_some() && hit != self.pressed => {
                self.pressed = None;
                input = Some(KeyInput::Release);
            }
            _ => {}
        }
        let mut repaint = self.handle.set_state(on_handle || self.dragging, self.dragging, now);
        repaint |= self.refresh_states(hit, now);
        (repaint || input.is_some(), input)
    }

    /// Tutamaçta mı (dokunmada tutamaç sahiplenilmez: işaretçi öykünmesiyle sürüklenir).
    pub fn on_handle(&self, theme: &Theme, p: Point) -> bool {
        self.handle_rect(theme).contains(p)
    }

    /// Ham dokunma (temas başına; uygulama `App::touch` ile sahiplenir). Tuş kalkışta yazılır,
    /// [`TOUCH_HOLD`] basılı kalırsa basılı tutulur ([`touch_hold`](Self::touch_hold)); parmak
    /// başka tuşa kayarsa hedef değişir (basılı tutulan bırakılır). Dönüş: (yeniden çiz, olay,
    /// bekleme zamanlayıcısı kurulmalı mı).
    pub fn touch(&mut self, event: &TouchEvent, now: Instant) -> (bool, Option<TouchInput>, bool) {
        let key = self.key_at(event.position).filter(|&i| self.caps[i].enabled);
        let mut input = None;
        let mut arm = false;
        match event.phase {
            TouchPhase::Down => {
                self.contacts.retain(|c| c.id != event.id);
                if self.contacts.len() < MAX_CONTACTS {
                    self.contacts.push(Contact { id: event.id, key, since: now, held: false });
                    arm = key.is_some();
                }
            }
            TouchPhase::Motion => {
                if let Some(c) = self.contacts.iter_mut().find(|c| c.id == event.id)
                    && c.key != key
                {
                    if std::mem::take(&mut c.held) {
                        input = Some(TouchInput::Release);
                    }
                    c.key = key;
                    c.since = now;
                    arm = key.is_some();
                }
            }
            TouchPhase::Up | TouchPhase::Cancel => {
                if let Some(i) = self.contacts.iter().position(|c| c.id == event.id) {
                    let c = self.contacts.swap_remove(i);
                    input = if c.held {
                        Some(TouchInput::Release)
                    } else if event.phase == TouchPhase::Up {
                        c.key.map(|k| TouchInput::Commit(self.caps[k].spec.action))
                    } else {
                        None
                    };
                }
            }
        }
        let repaint = self.refresh_states(None, now);
        (repaint || input.is_some(), input, arm)
    }

    /// Bekleme zamanlayıcısı: tuşunda [`TOUCH_HOLD`] kalan ve henüz basılmamış temaslar basılır.
    /// Erken (kaydırmayla eskimiş) zamanlayıcı bir şey yapmaz.
    pub fn touch_hold(&mut self, now: Instant) -> Vec<(TouchId, TouchInput)> {
        let mut out = Vec::new();
        for c in &mut self.contacts {
            if let Some(k) = c.key
                && !c.held
                && now.saturating_duration_since(c.since) >= TOUCH_HOLD
            {
                c.held = true;
                out.push((c.id, TouchInput::Hold(self.caps[k].spec.action)));
            }
        }
        out
    }

    /// Tuş kapaklarının hover/basılı durumu: işaretçinin üstündeki tuş (`hover`), işaretçiyle
    /// ya da bir parmakla basılı tuşlar.
    fn refresh_states(&mut self, hover: Option<usize>, now: Instant) -> bool {
        let mut repaint = false;
        for (i, c) in self.caps.iter_mut().enumerate() {
            let touched = self.contacts.iter().any(|t| t.key == Some(i));
            let hovered = (hover == Some(i) || touched) && c.enabled;
            repaint |= c.interaction.set_state(hovered, self.pressed == Some(i) || touched, now);
        }
        repaint
    }

    /// Tutamacın üstünde mi (imleç: taşıma).
    pub fn over_handle(&self, theme: &Theme, p: Point) -> bool {
        self.dragging || self.handle_rect(theme).contains(p)
    }

    /// Bir şey canlanıyor mu.
    pub fn is_animating(&mut self, now: Instant) -> bool {
        let caps = self.caps.iter_mut().fold(false, |a, c| c.interaction.is_animating(now) | a);
        caps | self.handle.is_animating(now)
    }

    /// Klavyenin girdi alan şekli (yüzey koordinatı): yalnızca klavye dikdörtgeni, gölge değil.
    pub fn input_rect(&self) -> Rect {
        self.frame
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let c = &theme.colors;
        let (pad, _, _) = metrics(theme);
        let r = theme.shape.radius_large;
        // Zemin: kabuğun panel geometrisi. Yanaşıkken alt köşeler çerçevenin altına taşar ve
        // kırpılır (yüzeyde altta gölge payı olabilir): yalnızca üstteki boşluğa açılan köşeler
        // yuvarlanır.
        let docked = self.shape == Shape::Docked;
        if docked {
            ui.painter.push_clip(self.frame);
        }
        let (rect, shadow) = match self.shape {
            Shape::Docked => (Rect::new(self.frame.x, self.frame.y, self.frame.width, self.frame.height + 2.0 * r), None),
            Shape::Floating => (
                self.frame,
                theme.shadow.enabled.then(|| ShadowStyle {
                    color: theme.shadow.color,
                    blur: theme.shadow.blur,
                    offset: Point::new(0.0, theme.shadow.offset_y),
                    spread: 0.0,
                }),
            ),
        };
        ui.painter.blob(&BlobField {
            frame: None,
            shapes: vec![BlobShape::new_rect(rect)],
            smoothing: theme.blob.smoothing,
            corner_radius: r,
            fill: ui.fade(c.surface),
            shadow,
        });
        if docked {
            ui.painter.pop_clip();
        }
        // Tutamaç: ortada ince hap; üstündeyken/sürüklerken belirginleşir.
        let h = self.handle_rect(theme);
        let pill = Rect::new(h.center().x - theme.space(5), h.center().y - 2.0, theme.space(10), 4.0);
        let handle_color = if self.handle.is_hovered() || self.dragging { c.on_surface } else { c.on_surface_variant };
        ui.painter.rect(pill, &RectStyle::filled(ui.fade(handle_color), style::pill(pill)));

        let radii = Corners::uniform(style::nested_radius(theme, r, pad));
        let tonal = style::tonal_container(theme);
        for cap in &mut self.caps {
            let (container, content) = if !cap.enabled {
                let container = style::disabled_container(theme);
                (container, style::disabled_content(theme, c.on_surface, &[container]))
            } else if cap.active {
                (c.primary, c.on_primary)
            } else if cap.is_char() {
                (c.surface_bright, c.on_surface)
            } else {
                (tonal, c.on_surface)
            };
            let rect = cap.rect;
            ui.painter.rect(rect, &RectStyle::filled(ui.fade(container), radii));
            cap.interaction.draw_state_layer(ui, rect, radii, content, container);
            let color: Color = ui.fade(content);
            let center = rect.center();
            if let Some(icon) = &mut cap.icon {
                icon.draw_centered(ui.painter, center, color);
            } else if !cap.label.text().is_empty() {
                let s = cap.label.measure_in(ui.painter);
                cap.label.draw(ui.painter, Point::new(center.x - s.width * 0.5, center.y - s.height * 0.5), color);
            }
        }
        if self.is_animating(ui.now) {
            ui.request_frame();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::osk::Layout;
    use crate::services::osk::keys::code;

    /// Klavye yüzeyde (10, 10) kayık: yerleşim ve isabet yüzey koordinatında.
    const ORIGIN: Point = Point::new(10.0, 10.0);

    fn view() -> (KeyboardView, Theme) {
        let theme = Theme::default();
        let mut v = KeyboardView::new(&theme, 48.0);
        let s = KeyboardView::preferred_size(&theme, 48.0, 10_000.0);
        v.set_frame(&theme, Rect::new(ORIGIN.x, ORIGIN.y, s.width, s.height), Shape::Floating);
        (v, theme)
    }

    fn ev(kind: PointerKind, p: Point) -> PointerEvent {
        PointerEvent::new(p, kind)
    }

    fn center_of(v: &KeyboardView, action: KeyAction) -> Point {
        v.caps.iter().find(|c| c.spec.action == action).unwrap().rect.center()
    }

    fn tev(id: i32, phase: TouchPhase, p: Point) -> TouchEvent {
        TouchEvent { id: TouchId(id), position: p, phase }
    }

    #[test]
    fn touch_tap_commits_on_lift_and_slide_corrects() {
        let (mut v, _) = view();
        let now = Instant::now();
        let a = center_of(&v, KeyAction::Char(code::AC01));
        let s = center_of(&v, KeyAction::Char(code::AC01 + 1));
        let (repaint, input, arm) = v.touch(&tev(0, TouchPhase::Down, a), now);
        assert!(repaint && arm && input.is_none(), "basışta yazılmaz, bekleme kurulur");
        let (_, input, arm) = v.touch(&tev(0, TouchPhase::Motion, s), now);
        assert!(arm && input.is_none(), "kaydırma hedefi değiştirir, yazmaz");
        let (_, input, _) = v.touch(&tev(0, TouchPhase::Up, s), now);
        assert_eq!(input, Some(TouchInput::Commit(KeyAction::Char(code::AC01 + 1))));
        // İptal yazmaz.
        v.touch(&tev(1, TouchPhase::Down, a), now);
        assert_eq!(v.touch(&tev(1, TouchPhase::Cancel, a), now).1, None);
    }

    #[test]
    fn touch_hold_presses_and_release_follows_contact() {
        let (mut v, _) = view();
        let now = Instant::now();
        let a = center_of(&v, KeyAction::Char(code::AC01));
        let s = center_of(&v, KeyAction::Char(code::AC01 + 1));
        v.touch(&tev(0, TouchPhase::Down, a), now);
        assert!(v.touch_hold(now + TOUCH_HOLD / 2).is_empty(), "erken zamanlayıcı etkisiz");
        assert_eq!(v.touch_hold(now + TOUCH_HOLD), [(TouchId(0), TouchInput::Hold(KeyAction::Char(code::AC01)))]);
        assert!(v.touch_hold(now + TOUCH_HOLD * 2).is_empty(), "bir kez basılır");
        // Kayınca bırakır, yeni tuşta bekleme yeniden başlar.
        let later = now + TOUCH_HOLD * 2;
        assert_eq!(v.touch(&tev(0, TouchPhase::Motion, s), later).1, Some(TouchInput::Release));
        assert!(v.touch_hold(later + TOUCH_HOLD / 2).is_empty());
        assert_eq!(v.touch_hold(later + TOUCH_HOLD), [(TouchId(0), TouchInput::Hold(KeyAction::Char(code::AC01 + 1)))]);
        assert_eq!(
            v.touch(&tev(0, TouchPhase::Up, s), later).1,
            Some(TouchInput::Release),
            "basılı tutulan kalkışta bırakılır, yeniden yazılmaz"
        );
    }

    #[test]
    fn two_thumbs_both_type_in_lift_order() {
        let (mut v, _) = view();
        let now = Instant::now();
        let a = center_of(&v, KeyAction::Char(code::AC01));
        let s = center_of(&v, KeyAction::Char(code::AC01 + 1));
        v.touch(&tev(0, TouchPhase::Down, a), now);
        v.touch(&tev(1, TouchPhase::Down, s), now);
        assert_eq!(v.touch(&tev(0, TouchPhase::Up, a), now).1, Some(TouchInput::Commit(KeyAction::Char(code::AC01))));
        assert_eq!(v.touch(&tev(1, TouchPhase::Up, s), now).1, Some(TouchInput::Commit(KeyAction::Char(code::AC01 + 1))));
    }

    #[test]
    fn handle_is_left_to_pointer_emulation() {
        let (v, theme) = view();
        let h = v.handle_rect(&theme).center();
        assert!(v.on_handle(&theme, h));
        assert!(!v.on_handle(&theme, center_of(&v, KeyAction::Char(code::AC01))));
    }

    #[test]
    fn preferred_size_follows_key_height_and_fits_width() {
        let theme = Theme::default();
        let s = KeyboardView::preferred_size(&theme, 48.0, 10_000.0);
        let (pad, gap, handle) = metrics(&theme);
        assert_eq!(s.height, (handle + 5.0 * 48.0 + 4.0 * gap + pad).ceil());
        assert_eq!(s.width, (15.0 * 60.0 + 2.0 * pad).floor());
        assert_eq!(KeyboardView::preferred_size(&theme, 48.0, 500.0).width, 500.0, "dar ekrana sığar");
    }

    #[test]
    fn keys_tile_inside_the_frame_below_the_handle() {
        let (v, theme) = view();
        let handle = v.handle_rect(&theme);
        for (i, a) in v.caps.iter().enumerate() {
            assert!(a.rect.width > 0.0 && a.rect.height == 48.0);
            assert!(a.rect.right() <= v.frame.right() + 0.01 && a.rect.x >= v.frame.x, "{:?}", a.spec);
            assert!(a.rect.y >= handle.bottom() && a.rect.bottom() <= v.frame.bottom() + 0.01, "{:?}", a.spec);
            for b in &v.caps[i + 1..] {
                let r = a.rect.intersect(&b.rect);
                assert!(r.width * r.height < 1e-3, "{:?} {:?}", a.spec, b.spec);
            }
        }
        assert_eq!(v.input_rect(), v.frame);
    }

    #[test]
    fn press_hold_release_and_slide_off() {
        let (mut v, theme) = view();
        let now = Instant::now();
        let a = center_of(&v, KeyAction::Char(code::AC01));
        let s = center_of(&v, KeyAction::Char(code::AC01 + 1));
        assert_eq!(v.pointer(&theme, &ev(PointerKind::Motion, a), now).1, None, "hover yalnızca görünüm");
        assert_eq!(
            v.pointer(&theme, &ev(PointerKind::Press(MouseButton::Left), a), now).1,
            Some(KeyInput::Press(KeyAction::Char(code::AC01)))
        );
        assert_eq!(v.pointer(&theme, &ev(PointerKind::Motion, a), now).1, None, "tuş içinde kalınca basılı");
        assert_eq!(v.pointer(&theme, &ev(PointerKind::Motion, s), now).1, Some(KeyInput::Release), "kayınca bırakır");
        assert_eq!(v.pointer(&theme, &ev(PointerKind::Release(MouseButton::Left), s), now).1, None, "zaten bırakıldı");
        v.pointer(&theme, &ev(PointerKind::Press(MouseButton::Left), s), now);
        assert_eq!(v.pointer(&theme, &ev(PointerKind::Leave, s), now).1, Some(KeyInput::Release), "yüzeyden çıkınca bırakır");
        assert_eq!(v.pointer(&theme, &ev(PointerKind::Press(MouseButton::Right), s), now).1, None);
    }

    #[test]
    fn handle_drag_reports_surface_positions_until_release() {
        let (mut v, theme) = view();
        let now = Instant::now();
        let h = v.handle_rect(&theme).center();
        assert!(v.over_handle(&theme, h));
        assert_eq!(v.pointer(&theme, &ev(PointerKind::Press(MouseButton::Left), h), now).1, Some(KeyInput::DragStart(h)));
        // Yüzeyin dışına (örtük kapma) taşınan konumlar da sürüklemedir; tuşlara basılmaz.
        let far = Point::new(-300.0, -200.0);
        assert_eq!(v.pointer(&theme, &ev(PointerKind::Motion, far), now).1, Some(KeyInput::DragMove(far)));
        let key = center_of(&v, KeyAction::Char(code::AC01));
        assert_eq!(v.pointer(&theme, &ev(PointerKind::Motion, key), now).1, Some(KeyInput::DragMove(key)));
        assert_eq!(v.pointer(&theme, &ev(PointerKind::Release(MouseButton::Left), far), now).1, Some(KeyInput::DragEnd(far)));
        assert!(!v.over_handle(&theme, far));
        // Bitti: sonraki basış yine tuştur.
        assert_eq!(
            v.pointer(&theme, &ev(PointerKind::Press(MouseButton::Left), key), now).1,
            Some(KeyInput::Press(KeyAction::Char(code::AC01)))
        );
    }

    #[test]
    fn frame_moves_the_keys() {
        let (mut v, theme) = view();
        let before = center_of(&v, KeyAction::Char(code::AC01));
        let f = v.frame;
        v.set_frame(&theme, Rect::new(f.x + 100.0, f.y + 50.0, f.width, f.height), Shape::Docked);
        let after = center_of(&v, KeyAction::Char(code::AC01));
        assert_eq!((after.x - before.x, after.y - before.y), (100.0, 50.0));
    }

    #[test]
    fn sync_shows_level_labels_and_layout_key() {
        crate::i18n::use_turkish_for_tests();
        let (mut v, theme) = view();
        let now = Instant::now();
        let mut osk = Osk::new(Layout::Tr).unwrap();
        osk.press(KeyAction::Shift);
        v.sync(&osk, now);
        let label = |v: &KeyboardView, a| v.caps.iter().find(|c| c.spec.action == a).unwrap().label.text().to_owned();
        assert_eq!(label(&v, KeyAction::Char(code::AC11)), "İ");
        assert_eq!(label(&v, KeyAction::SwitchLayout), "TR");
        assert!(v.caps.iter().find(|c| c.spec.action == KeyAction::Shift).unwrap().active);
        osk.set_layout(Layout::Us);
        v.sync(&osk, now);
        // US'te AltGr'li seviyesi olmayan tuş yok sayılmaz (aynı harf); AltGr basılabilir.
        let altgr = center_of(&v, KeyAction::AltGr);
        assert_eq!(
            v.pointer(&theme, &ev(PointerKind::Press(MouseButton::Left), altgr), now).1,
            Some(KeyInput::Press(KeyAction::AltGr))
        );
        assert_eq!(label(&v, KeyAction::SwitchLayout), "US");
    }

    #[test]
    fn accessible_names_are_text_not_symbols() {
        crate::i18n::use_turkish_for_tests();
        let (v, _) = view();
        let osk = Osk::new(Layout::Tr).unwrap();
        let names = v.accessible_keys(&osk);
        assert_eq!(names.len(), v.caps.len());
        for (_, n) in &names {
            assert!(!n.is_empty(), "{n}");
        }
        assert_eq!(accessible_name(&osk, KeyAction::Special(Special::Backspace)), "Geri sil");
        assert_eq!(accessible_name(&osk, KeyAction::Char(code::AD12 - 1)), "ğ");
        assert!(accessible_name(&osk, KeyAction::SwitchLayout).contains("TR"));
    }

    #[test]
    fn access_tree_names_keys_without_actions() {
        use nuxut::access::{AccessTree, Action, Toggled};
        crate::i18n::use_turkish_for_tests();
        let (mut v, _) = view();
        let mut osk = Osk::new(Layout::Tr).unwrap();
        osk.press(KeyAction::Shift);
        v.sync(&osk, Instant::now());
        let mut tree = AccessTree::new();
        v.access(&mut tree, &osk);
        assert!(tree.unlabeled().is_empty());
        let keys: Vec<_> = tree.nodes().filter(|(_, n)| n.role() == Role::Button).map(|(_, n)| n).collect();
        assert_eq!(keys.len(), v.caps.len());
        assert!(keys.iter().all(|n| !n.supports_action(Action::Click)), "AT tuşa basamaz");
        let shift = keys.iter().find(|n| n.label() == Some("Shift")).unwrap();
        assert_eq!(shift.toggled(), Some(Toggled::True));
        assert!(keys.iter().any(|n| n.label() == Some("Geri sil") && n.toggled().is_none()));
    }
}
