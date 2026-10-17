//! Sıradaki etkinlik (`next_event`): `lead_minutes` içinde başlayacak ya da süren etkinlik
//! bar'da "Ekip toplantısı · 12 dk"; yoksa gizli (uzunluk 0). Tık: saat popout'u (takvim).
//! Kaynak `ShellModel::calendar` (`nuxut-calendard`); bar'da bu widget varken `App` takvim
//! sorgusuna bugün + yarını ekler (`near_range`). Metin saat vuruşunda (dakikada bir, zaten var)
//! yenilenir; ek uyanma yok. Tasarım: `docs/design/next-event.md`.

use std::time::Instant;

use chrono::{DateTime, Local, TimeZone};
use nuxut::i18n::t;
use nuxut::ui::IconName;
use nuxut::{CursorShape, Painter, Point, Rect, TextSystem};
use nuxut_calendar_ipc::OccurrenceEntry;

use super::pill::Pill;
use super::{Action, BarTooltip, BarWidget, PanelId, WidgetCx, WidgetKind};
use crate::model::ShellModel;

/// Bar'daki başlığın en çok karakteri (tamamı ipucunda).
const MAX_TITLE_CHARS: usize = 24;
/// Bu kadar dakika kala hap `primary` dolguya geçer (dikkat).
const SOON_MINUTES: i64 = 5;
/// Saklanan en çok aday (yakın aralığın etkinlikleri; dış girdi sınırı).
const MAX_CANDIDATES: usize = 64;

/// Bugünün yerel gece yarısından iki gün: en uzun öncü süre (12 sa) günün herhangi bir anından
/// yarına taşsa da kapsanır; aralık günde bir değişir (sorgu günde bir yenilenir).
pub fn near_range(now: DateTime<Local>) -> (i64, i64) {
    let midnight = now.date_naive().and_hms_opt(0, 0, 0).and_then(|d| Local.from_local_datetime(&d).earliest());
    let from = midnight.map_or(now.timestamp() - 86_400, |m| m.timestamp());
    (from, from + 2 * 86_400)
}

/// Seçilen etkinlik ve durumu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub summary: String,
    pub start: i64,
    pub end: i64,
    /// Başladı mı (sürüyor).
    pub ongoing: bool,
}

/// Gösterilecek etkinlik: tüm gün ve iptal edilenler hariç; `now + lead` içinde başlayan en
/// yakın etkinlik, yoksa süren etkinliklerden en son başlayanı (yeni bir toplantı eskisinin
/// üstüne binmişse yenisi).
pub fn pick(entries: &[OccurrenceEntry], now: i64, lead_minutes: u32) -> Option<Pick> {
    let lead = i64::from(lead_minutes) * 60;
    let usable = entries.iter().filter(|e| !e.all_day && e.status != "cancelled" && e.end > now && e.end > e.start);
    let upcoming = usable.clone().filter(|e| e.start >= now && e.start <= now + lead).min_by_key(|e| e.start);
    let chosen = upcoming.or_else(|| usable.filter(|e| e.start < now).max_by_key(|e| e.start))?;
    Some(Pick { summary: chosen.summary.clone(), start: chosen.start, end: chosen.end, ongoing: chosen.start < now })
}

fn hhmm(ts: i64) -> String {
    Local.timestamp_opt(ts, 0).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
}

fn short_title(summary: &str) -> String {
    let s: String = summary.chars().filter(|c| !c.is_control()).collect();
    let s = s.trim();
    let s = if s.is_empty() { t!("shell-bar-next-event-untitled") } else { s.to_owned() };
    if s.chars().count() > MAX_TITLE_CHARS {
        let mut cut: String = s.chars().take(MAX_TITLE_CHARS - 1).collect();
        cut.push('…');
        cut
    } else {
        s
    }
}

/// Bar metni: "Başlık · 12 dk", bir saatten uzaksa "Başlık · 14:00", sürüyorsa "Başlık · bitiş 14:30".
pub fn pill_text(p: &Pick, now: i64) -> String {
    let title = short_title(&p.summary);
    if p.ongoing {
        return t!("shell-bar-next-event-ongoing", title = title, end = hhmm(p.end));
    }
    let minutes = (p.start - now + 59) / 60;
    if minutes < 1 {
        t!("shell-bar-next-event-now", title = title)
    } else if minutes < 60 {
        t!("shell-bar-next-event-in", title = title, minutes = minutes)
    } else {
        t!("shell-bar-next-event-at", title = title, time = hhmm(p.start))
    }
}

#[derive(Debug)]
pub struct NextEvent {
    pill: Pill,
    lead_minutes: u32,
    /// Yakın aralığın etkinlikleri (seçim her vuruşta bunlardan).
    candidates: Vec<OccurrenceEntry>,
    shown: Option<Pick>,
    text: String,
    soon: bool,
    tooltip: String,
}

impl NextEvent {
    pub fn new(cx: &WidgetCx) -> Self {
        Self {
            pill: Pill::new(cx, IconName::Event),
            lead_minutes: cx.settings.next_event.lead_minutes,
            candidates: Vec::new(),
            shown: None,
            text: String::new(),
            soon: false,
            tooltip: String::new(),
        }
    }

    fn visible(&self) -> bool {
        self.shown.is_some()
    }

    /// Seçimi `now`'a göre yeniler; görünüm değiştiyse `true`.
    fn refresh(&mut self, now: i64) -> bool {
        let shown = pick(&self.candidates, now, self.lead_minutes);
        let text = shown.as_ref().map(|p| pill_text(p, now)).unwrap_or_default();
        let soon = shown.as_ref().is_some_and(|p| !p.ongoing && p.start - now <= SOON_MINUTES * 60);
        if shown == self.shown && text == self.text && soon == self.soon {
            return false;
        }
        self.tooltip = shown
            .as_ref()
            .map(|p| {
                let title: String = p.summary.chars().filter(|c| !c.is_control()).take(200).collect();
                t!("shell-bar-next-event-tooltip", title = title, start = hhmm(p.start), end = hhmm(p.end))
            })
            .unwrap_or_default();
        self.pill.set_text(&text);
        self.shown = shown;
        self.text = text;
        self.soon = soon;
        true
    }
}

impl BarWidget for NextEvent {
    fn kind(&self) -> WidgetKind {
        WidgetKind::NextEvent
    }

    fn sync(&mut self, model: &ShellModel, _monitor: &str, _cx: &WidgetCx) -> bool {
        let now = Local::now();
        let (from, to) = near_range(now);
        let candidates: Vec<OccurrenceEntry> =
            model.calendar.iter().filter(|e| !e.all_day && e.end > from && e.start < to).take(MAX_CANDIDATES).cloned().collect();
        if candidates == self.candidates {
            return false;
        }
        self.candidates = candidates;
        self.refresh(now.timestamp())
    }

    fn set_time(&mut self, now: &DateTime<Local>, _cx: &WidgetCx) -> bool {
        self.refresh(now.timestamp())
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.pill.set_theme(cx);
        if self.lead_minutes != cx.settings.next_event.lead_minutes {
            self.lead_minutes = cx.settings.next_event.lead_minutes;
            self.refresh(Local::now().timestamp());
        }
    }

    fn measure(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        if self.visible() { self.pill.measure(text, cx) } else { 0.0 }
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, highlighted: bool) {
        if !self.visible() {
            return;
        }
        let c = &cx.theme.colors;
        if self.soon {
            self.pill.draw(painter, rect, cx, Some(c.primary), c.on_primary);
        } else {
            let fill = highlighted.then_some(c.surface_bright);
            self.pill.draw(painter, rect, cx, fill, if highlighted { c.primary } else { c.on_surface });
        }
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        self.visible() && self.pill.hover(rect, pointer, down, cx)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        (self.visible() && Pill::chip(rect, cx).contains(at)).then_some(Action::TogglePanel(PanelId::Clock))
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.pill.is_animating(now)
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        (self.visible() && Pill::chip(rect, cx).contains(p)).then_some(CursorShape::Pointer)
    }

    fn tooltip(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        (self.visible() && Pill::chip(rect, cx).contains(p)).then_some(BarTooltip { text: &self.tooltip, anchor: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(summary: &str, start: i64, end: i64) -> OccurrenceEntry {
        OccurrenceEntry { summary: summary.into(), start, end, status: "confirmed".into(), ..Default::default() }
    }

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn picks_nearest_upcoming_then_ongoing() {
        let a = ev("Ekip toplantısı", NOW + 12 * 60, NOW + 60 * 60);
        let b = ev("Öğle", NOW + 30 * 60, NOW + 90 * 60);
        let far = ev("Akşam", NOW + 3 * 3600, NOW + 4 * 3600);
        let ongoing = ev("Odak", NOW - 600, NOW + 1800);
        let all_day = OccurrenceEntry { all_day: true, ..ev("Bayram", NOW - 3600, NOW + 86_400) };
        let cancelled = OccurrenceEntry { status: "cancelled".into(), ..ev("İptal", NOW + 60, NOW + 600) };
        let list = [far.clone(), b, a.clone(), ongoing.clone(), all_day, cancelled];
        let p = pick(&list, NOW, 60).unwrap();
        assert_eq!((p.summary.as_str(), p.ongoing), ("Ekip toplantısı", false), "en yakın yaklaşan, iptal/tüm gün yok");
        // Öncü süre dışında: süren etkinlik.
        let p = pick(&[far.clone(), ongoing.clone()], NOW, 60).unwrap();
        assert_eq!((p.summary.as_str(), p.ongoing), ("Odak", true));
        assert_eq!(pick(std::slice::from_ref(&far), NOW, 60), None, "eşik dışında: gizli");
        assert!(pick(&[far], NOW, 240).is_some(), "eşik ayarı");
        assert_eq!(pick(&[ev("Bitti", NOW - 3600, NOW)], NOW, 60), None);
    }

    #[test]
    fn pill_texts() {
        crate::i18n::use_turkish_for_tests();
        let p = |start, end, ongoing| Pick { summary: "Ekip toplantısı".into(), start, end, ongoing };
        assert_eq!(pill_text(&p(NOW + 12 * 60, NOW + 3600, false), NOW), "Ekip toplantısı · 12 dk");
        assert_eq!(pill_text(&p(NOW + 30, NOW + 3600, false), NOW), "Ekip toplantısı · 1 dk");
        assert_eq!(pill_text(&p(NOW, NOW + 3600, false), NOW), "Ekip toplantısı · şimdi");
        assert_eq!(pill_text(&p(NOW + 90 * 60, NOW + 7200, false), NOW), format!("Ekip toplantısı · {}", hhmm(NOW + 90 * 60)));
        assert_eq!(pill_text(&p(NOW - 60, NOW + 1800, true), NOW), format!("Ekip toplantısı · bitiş {}", hhmm(NOW + 1800)));
        let long = Pick { summary: "Çok uzun bir başlığı olan çeyrek dönem planlama toplantısı".into(), ..p(NOW + 600, NOW + 3600, false) };
        assert!(pill_text(&long, NOW).starts_with("Çok uzun bir başlığı ol…"));
        let empty = Pick { summary: " \n".into(), ..p(NOW + 600, NOW + 3600, false) };
        assert_eq!(pill_text(&empty, NOW), "Adsız etkinlik · 10 dk");
    }

    #[test]
    fn near_range_covers_today_and_tomorrow() {
        let now = Local.with_ymd_and_hms(2026, 10, 16, 23, 30, 0).unwrap();
        let (from, to) = near_range(now);
        assert!(from <= now.timestamp() && now.timestamp() + 12 * 3600 < to);
        assert_eq!(to - from, 2 * 86_400);
    }
}
