//! Dosya ADINDAN MIME türü (shared-mime-info `globs2`, `subclasses`, `aliases`) ve filtre eşleme.
//!
//! İçerik koklanmaz (magic yok, karar 5): güvenilmeyen dosyanın baytı okunmaz, okuma da ağ
//! dosya sisteminde bloklamaz. Bilinmeyen tür [`OCTET_STREAM`]. Veritabanı dosyaları boyut
//! sınırlı okunur; bozuk satır atlanır.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nuxut_portal_backend::chooser::protocol::{Filter, Rule};

pub const OCTET_STREAM: &str = "application/octet-stream";
pub const DIRECTORY: &str = "inode/directory";
const TEXT_PLAIN: &str = "text/plain";
/// Bir veritabanı dosyasının en büyük boyu.
const MAX_DB_BYTES: u64 = 4 << 20;
/// Toplam glob kuralı sınırı.
const MAX_GLOBS: usize = 20_000;
/// Alt sınıf zincirinde en çok adım (döngüye karşı).
const MAX_DEPTH: usize = 16;

#[derive(Debug, Clone)]
struct Glob {
    weight: u32,
    pattern: String,
    mime: String,
    case_sensitive: bool,
}

/// shared-mime-info'nun ad tabanlı kısmı.
#[derive(Debug, Clone, Default)]
pub struct MimeDb {
    /// `*.uzantı` biçimli büyük/küçük harf duyarsız kurallar: küçük harf uzantı → (ağırlık, tür).
    suffix: HashMap<String, (u32, String)>,
    /// Diğer kurallar (tam ad, karmaşık glob, harf duyarlı).
    other: Vec<Glob>,
    parents: HashMap<String, Vec<String>>,
    aliases: HashMap<String, String>,
}

fn read_limited(path: &Path) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut s = String::new();
    f.take(MAX_DB_BYTES).read_to_string(&mut s).ok()?;
    Some(s)
}

/// `$XDG_DATA_HOME:$XDG_DATA_DIRS` altındaki `mime/` dizinleri, öncelik sırasıyla.
pub fn mime_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => out.push(PathBuf::from(d).join("mime")),
        None => out.extend(home.map(|h| h.join(".local/share/mime"))),
    }
    let dirs = std::env::var_os("XDG_DATA_DIRS").filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    out.extend(std::env::split_paths(&dirs).filter(|p| p.is_absolute()).map(|p| p.join("mime")));
    out
}

impl MimeDb {
    /// Sistem veritabanı (bulunamazsa boş: her dosya `application/octet-stream`).
    pub fn load() -> Self {
        Self::load_from(&mime_dirs())
    }

    /// Öncelik sırasıyla dizinlerden (ilk dizin kazanır: aynı kalıp sonrakilerde yok sayılır).
    pub fn load_from(dirs: &[PathBuf]) -> Self {
        let mut db = Self::default();
        let mut seen_patterns: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut count = 0;
        for dir in dirs {
            if let Some(text) = read_limited(&dir.join("globs2")) {
                for g in parse_globs2(&text) {
                    if count >= MAX_GLOBS {
                        break;
                    }
                    if !seen_patterns.insert(format!("{}\0{}", g.pattern, g.mime)) {
                        continue;
                    }
                    count += 1;
                    db.add(g);
                }
            }
            if let Some(text) = read_limited(&dir.join("subclasses")) {
                for (child, parent) in pairs(&text) {
                    let list = db.parents.entry(child).or_default();
                    if list.len() < MAX_DEPTH && !list.contains(&parent) {
                        list.push(parent);
                    }
                }
            }
            if let Some(text) = read_limited(&dir.join("aliases")) {
                for (alias, canonical) in pairs(&text) {
                    db.aliases.entry(alias).or_insert(canonical);
                }
            }
        }
        db
    }

    fn add(&mut self, g: Glob) {
        let simple_ext = g.pattern.strip_prefix("*.").filter(|e| !e.is_empty() && !e.contains(['*', '?', '[']));
        match simple_ext {
            Some(ext) if !g.case_sensitive => {
                let key = ext.to_lowercase();
                // Aynı uzantıda yüksek ağırlık kazanır; eşitlikte ilk (öncelikli dizin) kalır.
                match self.suffix.get(&key) {
                    Some((w, _)) if *w >= g.weight => {}
                    _ => {
                        self.suffix.insert(key, (g.weight, g.mime));
                    }
                }
            }
            _ => self.other.push(g),
        }
    }

    /// Takma adı kanonik türe çevirir.
    pub fn canonical<'a>(&'a self, mime: &'a str) -> &'a str {
        self.aliases.get(mime).map_or(mime, String::as_str)
    }

    /// Addan tür. Kural: en yüksek ağırlık, eşitlikte en uzun kalıp (shared-mime-info belirtimi);
    /// uzantıda en uzun uzantı (`.tar.gz` > `.gz`).
    pub fn guess(&self, name: &str) -> &str {
        let lower = name.to_lowercase();
        let mut best: Option<(u32, usize, &str)> = None;
        // Uzantılar: her noktadan sonrası aday (`a.tar.gz` → `tar.gz`, `gz`).
        for (i, _) in lower.match_indices('.') {
            let ext = &lower[i + 1..];
            if let Some((w, m)) = self.suffix.get(ext) {
                let len = ext.len() + 2;
                if best.is_none_or(|(bw, bl, _)| (*w, len) > (bw, bl)) {
                    best = Some((*w, len, m.as_str()));
                }
            }
        }
        for g in &self.other {
            let hit = if g.case_sensitive { glob_match(&g.pattern, name) } else { glob_match(&g.pattern, &lower) };
            if hit && best.is_none_or(|(bw, bl, _)| {
                let key = (g.weight, g.pattern.len());
                // Eşitlikte harf duyarlı kural kazanır (`*.C` C++, `*.c` C).
                key > (bw, bl) || (g.case_sensitive && key == (bw, bl))
            }) {
                best = Some((g.weight, g.pattern.len(), g.mime.as_str()));
            }
        }
        best.map_or(OCTET_STREAM, |(_, _, m)| m)
    }

    /// `mime` `wanted`'ın kendisi ya da alt sınıfı mı. `wanted` `tür/*` olabilir. Örtük kurallar:
    /// her `text/*` `text/plain`'in, her tür `application/octet-stream`'in alt sınıfıdır.
    pub fn is_a(&self, mime: &str, wanted: &str) -> bool {
        let wanted = self.canonical(wanted);
        let mut queue = vec![self.canonical(mime).to_owned()];
        let mut seen = Vec::new();
        while let Some(m) = queue.pop() {
            if type_matches(&m, wanted) {
                return true;
            }
            if seen.len() >= MAX_DEPTH * 4 || seen.contains(&m) {
                continue;
            }
            if let Some(ps) = self.parents.get(&m) {
                queue.extend(ps.iter().map(|p| self.canonical(p).to_owned()));
            }
            if m.starts_with("text/") && m != TEXT_PLAIN {
                queue.push(TEXT_PLAIN.to_owned());
            }
            seen.push(m);
        }
        wanted == OCTET_STREAM && mime != DIRECTORY
    }

    /// Filtre bir dosya adıyla eşleşiyor mu (klasörler çağıranın işi: her filtrede görünür).
    pub fn filter_matches(&self, filter: &Filter, name: &str) -> bool {
        let lower = name.to_lowercase();
        let mut mime: Option<&str> = None;
        filter.rules.iter().any(|r| match r {
            Rule::Glob(p) => glob_match(&p.to_lowercase(), &lower),
            Rule::Mime(m) => {
                let guessed = *mime.get_or_insert_with(|| self.guess(name));
                self.is_a(guessed, m)
            }
        })
    }
}

fn type_matches(mime: &str, wanted: &str) -> bool {
    if wanted == "*" || wanted == "*/*" {
        return true;
    }
    match wanted.strip_suffix("/*") {
        Some(major) => mime.split_once('/').is_some_and(|(m, _)| m == major),
        None => mime == wanted,
    }
}

/// `ağırlık:tür:kalıp[:bayraklar]`; bayraklarda `cs` = harf duyarlı.
fn parse_globs2(text: &str) -> impl Iterator<Item = Glob> + '_ {
    text.lines().filter(|l| !l.starts_with('#')).filter_map(|l| {
        let mut parts = l.splitn(4, ':');
        let weight = parts.next()?.trim().parse::<u32>().ok()?;
        let mime = parts.next()?.trim();
        let pattern = parts.next()?;
        let flags = parts.next().unwrap_or("");
        if mime.is_empty() || pattern.is_empty() || mime.len() > 255 || pattern.len() > 255 || !mime.contains('/') {
            return None;
        }
        let case_sensitive = flags.split(',').any(|f| f == "cs");
        Some(Glob { weight: weight.min(100), pattern: if case_sensitive { pattern.to_owned() } else { pattern.to_lowercase() }, mime: mime.to_owned(), case_sensitive })
    })
}

fn pairs(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.lines().filter(|l| !l.starts_with('#')).filter_map(|l| {
        let (a, b) = l.split_once(' ')?;
        let (a, b) = (a.trim(), b.trim());
        (a.contains('/') && b.contains('/') && a.len() <= 255 && b.len() <= 255).then(|| (a.to_owned(), b.to_owned()))
    })
}

/// fnmatch alt kümesi: `*`, `?`, `[...]` (aralık; `!`/`^` olumsuzlar). Harf duyarlılığı çağıranın
/// işi (duyarsızda iki taraf da `to_lowercase`). Kalıp ≤ 256 bayt olduğundan
/// yineleme yerine iki işaretçili geri izleme (doğrusal bellek, kötü kalıpta en çok O(n·m)).
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() {
            match p[pi] {
                '*' => {
                    star = Some((pi, ni));
                    pi += 1;
                    continue;
                }
                '?' => {
                    pi += 1;
                    ni += 1;
                    continue;
                }
                '[' => {
                    if let Some((matched, next)) = class(&p, pi, n[ni]) {
                        if matched {
                            pi = next;
                            ni += 1;
                            continue;
                        }
                    } else if n[ni] == '[' {
                        // Kapanmamış `[` düz karakter.
                        pi += 1;
                        ni += 1;
                        continue;
                    }
                }
                c if c == n[ni] => {
                    pi += 1;
                    ni += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some((sp, sn)) => {
                pi = sp + 1;
                ni = sn + 1;
                star = Some((sp, sn + 1));
            }
            None => return false,
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// `p[start] == '['` olan sınıfı `c` ile dener: `Some((eşleşti, sonraki kalıp indisi))`;
/// kapanmamışsa `None`.
fn class(p: &[char], start: usize, c: char) -> Option<(bool, usize)> {
    let mut i = start + 1;
    let negate = matches!(p.get(i), Some('!' | '^'));
    if negate {
        i += 1;
    }
    let mut matched = false;
    let mut first = true;
    while i < p.len() {
        let x = p[i];
        if x == ']' && !first {
            return Some((matched != negate, i + 1));
        }
        first = false;
        if p.get(i + 1) == Some(&'-') && p.get(i + 2).is_some_and(|&e| e != ']') {
            let hi = p[i + 2];
            if x <= c && c <= hi {
                matched = true;
            }
            i += 3;
        } else {
            if x == c {
                matched = true;
            }
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> MimeDb {
        let d = tempfile::tempdir().unwrap();
        let m = d.path().join("mime");
        std::fs::create_dir(&m).unwrap();
        std::fs::write(
            m.join("globs2"),
            "# yorum\n50:image/png:*.png\n50:application/gzip:*.gz\n55:application/x-compressed-tar:*.tar.gz\n\
             50:text/x-csrc:*.c\n50:text/x-c++src:*.C:cs\n10:text/x-readme:readme*\n50:image/jpeg:*.jpg\n\
             bozuk satır\n50:application/pdf:*.pdf\n",
        )
        .unwrap();
        std::fs::write(m.join("subclasses"), "application/x-compressed-tar application/gzip\nimage/svg+xml application/xml\n").unwrap();
        std::fs::write(m.join("aliases"), "image/pjpeg image/jpeg\napplication/x-pdf application/pdf\n").unwrap();
        MimeDb::load_from(&[m])
    }

    #[test]
    fn guess_by_name() {
        let db = db();
        assert_eq!(db.guess("A.PNG"), "image/png");
        assert_eq!(db.guess("x.tar.gz"), "application/x-compressed-tar");
        assert_eq!(db.guess("x.gz"), "application/gzip");
        assert_eq!(db.guess("main.c"), "text/x-csrc");
        assert_eq!(db.guess("main.C"), "text/x-c++src");
        assert_eq!(db.guess("README.md"), "text/x-readme");
        assert_eq!(db.guess("adsız"), OCTET_STREAM);
        assert_eq!(db.guess(".png"), "image/png");
    }

    #[test]
    fn subclass_alias_and_wildcards() {
        let db = db();
        assert!(db.is_a("application/x-compressed-tar", "application/gzip"));
        assert!(db.is_a("text/x-csrc", "text/plain"));
        assert!(db.is_a("image/png", "image/*"));
        assert!(db.is_a("image/jpeg", "image/pjpeg"));
        assert!(db.is_a("image/png", "application/octet-stream"));
        assert!(!db.is_a("image/png", "text/*"));
        assert!(!db.is_a(DIRECTORY, OCTET_STREAM));
    }

    #[test]
    fn filters() {
        let db = db();
        let f = Filter { name: "Resim".into(), rules: vec![Rule::Mime("image/*".into()), Rule::Glob("*.SVG".into())] };
        assert!(db.filter_matches(&f, "a.png"));
        assert!(db.filter_matches(&f, "b.svg"));
        assert!(!db.filter_matches(&f, "c.pdf"));
        let pdf = Filter { name: "PDF".into(), rules: vec![Rule::Mime("application/x-pdf".into())] };
        assert!(db.filter_matches(&pdf, "rapor.PDF"));
    }

    #[test]
    fn glob_syntax() {
        assert!(glob_match("*.png", "a.png"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a?c", "abc"));
        assert!(glob_match("*.[ch]", "x.h"));
        assert!(!glob_match("*.[!ch]", "x.h"));
        assert!(glob_match("img[0-9]*", "img7.jpg"));
        assert!(glob_match("a[b", "a[b"));
        assert!(glob_match("[]]x", "]x"));
        assert!(!glob_match("*.png", "a.pngx"));
        // Kötü kalıp hızlı biter.
        let name = "a".repeat(200);
        let pattern = format!("{}b", "*a".repeat(60));
        assert!(!glob_match(&pattern, &name));
    }

    #[test]
    fn real_database_if_present() {
        let db = MimeDb::load();
        if db.suffix.is_empty() {
            return;
        }
        assert_eq!(db.guess("a.png"), "image/png");
        assert!(db.is_a(db.guess("a.txt"), "text/plain"));
    }
}
