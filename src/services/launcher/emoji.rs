//! Emoji verisi ve araması (başlatıcının emoji kipi; `docs/design/emoji.md`).
//!
//! Veri derlemeye gömülüdür (`data/emoji.tsv`, `include_str!`): çalışma zamanında dosya okunmaz,
//! sisteme bağımlılık yoktur. İlk kullanımda bir kez ayrıştırılır ([`all`]; ~1900 emoji,
//! dizgiler gömülü veriye işaret eder, yalnız katlanmış arama metinleri ayrılır). Veri
//! `examples/emoji_import.rs` ile ibus'un CLDR türevi sözlüklerinden üretilir (Unicode License
//! v3, `data/LICENSE-UNICODE`). v1 kapsamı taban emojidir (ten rengi varyantı yok).
//!
//! Arama (UI thread'inde, tuş başına senkron): sorgu ve veriler aynı katlamadan geçer
//! ([`fold_char`]: nucleo'nun büyük/küçük harf + aksan katlaması — uygulama aramasıyla aynı
//! tablo — ve Türkçe `I/İ/ı → i`). Her sorgu kelimesi bir sözcüğün **başı** olmalıdır
//! (`kal` → "kırmızı kalp"); kademeler: ad tam eşit > ad sorguyla başlar > tüm kelimeler adda >
//! tüm kelimeler ad/anahtar sözcüklerde > alt dizgi. İki dil de aranır; etkin dilin puanı tam,
//! diğerinin [`SECONDARY_WEIGHT`] ile çarpılır (güçlü bir yabancı dil eşleşmesi zayıf bir
//! etkin dil eşleşmesini yine geçer: tr'de "cat" → 🐈, "çatal" değil). Kullanım
//! ([`Usage::emoji_frecency`]) kademe içinde öne alır, kademe atlatmaz. Boş sorgu: sık
//! kullanılanlar, sonra veri sırası (emoji-test).

use std::collections::HashMap;
use std::sync::OnceLock;

use nucleo_matcher::chars::{normalize, to_lower_case};

use super::usage::Usage;

/// Gömülü veri: `emoji  grup  en_ad  en_sözcükler  tr_ad  tr_sözcükler` (sözcükler `|` ile).
const DATA: &str = include_str!("data/emoji.tsv");

/// Sorgunun en büyük uzunluğu (bayt) ve kelime sayısı; fazlası kırpılır.
pub const MAX_QUERY: usize = 128;
const MAX_TERMS: usize = 8;
/// Etkin olmayan dilin puan ağırlığı (yüzde).
pub const SECONDARY_WEIGHT: u32 = 80;
/// Frecency → puan ölçeği ve tavanı (kademe aralığından küçük: kademe atlatmaz).
const FRECENCY_SCALE: f32 = 40.0;
const FRECENCY_CAP: u32 = 150;

/// Eşleşme kademeleri (puan).
const TIER_EXACT: u32 = 1000;
const TIER_NAME_PREFIX: u32 = 700;
const TIER_NAME_WORDS: u32 = 500;
const TIER_KEYWORDS: u32 = 300;
const TIER_SUBSTRING: u32 = 100;
/// Kullanım kademe atlatmaz: komşu kademeler arası fark frecency tavanından büyük (ikinci dilin
/// %80'i de aynı sınırı bozmaz: 1000·0,8 = 800 > 700 kasıtlıdır, bkz. modül belgesi).
const _TIERS_APART: () = assert!(
    TIER_SUBSTRING + FRECENCY_CAP < TIER_KEYWORDS
        && TIER_KEYWORDS + FRECENCY_CAP < TIER_NAME_WORDS
        && TIER_NAME_WORDS + FRECENCY_CAP < TIER_NAME_PREFIX
        && TIER_NAME_PREFIX + FRECENCY_CAP < TIER_EXACT
);

/// Unicode emoji grupları (emoji-test.txt; "Component" veride yok).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Smileys,
    People,
    Animals,
    Food,
    Travel,
    Activities,
    Objects,
    Symbols,
    Flags,
}

impl Group {
    pub const ALL: [Group; 9] = [
        Group::Smileys,
        Group::People,
        Group::Animals,
        Group::Food,
        Group::Travel,
        Group::Activities,
        Group::Objects,
        Group::Symbols,
        Group::Flags,
    ];

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "Smileys & Emotion" => Self::Smileys,
            "People & Body" => Self::People,
            "Animals & Nature" => Self::Animals,
            "Food & Drink" => Self::Food,
            "Travel & Places" => Self::Travel,
            "Activities" => Self::Activities,
            "Objects" => Self::Objects,
            "Symbols" => Self::Symbols,
            "Flags" => Self::Flags,
            _ => return None,
        })
    }

    /// Yerelleştirilmiş grup adı.
    pub fn label(self) -> String {
        use nuxut::i18n::t;
        match self {
            Self::Smileys => t!("shell-emoji-group-smileys"),
            Self::People => t!("shell-emoji-group-people"),
            Self::Animals => t!("shell-emoji-group-animals"),
            Self::Food => t!("shell-emoji-group-food"),
            Self::Travel => t!("shell-emoji-group-travel"),
            Self::Activities => t!("shell-emoji-group-activities"),
            Self::Objects => t!("shell-emoji-group-objects"),
            Self::Symbols => t!("shell-emoji-group-symbols"),
            Self::Flags => t!("shell-emoji-group-flags"),
        }
    }
}

/// Verideki diller ([`Emoji::name`] dizini).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En = 0,
    Tr = 1,
}

impl Lang {
    pub fn other(self) -> Self {
        match self {
            Self::En => Self::Tr,
            Self::Tr => Self::En,
        }
    }

    /// Etkin arayüz dilinden (ilk tercih Türkçeyse `Tr`, yoksa `En`).
    pub fn from_language(code: &str) -> Self {
        if code.eq_ignore_ascii_case("tr") { Self::Tr } else { Self::En }
    }
}

#[derive(Debug)]
pub struct Emoji {
    /// Panoya giden dizi (tam nitelikli, U+FE0F'li biçim).
    pub emoji: &'static str,
    pub group: Group,
    names: [&'static str; 2],
    keywords: [&'static str; 2],
    /// Katlanmış ad ve `ad|sözcük|…` (dil başına).
    folded_name: [Box<str>; 2],
    folded_all: [Box<str>; 2],
}

impl Emoji {
    /// CLDR kısa adı (dilde ad yoksa İngilizcesi).
    pub fn name(&self, lang: Lang) -> &'static str {
        match self.names[lang as usize] {
            "" => self.names[Lang::En as usize],
            n => n,
        }
    }

    /// CLDR anahtar sözcükleri.
    pub fn keywords(&self, lang: Lang) -> impl Iterator<Item = &'static str> {
        self.keywords[lang as usize].split('|').filter(|k| !k.is_empty())
    }
}

/// Veri satırı ayrıştırma hatası (yalnız testte ve hata ayıklamada anlamlı: veri gömülü).
fn parse_line(line: &'static str) -> Option<Emoji> {
    let mut f = line.split('\t');
    let (emoji, group, en, en_kw, tr, tr_kw) = (f.next()?, f.next()?, f.next()?, f.next()?, f.next()?, f.next()?);
    if f.next().is_some() || emoji.is_empty() || en.is_empty() {
        return None;
    }
    let group = Group::parse(group)?;
    let names = [en, tr];
    let keywords = [en_kw, tr_kw];
    let folded_name = names.map(|n| fold(n).into_boxed_str());
    let folded_all = [0, 1].map(|i| {
        let mut s = fold(names[i]);
        if !keywords[i].is_empty() {
            s.push('|');
            s.push_str(&fold(keywords[i]));
        }
        s.into_boxed_str()
    });
    Some(Emoji { emoji, group, names, keywords, folded_name, folded_all })
}

/// Başlık yorumları `# ` ile başlar; `#️⃣` (U+0023 U+FE0F U+20E3) gibi emoji satırı yorum değildir.
fn is_data_line(line: &str) -> bool {
    !line.is_empty() && !line.starts_with("# ")
}

struct Table {
    items: Vec<Emoji>,
    index: HashMap<&'static str, u16>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut items = Vec::with_capacity(2048);
        for line in DATA.lines().filter(|l| is_data_line(l)) {
            match parse_line(line) {
                Some(e) => items.push(e),
                // Gömülü veri testle doğrulanır; yine de bozuk satır paniğe yol açmaz.
                None => tracing::warn!("emoji verisinde bozuk satır atlandı"),
            }
        }
        let index = items.iter().enumerate().map(|(i, e)| (e.emoji, i as u16)).collect();
        Table { items, index }
    })
}

/// Tüm emojiler (veri sırası: Unicode emoji-test).
pub fn all() -> &'static [Emoji] {
    &table().items
}

/// Dizinin verideki indeksi (kullanım kaydı doğrulaması: dosyaya keyfi anahtar yazılmaz).
pub fn find(emoji: &str) -> Option<usize> {
    table().index.get(emoji).map(|&i| i as usize)
}

/// Arama katlaması: Türkçe noktalı/noktasız i tek `i`; sonra nucleo'nun basit büyük/küçük harf
/// katlaması ve Latin aksan normalleştirmesi (ş→s, ğ→g, ü→u, ö→o, ç→c, é→e…).
pub fn fold_char(c: char) -> char {
    match c {
        'I' | 'İ' | 'ı' => 'i',
        c => to_lower_case(normalize(to_lower_case(c))),
    }
}

/// Bir dizgiyi katlar (birleşik nokta U+0307 atılır: "i̇" → "i").
pub fn fold(s: &str) -> String {
    s.chars().filter(|&c| c != '\u{307}').map(fold_char).collect()
}

/// Sözcük karakteri (sözcük başı tespiti için).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// `term` `hay` içinde bir sözcüğün başında geçiyor mu.
fn word_prefix(hay: &str, term: &str) -> bool {
    let mut prev_word = false;
    for (i, c) in hay.char_indices() {
        if !prev_word && hay[i..].starts_with(term) {
            return true;
        }
        prev_word = is_word_char(c);
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmojiHit {
    /// [`all`] indeksi.
    pub index: u16,
    pub score: u32,
}

/// Emoji araması; tamponlar yeniden kullanılır.
#[derive(Default)]
pub struct EmojiSearcher {
    query: String,
    terms: Vec<String>,
    hits: Vec<EmojiHit>,
}

impl std::fmt::Debug for EmojiSearcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Sorgu kullanıcı metnidir: yalnızca uzunluk.
        f.debug_struct("EmojiSearcher").field("query_len", &self.query.len()).field("hits", &self.hits.len()).finish()
    }
}

impl EmojiSearcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bir dilde kademe puanı (eşleşme yoksa `None`).
    fn tier(&self, e: &Emoji, lang: Lang) -> Option<u32> {
        let name = &*e.folded_name[lang as usize];
        let all = &*e.folded_all[lang as usize];
        if name.is_empty() {
            return None;
        }
        if name == self.query {
            return Some(TIER_EXACT);
        }
        if name.starts_with(&self.query) {
            return Some(TIER_NAME_PREFIX);
        }
        if self.terms.iter().all(|t| word_prefix(name, t)) {
            return Some(TIER_NAME_WORDS);
        }
        if self.terms.iter().all(|t| word_prefix(all, t)) {
            return Some(TIER_KEYWORDS);
        }
        // Alt dizgi yalnız iki ve daha uzun kelimelerle (tek harf her yerde geçer).
        if self.terms.iter().all(|t| t.chars().count() >= 2 && all.contains(t.as_str())) {
            return Some(TIER_SUBSTRING);
        }
        None
    }

    /// Sorguyu çalıştırır; sonuçlar puan azalan, eşitlikte veri sırasında.
    pub fn search(&mut self, usage: &Usage, query: &str, primary: Lang, now: i64) -> &[EmojiHit] {
        let folded = fold(clip(query.trim()));
        let normalized = folded.split_whitespace().take(MAX_TERMS).collect::<Vec<_>>().join(" ");
        if normalized != self.query {
            self.terms = normalized.split(' ').filter(|t| !t.is_empty()).map(str::to_owned).collect();
            self.query = normalized;
        }
        self.hits.clear();
        let frecency = |e: &Emoji| ((usage.emoji_frecency(e.emoji, now) * FRECENCY_SCALE) as u32).min(FRECENCY_CAP);
        let items = all();
        if self.terms.is_empty() {
            self.hits.extend(items.iter().enumerate().map(|(i, e)| EmojiHit { index: i as u16, score: frecency(e) }));
        } else {
            for (i, e) in items.iter().enumerate() {
                let own = self.tier(e, primary);
                let other = self.tier(e, primary.other()).map(|s| s * SECONDARY_WEIGHT / 100);
                let Some(best) = own.max(other) else { continue };
                self.hits.push(EmojiHit { index: i as u16, score: best + frecency(e) });
            }
        }
        // Eşitlikte veri sırası. Kararlı sıralama yerine indeks anahtarlı kararsız sıralama: aynı
        // sonuç, ama kararlı sıralamanın her aramada ayırdığı geçici tampon yok.
        self.hits.sort_unstable_by_key(|h| (std::cmp::Reverse(h.score), h.index));
        &self.hits
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
    use super::*;

    fn top(s: &mut EmojiSearcher, q: &str, lang: Lang) -> Vec<&'static str> {
        s.search(&Usage::default(), q, lang, 0).iter().take(5).map(|h| all()[h.index as usize].emoji).collect()
    }

    #[test]
    fn data_is_complete_and_unique() {
        let raw = DATA.lines().filter(|l| !l.is_empty() && !l.starts_with("# ")).count();
        assert_eq!(raw, DATA.lines().count() - 3, "üç başlık satırı dışında hepsi veri");
        let items = all();
        assert_eq!(items.len(), raw, "her veri satırı ayrışır");
        assert!(items.len() > 1800, "taban emoji sayısı ({})", items.len());
        assert_eq!(table().index.len(), items.len(), "emoji dizileri tekil");
        for e in items {
            assert!(!e.names[0].is_empty(), "İngilizce ad: {}", e.emoji);
            assert!(!e.names[1].is_empty(), "Türkçe ad: {}", e.emoji);
            // v1: ten rengi varyantı ve tek başına bileşen yok.
            assert!(!e.emoji.chars().any(|c| ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)), "ten varyantı: {}", e.emoji);
            assert!(!e.emoji.contains('\t') && !e.emoji.contains('|'));
        }
        for g in Group::ALL {
            assert!(items.iter().any(|e| e.group == g), "{g:?} grubu boş");
        }
        // Veri sırası emoji-test sırasıdır.
        assert_eq!(items[0].emoji, "😀");
        assert_eq!(find("😀"), Some(0));
        assert_eq!(find("🇹🇷").map(|i| items[i].name(Lang::Tr)), Some("bayrak: Türkiye"));
        // Tam nitelikli biçim yazılır (U+FE0F'li).
        assert!(find("❤\u{FE0F}").is_some() && find("❤").is_none());
        assert!(find("firefox.desktop").is_none());
        // `#` ile başlayan emoji (tuş başlığı) yorum sanılmaz.
        assert!(find("#\u{FE0F}\u{20E3}").is_some());
        assert!(find("*\u{FE0F}\u{20E3}").is_some());
    }

    #[test]
    fn turkish_folding() {
        assert_eq!(fold("IŞIK"), "isik");
        assert_eq!(fold("ışık"), "isik");
        assert_eq!(fold("İSTANBUL"), "istanbul");
        assert_eq!(fold("i̇"), "i", "birleşik nokta atılır");
        assert_eq!(fold("ÇİÇEK Ğ Ö Ü Ş"), "cicek g o u s");
        assert_eq!(fold("Café"), "cafe");
        assert_eq!(fold("😀 ❤️"), "😀 ❤️", "emoji ve semboller değişmez");
    }

    #[test]
    fn word_prefix_matching() {
        assert!(word_prefix("kirmizi kalp", "kal"));
        assert!(word_prefix("bayrak: turkiye", "tur"));
        assert!(!word_prefix("kirmizi kalp", "alp"), "sözcük ortası değil");
        assert!(word_prefix("yuz|gulen yuz", "gul"));
    }

    #[test]
    fn searches_turkish_and_english() {
        let mut s = EmojiSearcher::new();
        // Türkçe ad ve aksansız yazım.
        assert_eq!(top(&mut s, "kırmızı kalp", Lang::Tr)[0], "❤\u{FE0F}");
        assert_eq!(top(&mut s, "kirmizi kalp", Lang::Tr)[0], "❤\u{FE0F}");
        assert_eq!(top(&mut s, "KIRMIZI KALP", Lang::Tr)[0], "❤\u{FE0F}");
        // İngilizce sorgu Türkçe arayüzde de çalışır.
        assert_eq!(top(&mut s, "red heart", Lang::Tr)[0], "❤\u{FE0F}");
        // Güçlü yabancı dil eşleşmesi zayıf etkin dil eşleşmesini geçer ("çatal" ≠ cat).
        assert_eq!(top(&mut s, "cat", Lang::Tr)[0], "🐈");
        assert_eq!(top(&mut s, "kedi", Lang::En)[0], "🐈");
        assert_eq!(top(&mut s, "kedi yüzü", Lang::Tr)[0], "🐱");
        assert_eq!(top(&mut s, "türkiye", Lang::Tr)[0], "🇹🇷");
        assert_eq!(top(&mut s, "turkiye", Lang::En)[0], "🇹🇷");
        // Anahtar sözcük (adda geçmeyen).
        // "lol": İngilizce ad öneki ("lollipop") anahtar sözcük eşleşmesinden (😀 "lol") önce.
        let lol = top(&mut s, "lol", Lang::Tr);
        assert_eq!(lol[0], "🍭");
        assert!(lol.contains(&"😀"), "yalnız anahtar sözcükte geçen de bulunur");
        assert!(s.search(&Usage::default(), "zzzqqq", Lang::Tr, 0).is_empty());
    }

    #[test]
    fn active_language_first_on_equal_strength() {
        let mut s = EmojiSearcher::new();
        // "gül" Türkçede çiçek (🌹 gül), İngilizcede yok; "rose" İngilizcede.
        assert_eq!(top(&mut s, "gül", Lang::Tr)[0], "🌹");
        assert_eq!(top(&mut s, "rose", Lang::En)[0], "🌹");
        // Aynı sorgu iki dilde ad tam eşleşmesiyse etkin dil öne geçer: puanlar.
        let e = &all()[find("🌹").unwrap()];
        let mut t = EmojiSearcher::new();
        t.search(&Usage::default(), "gul", Lang::Tr, 0);
        assert_eq!(t.tier(e, Lang::Tr), Some(TIER_EXACT));
        let tr_score = t.hits[0].score;
        t.search(&Usage::default(), "gul", Lang::En, 0);
        let en_score = t.hits.iter().find(|h| h.index as usize == find("🌹").unwrap()).unwrap().score;
        assert!(tr_score > en_score, "etkin dil tam, diğeri %{SECONDARY_WEIGHT}");
    }

    #[test]
    fn empty_query_orders_by_usage_then_data() {
        let mut s = EmojiSearcher::new();
        let mut u = Usage::default();
        let hits = s.search(&u, "  ", Lang::Tr, 0).to_vec();
        assert_eq!(hits.len(), all().len());
        assert_eq!(all()[hits[0].index as usize].emoji, "😀");
        u.record_emoji("🐱", 100);
        u.record_emoji("🐱", 100);
        u.record_emoji("🌹", 100);
        let order: Vec<_> = s.search(&u, "", Lang::Tr, 100).iter().take(3).map(|h| all()[h.index as usize].emoji).collect();
        assert_eq!(order, ["🐱", "🌹", "😀"]);
    }

    #[test]
    fn usage_breaks_ties_within_tier_only() {
        let mut s = EmojiSearcher::new();
        let mut u = Usage::default();
        let base = s.search(&u, "kalp", Lang::Tr, 1).to_vec();
        let emoji_at = |h: &EmojiHit| all()[h.index as usize].emoji;
        // 🫀'nin Türkçe adı tam "kalp": en üstte.
        assert_eq!(emoji_at(&base[0]), "🫀");
        // Aynı puanlı iki sonuçtan sonraki çok kullanılınca öne geçer, en üsttekini geçemez.
        let (a, b) = (1..base.len()).find_map(|i| (base[i].score == base[i + 1].score).then_some((base[i], base[i + 1]))).unwrap();
        for _ in 0..50 {
            u.record_emoji(emoji_at(&b), 1);
        }
        let boosted = s.search(&u, "kalp", Lang::Tr, 1).to_vec();
        let pos = |e: &str| boosted.iter().position(|h| emoji_at(h) == e).unwrap();
        assert!(pos(emoji_at(&b)) < pos(emoji_at(&a)), "aynı kademede kullanım öne alır");
        assert_eq!(emoji_at(&boosted[0]), "🫀", "kademe atlanmaz");
        // Kademe aralıklarının frecency tavanından büyük olması derlemede denetlenir (`_TIERS_APART`).
    }

    #[test]
    fn query_is_clipped_and_debug_redacted() {
        let mut s = EmojiSearcher::new();
        s.search(&Usage::default(), &"ş".repeat(MAX_QUERY), Lang::Tr, 0);
        assert!(s.query.len() <= MAX_QUERY);
        s.search(&Usage::default(), "gizli-sorgu", Lang::Tr, 0);
        assert!(!format!("{s:?}").contains("gizli"));
    }

    /// Hızın yükten bağımsız güvencesi: arama tek doğrusal tarama, sıcak döngüde ayırma yok.
    /// Isınmadan sonra sonuç tamponu yeniden ayrılmaz (kapasite ve adres sabit), sonuç sayısı
    /// veriyle sınırlı; aynı sorgunun tekrarı sözcük listesini yeniden kurmaz. (Duvar saati
    /// ölçen eski `search_is_fast` derleme yükünde ara sıra düşüyordu: 5552ba1, 7260b43 sınıfı;
    /// süre ölçümü `search_timing`'e, `#[ignore]` ile taşındı.)
    #[test]
    fn search_reuses_buffers_and_is_bounded() {
        let mut s = EmojiSearcher::new();
        let u = Usage::default();
        let queries = ["k", "ka", "kal", "kalp", "kırmızı k", "red heart", "a", "flag", ""];
        s.search(&u, "", Lang::Tr, 0);
        let (cap, ptr) = (s.hits.capacity(), s.hits.as_ptr());
        assert!(cap >= all().len());
        for q in queries {
            let n = s.search(&u, q, Lang::Tr, 0).len();
            assert!(n <= all().len(), "{q}");
            assert_eq!((s.hits.capacity(), s.hits.as_ptr()), (cap, ptr), "sonuç tamponu yeniden ayrılmaz: {q}");
        }
        s.search(&u, "kırmızı kalp", Lang::Tr, 0);
        let terms = s.terms.as_ptr();
        s.search(&u, "  KIRMIZI   kalp ", Lang::Tr, 0);
        assert_eq!(s.terms.as_ptr(), terms, "aynı (katlanmış) sorgu sözcükleri yeniden kurmaz");
    }

    /// Süre ölçümü (yüke duyarlı; varsayılan koşuda yok). Elle: `cargo test --release --lib
    /// emoji::tests::search_timing -- --ignored --nocapture`.
    #[test]
    #[ignore = "duvar saati ölçer; derleme yükünde güvenilmez (yük bağımsız güvence: search_reuses_buffers_and_is_bounded)"]
    fn search_timing() {
        let mut s = EmojiSearcher::new();
        let u = Usage::default();
        all();
        let t = std::time::Instant::now();
        for q in ["k", "ka", "kal", "kalp", "kırmızı k", "red heart", "a", "flag"] {
            s.search(&u, q, Lang::Tr, 0);
        }
        let elapsed = t.elapsed();
        eprintln!("8 sorgu: {elapsed:?} ({} emoji)", all().len());
        assert!(elapsed.as_millis() < 400, "{elapsed:?}");
    }
}
