//! Bildirim gövdesi biçimlendirmesi (Desktop Notifications Spec 1.2, "Markup").
//!
//! Spec'in izin verdiği alt küme [`BodySpan`] listesine çevrilir: `<b>`, `<i>`, `<u>`,
//! `<a href="...">` ve `<img alt="...">` (görsel çizilmez, `alt` metni gösterilir). Diğer tüm
//! etiketler atılır, içerikleri düz metin olarak kalır; `<br>` satır sonu olur. Ayrıştırıcı
//! hatalı girdiye toleranslıdır (kapanmayan/eşleşmeyen etiket, yarım entity, `a < b` gibi düz
//! metin) ve asla panik etmez; çıktı boyutu girdiyle doğrusaldır.
//!
//! Bağlantılar yalnızca `http`, `https` ve `mailto` şemalarında korunur ([`validate_link`]);
//! diğerlerinde metin kalır, bağlantı düşer. Bağlantı açmak UI'nın işidir ve ancak kullanıcı
//! tıklayınca yapılır (bkz. `NotifyHandle::open_link`).

/// Bir metin parçasının biçimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// Aynı biçim ve bağlantıya sahip ardışık gövde metni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodySpan {
    pub text: String,
    pub style: TextStyle,
    /// Doğrulanmış bağlantı (yalnızca http/https/mailto).
    pub link: Option<BodyLink>,
}

/// Doğrulanmış bağlantı hedefi ([`validate_link`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyLink {
    /// Ayrıştırıcının normalleştirdiği URL; açılacak olan tam olarak budur.
    pub url: String,
    /// Gerçek hedef: http(s) için host (IDN punycode olarak), mailto için adres(ler).
    /// Görünen metin hedefi yansıtmayabilir; UI bunu (ör. ipucu/alt satır) göstermeli.
    pub host: String,
}

/// Biçimsiz tek parça gövde.
pub fn plain_body(text: &str) -> Vec<BodySpan> {
    let mut out = Vec::new();
    push(&mut out, &sanitize(text), TextStyle::default(), None);
    out
}

/// Gövdenin biçimsiz metni (arama, erişilebilirlik, kopyalama için).
pub fn body_text(spans: &[BodySpan]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// Metni ekler; son parçayla aynı biçimdeyse birleştirir.
fn push(out: &mut Vec<BodySpan>, text: &str, style: TextStyle, link: Option<&BodyLink>) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.style == style
        && last.link.as_ref() == link
    {
        last.text.push_str(text);
        return;
    }
    out.push(BodySpan { text: text.to_owned(), style, link: link.cloned() });
}

/// İç içe biçim sayacının üst sınırı (kapanmayan binlerce `<b>` taşma yapmasın).
const MAX_DEPTH: u32 = 64;
/// Bir etiketin (`<` ile `>` arası) en uzun hali; daha uzunsa `<` düz metin sayılır.
const MAX_TAG_BYTES: usize = 2048;
/// Kabul edilen en uzun bağlantı.
pub const MAX_LINK_BYTES: usize = 2048;

#[derive(Debug, Default)]
struct State {
    bold: u32,
    italic: u32,
    underline: u32,
    link: Option<BodyLink>,
}

impl State {
    fn style(&self) -> TextStyle {
        TextStyle { bold: self.bold > 0, italic: self.italic > 0, underline: self.underline > 0 }
    }
}

/// Spec alt kümesindeki markup'ı zengin metne çevirir.
pub fn parse_markup(input: &str) -> Vec<BodySpan> {
    let mut out = Vec::new();
    let mut state = State::default();
    let mut text = String::new();
    let mut i = 0;
    let bytes = input.as_bytes();

    while i < input.len() {
        match bytes[i] {
            b'<' => {
                if let Some(consumed) = parse_tag(&input[i..], &mut state, &mut out, &mut text) {
                    i += consumed;
                } else {
                    text.push('<');
                    i += 1;
                }
            }
            b'&' => {
                let (ch, consumed) = decode_entity(&input[i..]).unwrap_or(('&', 1));
                push_char(&mut text, ch);
                i += consumed;
            }
            _ => {
                // `i` her zaman karakter sınırında: yalnızca ASCII baytlarda özel işlem var.
                let ch = input[i..].chars().next().unwrap_or('\u{fffd}');
                push_char(&mut text, ch);
                i += ch.len_utf8();
            }
        }
    }
    flush(&mut out, &state, &mut text);
    out
}

fn flush(out: &mut Vec<BodySpan>, state: &State, text: &mut String) {
    push(out, text, state.style(), state.link.as_ref());
    text.clear();
}

/// Kontrol karakterlerini atar (satır sonu ve sekme kalır, `\r` atılır).
fn push_char(text: &mut String, ch: char) {
    if ch == '\n' || ch == '\t' || !ch.is_control() {
        text.push(ch);
    }
}

/// Kontrol karakterlerinden arındırılmış kopya.
pub fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        push_char(&mut out, ch);
    }
    out
}

/// `input` `<` ile başlar. Etiket tanınırsa işler ve tükettiği bayt sayısını döndürür;
/// etiket değilse (`a < b`, `<3`, kapanmayan `<`) `None` → `<` düz metindir.
fn parse_tag(input: &str, state: &mut State, out: &mut Vec<BodySpan>, text: &mut String) -> Option<usize> {
    let rest = &input[1..];
    if let Some(comment) = rest.strip_prefix("!--") {
        // Yorum: kapanmazsa sonuna kadar atılır.
        return Some(comment.find("-->").map_or(input.len(), |end| 4 + end + 3));
    }
    let first = rest.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '/' || first == '!' || first == '?') {
        return None;
    }
    let end = rest.bytes().take(MAX_TAG_BYTES).position(|b| b == b'>')?;
    let inner = &rest[..end];
    let consumed = end + 2;

    let (closing, body) = match inner.strip_prefix('/') {
        Some(body) => (true, body),
        None => (false, inner),
    };
    let name_len = body.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(body.len());
    let name = body[..name_len].to_ascii_lowercase();
    let attrs = &body[name_len..];

    match name.as_str() {
        "b" | "i" | "u" => {
            flush(out, state, text);
            let counter = match name.as_str() {
                "b" => &mut state.bold,
                "i" => &mut state.italic,
                _ => &mut state.underline,
            };
            if closing {
                *counter = counter.saturating_sub(1);
            } else if !attrs.trim_end().ends_with('/') {
                *counter = (*counter + 1).min(MAX_DEPTH);
            }
        }
        "a" => {
            flush(out, state, text);
            state.link = if closing { None } else { attr(attrs, "href").and_then(|h| validate_link(&h)) };
        }
        "img" if !closing => {
            if let Some(alt) = attr(attrs, "alt") {
                text.push_str(&sanitize(&alt));
            }
        }
        "br" => text.push('\n'),
        // Bilinmeyen etiket (ve `<!DOCTYPE>`, `<?xml?>`): atılır, içerik metin olarak kalır.
        _ => {}
    }
    Some(consumed)
}

/// Etiket özniteliğini okur: `key="v"`, `key='v'` veya `key=v`. Değerdeki entity'ler çözülür.
fn attr(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if rest.is_empty() {
            return None;
        }
        let name_end = rest.find(|c: char| c == '=' || c.is_whitespace() || c == '/').unwrap_or(rest.len());
        let name = &rest[..name_end];
        rest = rest[name_end..].trim_start();
        let value = if let Some(after_eq) = rest.strip_prefix('=') {
            let after_eq = after_eq.trim_start();
            let (value, remaining) = match after_eq.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let body = &after_eq[1..];
                    let close = body.find(q).unwrap_or(body.len());
                    (&body[..close], body.get(close + 1..).unwrap_or(""))
                }
                _ => {
                    let close = after_eq.find(char::is_whitespace).unwrap_or(after_eq.len());
                    (&after_eq[..close], &after_eq[close..])
                }
            };
            rest = remaining;
            Some(value)
        } else {
            None
        };
        if name.eq_ignore_ascii_case(key) {
            return value.map(decode_entities);
        }
    }
}

/// Metindeki tüm entity'leri çözer (öznitelik değerleri için).
fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input.as_bytes()[i] == b'&' {
            let (ch, consumed) = decode_entity(&input[i..]).unwrap_or(('&', 1));
            out.push(ch);
            i += consumed;
        } else {
            let ch = input[i..].chars().next().unwrap_or('\u{fffd}');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// `input` `&` ile başlar. XML'in beş adlandırılmış entity'si ve sayısal başvurular
/// (`&#38;`, `&#x26;`) çözülür; tanınmayan veya yarım entity → `None` (`&` düz metin).
fn decode_entity(input: &str) -> Option<(char, usize)> {
    let end = input.bytes().take(12).position(|b| b == b';')?;
    let name = &input[1..end];
    let ch = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse::<u32>().ok()?,
            };
            let ch = char::from_u32(code)?;
            if ch == '\0' {
                return None;
            }
            ch
        }
    };
    Some((ch, end + 1))
}

/// Görünmez biçim/yön karakterleri (sıfır genişlik, bidi geçersiz kılma, BOM): bağlantıyı
/// görünende farklı gösterebilir.
fn is_invisible_format(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
}

/// Bağlantıyı doğrular ve normalleştirir. Girdi entity'leri çözülmüş ham değerdir (ör.
/// `&#106;avascript:` buraya `javascript:` olarak gelir). Kurallar:
/// - [`MAX_LINK_BYTES`] sınırı; boşluk, kontrol, görünmez biçim karakteri ve `\` yok (baştaki
///   boşluk/NUL kırpılmaz, reddedilir; WHATWG ayrıştırıcısının `\` → `/` dönüşümüne güvenilmez).
/// - `url` crate'iyle (WHATWG) ayrıştırılır; şema allowlist: `http`, `https`, `mailto`.
/// - http(s): host zorunlu; kullanıcı adı/parola (`https://good@evil`) reddedilir.
/// - mailto: adres zorunlu ve `@` içerir; sorgu (`?attach=`, `?body=` başlıkları) reddedilir.
/// - Açılacak olan normalleştirilmiş URL'dir ([`BodyLink::url`]).
pub fn validate_link(raw: &str) -> Option<BodyLink> {
    if raw.is_empty()
        || raw.len() > MAX_LINK_BYTES
        || raw.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\' || is_invisible_format(c))
    {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    let host = match url.scheme() {
        "http" | "https" => {
            // Ayrıştırıcının onardığı biçimler (`https:/evil`, `https:///evil`) kabul edilmez:
            // ham metin tam olarak `şema://host` biçiminde olmalı.
            let authority = raw.split_once(':').map_or("", |(_, rest)| rest);
            if !authority.starts_with("//") || authority[2..].starts_with('/') {
                return None;
            }
            // Kullanıcı bilgisi hiç kabul edilmez; ayrıştırıcının sildiği boş `:@` dahil.
            if authority[2..].split(['/', '?', '#']).next().is_some_and(|a| a.contains('@')) {
                return None;
            }
            if !url.username().is_empty() || url.password().is_some() {
                return None;
            }
            url.host_str().filter(|h| !h.is_empty())?.to_owned()
        }
        "mailto" => {
            let address = url.path();
            if url.query().is_some() || address.is_empty() || !address.contains('@') {
                return None;
            }
            address.to_owned()
        }
        _ => return None,
    };
    let normalized = url.to_string();
    (normalized.len() <= MAX_LINK_BYTES).then_some(BodyLink { url: normalized, host })
}

/// Bağlantının açılmasına izin verilir mi ([`validate_link`]).
pub fn is_safe_link(url: &str) -> bool {
    validate_link(url).is_some()
}

/// `s`'yi en fazla `max` karaktere kısaltır (karakter sınırında).
pub fn truncate_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// `s`'yi en fazla `max` bayta kısaltır (karakter sınırında).
pub fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
