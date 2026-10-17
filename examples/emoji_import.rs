//! Emoji verisini ibus sözlüklerinden repo verisine bir kez dönüştürür (çalışma zamanında ve
//! derlemede kullanılmaz; sisteme bağımlılık yok).
//!
//! ```sh
//! cargo run --example emoji_import -- \
//!     /usr/share/ibus/dicts/emoji-en.dict /usr/share/ibus/dicts/emoji-tr.dict \
//!     src/services/launcher/data/emoji.tsv
//! ```
//!
//! Kaynak: ibus paketinin `emoji-<dil>.dict` dosyaları (Unicode CLDR annotations +
//! `emoji-test.txt` sırası; ibus 1.5.34'te CLDR verisinden üretilir). Dosya biçimi GVariant
//! `av`: her öğe `(s tür adı, a{sv} ekler, s emoji, u n, s×n açıklama sözcüğü, s ad, s grup)`.
//! Bu araç tam bir GVariant ayrıştırıcısı değildir: her kaydı alanlarıyla okur ve kaydın
//! ardındaki tür imzasını (`(sa{sv}su` + `s`×(n+2) + `)`) doğrular; tutmayan kayıt hata verir.
//!
//! Çıktı: satır başına bir emoji, sekmeyle ayrılmış
//! `emoji  grup  en_ad  en_sözcükler  tr_ad  tr_sözcükler` (sözcükler `|` ile; adla aynı olan
//! sözcük atılır). Sıra İngilizce sözlüğün sırasıdır (emoji-test.txt).
//!
//! Seçim (v1 kapsamı, docs/design/emoji.md):
//! - Aynı emojinin nitelik biçimleri (U+FE0F'li/siz) tek satır olur: en çok U+FE0F taşıyan
//!   (tam nitelikli) biçim yazılır, açıklama sözcükleri birleşir.
//! - Ten rengi varyantları (U+1F3FB..U+1F3FF içeren diziler) ve tek başına bileşenler
//!   (ten/saç) atılır: v1 taban emojidir.
//! - emoji-test yorumundan gelen `E15.1 ad` biçimli adların sürüm öneki atılır.

use std::collections::HashMap;
use std::fmt::Write as _;

const MAGIC: &[u8] = b"IBusEmojiData\0";

#[derive(Debug)]
struct Record {
    emoji: String,
    keywords: Vec<String>,
    name: String,
    group: String,
}

fn cstr(data: &[u8], at: &mut usize) -> Result<String, String> {
    let end = data[*at..].iter().position(|&b| b == 0).ok_or("NUL yok")? + *at;
    let s = std::str::from_utf8(&data[*at..end]).map_err(|e| format!("UTF-8 değil @{at}: {e}"))?;
    *at = end + 1;
    Ok(s.to_owned())
}

fn align(start: usize, at: usize, to: usize) -> usize {
    let rel = at - start;
    start + rel.div_ceil(to) * to
}

fn find(data: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    data.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn parse(data: &[u8]) -> Result<Vec<Record>, String> {
    let mut out = Vec::new();
    // Dosya başlığı da tür adıyla başlar; kayıtlar ondan sonra.
    let mut at = find(data, 1, MAGIC).ok_or("kayıt yok")?;
    loop {
        let start = at;
        if start % 8 != 0 {
            return Err(format!("kayıt hizasız @{start}"));
        }
        at += MAGIC.len();
        at = align(start, at, 8); // a{sv} (boş) ve ardındaki emoji dizgisi
        let emoji = cstr(data, &mut at)?;
        at = align(start, at, 4);
        let n = u32::from_le_bytes(data.get(at..at + 4).ok_or("kısa")?.try_into().unwrap()) as usize;
        at += 4;
        if n > 256 {
            return Err(format!("{emoji}: {n} sözcük (bozuk?)"));
        }
        let mut strings = Vec::with_capacity(n + 2);
        for _ in 0..n + 2 {
            strings.push(cstr(data, &mut at)?);
        }
        // Kaydın tür imzası: yalnız çerçeve ofsetleri (≤ 2 bayt × alan) arada kalabilir.
        let sig = format!("\0(sa{{sv}}su{})", "s".repeat(n + 2));
        let sig_at = find(data, at, sig.as_bytes()).ok_or_else(|| format!("{emoji}: tür imzası yok"))?;
        if sig_at - at > 2 * (n + 6) {
            return Err(format!("{emoji}: tür imzası beklenenden uzakta ({} bayt)", sig_at - at));
        }
        let group = strings.pop().unwrap();
        let name = strings.pop().unwrap();
        out.push(Record { emoji, keywords: strings, name, group });
        at = sig_at + sig.len();
        match find(data, at, MAGIC) {
            // Sonraki kayıt en çok bir hizalama + dizi çerçevesi ötede olmalı.
            Some(next) if next - at < 16 => at = next,
            Some(next) => return Err(format!("kayıtlar arasında beklenmeyen {} bayt @{at}", next - at)),
            None => break,
        }
    }
    Ok(out)
}

fn is_skin_tone(c: char) -> bool {
    ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
}

/// Tek başına bileşen: ten tonu ya da saç bileşeni (U+1F9B0..U+1F9B3) — seçicide gösterilmez.
fn is_component(e: &str) -> bool {
    let mut cs = e.chars().filter(|&c| c != '\u{FE0F}');
    match (cs.next(), cs.next()) {
        (Some(c), None) => is_skin_tone(c) || ('\u{1F9B0}'..='\u{1F9B3}').contains(&c),
        _ => false,
    }
}

/// Nitelik biçimlerinden bağımsız anahtar (U+FE0F atılır).
fn base_key(e: &str) -> String {
    e.chars().filter(|&c| c != '\u{FE0F}').collect()
}

fn fe0f_count(e: &str) -> usize {
    e.chars().filter(|&c| c == '\u{FE0F}').count()
}

/// Yazılan en yeni emoji sürümü: dağıtımın renkli fontları (Noto Color Emoji 2.051, Twemoji)
/// Emoji 16.0'ı kapsar; daha yeni emoji kutu (tofu) çizilirdi. Font güncellenince yükseltilir.
const MAX_EMOJI_VERSION: f32 = 16.0;

/// `E17.0 apple core` → 17.0 (CLDR adı henüz olmayan, emoji-test yorumundan gelen kayıtlar).
fn version(name: &str) -> Option<f32> {
    let v = name.strip_prefix('E')?.split_once(' ')?.0;
    v.parse().ok()
}

/// `E15.1 man walking` → `man walking` (emoji-test yorumundan gelen sürüm öneki).
fn strip_version(name: &str) -> &str {
    let Some(rest) = name.strip_prefix('E') else { return name };
    match rest.split_once(' ') {
        Some((v, tail)) if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.') => tail,
        _ => name,
    }
}

/// Sekme/satır sonu/ayırıcı karakterleri temizler.
fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' || c == '|' { ' ' } else { c }).collect::<String>().trim().to_owned()
}

/// Bir emojinin birleşik kaydı (bir dil).
#[derive(Default)]
struct Merged {
    emoji: String,
    name: String,
    group: String,
    keywords: Vec<String>,
}

impl Merged {
    fn add(&mut self, r: &Record) {
        let name = clean(strip_version(&r.name));
        // Tam nitelikli biçim (en çok U+FE0F) ve gerçek CLDR adı (sürüm öneksiz) yeğlenir.
        let versioned = strip_version(&r.name).len() != r.name.len();
        let better = self.emoji.is_empty() || fe0f_count(&r.emoji) > fe0f_count(&self.emoji);
        if better {
            self.emoji = r.emoji.clone();
        }
        if self.name.is_empty() || (!versioned && better) {
            self.name = name;
        }
        if self.group.is_empty() {
            self.group = clean(&r.group);
        }
        for k in &r.keywords {
            let k = clean(k);
            if !k.is_empty() && !self.keywords.contains(&k) {
                self.keywords.push(k);
            }
        }
    }

    fn keywords(&self) -> String {
        let name = self.name.to_lowercase();
        self.keywords.iter().filter(|k| k.to_lowercase() != name).cloned().collect::<Vec<_>>().join("|")
    }
}

/// Kayıtları taban anahtarına göre birleştirir; sıra ilk görülüşündür.
fn merge(records: &[Record]) -> (Vec<String>, HashMap<String, Merged>, usize, usize) {
    let mut order = Vec::new();
    let mut map: HashMap<String, Merged> = HashMap::new();
    let (mut skin, mut component) = (0, 0);
    for r in records {
        if is_component(&r.emoji) || r.group == "Component" {
            component += 1;
            continue;
        }
        if r.emoji.chars().any(is_skin_tone) {
            skin += 1;
            continue;
        }
        if version(&r.name).is_some_and(|v| v > MAX_EMOJI_VERSION) {
            continue;
        }
        let key = base_key(&r.emoji);
        if !map.contains_key(&key) {
            order.push(key.clone());
        }
        map.entry(key).or_default().add(r);
    }
    (order, map, skin, component)
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [en, tr, out] = args.as_slice() else {
        return Err("kullanım: emoji_import <emoji-en.dict> <emoji-tr.dict> <çıktı.tsv>".into());
    };
    let en_records = parse(&std::fs::read(en).map_err(|e| e.to_string())?)?;
    let tr_records = parse(&std::fs::read(tr).map_err(|e| e.to_string())?)?;
    let (order, en, skin, component) = merge(&en_records);
    let (_, tr, _, _) = merge(&tr_records);

    let mut text = String::from(
        "# Nuxut emoji verisi: examples/emoji_import.rs ile ibus 1.5.34 emoji-en/tr.dict'ten üretildi (elle düzenlemeyin).\n\
         # Kaynak veri: Unicode CLDR annotations + emoji-test.txt (Unicode License v3, LICENSE-UNICODE).\n\
         # emoji\tgrup\ten_ad\ten_sözcükler\ttr_ad\ttr_sözcükler\n",
    );
    let (mut kept, mut no_name, mut no_tr) = (0, 0, 0);
    for key in &order {
        let e = &en[key];
        // Grupsuz kayıtlar CLDR'nin emoji olmayan sembol açıklamalarıdır (noktalama: `{`, `—`, `、`).
        if e.name.is_empty() || e.group.is_empty() {
            no_name += 1;
            continue;
        }
        let (tr_name, tr_kw) = match tr.get(key) {
            Some(t) if !t.name.is_empty() => (t.name.clone(), t.keywords()),
            _ => {
                no_tr += 1;
                (String::new(), String::new())
            }
        };
        let _ = writeln!(text, "{}\t{}\t{}\t{}\t{}\t{}", e.emoji, e.group, e.name, e.keywords(), tr_name, tr_kw);
        kept += 1;
    }
    std::fs::write(out, &text).map_err(|e| e.to_string())?;
    eprintln!(
        "en kayıt {}, tr kayıt {}; yazılan {kept} (ten varyantı {skin}, bileşen {component}, grupsuz sembol {no_name} atıldı; tr adı eksik {no_tr}); {} bayt",
        en_records.len(),
        tr_records.len(),
        text.len()
    );
    Ok(())
}
