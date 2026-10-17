//! Bar uç eylemleri (`[bar.edge_actions]`): en baştaki/en sondaki widget'a sol tık, sağ tık ve
//! kaydırma eylemi atanır. Tasarım: `docs/design/bar-edge-actions.md`.
//!
//! Burada saf kısımlar durur: eylem adlarının ayrıştırılması, doğrulama, şema, kaydırma birikimi
//! ve ipucu ikonunun animasyonlu çizimi. Yerleşim ve isabet testi `Screen`'de; eylemlerin
//! uygulanması `app/system.rs`'te (`bar_edge_command`).

use std::time::{Duration, Instant};

use nuxut::i18n::t;
use nuxut::ui::{Icon, IconName, IconView, TextRole};
use nuxut::{Animated, Painter, Point, PointerKind, Rect, Theme};
use serde::Deserialize;

use crate::layout::Edge;
use crate::model::ShellModel;
use crate::schema::SettingSchema;
use crate::widgets::{Action, PanelId};

/// Kaydırma adımı (yüzde / çentik) sınırları.
pub const MIN_SCROLL_STEP: u32 = 1;
pub const MAX_SCROLL_STEP: u32 = 25;
/// Dokunmatik yüzeyde bir çentiğe karşılık gelen kaydırma (mantıksal px).
pub const TOUCHPAD_PX_PER_STEP: f32 = 40.0;
/// Bu kadar boşta kalan birikim sıfırlanır.
pub const SCROLL_IDLE_RESET: Duration = Duration::from_millis(400);
/// Tek olayda uygulanan en fazla çentik (hızlı fırlatma tek seferde %100 atlatmasın).
pub const MAX_STEPS_PER_EVENT: i32 = 10;

/// Bar'ın iki ucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeSide {
    /// Yatay: en sol, dikey: en üst.
    Start,
    /// Yatay: en sağ, dikey: en alt.
    End,
}

impl EdgeSide {
    pub const ALL: [EdgeSide; 2] = [EdgeSide::Start, EdgeSide::End];

    pub fn name(self) -> &'static str {
        match self {
            EdgeSide::Start => "start",
            EdgeSide::End => "end",
        }
    }
}

/// Tık eylemi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClickAction {
    /// Widget'ın kendi davranışı.
    #[default]
    Widget,
    /// Hiçbir şey (widget'ınki de bastırılır).
    None,
    Panel(PanelId),
    /// Çıkışı sessize al / aç.
    Volume,
    /// Mikrofonu sessize al / aç.
    Mic,
}

impl ClickAction {
    /// Şemadaki seçenekler (sıra = Ayarlar'daki sıra).
    pub const OPTIONS: [&'static str; 10] =
        ["widget", "none", "panel:left", "panel:right", "panel:top", "panel:bottom", "panel:clock", "panel:launcher", "volume", "mic"];

    pub fn parse(s: &str) -> Result<Self, &'static str> {
        Ok(match s {
            "widget" => Self::Widget,
            "none" => Self::None,
            "volume" => Self::Volume,
            "mic" => Self::Mic,
            "panel:clock" => Self::Panel(PanelId::Clock),
            "panel:launcher" => Self::Panel(PanelId::Launcher),
            "brightness" | "workspace" => return Err("yalnızca scroll için"),
            _ => match s.strip_prefix("panel:").and_then(|n| Edge::ALL.into_iter().find(|e| e.name() == n)) {
                Some(e) => Self::Panel(PanelId::Edge(e)),
                None => return Err("bilinmeyen eylem"),
            },
        })
    }

    /// Şemadaki seçeneklerin görünen adı (bilinmeyen değerde değerin kendisi döner).
    fn option_label(value: &str) -> String {
        match value {
            "widget" => t!("shell-schema-edge-click-widget"),
            "none" => t!("shell-schema-edge-click-none"),
            "panel:left" => t!("shell-schema-edge-click-panel-left"),
            "panel:right" => t!("shell-schema-edge-click-panel-right"),
            "panel:top" => t!("shell-schema-edge-click-panel-top"),
            "panel:bottom" => t!("shell-schema-edge-click-panel-bottom"),
            "panel:clock" => t!("shell-schema-edge-click-panel-clock"),
            "panel:launcher" => t!("shell-schema-edge-click-panel-launcher"),
            "volume" => t!("shell-schema-edge-click-volume"),
            "mic" => t!("shell-schema-edge-click-mic"),
            other => other.to_owned(),
        }
    }

    /// Bu tık uç eylemince mi işlenir? `None`: widget'a gider. `Some(eylem)`: tüketildi
    /// (`Some(None)` = hiçbir şey yapma).
    pub fn resolve(self) -> Option<Option<Action>> {
        match self {
            Self::Widget => None,
            Self::None => Some(None),
            Self::Panel(p) => Some(Some(Action::TogglePanel(p))),
            Self::Volume => Some(Some(Action::BarEdge(EdgeCommand::ToggleMute))),
            Self::Mic => Some(Some(Action::BarEdge(EdgeCommand::ToggleMicMute))),
        }
    }

    /// Tıklanabilir bir eylem mi (imleç `Pointer`).
    pub fn is_action(self) -> bool {
        !matches!(self, Self::Widget | Self::None)
    }
}

/// Kaydırma eylemi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollAction {
    #[default]
    Widget,
    None,
    Volume,
    Mic,
    Brightness,
    Workspace,
}

impl ScrollAction {
    pub const OPTIONS: [&'static str; 6] = ["widget", "none", "volume", "mic", "brightness", "workspace"];

    pub fn parse(s: &str) -> Result<Self, &'static str> {
        Ok(match s {
            "widget" => Self::Widget,
            "none" => Self::None,
            "volume" => Self::Volume,
            "mic" => Self::Mic,
            "brightness" => Self::Brightness,
            "workspace" => Self::Workspace,
            p if p.starts_with("panel:") => return Err("yalnızca tık için"),
            _ => return Err("bilinmeyen eylem"),
        })
    }

    /// Şemadaki seçeneklerin görünen adı (bilinmeyen değerde değerin kendisi döner).
    fn option_label(value: &str) -> String {
        match value {
            "widget" => t!("shell-schema-edge-scroll-widget"),
            "none" => t!("shell-schema-edge-scroll-none"),
            "volume" => t!("shell-schema-edge-scroll-volume"),
            "mic" => t!("shell-schema-edge-scroll-mic"),
            "brightness" => t!("shell-schema-edge-scroll-brightness"),
            "workspace" => t!("shell-schema-edge-scroll-workspace"),
            other => other.to_owned(),
        }
    }

    /// Uç widget'ın dış yanında ipucu ikonu için yer ayrılır mı?
    pub fn has_hint(self) -> bool {
        !matches!(self, Self::Widget | Self::None)
    }

    /// İpucu ikonu (canlı durumdan). `monitor`: bar'ın monitörü (parlaklık).
    pub fn hint_icon(self, model: &ShellModel, monitor: &str) -> Option<IconName> {
        let percent = |v: f32| (v.clamp(0.0, 1.0) * 100.0).round() as u8;
        Some(match self {
            Self::Widget | Self::None => return None,
            Self::Volume => match model.system.audio.default_sink() {
                Some(d) => IconName::volume(percent(d.volume), d.muted),
                None => IconName::VolumeHigh,
            },
            Self::Mic => match model.system.audio.default_source() {
                Some(d) if d.muted => IconName::MicOff,
                _ => IconName::Mic,
            },
            Self::Brightness => match model.system.brightness_display(Some(monitor)) {
                Some(d) => IconName::brightness(d.percent.clamp(0.0, 100.0).round() as u8),
                None => IconName::Brightness,
            },
            Self::Workspace => IconName::Grid,
        })
    }
}

/// Uç eyleminin `App`'e giden komutu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeCommand {
    /// `steps` çentik (pozitif = artır / sonraki workspace).
    Scroll { action: ScrollAction, steps: i32 },
    ToggleMute,
    ToggleMicMute,
}

/// Bir ucun çözülmüş eylemleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EdgeBinding {
    pub left_click: ClickAction,
    pub right_click: ClickAction,
    pub scroll: ScrollAction,
}

/// Uç bölgesindeki bir işaretçi olayının sahibi ([`EdgeBinding::route`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeRoute {
    /// Atanmış kaydırma eylemi: çentikler birikimden geçip [`EdgeCommand::Scroll`] olur.
    Scroll(ScrollAction),
    /// Uç eylemince tüketildi; varsa eylem.
    Done(Option<Action>),
}

impl EdgeBinding {
    /// Öncelik kuralı: `widget` dışında bir eylem atanmışsa uç eylemi kazanır; `widget`
    /// (varsayılan) ise `None` döner ve olay uç widget'ına gider (ör. workspace kaydırması).
    pub fn route(&self, kind: &PointerKind) -> Option<EdgeRoute> {
        use nuxut::Button;
        match *kind {
            PointerKind::Scroll { .. } => match self.scroll {
                ScrollAction::Widget => None,
                ScrollAction::None => Some(EdgeRoute::Done(None)),
                action => Some(EdgeRoute::Scroll(action)),
            },
            PointerKind::ScrollEnd => (self.scroll != ScrollAction::Widget).then_some(EdgeRoute::Done(None)),
            // Basma tüketilir (widget'ın basılı hâli görünmez); eylem bırakmada.
            PointerKind::Press(Button::Left) => self.left_click.resolve().map(|_| EdgeRoute::Done(None)),
            PointerKind::Press(Button::Right) => self.right_click.resolve().map(|_| EdgeRoute::Done(None)),
            PointerKind::Release(Button::Left) => self.left_click.resolve().map(EdgeRoute::Done),
            PointerKind::Release(Button::Right) => self.right_click.resolve().map(EdgeRoute::Done),
            _ => None,
        }
    }
}

/// Bir ucun varsayılan eylem adları (sol tık, sağ tık, kaydırma). Kullanıcının örneği
/// (yorum 5): sağ uçta kaydırma = ses, sol tık = sağ panel, sağ tık atanmamış (widget'ın
/// kendi davranışı); sol uçta kaydırma = parlaklık.
pub const fn side_defaults(side: EdgeSide) -> [&'static str; 3] {
    match side {
        EdgeSide::Start => ["widget", "widget", "brightness"],
        EdgeSide::End => ["panel:right", "widget", "volume"],
    }
}

/// `[bar.edge_actions.start|end]`: ham dizeler; doğrulamada [`EdgeBinding`]'e çözülür.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeSideConfig {
    pub left_click: String,
    pub right_click: String,
    pub scroll: String,
    pub binding: EdgeBinding,
}

impl EdgeSideConfig {
    pub fn defaults(side: EdgeSide) -> Self {
        let [l, r, s] = side_defaults(side);
        // Varsayılan adlar geçerlidir (test korur); çözülemezse `widget`.
        let binding = EdgeBinding {
            left_click: ClickAction::parse(l).unwrap_or_default(),
            right_click: ClickAction::parse(r).unwrap_or_default(),
            scroll: ScrollAction::parse(s).unwrap_or_default(),
        };
        Self { left_click: l.into(), right_click: r.into(), scroll: s.into(), binding }
    }
}

/// `[bar.edge_actions]`. Kısmi tablo yalnızca yazılan anahtarı değiştirir; diğerleri o ucun
/// kendi varsayılanında kalır (`[bar.edge_actions.end] scroll = "mic"` sol tıkı korur).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "EdgeActionsPatch")]
pub struct EdgeActionsConfig {
    /// Bir çentikte ses/mikrofon/parlaklık değişimi (yüzde).
    pub scroll_step: u32,
    pub start: EdgeSideConfig,
    pub end: EdgeSideConfig,
}

impl Default for EdgeActionsConfig {
    fn default() -> Self {
        Self { scroll_step: 5, start: EdgeSideConfig::defaults(EdgeSide::Start), end: EdgeSideConfig::defaults(EdgeSide::End) }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct EdgeSidePatch {
    left_click: Option<String>,
    right_click: Option<String>,
    scroll: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct EdgeActionsPatch {
    scroll_step: Option<u32>,
    start: EdgeSidePatch,
    end: EdgeSidePatch,
}

impl From<EdgeActionsPatch> for EdgeActionsConfig {
    fn from(p: EdgeActionsPatch) -> Self {
        let d = Self::default();
        let mut c = Self { scroll_step: p.scroll_step.unwrap_or(d.scroll_step), ..d };
        for (patch, side) in [(p.start, &mut c.start), (p.end, &mut c.end)] {
            if let Some(v) = patch.left_click {
                side.left_click = v;
            }
            if let Some(v) = patch.right_click {
                side.right_click = v;
            }
            if let Some(v) = patch.scroll {
                side.scroll = v;
            }
        }
        c
    }
}

impl EdgeActionsConfig {
    pub fn side(&self, side: EdgeSide) -> &EdgeSideConfig {
        match side {
            EdgeSide::Start => &self.start,
            EdgeSide::End => &self.end,
        }
    }

    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(SettingSchema::int(
            "bar.edge_actions.scroll_step",
            t!("shell-bar-edge-scroll-step-label"),
            d.scroll_step.into(),
            MIN_SCROLL_STEP.into(),
            MAX_SCROLL_STEP.into(),
        ));
        for side in EdgeSide::ALL {
            let key = |k: &str| format!("bar.edge_actions.{}.{k}", side.name());
            let [dl, dr, ds] = side_defaults(side);
            let (lc, rc, sc) = match side {
                EdgeSide::Start => (
                    t!("shell-bar-edge-start-left-click-label"),
                    t!("shell-bar-edge-start-right-click-label"),
                    t!("shell-bar-edge-start-scroll-label"),
                ),
                EdgeSide::End => (
                    t!("shell-bar-edge-end-left-click-label"),
                    t!("shell-bar-edge-end-right-click-label"),
                    t!("shell-bar-edge-end-scroll-label"),
                ),
            };
            out.push(
                SettingSchema::choice(key("left_click"), lc, dl, ClickAction::OPTIONS.to_vec())
                    .labels(ClickAction::OPTIONS.iter().map(|o| ClickAction::option_label(o)).collect()),
            );
            out.push(
                SettingSchema::choice(key("right_click"), rc, dr, ClickAction::OPTIONS.to_vec())
                    .labels(ClickAction::OPTIONS.iter().map(|o| ClickAction::option_label(o)).collect()),
            );
            out.push(
                SettingSchema::choice(key("scroll"), sc, ds, ScrollAction::OPTIONS.to_vec())
                    .labels(ScrollAction::OPTIONS.iter().map(|o| ScrollAction::option_label(o)).collect()),
            );
        }
    }

    /// Sınırlar ve eylem adlarını çözer; geçersiz değer uyarıyla o ucun varsayılanına döner. İdempotent.
    pub fn validate(&mut self, w: &mut Vec<String>) {
        if !(MIN_SCROLL_STEP..=MAX_SCROLL_STEP).contains(&self.scroll_step) {
            let fixed = self.scroll_step.clamp(MIN_SCROLL_STEP, MAX_SCROLL_STEP);
            w.push(format!(
                "bar.edge_actions.scroll_step = {} aralık dışı, {fixed} kullanılıyor ({MIN_SCROLL_STEP}..={MAX_SCROLL_STEP})",
                self.scroll_step
            ));
            self.scroll_step = fixed;
        }
        for (side, cfg) in [(EdgeSide::Start, &mut self.start), (EdgeSide::End, &mut self.end)] {
            let key = |k: &str| format!("bar.edge_actions.{}.{k}", side.name());
            let [dl, dr, ds] = side_defaults(side);
            cfg.binding.left_click = resolve(&mut cfg.left_click, &key("left_click"), dl, ClickAction::parse, w);
            cfg.binding.right_click = resolve(&mut cfg.right_click, &key("right_click"), dr, ClickAction::parse, w);
            cfg.binding.scroll = resolve(&mut cfg.scroll, &key("scroll"), ds, ScrollAction::parse, w);
        }
    }
}

fn resolve<T: Default>(
    value: &mut String,
    key: &str,
    default: &str,
    parse: fn(&str) -> Result<T, &'static str>,
    w: &mut Vec<String>,
) -> T {
    match parse(value) {
        Ok(v) => v,
        Err(reason) => {
            let shown: String = crate::config::sanitize(value).chars().take(40).collect();
            w.push(format!("{key} = \"{shown}\" geçersiz ({reason}), \"{default}\" kullanılıyor"));
            *value = default.into();
            parse(default).unwrap_or_default()
        }
    }
}

/// Tekerlek ve dokunmatik yüzey kaydırmasını tam çentiklere çevirir (kesir birikir).
#[derive(Debug, Clone, Default)]
pub struct ScrollAccumulator {
    acc: f32,
    last: Option<Instant>,
}

impl ScrollAccumulator {
    /// `PointerKind::Scroll` (pozitif y aşağı). Döner: uygulanacak çentik (pozitif = yukarı /
    /// artır), en fazla [`MAX_STEPS_PER_EVENT`].
    pub fn push(&mut self, delta: Point, steps: Point, now: Instant) -> i32 {
        if self.last.is_some_and(|t| now.saturating_duration_since(t) > SCROLL_IDLE_RESET) {
            self.acc = 0.0;
        }
        self.last = Some(now);
        // Yatay baskın kaydırma yok sayılır (dokunmatik yüzeyde diyagonal gürültü).
        let (x, y) = if steps.x != 0.0 || steps.y != 0.0 {
            (steps.x, steps.y)
        } else {
            (delta.x / TOUCHPAD_PX_PER_STEP, delta.y / TOUCHPAD_PX_PER_STEP)
        };
        let amount = -y;
        if !amount.is_finite() || amount == 0.0 || x.abs() > y.abs() {
            return 0;
        }
        if self.acc * amount < 0.0 {
            // Yön değişti: eski yönün kesri yeni yönü geciktirmesin.
            self.acc = 0.0;
        }
        self.acc = (self.acc + amount).clamp(-(MAX_STEPS_PER_EVENT as f32) - 1.0, MAX_STEPS_PER_EVENT as f32 + 1.0);
        let whole = self.acc.trunc();
        self.acc -= whole;
        (whole as i32).clamp(-MAX_STEPS_PER_EVENT, MAX_STEPS_PER_EVENT)
    }

    /// Dokunmatik yüzeyde parmaklar kalktı: kalan kesir atılır.
    pub fn end(&mut self) {
        self.acc = 0.0;
    }
}

/// Uç widget'ın dış yanında beliren ipucu ikonu. Yeri baştan ayrılır; burada yalnızca
/// görünürlük animasyonu ve çizim.
#[derive(Debug)]
pub struct EdgeHint {
    icon: IconView,
    /// 0 = gizli, 1 = yerinde (`motion.spatial` yayı).
    shown: Animated<f32>,
}

impl EdgeHint {
    pub fn new(theme: &Theme) -> Self {
        Self { icon: IconView::with_role(theme, Icon::Named(None), TextRole::IconSmall), shown: Animated::new(0.0, theme.motion.spatial) }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.icon.set_theme(theme, now);
        self.shown.set_spring(theme.motion.spatial, now);
    }

    /// İkon değiştiyse `true`.
    pub fn set_icon(&mut self, icon: Option<IconName>) -> bool {
        self.icon.set_icon(Icon::Named(icon))
    }

    /// Fare eylem bölgesinde mi. Hedef değiştiyse `true`.
    pub fn set_hovered(&mut self, hovered: bool, now: Instant) -> bool {
        let target = if hovered { 1.0 } else { 0.0 };
        if self.shown.target() == target {
            return false;
        }
        self.shown.set_target(target, now);
        true
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        !self.shown.is_settled(now)
    }

    /// `slot`: ayrılmış yuva; `toward`: yuvadan widget'a birim yön (ikon oradan büyüyerek çıkar).
    pub fn draw(&mut self, painter: &mut Painter, slot: Rect, toward: Point, theme: &Theme, now: Instant) {
        let t = self.shown.value(now).clamp(0.0, 1.0);
        if t <= 0.003 {
            return;
        }
        let shift = (1.0 - t) * theme.space(2);
        let c = slot.center();
        let center = Point::new(c.x + toward.x * shift, c.y + toward.y * shift);
        let color = theme.colors.on_surface_variant;
        self.icon.draw_centered(painter, center, color.with_alpha(color.a * t));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validated(text: &str) -> (EdgeActionsConfig, Vec<String>) {
        let mut c: EdgeActionsConfig = toml::from_str(text).unwrap();
        let mut w = Vec::new();
        c.validate(&mut w);
        (c, w)
    }

    #[test]
    fn route_priority_assigned_action_beats_widget() {
        use nuxut::Button;
        let scroll = PointerKind::Scroll { delta: Point::ZERO, steps: Point::new(0.0, 1.0) };
        let release = PointerKind::Release(Button::Left);
        let press = PointerKind::Press(Button::Left);
        let widget = EdgeBinding::default();
        assert_eq!(widget.route(&scroll), None, "`widget`: kaydırma uç widget'ına");
        assert_eq!(widget.route(&PointerKind::ScrollEnd), None);
        assert_eq!(widget.route(&release), None, "`widget`: tık uç widget'ına");
        assert_eq!(widget.route(&press), None);
        assert_eq!(widget.route(&PointerKind::Motion), None);

        let volume = EdgeBinding { scroll: ScrollAction::Volume, ..Default::default() };
        assert_eq!(volume.route(&scroll), Some(EdgeRoute::Scroll(ScrollAction::Volume)));
        assert_eq!(volume.route(&PointerKind::ScrollEnd), Some(EdgeRoute::Done(None)));
        assert_eq!(volume.route(&release), None, "yalnızca kaydırma atandı; tık widget'ta");
        let none = EdgeBinding { scroll: ScrollAction::None, ..Default::default() };
        assert_eq!(none.route(&scroll), Some(EdgeRoute::Done(None)), "`none` widget'ı da bastırır");

        let panel = EdgeBinding { left_click: ClickAction::Panel(PanelId::Edge(Edge::Right)), ..Default::default() };
        assert_eq!(panel.route(&press), Some(EdgeRoute::Done(None)), "basma tüketilir");
        assert_eq!(panel.route(&release), Some(EdgeRoute::Done(Some(Action::TogglePanel(PanelId::Edge(Edge::Right))))));
        assert_eq!(panel.route(&scroll), None, "yalnızca tık atandı; kaydırma widget'ta");
    }

    #[test]
    fn user_scenario_parses() {
        let (c, w) = validated(
            "scroll_step = 2\n[start]\nscroll = \"brightness\"\n[end]\nleft_click = \"panel:right\"\nright_click = \"none\"\nscroll = \"volume\"\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.scroll_step, 2);
        assert_eq!(c.start.binding, EdgeBinding { scroll: ScrollAction::Brightness, ..Default::default() });
        assert_eq!(
            c.end.binding,
            EdgeBinding {
                left_click: ClickAction::Panel(PanelId::Edge(Edge::Right)),
                right_click: ClickAction::None,
                scroll: ScrollAction::Volume
            }
        );
    }

    #[test]
    fn every_option_parses_and_names_round_trip() {
        for o in ClickAction::OPTIONS {
            assert!(ClickAction::parse(o).is_ok(), "{o}");
        }
        for o in ScrollAction::OPTIONS {
            assert!(ScrollAction::parse(o).is_ok(), "{o}");
        }
    }

    #[test]
    fn invalid_actions_fall_back_with_warning() {
        crate::i18n::use_turkish_for_tests();
        let (c, w) = validated(
            "scroll_step = 99\n[start]\nleft_click = \"brightness\"\nscroll = \"panel:right\"\n[end]\nright_click = \"exec:rm -rf ~\"\n",
        );
        assert_eq!(c.scroll_step, MAX_SCROLL_STEP);
        let d = EdgeActionsConfig::default();
        assert_eq!(c.start.binding, d.start.binding, "geçersiz değer o ucun varsayılanına döner");
        assert_eq!(c.end.binding, d.end.binding);
        assert_eq!(c.start.scroll, "brightness");
        assert_eq!(c.start.left_click, "widget");
        assert_eq!(c.end.right_click, "widget");
        assert_eq!(w.len(), 4, "{w:?}");
        assert!(w[1].contains("yalnızca scroll"), "{w:?}");
        assert!(w[2].contains("yalnızca tık"), "{w:?}");
        // İdempotent: ikinci doğrulama uyarısız.
        let mut c2 = c.clone();
        let mut w2 = Vec::new();
        c2.validate(&mut w2);
        assert!(w2.is_empty() && c2 == c);
    }

    #[test]
    fn defaults_follow_user_scenario_and_partial_tables_keep_them() {
        let d = EdgeActionsConfig::default();
        assert_eq!(d.start.binding, EdgeBinding { scroll: ScrollAction::Brightness, ..Default::default() });
        assert_eq!(
            d.end.binding,
            EdgeBinding {
                left_click: ClickAction::Panel(PanelId::Edge(Edge::Right)),
                right_click: ClickAction::Widget,
                scroll: ScrollAction::Volume
            }
        );
        // Boş tablo = varsayılan; kısmi tablo yalnızca yazılanı değiştirir.
        assert_eq!(validated("").0, d);
        let (c, w) = validated("[end]\nscroll = \"mic\"\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.end.binding.scroll, ScrollAction::Mic);
        assert_eq!(c.end.binding.left_click, d.end.binding.left_click);
        assert_eq!(c.start, d.start);
        // Varsayılan adlar geçerli (Default çözülmüş bağlamayla doğrulanmışa eşit).
        let mut v = d.clone();
        let mut w = Vec::new();
        v.validate(&mut w);
        assert!(w.is_empty() && v == d, "{w:?}");
    }

    #[test]
    fn click_resolution() {
        assert_eq!(ClickAction::Widget.resolve(), None);
        assert_eq!(ClickAction::None.resolve(), Some(None));
        assert_eq!(ClickAction::parse("panel:launcher"), Ok(ClickAction::Panel(PanelId::Launcher)));
        assert_eq!(ClickAction::Panel(PanelId::Launcher).resolve(), Some(Some(Action::TogglePanel(PanelId::Launcher))));
        assert!(ClickAction::OPTIONS.iter().all(|o| ClickAction::parse(o).is_ok()));
        assert_eq!(ScrollAction::parse("panel:launcher"), Err("yalnızca tık için"));
        assert_eq!(ClickAction::Panel(PanelId::Clock).resolve(), Some(Some(Action::TogglePanel(PanelId::Clock))));
        assert_eq!(ClickAction::Volume.resolve(), Some(Some(Action::BarEdge(EdgeCommand::ToggleMute))));
        assert!(ClickAction::Mic.is_action() && !ClickAction::None.is_action());
        assert!(ScrollAction::Brightness.has_hint() && !ScrollAction::Widget.has_hint());
    }

    #[test]
    fn wheel_notches_apply_directly() {
        let mut a = ScrollAccumulator::default();
        let t = Instant::now();
        // Yukarı = artır.
        assert_eq!(a.push(Point::new(0.0, -15.0), Point::new(0.0, -1.0), t), 1);
        assert_eq!(a.push(Point::new(0.0, 15.0), Point::new(0.0, 1.0), t), -1);
        // Dev çentik sınırlanır.
        assert_eq!(a.push(Point::ZERO, Point::new(0.0, -50.0), t), MAX_STEPS_PER_EVENT);
    }

    #[test]
    fn high_resolution_wheel_accumulates() {
        let mut a = ScrollAccumulator::default();
        let t = Instant::now();
        let eighth = Point::new(0.0, -0.125);
        let total: i32 = (0..16).map(|_| a.push(Point::ZERO, eighth, t)).sum();
        assert_eq!(total, 2);
    }

    #[test]
    fn touchpad_accumulates_and_resets() {
        let mut a = ScrollAccumulator::default();
        let t = Instant::now();
        let px = |y: f32| Point::new(0.0, y);
        assert_eq!(a.push(px(-25.0), Point::ZERO, t), 0);
        assert_eq!(a.push(px(-25.0), Point::ZERO, t), 1, "50 px ≥ bir çentik");
        // Kalan 10 px; yön değişince atılır.
        assert_eq!(a.push(px(30.0), Point::ZERO, t), 0);
        assert_eq!(a.push(px(15.0), Point::ZERO, t), -1);
        // ScrollEnd kesri atar.
        a.push(px(-30.0), Point::ZERO, t);
        a.end();
        assert_eq!(a.push(px(-30.0), Point::ZERO, t), 0);
        // Boşta kalınca kesir atılır.
        assert_eq!(a.push(px(-30.0), Point::ZERO, t + SCROLL_IDLE_RESET * 2), 0);
        // Yatay baskın ve NaN yok sayılır.
        assert_eq!(a.push(Point::new(-200.0, -10.0), Point::ZERO, t), 0);
        assert_eq!(a.push(Point::new(0.0, f32::NAN), Point::ZERO, t), 0);
    }
}
