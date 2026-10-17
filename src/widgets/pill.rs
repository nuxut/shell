//! Bar'ın ikon + kısa metin hapı (büyüteç, klavye düzeni, sıradaki etkinlik): öğe boyunda,
//! yatay bar'da ikon ve metin yan yana, dikey bar'da yalnız ikon (metin kalınlığa sığmaz;
//! `text_in_vertical` ile kısa metin — "TR" — ikon yerine). Dolgu çağırandan: vurgulu durum
//! `primary` dolgu + `on_primary` içerik (caffeine ile aynı dil), sakin durum dolgusuz.

use std::time::Instant;

use nuxut::render::RectStyle;
use nuxut::ui::{Icon, IconName, IconView, Interaction, Label, TextRole, text_style};
use nuxut::{Color, Corners, Painter, Point, Rect, TextSystem};

use super::{WidgetCx, center_in, chip_fill, draw_hover};

#[derive(Debug)]
pub struct Pill {
    icon: IconView,
    label: Label,
    /// Dikey bar'da ikon yerine metin.
    text_in_vertical: bool,
    pub hover: Interaction,
}

impl Pill {
    pub fn new(cx: &WidgetCx, icon: IconName) -> Self {
        Self {
            icon: IconView::with_role(cx.theme, Icon::named(icon), TextRole::IconSmall),
            label: Label::new("", Self::style(cx)),
            text_in_vertical: false,
            hover: Interaction::new(cx.theme),
        }
    }

    pub fn with_text_in_vertical(mut self) -> Self {
        self.text_in_vertical = true;
        self
    }

    fn style(cx: &WidgetCx) -> nuxut::TextStyle {
        text_style(cx.theme, TextRole::Label).with_tabular_numbers().with_max_lines(1)
    }

    /// Metin değiştiyse `true`.
    pub fn set_text(&mut self, text: &str) -> bool {
        self.label.set_text(text)
    }

    pub fn set_theme(&mut self, cx: &WidgetCx) {
        self.icon.set_theme(cx.theme, cx.now);
        self.label.set_style(Self::style(cx));
        self.hover.set_theme(cx.theme, cx.now);
    }

    fn padding(cx: &WidgetCx) -> f32 {
        cx.theme.space(2)
    }

    /// İkon ile metin arası (sysmon ölçümleriyle aynı ritim).
    fn gap(cx: &WidgetCx) -> f32 {
        cx.theme.space(2) * 0.75
    }

    fn shows_icon(&self, cx: &WidgetCx) -> bool {
        !(cx.vertical && self.text_in_vertical)
    }

    fn shows_text(&self, cx: &WidgetCx) -> bool {
        !cx.vertical || self.text_in_vertical
    }

    /// Bar ekseni boyunca uzunluk.
    pub fn measure(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        if cx.vertical {
            return cx.item;
        }
        let t = self.label.measure(text).width;
        let icon = cx.item * 0.6;
        Self::padding(cx) * 2.0 + icon + if t > 0.0 { Self::gap(cx) + t } else { 0.0 }
    }

    /// Hap dikdörtgeni: bar'a dik eksende öğe boyu.
    pub fn chip(rect: Rect, cx: &WidgetCx) -> Rect {
        if cx.vertical {
            Rect::new(rect.center().x - cx.item * 0.5, rect.center().y - cx.item * 0.5, cx.item, cx.item)
        } else {
            Rect::new(rect.x, rect.center().y - cx.item * 0.5, rect.width, cx.item)
        }
    }

    /// `filled`: `Some(dolgu)` → dolgulu hap, içerik `fg`.
    pub fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, filled: Option<Color>, fg: Color) {
        let chip = Self::chip(rect, cx);
        let radii = Corners::uniform(chip.height.min(chip.width) * 0.5);
        if let Some(fill) = filled {
            painter.rect(chip, &RectStyle::filled(fill, radii));
        }
        draw_hover(painter, &self.hover, chip, radii, cx, filled.unwrap_or(chip_fill(cx.theme, false)), &[fg]);
        let (icon, text) = (self.shows_icon(cx), self.shows_text(cx));
        let ts = self.label.measure_in(painter);
        if icon && !text {
            self.icon.draw_centered(painter, chip.center(), fg);
            return;
        }
        if text && !icon {
            self.label.draw(painter, center_in(chip, ts.width, ts.height), fg);
            return;
        }
        let x = chip.x + Self::padding(cx);
        let icon_w = cx.item * 0.6;
        self.icon.draw_centered(painter, Point::new(x + icon_w * 0.5, chip.center().y), fg);
        self.label.draw(painter, Point::new(x + icon_w + Self::gap(cx), chip.center().y - ts.height * 0.5), fg);
    }

    pub fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        self.hover.pointer(Self::chip(rect, cx), pointer, down, cx.now)
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        self.hover.is_animating(now)
    }
}
