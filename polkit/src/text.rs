//! Diyalogda gösterilen dış metinlerin temizliği (oltalama sınırı, tasarım özellik 7).
//!
//! polkitd'den (eylem mesajı), yardımcıdan (PAM metinleri) ve `/proc/<pid>/comm`'dan
//! (isteyen süreç; süreç kendi adını seçer) gelen metin düz metin olarak çizilir — framework
//! etiketlerinde biçimlendirme dili yok. Burada ayrıca:
//! - denetim karakterleri (C0, DEL, C1) boşluğa döner, ardışık boşluklar teke iner;
//! - yön denetimi karakterleri (LRE/RLE/PDF/LRO/RLO, LRI/RLI/FSI/PDI, LRM/RLM/ALM) ve sıfır
//!   genişlikli biçim karakterleri atılır: metnin görünen sırasını değiştirip başka bir eylem
//!   gibi göstermesin;
//! - uzunluk karakter sayısıyla kırpılır, kırpıldıysa `…` eklenir.
//!
//! **Tanınması gereken** metinler (yürütülebilir yol, süreç adı, giriş adı, eylem kimliği)
//! temizlenmez, [`escape`] edilir: düz ASCII (`!`..=`~`) dışındaki her karakter `\u{…}` olur —
//! homoglyph (Kiril `е`), boşluk, satır ve yön karakterleri görünür, sessizce atılmaz. Bunlar
//! **ortadan** kırpılır ([`escape_middle`], [`display_path`]): yolda kök bileşen ve dosya adı
//! her zaman görünür; baştan kırpma `~/.cache/x/usr/bin/app`'i `…/usr/bin/app` gösterirdi.
//! Çevrilmiş metinler (eylem mesajı, PAM metinleri, GECOS adı) [`sanitize`] ile kalır.

/// Eylem mesajı için üst sınır (karakter).
pub const MAX_MESSAGE_CHARS: usize = 300;
/// PAM metinleri (istem, hata, bilgi).
pub const MAX_PAM_CHARS: usize = 160;
/// Kısa alanlar (eylem kimliği, süreç adı, kullanıcı adı).
pub const MAX_SHORT_CHARS: usize = 96;

fn is_format_control(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}' // ZWSP, ZWNJ, ZWJ, LRM, RLM
        | '\u{202A}'..='\u{202E}' // LRE, RLE, PDF, LRO, RLO
        | '\u{2060}'..='\u{2064}' // WJ, görünmez işleçler
        | '\u{2066}'..='\u{2069}' // LRI, RLI, FSI, PDI
        | '\u{061C}' // ALM
        | '\u{FEFF}' // BOM / ZWNBSP
        | '\u{FFF9}'..='\u{FFFB}' // satır arası açıklama
    )
}

/// Gösterilecek tek satırlık temiz metin.
pub fn sanitize(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars * 4));
    let mut count = 0;
    let mut pending_space = false;
    let mut truncated = false;
    for c in s.chars() {
        if is_format_control(c) {
            continue;
        }
        if c.is_control() || c.is_whitespace() {
            pending_space = count > 0;
            continue;
        }
        let needed = usize::from(pending_space) + 1;
        if count + needed > max_chars {
            truncated = true;
            break;
        }
        if pending_space {
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        out.push(c);
        count += 1;
    }
    if truncated {
        // `…` için yer aç.
        if count >= max_chars {
            out.pop();
        }
        out.push('…');
    }
    out
}
/// Karışabilir olmayan güvenli karakter: yazdırılabilir ASCII, boşluk hariç (`!`..=`~`).
/// Yol ve kimlik gibi "tanınması gereken" metinlerde yalnızca bunlar olduğu gibi gösterilir.
pub fn is_plain_ascii(c: char) -> bool {
    c.is_ascii_graphic()
}

/// Tek karakterin gösterimi: düz ASCII olduğu gibi, gerisi `\u{…}` kaçışıyla (homoglyph,
/// boşluk, denetim ve yön karakterleri görünür ve ayırt edilebilir olur).
fn escape_char(c: char, out: &mut String) {
    if is_plain_ascii(c) {
        out.push(c);
    } else {
        use std::fmt::Write;
        let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
    }
}

/// [`escape_char`] ile kaçışlı metin ve kaçış gerekti mi.
pub fn escape(s: &str) -> (String, bool) {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        escape_char(c, &mut out);
    }
    let escaped = !s.chars().all(is_plain_ascii);
    (out, escaped)
}

fn escaped_len(s: &str) -> usize {
    escape(s).0.chars().count()
}

/// Kaçışlı ve **ortadan** kırpılmış tek parça: baş ve sondan karakter (kaçış birimi bölünmez)
/// alınır, araya `…`. `max_chars` en az 3 olmalı; daha azsa yalnızca `…`.
pub fn escape_middle(s: &str, max_chars: usize) -> String {
    let (full, _) = escape(s);
    if full.chars().count() <= max_chars {
        return full;
    }
    let chars: Vec<char> = s.chars().collect();
    let budget = max_chars.saturating_sub(1);
    let (mut head, mut tail) = (String::new(), String::new());
    let (mut i, mut j) = (0usize, chars.len());
    let mut used = 0;
    let mut take_head = true;
    while i < j {
        let c = if take_head { chars[i] } else { chars[j - 1] };
        let mut unit = String::new();
        escape_char(c, &mut unit);
        let n = unit.chars().count();
        if used + n > budget {
            break;
        }
        used += n;
        if take_head {
            head.push_str(&unit);
            i += 1;
        } else {
            tail.insert_str(0, &unit);
            j -= 1;
        }
        take_head = !take_head;
    }
    format!("{head}…{tail}")
}

/// Yürütülebilir dosya yolunun gösterimi: `home` altıysa `~` ile, düz ASCII dışı her karakter
/// kaçışlı, uzunsa **ortadan** kırpılır — kök bileşen (`/usr`, `~`) ve dosya adı her zaman
/// görünür (`/usr/…/bin/tool`). Baştan kırpma yok: `~/.cache/x/usr/bin/app` asla
/// `…/usr/bin/app` gibi görünmez.
pub fn display_path(path: &str, home: Option<&str>, max_chars: usize) -> String {
    let home = home.map(|h| h.trim_end_matches('/')).filter(|h| h.len() > 1);
    let shown: std::borrow::Cow<'_, str> = match home {
        Some(h) if path == h => "~".into(),
        Some(h) if path.strip_prefix(h).is_some_and(|r| r.starts_with('/')) => {
            format!("~{}", &path[h.len()..]).into()
        }
        _ => path.into(),
    };
    let (full, _) = escape(&shown);
    if full.chars().count() <= max_chars {
        return full;
    }
    // Bileşenler: ilk ("~" ya da "/usr" — boş ilk parça kök "/" demek) ve son (dosya adı).
    let parts: Vec<&str> = shown.split('/').collect();
    let (first, rest) = if shown.starts_with('/') {
        (format!("/{}", parts.get(1).copied().unwrap_or_default()), parts.get(2..).unwrap_or_default())
    } else {
        (parts[0].to_owned(), parts.get(1..).unwrap_or_default())
    };
    let Some((name, middle)) = rest.split_last() else {
        return escape_middle(&shown, max_chars);
    };
    // Atlanan ara bileşen varsa `/…/`, yoksa yalnızca `/`.
    let gap_str = if middle.is_empty() { "/" } else { "/…/" };
    let gap = gap_str.chars().count();
    let head = escape_middle(&first, max_chars.saturating_sub(gap) / 3);
    let head_len = head.chars().count();
    let name_room = max_chars.saturating_sub(head_len + gap);
    if escaped_len(name) > name_room {
        return format!("{head}{gap_str}{}", escape_middle(name, name_room));
    }
    // Dosya adına sondan sığan üst bileşenleri ekle.
    let mut tail = escape(name).0;
    for comp in middle.iter().rev() {
        let (c, _) = escape(comp);
        if head_len + gap + c.chars().count() + 1 + tail.chars().count() > max_chars {
            break;
        }
        tail = format!("{c}/{tail}");
    }
    format!("{head}{gap_str}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_makes_confusables_visible() {
        assert_eq!(escape("/usr/bin/x"), ("/usr/bin/x".into(), false));
        // Kiril "е" (U+0435) Latin "e" gibi görünür: kaçışlı.
        assert_eq!(escape("gnom\u{435}"), ("gnom\\u{435}".into(), true));
        assert_eq!(escape("a b"), ("a\\u{20}b".into(), true));
        assert_eq!(escape("a\u{202E}b\n"), ("a\\u{202e}b\\u{a}".into(), true));
    }

    #[test]
    fn middle_truncation_keeps_root_and_file_name() {
        assert_eq!(display_path("/usr/bin/x", None, 40), "/usr/bin/x");
        let long = format!("/usr/lib/{}/libexec/helper-tool", "d".repeat(80));
        let s = display_path(&long, None, 40);
        assert!(s.starts_with("/usr/…/") && s.ends_with("/libexec/helper-tool"), "{s}");
        assert!(s.chars().count() <= 40, "{s}");
        // Uzun dosya adı da ortadan: başı ve sonu görünür.
        let s = display_path(&format!("/opt/{}", "n".repeat(100)), None, 30);
        assert!(s.starts_with("/opt/n") && s.ends_with('n') && s.contains('…'), "{s}");
        assert!(s.chars().count() <= 30, "{s}");
        assert_eq!(escape_middle("abcdefgh", 5), "ab…gh");
        assert_eq!(escape_middle("ab\u{435}de", 4), "ab…e");
        // Kaçış birimi bölünmez: sığmayan \u{435} hiç alınmaz.
        assert_eq!(escape_middle("a\u{435}\u{435}b", 4), "a…b");
    }

    /// Taklit yolları asla sistem yolu gibi görünmez.
    #[test]
    fn spoofed_paths_never_look_like_system_paths() {
        let home = Some("/home/u");
        let spoof = "/home/u/.cache/x/usr/bin/gnome-software";
        let s = display_path(spoof, home, 96);
        assert_eq!(s, "~/.cache/x/usr/bin/gnome-software");
        // Uzun taklit: kök "~" görünür kalır, "…/usr/bin/…" ile başlamaz.
        let deep = format!("/home/u/{}/usr/bin/gnome-software", "a/".repeat(60));
        for max in [20, 40, 96] {
            let s = display_path(&deep, home, max);
            assert!(s.starts_with("~/…/") || s.starts_with("~/a"), "{s}");
            assert!(!s.starts_with('/') && !s.starts_with('…'), "{s}");
            assert!(s.chars().count() <= max, "{s}");
        }
        // Ev dışı kullanıcı yazılabilir kök de görünür kalır.
        let s = display_path(&format!("/tmp/{}/usr/bin/pkexec", "z".repeat(90)), home, 40);
        assert!(s.starts_with("/tmp/…/"), "{s}");
        // Homoglyph: Kiril "е" kaçışlı görünür.
        let s = display_path("/usr/bin/gnom\u{435}-software", home, 96);
        assert_eq!(s, "/usr/bin/gnom\\u{435}-software");
        // Ev dizini öneki yalnızca bileşen sınırında.
        assert_eq!(display_path("/home/user2/x", home, 96), "/home/user2/x");
    }

    #[test]
    fn controls_and_whitespace_collapse() {
        assert_eq!(sanitize("  a\n\n b\t\x07c\u{85}d  ", 50), "a b c d");
        assert_eq!(sanitize("\x1b[31mkırmızı", 50), "[31mkırmızı");
        assert_eq!(sanitize("", 5), "");
    }

    #[test]
    fn bidi_overrides_removed() {
        // "exe.txt" gibi görünen RLO hilesi.
        assert_eq!(sanitize("a\u{202E}txt.exe\u{202C}b", 50), "atxt.exeb");
        assert_eq!(sanitize("x\u{2066}y\u{2069}\u{200F}\u{FEFF}z", 50), "xyz");
    }

    #[test]
    fn truncation_by_chars() {
        assert_eq!(sanitize("abcdef", 4), "abc…");
        assert_eq!(sanitize("abcd", 4), "abcd");
        assert_eq!(sanitize("ğüşiöç", 3), "ğü…");
        let long = "ş".repeat(1000);
        let s = sanitize(&long, MAX_MESSAGE_CHARS);
        assert_eq!(s.chars().count(), MAX_MESSAGE_CHARS);
        assert!(s.ends_with('…'));
    }
}
