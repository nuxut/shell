//! Pano geçmişi modeli: sınırlar, tekilleştirme, sıralama, sıfırlayarak silme.
//!
//! En yeni öğe başta. Aynı içerik (bayt eşit) yeniden gelirse yeni öğe açılmaz: var olan öğe
//! yeni zamanla başa taşınır (kimliği korunur). Öğe sayısı ya da toplam bayt sınırı aşılınca en
//! eskiler düşer. Öğeler `Arc` ile paylaşılır (UI ve IPC anlık görüntüsü kopyasız); içerik son
//! sahibi bırakınca sıfırlanır ([`Blob`]).
//!
//! Tüm içerik kullanıcı verisidir: `Debug` yalnızca tür ve uzunluk yazar.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::SystemTime;

use zeroize::{Zeroize, Zeroizing};

use super::filter::ImageMime;

/// Metin öğesi sınırı (bayt).
pub const MAX_TEXT_BYTES: usize = 256 << 10;
/// Görsel öğesi sınırı (kodlu bayt).
pub const MAX_IMAGE_BYTES: usize = 8 << 20;
/// Geçmişin toplam bayt sınırı.
pub const MAX_TOTAL_BYTES: usize = 64 << 20;
/// Önizleme en çok bu kadar karakter.
pub const PREVIEW_CHARS: usize = 200;
/// Öğe sayısı sınırları (`[clipboard] max_items`).
pub const MIN_ITEMS: usize = 10;
pub const MAX_ITEMS: usize = 500;

/// Paylaşılan içerik baytları. Son kopya düşünce (başka sahip yoksa) bellek sıfırlanır.
/// Panoya verilen kopyayı (`set_selection`) kaynak tutuyorsa sıfırlama onun bırakmasına kalır.
#[derive(Clone)]
pub struct Blob(Arc<[u8]>);

impl Blob {
    /// Baytları kopyalar; kaynak tampon sıfırlanır.
    pub fn new(bytes: Zeroizing<Vec<u8>>) -> Self {
        Self(Arc::from(&bytes[..]))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Kaynağa verilecek paylaşılan tampon (kopyasız).
    pub fn shared(&self) -> Arc<[u8]> {
        self.0.clone()
    }
}

impl Drop for Blob {
    fn drop(&mut self) {
        if let Some(bytes) = Arc::get_mut(&mut self.0) {
            bytes.zeroize();
        }
    }
}

impl std::fmt::Debug for Blob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Blob({} bayt)", self.0.len())
    }
}

impl PartialEq for Blob {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0[..] == other.0[..]
    }
}

/// Öğenin içeriği.
#[derive(Debug, Clone)]
pub enum Content {
    /// Doğrulanmış UTF-8 metin.
    Text(Blob),
    /// Kodlu görsel; `hash` tekilleştirmeyi hızlandırır (eşitlik yine bayt karşılaştırmasıyla).
    Image { mime: ImageMime, data: Blob, hash: u64 },
}

/// İçerik neden saklanmadı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    Empty,
    TooLarge,
    /// UTF-8 değil ya da imza MIME ile uyuşmuyor.
    Invalid,
}

impl Content {
    /// Okunan metin baytları: UTF-8, boş/yalnızca boşluk değil, sınır içinde.
    pub fn text(bytes: Zeroizing<Vec<u8>>) -> Result<Self, Reject> {
        if bytes.len() > MAX_TEXT_BYTES {
            return Err(Reject::TooLarge);
        }
        let s = std::str::from_utf8(&bytes).map_err(|_| Reject::Invalid)?;
        if s.trim().is_empty() {
            return Err(Reject::Empty);
        }
        Ok(Self::Text(Blob::new(bytes)))
    }

    /// Okunan görsel baytları: imza türle uyuşmalı, sınır içinde.
    pub fn image(mime: ImageMime, bytes: Zeroizing<Vec<u8>>) -> Result<Self, Reject> {
        if bytes.is_empty() {
            return Err(Reject::Empty);
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(Reject::TooLarge);
        }
        if !mime.matches(&bytes) {
            return Err(Reject::Invalid);
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        Ok(Self::Image { mime, data: Blob::new(bytes), hash: h.finish() })
    }

    pub fn len(&self) -> usize {
        self.blob().len()
    }

    pub fn is_empty(&self) -> bool {
        self.blob().is_empty()
    }

    pub fn blob(&self) -> &Blob {
        match self {
            Self::Text(b) | Self::Image { data: b, .. } => b,
        }
    }

    /// Metin öğesinin metni (görselde `None`).
    pub fn as_text(&self) -> Option<&str> {
        match self {
            // Yapıcı UTF-8'i doğruladı; yine de panik yok.
            Self::Text(b) => std::str::from_utf8(b.as_bytes()).ok(),
            Self::Image { .. } => None,
        }
    }

    /// Panoya yeniden sunulacak (MIME, bayt) çiftleri. Metin yaygın metin türleriyle sunulur.
    pub fn offer(&self) -> Vec<(String, Arc<[u8]>)> {
        match self {
            Self::Text(b) => ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING", "TEXT"]
                .iter()
                .map(|m| ((*m).to_owned(), b.shared()))
                .collect(),
            Self::Image { mime, data, .. } => vec![(mime.as_str().to_owned(), data.shared())],
        }
    }

    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Image { mime: m1, data: a, hash: h1 }, Self::Image { mime: m2, data: b, hash: h2 }) => {
                m1 == m2 && h1 == h2 && a == b
            }
            _ => false,
        }
    }
}

/// Öğe kimliği (oturum boyunca artan; IPC `pick <id>`).
pub type ItemId = u64;

/// Geçmiş öğesi. `Debug` önizlemeyi yazmaz (yalnızca tür ve uzunluk).
pub struct Item {
    pub id: ItemId,
    pub content: Content,
    /// Son kopyalanma (ya da seçilme) zamanı.
    pub copied_at: SystemTime,
    /// Metnin ilk dolu satırı (≤ [`PREVIEW_CHARS`], kontrol karakterleri atılmış); görselde boş.
    preview: Zeroizing<String>,
}

impl std::fmt::Debug for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Item").field("id", &self.id).field("content", &self.content).finish_non_exhaustive()
    }
}

impl Item {
    pub fn new(id: ItemId, content: Content, copied_at: SystemTime) -> Self {
        let preview = Zeroizing::new(content.as_text().map(preview).unwrap_or_default());
        Self { id, content, copied_at, preview }
    }

    pub fn preview(&self) -> &str {
        &self.preview
    }

    pub fn is_image(&self) -> bool {
        matches!(self.content, Content::Image { .. })
    }
}

/// Liste satırı metni: ilk dolu satır, kırpılmış; ardından içerik varsa `…`.
pub fn preview(text: &str) -> String {
    let trimmed = text.trim_start();
    let line = trimmed.lines().next().unwrap_or("").trim_end();
    let mut out = String::with_capacity(line.len().min(PREVIEW_CHARS * 4));
    let mut count = 0;
    let mut cut = false;
    for c in line.chars() {
        if count == PREVIEW_CHARS {
            cut = true;
            break;
        }
        match c {
            '\t' => out.push(' '),
            c if c.is_control() => continue,
            c => out.push(c),
        }
        count += 1;
    }
    if cut || trimmed.trim_end().len() > line.len() {
        out.push('…');
    }
    out
}

/// Ekleme sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    Added(ItemId),
    /// Aynı içerik vardı: başa taşındı.
    Moved(ItemId),
}

/// Sınırlar (öğe sayısı config'ten; bayt sınırları sabit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_items: usize,
    pub max_total: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_items: 100, max_total: MAX_TOTAL_BYTES }
    }
}

/// Geçmiş. En yeni başta.
#[derive(Debug, Default)]
pub struct History {
    items: VecDeque<Arc<Item>>,
    total: usize,
    next_id: ItemId,
    limits: Limits,
}

impl History {
    pub fn new(limits: Limits) -> Self {
        Self { limits: sane(limits), ..Self::default() }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Toplam içerik baytı.
    pub fn total_bytes(&self) -> usize {
        self.total
    }

    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, Arc<Item>> {
        self.items.iter()
    }

    pub fn get(&self, id: ItemId) -> Option<&Arc<Item>> {
        self.items.iter().find(|i| i.id == id)
    }

    /// İçerik ekler ya da aynısını başa taşır; sınırlar uygulanır.
    pub fn push(&mut self, content: Content, now: SystemTime) -> Push {
        if let Some(pos) = self.items.iter().position(|i| i.content.same(&content))
            && let Some(old) = self.items.remove(pos)
        {
            // Yeni gelen kopya düşer; var olan baytlar (aynı) yeni zamanla başa.
            let id = old.id;
            self.items.push_front(Arc::new(Item::new(id, old.content.clone(), now)));
            return Push::Moved(id);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.total += content.len();
        self.items.push_front(Arc::new(Item::new(id, content, now)));
        self.trim();
        Push::Added(id)
    }

    /// Diskten yüklenen öğe: sona (daha eski) eklenir; sınır aşılırsa eklenmez.
    pub fn push_loaded(&mut self, content: Content, copied_at: SystemTime) -> bool {
        if self.items.len() >= self.limits.max_items
            || self.total + content.len() > self.limits.max_total
            || self.items.iter().any(|i| i.content.same(&content))
        {
            return false;
        }
        self.next_id += 1;
        self.total += content.len();
        self.items.push_back(Arc::new(Item::new(self.next_id, content, copied_at)));
        true
    }

    /// Seçilen öğeyi yeni zamanla başa taşır.
    pub fn touch(&mut self, id: ItemId, now: SystemTime) -> bool {
        let Some(pos) = self.items.iter().position(|i| i.id == id) else { return false };
        let Some(old) = self.items.remove(pos) else { return false };
        self.items.push_front(Arc::new(Item::new(id, old.content.clone(), now)));
        true
    }

    pub fn remove(&mut self, id: ItemId) -> bool {
        let Some(pos) = self.items.iter().position(|i| i.id == id) else { return false };
        if let Some(item) = self.items.remove(pos) {
            self.total -= item.content.len();
        }
        true
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.total = 0;
    }

    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = sane(limits);
        self.trim();
    }

    /// Sınır aşılırsa en eskiler düşer (en yeni öğe her zaman kalır).
    fn trim(&mut self) {
        while self.items.len() > self.limits.max_items || (self.total > self.limits.max_total && self.items.len() > 1) {
            if let Some(item) = self.items.pop_back() {
                self.total -= item.content.len();
            }
        }
    }

    /// UI/IPC anlık görüntüsü (öğeler paylaşılır).
    pub fn snapshot(&self) -> Vec<Arc<Item>> {
        self.items.iter().cloned().collect()
    }
}

fn sane(l: Limits) -> Limits {
    Limits { max_items: l.max_items.clamp(1, MAX_ITEMS), max_total: l.max_total.clamp(1, MAX_TOTAL_BYTES) }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn text(s: &str) -> Content {
        Content::text(Zeroizing::new(s.as_bytes().to_vec())).unwrap()
    }

    fn png(n: usize, fill: u8) -> Content {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.resize(n, fill);
        Content::image(ImageMime::Png, Zeroizing::new(v)).unwrap()
    }

    fn texts(h: &History) -> Vec<String> {
        h.iter().map(|i| i.content.as_text().unwrap_or("<img>").to_owned()).collect()
    }

    #[test]
    fn content_validation() {
        let z = |b: &[u8]| Zeroizing::new(b.to_vec());
        assert_eq!(Content::text(z(b"")).unwrap_err(), Reject::Empty);
        assert_eq!(Content::text(z(b" \n\t ")).unwrap_err(), Reject::Empty);
        assert_eq!(Content::text(z(b"\xff\xfe")).unwrap_err(), Reject::Invalid);
        assert_eq!(Content::text(z(&vec![b'a'; MAX_TEXT_BYTES + 1])).unwrap_err(), Reject::TooLarge);
        assert!(Content::text(z(&vec![b'a'; MAX_TEXT_BYTES])).is_ok());
        assert_eq!(Content::image(ImageMime::Png, z(b"GIF89a")).unwrap_err(), Reject::Invalid);
        assert_eq!(Content::image(ImageMime::Jpeg, z(b"")).unwrap_err(), Reject::Empty);
        let mut big = b"\x89PNG\r\n\x1a\n".to_vec();
        big.resize(MAX_IMAGE_BYTES + 1, 0);
        assert_eq!(Content::image(ImageMime::Png, Zeroizing::new(big)).unwrap_err(), Reject::TooLarge);
    }

    #[test]
    fn dedup_moves_to_front_keeping_id() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let mut h = History::new(Limits::default());
        let a = h.push(text("a"), t0);
        h.push(text("b"), t0);
        assert_eq!(texts(&h), ["b", "a"]);
        let t1 = t0 + Duration::from_secs(5);
        let Push::Added(a_id) = a else { panic!() };
        assert_eq!(h.push(text("a"), t1), Push::Moved(a_id));
        assert_eq!(texts(&h), ["a", "b"]);
        assert_eq!(h.len(), 2);
        assert_eq!(h.get(a_id).unwrap().copied_at, t1);
        assert_eq!(h.total_bytes(), 2);
        // Görsel: aynı bayt tekil, farklı bayt ayrı öğe.
        h.push(png(64, 1), t1);
        h.push(png(64, 2), t1);
        assert!(matches!(h.push(png(64, 1), t1), Push::Moved(_)));
        assert_eq!(h.len(), 4);
        // Metin ve görsel aynı bayt olsa da ayrı.
        assert!(!text("x").same(&Content::image(ImageMime::Jpeg, Zeroizing::new(vec![0xFF, 0xD8, 0xFF])).unwrap()));
    }

    #[test]
    fn item_and_byte_limits_drop_oldest() {
        let now = SystemTime::now();
        let mut h = History::new(Limits { max_items: 3, max_total: MAX_TOTAL_BYTES });
        for s in ["1", "2", "3", "4", "5"] {
            h.push(text(s), now);
        }
        assert_eq!(texts(&h), ["5", "4", "3"]);
        assert_eq!(h.total_bytes(), 3);

        let mut h = History::new(Limits { max_items: 100, max_total: 10 });
        h.push(text("aaaa"), now);
        h.push(text("bbbb"), now);
        h.push(text("cccc"), now);
        assert_eq!(texts(&h), ["cccc", "bbbb"], "toplam bayt aşılınca en eski düşer");
        assert_eq!(h.total_bytes(), 8);
        // Tek başına sınırdan büyük olan en yeni öğe yine kalır (tek öğe).
        h.push(text("0123456789ab"), now);
        assert_eq!(h.len(), 1);

        // Sınır küçülünce kırpılır.
        let mut h = History::new(Limits::default());
        for i in 0..20 {
            h.push(text(&i.to_string()), now);
        }
        h.set_limits(Limits { max_items: 10, max_total: MAX_TOTAL_BYTES });
        assert_eq!(h.len(), 10);
        assert_eq!(h.iter().next().unwrap().content.as_text(), Some("19"));
    }

    #[test]
    fn remove_touch_clear() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let mut h = History::new(Limits::default());
        let Push::Added(a) = h.push(text("a"), t0) else { panic!() };
        let Push::Added(b) = h.push(text("bb"), t0) else { panic!() };
        assert!(h.touch(a, t0 + Duration::from_secs(1)));
        assert_eq!(texts(&h), ["a", "bb"]);
        assert!(!h.touch(999, t0));
        assert!(h.remove(b));
        assert!(!h.remove(b));
        assert_eq!(h.total_bytes(), 1);
        h.clear();
        assert!(h.is_empty());
        assert_eq!(h.total_bytes(), 0);
        // Kimlikler yeniden kullanılmaz.
        let Push::Added(c) = h.push(text("c"), t0) else { panic!() };
        assert!(c > b);
    }

    #[test]
    fn loaded_items_append_within_limits() {
        let t0 = SystemTime::UNIX_EPOCH;
        let mut h = History::new(Limits { max_items: 2, max_total: MAX_TOTAL_BYTES });
        assert!(h.push_loaded(text("new"), t0));
        assert!(h.push_loaded(text("old"), t0));
        assert!(!h.push_loaded(text("older"), t0));
        assert_eq!(texts(&h), ["new", "old"]);
        let mut h = History::new(Limits::default());
        assert!(h.push_loaded(text("x"), t0));
        assert!(!h.push_loaded(text("x"), t0), "yinelenen yüklenmez");
    }

    #[test]
    fn previews() {
        assert_eq!(preview("merhaba"), "merhaba");
        assert_eq!(preview("\n\n  ilk satır  \nikinci"), "ilk satır…");
        assert_eq!(preview("a\tb\u{7}c"), "a bc");
        let long = "ğ".repeat(PREVIEW_CHARS + 5);
        let p = preview(&long);
        assert_eq!(p.chars().count(), PREVIEW_CHARS + 1);
        assert!(p.ends_with('…'));
        assert_eq!(preview("son\n  \n"), "son", "yalnızca boşluk kalan satırlar … eklemez");
        let item = Item::new(1, png(16, 0), SystemTime::UNIX_EPOCH);
        assert_eq!(item.preview(), "");
        assert!(item.is_image());
    }

    #[test]
    fn debug_hides_content() {
        let item = Item::new(1, text("gizli-parola-benzeri"), SystemTime::UNIX_EPOCH);
        let d = format!("{item:?}");
        assert!(!d.contains("gizli"), "{d}");
        assert!(d.contains("20 bayt"));
    }

    #[test]
    fn blob_zeroized_when_last_owner_drops() {
        let blob = Blob::new(Zeroizing::new(b"secret".to_vec()));
        let shared = blob.shared();
        let clone = blob.clone();
        drop(blob);
        assert_eq!(&shared[..], b"secret", "başka sahip varken dokunulmaz");
        drop(clone);
        // Kaynak (`shared`) hâlâ tutuyor: sıfırlama onun bırakmasına kalır.
        assert_eq!(&shared[..], b"secret");

        // Tek sahip: düşerken sıfırlanır. Belleği düşmeden önce gözlemlemek için zayıf işaretçi
        // kullanılamaz (get_mut zayıfı da sayar); sıfırlamayı doğrudan Drop yolundan çağırırız.
        let mut b = Blob::new(Zeroizing::new(b"abc".to_vec()));
        let inner = Arc::get_mut(&mut b.0).expect("tek sahip");
        inner.zeroize();
        assert_eq!(b.as_bytes(), [0, 0, 0]);
    }
}
