//! Giriş formu: kilit ekranının görünümünün ([`nuxut_lock::view::OutputView`]: duvar kağıdı,
//! karartma, işaret, saat, tarih) üstünde, yalnızca "ev sahibi" çıkışta çizilir.
//!
//! Düzen (kilitle aynı eksen): avatar + ad (birden çok kullanıcıda iki yanda oklar), istem
//! alanı, durum satırları; alt şeritte solda oturum düğmesi, sağda yeniden başlat ve kapat.
//! Tab odağı alan → oturum → yeniden başlat → kapat sırasıyla dolaşır.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nuxut::i18n::t;
use nuxut::render::{Texture, TextureOptions};
use nuxut::ui::{
    Avatar, AvatarSize, Button, ButtonVariant, Icon, IconButton, IconButtonVariant, IconName, Label,
    Spinner, TextInput, TextInputEvent, TextRole, Ui, text_style,
};
use nuxut::{Color, KeyEvent, Point, PointerEvent, Rect, Size, Theme};
use nuxut_lock::state::AuthStatus;
use nuxut_lock::view::{Tone, status_lines};
use nuxut_pam::{MessageKind, PamMessage};

use crate::power::PowerAction;

/// Formun çizimde ihtiyaç duyduğu anlık durum (uygulamadan).
#[derive(Debug)]
pub struct FormState<'a> {
    /// Seçili kullanıcının görünen adı; `None` → kullanıcı adı girişi.
    pub display_name: Option<&'a str>,
    pub user_name: Option<&'a str>,
    pub many_users: bool,
    /// İstem: (gizli mi, yer tutucu).
    pub prompt: Option<(bool, String)>,
    pub status: &'a AuthStatus,
    pub notices: &'a [PamMessage],
    pub backoff: Option<Duration>,
    pub caps_lock: bool,
    pub busy: bool,
    pub session: Option<&'a str>,
    /// Etkin klavye düzeni (`TR`); `None` → düğme yok.
    pub layout: Option<&'a str>,
    pub power: &'a [PowerAction],
}

/// Klavye odağı (Tab sırası).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    Session,
    Layout,
    Reboot,
    PowerOff,
}

/// Formun ürettiği kullanıcı niyeti.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Submit,
    PreviousUser,
    NextUser,
    NextSession,
    NextLayout,
    Power(PowerAction),
    /// Yalnızca yeniden çizim.
    Repaint,
}

#[derive(Debug)]
pub struct Form {
    secret: TextInput,
    visible: TextInput,
    /// Şu an gösterilen alan gizli mi.
    secret_mode: bool,
    avatar: Avatar,
    avatar_user: Option<String>,
    textures: HashMap<String, Texture>,
    pending_images: Vec<(String, nuxut::render::ImageData)>,
    name: Label,
    prev: IconButton,
    next: IconButton,
    session: Button,
    layout: Button,
    reboot: IconButton,
    power_off: IconButton,
    spinner: Spinner,
    status: Vec<(Label, Tone)>,
    focus: Focus,
    /// Son çizimde görünen düğmeler (işaretçi/klavye yalnız bunlara gider).
    shown_arrows: bool,
    shown_power: Vec<PowerAction>,
    shown_session: bool,
    shown_layout: bool,
}

impl Form {
    pub fn new(theme: &Theme) -> Self {
        let secret = TextInput::new(theme)
            .password(true)
            .with_max_bytes(nuxut_pam::MAX_SECRET_BYTES)
            .pill_shaped(true);
        let visible = TextInput::new(theme).with_max_bytes(256).pill_shaped(true);
        Self {
            secret,
            visible,
            secret_mode: true,
            avatar: Avatar::new(theme, "", AvatarSize::Large),
            avatar_user: None,
            textures: HashMap::new(),
            pending_images: Vec::new(),
            name: Label::new("", text_style(theme, TextRole::Title)),
            prev: IconButton::new(theme, Icon::named(IconName::ChevronLeft), IconButtonVariant::Standard),
            next: IconButton::new(theme, Icon::named(IconName::ChevronRight), IconButtonVariant::Standard),
            session: Button::new(theme, ButtonVariant::Text)
                .with_label(theme, "")
                .with_icon(theme, Icon::named(IconName::Desktop)),
            layout: Button::new(theme, ButtonVariant::Text)
                .with_label(theme, "")
                .with_icon(theme, Icon::named(IconName::Keyboard)),
            reboot: IconButton::new(theme, Icon::named(IconName::Restart), IconButtonVariant::Standard),
            power_off: IconButton::new(theme, Icon::named(IconName::Power), IconButtonVariant::Standard),
            spinner: Spinner::new(theme),
            status: Vec::new(),
            focus: Focus::Input,
            shown_arrows: false,
            shown_power: Vec::new(),
            shown_session: false,
            shown_layout: false,
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.secret.set_theme(theme, now);
        self.visible.set_theme(theme, now);
        self.avatar.set_theme(theme, now);
        self.name.set_style(text_style(theme, TextRole::Title));
        for b in [&mut self.prev, &mut self.next, &mut self.reboot, &mut self.power_off] {
            b.set_theme(theme, now);
        }
        self.session.set_theme(theme, now);
        self.layout.set_theme(theme, now);
        self.spinner.set_theme(theme, now);
        for (l, _) in &mut self.status {
            l.set_style(text_style(theme, TextRole::Body));
        }
    }

    /// Çözülmüş avatar (GPU'ya ilk çizimde yüklenir).
    pub fn add_avatar(&mut self, user: String, image: nuxut::render::ImageData) {
        self.pending_images.push((user, image));
    }

    fn input(&mut self) -> &mut TextInput {
        if self.secret_mode { &mut self.secret } else { &mut self.visible }
    }

    pub fn input_ref(&self) -> &TextInput {
        if self.secret_mode { &self.secret } else { &self.visible }
    }

    /// Alanın içeriği (gizli kipte de) — yanıt tamponuna kopyalamak için.
    pub fn with_text<R>(&self, f: impl FnOnce(&str) -> R) -> R {
        self.input_ref().with_secret(f)
    }

    pub fn clear(&mut self) {
        self.secret.set_text("");
        self.visible.set_text("");
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) {
        self.secret.set_enabled(enabled);
        self.visible.set_enabled(enabled);
        if enabled && self.focus == Focus::Input {
            self.input().set_focused(true, now);
        }
    }

    /// Gizli/yankılı alan seçimi; değişince içerik silinir.
    pub fn set_secret_mode(&mut self, secret: bool, now: Instant) {
        if self.secret_mode != secret {
            self.clear();
            self.secret_mode = secret;
            if self.focus == Focus::Input {
                self.input().set_focused(true, now);
            }
        }
    }

    pub fn set_focused(&mut self, focused: bool, now: Instant) {
        if self.focus == Focus::Input {
            self.input().set_focused(focused, now);
        }
    }

    pub fn ime_state(&self) -> nuxut::ui::ImeState {
        self.input_ref().ime_state()
    }

    pub fn input_mut(&mut self) -> &mut TextInput {
        self.input()
    }

    fn focus_order(&self) -> Vec<Focus> {
        let mut v = vec![Focus::Input];
        if self.shown_session {
            v.push(Focus::Session);
        }
        if self.shown_layout {
            v.push(Focus::Layout);
        }
        if self.shown_power.contains(&PowerAction::Reboot) {
            v.push(Focus::Reboot);
        }
        if self.shown_power.contains(&PowerAction::PowerOff) {
            v.push(Focus::PowerOff);
        }
        v
    }

    fn move_focus(&mut self, back: bool, now: Instant) {
        let order = self.focus_order();
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let n = order.len();
        let next = order[if back { (i + n - 1) % n } else { (i + 1) % n }];
        self.set_focus(next, now);
    }

    fn set_focus(&mut self, focus: Focus, now: Instant) {
        self.session.set_focus_visible(focus == Focus::Session);
        self.layout.set_focus_visible(focus == Focus::Layout);
        self.reboot.set_focus_visible(focus == Focus::Reboot);
        self.power_off.set_focus_visible(focus == Focus::PowerOff);
        let input = focus == Focus::Input;
        self.focus = focus;
        self.input().set_focused(input, now);
    }

    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Option<Intent> {
        use nuxut::Key;
        if event.key == Key::Tab {
            self.move_focus(event.modifiers.shift, now);
            return Some(Intent::Repaint);
        }
        match self.focus {
            Focus::Input => {
                if self.shown_arrows && !event.modifiers.ctrl {
                    match event.key {
                        Key::Up => return Some(Intent::PreviousUser),
                        Key::Down => return Some(Intent::NextUser),
                        _ => {}
                    }
                }
                // Ctrl+U: alanı temizle (kilitle aynı).
                if event.modifiers.ctrl && matches!(event.key, Key::Char('u' | 'U')) {
                    self.input().set_text("");
                    return Some(Intent::Repaint);
                }
                let r = self.input().key(event, now);
                match r.event {
                    Some(TextInputEvent::Submit) => Some(Intent::Submit),
                    Some(TextInputEvent::Cancel) => {
                        self.input().set_text("");
                        Some(Intent::Repaint)
                    }
                    // Pano yok: kopyala/yapıştır gizli kipte zaten yok sayılır.
                    Some(_) => Some(Intent::Repaint),
                    None => r.repaint.then_some(Intent::Repaint),
                }
            }
            Focus::Session => self.session.key(event, now).event.map(|()| Intent::NextSession),
            Focus::Layout => self.layout.key(event, now).event.map(|()| Intent::NextLayout),
            Focus::Reboot => self.reboot.key(event, now).event.map(|_| Intent::Power(PowerAction::Reboot)),
            Focus::PowerOff => self.power_off.key(event, now).event.map(|_| Intent::Power(PowerAction::PowerOff)),
        }
    }

    pub fn pointer(&mut self, theme: &Theme, text: &mut nuxut::TextSystem, event: &PointerEvent, now: Instant) -> Option<Intent> {
        let mut repaint = false;
        if self.shown_arrows {
            let r = self.prev.pointer(event, now);
            if r.event.is_some() {
                return Some(Intent::PreviousUser);
            }
            repaint |= r.repaint;
            let r = self.next.pointer(event, now);
            if r.event.is_some() {
                return Some(Intent::NextUser);
            }
            repaint |= r.repaint;
        }
        if self.shown_session {
            let r = self.session.pointer(event, now);
            if r.event.is_some() {
                return Some(Intent::NextSession);
            }
            repaint |= r.repaint;
        }
        if self.shown_layout {
            let r = self.layout.pointer(event, now);
            if r.event.is_some() {
                return Some(Intent::NextLayout);
            }
            repaint |= r.repaint;
        }
        if self.shown_power.contains(&PowerAction::Reboot) {
            let r = self.reboot.pointer(event, now);
            if r.event.is_some() {
                return Some(Intent::Power(PowerAction::Reboot));
            }
            repaint |= r.repaint;
        }
        if self.shown_power.contains(&PowerAction::PowerOff) {
            let r = self.power_off.pointer(event, now);
            if r.event.is_some() {
                return Some(Intent::Power(PowerAction::PowerOff));
            }
            repaint |= r.repaint;
        }
        let r = self.input().pointer(text, theme, event, now);
        if r.event.is_some() || r.repaint {
            repaint = true;
        }
        repaint.then_some(Intent::Repaint)
    }

    /// Çizer (ev sahibi çıkış). Dönüş: başka kare gerekiyor mu.
    pub fn draw(&mut self, ui: &mut Ui, size: Size, shake: f32, state: &FormState<'_>) -> bool {
        let theme = ui.theme;
        let c = &theme.colors;
        let center_x = size.width * 0.5;

        for (user, data) in self.pending_images.drain(..) {
            let tex = ui.painter.upload_texture(data, TextureOptions { mipmaps: false });
            self.textures.insert(user, tex);
        }
        if self.avatar_user.as_deref() != state.user_name {
            self.avatar_user = state.user_name.map(str::to_owned);
            self.avatar.set_name(state.display_name.unwrap_or(""));
        }
        self.avatar.set_image(state.user_name.and_then(|u| self.textures.get(u).cloned()));

        let side = AvatarSize::Large.side(theme);
        let mut y = size.height * 0.5;
        if let Some(display) = state.display_name {
            let avatar_center = Point::new(center_x, y + side * 0.5);
            self.avatar.set_center(avatar_center);
            self.avatar.draw(ui);
            self.shown_arrows = state.many_users;
            if state.many_users {
                let s = self.prev.preferred_size(theme);
                let dx = side * 0.5 + theme.space(6);
                self.prev.set_rect(Rect::new(center_x - dx - s.width, avatar_center.y - s.height * 0.5, s.width, s.height));
                self.next.set_rect(Rect::new(center_x + dx, avatar_center.y - s.height * 0.5, s.width, s.height));
                self.prev.draw(ui);
                self.next.draw(ui);
            }
            y += side + theme.space(3);
            self.name.set_text(display);
            let name = self.name.measure_in(ui.painter);
            self.name.draw(ui.painter, Point::new(center_x - name.width * 0.5, y), ui.fade(c.on_surface));
            y += name.height + theme.space(4);
        } else {
            self.shown_arrows = false;
            y += side * 0.5;
        }

        let width = (size.width - theme.space(16)).clamp(theme.space(40), theme.space(90));
        let placeholder = match &state.prompt {
            Some((_, p)) => p.clone(),
            None if state.display_name.is_none() => t!("shell-greeter-username"),
            None => t!("shell-greeter-password"),
        };
        let input = self.input();
        input.set_placeholder(&placeholder);
        let pref = input.preferred_size(theme, width);
        input.set_rect(Rect::new(center_x - width * 0.5 + shake, y, width, pref.height));
        input.draw(ui);
        y += pref.height + theme.space(3);

        if state.busy {
            let s = self.spinner.preferred_size(theme);
            self.spinner.set_rect(Rect::new(center_x - s.width * 0.25, y, s.width * 0.5, s.height * 0.5));
            self.spinner.draw(ui);
            y += s.height * 0.5 + theme.space(2);
        }

        let lines = form_lines(state);
        self.status.resize_with(lines.len(), || (Label::new("", text_style(theme, TextRole::Body)), Tone::Normal));
        for ((label, tone), (text, t)) in self.status.iter_mut().zip(lines) {
            label.set_text(&text);
            label.set_max_width(Some(width));
            *tone = t;
            let s = label.measure_in(ui.painter);
            let color: Color = match tone {
                Tone::Normal => c.on_surface_variant,
                Tone::Error => c.error,
            };
            label.draw(ui.painter, Point::new(center_x - s.width * 0.5, y), ui.fade(color));
            y += s.height + theme.space(1);
        }

        // Alt şerit.
        let margin = theme.space(6);
        self.shown_session = state.session.is_some();
        if let Some(name) = state.session {
            self.session.set_label(&t!("shell-greeter-session", name = name));
            self.session.set_max_width(theme, Some(size.width * 0.4));
            let s = self.session.preferred_size(ui.painter.text_system(), theme);
            self.session.set_rect(Rect::new(margin, size.height - margin - s.height, s.width, s.height));
            self.session.draw(ui);
        }
        self.shown_power = state.power.to_vec();
        let mut x = size.width - margin;
        for action in [PowerAction::PowerOff, PowerAction::Reboot] {
            if !state.power.contains(&action) {
                continue;
            }
            let button = match action {
                PowerAction::PowerOff => &mut self.power_off,
                PowerAction::Reboot => &mut self.reboot,
            };
            let s = button.preferred_size(theme);
            x -= s.width;
            button.set_rect(Rect::new(x, size.height - margin - s.height, s.width, s.height));
            button.draw(ui);
            x -= theme.space(2);
        }
        // Klavye düzeni: güç düğmelerinin solunda (şifre hangi düzende yazılıyor).
        self.shown_layout = state.layout.is_some();
        if let Some(code) = state.layout {
            self.layout.set_label(&t!("shell-greeter-layout", layout = code));
            let s = self.layout.preferred_size(ui.painter.text_system(), theme);
            x -= s.width;
            self.layout.set_rect(Rect::new(x, size.height - margin - s.height, s.width, s.height));
            self.layout.draw(ui);
        }
        if !self.focus_order().contains(&self.focus) {
            self.set_focus(Focus::Input, ui.now);
        }
        ui.wants_frame()
    }
}

/// Alanın altındaki satırlar: kilidin durum satırları (hata, gecikme, Caps Lock) + bu
/// denemenin PAM bilgi mesajları (ör. parmak izi istemi), en çok üç.
pub fn form_lines(state: &FormState<'_>) -> Vec<(String, Tone)> {
    let mut lines = status_lines(state.status, state.backoff, state.caps_lock, None);
    for m in state.notices {
        let tone = if m.kind == MessageKind::Error { Tone::Error } else { Tone::Normal };
        lines.push((m.text.clone(), tone));
    }
    lines.truncate(3);
    lines
}

/// PAM istem metninden yer tutucu: standart şifre istemi ("Password:") yerelleştirilir, diğer
/// istemler (doğrulama kodu…) PAM'in metniyle, sondaki iki nokta atılarak gösterilir.
pub fn placeholder_for(secret: bool, text: &str) -> String {
    let trimmed = text.trim().trim_end_matches(':').trim();
    let lower = trimmed.to_lowercase();
    if trimmed.is_empty() || (secret && matches!(lower.as_str(), "password" | "parola" | "şifre" | "passwort")) { // i18n: tanı PAM istemiyle karşılaştırma (gösterilmez)
        return if secret { t!("shell-greeter-password") } else { String::new() };
    }
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_pam::Outcome;

    fn state<'a>(status: &'a AuthStatus, notices: &'a [PamMessage]) -> FormState<'a> {
        FormState {
            display_name: Some("Ayşe"),
            user_name: Some("ayse"),
            many_users: false,
            prompt: None,
            status,
            notices,
            backoff: None,
            caps_lock: false,
            busy: false,
            session: Some("Nuxut"),
            layout: Some("TR"),
            power: &[],
        }
    }

    #[test]
    fn placeholders() {
        crate::i18n::use_turkish_for_tests();
        assert_eq!(placeholder_for(true, "Password: "), "Şifre");
        assert_eq!(placeholder_for(true, ""), "Şifre");
        assert_eq!(placeholder_for(false, "Verification code:"), "Verification code");
        assert_eq!(placeholder_for(true, "Doğrulama kodu"), "Doğrulama kodu");
    }

    #[test]
    fn notices_follow_status_lines() {
        crate::i18n::use_turkish_for_tests();
        let notices = [PamMessage { kind: MessageKind::Info, text: "Parmağınızı okuyucuya koyun".into() }];
        let idle = AuthStatus::Idle;
        let mut s = state(&idle, &notices);
        s.caps_lock = true;
        assert_eq!(
            form_lines(&s),
            [("Caps Lock açık".to_owned(), Tone::Error), ("Parmağınızı okuyucuya koyun".to_owned(), Tone::Normal)]
        );
        let failed = AuthStatus::Failed { outcome: Outcome::Denied, messages: vec![] };
        assert_eq!(form_lines(&state(&failed, &[]))[0].0, "Yanlış şifre");
    }
}
