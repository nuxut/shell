//! Yazım hatası önerisi: bilinmeyen anahtara en yakın bilinen kardeş anahtar.

/// Optimal hizalama (Damerau-Levenshtein, bitişik yer değiştirme 1 sayılır) uzaklığı.
pub(crate) fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let w = b.len() + 1;
    let mut d = vec![0usize; (a.len() + 1) * w];
    for i in 0..=a.len() {
        d[i * w] = i;
    }
    for (j, cell) in d.iter_mut().enumerate().take(w) {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[(i - 1) * w + j] + 1).min(d[i * w + j - 1] + 1).min(d[(i - 1) * w + j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[(i - 2) * w + j - 2] + 1);
            }
            d[i * w + j] = v;
        }
    }
    d[a.len() * w + b.len()]
}

/// Büyük/küçük harf ve `-`/`_` farkı yazım hatası sayılmaz, eşittir.
fn normalize(s: &str) -> String {
    s.to_lowercase().replace('-', "_")
}

/// `name`'e benzeyen tek aday. Eşik: normalleştirilmiş eşitlik ya da kısa adlarda 1, uzunlarda 2
/// düzenleme. En iyi uzaklıkta birden çok aday varsa öneri yok (yanlış tahmin, öneri vermemekten
/// kötüdür).
pub(crate) fn suggest<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let n = normalize(name);
    // İki harfli addan tahmin yürütülmez.
    if n.chars().count() < 3 {
        return None;
    }
    let limit = if n.chars().count() <= 4 { 1 } else { 2 };
    let mut best: Option<(usize, &'a str)> = None;
    let mut tie = false;
    for &c in candidates {
        if c == name {
            continue;
        }
        let d = distance(&n, &normalize(c));
        if d > limit {
            continue;
        }
        match best {
            Some((bd, _)) if d > bd => {}
            Some((bd, _)) if d == bd => tie = true,
            _ => {
                best = Some((d, c));
                tie = false;
            }
        }
    }
    if tie { None } else { best.map(|(_, c)| c) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        assert_eq!(distance("size", "siez"), 1);
        assert_eq!(distance("thickness", "thicknes"), 1);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
    }

    #[test]
    fn suggestions() {
        let c = ["size", "edge", "start", "end", "padding", "spacing"];
        assert_eq!(suggest("siez", &c), Some("size"));
        assert_eq!(suggest("Start", &c), Some("start"));
        assert_eq!(suggest("paddin", &c), Some("padding"));
        assert_eq!(suggest("left", &c), None);
        assert_eq!(suggest("xyz", &c), None);
        // İki harfli ad: tahmin yok.
        assert_eq!(suggest("ed", &c), None);
        assert_eq!(suggest("inner-radius", &["inner_radius"]), Some("inner_radius"));
    }
}
