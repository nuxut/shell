//! Durum göstergeleri bar widget'ı (`status`, t062): ağ, Bluetooth, ses ve pil için kompakt
//! ikon grubu (isteğe bağlı yüzde değeriyle). Üzerine gelince göstergenin kendi ipucu, tıklayınca
//! hızlı ayarlar paneli (`[bar.widget.status] panel`, varsayılan sağ çekmece), ses göstergesi
//! üzerinde tekerlek sesi değiştirir (bar uç eylemiyle aynı yol: `[bar.edge_actions] scroll_step`
//! adımı, OSD).
//!
//! Veri `ShellModel::system`'dan (mevcut ses/ağ/Bluetooth/güç servisleri); widget servis
//! tutamağı tutmaz. Boşta iş yok: `sync` görünen değeri (ikon, değer metni, ton) yeniden
//! hesaplar ve yalnızca o değişince `true` döner — ör. Wi-Fi gücü %71→%69 aynı ikon
//! kademesindeyse kare çizilmez; ipucu metni sessizce güncellenir.
//!
//! Görsel dil `sysmon` ile aynı: `IconSmall` ikon, aynı hap, iç boşluk ve aralık; değer etiketi
//! en geniş metne ("%100") göre yer ayırır (komşular kaymaz); uyarı (pil ≤ %15, şarjda değil)
//! `error` rengiyle; sessiz/bağlantısız göstergeler soluk (`on_surface_variant`).
//! Gösterge yoksa (servis yok, pil yok, Bluetooth kapalı) gizlidir; hiçbiri yoksa widget gizli.

use std::time::Instant;

use nuxut::i18n::t;
use nuxut::render::TextAlign;
use nuxut::ui::{Icon, IconName, IconView, Interaction, Label, TextRole, text_style};
use nuxut::{Color, Corners, CursorShape, Painter, Point, PointerEvent, PointerKind, Rect, RectStyle, TextSystem};

use super::quick::{battery_icon, format_duration, volume_icon, wifi_icon};
use super::{Action, BarTooltip, BarWidget, PanelId, WidgetCx, WidgetKind, chip_fill, draw_hover};
use crate::bar_edge::{EdgeCommand, ScrollAccumulator, ScrollAction};
use crate::model::ShellModel;
use crate::services::network::{ActiveState, ConnectionKind, Connectivity, NmState};
use crate::services::power::{ChargeState, WarningLevel};
use crate::system::SystemModel;
use crate::widgets::registry::StatusSettings;

/// Pil uyarı eşiği (%, şarjda değilken; ayrıca UPower `Critical`/`Action` düzeyi).
const BATTERY_ALERT_PERCENT: u32 = 15;
/// İpucunda bir adın en fazla grafemi (adlar servislerde zaten sınırlı; ipucu dar kalsın).
const MAX_TIP_NAME: usize = 40;
/// İpucunda listelenen en fazla bağlı Bluetooth cihazı.
const MAX_TIP_DEVICES: usize = 3;

/// Göstergeler; dizi sırası bar'daki sırasıdır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Network,
    Bluetooth,
    Volume,
    Battery,
}

impl Slot {
    const ALL: [Slot; 4] = [Slot::Network, Slot::Bluetooth, Slot::Volume, Slot::Battery];

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Normal,
    /// Sessiz, bağlantı yok, internet yok: soluk.
    Dim,
    /// Düşük pil.
    Alert,
}

/// Bir göstergenin görünen hâli (karşılaştırma bununla; ipucu ayrı tutulur).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Look {
    icon: IconName,
    value: Option<String>,
    tone: Tone,
}

/// Göstergenin hâli ve ipucu (saf; testler doğrudan bunu sınar).
#[derive(Debug, Clone, PartialEq, Eq)]
struct View {
    look: Look,
    tooltip: String,
}

/// Ad kırpma: kontrol karakterleri atılır, uzunsa "…".
fn short(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() > MAX_TIP_NAME {
        let mut s: String = clean.chars().take(MAX_TIP_NAME - 1).collect();
        s.push('…');
        s
    } else {
        clean
    }
}

/// Ayarlara göre bir göstergenin görünümü; `None` = gizli.
fn view(slot: Slot, sys: &SystemModel, s: &StatusSettings) -> Option<View> {
    match slot {
        Slot::Network if s.network => network_view(sys),
        Slot::Bluetooth if s.bluetooth => bluetooth_view(sys),
        Slot::Volume if s.volume => volume_view(sys, s.volume_percent),
        Slot::Battery if s.battery => battery_view(sys, s.battery_percent),
        _ => None,
    }
}

fn network_view(sys: &SystemModel) -> Option<View> {
    let n = &sys.network;
    if !n.available {
        return None;
    }
    let has_wifi = !n.wifi_devices.is_empty();
    let ap = n.active_access_point();
    let ethernet_up = n.active.iter().any(|a| a.kind == ConnectionKind::Ethernet && a.state == ActiveState::Activated);
    let mut lines = Vec::with_capacity(3);
    let (icon, mut tone) = if n.state.is_connected() {
        // Birincil bağlantı VPN/diğer olabilir: alttaki bağı göster (Wi-Fi önce, sonra kablo).
        let wifi_primary = n.primary_kind == Some(ConnectionKind::Wifi) || (n.primary_kind != Some(ConnectionKind::Ethernet) && ap.is_some());
        if wifi_primary && let Some(ap) = ap {
            lines.push(t!("shell-bar-status-wifi", ssid = short(&ap.ssid), strength = crate::percent::ratio(ap.strength.min(100))));
            (wifi_icon(ap.strength), Tone::Normal)
        } else if wifi_primary {
            lines.push(t!("shell-bar-status-wifi-connected"));
            (IconName::Wifi, Tone::Normal)
        } else {
            let name = n.primary_name.as_deref().map(short);
            lines.push(match (ethernet_up || n.primary_kind == Some(ConnectionKind::Ethernet), name) {
                (true, Some(name)) => t!("shell-bar-status-ethernet-named", name = name),
                (true, None) => t!("shell-bar-status-ethernet"),
                (false, Some(name)) => t!("shell-bar-status-connected-named", name = name),
                (false, None) => t!("shell-bar-status-connected"),
            });
            (IconName::Ethernet, Tone::Normal)
        }
    } else if n.state == NmState::Connecting {
        lines.push(t!("shell-bar-status-connecting"));
        (if has_wifi { IconName::Wifi } else { IconName::Ethernet }, Tone::Dim)
    } else if has_wifi && !(n.wifi_enabled && n.wifi_hardware_enabled) {
        lines.push(t!("shell-bar-status-wifi-off"));
        (IconName::WifiOff, Tone::Dim)
    } else {
        lines.push(t!("shell-bar-status-offline"));
        (if has_wifi { IconName::WifiOff } else { IconName::Ethernet }, Tone::Dim)
    };
    if n.state.is_connected() {
        match n.connectivity {
            Connectivity::None | Connectivity::Limited => {
                lines.push(t!("shell-bar-status-no-internet"));
                tone = Tone::Dim;
            }
            Connectivity::Portal => {
                lines.push(t!("shell-bar-status-portal"));
                tone = Tone::Dim;
            }
            Connectivity::Full | Connectivity::Unknown => {}
        }
    }
    for vpn in n.active.iter().filter(|a| a.kind.is_vpn() && a.state == ActiveState::Activated) {
        lines.push(t!("shell-bar-status-vpn", name = short(&vpn.id)));
    }
    Some(View { look: Look { icon, value: None, tone }, tooltip: lines.join("\n") })
}

fn bluetooth_view(sys: &SystemModel) -> Option<View> {
    let b = &sys.bluetooth;
    // Kapalıyken (ya da adaptör yokken) gizli: bar'da yalnızca açık/bağlı durumu.
    if !b.available || !b.powered() {
        return None;
    }
    let connected: Vec<_> = b.devices.iter().filter(|d| d.connected).collect();
    if connected.is_empty() {
        return Some(View {
            look: Look { icon: IconName::Bluetooth, value: None, tone: Tone::Normal },
            tooltip: t!("shell-bar-status-bluetooth-on"),
        });
    }
    let mut lines = vec![t!("shell-bar-status-bluetooth-connected", count = connected.len())];
    for d in connected.iter().take(MAX_TIP_DEVICES) {
        let name = short(if d.name.is_empty() { d.address.as_str() } else { d.name.as_str() });
        lines.push(match d.battery {
            Some(p) => t!("shell-bar-status-bluetooth-device-battery", name = name, percent = crate::percent::ratio(p.min(100))),
            None => name,
        });
    }
    if connected.len() > MAX_TIP_DEVICES {
        lines.push(t!("shell-bar-status-bluetooth-more", count = connected.len() - MAX_TIP_DEVICES));
    }
    Some(View { look: Look { icon: IconName::BluetoothConnected, value: None, tone: Tone::Normal }, tooltip: lines.join("\n") })
}

fn volume_view(sys: &SystemModel, show_value: bool) -> Option<View> {
    let a = &sys.audio;
    let sink = a.default_sink().filter(|_| a.available)?;
    let level = if sink.volume.is_finite() { (sink.volume.max(0.0) * 100.0).round().min(999.0) as u32 } else { 0 };
    let state = if sink.muted {
        t!("shell-bar-status-volume-muted", percent = crate::percent::ratio(level))
    } else {
        t!("shell-bar-status-volume", percent = crate::percent::ratio(level))
    };
    Some(View {
        look: Look {
            icon: volume_icon(sink.volume, sink.muted),
            value: show_value.then(|| t!("shell-bar-status-percent", percent = crate::percent::ratio(level))),
            tone: if sink.muted { Tone::Dim } else { Tone::Normal },
        },
        tooltip: format!("{state}\n{}", short(&sink.description)),
    })
}

fn battery_view(sys: &SystemModel, show_value: bool) -> Option<View> {
    let b = sys.power.battery()?;
    let level = b.percentage.round().clamp(0.0, 100.0) as u32;
    let charging = b.state.is_charging();
    let alert = !charging && (level <= BATTERY_ALERT_PERCENT || b.warning >= WarningLevel::Critical);
    let detail = match b.state {
        ChargeState::FullyCharged => Some(t!("shell-bar-status-battery-full")),
        _ if charging => Some(match b.time_to_full {
            Some(d) => t!("shell-bar-status-battery-charging-time", time = format_duration(d)),
            None => t!("shell-bar-status-battery-charging"),
        }),
        ChargeState::Discharging => b.time_to_empty.map(|d| t!("shell-bar-status-battery-remaining", time = format_duration(d))),
        _ => None,
    };
    let mut tooltip = t!("shell-bar-status-battery", percent = crate::percent::ratio(level));
    if let Some(d) = detail {
        tooltip.push_str(" · ");
        tooltip.push_str(&d);
    }
    Some(View {
        look: Look {
            icon: battery_icon(&b),
            value: show_value.then(|| t!("shell-bar-status-percent", percent = crate::percent::ratio(level))),
            tone: if alert { Tone::Alert } else { Tone::Normal },
        },
        tooltip,
    })
}

#[derive(Debug)]
struct Indicator {
    icon: IconView,
    label: Label,
    template: Label,
    look: Option<Look>,
    tooltip: String,
    /// Son ölçümdeki ikon ve değer boyutları (ana eksen: yatay genişlik, dikey yükseklik).
    icon_size: f32,
    value_size: (f32, f32),
}

impl Indicator {
    fn new(cx: &WidgetCx) -> Self {
        let style = value_style(cx);
        Self {
            icon: IconView::with_role(cx.theme, Icon::named(IconName::Wifi), TextRole::IconSmall),
            label: Label::new("", style.clone()),
            template: Label::new(&t!("shell-bar-status-percent", percent = 1.0), style),
            look: None,
            tooltip: String::new(),
            icon_size: 0.0,
            value_size: (0.0, 0.0),
        }
    }

    fn has_value(&self) -> bool {
        self.look.as_ref().is_some_and(|l| l.value.is_some())
    }

    /// Ana eksen boyunca uzunluk.
    fn length(&self, cx: &WidgetCx) -> f32 {
        if !self.has_value() {
            return self.icon_size;
        }
        let value = if cx.vertical { self.value_size.1 } else { self.value_size.0 };
        self.icon_size + cx.theme.space(1) + value
    }

    /// Görünümü uygular; görünen bir şey değiştiyse `true`.
    fn apply(&mut self, view: Option<View>) -> bool {
        let (look, tooltip) = match view {
            Some(v) => (Some(v.look), v.tooltip),
            None => (None, String::new()),
        };
        self.tooltip = tooltip;
        if self.look == look {
            return false;
        }
        if let Some(l) = &look {
            self.icon.set_icon(Icon::named(l.icon));
            self.label.set_text(l.value.as_deref().unwrap_or(""));
        }
        self.look = look;
        true
    }
}

fn value_style(cx: &WidgetCx) -> nuxut::TextStyle {
    // sysmon ile aynı: dikey bar'da bir kademe küçük ve ortalı.
    let role = if cx.vertical { TextRole::Caption } else { TextRole::Body };
    let mut s = text_style(cx.theme, role).with_tabular_numbers();
    if cx.vertical {
        s.align = TextAlign::Center;
    }
    s
}

#[derive(Debug)]
pub struct Status {
    indicators: [Indicator; 4],
    hover: Interaction,
    settings: StatusSettings,
    /// Tekerlek çentik birikimi (ses göstergesi üzerinde).
    scroll: ScrollAccumulator,
    /// Son çizimdeki widget dikdörtgeni (kaydırma isabeti; olay bar sırasından önce gelir).
    last_rect: Option<Rect>,
    /// Son modelden alınan sistem durumu (ayar değişince yeniden hesaplamak için).
    system: Option<SystemModel>,
}

impl Status {
    pub fn new(cx: &WidgetCx) -> Self {
        Self {
            indicators: std::array::from_fn(|_| Indicator::new(cx)),
            hover: Interaction::new(cx.theme),
            settings: cx.settings.status.clone(),
            scroll: ScrollAccumulator::default(),
            last_rect: None,
            system: None,
        }
    }

    fn padding(cx: &WidgetCx) -> f32 {
        if cx.vertical { cx.theme.space(2) } else { cx.theme.space(3) }
    }

    fn gap(cx: &WidgetCx) -> f32 {
        if cx.vertical { cx.theme.space(2) } else { cx.theme.space(3) }
    }

    /// Görünen bir şey değiştiyse `true`.
    fn refresh(&mut self) -> bool {
        let Some(sys) = &self.system else { return false };
        let mut changed = false;
        for slot in Slot::ALL {
            changed |= self.indicators[slot.index()].apply(view(slot, sys, &self.settings));
        }
        changed
    }

    fn visible(&self) -> impl Iterator<Item = (Slot, &Indicator)> {
        Slot::ALL.into_iter().map(|s| (s, &self.indicators[s.index()])).filter(|(_, i)| i.look.is_some())
    }

    /// Görünen göstergelerin isabet dikdörtgenleri (aralığın yarısı iki yana; uçlar hap
    /// kenarına uzar: göstergeler arasında ipucu kesilmez).
    fn hit_rects(&self, rect: Rect, cx: &WidgetCx) -> Vec<(Slot, Rect)> {
        let chip = chip_rect(rect, cx);
        let pad = Self::padding(cx);
        let gap = Self::gap(cx);
        let mut pos = if cx.vertical { chip.y + pad } else { chip.x + pad };
        let shown: Vec<_> = self.visible().map(|(s, i)| (s, i.length(cx))).collect();
        let last = shown.len().saturating_sub(1);
        shown
            .into_iter()
            .enumerate()
            .map(|(n, (slot, len))| {
                let start = if n == 0 { pos - pad } else { pos - gap * 0.5 };
                let end = if n == last { pos + len + pad } else { pos + len + gap * 0.5 };
                pos += len + gap;
                let r = if cx.vertical {
                    Rect::new(chip.x, start, chip.width, end - start)
                } else {
                    Rect::new(start, chip.y, end - start, chip.height)
                };
                (slot, r)
            })
            .collect()
    }

    fn slot_at(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<(Slot, Rect)> {
        self.hit_rects(rect, cx).into_iter().find(|(_, r)| r.contains(p))
    }

    fn target(&self) -> PanelId {
        self.settings.panel.into()
    }
}

impl BarWidget for Status {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Status
    }

    fn sync(&mut self, model: &ShellModel, _monitor: &str, _cx: &WidgetCx) -> bool {
        // Model her olayda (Hyprland, bildirim…) gelir: yalnızca sistem durumu değiştiyse hesapla.
        let sys = &model.system;
        if let Some(old) = &self.system
            && old.audio == sys.audio
            && old.network == sys.network
            && old.bluetooth == sys.bluetooth
            && old.power == sys.power
        {
            return false;
        }
        let mut copy = self.system.take().unwrap_or_default();
        copy.audio.clone_from(&sys.audio);
        copy.network.clone_from(&sys.network);
        copy.bluetooth.clone_from(&sys.bluetooth);
        copy.power.clone_from(&sys.power);
        self.system = Some(copy);
        self.refresh()
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.hover.set_theme(cx.theme, cx.now);
        let style = value_style(cx);
        for i in &mut self.indicators {
            i.icon.set_theme(cx.theme, cx.now);
            i.label.set_style(style.clone());
            i.template.set_style(style.clone());
        }
        if self.settings != cx.settings.status {
            self.settings = cx.settings.status.clone();
            self.refresh();
        }
    }

    fn measure(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        let mut total = 0.0;
        let mut n = 0usize;
        for i in self.indicators.iter_mut().filter(|i| i.look.is_some()) {
            let s = i.icon.measure(text);
            i.icon_size = if cx.vertical { s.height } else { s.width };
            if i.has_value() {
                let t = i.template.measure(text);
                let v = i.label.measure(text);
                i.value_size = (t.width.max(v.width), t.height.max(v.height));
            }
            total += i.length(cx);
            n += 1;
        }
        if n == 0 {
            return 0.0;
        }
        total + Self::gap(cx) * (n - 1) as f32 + Self::padding(cx) * 2.0
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, highlighted: bool) {
        self.last_rect = Some(rect);
        let colors = &cx.theme.colors;
        let chip = chip_rect(rect, cx);
        let radii = Corners::uniform(if cx.vertical { cx.theme.shape.radius_small } else { chip.height * 0.5 });
        if highlighted {
            painter.rect(chip, &RectStyle::filled(colors.surface_bright, radii));
        }
        let color = |tone: Tone| -> Color {
            match tone {
                Tone::Alert => colors.error,
                _ if highlighted => colors.primary,
                Tone::Normal => colors.on_surface,
                Tone::Dim => colors.on_surface_variant,
            }
        };
        let contents: Vec<Color> = self.indicators.iter().filter_map(|i| i.look.as_ref()).map(|l| color(l.tone)).collect();
        draw_hover(painter, &self.hover, chip, radii, cx, chip_fill(cx.theme, highlighted), &contents);
        let pad = Self::padding(cx);
        let gap = Self::gap(cx);
        let inner = cx.theme.space(1);
        let mut pos = if cx.vertical { chip.y + pad } else { chip.x + pad };
        for i in self.indicators.iter_mut() {
            let Some(look) = &i.look else { continue };
            let c = color(look.tone);
            let has_value = look.value.is_some();
            if cx.vertical {
                let x = chip.center().x;
                i.icon.draw_centered(painter, Point::new(x, pos + i.icon_size * 0.5), c);
                pos += i.icon_size;
                if has_value {
                    let size = i.label.measure_in(painter);
                    i.label.draw(painter, Point::new(x - size.width * 0.5, pos + inner), c);
                    pos += inner + i.value_size.1;
                }
            } else {
                let y = chip.center().y;
                i.icon.draw_centered(painter, Point::new(pos + i.icon_size * 0.5, y), c);
                pos += i.icon_size;
                if has_value {
                    let size = i.label.measure_in(painter);
                    i.label.draw(painter, Point::new(pos + inner, y - size.height * 0.5), c);
                    pos += inner + i.value_size.0;
                }
            }
            pos += gap;
        }
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        self.hover.pointer(chip_rect(rect, cx), pointer, down, cx.now)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        chip_rect(rect, cx).contains(at).then(|| Action::TogglePanel(self.target()))
    }

    fn panel(&self, _cx: &WidgetCx) -> Option<PanelId> {
        Some(self.target())
    }

    /// Tek düğme (tık paneli açar); değer görünen göstergelerin ipuçları: "Wi-Fi: Ev, Ses %64, Pil %80".
    fn access_items(&self, rect: Rect, cx: &WidgetCx) -> Vec<super::AccessItem> {
        let shown: Vec<&str> = self
            .hit_rects(rect, cx)
            .into_iter()
            .map(|(slot, _)| self.indicators[slot.index()].tooltip.as_str())
            .filter(|t| !t.is_empty())
            .collect();
        let value = shown.join(", ").lines().collect::<Vec<_>>().join(", ");
        vec![super::AccessItem::new("widget", self.kind().title(), chip_rect(rect, cx)).with_value(value)]
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.hover.is_animating(now)
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        chip_rect(rect, cx).contains(p).then_some(CursorShape::Pointer)
    }

    /// Göstergenin kendi ipucu; balon o göstergenin altında/yanında.
    fn tooltip(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        if !chip_rect(rect, cx).contains(p) {
            return None;
        }
        let (slot, anchor) = self.slot_at(rect, p, cx)?;
        let text = &self.indicators[slot.index()].tooltip;
        (!text.is_empty()).then_some(BarTooltip { text, anchor: Some(anchor) })
    }

    /// Ses göstergesi üzerinde tekerlek: sesi `scroll_step` adımla değiştirir (uç eylemiyle aynı
    /// yol; OSD görünür). Diğer göstergelerde kaydırma bar'a düşer.
    fn overlay_pointer(&mut self, event: &PointerEvent, _text: &mut TextSystem, cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        let rect = self.last_rect?;
        match event.kind {
            PointerKind::Scroll { delta, steps } => {
                let (slot, _) = self.slot_at(rect, event.position, cx)?;
                if slot != Slot::Volume || !chip_rect(rect, cx).contains(event.position) {
                    return None;
                }
                let n = self.scroll.push(delta, steps, cx.now);
                Some((false, (n != 0).then_some(Action::BarEdge(EdgeCommand::Scroll { action: ScrollAction::Volume, steps: n }))))
            }
            PointerKind::ScrollEnd => {
                self.scroll.end();
                None
            }
            _ => None,
        }
    }
}

/// Hap: bar'a dik eksende öğe boyunda (sysmon ve saatle aynı).
fn chip_rect(rect: Rect, cx: &WidgetCx) -> Rect {
    if cx.vertical {
        Rect::new(rect.center().x - cx.item * 0.5, rect.y, cx.item, rect.height)
    } else {
        Rect::new(rect.x, rect.center().y - cx.item * 0.5, rect.width, cx.item)
    }
}

#[cfg(test)]
mod tests;
