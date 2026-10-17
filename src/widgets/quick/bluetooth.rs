//! Bluetooth kartı: adaptör anahtarı ve eşleşmiş cihazlar (tıklama bağlar / bağlantıyı
//! keser). Cihaz eşleştirme isteği gelirse kartın altında sorulur: kod doğrulama ve izin
//! istekleri kabul/ret düğmeleriyle; gösterilecek kodlar bilgi olarak. PIN girişi
//! gerektirenler `SystemServices` tarafından reddedilir (ayarlar uygulamasının işi).

use std::time::Instant;

use nuxut::ui::{Button, ButtonVariant, Icon, IconName, Label, List, ListEvent, ListItem, ListTrailing, TextRole, Ui, text_style};
use nuxut::i18n::t;
use nuxut::{CursorShape, Point, PointerEvent, Rect, TextSystem, Theme};

use super::{bluetooth_device_icon, draw_error, error_height, padding, set_flag};
use crate::model::ShellModel;
use crate::services::bluetooth::{BluetoothState, Device, PairingKind, PairingRequest};
use crate::system::{ErrorSource, SystemCommand};
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

/// Listede görünen en fazla satır (fazlası kaydırılır).
const VISIBLE_ROWS: usize = 5;
/// Listeye alınan en fazla cihaz.
const MAX_DEVICES: usize = 16;

#[derive(Debug, Clone, PartialEq)]
enum RowKind {
    Switch,
    /// Cihaz yolu ve bağlı mı.
    Device { path: String, connected: bool },
}

#[derive(Debug, Clone, PartialEq)]
struct RowSpec {
    kind: RowKind,
    title: String,
    subtitle: Option<String>,
    leading: IconName,
    trailing: ListTrailing,
}

#[derive(Debug)]
struct Prompt {
    id: u64,
    question: Label,
    /// Cevap gerekmiyorsa (kod gösterimi) düğme yok.
    buttons: Option<(Button, Button)>,
}

#[derive(Debug)]
pub struct BluetoothCard {
    rows: Vec<RowSpec>,
    list: List,
    prompt: Option<Prompt>,
    error: Label,
    has_error: bool,
}

pub(super) fn switch_subtitle(b: &BluetoothState) -> String {
    if !b.available {
        return t!("shell-quick-bluetooth-unavailable");
    }
    if b.adapter().is_none() {
        return t!("shell-quick-bluetooth-no-adapter");
    }
    if !b.powered() {
        return t!("shell-quick-state-off");
    }
    match b.devices.iter().filter(|d| d.connected).count() {
        0 => t!("shell-quick-bluetooth-no-devices"),
        1 => b.devices.iter().find(|d| d.connected).map(|d| d.name.clone()).unwrap_or_default(),
        n => t!("shell-quick-bluetooth-devices-connected", count = n),
    }
}

/// Eşleşmiş (ya da bağlı) cihazlar: bağlı olan önce, sonra ada göre.
fn devices(b: &BluetoothState) -> Vec<&Device> {
    let mut list: Vec<&Device> = b.devices.iter().filter(|d| (d.paired || d.connected) && !d.blocked).collect();
    list.sort_by(|a, b| b.connected.cmp(&a.connected).then_with(|| a.name.cmp(&b.name)));
    list.truncate(MAX_DEVICES);
    list
}

fn device_subtitle(d: &Device) -> Option<String> {
    match (d.connected, d.battery) {
        (true, Some(p)) => Some(t!("shell-quick-bluetooth-device-battery", percent = crate::percent::ratio(p))),
        (true, None) => Some(t!("shell-quick-state-connected")),
        (false, _) => None,
    }
}

fn row_specs(b: &BluetoothState) -> Vec<RowSpec> {
    let powered = b.powered();
    let mut rows = vec![RowSpec {
        kind: RowKind::Switch,
        title: "Bluetooth".into(),
        subtitle: Some(switch_subtitle(b)),
        leading: match (powered, b.devices.iter().any(|d| d.connected)) {
            (false, _) => IconName::BluetoothOff,
            (true, true) => IconName::BluetoothConnected,
            (true, false) => IconName::Bluetooth,
        },
        trailing: ListTrailing::Toggle(powered),
    }];
    if !powered {
        return rows;
    }
    rows.extend(devices(b).into_iter().map(|d| RowSpec {
        kind: RowKind::Device { path: d.path.clone(), connected: d.connected },
        title: d.name.clone(),
        subtitle: device_subtitle(d),
        leading: bluetooth_device_icon(d.icon.as_deref()),
        trailing: ListTrailing::None,
    }));
    rows
}

/// İstem metni ve cevap gerekip gerekmediği.
fn prompt_text(r: &PairingRequest) -> (String, bool) {
    let name = &r.name;
    match &r.kind {
        PairingKind::Confirm { passkey } => {
            (t!("shell-quick-bluetooth-confirm-passkey", name = name, passkey = format!("{passkey:06}")), true)
        }
        PairingKind::Authorize => (t!("shell-quick-bluetooth-authorize-connect", name = name), true),
        PairingKind::AuthorizeService { .. } => (t!("shell-quick-bluetooth-authorize-service", name = name), true),
        PairingKind::DisplayPinCode { pin } => (t!("shell-quick-bluetooth-display-code", name = name, code = pin), false),
        PairingKind::DisplayPasskey { passkey, .. } => {
            (t!("shell-quick-bluetooth-display-code", name = name, code = format!("{passkey:06}")), false)
        }
        // `SystemServices` beklemeden reddeder (PIN/passkey ve tanınmayan türler); buraya gelmez.
        _ => (t!("shell-quick-bluetooth-pin-request", name = name), false),
    }
}

fn build_item(theme: &Theme, r: &RowSpec) -> ListItem {
    let mut item = ListItem::new(theme, r.title.clone()).with_leading(theme, Icon::named(r.leading));
    if let Some(s) = &r.subtitle {
        item = item.with_subtitle(theme, s.clone());
    }
    item.with_trailing(theme, r.trailing.clone())
}

impl BluetoothCard {
    pub fn new(cx: &WidgetCx) -> Self {
        Self {
            rows: Vec::new(),
            list: List::new(cx.theme),
            prompt: None,
            error: Label::new("", text_style(cx.theme, TextRole::Caption)),
            has_error: false,
        }
    }

    fn list_height(&self, theme: &Theme) -> f32 {
        let gap = theme.space(1);
        let rows: f32 = self.list.items().iter().take(VISIBLE_ROWS).map(|i| i.height(theme) + gap).sum();
        (rows - gap).max(0.0)
    }

    fn prompt_height(p: &mut Prompt, text: &mut TextSystem, theme: &Theme) -> f32 {
        let q = p.question.measure(text).height;
        let b = p.buttons.as_mut().map_or(0.0, |(a, _)| theme.space(1) + a.preferred_size(text, theme).height);
        q + b
    }

    /// (liste, istem, hata konumu).
    fn layout(&mut self, text: &mut TextSystem, rect: Rect, cx: &WidgetCx) -> (Rect, Rect, Point) {
        let t = cx.theme;
        let p = padding(t);
        let w = rect.width - 2.0 * p;
        let list = Rect::new(rect.x + p, rect.y + p, w, self.list_height(t));
        let mut y = list.bottom();
        let prompt = match &mut self.prompt {
            Some(pr) => {
                y += t.space(2);
                let h = Self::prompt_height(pr, text, t);
                let r = Rect::new(rect.x + p, y, w, h);
                y += h;
                r
            }
            None => Rect::ZERO,
        };
        (list, prompt, Point::new(rect.x + p, y + t.space(1)))
    }

    fn place_buttons(p: &mut Prompt, text: &mut TextSystem, theme: &Theme, rect: Rect) {
        let q = p.question.measure(text).height;
        if let Some((accept, reject)) = &mut p.buttons {
            let a = accept.preferred_size(text, theme);
            let r = reject.preferred_size(text, theme);
            let y = rect.y + q + theme.space(1);
            accept.set_rect(Rect::new(rect.right() - a.width, y, a.width, a.height));
            reject.set_rect(Rect::new(rect.right() - a.width - theme.space(2) - r.width, y, r.width, r.height));
        }
    }

    fn sync_prompt(&mut self, req: Option<&PairingRequest>, width: f32, cx: &WidgetCx) -> bool {
        match (req, &self.prompt) {
            (Some(r), Some(p)) if p.id == r.id => false,
            (Some(r), _) => {
                let t = cx.theme;
                let (text, reply) = prompt_text(r);
                let mut question = Label::new(text, text_style(t, TextRole::Body));
                question.set_max_width(Some(width));
                let accept = match r.kind {
                    PairingKind::Confirm { .. } => t!("shell-quick-bluetooth-pair"),
                    _ => t!("shell-quick-bluetooth-allow"),
                };
                let buttons = reply.then(|| {
                    (
                        Button::new(t, ButtonVariant::Filled).with_label(t, accept),
                        Button::new(t, ButtonVariant::Text).with_label(t, t!("shell-quick-bluetooth-deny")),
                    )
                });
                self.prompt = Some(Prompt { id: r.id, question, buttons });
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

impl PanelWidget for BluetoothCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Bluetooth
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let s = &model.system;
        let mut changed = false;
        let rows = row_specs(&s.bluetooth);
        if rows != self.rows {
            let reusable = rows.len() == self.rows.len()
                && rows.iter().zip(&self.rows).all(|(a, b)| a.kind == b.kind && a.leading == b.leading && a.trailing == b.trailing);
            if reusable {
                for (item, r) in self.list.items_mut().iter_mut().zip(&rows) {
                    item.set_title(&r.title);
                    item.set_subtitle(cx.theme, r.subtitle.as_deref());
                }
            } else {
                *self.list.items_mut() = rows.iter().map(|r| build_item(cx.theme, r)).collect();
            }
            if let Some(item) = self.list.items_mut().first_mut() {
                item.set_toggle(s.bluetooth.powered(), cx.now);
                item.set_enabled(s.bluetooth.adapter().is_some(), cx.now);
            }
            self.rows = rows;
            changed = true;
        }
        let width = (cx.bounds.width - 2.0 * padding(cx.theme)).max(0.0);
        changed |= self.sync_prompt(s.pairing.as_ref(), width, cx);
        let err = s.error_for(ErrorSource::Bluetooth).unwrap_or_default();
        changed |= set_flag(&mut self.has_error, !err.is_empty());
        changed |= self.error.set_text(err);
        changed
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        let t = cx.theme;
        self.list.set_theme(t, cx.now);
        if let Some(p) = &mut self.prompt {
            p.question.set_style(text_style(t, TextRole::Body));
            if let Some((a, r)) = &mut p.buttons {
                a.set_theme(t, cx.now);
                r.set_theme(t, cx.now);
            }
        }
        self.error.set_style(text_style(t, TextRole::Caption));
    }

    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        let p = padding(cx.theme);
        if let Some(pr) = &mut self.prompt {
            pr.question.set_max_width(Some((width - 2.0 * p).max(0.0)));
        }
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
            Self::place_buttons(p, ui.painter.text_system(), t, prompt);
            let c = ui.fade(t.colors.on_surface);
            p.question.draw(ui.painter, prompt.origin(), c);
            if let Some((a, r)) = &mut p.buttons {
                r.draw(ui);
                a.draw(ui);
            }
        }
        if self.has_error {
            draw_error(ui, &mut self.error, err, t);
        }
    }

    /// Bluetooth anahtarı ve cihaz listesi (anahtar cihaz yolu); eşleştirme sorusu ve düğmeleri.
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        if self.rows.is_empty() && self.prompt.is_none() {
            return;
        }
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            let rows = &self.rows;
            self.list
                .access(tree, "devices", |i| match rows.get(i).map(|r| &r.kind) {
                    Some(RowKind::Switch) => "switch".to_owned(),
                    Some(RowKind::Device { path, .. }) => path.clone(),
                    None => i.to_string(),
                })
                .label(&self.kind().title());
            if let Some(p) = &self.prompt {
                p.question.access(tree, "question", rect).live(nuxut::access::Live::Polite);
                if let Some((deny, allow)) = &p.buttons {
                    deny.access(tree, "deny");
                    allow.access(tree, "allow");
                }
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
            Self::place_buttons(p, text, t, prompt);
            if let Some((a, r)) = &mut p.buttons {
                let ar = a.pointer(event, cx.now);
                let rr = r.pointer(event, cx.now);
                repaint |= ar.repaint | rr.repaint;
                let answer = if ar.event.is_some() {
                    Some(true)
                } else if rr.event.is_some() {
                    Some(false)
                } else {
                    None
                };
                if let Some(accept) = answer {
                    let id = p.id;
                    self.prompt = None;
                    return (true, Some(Action::System(SystemCommand::PairingReply { id, accept })));
                }
            }
        }
        let r = self.list.pointer(t, event, cx.now);
        repaint |= r.repaint;
        let powered = self.rows.first().is_some_and(|r| r.trailing == ListTrailing::Toggle(true));
        let cmd = r.event.and_then(|(i, e)| match (&self.rows.get(i)?.kind, e) {
            (RowKind::Switch, ListEvent::Toggled(on)) => Some(SystemCommand::SetBluetooth(on)),
            (RowKind::Switch, ListEvent::Clicked) => Some(SystemCommand::SetBluetooth(!powered)),
            (RowKind::Device { path, connected: true }, _) => Some(SystemCommand::BluetoothDisconnect(path.clone())),
            (RowKind::Device { path, connected: false }, _) => Some(SystemCommand::BluetoothConnect(path.clone())),
        });
        (repaint || cmd.is_some(), cmd.map(Action::System))
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        if let Some((a, r)) = self.prompt.as_ref().and_then(|pr| pr.buttons.as_ref())
            && let Some(c) = a.cursor(p).or_else(|| r.cursor(p))
        {
            return Some(c);
        }
        self.list.cursor(p)
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::bluetooth::Adapter;

    fn device(path: &str, name: &str, paired: bool, connected: bool) -> Device {
        Device {
            path: path.into(),
            adapter: "/org/bluez/hci0".into(),
            address: "00:11:22:33:44:55".into(),
            name: name.into(),
            icon: Some("audio-headset".into()),
            connected,
            paired,
            trusted: paired,
            blocked: false,
            battery: connected.then_some(80),
            rssi: None,
            ..Device::default()
        }
    }

    fn state(powered: bool) -> BluetoothState {
        BluetoothState {
            available: true,
            adapters: vec![Adapter {
                path: "/org/bluez/hci0".into(),
                name: "hci0".into(),
                address: "AA".into(),
                powered,
                discovering: false,
                pairable: false,
                discoverable: false,
                discoverable_timeout: 0,
                power_state: Default::default(),
            }],
            devices: vec![
                device("/d/b", "Bilinmeyen", false, false),
                device("/d/z", "Zil", true, false),
                device("/d/k", "Kulaklık", true, true),
            ],
            ..BluetoothState::default()
        }
    }

    #[test]
    fn rows_list_paired_connected_first() {
        crate::i18n::use_turkish_for_tests();
        let rows = row_specs(&state(true));
        assert_eq!(rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["Bluetooth", "Kulaklık", "Zil"]);
        assert_eq!(rows[0].subtitle.as_deref(), Some("Kulaklık"));
        assert_eq!(rows[0].leading, IconName::BluetoothConnected);
        assert_eq!(rows[1].subtitle.as_deref(), Some("Bağlı · pil %80"));
        assert_eq!(rows[1].leading, IconName::Headphones);
        assert_eq!(rows[2].subtitle, None);
        let off = row_specs(&state(false));
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].subtitle.as_deref(), Some("Kapalı"));
    }

    #[test]
    fn prompts_need_reply_only_for_questions() {
        crate::i18n::use_turkish_for_tests();
        let req = |kind| PairingRequest { id: 1, device: "/d".into(), name: "Kulaklık".into(), kind };
        assert_eq!(prompt_text(&req(PairingKind::Confirm { passkey: 42 })), ("Kulaklık: 000042 kodu cihazdakiyle aynı mı?".into(), true));
        assert!(prompt_text(&req(PairingKind::Authorize)).1);
        assert!(!prompt_text(&req(PairingKind::DisplayPinCode { pin: "1234".into() })).1);
    }
}
