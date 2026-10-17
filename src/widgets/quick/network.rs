//! Ağ kartı: Wi-Fi anahtarı, kablolu bağlantı durumu ve yakındaki ağlar. Kayıtlı ya da açık
//! ağa tıklamak bağlanır, bağlı ağa tıklamak bağlantıyı keser; şifreli yeni ağda (ya da NM
//! şifre isteyince) kartın altında şifre alanı açılır. Kurumsal (802.1X) ağlar ayarlar
//! uygulamasının işi.
//!
//! Şifre yalnızca `TextInput`'un sıfırlanan tamponunda ve `Secret`'ta yaşar; loglanmaz.

use std::time::Instant;

use nuxut::ui::{
    Button, ButtonVariant, Icon, IconName, Label, List, ListEvent, ListItem, ListTrailing, PASSWORD_MAX_BYTES,
    TextInput, TextInputEvent, TextRole, Ui, text_style,
};
use nuxut::i18n::t;
use nuxut::{CursorShape, KeyEvent, Point, PointerEvent, Rect, TextSystem, Theme};

use super::{draw_error, error_height, padding, set_flag, wifi_icon};
use crate::model::ShellModel;
use crate::services::network::{
    AccessPoint, ActiveState, ConnectionKind, NetworkState, Secret, SecretKind, SecretRequest, Security, valid_secret,
};
use crate::system::{ErrorSource, SystemCommand, SystemModel};
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

/// Listede gösterilen en fazla ağ satırı (fazlası kaydırılır).
const VISIBLE_ROWS: usize = 6;
/// Listeye alınan en fazla ağ.
const MAX_NETWORKS: usize = 24;

/// Bir satırın ne olduğu (tıklamada ne yapılacağı).
#[derive(Debug, Clone, PartialEq)]
enum RowKind {
    WifiSwitch,
    Wired,
    /// Erişim noktası (`AccessPoint::path`).
    Network { path: String, ssid: String, security: Security, saved: bool, active: Option<String> },
}

/// Satırın görünür içeriği: değişmediyse liste yeniden kurulmaz (hover durumu korunur).
#[derive(Debug, Clone, PartialEq)]
struct RowSpec {
    kind: RowKind,
    title: String,
    subtitle: Option<String>,
    leading: IconName,
    trailing: ListTrailing,
}

/// Şifre isteminin hedefi.
#[derive(Debug, Clone, PartialEq)]
enum PromptTarget {
    /// Yeni ağa bağlanırken.
    AccessPoint { path: String, security: Security },
    /// NM ajanının isteği.
    Agent { id: u64 },
}

#[derive(Debug)]
struct Prompt {
    target: PromptTarget,
    title: Label,
    input: TextInput,
    cancel: Button,
    connect: Button,
    /// Doğrulanacak güvenlik türü (ajan isteğinde bilinmiyorsa WPA-PSK kuralı).
    security: Security,
}

#[derive(Debug)]
pub struct NetworkCard {
    rows: Vec<RowSpec>,
    list: List,
    prompt: Option<Prompt>,
    /// Kartın kendi uyarısı (servise gitmeyen tıklama); sonraki tıklamada silinir.
    notice: Option<String>,
    model_error: Option<String>,
    error: Label,
    has_error: bool,
}

/// Wi-Fi satırının alt metni.
pub(super) fn wifi_subtitle(n: &NetworkState) -> String {
    if !n.available {
        return t!("shell-quick-network-nm-unavailable");
    }
    if !n.wifi_hardware_enabled {
        return t!("shell-quick-network-hardware-off");
    }
    if !n.wifi_enabled {
        return t!("shell-quick-state-off");
    }
    match n.active_access_point() {
        Some(ap) => ap.ssid.clone(),
        None => t!("shell-quick-network-disconnected"),
    }
}

/// Erişim noktasının etkin bağlantısı (bağlıysa ya da bağlanıyorsa): yolu ve durumu.
fn active_for<'a>(n: &'a NetworkState, ap: &AccessPoint) -> Option<(&'a str, ActiveState)> {
    n.active.iter().filter(|a| a.kind == ConnectionKind::Wifi).find_map(|a| {
        let saved = n.saved.iter().find(|s| s.path == a.connection)?;
        (saved.ssid.as_deref() == Some(ap.ssid_bytes.as_slice())).then_some((a.path.as_str(), a.state))
    })
}

fn row_specs(s: &SystemModel) -> Vec<RowSpec> {
    let n = &s.network;
    let mut rows = vec![RowSpec {
        kind: RowKind::WifiSwitch,
        title: "Wi-Fi".into(),
        subtitle: Some(wifi_subtitle(n)),
        leading: if n.wifi_enabled { IconName::Wifi } else { IconName::WifiOff },
        trailing: ListTrailing::Toggle(n.wifi_enabled),
    }];
    if n.primary_kind == Some(ConnectionKind::Ethernet) {
        rows.push(RowSpec {
            kind: RowKind::Wired,
            title: t!("shell-quick-network-wired"),
            subtitle: n.primary_name.clone(),
            leading: IconName::Ethernet,
            trailing: ListTrailing::Text(t!("shell-quick-state-connected")),
        });
    }
    if !n.wifi_enabled {
        return rows;
    }
    // Servis SSID başına en güçlü BSS'i, etkin önce ve sinyale göre sıralı verir.
    for ap in n.access_points.iter().take(MAX_NETWORKS) {
        let active = active_for(n, ap);
        let status = match active.map(|(_, st)| st) {
            Some(ActiveState::Activated) => Some(t!("shell-quick-state-connected")),
            Some(ActiveState::Activating) => Some(t!("shell-quick-network-connecting")),
            Some(ActiveState::Deactivating) => Some(t!("shell-quick-network-disconnecting")),
            _ if ap.saved => Some(t!("shell-quick-network-saved")),
            _ => None,
        };
        let trailing = if ap.security.needs_password() || ap.security == Security::Enterprise {
            ListTrailing::Icon(Icon::named(IconName::Lock))
        } else {
            ListTrailing::None
        };
        rows.push(RowSpec {
            kind: RowKind::Network {
                path: ap.path.clone(),
                ssid: ap.ssid.clone(),
                security: ap.security,
                saved: ap.saved,
                active: active.map(|(p, _)| p.to_owned()),
            },
            title: ap.ssid.clone(),
            subtitle: status,
            leading: wifi_icon(ap.strength),
            trailing,
        });
    }
    rows
}

fn build_item(theme: &Theme, r: &RowSpec) -> ListItem {
    let mut item = ListItem::new(theme, r.title.clone()).with_leading(theme, Icon::named(r.leading));
    if let Some(s) = &r.subtitle {
        item = item.with_subtitle(theme, s.clone());
    }
    item.with_trailing(theme, r.trailing.clone())
}

impl Prompt {
    fn new(theme: &Theme, target: PromptTarget, title: &str, security: Security, now: Instant) -> Self {
        let mut input = TextInput::new(theme).password(true).with_max_bytes(PASSWORD_MAX_BYTES).with_placeholder(t!("shell-quick-network-password-placeholder"));
        input.set_focused(true, now);
        let mut connect = Button::new(theme, ButtonVariant::Filled).with_label(theme, t!("shell-quick-network-connect"));
        connect.set_enabled(false, now);
        Self {
            target,
            title: Label::new(title, text_style(theme, TextRole::Body).with_max_lines(1)),
            input,
            cancel: Button::new(theme, ButtonVariant::Text).with_label(theme, t!("shell-quick-network-cancel")),
            connect,
            security,
        }
    }

    fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.title.set_style(text_style(theme, TextRole::Body).with_max_lines(1));
        self.input.set_theme(theme, now);
        self.cancel.set_theme(theme, now);
        self.connect.set_theme(theme, now);
    }

    fn valid(&self) -> bool {
        self.input.with_secret(|s| valid_secret(self.security, s))
    }

    /// Gönderim komutu. Yeni ağda vazgeçmek servise bir şey göndermez (`None`); ajan
    /// isteğinde vazgeçmek NM'ye "şifre yok" der.
    fn command(&self, submit: bool) -> Option<SystemCommand> {
        let secret = submit.then(|| self.input.with_secret(|s: &str| Secret::new(s.to_owned())));
        match &self.target {
            PromptTarget::AccessPoint { path, .. } => {
                secret.map(|s| SystemCommand::ConnectAccessPoint { path: path.clone(), password: Some(s) })
            }
            PromptTarget::Agent { id } => Some(SystemCommand::ProvideSecret { id: *id, secret }),
        }
    }

    fn height(&mut self, text: &mut TextSystem, theme: &Theme, width: f32) -> f32 {
        let t = self.title.measure(text).height;
        let i = self.input.preferred_size(theme, width).height;
        let b = self.connect.preferred_size(text, theme).height;
        t + theme.space(1) + i + theme.space(1) + b
    }

    /// (başlık, giriş, vazgeç, bağlan).
    fn place(&mut self, text: &mut TextSystem, theme: &Theme, rect: Rect) -> Point {
        let t = self.title.measure(text).height;
        let title = Point::new(rect.x, rect.y);
        let i = self.input.preferred_size(theme, rect.width).height;
        let input = Rect::new(rect.x, rect.y + t + theme.space(1), rect.width, i);
        self.input.set_rect(input);
        let c = self.connect.preferred_size(text, theme);
        let y = input.bottom() + theme.space(1);
        self.connect.set_rect(Rect::new(rect.right() - c.width, y, c.width, c.height));
        let v = self.cancel.preferred_size(text, theme);
        self.cancel.set_rect(Rect::new(rect.right() - c.width - theme.space(2) - v.width, y, v.width, v.height));
        title
    }
}

impl NetworkCard {
    pub fn new(cx: &WidgetCx) -> Self {
        Self {
            rows: Vec::new(),
            list: List::new(cx.theme),
            prompt: None,
            notice: None,
            model_error: None,
            error: Label::new("", text_style(cx.theme, TextRole::Caption)),
            has_error: false,
        }
    }

    fn list_height(&self, theme: &Theme) -> f32 {
        let gap = theme.space(1);
        let items = self.list.items();
        let full: f32 = items.iter().map(|i| i.height(theme) + gap).sum::<f32>() - gap;
        let visible: f32 = items.iter().take(VISIBLE_ROWS).map(|i| i.height(theme) + gap).sum::<f32>() - gap;
        full.min(visible).max(0.0)
    }

    /// (liste, istem alanı, hata konumu).
    fn layout(&mut self, text: &mut TextSystem, rect: Rect, cx: &WidgetCx) -> (Rect, Rect, Point) {
        let t = cx.theme;
        let p = padding(t);
        let w = rect.width - 2.0 * p;
        let list = Rect::new(rect.x + p, rect.y + p, w, self.list_height(t));
        let mut y = list.bottom();
        let prompt = match &mut self.prompt {
            Some(pr) => {
                y += t.space(2);
                let h = pr.height(text, t, w);
                let r = Rect::new(rect.x + p, y, w, h);
                y += h;
                r
            }
            None => Rect::ZERO,
        };
        (list, prompt, Point::new(rect.x + p, y + t.space(1)))
    }

    fn open_prompt(&mut self, target: PromptTarget, title: &str, security: Security, cx: &WidgetCx) {
        self.prompt = Some(Prompt::new(cx.theme, target, title, security, cx.now));
    }

    fn row_clicked(&mut self, index: usize, event: ListEvent, model_wifi: bool, cx: &WidgetCx) -> Option<SystemCommand> {
        let kind = self.rows.get(index)?.kind.clone();
        match (&kind, event) {
            (RowKind::WifiSwitch, ListEvent::Toggled(on)) => Some(SystemCommand::SetWifi(on)),
            (RowKind::WifiSwitch, ListEvent::Clicked) => Some(SystemCommand::SetWifi(!model_wifi)),
            (RowKind::Wired, _) => None,
            (RowKind::Network { path, ssid, security, saved, active }, _) => {
                if let Some(a) = active {
                    return Some(SystemCommand::Deactivate(a.clone()));
                }
                if *security == Security::Enterprise {
                    self.notice = Some(t!("shell-quick-network-enterprise-notice"));
                    return None;
                }
                if security.needs_password() && !saved {
                    let (path, ssid, security) = (path.clone(), ssid.clone(), *security);
                    self.open_prompt(PromptTarget::AccessPoint { path, security }, &t!("shell-quick-network-password-title", ssid = ssid), security, cx);
                    return None;
                }
                Some(SystemCommand::ConnectAccessPoint { path: path.clone(), password: None })
            }
        }
    }

    /// Hata satırı: kartın kendi uyarısı önce, yoksa servis hatası.
    fn refresh_error(&mut self) -> bool {
        let text = self.notice.as_deref().or(self.model_error.as_deref()).unwrap_or_default();
        let mut changed = set_flag(&mut self.has_error, !text.is_empty());
        changed |= self.error.set_text(text);
        changed
    }

    fn finish_prompt(&mut self, submit: bool) -> Option<Action> {
        self.prompt.take()?.command(submit).map(Action::System)
    }
}

impl PanelWidget for NetworkCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Network
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let s = &model.system;
        let mut changed = false;
        let rows = row_specs(s);
        if rows != self.rows {
            let reusable = rows.len() == self.rows.len()
                && rows.iter().zip(&self.rows).all(|(a, b)| a.kind == b.kind && a.leading == b.leading && a.trailing == b.trailing);
            if reusable {
                // Yalnızca metinler değişti: satırları koru (hover/anahtar animasyonu sürer).
                for (item, r) in self.list.items_mut().iter_mut().zip(&rows) {
                    item.set_title(&r.title);
                    item.set_subtitle(cx.theme, r.subtitle.as_deref());
                }
            } else {
                *self.list.items_mut() = rows.iter().map(|r| build_item(cx.theme, r)).collect();
            }
            if let Some(item) = self.list.items_mut().first_mut() {
                item.set_toggle(s.network.wifi_enabled, cx.now);
                item.set_enabled(s.network.available && s.network.wifi_hardware_enabled, cx.now);
            }
            self.rows = rows;
            changed = true;
        }
        changed |= self.sync_agent_request(s.secret_request.as_ref(), cx);
        self.model_error = s.error_for(ErrorSource::Network).map(str::to_owned);
        changed | self.refresh_error()
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.list.set_theme(cx.theme, cx.now);
        if let Some(p) = &mut self.prompt {
            p.set_theme(cx.theme, cx.now);
        }
        self.error.set_style(text_style(cx.theme, TextRole::Caption));
    }

    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        let p = padding(cx.theme);
        let (_, _, err) = self.layout(text, Rect::new(0.0, 0.0, width, 0.0), cx);
        self.error.set_max_width(Some((width - 2.0 * p).max(0.0)));
        let e = error_height(&mut self.error, text, cx.theme, self.has_error);
        Some(err.y - cx.theme.space(1) + e + p)
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        let t = cx.theme;
        let (list, prompt, err) = self.layout(ui.painter.text_system(), rect, cx);
        if self.list.rect() != list {
            self.list.set_rect(t, list, cx.now);
        }
        self.list.draw(ui);
        if let Some(p) = &mut self.prompt {
            let title = p.place(ui.painter.text_system(), t, prompt);
            let c = ui.fade(t.colors.on_surface);
            p.title.draw(ui.painter, title, c);
            p.input.draw(ui);
            p.cancel.draw(ui);
            p.connect.draw(ui);
        }
        if self.has_error {
            draw_error(ui, &mut self.error, err, t);
        }
    }

    /// Wi-Fi anahtarı, kablolu bağlantı ve ağ listesi (anahtar erişim noktası yolu); açıkken
    /// şifre istemi (parola alanı yalnız maske; ADR-0011 karar 6) ve düğmeleri.
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        if self.rows.is_empty() && self.prompt.is_none() {
            return;
        }
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            let rows = &self.rows;
            self.list
                .access(tree, "networks", |i| match rows.get(i).map(|r| &r.kind) {
                    Some(RowKind::WifiSwitch) => "wifi".to_owned(),
                    Some(RowKind::Wired) => "wired".to_owned(),
                    Some(RowKind::Network { path, .. }) => path.clone(),
                    None => i.to_string(),
                })
                .label(&self.kind().title());
            if let Some(p) = &self.prompt {
                p.input.access(tree, "password").label(p.title.text());
                p.cancel.access(tree, "cancel");
                p.connect.access(tree, "connect");
            }
            if self.has_error {
                self.error.access(tree, "error", rect);
            }
        });
    }

    fn pointer(&mut self, event: &PointerEvent, rect: Rect, text: &mut TextSystem, cx: &WidgetCx) -> (bool, Option<Action>) {
        let t = cx.theme;
        let (list, prompt, _) = self.layout(text, rect, cx);
        if self.list.rect() != list {
            self.list.set_rect(t, list, cx.now);
        }
        let mut repaint = false;
        if let Some(p) = &mut self.prompt {
            p.place(text, t, prompt);
            let i = p.input.pointer(text, t, event, cx.now);
            let c = p.cancel.pointer(event, cx.now);
            let k = p.connect.pointer(event, cx.now);
            repaint |= i.repaint | c.repaint | k.repaint;
            if c.event.is_some() {
                return (true, self.finish_prompt(false));
            }
            if k.event.is_some() && p.valid() {
                return (true, self.finish_prompt(true));
            }
        }
        let r = self.list.pointer(t, event, cx.now);
        repaint |= r.repaint;
        let wifi_on = self.rows.first().is_some_and(|r| r.trailing == ListTrailing::Toggle(true));
        let action = r.event.and_then(|(i, e)| {
            self.notice = None;
            let cmd = self.row_clicked(i, e, wifi_on, cx);
            repaint |= self.refresh_error();
            cmd
        });
        let action = action.map(Action::System);
        (repaint || action.is_some() || self.prompt.is_some(), action)
    }

    fn key(&mut self, event: &KeyEvent, cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        let p = self.prompt.as_mut()?;
        let r = p.input.key(event, cx.now);
        let valid = p.valid();
        p.connect.set_enabled(valid, cx.now);
        match r.event {
            Some(TextInputEvent::Submit) if valid => Some((true, self.finish_prompt(true))),
            Some(TextInputEvent::Cancel) => Some((true, self.finish_prompt(false))),
            Some(TextInputEvent::PasteRequested) => Some((r.repaint, Some(Action::RequestPaste))),
            // Şifre alanı Copy/Cut üretmez (`password(true)`); pano yazılmaz.
            _ => Some((r.repaint, None)),
        }
    }

    fn paste(&mut self, text: &str, cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        let p = self.prompt.as_mut()?;
        let r = p.input.paste(text);
        p.connect.set_enabled(p.valid(), cx.now);
        Some((r.repaint || r.event.is_some(), None))
    }

    fn panel_closed(&mut self, _now: Instant) {
        // Ajan isteği açık kalır (NM zaman aşımına kadar bekler); yeni ağ istemi kapanır.
        if matches!(self.prompt.as_ref().map(|p| &p.target), Some(PromptTarget::AccessPoint { .. })) {
            self.prompt = None;
        }
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        if let Some(pr) = &self.prompt
            && let Some(c) = pr.input.cursor(p).or_else(|| pr.cancel.cursor(p)).or_else(|| pr.connect.cursor(p))
        {
            return Some(c);
        }
        self.list.cursor(p)
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        false
    }
}

impl NetworkCard {
    /// NM şifre isterse istemi açar; istek bitince (başka yerden cevaplandı) kapatır.
    fn sync_agent_request(&mut self, req: Option<&SecretRequest>, cx: &WidgetCx) -> bool {
        let current = match self.prompt.as_ref().map(|p| &p.target) {
            Some(PromptTarget::Agent { id }) => Some(*id),
            _ => None,
        };
        match (req, current) {
            (Some(r), Some(id)) if r.id == id => false,
            (Some(r), _) => {
                let name = r.ssid.as_deref().unwrap_or(&r.connection);
                let title = if r.retry {
                    t!("shell-quick-network-password-retry", name = name)
                } else {
                    t!("shell-quick-network-password-title", ssid = name)
                };
                // Ajan isteğinde güvenlik türü bilinmez: WEP anahtarı ayrı, diğerleri WPA kuralıyla.
                let security = match r.kind {
                    SecretKind::WepKey => Security::Wep,
                    // WPA kuralı en geniş kişisel kural; gelecekteki türler de onunla.
                    _ => Security::WpaPsk,
                };
                self.open_prompt(PromptTarget::Agent { id: r.id }, &title, security, cx);
                true
            }
            (None, Some(_)) => {
                self.prompt = None;
                true
            }
            (None, None) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::network::{ActiveConnection, SavedConnection};

    fn ap(path: &str, ssid: &str, strength: u8, security: Security, saved: bool) -> AccessPoint {
        AccessPoint {
            path: path.into(),
            ssid: ssid.into(),
            ssid_bytes: ssid.as_bytes().to_vec(),
            strength,
            security,
            frequency: 2412,
            active: false,
            saved,
        }
    }

    fn state() -> NetworkState {
        NetworkState { available: true, wifi_enabled: true, wifi_hardware_enabled: true, ..NetworkState::default() }
    }

    #[test]
    fn rows_describe_state() {
        crate::i18n::use_turkish_for_tests();
        let mut s = SystemModel { network: state(), ..SystemModel::default() };
        s.network.access_points = vec![ap("/a", "ev", 80, Security::WpaPsk, true), ap("/b", "kafe", 30, Security::Open, false)];
        s.network.saved = vec![SavedConnection {
            path: "/s/1".into(),
            id: "ev".into(),
            uuid: "u".into(),
            kind: ConnectionKind::Wifi,
            ssid: Some(b"ev".to_vec()),
            autoconnect: true,
            metered: Default::default(),
            active: Some("/act/1".into()),
        }];
        s.network.active = vec![ActiveConnection {
            path: "/act/1".into(),
            id: "ev".into(),
            kind: ConnectionKind::Wifi,
            state: ActiveState::Activated,
            connection: "/s/1".into(),
        }];
        let rows = row_specs(&s);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].trailing, ListTrailing::Toggle(true));
        assert_eq!(rows[1].subtitle.as_deref(), Some("Bağlı"));
        assert_eq!(rows[1].leading, IconName::Wifi4);
        assert!(matches!(&rows[1].kind, RowKind::Network { active: Some(a), .. } if a == "/act/1"));
        assert_eq!(rows[2].subtitle, None);
        assert_eq!(rows[2].trailing, ListTrailing::None);

        s.network.wifi_enabled = false;
        let rows = row_specs(&s);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subtitle.as_deref(), Some("Kapalı"));
    }

    #[test]
    fn pasted_password_reaches_connect_command() {
        let theme = Theme::default();
        let settings = crate::widgets::WidgetSettings::default();
        let cx = WidgetCx {
            theme: &theme,
            settings: &settings,
            vertical: false,
            item: 26.0,
            now: Instant::now(),
            bounds: Rect::new(0.0, 0.0, 1920.0, 1080.0),
        };
        let mut card = NetworkCard::new(&cx);
        assert!(card.paste("yok", &cx).is_none(), "istem yokken tüketilmez");
        let target = PromptTarget::AccessPoint { path: "/ap/1".into(), security: Security::WpaPsk };
        card.open_prompt(target, "ev", Security::WpaPsk, &cx);
        assert!(card.paste("gizli-sifre", &cx).is_some());
        let expected = SystemCommand::ConnectAccessPoint { path: "/ap/1".into(), password: Some(Secret::new("gizli-sifre")) };
        assert_eq!(card.finish_prompt(true), Some(Action::System(expected)));
    }
}
