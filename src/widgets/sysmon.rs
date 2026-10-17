//! Sistem izleyicisi bar widget'ı: `[bar.widget.sysmon]`'da seçilen ölçümler (varsayılan CPU ve
//! bellek) ikon + değer olarak; üzerine gelince özet ipucu, tıklayınca ayrıntı popout'u
//! ([`PanelId::Sysmon`], `src/sysmon_view.rs`).
//!
//! Değer etiketinin genişliği en geniş olası metne göre ayrılır ("%100"): değer değişince
//! komşu widget'lar kaymaz. Değer yalnızca eşiği aşınca yeniden yazılır (±2 puan / ±2 °C / ağda
//! %25 ve en az 4 KiB/s; uyarı eşiği geçilince hemen): boşta %1↔%2 titreşimi kare çizdirmez.
//! İpucu her ölçümde günceldir. Etiket değişmedikçe `sync` `false` döner (kare yok).
//! Yüksek kullanım (≥ %90, sıcaklık ≥ 85 °C) değeri framework `Threshold`'un rengiyle (varsayılan
//! `error` rolü) gösterir; geri dönüş paylı: eşik çevresinde salınan değer rengi titretmez.

use std::sync::Arc;
use std::time::Instant;

use nuxut::render::TextAlign;
use nuxut::ui::style::THRESHOLD_HYSTERESIS;
use nuxut::ui::{Icon, IconName, IconView, Interaction, Label, TextRole, Threshold, text_style};
use nuxut::{Corners, CursorShape, Painter, Point, Rect, RectStyle, TextSystem};

use super::{Action, BarTooltip, BarWidget, PanelId, WidgetCx, WidgetKind, chip_fill, draw_hover};
use crate::model::ShellModel;
use crate::services::sysmon::{ALERT_CELSIUS, ALERT_PERCENT, Snapshot, format};
use crate::widgets::registry::SysmonSettings;

/// Bar değerinin yeniden yazılma eşikleri: yüzde puanı, °C, ağ hızında göreli pay ve taban (B/s).
const PERCENT_DEADBAND: f32 = 2.0;
const CELSIUS_DEADBAND: f32 = 2.0;
const RATE_RELATIVE: f32 = 0.25;
const RATE_FLOOR: f32 = 4096.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetricKind {
    Cpu,
    Memory,
    Gpu,
    Temperature,
    Network,
    Disk,
}

impl MetricKind {
    fn icon(self) -> Option<IconName> {
        Some(match self {
            MetricKind::Cpu => IconName::Cpu,
            MetricKind::Memory => IconName::Memory,
            MetricKind::Gpu => IconName::Gpu,
            MetricKind::Temperature => IconName::Temperature,
            // Oklar yönü zaten söylüyor.
            MetricKind::Network => return None,
            MetricKind::Disk => IconName::Disk,
        })
    }

    /// Ayrılan genişliğin şablonu (en geniş olası değer).
    fn template(self) -> String {
        match self {
            MetricKind::Temperature => "100°".to_owned(),
            MetricKind::Network => "↓999K ↑999K".to_owned(),
            _ => crate::percent::text(100.0),
        }
    }

    /// Uyarı eşiği (değer biriminde: % ya da °C). Rengi framework varsayılanı (`error` rolü);
    /// geri dönüş payı yüzdede `THRESHOLD_HYSTERESIS` puan karşılığı, sıcaklıkta yazma eşiği kadar.
    fn threshold(self) -> Option<Threshold> {
        match self {
            MetricKind::Network => None,
            MetricKind::Temperature => Some(Threshold::new(ALERT_CELSIUS).with_hysteresis(CELSIUS_DEADBAND)),
            _ => Some(Threshold::new(ALERT_PERCENT).with_hysteresis(THRESHOLD_HYSTERESIS * 100.0)),
        }
    }

    /// Önceki durum `was` iken ham değer `raw` için uyarı durumu.
    fn alert(self, raw: f32, was: bool) -> bool {
        self.threshold().is_some_and(|t| t.crossed(raw, was))
    }

    /// Ölçüm yoksa `None` (gizli). `raw`: eşik karşılaştırması için ham değer(ler).
    fn value(self, s: &Snapshot) -> Option<Value> {
        let pct = |v: f32| Value { text: format::percent(v), raw: [v, 0.0] };
        match self {
            MetricKind::Cpu => Some(s.cpu.map_or_else(|| Value { text: "—".to_owned(), raw: [f32::NAN, 0.0] }, pct)),
            MetricKind::Memory => s.memory_percent().map(pct),
            MetricKind::Gpu => s.gpus.iter().find_map(|g| g.busy).map(pct),
            MetricKind::Temperature => s.cpu_temp.map(|c| Value { text: format::celsius_compact(c), raw: [c, 0.0] }),
            MetricKind::Network => s.net.map(|n| Value {
                text: format!("↓{} ↑{}", format::rate_compact(n.down), format::rate_compact(n.up)),
                raw: [n.down as f32, n.up as f32],
            }),
            MetricKind::Disk => s.root.map(|r| pct(r.percent())),
        }
    }

    /// Gösterilen değerden bu kadar uzaklaştı mı (yoksa etiket ve kare değişmez: boşta %1↔%2
    /// titreşimi bar'ı her ölçümde yeniden çizdirmez).
    fn significant(self, old: [f32; 2], new: [f32; 2]) -> bool {
        if old[0].is_nan() != new[0].is_nan() {
            return true;
        }
        match self {
            MetricKind::Network => (0..2).any(|i| {
                let d = (new[i] - old[i]).abs();
                d >= RATE_FLOOR && d >= old[i].max(new[i]) * RATE_RELATIVE
            }),
            MetricKind::Temperature => (new[0] - old[0]).abs() >= CELSIUS_DEADBAND,
            _ => (new[0] - old[0]).abs() >= PERCENT_DEADBAND,
        }
    }
}

/// Bir ölçümün bar metni.
#[derive(Debug, Clone, PartialEq)]
struct Value {
    text: String,
    raw: [f32; 2],
}

/// Dikey bar dar: çok parçalı değer (ağ: "↓12K ↑9K") satırlara bölünür.
fn stacked(text: &str, vertical: bool) -> String {
    if vertical { text.replace(' ', "\n") } else { text.to_owned() }
}

/// Ayarlardan bar'da gösterilecek ölçümler (sabit sıra).
fn selected(s: &SysmonSettings) -> Vec<MetricKind> {
    [
        (s.cpu, MetricKind::Cpu),
        (s.memory, MetricKind::Memory),
        (s.gpu, MetricKind::Gpu),
        (s.temperature, MetricKind::Temperature),
        (s.network, MetricKind::Network),
        (s.disk, MetricKind::Disk),
    ]
    .into_iter()
    .filter_map(|(on, k)| on.then_some(k))
    .collect()
}

#[derive(Debug)]
struct Metric {
    kind: MetricKind,
    icon: Option<IconView>,
    label: Label,
    template: Label,
    visible: bool,
    alert: bool,
    /// Etikette gösterilen ham değer (eşik karşılaştırması; `None` = henüz yok).
    shown: Option<[f32; 2]>,
    /// Son ölçümdeki (ikon, değer) boyutları.
    icon_size: f32,
    value_size: (f32, f32),
}

impl Metric {
    fn new(kind: MetricKind, cx: &WidgetCx) -> Self {
        let style = value_style(cx);
        Self {
            kind,
            icon: kind.icon().map(|i| IconView::with_role(cx.theme, Icon::named(i), TextRole::IconSmall)),
            label: Label::new("", style.clone()),
            template: Label::new(stacked(&kind.template(), cx.vertical), style),
            visible: false,
            alert: false,
            shown: None,
            icon_size: 0.0,
            value_size: (0.0, 0.0),
        }
    }

    /// Ana eksen boyunca uzunluk (yatay: genişlik, dikey: yükseklik).
    fn length(&self, cx: &WidgetCx) -> f32 {
        let gap = cx.theme.space(1);
        if cx.vertical {
            self.icon_size + if self.icon.is_some() { gap } else { 0.0 } + self.value_size.1
        } else {
            self.icon_size + if self.icon.is_some() { gap } else { 0.0 } + self.value_size.0
        }
    }
}

fn value_style(cx: &WidgetCx) -> nuxut::TextStyle {
    // Dikey bar dar: değerler bir kademe küçük ve ortalı.
    let role = if cx.vertical { TextRole::Caption } else { TextRole::Body };
    let mut s = text_style(cx.theme, role).with_tabular_numbers();
    if cx.vertical {
        s.align = TextAlign::Center;
    }
    s
}

#[derive(Debug)]
pub struct Sysmon {
    metrics: Vec<Metric>,
    /// Hiçbir ölçüm seçili değilken gösterilen ikon.
    idle_icon: IconView,
    hover: Interaction,
    settings: SysmonSettings,
    vertical: bool,
    snapshot: Option<Arc<Snapshot>>,
    tooltip: String,
}

impl Sysmon {
    pub fn new(cx: &WidgetCx) -> Self {
        let settings = cx.settings.sysmon.clone();
        Self {
            metrics: selected(&settings).into_iter().map(|k| Metric::new(k, cx)).collect(),
            idle_icon: IconView::with_role(cx.theme, Icon::named(IconName::Performance), TextRole::IconSmall),
            hover: Interaction::new(cx.theme),
            settings,
            vertical: cx.vertical,
            snapshot: None,
            tooltip: String::new(),
        }
    }

    fn padding(cx: &WidgetCx) -> f32 {
        if cx.vertical { cx.theme.space(2) } else { cx.theme.space(3) }
    }

    fn gap(cx: &WidgetCx) -> f32 {
        if cx.vertical { cx.theme.space(2) } else { cx.theme.space(3) }
    }

    /// Ölçümden etiketleri günceller. Görünen bir şey değiştiyse `true`.
    fn apply(&mut self) -> bool {
        let Some(s) = self.snapshot.clone() else { return false };
        let vertical = self.vertical;
        let mut changed = false;
        for m in &mut self.metrics {
            let v = m.kind.value(&s);
            let visible = v.is_some();
            changed |= std::mem::replace(&mut m.visible, visible) != visible;
            match v {
                Some(v) => {
                    let alert = m.kind.alert(v.raw[0], m.alert);
                    let due = alert != m.alert || m.shown.is_none_or(|old| m.kind.significant(old, v.raw));
                    if due {
                        m.shown = Some(v.raw);
                        changed |= m.label.set_text(&stacked(&v.text, vertical));
                        changed |= std::mem::replace(&mut m.alert, alert) != alert;
                    }
                }
                None => m.shown = None,
            }
        }
        self.tooltip = format::tooltip(&s);
        changed
    }

    fn any_visible(&self) -> bool {
        self.metrics.iter().any(|m| m.visible)
    }
}

impl BarWidget for Sysmon {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Sysmon
    }

    fn sync(&mut self, model: &ShellModel, _monitor: &str, _cx: &WidgetCx) -> bool {
        match (&model.sysmon, &self.snapshot) {
            (Some(new), Some(old)) if Arc::ptr_eq(new, old) => return false,
            (None, None) => return false,
            _ => {}
        }
        self.snapshot = model.sysmon.clone();
        self.apply()
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.hover.set_theme(cx.theme, cx.now);
        self.idle_icon.set_theme(cx.theme, cx.now);
        if self.settings != cx.settings.sysmon || self.vertical != cx.vertical {
            self.settings = cx.settings.sysmon.clone();
            self.vertical = cx.vertical;
            self.metrics = selected(&self.settings).into_iter().map(|k| Metric::new(k, cx)).collect();
            self.apply();
            return;
        }
        let style = value_style(cx);
        for m in &mut self.metrics {
            if let Some(i) = &mut m.icon {
                i.set_theme(cx.theme, cx.now);
            }
            m.label.set_style(style.clone());
            m.template.set_style(style.clone());
        }
    }

    fn measure(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        let pad = Self::padding(cx);
        if !self.any_visible() {
            let s = self.idle_icon.measure(text);
            return pad * 2.0 + if cx.vertical { s.height } else { s.width };
        }
        let mut total = 0.0;
        let mut n = 0;
        for m in self.metrics.iter_mut().filter(|m| m.visible) {
            m.icon_size = m.icon.as_mut().map_or(0.0, |i| if cx.vertical { i.measure(text).height } else { i.measure(text).width });
            let t = m.template.measure(text);
            let v = m.label.measure(text);
            m.value_size = (t.width.max(v.width), t.height.max(v.height));
            total += m.length(cx);
            n += 1;
        }
        total + Self::gap(cx) * (n.max(1) - 1) as f32 + pad * 2.0
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, highlighted: bool) {
        let colors = &cx.theme.colors;
        let chip = chip_rect(rect, cx);
        let radii = Corners::uniform(if cx.vertical { cx.theme.shape.radius_small } else { chip.height * 0.5 });
        if highlighted {
            painter.rect(chip, &RectStyle::filled(colors.surface_bright, radii));
        }
        let text_color = if highlighted { colors.primary } else { colors.on_surface };
        let icon_color = if highlighted { colors.primary } else { colors.on_surface_variant };
        let mut contents = vec![text_color, icon_color];
        contents.extend(
            self.metrics
                .iter()
                .filter(|m| m.visible && m.alert)
                .filter_map(|m| m.kind.threshold())
                .map(|t| t.tint.resolve(cx.theme)),
        );
        draw_hover(painter, &self.hover, chip, radii, cx, chip_fill(cx.theme, highlighted), &contents);
        if !self.any_visible() {
            self.idle_icon.draw_centered(painter, chip.center(), text_color);
            return;
        }
        let pad = Self::padding(cx);
        let gap = Self::gap(cx);
        let inner = cx.theme.space(1);
        let mut pos = if cx.vertical { chip.y + pad } else { chip.x + pad };
        for m in self.metrics.iter_mut().filter(|m| m.visible) {
            let color = match m.kind.threshold() {
                Some(t) if m.alert => t.tint.resolve(cx.theme),
                _ => text_color,
            };
            let size = m.label.measure_in(painter);
            if cx.vertical {
                let cx_ = chip.center().x;
                if let Some(i) = &mut m.icon {
                    i.draw_centered(painter, Point::new(cx_, pos + m.icon_size * 0.5), icon_color);
                    pos += m.icon_size + inner;
                }
                m.label.draw(painter, Point::new(cx_ - size.width * 0.5, pos), color);
                pos += m.value_size.1 + gap;
            } else {
                let cy = chip.center().y;
                if let Some(i) = &mut m.icon {
                    i.draw_centered(painter, Point::new(pos + m.icon_size * 0.5, cy), icon_color);
                    pos += m.icon_size + inner;
                }
                m.label.draw(painter, Point::new(pos, cy - size.height * 0.5), color);
                pos += m.value_size.0 + gap;
            }
        }
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        self.hover.pointer(chip_rect(rect, cx), pointer, down, cx.now)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        chip_rect(rect, cx).contains(at).then_some(Action::TogglePanel(PanelId::Sysmon))
    }

    fn panel(&self, _cx: &WidgetCx) -> Option<PanelId> {
        Some(PanelId::Sysmon)
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.hover.is_animating(now)
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        chip_rect(rect, cx).contains(p).then_some(CursorShape::Pointer)
    }

    fn tooltip(&self, _rect: Rect, _p: Point, _cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        (!self.tooltip.is_empty()).then_some(BarTooltip { text: &self.tooltip, anchor: None })
    }
}

/// Hap: bar'a dik eksende öğe boyunda (saatle aynı).
fn chip_rect(rect: Rect, cx: &WidgetCx) -> Rect {
    if cx.vertical {
        Rect::new(rect.center().x - cx.item * 0.5, rect.y, cx.item, rect.height)
    } else {
        Rect::new(rect.x, rect.center().y - cx.item * 0.5, rect.width, cx.item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::sysmon::{Gpu, Memory, Rates};

    fn cx_parts() -> (nuxut::Theme, crate::widgets::WidgetSettings) {
        (nuxut::Theme::default(), crate::widgets::WidgetSettings::default())
    }

    #[test]
    fn selection_follows_settings() {
        assert_eq!(selected(&SysmonSettings::default()), [MetricKind::Cpu, MetricKind::Memory]);
        let none = SysmonSettings { cpu: false, memory: false, ..SysmonSettings::default() };
        assert!(selected(&none).is_empty());
    }

    #[test]
    fn values_hide_missing_and_flag_alerts() {
        crate::percent::tr_catalog();
        let s = Snapshot {
            cpu: Some(95.0),
            memory: Some(Memory { total: 100, used: 50, swap_total: 0, swap_used: 0 }),
            gpus: vec![Gpu { driver: "amdgpu".into(), suspended: true, busy: None, vram: None }],
            net: Some(Rates { down: 0.0, up: 2048.0 }),
            cpu_temp: Some(86.0),
            ..Snapshot::default()
        };
        let tv = |k: MetricKind, s: &Snapshot| k.value(s).map(|v| (v.text, k.alert(v.raw[0], false)));
        assert_eq!(tv(MetricKind::Cpu, &s), Some(("%95".into(), true)));
        assert_eq!(tv(MetricKind::Memory, &s), Some(("%50".into(), false)));
        assert_eq!(tv(MetricKind::Gpu, &s), None, "uyuyan GPU: gizli");
        assert_eq!(tv(MetricKind::Temperature, &s), Some(("86°".into(), true)));
        assert_eq!(tv(MetricKind::Network, &s), Some(("↓0K ↑2K".into(), false)));
        assert_eq!(tv(MetricKind::Disk, &s), None);
        assert_eq!(tv(MetricKind::Cpu, &Snapshot::default()), Some(("—".into(), false)), "ilk ölçüm gelene kadar");
    }

    #[test]
    fn thresholds_map_to_framework_defaults() {
        use nuxut::theme::ColorRole;
        use nuxut::ui::Tint;
        assert_eq!(MetricKind::Gpu.icon(), Some(IconName::Gpu));
        for k in [MetricKind::Cpu, MetricKind::Memory, MetricKind::Gpu, MetricKind::Disk, MetricKind::Temperature] {
            let t = k.threshold().unwrap();
            assert_eq!(t.tint, Tint::Role(ColorRole::Error), "renk tema rolünden, gömülü değil");
        }
        assert_eq!(MetricKind::Cpu.threshold().unwrap().level, ALERT_PERCENT);
        assert_eq!(MetricKind::Temperature.threshold().unwrap().level, ALERT_CELSIUS);
        assert!(MetricKind::Network.threshold().is_none());
        // Geri dönüş payı: eşikte açılır, payın altına inene dek açık kalır.
        assert!(!MetricKind::Cpu.alert(89.9, false));
        assert!(MetricKind::Cpu.alert(90.0, false));
        assert!(MetricKind::Cpu.alert(87.5, true));
        assert!(!MetricKind::Cpu.alert(86.9, true));
        assert!(MetricKind::Cpu.alert(f32::NAN, true), "NaN durumu değiştirmez");
        assert!(MetricKind::Temperature.alert(83.5, true));
        assert!(!MetricKind::Temperature.alert(82.9, true));
        assert!(!MetricKind::Network.alert(1e9, false));
    }

    #[test]
    fn deadband() {
        assert!(!MetricKind::Cpu.significant([1.0, 0.0], [2.9, 0.0]), "boşta titreşim");
        assert!(MetricKind::Cpu.significant([1.0, 0.0], [3.0, 0.0]));
        assert!(MetricKind::Cpu.significant([f32::NAN, 0.0], [1.0, 0.0]), "ilk değer");
        assert!(!MetricKind::Temperature.significant([50.0, 0.0], [51.5, 0.0]));
        assert!(!MetricKind::Network.significant([1000.0, 0.0], [4000.0, 0.0]), "taban altında");
        assert!(!MetricKind::Network.significant([100_000.0, 0.0], [110_000.0, 0.0]), "%10 değişim");
        assert!(MetricKind::Network.significant([100_000.0, 0.0], [140_000.0, 0.0]));
        assert!(MetricKind::Network.significant([0.0, 0.0], [0.0, 50_000.0]), "yükleme de sayılır");
    }

    #[test]
    fn sync_only_reports_visible_changes() {
        crate::percent::tr_catalog();
        let (theme, settings) = cx_parts();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 18.0, now: Instant::now(), bounds: Rect::ZERO };
        let mut w = Sysmon::new(&cx);
        let mut model = ShellModel::default();
        assert!(!w.sync(&model, "M", &cx), "veri yok");
        let snap = |cpu: f32| Snapshot { cpu: Some(cpu), ..Snapshot::default() };
        model.sysmon = Some(Arc::new(snap(10.2)));
        assert!(w.sync(&model, "M", &cx));
        assert!(!w.sync(&model, "M", &cx), "aynı Arc");
        model.sysmon = Some(Arc::new(snap(11.4)));
        assert!(!w.sync(&model, "M", &cx), "eşik altında: etiket ve kare değişmez");
        assert!(
            w.tooltip(Rect::ZERO, Point::ZERO, &cx).is_some_and(|t| t.text.starts_with("İşlemci %11")),
            "ipucu her zaman güncel"
        );
        model.sysmon = Some(Arc::new(snap(12.6)));
        assert!(w.sync(&model, "M", &cx));
        assert_eq!(w.metrics[0].label.text(), "%13");
        model.sysmon = Some(Arc::new(snap(91.0)));
        assert!(w.sync(&model, "M", &cx));
        assert!(w.metrics[0].alert);
        model.sysmon = Some(Arc::new(snap(89.5)));
        assert!(!w.sync(&model, "M", &cx), "geri dönüş payı içinde: uyarı sürer, kare yok");
        assert!(w.metrics[0].alert);
        model.sysmon = Some(Arc::new(snap(88.0)));
        assert!(w.sync(&model, "M", &cx), "uyarı sürerken eşik dışı değişim yazılır");
        model.sysmon = Some(Arc::new(snap(86.5)));
        assert!(w.sync(&model, "M", &cx), "uyarı kalkınca eşik altı değişim de yazılır");
        assert!(!w.metrics[0].alert);
    }
}
