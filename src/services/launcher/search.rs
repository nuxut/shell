//! Bulanık arama ve sıralama (`nucleo-matcher`; UI thread'inde, tuş başına senkron).
//!
//! Her uygulamanın dört samanlığı ([`Field`]) ayrı eşlenir; puan = en iyi alanın nucleo puanı ×
//! alan ağırlığı + ad öneki bonusu + frecency bonusu. Çok kelimeli sorguda her kelime aynı alanda
//! eşleşmelidir. Boş sorgu: frecency azalan, eşitlikte alfabetik. Ayırma yok (tamponlar yeniden
//! kullanılır); 5000 uygulamada tuş başına hedef < 2 ms (`examples/launcher_bench.rs`).
//!
//! Vurgu indeksleri yalnızca görünen satırlar için istenir ([`Searcher::name_highlight`]).


use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

use super::catalog::{Catalog, Field};
use super::usage::Usage;

/// Sorgunun en büyük uzunluğu (bayt); fazlası kırpılır.
pub const MAX_QUERY: usize = 256;
/// Alan ağırlıkları (yüzde), [`Field`] sırasıyla.
const WEIGHTS: [u32; 4] = [100, 70, 60, 55];
/// Ad sorguyla başlıyorsa ek puan (nucleo'nun kelime başı bonusunun üstüne).
const NAME_PREFIX_BONUS: u32 = 24;
/// Frecency → puan ölçeği ve tavanı (≈ 2–3 eşleşen karakter; güçlü eşleşmeyi ezmez).
const FRECENCY_SCALE: f32 = 12.0;
const FRECENCY_CAP: u32 = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    /// Katalog indeksi.
    pub index: u32,
    pub score: u32,
}

pub struct Searcher {
    matcher: Matcher,
    pattern: Pattern,
    query: String,
    hits: Vec<Hit>,
    indices: Vec<u32>,
}

impl std::fmt::Debug for Searcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Sorgu kullanıcı metnidir: yalnızca uzunluk.
        f.debug_struct("Searcher").field("query_len", &self.query.len()).field("hits", &self.hits.len()).finish()
    }
}

impl Default for Searcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Searcher {
    pub fn new() -> Self {
        let mut config = Config::DEFAULT;
        config.prefer_prefix = true;
        Self {
            matcher: Matcher::new(config),
            pattern: Pattern::default(),
            query: String::new(),
            hits: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Sorguyu çalıştırır; sonuçlar puan azalan sırada.
    pub fn search(&mut self, catalog: &Catalog, usage: &Usage, query: &str, now: i64) -> &[Hit] {
        let query = clip(query.trim());
        if query != self.query {
            self.query.clear();
            self.query.push_str(query);
            // Düz bulanık kelimeler (fzf sözdizimi `^ $ ! '` yok: kullanıcı metni olduğu gibi aranır).
            self.pattern = Pattern::new(query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy);
        }
        self.hits.clear();
        let frecency = |id: &str| ((usage.frecency(id, now) * FRECENCY_SCALE) as u32).min(FRECENCY_CAP);

        if self.pattern.atoms.is_empty() {
            self.hits.extend(
                catalog.apps.iter().enumerate().map(|(i, a)| Hit { index: i as u32, score: frecency(&a.entry.id) }),
            );
            // Katalog alfabetik: kararlı sıralama eşitlikte bu sırayı korur.
            self.hits.sort_by_key(|h| std::cmp::Reverse(h.score));
            return &self.hits;
        }

        let query_lower = query.to_lowercase();
        for (i, app) in catalog.apps.iter().enumerate() {
            let mut best = 0u32;
            let mut matched = false;
            for (field, haystack) in app.haystacks.iter().enumerate() {
                if haystack.is_empty() {
                    continue;
                }
                if let Some(s) = self.pattern.score(haystack.slice(..), &mut self.matcher) {
                    matched = true;
                    best = best.max(s * WEIGHTS[field] / 100);
                }
            }
            if !matched {
                continue;
            }
            if app.sort_key.starts_with(&query_lower) {
                best += NAME_PREFIX_BONUS;
            }
            self.hits.push(Hit { index: i as u32, score: best + frecency(&app.entry.id) });
        }
        self.hits.sort_by_key(|h| std::cmp::Reverse(h.score));
        &self.hits
    }

    /// Son sonuçlar.
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    /// Son sorgu adla eşleşiyorsa adın vurgulanacak karakter indeksleri (sıralı, tekrarsız;
    /// `ResultItem::match_chars` bayt aralığına çevirir). Eşleşme yoksa boş.
    pub fn name_highlight(&mut self, catalog: &Catalog, index: usize) -> &[u32] {
        self.indices.clear();
        let Some(app) = catalog.apps.get(index) else { return &self.indices };
        if self.pattern.atoms.is_empty() {
            return &self.indices;
        }
        let haystack = &app.haystacks[Field::Name as usize];
        if self.pattern.indices(haystack.slice(..), &mut self.matcher, &mut self.indices).is_none() {
            self.indices.clear();
            return &self.indices;
        }
        self.indices.sort_unstable();
        self.indices.dedup();
        &self.indices
    }
}

fn clip(s: &str) -> &str {
    if s.len() <= MAX_QUERY {
        return s;
    }
    let mut end = MAX_QUERY;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::services::launcher::desktop::{self, Locale};

    fn entry(id: &str, body: &str) -> desktop::DesktopEntry {
        let text = format!("[Desktop Entry]\nType=Application\n{body}");
        desktop::parse(id, Path::new("/x"), &text, &Locale::default()).unwrap()
    }

    fn catalog() -> Catalog {
        Catalog::from_entries(
            [
                entry("firefox.desktop", "Name=Firefox\nGenericName=Web Browser\nExec=firefox %u\nKeywords=internet;www;"),
                entry("org.gnome.Nautilus.desktop", "Name=Files\nExec=nautilus\nKeywords=folder;explorer;"),
                entry("fileroller.desktop", "Name=File Roller\nExec=file-roller"),
                entry("kitty.desktop", "Name=kitty\nGenericName=Terminal Emulator\nExec=kitty"),
                entry("foot.desktop", "Name=Foot\nGenericName=Terminal\nExec=foot"),
                entry("ayarlar.desktop", "Name=Sistem Ayarları\nExec=ayarlar"),
            ],
            1,
        )
    }

    fn ids<'a>(c: &'a Catalog, hits: &[Hit]) -> Vec<&'a str> {
        hits.iter().map(|h| c.apps[h.index as usize].entry.id.as_str()).collect()
    }

    #[test]
    fn name_beats_other_fields() {
        let c = catalog();
        let mut s = Searcher::new();
        let u = Usage::default();
        let hits = s.search(&c, &u, "fire", 0).to_vec();
        assert_eq!(ids(&c, &hits)[0], "firefox.desktop");
        let hits = s.search(&c, &u, "terminal", 0).to_vec();
        assert_eq!(ids(&c, &hits).len(), 2);
        let hits = s.search(&c, &u, "nautilus", 0).to_vec();
        assert_eq!(ids(&c, &hits), ["org.gnome.Nautilus.desktop"], "program/kimlik aranır");
        let hits = s.search(&c, &u, "web browser", 0).to_vec();
        assert_eq!(ids(&c, &hits), ["firefox.desktop"], "çok kelime aynı alanda");
        let hits = s.search(&c, &u, "zzzz", 0).to_vec();
        assert!(hits.is_empty());
        let hits = s.search(&c, &u, "ayarlari", 0).to_vec();
        assert_eq!(ids(&c, &hits), ["ayarlar.desktop"], "aksan normalleştirme");
    }

    #[test]
    fn frecency_breaks_ties_but_not_strong_matches() {
        let c = catalog();
        let mut s = Searcher::new();
        let mut u = Usage::default();
        let now = 1_000_000;
        let base = s.search(&c, &u, "fil", now).to_vec();
        assert_eq!(ids(&c, &base)[..2], ["fileroller.desktop", "org.gnome.Nautilus.desktop"], "eşitlikte alfabetik");
        for _ in 0..20 {
            u.record("org.gnome.Nautilus.desktop", now);
        }
        let boosted = s.search(&c, &u, "fil", now).to_vec();
        assert_eq!(ids(&c, &boosted)[0], "org.gnome.Nautilus.desktop", "eşit güçte eşleşmede kullanım öne alır");
        // Çok kullanılan ama zayıf eşleşen, güçlü ad eşleşmesini geçemez.
        for _ in 0..200 {
            u.record("kitty.desktop", now);
        }
        let hits = s.search(&c, &u, "foot", now).to_vec();
        assert_eq!(ids(&c, &hits)[0], "foot.desktop");
    }

    #[test]
    fn empty_query_orders_by_usage_then_name() {
        let c = catalog();
        let mut s = Searcher::new();
        let mut u = Usage::default();
        u.record("kitty.desktop", 100);
        let hits = s.search(&c, &u, "  ", 100).to_vec();
        assert_eq!(ids(&c, &hits), [
            "kitty.desktop",
            "fileroller.desktop",
            "org.gnome.Nautilus.desktop",
            "firefox.desktop",
            "foot.desktop",
            "ayarlar.desktop"
        ]);
    }

    #[test]
    fn highlight_char_indices() {
        let c = catalog();
        let mut s = Searcher::new();
        let u = Usage::default();
        s.search(&c, &u, "ayarl", 0);
        let (i, _) = c.find("ayarlar.desktop").unwrap();
        let name: Vec<char> = c.apps[i].entry.name.chars().collect();
        let lit: String = s.name_highlight(&c, i).iter().map(|&k| name[k as usize]).collect();
        assert_eq!(lit, "Ayarl");
        s.search(&c, &u, "nautilus", 0);
        let (i, _) = c.find("org.gnome.Nautilus.desktop").unwrap();
        assert!(s.name_highlight(&c, i).is_empty(), "ad eşleşmiyorsa vurgu yok");
    }

    #[test]
    fn query_is_clipped_and_debug_redacted() {
        let c = catalog();
        let mut s = Searcher::new();
        let long = "ş".repeat(MAX_QUERY);
        s.search(&c, &Usage::default(), &long, 0);
        assert!(s.query.len() <= MAX_QUERY);
        s.search(&c, &Usage::default(), "gizli-sorgu", 0);
        assert!(!format!("{s:?}").contains("gizli"));
    }
}
