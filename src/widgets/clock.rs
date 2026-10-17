//! Saat: yatay bar'da `format`, dikey bar'da `vertical_format` (satırlara bölünmüş, ortalı).
//! Tıklayınca tarih popout'unu açar; popout açıkken hap vurgulanır. Üzerine gelince ipucu
//! balonunda gün adıyla tam tarih ([`clock::full_date`], zaman locale'iyle: "Salı, 13 Ekim
//! 2026"); popout açıkken ipucu yok (popout aynı tarihi büyük gösterir, balon onun üstüne
//! düşerdi).

use std::time::Instant;

use chrono::{DateTime, Local};
use nuxut::render::TextAlign;
use nuxut::ui::{Interaction, Label, TextRole, text_style};
use nuxut::{Corners, CursorShape, Painter, Point, Rect, RectStyle, TextSystem};

use super::{Action, BarTooltip, BarWidget, PanelId, WidgetCx, WidgetKind, center_in, chip_fill, draw_hover};
use crate::services::clock::{self, ClockFormat};
use nuxut::locale::Locale;

#[derive(Debug)]
pub struct Clock {
    label: Label,
    hover: Interaction,
    /// Saat/tarih locale'i (`[locale] formats` + `hour_cycle`): kurulumda alınır; dil ya da
    /// `hour_cycle` değişince (`locale_changed`) widget yeniden kurulur.
    locale: Locale,
    /// Son biçimlendirmede kullanılan (kullanıcı kalıbı, dikey mi, çözülmüş) biçim; kalıp
    /// `None` = otomatik ([`ClockFormat::auto`]).
    format: Option<(Option<String>, bool, ClockFormat)>,
    vertical: bool,
    /// İpucu metni: bugünün tam tarihi (gün dönünce `set_time` yeniler).
    full_date: String,
    /// Saat popout'u açık mı (son çizimin `highlighted`'ı; tıklamada hemen çevrilir ki
    /// aynı olayın hover adımı balonu kapatsın).
    popout_open: bool,
}

impl Clock {
    pub fn new(cx: &WidgetCx) -> Self {
        let mut s = Self {
            label: Label::new("", Self::style(cx)),
            hover: Interaction::new(cx.theme),
            locale: clock::time_locale(),
            format: None,
            vertical: cx.vertical,
            full_date: String::new(),
            popout_open: false,
        };
        s.set_time(&Local::now(), cx);
        s
    }

    fn style(cx: &WidgetCx) -> nuxut::TextStyle {
        // Saat göstergesi: temanın `display` ailesi, bar'a sığan gövde boyutunda; rakamlar eş
        // genişlikte (dakika değişince metin kaymaz).
        let mut s = text_style(cx.theme, TextRole::Display).with_tabular_numbers();
        s.size = text_style(cx.theme, TextRole::Body).size;
        if cx.vertical {
            s.align = TextAlign::Center;
            s.line_height = 1.05;
        }
        s
    }

    fn padding(cx: &WidgetCx) -> f32 {
        if cx.vertical { cx.theme.space(2) } else { cx.theme.space(3) }
    }
}

impl BarWidget for Clock {
    fn access_items(&self, rect: Rect, _cx: &WidgetCx) -> Vec<super::AccessItem> {
        let mut value = self.label.text().to_owned();
        if !self.full_date.is_empty() {
            value.push_str(", ");
            value.push_str(&self.full_date);
        }
        vec![super::AccessItem::new("clock", self.kind().title(), rect).with_value(value)]
    }

    fn kind(&self) -> WidgetKind {
        WidgetKind::Clock
    }

    fn set_time(&mut self, now: &DateTime<Local>, cx: &WidgetCx) -> bool {
        let pattern = cx.settings.clock.pattern(cx.vertical);
        if self.format.as_ref().is_none_or(|(p, v, _)| p.as_deref() != pattern || *v != cx.vertical) {
            // Ayarlar doğrulanmış; yine de geçersiz kalıpta panik yerine otomatik.
            let f = ClockFormat::resolve(pattern, cx.vertical, &self.locale);
            self.format = Some((pattern.map(str::to_owned), cx.vertical, f));
        }
        let text = self.format.as_ref().map(|(_, _, f)| f.format(now)).unwrap_or_default();
        // İpucu bar'da çizilmez (balonu `Screen` her hover'da sorar): hasar gerekmez.
        self.full_date = clock::full_date(now, &self.locale);
        self.label.set_text(&text)
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.hover.set_theme(cx.theme, cx.now);
        if self.vertical != cx.vertical {
            self.vertical = cx.vertical;
            self.set_time(&Local::now(), cx);
        }
        self.label.set_style(Self::style(cx));
    }

    fn measure(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        let s = self.label.measure(text);
        if cx.vertical { s.height + Self::padding(cx) * 2.0 } else { s.width + Self::padding(cx) * 2.0 }
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, highlighted: bool) {
        self.popout_open = highlighted;
        let colors = &cx.theme.colors;
        let chip = chip_rect(rect, cx);
        let radii = Corners::uniform(if cx.vertical { cx.theme.shape.radius_small } else { chip.height * 0.5 });
        if highlighted {
            painter.rect(chip, &RectStyle::filled(colors.surface_bright, radii));
        }
        let color = if highlighted { colors.primary } else { colors.on_surface };
        draw_hover(painter, &self.hover, chip, radii, cx, chip_fill(cx.theme, highlighted), &[color]);
        let s = self.label.measure_in(painter);
        self.label.draw(painter, center_in(chip, s.width, s.height), color);
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        self.hover.pointer(chip_rect(rect, cx), pointer, down, cx.now)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        let hit = chip_rect(rect, cx).contains(at);
        if hit {
            self.popout_open = !self.popout_open;
        }
        hit.then_some(Action::TogglePanel(PanelId::Clock))
    }

    fn panel(&self, _cx: &WidgetCx) -> Option<PanelId> {
        Some(PanelId::Clock)
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.hover.is_animating(now)
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        chip_rect(rect, cx).contains(p).then_some(CursorShape::Pointer)
    }

    fn tooltip(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        let chip = chip_rect(rect, cx);
        (!self.popout_open && !self.full_date.is_empty() && chip.contains(p))
            .then_some(BarTooltip { text: &self.full_date, anchor: Some(chip) })
    }
}

/// Saat hapı: bar'a dik eksende öğe boyunda, bar ekseninde widget boyunca.
fn chip_rect(rect: Rect, cx: &WidgetCx) -> Rect {
    if cx.vertical {
        Rect::new(rect.center().x - cx.item * 0.5, rect.y, cx.item, rect.height)
    } else {
        Rect::new(rect.x, rect.center().y - cx.item * 0.5, rect.width, cx.item)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::widgets::WidgetSettings;

    fn cx<'a>(theme: &'a nuxut::Theme, settings: &'a WidgetSettings) -> WidgetCx<'a> {
        WidgetCx { theme, settings, vertical: false, item: 26.0, now: Instant::now(), bounds: Rect::ZERO }
    }

    /// t119: yazılmamış biçim widget'ın locale'inin 12/24 saatine uyar; yazılmış kalıp kazanır;
    /// dikey bar satırlara böler.
    #[test]
    fn bar_text_follows_hour_cycle_unless_written() {
        let theme = nuxut::Theme::default();
        let t = Local.with_ymd_and_hms(2026, 10, 13, 13, 5, 0).unwrap();
        let text = |settings: &WidgetSettings, locale: Locale, vertical: bool| {
            let mut cx = cx(&theme, settings);
            cx.vertical = vertical;
            let mut w = Clock::new(&cx);
            w.locale = locale;
            w.format = None;
            w.set_time(&t, &cx);
            w.label.text().to_owned()
        };
        let en = || Locale::parse("en_US").unwrap();
        let tr = || Locale::parse("tr_TR").unwrap();
        let auto = WidgetSettings::default();
        assert_eq!(text(&auto, tr(), false), "13:05");
        assert_eq!(text(&auto, en(), false), "1:05 PM");
        assert_eq!(text(&auto, en(), true), "1\n05\nPM");
        assert_eq!(text(&auto, en().with_12_hour(Some(false)), false), "13:05");
        assert_eq!(text(&auto, tr().with_12_hour(Some(true)), false), clock::format_hm(&tr().with_12_hour(Some(true)), 13, 5));
        assert_eq!(text(&auto, tr(), true), "13\n05");
        let mut written = WidgetSettings::default();
        written.clock.format = Some("%H:%M".into());
        written.clock.vertical_format = Some("%H".into());
        assert_eq!(text(&written, en().with_12_hour(Some(true)), false), "13:05", "açık kalıp kazanır");
        assert_eq!(text(&written, en(), true), "13");
    }

    #[test]
    fn tooltip_shows_full_date_and_follows_day() {
        let (theme, settings) = (nuxut::Theme::default(), WidgetSettings::default());
        let cx = cx(&theme, &settings);
        let mut w = Clock::new(&cx);
        let rect = Rect::new(0.0, 0.0, 60.0, 34.0);
        let inside = rect.center();
        let day = Local.with_ymd_and_hms(2026, 10, 12, 23, 59, 0).unwrap();
        w.set_time(&day, &cx);
        let tip = w.tooltip(rect, inside, &cx).expect("ipucu var");
        assert_eq!(tip.text, clock::full_date(&day, &clock::time_locale()));
        assert_eq!(tip.anchor, Some(chip_rect(rect, &cx)));
        // Gün dönünce metin de döner.
        let next = Local.with_ymd_and_hms(2026, 10, 13, 0, 0, 0).unwrap();
        w.set_time(&next, &cx);
        assert_eq!(w.tooltip(rect, inside, &cx).unwrap().text, clock::full_date(&next, &clock::time_locale()));
        assert_ne!(clock::full_date(&day, &clock::time_locale()), clock::full_date(&next, &clock::time_locale()));
        // Hap dışı (dikey eksende hap öğe boyunda) ve popout açıkken ipucu yok.
        assert!(w.tooltip(rect, Point::new(inside.x, 1.0), &cx).is_none());
        assert_eq!(w.click(rect, inside, &cx), Some(Action::TogglePanel(PanelId::Clock)));
        assert!(w.tooltip(rect, inside, &cx).is_none(), "popout açılıyor");
        assert_eq!(w.click(rect, inside, &cx), Some(Action::TogglePanel(PanelId::Clock)));
        assert!(w.tooltip(rect, inside, &cx).is_some(), "popout kapandı");
    }
}
