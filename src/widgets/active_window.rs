//! Odaklı pencerenin başlığı. Yalnızca yatay bar'da; dikey bar'da gizli (uzunluk 0) — başlık
//! dikey bar'ın dar şeridine sığmaz, döndürülmüş metin de okunmaz.
//!
//! Başlık iki sınırla kısalır: önce en fazla `max_chars` grafem (uzunsa sonuna "…"), sonra
//! piksel sınırı `max_width` (framework tek satırda "…" ile keser). İkisinden biri kesmişse tam başlık
//! [`ActiveWindow::tooltip`] ile ortak ipucu balonuna verilir (framework `Tooltip`, `Screen` çizer).

use std::borrow::Cow;
use std::time::Instant;

use nuxut::ui::{Label, TextRole, text_style};
use nuxut::{Painter, Point, Rect, TextSystem};
use unicode_segmentation::UnicodeSegmentation;

use super::{Action, BarTooltip, BarWidget, WidgetCx, WidgetKind};
use crate::model::ShellModel;

/// Kırpma işareti (tek grafem).
const ELLIPSIS: &str = "…";

#[derive(Debug)]
pub struct ActiveWindow {
    label: Label,
    has_text: bool,
    /// Tam (temizlenmiş) başlık; kırpılmışsa ipucunda gösterilir.
    full: String,
    /// `max_chars` ile kırpıldı.
    truncated: bool,
    /// Gösterilen metnin sınırsız genişliği (ölçümde önbelleklenir; metin/stil değişince `None`).
    natural: Option<f32>,
}

impl ActiveWindow {
    pub fn new(cx: &WidgetCx) -> Self {
        let mut label = Label::new("", style(cx));
        label.set_max_width(Some(cx.settings.active_window.max_width));
        Self { label, has_text: false, full: String::new(), truncated: false, natural: None }
    }

    /// Kısaltılmış metni etikete yazar. Değiştiyse `true`.
    fn apply_text(&mut self, cx: &WidgetCx) -> bool {
        let shown = truncate_graphemes(&self.full, cx.settings.active_window.max_chars as usize);
        self.truncated = matches!(shown, Cow::Owned(_));
        let changed = self.label.set_text(&shown);
        if changed {
            self.natural = None;
        }
        changed
    }
}

impl BarWidget for ActiveWindow {
    // Ekranda görünen başlık (kırpılmışsa tamamı ipucunda da görünür).
    fn access_items(&self, rect: Rect, _cx: &WidgetCx) -> Vec<super::AccessItem> {
        if !self.has_text {
            return Vec::new();
        }
        vec![super::AccessItem::new("window", self.kind().title(), rect).with_value(self.full.as_str())]
    }

    fn kind(&self) -> WidgetKind {
        WidgetKind::ActiveWindow
    }

    fn sync(&mut self, model: &ShellModel, monitor: &str, cx: &WidgetCx) -> bool {
        let title = model.focused_window(monitor).map(|w| w.title.as_str()).unwrap_or("");
        let clean = sanitize_title(title);
        if clean == self.full {
            return false;
        }
        self.full = clean.into_owned();
        self.has_text = !self.full.is_empty();
        self.apply_text(cx)
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.natural = None;
        self.label.set_style(style(cx));
        self.label.set_max_width(Some(cx.settings.active_window.max_width));
        self.apply_text(cx);
    }

    fn measure(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        if cx.vertical || !self.has_text {
            return 0.0;
        }
        if self.natural.is_none() {
            self.natural = Some(text.layout(self.label.text(), self.label.style(), None).size().width);
        }
        self.label.measure(text).width.min(cx.settings.active_window.max_width)
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, _highlighted: bool) {
        let s = self.label.measure_in(painter);
        painter.push_clip(rect);
        self.label.draw(painter, Point::new(rect.x, rect.center().y - s.height * 0.5), cx.theme.colors.on_surface_variant);
        painter.pop_clip();
    }

    fn hover(&mut self, _rect: Rect, _pointer: Option<Point>, _down: bool, _cx: &WidgetCx) -> bool {
        false
    }

    fn click(&mut self, _rect: Rect, _at: Point, _cx: &WidgetCx) -> Option<Action> {
        None
    }

    fn tooltip(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        let width_cut = self.natural.is_some_and(|w| w > cx.settings.active_window.max_width + 0.5);
        ((self.truncated || width_cut) && rect.contains(p)).then_some(BarTooltip { text: self.full.as_str(), anchor: None })
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        false
    }
}

/// Tek satır: `max_width`ı aşarsa framework genişlikte "…" ile keser.
fn style(cx: &WidgetCx) -> nuxut::TextStyle {
    text_style(cx.theme, TextRole::Body).with_max_lines(1)
}

/// Başlık dış girdidir (uygulama yazar): kontrol karakterleri (satır sonu, sekme) boşluğa
/// çevrilir, baş/son boşluk atılır.
fn sanitize_title(title: &str) -> Cow<'_, str> {
    let trimmed = title.trim();
    if trimmed.chars().any(char::is_control) {
        Cow::Owned(trimmed.chars().map(|c| if c.is_control() { ' ' } else { c }).collect())
    } else {
        Cow::Borrowed(trimmed)
    }
}

/// En fazla `max` grafem (kullanıcının gördüğü karakter: birleşik emoji, aksanlı harf bölünmez).
/// Uzunsa `max - 1` grafem + "…" (`Cow::Owned`); sığıyorsa olduğu gibi (`Cow::Borrowed`).
/// Kırpılan yerin sonundaki boşluk "…"den önce atılır. Uzun girdide yalnızca `max + 1`
/// grafeme kadar taranır.
pub fn truncate_graphemes(text: &str, max: usize) -> Cow<'_, str> {
    let max = max.max(1);
    let mut cut = None;
    for (n, (i, _)) in text.grapheme_indices(true).enumerate() {
        if n == max - 1 {
            cut = Some(i);
        }
        if n == max {
            let head = text[..cut.unwrap_or(0)].trim_end();
            return Cow::Owned(format!("{head}{ELLIPSIS}"));
        }
    }
    Cow::Borrowed(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_titles_are_untouched() {
        assert!(matches!(truncate_graphemes("Firefox", 40), Cow::Borrowed("Firefox")));
        assert!(matches!(truncate_graphemes("", 8), Cow::Borrowed("")));
        // Tam sınırda kırpılmaz.
        assert!(matches!(truncate_graphemes("abcdefgh", 8), Cow::Borrowed("abcdefgh")));
    }

    #[test]
    fn long_titles_get_ellipsis_within_limit() {
        let t = truncate_graphemes("abcdefghi", 8);
        assert_eq!(t, "abcdefg…");
        assert_eq!(t.graphemes(true).count(), 8);
    }

    #[test]
    fn graphemes_are_not_split() {
        // Aile emojisi (ZWJ dizisi) ve birleşik aksan tek grafem sayılır, ortadan bölünmez.
        let family = "👨‍👩‍👧";
        let e_acute = "e\u{301}";
        let title = format!("{family}{e_acute}{family}{e_acute}xyz");
        let t = truncate_graphemes(&title, 4);
        assert_eq!(t, format!("{family}{e_acute}{family}…"));
        // Türkçe ve CJK: bayt değil grafem.
        assert_eq!(truncate_graphemes("çğıöşü İstanbul", 6), "çğıöş…");
        assert_eq!(truncate_graphemes("日本語のタイトル", 4), "日本語…");
    }

    #[test]
    fn trailing_space_before_ellipsis_is_dropped() {
        assert_eq!(truncate_graphemes("Mozilla Firefox", 9), "Mozilla…");
    }

    #[test]
    fn titles_are_sanitized() {
        assert_eq!(sanitize_title("  a\nb\tc  "), "a b c");
        assert!(matches!(sanitize_title("plain"), Cow::Borrowed("plain")));
    }

    #[test]
    fn huge_title_is_cheap_and_bounded() {
        let big = "x".repeat(1 << 20);
        let t = truncate_graphemes(&big, 200);
        assert_eq!(t.graphemes(true).count(), 200);
    }
}
