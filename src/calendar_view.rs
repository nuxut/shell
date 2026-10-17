//! Saat popout'unun içeriği: büyük saat, uzun tarih, ay takvimi (`CalendarGrid`, etkinlik
//! noktalarıyla) ve seçili günün etkinlikleri. Veri `nuxut-calendard`'dan (model.calendar);
//! daemon yoksa takvim yine çalışır, yalnızca etkinlik gösterilmez.
//!
//! Tarih hesabı yerel saat ofsetiyle (`chrono::Local`); takvim ızgarası 6 hafta (42 gün)
//! gösterir. Aynı görünüm iki kipte kullanılır ([`CalendarMode`]): saat popout'u ve kenar
//! paneli "Takvim" kartı (`widgets::calendar_card`). Servis tek aralık sorgular; görünür tüm
//! takvimlerin aralıkları [`query_range`] ile birleştirilir (`docs/design/calendar-card.md`).

use std::time::Instant;

use chrono::{DateTime, Local, Offset, TimeZone, Timelike};
use nuxut::i18n::t;
use nuxut::locale::Locale;
use nuxut::ui::{CalendarEvent, CalendarGrid, Date, Label, MAX_EVENT_DOTS, TextRole, Ui, locale_from_env, style, text_style};
use nuxut::{Color, Corners, CursorShape, KeyEvent, Point, PointerEvent, Rect, RectStyle, TextSystem, Theme};
use nuxut_calendar_ipc::OccurrenceEntry;

use crate::services::clock;

/// Seçili günün listesinde en çok kaç etkinlik satırı (fazlası "+N daha").
pub const MAX_EVENT_ROWS: usize = 5;
/// Kart kipinde etkinlik satırı sayısı: kart, panelde başka kartlarla yer paylaşır.
pub const CARD_EVENT_ROWS: usize = 3;
/// Kart kipinde içerik (ızgara + liste) en çok bu genişlikte, kartta ortalı. Hücreler kare
/// olduğundan geniş bir panelde (üst/alt panel ekran boyu) ızgara yoksa ekran boyu uzardı.
pub const CARD_MAX_CONTENT_WIDTH: f32 = 320.0;
/// Birleşik sorgunun en geniş aralığı: bunu aşan birleşimde (görünümler birbirinden çok uzak
/// aylarda) yalnızca son gezinilen görünümün aralığı sorgulanır (servis yanıtı en çok
/// `MAX_ENTRIES` oluşum; çok geniş aralık yakın ayı kırpardı).
pub const MAX_QUERY_SPAN: i64 = 400 * 86_400;

/// Takvim görünümünün kullanıldığı yer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarMode {
    /// Saat popout'u: büyük saat + uzun tarih başlığı, [`MAX_EVENT_ROWS`] satır.
    Popout,
    /// Kenar paneli kartı: başlıksız (saat ve tarih bar'da ve saatin ipucunda), dar iç boşluk,
    /// [`CARD_EVENT_ROWS`] satır, içerik [`CARD_MAX_CONTENT_WIDTH`] ile sınırlı.
    Card,
}

/// Görünür takvimlerin aralıklarından servise gidecek tek sorgu: birleşimi kapsayan en küçük
/// aralık (`[min from, max to)`). Kapsayan aralık [`MAX_QUERY_SPAN`]'i aşarsa `focus` (son ay
/// değiştiren görünümün aralığı; görünürlerden biriyse) ya da ilk aralık. Görünür takvim yoksa
/// `None` (son sorgu korunur).
pub fn query_range(ranges: &[(i64, i64)], focus: Option<(i64, i64)>) -> Option<(i64, i64)> {
    let from = ranges.iter().map(|r| r.0).min()?;
    let to = ranges.iter().map(|r| r.1).max()?;
    if to - from <= MAX_QUERY_SPAN {
        return Some((from, to));
    }
    Some(focus.filter(|f| ranges.contains(f)).unwrap_or(ranges[0]))
}

/// Sistemin şu anki yerel saat ofseti (sn).
pub fn local_offset() -> i64 {
    i64::from(Local::now().offset().fix().local_minus_utc())
}

/// Bir günün yerel gece yarısının Unix zamanı.
fn day_start(d: Date, offset: i64) -> i64 {
    d.to_days() * 86_400 - offset
}

/// Ay ızgarasının gösterdiği 6 haftalık aralık `[from, to)` (Unix sn).
pub fn grid_range(year: i32, month: u8, first_weekday: u8, offset: i64) -> (i64, i64) {
    let first = Date::new(year, month, 1);
    let lead = (i64::from(first.weekday()) - i64::from(first_weekday)).rem_euclid(7);
    let start = first.add_days(-lead);
    let from = day_start(start, offset);
    (from, from + 42 * 86_400)
}

/// `YYYY-MM-DD` → Date.
fn parse_date(s: &str) -> Option<Date> {
    let mut it = s.split('-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    (it.next().is_none() && (1..=12).contains(&m) && (1..=31).contains(&d)).then(|| Date::new(y, m, d))
}

/// Etkinliğin kapsadığı yerel günler (`[ilk, son]`, en çok 400 gün).
fn entry_days(e: &OccurrenceEntry, offset: i64) -> (Date, Date) {
    if e.all_day
        && let Some(a) = parse_date(&e.start_date)
    {
        // `end_date` hariç; yoksa tek gün.
        let b = parse_date(&e.end_date).map_or(a, |b| b.add_days(-1));
        return (a, if b.to_days() < a.to_days() { a } else { b });
    }
    let a = Date::from_unix_seconds(e.start, offset);
    // Bitiş anı hariç (00:00'da biten etkinlik ertesi güne taşmaz).
    let b = Date::from_unix_seconds((e.end - 1).max(e.start), offset);
    (a, b)
}

/// Daemon'dan gelen takvim rengi (`#rrggbb`, servis doğruladı). Bu bir veri; koda gömülü
/// renk değil.
fn entry_color(e: &OccurrenceEntry) -> Option<Color> {
    Color::from_hex(&e.color).ok() // renk-verisi: takvim rengi daemon'dan gelir

}

/// Gündem satırının takvim noktası (takvim rengi, yoksa `primary`): metin dışı işaret, yüksek
/// karşıtlıkta yüzeylere ≥ 3 (framework `style::faded_mark`, gündem şeridiyle aynı; t124).
fn event_dot(theme: &Theme, color: Option<Color>) -> Color {
    style::faded_mark(theme, color.unwrap_or(theme.colors.primary), 1.0, &[])
}

/// Ay ızgarası noktaları: gün başına en çok `MAX_EVENT_DOTS` renk (`None` = tema vurgusu).
pub fn day_dots(entries: &[OccurrenceEntry], offset: i64) -> Vec<(Date, Vec<Option<Color>>)> {
    let mut map: std::collections::BTreeMap<i64, Vec<Option<Color>>> = std::collections::BTreeMap::new();
    for e in entries {
        let (a, b) = entry_days(e, offset);
        let span = (b.to_days() - a.to_days()).clamp(0, 400);
        for i in 0..=span {
            let dots = map.entry(a.to_days() + i).or_default();
            if dots.len() < MAX_EVENT_DOTS {
                dots.push(entry_color(e));
            }
        }
    }
    map.into_iter().map(|(d, dots)| (Date::from_days(d), dots)).collect()
}

/// Seçili günün etkinlikleri (başlangıca göre; tüm gün olanlar önce).
pub fn day_events(entries: &[OccurrenceEntry], day: Date, offset: i64) -> Vec<&OccurrenceEntry> {
    let mut out: Vec<&OccurrenceEntry> = entries
        .iter()
        .filter(|e| {
            let (a, b) = entry_days(e, offset);
            (a.to_days()..=b.to_days()).contains(&day.to_days())
        })
        .collect();
    out.sort_by_key(|e| (!e.all_day, e.start));
    out
}

/// Satırdaki saat metni: "Tüm gün" ya da "14:30" / "2:30 PM" (`[locale] hour_cycle`).
pub fn event_time(e: &OccurrenceEntry, offset: i64, locale: &Locale) -> String {
    if e.all_day {
        return t!("shell-calendar-all-day");
    }
    let local = e.start + offset;
    let mins = local.rem_euclid(86_400) / 60;
    // 0..1440: saat 0..=23, dakika 0..=59 (u32'ye sığar).
    clock::format_hm(locale, (mins / 60) as u32, (mins % 60) as u32)
}

#[derive(Debug)]
struct EventRow {
    time: Label,
    summary: Label,
    color: Option<Color>,
}

#[derive(Debug)]
pub struct CalendarView {
    mode: CalendarMode,
    /// Saat/tarih locale'i (`[locale] formats` + `hour_cycle`); `relocalize`'da yenilenir.
    locale: Locale,
    big_time: Label,
    long_date: Label,
    grid: CalendarGrid,
    today: Date,
    selected: Date,
    entries: Vec<OccurrenceEntry>,
    available: bool,
    rows: Vec<EventRow>,
    more: Label,
    empty: Label,
}

impl CalendarView {
    pub fn new(theme: &Theme, mode: CalendarMode) -> Self {
        let now = Local::now();
        let today = Date::from_unix_seconds(now.timestamp(), local_offset());
        let mut s = Self {
            mode,
            locale: clock::time_locale(),
            big_time: Label::new("", big_time_style(theme)),
            long_date: Label::new("", text_style(theme, TextRole::Title)),
            grid: CalendarGrid::new(theme, today, &locale_from_env()),
            today,
            selected: today,
            entries: Vec::new(),
            available: false,
            rows: Vec::new(),
            more: Label::new("", text_style(theme, TextRole::Caption)),
            empty: Label::new("", text_style(theme, TextRole::Caption)),
        };
        s.set_time(&now);
        s.rebuild_rows(theme);
        s
    }

    pub fn mode(&self) -> CalendarMode {
        self.mode
    }

    fn inset(&self, theme: &Theme) -> f32 {
        match self.mode {
            CalendarMode::Popout => theme.space(6),
            CalendarMode::Card => theme.space(3),
        }
    }

    fn row_h(theme: &Theme) -> f32 {
        theme.space(7)
    }

    fn max_rows(&self) -> usize {
        match self.mode {
            CalendarMode::Popout => MAX_EVENT_ROWS,
            CalendarMode::Card => CARD_EVENT_ROWS,
        }
    }

    /// Izgara ile liste arası.
    fn list_gap(&self, theme: &Theme) -> f32 {
        match self.mode {
            CalendarMode::Popout => theme.space(4),
            CalendarMode::Card => theme.space(2),
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.big_time.set_style(big_time_style(theme));
        self.long_date.set_style(text_style(theme, TextRole::Title));
        self.grid.set_theme(theme, now);
        self.more.set_style(text_style(theme, TextRole::Caption));
        self.empty.set_style(text_style(theme, TextRole::Caption));
        self.rebuild_rows(theme);
    }

    /// Dil değişti: etkinlik satırları ve boş/fazla metinleri yeni katalogla, saat ve uzun
    /// tarih son ana göre yeniden kurulur. Seçili gün ve gösterilen ay korunur.
    pub fn relocalize(&mut self, theme: &Theme) {
        self.locale = clock::time_locale();
        // Saat dilimi de değişmiş olabilir (aynı yol): gün noktaları yeni ofsetle.
        self.grid.set_events(day_dots(&self.entries, local_offset()));
        self.rebuild_rows(theme);
        self.set_time(&Local::now());
    }

    /// Saat ilerledi. Görünüm değiştiyse `true` (kart kipinde saat/tarih çizilmez: yalnızca
    /// gün dönümü).
    pub fn set_time<Tz: TimeZone>(&mut self, now: &DateTime<Tz>) -> bool
    where
        Tz::Offset: std::fmt::Display,
    {
        let mut changed = false;
        if self.mode == CalendarMode::Popout {
            changed |= self.big_time.set_text(&clock::format_hm(&self.locale, now.hour(), now.minute()));
            changed |= self.long_date.set_text(&clock::long_date(now, &self.locale));
        }
        let today = Date::from_unix_seconds(now.timestamp(), local_offset());
        if today != self.today {
            // Gün döndü: seçim bugünü izliyorsa o da ilerler.
            if self.selected == self.today {
                self.selected = today;
            }
            self.today = today;
            self.grid.set_today(today);
            changed = true;
        }
        changed
    }

    /// Takvim verisi değişti (model). Değiştiyse `true`.
    pub fn set_entries(&mut self, theme: &Theme, entries: &[OccurrenceEntry], available: bool) -> bool {
        if self.available == available && self.entries == entries {
            return false;
        }
        self.available = available;
        self.entries = entries.to_vec();
        self.grid.set_events(day_dots(&self.entries, local_offset()));
        self.rebuild_rows(theme);
        true
    }

    fn rebuild_rows(&mut self, theme: &Theme) {
        let offset = local_offset();
        let events = day_events(&self.entries, self.selected, offset);
        let max = self.max_rows();
        self.rows = events
            .iter()
            .take(max)
            .map(|e| EventRow {
                time: Label::new(event_time(e, offset, &self.locale), text_style(theme, TextRole::Caption).with_tabular_numbers()),
                // Tek satır, taşan kısım "…" (genişlik çizimde verilir).
                summary: Label::new(e.summary.clone(), text_style(theme, TextRole::Body).with_max_lines(1)),
                color: entry_color(e),
            })
            .collect();
        let extra = events.len().saturating_sub(max);
        self.more.set_text(&if extra > 0 { t!("shell-calendar-more-events", count = extra) } else { String::new() });
        let empty = if !self.available {
            t!("shell-calendar-service-unavailable")
        } else if self.selected == self.today {
            t!("shell-calendar-empty-today")
        } else {
            t!("shell-calendar-empty-day")
        };
        self.empty.set_text(&empty);
    }

    /// Izgaranın şu an gösterdiği ayın sorgulanacak aralığı.
    pub fn requested_range(&self) -> (i64, i64) {
        let (y, m) = self.grid.month();
        grid_range(y, m, self.grid.first_weekday(), local_offset())
    }

    /// İçerik sütunu: iç boşluk düşülmüş; kart kipinde [`CARD_MAX_CONTENT_WIDTH`] ile sınırlı
    /// ve ortalı.
    fn content(&self, theme: &Theme, area: Rect) -> Rect {
        let inset = self.inset(theme);
        let mut w = (area.width - 2.0 * inset).max(0.0);
        if self.mode == CalendarMode::Card {
            w = w.min(CARD_MAX_CONTENT_WIDTH);
        }
        let x = area.x + ((area.width - w) * 0.5).max(0.0);
        Rect::new(x, area.y + inset, w, (area.height - 2.0 * inset).max(0.0))
    }

    /// Izgaranın üstündeki başlığın (büyük saat + uzun tarih) yüksekliği; kart kipinde 0.
    fn header_height(&self, text: &mut TextSystem, theme: &Theme) -> f32 {
        match self.mode {
            CalendarMode::Popout => {
                let t = self.big_time.clone().measure(text).height;
                let d = self.long_date.clone().measure(text).height;
                t + theme.space(1) + d + theme.space(4)
            }
            CalendarMode::Card => 0.0,
        }
    }

    /// İçerik yüksekliği (verilen genişlikte): popout'un ve kartın boyu. Liste her zaman
    /// en çok satır kadar yer ayırır (gün değiştirince boy oynamaz).
    pub fn preferred_height(&mut self, text: &mut TextSystem, theme: &Theme, width: f32) -> f32 {
        let inset = self.inset(theme);
        let content = self.content(theme, Rect::new(0.0, 0.0, width, 0.0));
        let header = self.header_height(text, theme);
        let grid = self.grid.preferred_size(theme, content.width).height;
        let list = Self::row_h(theme) * self.max_rows() as f32 + self.more.measure(text).height;
        inset + header + grid + self.list_gap(theme) + list + inset
    }

    /// Ögelerin dikdörtgenleri: (başlık başlangıcı, ızgara, liste).
    fn layout(&self, text: &mut TextSystem, theme: &Theme, area: Rect) -> (Point, Rect, Rect) {
        let content = self.content(theme, area);
        let header = Point::new(content.x, content.y);
        let gy = header.y + self.header_height(text, theme);
        let gh = self.grid.preferred_size(theme, content.width).height;
        let grid = Rect::new(content.x, gy, content.width, gh);
        let ly = grid.bottom() + self.list_gap(theme);
        let list = Rect::new(content.x, ly, content.width, (content.bottom() - ly).max(0.0));
        (header, grid, list)
    }

    /// İşaretçi: ızgaraya gider. Ay değişirse sorgu aralığı döner.
    pub fn pointer(
        &mut self,
        text: &mut TextSystem,
        theme: &Theme,
        area: Rect,
        event: &PointerEvent,
        now: Instant,
    ) -> (bool, Option<(i64, i64)>) {
        let (_, grid, _) = self.layout(text, theme, area);
        self.grid.set_rect(theme, grid);
        let r = self.grid.pointer(theme, event, now);
        self.handle(theme, r.repaint, r.event)
    }

    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> (bool, Option<(i64, i64)>) {
        let r = self.grid.key(theme, event, now);
        // Oklar/PageUp/PageDown ızgaranın seçimini taşır ama yalnızca Enter `DaySelected` verir:
        // liste vurgulanan günü izlesin (yoksa ızgara 13 Kasım'ı, liste 13 Ekim'i gösterir).
        let moved = self.grid.selected().filter(|d| *d != self.selected);
        if let Some(d) = moved {
            self.selected = d;
            self.rebuild_rows(theme);
        }
        let (redraw, range) = self.handle(theme, r.repaint, r.event);
        (redraw || moved.is_some(), range)
    }

    fn handle(&mut self, theme: &Theme, repaint: bool, event: Option<CalendarEvent>) -> (bool, Option<(i64, i64)>) {
        match event {
            Some(CalendarEvent::DaySelected(d)) => {
                self.selected = d;
                self.rebuild_rows(theme);
                (true, None)
            }
            Some(CalendarEvent::MonthChanged { year, month }) => {
                (true, Some(grid_range(year, month, self.grid.first_weekday(), local_offset())))
            }
            None => (repaint, None),
        }
    }


    /// Ekran okuyucu düğümleri (ADR-0011) `tree`'nin şu anki ebeveyni altına: popout'ta saat ve
    /// bugünün tarihi, seçili gün (ok tuşlarıyla değişir) ve o günün etkinlikleri ("10:00
    /// Toplantı"). Etkinlik grubu canlı bölge: gün değişince Orca yeni listeyi okur. Ay
    /// ızgarasının hücreleri tek tek ağaçta değil (seçili gün ve etkinlikler yeterli; ızgara
    /// klavyeyle gezilir).
    pub fn access(&self, tree: &mut nuxut::access::AccessTree, area: Rect) {
        use nuxut::access::{Live, Role};
        if self.mode == CalendarMode::Popout {
            let now = [self.big_time.text(), self.long_date.text()].join(", ");
            tree.push("now", Role::Label).value(&now).bounds(area);
        }
        let selected = chrono::NaiveDate::from_ymd_opt(self.selected.year, self.selected.month.into(), self.selected.day.into())
            .map(|d| clock::long_date(&d, &self.locale))
            .unwrap_or_default();
        let day = tree.push("day", Role::Group).label(&selected).bounds(area).live(Live::Polite).id();
        tree.within(day, |tree| {
            if self.rows.is_empty() {
                tree.push("empty", Role::Label).value(self.empty.text()).bounds(area);
                return;
            }
            for (i, row) in self.rows.iter().enumerate() {
                let text = [row.time.text(), row.summary.text()].join(" ");
                tree.push(("event", i), Role::Label).value(&text).bounds(area);
            }
            if !self.more.text().is_empty() {
                tree.push("more", Role::Label).value(self.more.text()).bounds(area);
            }
        });
    }
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.grid.cursor(p)
    }

    pub fn draw(&mut self, ui: &mut Ui, area: Rect, theme: &Theme) {
        let colors = &theme.colors;
        let (header, grid, list) = self.layout(ui.painter.text_system(), theme, area);

        if self.mode == CalendarMode::Popout {
            let t = self.big_time.measure_in(ui.painter);
            self.big_time.draw(ui.painter, header, ui.fade(colors.on_surface));
            let date_at = Point::new(header.x, header.y + t.height + theme.space(1));
            self.long_date.draw(ui.painter, date_at, ui.fade(colors.primary));
        }

        self.grid.set_rect(theme, grid);
        self.grid.draw(ui);

        let row_h = Self::row_h(theme);
        if self.rows.is_empty() {
            self.empty.set_max_width(Some(list.width));
            let s = self.empty.measure_in(ui.painter);
            self.empty.draw(ui.painter, Point::new(list.x, list.y + (row_h - s.height) * 0.5), ui.fade(colors.on_surface_variant));
            return;
        }
        let dot = theme.space(2);
        let tx = list.x + dot + theme.space(3);
        // Saat sütunu en geniş saat metni kadar ("Tüm gün" / "All day" dar sabit sütuna
        // sığmayabilir), en az popout'un eski sütunu.
        let time_w = self.rows.iter_mut().map(|r| r.time.measure_in(ui.painter).width).fold(0.0_f32, f32::max);
        let sx = tx + (time_w + theme.space(3)).max(theme.space(14));
        let sw = (list.right() - sx).max(0.0);
        for (i, row) in self.rows.iter_mut().enumerate() {
            let y = list.y + i as f32 * row_h;
            if y + row_h > list.bottom() + 0.5 {
                break;
            }
            let cy = y + row_h * 0.5;
            let color = event_dot(theme, row.color);
            ui.painter.rect(Rect::new(list.x, cy - dot * 0.5, dot, dot), &RectStyle::filled(ui.fade(color), Corners::uniform(dot * 0.5)));
            let ts = row.time.measure_in(ui.painter);
            row.time.draw(ui.painter, Point::new(tx, cy - ts.height * 0.5), ui.fade(colors.on_surface_variant));
            row.summary.set_max_width(Some(sw));
            let ss = row.summary.measure_in(ui.painter);
            row.summary.draw(ui.painter, Point::new(sx, cy - ss.height * 0.5), ui.fade(colors.on_surface));
        }
        let more_y = list.y + self.rows.len() as f32 * row_h;
        self.more.set_max_width(Some((list.right() - tx).max(0.0)));
        self.more.draw(ui.painter, Point::new(tx, more_y), ui.fade(colors.on_surface_variant));
    }
}

fn big_time_style(theme: &Theme) -> nuxut::TextStyle {
    // Aile ve ağırlık temanın `display` rolünden; yalnızca boyut büyür. Saniyeler değişirken
    // rakamlar kaymasın diye eş genişlik (tnum).
    let mut s = text_style(theme, TextRole::Display).with_tabular_numbers();
    s.size = (theme.typography.size * 3.6).round();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed(start: i64, end: i64, summary: &str) -> OccurrenceEntry {
        OccurrenceEntry { start, end, summary: summary.into(), ..Default::default() }
    }

    fn all_day(start: &str, end: &str) -> OccurrenceEntry {
        OccurrenceEntry { all_day: true, start_date: start.into(), end_date: end.into(), ..Default::default() }
    }

    #[test]
    fn grid_range_covers_six_weeks_from_first_weekday() {
        // Ekim 2026: 1 Ekim Perşembe; hafta Pazartesi başlar → 28 Eylül.
        let (from, to) = grid_range(2026, 10, 0, 0);
        assert_eq!(Date::from_unix_seconds(from, 0), Date::new(2026, 9, 28));
        assert_eq!(to - from, 42 * 86_400);
        // Pazar başlangıçlı yerel ayar → 27 Eylül; +3 saat ofset gece yarısını kaydırır.
        let (from, _) = grid_range(2026, 10, 6, 3 * 3600);
        assert_eq!(Date::from_unix_seconds(from, 3 * 3600), Date::new(2026, 9, 27));
        assert_eq!((from + 3 * 3600) % 86_400, 0);
    }

    #[test]
    fn dots_span_multi_day_and_cap_per_day() {
        let day = Date::new(2026, 10, 4).to_days() * 86_400;
        let mut entries = vec![timed(day + 3600, day + 2 * 86_400 + 3600, "üç gün")];
        for i in 0..5 {
            entries.push(timed(day + i * 60, day + i * 60 + 60, "kısa"));
        }
        entries.push(all_day("2026-10-10", "2026-10-12"));
        let dots = day_dots(&entries, 0);
        let get = |d: Date| dots.iter().find(|(x, _)| *x == d).map(|(_, v)| v.len()).unwrap_or(0);
        assert_eq!(get(Date::new(2026, 10, 4)), MAX_EVENT_DOTS);
        assert_eq!(get(Date::new(2026, 10, 5)), 1);
        assert_eq!(get(Date::new(2026, 10, 6)), 1);
        assert_eq!(get(Date::new(2026, 10, 7)), 0);
        assert_eq!(get(Date::new(2026, 10, 10)), 1);
        assert_eq!(get(Date::new(2026, 10, 11)), 1);
        assert_eq!(get(Date::new(2026, 10, 12)), 0, "end_date hariç");
    }

    #[test]
    fn midnight_end_does_not_spill_into_next_day() {
        let day = Date::new(2026, 10, 4).to_days() * 86_400;
        let e = timed(day + 20 * 3600, day + 86_400, "gece");
        assert_eq!(day_events(std::slice::from_ref(&e), Date::new(2026, 10, 5), 0).len(), 0);
        assert_eq!(day_events(&[e], Date::new(2026, 10, 4), 0).len(), 1);
    }

    #[test]
    fn day_events_sorted_all_day_first_and_time_labels() {
        crate::i18n::use_turkish_for_tests();
        let day = Date::new(2026, 10, 4).to_days() * 86_400;
        let entries = vec![timed(day + 14 * 3600 + 30 * 60, day + 15 * 3600, "b"), all_day("2026-10-04", ""), timed(day + 9 * 3600, day + 10 * 3600, "a")];
        let ev = day_events(&entries, Date::new(2026, 10, 4), 0);
        assert!(ev[0].all_day);
        assert_eq!(ev[1].summary, "a");
        let tr = Locale::parse("tr_TR").unwrap();
        assert_eq!(event_time(ev[0], 0, &tr), "Tüm gün");
        assert_eq!(event_time(ev[2], 0, &tr), "14:30");
        assert_eq!(event_time(ev[2], 3 * 3600, &tr), "17:30");
        // t119: etkinlik saati [locale] hour_cycle'a uyar (auto en_US 12 s, h23 zorlanınca 24 s).
        let en = Locale::parse("en_US").unwrap();
        assert_eq!(event_time(ev[2], 0, &en), "2:30 PM");
        assert_eq!(event_time(ev[1], 0, &en), "9:00 AM");
        assert_eq!(event_time(ev[2], 0, &en.with_12_hour(Some(false))), "14:30");
        let [_, pm] = tr.day_periods();
        assert_eq!(event_time(ev[2], 0, &tr.with_12_hour(Some(true))), format!("2:30 {pm}"));
    }

    #[test]
    fn query_range_covers_all_visible_views() {
        let oct = grid_range(2026, 10, 0, 0);
        let nov = grid_range(2026, 11, 0, 0);
        let mar = grid_range(2027, 3, 0, 0);
        assert_eq!(query_range(&[], None), None, "görünür takvim yok: sorgu yok");
        assert_eq!(query_range(&[oct], None), Some(oct));
        // Popout Ekim, kart Kasım (ya da tersi): ikisini de kapsayan tek aralık.
        let both = query_range(&[oct, nov], Some(nov)).unwrap();
        assert_eq!(both, (oct.0, nov.1));
        assert_eq!(query_range(&[nov, oct], None), Some(both), "sıra önemsiz");
        assert_eq!(query_range(&[oct, oct], None), Some(oct), "aynı ay: tek ayın aralığı");
        let wide = query_range(&[oct, mar], None).unwrap();
        assert!(wide.0 <= oct.0 && wide.1 >= mar.1);
        // Her görünümün günleri birleşik sorgunun içinde: etkinlikleri ikisine de gelir.
        for (from, to) in [oct, nov, mar] {
            let q = query_range(&[oct, nov, mar], None).unwrap();
            assert!(q.0 <= from && to <= q.1);
        }
    }

    #[test]
    fn query_range_too_wide_prefers_focused_view() {
        let a = grid_range(2026, 10, 0, 0);
        let b = grid_range(2028, 1, 0, 0);
        assert!(b.1 - a.0 > MAX_QUERY_SPAN);
        assert_eq!(query_range(&[a, b], Some(b)), Some(b), "son gezinilen görünüm");
        assert_eq!(query_range(&[a, b], Some(a)), Some(a));
        // Odak artık görünür değil (kapanan popout): ilk görünür.
        let gone = grid_range(2030, 5, 0, 0);
        assert_eq!(query_range(&[a, b], Some(gone)), Some(a));
        assert_eq!(query_range(&[a, b], None), Some(a));
    }

    #[test]
    fn card_mode_is_compact_and_capped() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut popout = CalendarView::new(&theme, CalendarMode::Popout);
        let mut card = CalendarView::new(&theme, CalendarMode::Card);
        let w = 360.0;
        // Kart: başlık yok, daha az satır → aynı genişlikte popout'tan kısa.
        assert!(card.preferred_height(&mut text, &theme, w) < popout.preferred_height(&mut text, &theme, w));
        // Geniş panelde (üst panel, ekran boyu) ızgara büyümez; dar panelde küçülür.
        let wide = card.preferred_height(&mut text, &theme, 1800.0);
        let capped = card.preferred_height(&mut text, &theme, CARD_MAX_CONTENT_WIDTH + 2.0 * theme.space(3));
        assert_eq!(wide, capped);
        assert!(card.preferred_height(&mut text, &theme, 220.0) < capped);
        // İçerik kartta ortalı.
        let c = card.content(&theme, Rect::new(100.0, 0.0, 1000.0, 500.0));
        assert_eq!(c.width, CARD_MAX_CONTENT_WIDTH);
        assert!((c.center().x - 600.0).abs() < 0.5);
        // Gün dönümü dışında kart saat ilerleyince yeniden çizim istemez.
        let t = |d: u32, h: u32, m: u32| Local.with_ymd_and_hms(2031, 1, d, h, m, 0).unwrap();
        card.set_time(&t(5, 10, 0));
        assert!(!card.set_time(&t(5, 10, 1)), "kart saati çizmez");
        assert!(card.set_time(&t(6, 0, 0)), "gün dönümü");
        popout.set_time(&t(5, 10, 0));
        assert!(popout.set_time(&t(5, 10, 1)), "popout büyük saati");
        assert_eq!(card.mode(), CalendarMode::Card);
    }

    #[test]
    fn card_rows_follow_card_limit() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let mut card = CalendarView::new(&theme, CalendarMode::Card);
        let today = card.today;
        let day = today.to_days() * 86_400 - local_offset();
        let entries: Vec<_> = (0..6).map(|i| timed(day + 3600 * (i + 1), day + 3600 * (i + 1) + 600, "toplantı")).collect();
        assert!(card.set_entries(&theme, &entries, true));
        assert_eq!(card.rows.len(), CARD_EVENT_ROWS);
        assert_eq!(card.more.text(), "+3 etkinlik daha");
        let mut popout = CalendarView::new(&theme, CalendarMode::Popout);
        popout.set_entries(&theme, &entries, true);
        assert_eq!(popout.rows.len(), MAX_EVENT_ROWS);
        assert!(!card.set_entries(&theme, &entries, true), "aynı veri: değişiklik yok");
        assert!(card.set_entries(&theme, &[], false));
        assert_eq!(card.empty.text(), "Takvim servisi çalışmıyor");
    }

    #[test]
    fn keyboard_navigation_moves_event_list_selection() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let mut v = CalendarView::new(&theme, CalendarMode::Card);
        let tomorrow = v.today.add_days(1);
        let start = tomorrow.to_days() * 86_400 - local_offset() + 9 * 3600;
        v.set_entries(&theme, &[timed(start, start + 3600, "yarın")], true);
        assert!(v.rows.is_empty(), "bugün etkinlik yok");
        let right = KeyEvent::new(nuxut::Key::Right, nuxut::KeyState::Pressed, None, nuxut::Modifiers::default());
        let (redraw, _) = v.key(&theme, &right, Instant::now());
        assert!(redraw);
        assert_eq!(v.selected, tomorrow);
        assert_eq!(v.rows.len(), 1, "liste ok tuşuyla seçilen günü izler");
        assert_eq!(v.rows[0].summary.text(), "yarın");
    }

    #[test]
    fn parse_date_rejects_garbage() {
        assert_eq!(parse_date("2026-10-04"), Some(Date::new(2026, 10, 4)));
        for bad in ["", "2026-13-01", "2026-10", "x-y-z", "2026-10-04-1"] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn event_dot_standard_unchanged_high_contrast_at_least_three() {
        let theme = Theme::default();
        let pale = Color::rgb(0.30, 0.32, 0.36); // renk-verisi: soluk takvim rengi (test girdisi)
        assert_eq!(event_dot(&theme, None), theme.colors.primary);
        assert_eq!(event_dot(&theme, Some(pale)), pale);
        for mode in ["dark", "light"] {
            let high = Theme::parse(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"high\"\n")).unwrap().0;
            let dot = event_dot(&high, Some(pale));
            let ratio = style::worst_over(dot, &style::backdrops(&high, &[]));
            assert!(ratio >= nuxut::theme::contrast::NON_TEXT_MIN, "{mode}: {ratio}");
        }
    }
}
