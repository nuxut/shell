//! Başlatıcının emoji kipi içeriği (`docs/design/emoji.md`): framework [`GlyphGrid`] + altında
//! seçili emojinin adını gösteren satır. [`crate::launcher_view::LauncherView`] emoji kipinde
//! arama alanının altını buna bırakır.
//!
//! - **Boş sorgu:** bölümler: "Sık kullanılanlar" (kullanım kaydı olanlar, en çok
//!   [`RECENT_MAX`]) ve Unicode grupları (veri sırası). **Sorgu:** tek düz sonuç kümesi
//!   (en çok [`MAX_EMOJI_RESULTS`], puan sırası; [`EmojiSearcher`]).
//! - **Ad satırı:** seçili emoji büyük ([`Icon::glyph`]) + "etkin dildeki ad · diğer dildeki ad";
//!   seçim yokken kip ikonu ([`IconName::Emoji`]).
//! - **Font kapsaması:** ilk çizimde kurulu renkli emoji ailesiyle (fontconfig'in yedeği olan
//!   aileler, [`EMOJI_FAMILIES`]) her emoji bir kez şekillendirilir; tek glife inmeyen (font o
//!   diziyi bilmiyor: kutu ya da parçalı ZWJ çizilirdi) emoji gösterilmez. Aile yoksa süzme yok.
//! - **Klavye:** ↑ ↓ ← → PgUp PgDn Home End Enter ızgaraya; harfler, Space, Backspace arama
//!   alanına (ızgara tüketmez).

use std::time::Instant;

use nuxut::i18n::t;
use nuxut::ui::{
    GlyphColumns, GlyphGrid, GlyphGridEvent, GlyphItem, GlyphSection, Icon, IconName, IconView, Label, Ui, text_style,
    TextRole,
};
use nuxut::{CursorShape, Key, KeyEvent, KeyState, Point, PointerEvent, Rect, TextSystem, Theme};

use crate::services::launcher::emoji::{self, Group, Lang};
use crate::services::launcher::{EmojiSearcher, Usage};

/// Sorguda gösterilen en çok emoji.
pub const MAX_EMOJI_RESULTS: usize = 240;
/// "Sık kullanılanlar" bölümündeki en çok emoji.
pub const RECENT_MAX: usize = 24;
/// Kapsama denetiminde sırayla denenen renkli emoji aileleri (ilk kurulu olan).
pub const EMOJI_FAMILIES: [&str; 5] = ["Noto Color Emoji", "Twemoji", "JoyPixels", "Apple Color Emoji", "Segoe UI Emoji"];

/// Kurulu ilk emoji ailesinde her verinin tek glife inip inmediği (`emoji::all` sırasıyla).
/// Aile yoksa `None` (süzme yapılmaz).
pub fn coverage(text: &mut TextSystem) -> Option<(&'static str, Vec<bool>)> {
    let family = EMOJI_FAMILIES.into_iter().find(|f| text.has_family(f))?;
    let covered = emoji::all().iter().map(|e| text.glyphs_in_family(family, e.emoji) == Some(1)).collect();
    Some((family, covered))
}

/// Kapsama denetlenmediyse ya da emoji ailesi yoksa her emoji gösterilir.
fn is_covered(coverage: &Option<Option<Vec<bool>>>, i: usize) -> bool {
    match coverage {
        Some(Some(c)) => c.get(i).copied().unwrap_or(true),
        _ => true,
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[derive(Debug)]
pub struct EmojiView {
    grid: GlyphGrid,
    searcher: EmojiSearcher,
    /// Izgaranın düz indeksi → `emoji::all` indeksi.
    rows: Vec<u16>,
    footer_icon: IconView,
    footer: Label,
    /// `None`: henüz denetlenmedi; `Some(None)`: emoji ailesi yok (süzme yok).
    coverage: Option<Option<Vec<bool>>>,
    query: String,
    /// Listenin kurulduğu dil; `None`: henüz kurulmadı ya da yer unutuldu ([`Self::forget_position`]).
    lang: Option<Lang>,
}

impl EmojiView {
    pub fn new(theme: &Theme) -> Self {
        let mut grid = GlyphGrid::new(theme);
        grid.set_columns(theme, GlyphColumns::Auto, Instant::now());
        Self {
            grid,
            searcher: EmojiSearcher::new(),
            rows: Vec::new(),
            footer_icon: IconView::with_role(theme, Icon::named(IconName::Emoji), TextRole::IconLarge),
            footer: Label::new("", text_style(theme, TextRole::Body)),
            coverage: None,
            query: String::new(),
            lang: None,
        }
    }

    /// Ad satırının yüksekliği.
    pub fn footer_height(theme: &Theme) -> f32 {
        theme.space(12)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Düz indeksteki emoji.
    pub fn emoji_at(&self, i: usize) -> Option<&'static str> {
        let idx = *self.rows.get(i)? as usize;
        emoji::all().get(idx).map(|e| e.emoji)
    }

    pub fn selected_emoji(&self) -> Option<&'static str> {
        self.emoji_at(self.grid.selected()?)
    }

    /// Kapsamayı (testte) elle verir.
    #[cfg(test)]
    pub fn set_coverage(&mut self, coverage: Option<Vec<bool>>) {
        self.coverage = Some(coverage);
    }

    /// Bir sonraki [`Self::refresh`] yeni liste sayar (panel açılışı, kip değişimi): seçim ve
    /// kaydırma başa döner.
    pub fn forget_position(&mut self) {
        self.lang = None;
    }

    /// Listeyi sorgu ve kullanıma göre kurar. Yeni sorguda ilk öğe seçilir. Sorgu aynıysa
    /// (dil değişimi, kullanım güncellemesi, kapsama denetimi) kullanıcının yeri korunur: veri ve
    /// dil değişmediyse ızgaraya dokunulmaz (seçim ve kaydırma yerinde); değiştiyse yeniden
    /// kurulur (`GlyphGrid::replace_sections`: kaydırma ofseti korunur) ve aynı emoji yeniden
    /// seçilir. Döner: ızgara yeniden kuruldu mu.
    pub fn refresh(&mut self, theme: &Theme, usage: &Usage, query: &str, now: Instant) -> bool {
        let same_query = self.lang.is_some() && self.query == query;
        let primary = primary_lang();
        let all = emoji::all();
        let coverage = &self.coverage;
        let hits = self.searcher.search(usage, query, primary, now_unix());
        let mut rows: Vec<u16> = Vec::new();
        let mut sections = Vec::with_capacity(Group::ALL.len() + 1);
        if query.trim().is_empty() {
            // Kullanım kaydı olanlar puan > 0 ile başta gelir (frecency sırası).
            let recent: Vec<u16> = hits
                .iter()
                .take_while(|h| h.score > 0)
                .map(|h| h.index)
                .filter(|&i| is_covered(coverage, i as usize))
                .take(RECENT_MAX)
                .collect();
            if !recent.is_empty() {
                rows.extend(&recent);
                sections.push((t!("shell-launcher-emoji-recent"), recent.len()));
            }
            for g in Group::ALL {
                let start = rows.len();
                rows.extend((0..all.len()).filter(|&i| all[i].group == g && is_covered(coverage, i)).map(|i| i as u16));
                sections.push((g.label(), rows.len() - start));
            }
        } else {
            rows.extend(
                hits.iter().map(|h| h.index).filter(|&i| is_covered(coverage, i as usize)).take(MAX_EMOJI_RESULTS),
            );
            sections.push((String::new(), rows.len()));
        }
        if same_query && self.lang == Some(primary) && rows == self.rows {
            return false;
        }
        let previous = same_query.then(|| self.grid.selected().map(|i| (i, self.emoji_at(i)))).flatten();
        let item = |i: u16| GlyphItem::new(all[i as usize].emoji, all[i as usize].name(primary));
        let mut members = rows.iter().copied();
        let built: Vec<GlyphSection> = sections
            .into_iter()
            .map(|(title, n)| GlyphSection::new(title, members.by_ref().take(n).map(item).collect()))
            .collect();
        self.rows = rows;
        self.query.clear();
        self.query.push_str(query);
        self.lang = Some(primary);
        match previous {
            // Aynı görünüm: kaydırma ofseti ve süren hareket korunur (framework b96e3cb).
            Some((i, e)) => {
                let selected = self.restore_index(i, e);
                self.grid.replace_sections(theme, built, selected, now);
            }
            None => {
                self.grid.set_sections(theme, built, now);
                if !self.rows.is_empty() {
                    self.grid.set_selected(0, now);
                }
            }
        }
        self.update_footer();
        true
    }

    /// Yeniden kurulan listede önceki seçimin yeri: aynı indeksteki aynı emoji, yoksa emojinin
    /// ilk görüldüğü yer (sık kullanılanlar bölümü değişmiş olabilir), o da yoksa aynı indeks
    /// (sona kırpılır).
    fn restore_index(&self, index: usize, emoji: Option<&'static str>) -> usize {
        let Some(e) = emoji else { return index.min(self.rows.len().saturating_sub(1)) };
        if self.emoji_at(index) == Some(e) {
            return index;
        }
        (0..self.rows.len()).find(|&i| self.emoji_at(i) == Some(e)).unwrap_or(index.min(self.rows.len() - 1))
    }

    /// Ad satırı: seçili emoji ve iki dildeki adı.
    fn update_footer(&mut self) {
        let primary = primary_lang();
        let selected = self.grid.selected().and_then(|i| self.rows.get(i)).and_then(|&i| emoji::all().get(i as usize));
        match selected {
            Some(e) => {
                let (name, other) = (e.name(primary), e.name(primary.other()));
                self.footer_icon.set_icon(Icon::glyph(e.emoji));
                let text = if name == other { name.to_owned() } else { format!("{name} · {other}") };
                self.footer.set_text(&text);
            }
            None => {
                self.footer_icon.set_icon(Icon::named(IconName::Emoji));
                self.footer.set_text("");
            }
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.grid.set_theme(theme, now);
        self.footer_icon.set_theme(theme, now);
        self.footer.set_style(text_style(theme, TextRole::Body));
    }

    /// Izgaraya giden tuş mu (gezinme ve Enter).
    pub fn wants_key(event: &KeyEvent) -> bool {
        matches!(
            event.key,
            Key::Up | Key::Down | Key::Left | Key::Right | Key::PageUp | Key::PageDown | Key::Home | Key::End | Key::Enter
        )
    }

    /// Klavye. Döner: (yeniden çizim, seçilen emoji).
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> (bool, Option<&'static str>) {
        if event.state == KeyState::Released && event.key == Key::Enter {
            return (false, None);
        }
        let r = self.grid.key(event, now);
        self.handle(r.repaint, r.event)
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> (bool, Option<&'static str>) {
        let r = self.grid.pointer(theme, event, now);
        self.handle(r.repaint, r.event)
    }

    fn handle(&mut self, repaint: bool, event: Option<GlyphGridEvent>) -> (bool, Option<&'static str>) {
        match event {
            Some(GlyphGridEvent::Selected(_)) => {
                self.update_footer();
                (true, None)
            }
            Some(GlyphGridEvent::Activated(i)) => (true, self.emoji_at(i)),
            None => (repaint, None),
        }
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.grid.cursor(p)
    }

    /// Izgara `area`'nın üstünde, ad satırı altında.
    pub fn layout(&mut self, theme: &Theme, area: Rect, now: Instant) {
        let footer = Self::footer_height(theme);
        let grid = Rect::new(area.x, area.y, area.width, (area.height - footer - theme.space(2)).max(0.0));
        if self.grid.rect() != grid {
            self.grid.set_rect(theme, grid, now);
        }
    }

    /// Kapsama ilk çizimde denetlenir; değiştiyse liste yeniden kurulur (aynı kare).
    fn ensure_coverage(&mut self, ui: &mut Ui, usage: &Usage) {
        if self.coverage.is_some() {
            return;
        }
        let found = coverage(ui.painter.text_system());
        if let Some((family, c)) = &found {
            let hidden = c.iter().filter(|v| !**v).count();
            tracing::debug!(family, hidden, "emoji font kapsaması denetlendi");
        }
        self.coverage = Some(found.map(|(_, c)| c));
        let query = self.query.clone();
        self.refresh(ui.theme, usage, &query, ui.now);
    }

    /// `area`: arama alanının altındaki bölge (ızgara + ad satırı).
    pub fn draw(&mut self, ui: &mut Ui, area: Rect, usage: &Usage) {
        let theme = ui.theme;
        self.ensure_coverage(ui, usage);
        self.layout(theme, area, ui.now);
        self.grid.draw(ui);
        let h = Self::footer_height(theme);
        let y = area.y + area.height - h;
        let icon = self.footer_icon.measure(ui.painter.text_system());
        let center = Point::new(area.x + theme.space(2) + icon.width * 0.5, y + h * 0.5);
        self.footer_icon.draw_centered(ui.painter, center, ui.fade(theme.colors.on_surface));
        let m = self.footer.measure_in(ui.painter);
        let at = Point::new(area.x + theme.space(4) + icon.width, y + (h - m.height) * 0.5);
        self.footer.draw(ui.painter, at, ui.fade(theme.colors.on_surface_variant));
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        self.footer_icon.animate(now)
    }

    #[cfg(test)]
    pub fn grid(&self) -> &GlyphGrid {
        &self.grid
    }
}

/// Emoji aramasının etkin dili: arayüzün ilk dili (Türkçe değilse İngilizce).
pub fn primary_lang() -> Lang {
    nuxut::i18n::current().languages().first().map_or(Lang::En, |l| Lang::from_language(l.language.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: Key) -> KeyEvent {
        KeyEvent { key: k, state: KeyState::Pressed, text: None, modifiers: nuxut::Modifiers::default(), xkb: None }
    }

    #[test]
    fn empty_query_has_recent_and_group_sections() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = EmojiView::new(&theme);
        v.refresh(&theme, &Usage::default(), "", now);
        assert_eq!(v.grid().section_count(), Group::ALL.len(), "kullanım yokken yalnız gruplar");
        assert_eq!(v.len(), emoji::all().len());
        assert_eq!(v.selected_emoji(), Some("😀"));

        let mut u = Usage::default();
        u.record_emoji("🐈", now_unix());
        u.record_emoji("🌹", now_unix() - 10 * 86_400);
        // Açılış gibi (yer unutulur): ilk öğe, yani en sık kullanılan seçili.
        v.forget_position();
        v.refresh(&theme, &u, "", now);
        assert_eq!(v.grid().section_count(), Group::ALL.len() + 1);
        assert_eq!(v.grid().section_of(0), Some(0));
        assert_eq!((v.emoji_at(0), v.emoji_at(1)), (Some("🐈"), Some("🌹")), "sık kullanılanlar başta");
        assert_eq!(v.len(), emoji::all().len() + 2, "sık kullanılanlar grupta da durur");
        assert_eq!(v.footer.text(), "kedi · cat");
    }

    #[test]
    fn query_is_flat_and_keys_pick() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = EmojiView::new(&theme);
        v.layout(&theme, Rect::new(0.0, 0.0, 480.0, 400.0), now);
        v.refresh(&theme, &Usage::default(), "kedi", now);
        assert_eq!(v.grid().section_count(), 1, "sorguda tek düz küme");
        assert_eq!(v.selected_emoji(), Some("🐈"));
        assert!(v.len() <= MAX_EMOJI_RESULTS);
        // → seçimi taşır, ad satırı güncellenir; Enter seçileni verir.
        let (redraw, out) = v.key(&key(Key::Right), now);
        assert!(redraw && out.is_none());
        let second = v.selected_emoji().unwrap();
        assert_ne!(second, "🐈");
        assert!(v.footer.text().contains(" · "));
        let (_, out) = v.key(&key(Key::Enter), now);
        assert_eq!(out, Some(second));
        // Harfler ızgaraya gitmez (arama alanına).
        assert!(!EmojiView::wants_key(&key(Key::Char('a'))));
        assert!(EmojiView::wants_key(&key(Key::Left)) && EmojiView::wants_key(&key(Key::Enter)));
        // Sonuç yok: seçim yok, ad satırı boş, kip ikonu.
        v.refresh(&theme, &Usage::default(), "zzzqqq", now);
        assert!(v.is_empty() && v.selected_emoji().is_none());
        assert_eq!(v.footer.text(), "");
        assert_eq!(v.footer_icon.icon(), &Icon::named(IconName::Emoji));
    }

    #[test]
    fn uncovered_emoji_are_hidden() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = EmojiView::new(&theme);
        let mut c = vec![true; emoji::all().len()];
        c[emoji::find("🐈").unwrap()] = false;
        v.set_coverage(Some(c));
        v.refresh(&theme, &Usage::default(), "kedi", now);
        assert!(!(0..v.len()).any(|i| v.emoji_at(i) == Some("🐈")));
        v.refresh(&theme, &Usage::default(), "", now);
        assert_eq!(v.len(), emoji::all().len() - 1);
        // Aile yoksa süzme yok.
        v.set_coverage(None);
        v.refresh(&theme, &Usage::default(), "", now);
        assert_eq!(v.len(), emoji::all().len());
    }

    /// inceleme şartı: aynı sorguyla yenileme (kullanım, dil, kapsama) kullanıcının
    /// seçimini sıfırlamaz; yalnız yeni sorgu ve açılış başa döner.
    #[test]
    fn same_query_refresh_keeps_position() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = EmojiView::new(&theme);
        v.layout(&theme, Rect::new(0.0, 0.0, 480.0, 400.0), now);
        assert!(v.refresh(&theme, &Usage::default(), "", now));
        for _ in 0..3 {
            v.key(&key(Key::Down), now);
        }
        let picked = v.selected_emoji().unwrap();
        let at = v.grid().selected().unwrap();
        assert_ne!(at, 0);

        // Veri ve dil aynı: ızgaraya dokunulmaz.
        assert!(!v.refresh(&theme, &Usage::default(), "", now));
        assert_eq!(v.grid().selected(), Some(at));

        // Sık kullanılanlar değişti: yeniden kurulur, aynı emoji seçili kalır (yeri kaydı).
        let mut u = Usage::default();
        u.record_emoji("🌹", now_unix());
        assert!(v.refresh(&theme, &u, "", now));
        assert_eq!(v.emoji_at(0), Some("🌹"));
        assert_eq!(v.selected_emoji(), Some(picked));
        assert_eq!(v.grid().selected(), Some(at + 1));

        // Dil değişti (adlar ve başlıklar): yeniden kurulur, seçim yerinde.
        v.lang = Some(primary_lang().other());
        assert!(v.refresh(&theme, &u, "", now));
        assert_eq!(v.selected_emoji(), Some(picked));

        // Seçili emoji kapsamadan düştü: aynı indekse yakın bir yer, panik yok.
        let mut c = vec![true; emoji::all().len()];
        c[emoji::find(picked).unwrap()] = false;
        v.set_coverage(Some(c));
        assert!(v.refresh(&theme, &u, "", now));
        assert_eq!(v.grid().selected(), Some(at + 1));

        // Yeni sorgu ve açılış başa döner.
        assert!(v.refresh(&theme, &u, "kedi", now));
        assert_eq!(v.grid().selected(), Some(0));
        v.key(&key(Key::Right), now);
        v.forget_position();
        assert!(v.refresh(&theme, &u, "kedi", now));
        assert_eq!(v.grid().selected(), Some(0));
    }

    /// Sistem fontlarıyla gerçek kapsama: kurulu emoji fontu veriyi (Emoji ≤ 16.0) neredeyse
    /// tümüyle tek glife indirmeli (tek glif kuralı geçerli emojiyi gizlemesin). Emoji fontu
    /// yoksa atlanır.
    #[test]
    fn system_emoji_font_covers_data() {
        let mut text = TextSystem::new();
        let Some((family, c)) = coverage(&mut text) else { return };
        let hidden: Vec<&str> =
            emoji::all().iter().zip(&c).filter(|(_, ok)| !**ok).map(|(e, _)| e.emoji).collect();
        eprintln!("{family}: {} / {} emoji gizlenir {:?}", hidden.len(), c.len(), &hidden[..hidden.len().min(10)]);
        let ratio = hidden.len() as f32 / c.len() as f32;
        // Fontun bilmediği ZWJ dizisi parçalı çizilirdi: tek glif kuralı onu reddeder.
        assert_ne!(text.glyphs_in_family(family, "🐈\u{200D}🌹"), Some(1));
        assert!(ratio < 0.01, "{family}: {} / {} gizlenir: {:?}", hidden.len(), c.len(), &hidden[..hidden.len().min(20)]);
    }
}
