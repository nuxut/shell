//! Ses kartı: varsayılan çıkış ve giriş için sessiz düğmesi + seviye kaydırıcısı; birden fazla
//! çıkış (ya da giriş) cihazı varsa kaydırıcının altında cihaz listesi — tek tıkla varsayılan
//! cihaz değişir, etkin cihaz seçili ve işaretli (`docs/design/audio-devices.md`).
//! Kaydırıcı 0..`max_volume` aralığını gösterir (yükseltme açıksa %100'ün üstü).

use std::time::Instant;

use nuxut::i18n::t;

use nuxut::ui::{Icon, IconName, Label, List, ListDensity, ListEvent, ListItem, ListTrailing, TextRole, Ui, text_style};
use nuxut::{CursorShape, Point, PointerEvent, Rect, TextSystem, Theme};

use super::slider_row::{RowEvent, SliderRow};
use super::{draw_error, error_height, mic_icon, padding, set_flag, volume_icon};
use crate::model::ShellModel;
use crate::services::audio::{AudioDevice, AudioTarget, DeviceClass};
use crate::system::{ErrorSource, SystemCommand};
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

/// Cihaz listesinde görünen en fazla satır (fazlası liste içinde kaydırılır).
const VISIBLE_DEVICES: usize = 4;

/// Cihaz türü → ikon.
pub(super) fn device_icon(class: DeviceClass) -> IconName {
    match class {
        DeviceClass::Speaker => IconName::Speaker,
        DeviceClass::Headphones | DeviceClass::Headset => IconName::Headphones,
        DeviceClass::Display => IconName::Monitor,
        DeviceClass::Bluetooth => IconName::Bluetooth,
        DeviceClass::Usb => IconName::Usb,
        DeviceClass::Microphone => IconName::Mic,
        DeviceClass::Webcam => IconName::Videocam,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DeviceSpec {
    id: u32,
    title: String,
    icon: IconName,
    active: bool,
}

/// Seçilecek bir şey yoksa (0–1 cihaz) liste yok.
fn device_specs(devices: &[AudioDevice]) -> Vec<DeviceSpec> {
    if devices.len() < 2 {
        return Vec::new();
    }
    devices
        .iter()
        .map(|d| DeviceSpec { id: d.id, title: d.description.clone(), icon: device_icon(d.class), active: d.is_default })
        .collect()
}

/// Bir yönün (çıkış/giriş) cihaz listesi.
#[derive(Debug)]
struct DevicePicker {
    specs: Vec<DeviceSpec>,
    list: List,
}

impl DevicePicker {
    fn new(theme: &Theme) -> Self {
        Self { specs: Vec::new(), list: List::new(theme).with_density(ListDensity::Compact) }
    }

    fn visible(&self) -> bool {
        !self.specs.is_empty()
    }

    fn sync(&mut self, devices: &[AudioDevice], theme: &Theme) -> bool {
        let specs = device_specs(devices);
        if specs == self.specs {
            return false;
        }
        let density = self.list.density();
        *self.list.items_mut() = specs
            .iter()
            .map(|s| {
                let trailing = if s.active { ListTrailing::Icon(Icon::named(IconName::Check)) } else { ListTrailing::None };
                let mut item = ListItem::new(theme, s.title.clone())
                    .with_leading(theme, Icon::named(s.icon))
                    .with_trailing(theme, trailing);
                item.set_selected(s.active);
                // Liste yoğunluğu öğeye ancak yerleşimde geçer; yükseklik ondan önce sorulur.
                item.set_density(density);
                item
            })
            .collect();
        self.specs = specs;
        true
    }

    fn height(&self, theme: &Theme) -> f32 {
        let gap = theme.space(1);
        let rows: f32 = self.list.items().iter().take(VISIBLE_DEVICES).map(|i| i.height(theme) + gap).sum();
        (rows - gap).max(0.0)
    }

    fn place(&mut self, theme: &Theme, rect: Rect, now: Instant) {
        if self.list.rect() != rect {
            self.list.set_rect(theme, rect, now);
        }
    }

    /// Etkin olmayan cihaza tık → varsayılan yap.
    fn pointer(&mut self, event: &PointerEvent, theme: &Theme, now: Instant) -> (bool, Option<SystemCommand>) {
        let r = self.list.pointer(theme, event, now);
        let cmd = r.event.and_then(|(i, e)| {
            let s = self.specs.get(i)?;
            (e == ListEvent::Clicked && !s.active).then_some(SystemCommand::SetDefaultAudio(s.id))
        });
        (r.repaint, cmd)
    }
}

#[derive(Debug)]
pub struct VolumeCard {
    output: SliderRow,
    input: SliderRow,
    outputs: DevicePicker,
    inputs: DevicePicker,
    has_input: bool,
    max_volume: f32,
    error: Label,
    has_error: bool,
}

/// Algısal seviye → (kaydırıcı 0..=1, yüzde).
pub(super) fn slider_value(volume: f32, max: f32) -> (f32, u32) {
    let v = volume.max(0.0);
    ((v / max.max(0.01)).clamp(0.0, 1.0), (v * 100.0).round() as u32)
}

/// Kaydırıcı → binde seviye.
pub(super) fn permille(value: f32, max: f32) -> u16 {
    (value.clamp(0.0, 1.0) * max * 1000.0).round().clamp(0.0, f32::from(u16::MAX)) as u16
}

/// Kartın parçalarının yerleri.
#[derive(Debug, Clone, Copy)]
struct Layout {
    output: Rect,
    outputs: Rect,
    input: Rect,
    inputs: Rect,
    /// Son parçanın altı.
    bottom: f32,
}

impl VolumeCard {
    pub fn new(cx: &WidgetCx) -> Self {
        let t = cx.theme;
        Self {
            output: SliderRow::new(t, volume_icon(0.0, false)),
            input: SliderRow::new(t, mic_icon(false)),
            outputs: DevicePicker::new(t),
            inputs: DevicePicker::new(t),
            has_input: false,
            max_volume: 1.0,
            error: Label::new("", text_style(t, TextRole::Caption)),
            has_error: false,
        }
    }

    /// Çıkış satırı, çıkış cihazları, giriş satırı, giriş cihazları (görünmeyen parça boş
    /// dikdörtgen, yer kaplamaz).
    fn layout(&self, rect: Rect, cx: &WidgetCx) -> Layout {
        let t = cx.theme;
        let p = padding(t);
        let gap = t.space(1);
        let (x, w) = (rect.x + p, rect.width - 2.0 * p);
        let h = self.output.height(t);
        let output = Rect::new(x, rect.y + p, w, h);
        let mut y = output.bottom();
        let mut section = |visible: bool, height: f32| {
            if !visible {
                return Rect::new(x, y, w, 0.0);
            }
            let r = Rect::new(x, y + gap, w, height);
            y = r.bottom();
            r
        };
        let outputs = section(self.outputs.visible(), self.outputs.height(t));
        let input = section(self.has_input, h);
        let inputs = section(self.has_input && self.inputs.visible(), self.inputs.height(t));
        Layout { output, outputs, input, inputs, bottom: y }
    }

    fn sync_row(row: &mut SliderRow, dev: Option<&AudioDevice>, max: f32, input: bool, now: Instant) -> bool {
        let (volume, muted) = dev.map_or((0.0, false), |d| (d.volume, d.muted));
        let icon = if input { mic_icon(muted) } else { volume_icon(volume, muted) };
        let (value, pct) = slider_value(volume, max);
        row.set(icon, value, pct, dev.is_some(), now)
    }
}

impl PanelWidget for VolumeCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Volume
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let s = &model.system;
        self.max_volume = s.max_volume;
        let mut changed = Self::sync_row(&mut self.output, s.audio.default_sink(), s.max_volume, false, cx.now);
        let source = s.audio.default_source();
        changed |= set_flag(&mut self.has_input, source.is_some());
        changed |= Self::sync_row(&mut self.input, source, s.max_volume, true, cx.now);
        changed |= self.outputs.sync(&s.audio.sinks, cx.theme);
        changed |= self.inputs.sync(&s.audio.sources, cx.theme);
        let err = s.error_for(ErrorSource::Audio).unwrap_or_default();
        changed |= set_flag(&mut self.has_error, !err.is_empty());
        changed |= self.error.set_text(err);
        changed
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.output.set_theme(cx.theme, cx.now);
        self.input.set_theme(cx.theme, cx.now);
        self.outputs.list.set_theme(cx.theme, cx.now);
        self.inputs.list.set_theme(cx.theme, cx.now);
        self.error.set_style(text_style(cx.theme, TextRole::Caption));
    }

    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        let p = padding(cx.theme);
        let l = self.layout(Rect::new(0.0, 0.0, width, 0.0), cx);
        self.error.set_max_width(Some((width - 2.0 * p).max(0.0)));
        Some(l.bottom + error_height(&mut self.error, text, cx.theme, self.has_error) + p)
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        let t = cx.theme;
        let l = self.layout(rect, cx);
        self.output.draw(ui, l.output);
        if self.outputs.visible() {
            self.outputs.place(t, l.outputs, cx.now);
            self.outputs.list.draw(ui);
        }
        if self.has_input {
            self.input.draw(ui, l.input);
            if self.inputs.visible() {
                self.inputs.place(t, l.inputs, cx.now);
                self.inputs.list.draw(ui);
            }
        }
        if self.has_error {
            let x = l.output.x;
            draw_error(ui, &mut self.error, Point::new(x, l.bottom + t.space(1)), t);
        }
    }

    fn pointer(&mut self, event: &PointerEvent, rect: Rect, _text: &mut TextSystem, cx: &WidgetCx) -> (bool, Option<Action>) {
        let t = cx.theme;
        let l = self.layout(rect, cx);
        let (mut repaint, ev) = self.output.pointer(event, l.output, t, cx.now);
        let mut action = ev.map(|e| match e {
            RowEvent::Icon => SystemCommand::ToggleMute(AudioTarget::DefaultSink),
            RowEvent::Value(v) => {
                SystemCommand::SetVolume { target: AudioTarget::DefaultSink, permille: permille(v, self.max_volume) }
            }
        });
        if self.outputs.visible() {
            self.outputs.place(t, l.outputs, cx.now);
            let (r, cmd) = self.outputs.pointer(event, t, cx.now);
            repaint |= r;
            action = action.or(cmd);
        }
        if self.has_input {
            let (r, ev) = self.input.pointer(event, l.input, t, cx.now);
            repaint |= r;
            action = action.or(ev.map(|e| match e {
                RowEvent::Icon => SystemCommand::ToggleMute(AudioTarget::DefaultSource),
                RowEvent::Value(v) => {
                    SystemCommand::SetVolume { target: AudioTarget::DefaultSource, permille: permille(v, self.max_volume) }
                }
            }));
            if self.inputs.visible() {
                self.inputs.place(t, l.inputs, cx.now);
                let (r, cmd) = self.inputs.pointer(event, t, cx.now);
                repaint |= r;
                action = action.or(cmd);
            }
        }
        (repaint || action.is_some(), action.map(Action::System))
    }

    /// Çıkış kaydırıcısı (+ sessiz düğmesi), çıkış cihazları listesi, giriş kaydırıcısı ve giriş
    /// cihazları. Cihaz satırının anahtarı PipeWire düğüm kimliği.
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            self.output.access(tree, "output", &t!("shell-a11y-volume-output"), &t!("shell-keybinds-volume-mute-label"));
            if self.outputs.visible() {
                let specs = &self.outputs.specs;
                self.outputs.list.access(tree, "outputs", |i| specs.get(i).map(|s| s.id)).label(&t!("shell-a11y-volume-outputs"));
            }
            if self.has_input {
                self.input.access(tree, "input", &t!("shell-a11y-volume-input"), &t!("shell-keybinds-mic-mute-label"));
                if self.inputs.visible() {
                    let specs = &self.inputs.specs;
                    self.inputs.list.access(tree, "inputs", |i| specs.get(i).map(|s| s.id)).label(&t!("shell-a11y-volume-inputs"));
                }
            }
        });
    }

    fn access_action(
        &mut self,
        request: &nuxut::access::AccessRequest,
        parent: nuxut::access::NodeId,
        _rect: Rect,
        cx: &WidgetCx,
    ) -> Option<(bool, Option<Action>)> {
        let card = crate::widgets::access_card_id(parent, self.kind());
        let (target, ev) = match self.output.access_action(card, "output", request, cx.now) {
            Some(ev) => (AudioTarget::DefaultSink, ev),
            None if self.has_input => (AudioTarget::DefaultSource, self.input.access_action(card, "input", request, cx.now)?),
            None => return None,
        };
        let RowEvent::Value(v) = ev else { return None };
        Some((true, Some(Action::System(SystemCommand::SetVolume { target, permille: permille(v, self.max_volume) }))))
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        self.output
            .cursor(p)
            .or_else(|| if self.outputs.visible() { self.outputs.list.cursor(p) } else { None })
            .or_else(|| if self.has_input { self.input.cursor(p) } else { None })
            .or_else(|| if self.has_input && self.inputs.visible() { self.inputs.list.cursor(p) } else { None })
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        // Kaydırıcı/düğme/liste kendi animasyonları sürerken `ui.request_frame()` der.
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::WidgetSettings;
    use nuxut::{Button, PointerKind};

    #[test]
    fn slider_maps_boost_range() {
        assert_eq!(slider_value(0.5, 1.0), (0.5, 50));
        assert_eq!(slider_value(1.5, 1.5), (1.0, 150));
        assert_eq!(slider_value(2.0, 1.0), (1.0, 200));
        assert_eq!(permille(1.0, 1.5), 1500);
        assert_eq!(permille(0.5, 1.0), 500);
        assert_eq!(permille(-1.0, 1.0), 0);
    }

    fn device(id: u32, name: &str, class: DeviceClass, is_default: bool) -> AudioDevice {
        AudioDevice { id, name: name.into(), description: name.into(), volume: 0.5, muted: false, is_default, class }
    }

    #[test]
    fn picker_only_with_a_choice_and_marks_active() {
        assert!(device_specs(&[device(1, "Hoparlör", DeviceClass::Speaker, true)]).is_empty(), "tek cihaz: liste yok");
        let specs = device_specs(&[
            device(1, "Hoparlör", DeviceClass::Speaker, false),
            device(2, "HDMI", DeviceClass::Display, true),
            device(3, "Kulaklık", DeviceClass::Headset, false),
        ]);
        assert_eq!(specs.iter().map(|s| (s.icon, s.active)).collect::<Vec<_>>(), [
            (IconName::Speaker, false),
            (IconName::Monitor, true),
            (IconName::Headphones, false)
        ]);
        assert_eq!(device_icon(DeviceClass::Bluetooth), IconName::Bluetooth);
        assert_eq!(device_icon(DeviceClass::Webcam), IconName::Videocam);
    }

    #[test]
    fn click_on_inactive_device_sets_default() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let settings = WidgetSettings::default();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now: Instant::now(), bounds: Rect::new(0.0, 0.0, 360.0, 800.0) };
        let mut card = VolumeCard::new(&cx);
        let mut model = ShellModel::default();
        model.system.audio.available = true;
        model.system.audio.sinks =
            vec![device(1, "Hoparlör", DeviceClass::Speaker, true), device(2, "Kulaklık", DeviceClass::Headphones, false)];
        model.system.audio.sources = vec![device(5, "Mikrofon", DeviceClass::Microphone, true)];
        assert!(card.sync(&model, &cx));
        assert!(card.outputs.visible() && !card.inputs.visible(), "tek giriş: giriş listesi yok");
        let mut text = TextSystem::new();
        let rect = Rect::new(0.0, 0.0, 360.0, card.height(&mut text, 360.0, &cx).unwrap());
        let l = card.layout(rect, &cx);
        assert!(l.outputs.height > 0.0 && l.outputs.y >= l.output.bottom() && l.input.y >= l.outputs.bottom());
        assert_eq!(l.inputs.height, 0.0);
        // Satır dikdörtgenleri listenin ilk işaretçi olayında yerleşir.
        let mut text = TextSystem::new();
        let _ = card.pointer(&PointerEvent::new(Point::new(l.outputs.x + 1.0, l.outputs.y + 1.0), PointerKind::Motion), rect, &mut text, &cx);
        let row = |i: usize| card.outputs.list.items()[i].rect().center();
        let (active, other) = (row(0), row(1));
        let click = |card: &mut VolumeCard, p: Point| {
            let mut text = TextSystem::new();
            let mut got = None;
            for kind in [PointerKind::Enter, PointerKind::Motion, PointerKind::Press(Button::Left), PointerKind::Release(Button::Left)] {
                got = got.or(card.pointer(&PointerEvent::new(p, kind), rect, &mut text, &cx).1);
            }
            got
        };
        assert_eq!(click(&mut card, other), Some(Action::System(SystemCommand::SetDefaultAudio(2))));
        assert_eq!(click(&mut card, active), None, "etkin cihaza tık bir şey yapmaz");
        // Varsayılan değişince işaret yer değiştirir.
        model.system.audio.sinks[0].is_default = false;
        model.system.audio.sinks[1].is_default = true;
        let placed = card.outputs.height(&theme);
        assert!(card.sync(&model, &cx));
        assert!(card.outputs.list.items()[1].is_selected() && !card.outputs.list.items()[0].is_selected());
        // Yeni öğeler yerleşimden önce de sık yoğunlukta ölçülür (kart bir kare uzamaz).
        assert_eq!(card.outputs.height(&theme), placed);
        assert!(!card.sync(&model, &cx), "değişmeyen durum çizim istemez");
    }

    /// Ekran okuyucu: çıkış kaydırıcısı adı ve "%50" değeriyle, sessiz düğmesi, cihaz listesi;
    /// AT'nin Increment'i işaretçiyle aynı komutu verir (bir adım yukarı).
    #[test]
    fn access_tree_and_slider_step() {
        use nuxut::access::{AccessAction, AccessRequest, AccessTree, Role, child_id};
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let settings = WidgetSettings::default();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now: Instant::now(), bounds: Rect::new(0.0, 0.0, 360.0, 800.0) };
        let mut card = VolumeCard::new(&cx);
        let mut model = ShellModel::default();
        model.system.audio.available = true;
        model.system.audio.sinks =
            vec![device(1, "Hoparlör", DeviceClass::Speaker, true), device(2, "Kulaklık", DeviceClass::Headphones, false)];
        card.sync(&model, &cx);
        let mut text = TextSystem::new();
        let rect = Rect::new(0.0, 0.0, 360.0, card.height(&mut text, 360.0, &cx).unwrap());
        let _ = card.pointer(&PointerEvent::new(Point::new(-5.0, -5.0), PointerKind::Motion), rect, &mut text, &cx);
        let mut tree = AccessTree::new();
        card.access(&mut tree, rect, &cx);
        let slider = tree.nodes().find(|(_, n)| n.role() == Role::Slider).map(|(_, n)| n).expect("kaydırıcı");
        assert_eq!(slider.label(), Some("Ses düzeyi"));
        assert_eq!(slider.value(), Some("%50"));
        assert!(tree.nodes().any(|(_, n)| n.label() == Some("Sesi kapat")));
        assert!(tree.nodes().any(|(_, n)| n.label() == Some("Kulaklık")), "cihaz listesi");
        assert!(tree.unlabeled().is_empty());
        let card_id = crate::widgets::access_card_id(nuxut::access::ROOT_ID, WidgetKind::Volume);
        let up = AccessRequest { node: child_id(card_id, "output"), action: AccessAction::Increment };
        let got = card.access_action(&up, nuxut::access::ROOT_ID, rect, &cx);
        assert!(
            matches!(got, Some((_, Some(Action::System(SystemCommand::SetVolume { target: AudioTarget::DefaultSink, permille })))) if permille > 500),
            "{got:?}"
        );
        let other = AccessRequest { node: child_id(card_id, "nothing"), action: AccessAction::Increment };
        assert!(card.access_action(&other, nuxut::access::ROOT_ID, rect, &cx).is_none());
    }
}
