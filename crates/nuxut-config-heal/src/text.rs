//! Metin düzeyi işlemler: satır aralıkları, bir anahtar yolunun dosyadaki satırları, silme, yoruma
//! alma, değiştirme. Onarım dosyayı yeniden biçimlendirmez: dokunulmayan her bayt aynı kalır.
//!
//! Konumlar `toml_edit::Document`'ın (span'lı ayrıştırma) verdiği bayt aralıklarıdır. Bir anahtar
//! ancak kendi satır(lar)ına sahipse (satır başında anahtar, değerden sonra yalnızca boşluk ya da
//! yorum) metinden silinir ya da yoruma alınır; satır içi tablonun üyesi gibi paylaşılan satırlara
//! dokunulmaz.

use std::ops::Range;

use toml_edit::{Document, Item, Table, TableLike, Value};

/// `pos`'un bulunduğu satırın başı.
pub(crate) fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

/// `pos`'un bulunduğu satırın sonu (satır sonu karakteri dahil).
pub(crate) fn line_end(text: &str, pos: usize) -> usize {
    text[pos..].find('\n').map_or(text.len(), |i| pos + i + 1)
}

/// 1 tabanlı satır numarası.
pub(crate) fn line_no(text: &str, pos: usize) -> usize {
    text[..pos.min(text.len())].matches('\n').count() + 1
}

/// Satırın baştaki boşluğu.
fn indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Bir anahtar = değer ifadesi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stmt {
    /// İfadenin kapladığı tam satırlar.
    pub lines: Range<usize>,
    /// Son anahtar parçası (`bar.size`'da `size`).
    pub key: Range<usize>,
    pub value: Range<usize>,
    /// Değerden sonra aynı satırdaki yorum (`# …`, satır sonu hariç).
    pub comment: Option<Range<usize>>,
}

impl Stmt {
    /// Anahtarın ve değerin konumundan; satırlar paylaşılıyorsa `None`.
    fn new(text: &str, key: Range<usize>, value: Range<usize>) -> Option<Self> {
        if key.start > value.start || value.end > text.len() {
            return None;
        }
        let start = line_start(text, key.start);
        // Anahtarın önünde yalnızca noktalı anahtarın üst parçaları olabilir (`bar.size`).
        if text[start..key.start].contains(['{', ',', '=', '[', '\n']) {
            return None;
        }
        let end = line_end(text, value.end);
        let rest = text[value.end..end].trim_end_matches(['\n', '\r']);
        let trimmed = rest.trim_start();
        let comment = match trimmed.chars().next() {
            None => None,
            Some('#') => {
                let at = value.end + (rest.len() - trimmed.len());
                Some(at..value.end + rest.len())
            }
            Some(_) => return None,
        };
        Some(Self { lines: start..end, key, value, comment })
    }

    /// İfadenin hemen üstündeki satır bir yorum mu (arada boş satır yok)?
    pub fn has_comment_above(&self, text: &str) -> bool {
        if self.lines.start == 0 {
            return false;
        }
        let prev = line_start(text, self.lines.start - 1);
        text[prev..self.lines.start].trim_start().starts_with('#')
    }

    pub fn has_comment(&self, text: &str) -> bool {
        self.comment.is_some() || self.has_comment_above(text)
    }
}

/// Bir yolun dosyadaki karşılığı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// Tek ifade (değer; satır içi tablo ya da dizi de olabilir).
    Leaf(Stmt),
    /// Tablo: altındaki tüm ifadeler ve başlık satırları.
    Subtree { stmts: Vec<Stmt>, headers: Vec<Range<usize>> },
}

impl Target {
    /// Silinecek/yoruma alınacak satır aralıkları (sıralı, çakışmasız).
    pub fn ranges(&self) -> Vec<Range<usize>> {
        let mut r: Vec<Range<usize>> = match self {
            Self::Leaf(s) => vec![s.lines.clone()],
            Self::Subtree { stmts, headers } => stmts.iter().map(|s| s.lines.clone()).chain(headers.iter().cloned()).collect(),
        };
        r.sort_by_key(|r| r.start);
        r.dedup_by(|b, a| b.start < a.end);
        r
    }

    pub fn first_line(&self) -> usize {
        self.ranges().first().map_or(0, |r| r.start)
    }
}

/// Yol parçası: tablo anahtarı ya da (dizi tablosunda) sıra.
pub(crate) fn segment_index(seg: &str) -> Option<usize> {
    seg.strip_prefix('[')?.strip_suffix(']')?.parse().ok()
}

/// `path`'in metindeki ifadeleri. Yol yoksa ya da satır içi bir yapının içindeyse `None`.
pub(crate) fn locate(doc: &Document<&str>, path: &[String]) -> Option<Target> {
    let text = doc.raw();
    let (last, parents) = path.split_last()?;
    let mut cur: &dyn TableLike = doc.as_table();
    let mut i = 0;
    while i < parents.len() {
        let item = cur.get(&parents[i])?;
        i += 1;
        cur = match item {
            Item::Table(t) => t,
            Item::ArrayOfTables(a) => {
                let idx = segment_index(parents.get(i)?)?;
                i += 1;
                a.get(idx)?
            }
            // Satır içi tablonun üyesi kendi satırına sahip değildir.
            _ => return None,
        };
    }
    // Dizi tablosunun tek öğesi hedeflenmez.
    if segment_index(last).is_some() {
        return None;
    }
    let key = cur.key(last)?;
    match cur.get(last)? {
        Item::Value(v) => Some(Target::Leaf(Stmt::new(text, key.span()?, v.span()?)?)),
        Item::Table(t) => {
            let mut stmts = Vec::new();
            let mut headers = Vec::new();
            subtree(text, t, &mut stmts, &mut headers)?;
            Some(Target::Subtree { stmts, headers })
        }
        Item::ArrayOfTables(a) => {
            let mut stmts = Vec::new();
            let mut headers = Vec::new();
            for t in a.iter() {
                subtree(text, t, &mut stmts, &mut headers)?;
            }
            Some(Target::Subtree { stmts, headers })
        }
        Item::None => None,
    }
}

/// Tablonun başlığı (açık tabloysa) ve altındaki her ifade.
fn subtree(text: &str, t: &Table, stmts: &mut Vec<Stmt>, headers: &mut Vec<Range<usize>>) -> Option<()> {
    if !t.is_implicit() && !t.is_dotted() {
        let span = t.span()?;
        headers.push(line_start(text, span.start)..line_end(text, span.start));
    }
    for (k, item) in t.iter() {
        match item {
            Item::Value(v) => stmts.push(Stmt::new(text, t.key(k)?.span()?, v.span()?)?),
            Item::Table(child) => subtree(text, child, stmts, headers)?,
            Item::ArrayOfTables(a) => {
                for child in a.iter() {
                    subtree(text, child, stmts, headers)?;
                }
            }
            Item::None => {}
        }
    }
    Some(())
}

/// Belgedeki her yaprak ifade ve yolu (satır içi tablolar tek ifade; dizi tablolarının içi hariç).
pub(crate) fn leaves(doc: &Document<&str>) -> Vec<(Vec<String>, Stmt)> {
    fn walk(text: &str, t: &Table, path: &mut Vec<String>, out: &mut Vec<(Vec<String>, Stmt)>) {
        for (k, item) in t.iter() {
            path.push(k.to_owned());
            match item {
                Item::Value(v) => {
                    if let Some(s) = t.key(k).and_then(|key| Stmt::new(text, key.span()?, v.span()?)) {
                        out.push((path.clone(), s));
                    }
                }
                Item::Table(child) => walk(text, child, path, out),
                Item::ArrayOfTables(_) | Item::None => {}
            }
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(doc.raw(), doc.as_table(), &mut Vec::new(), &mut out);
    out
}

/// Açık (başlıklı) tabloların yolları, belge sırasıyla (dizi tabloları hariç).
pub(crate) fn explicit_tables(doc: &Document<&str>) -> Vec<Vec<String>> {
    fn walk(t: &Table, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        for (k, item) in t.iter() {
            if let Item::Table(child) = item {
                path.push(k.to_owned());
                if !child.is_implicit() && !child.is_dotted() {
                    out.push(path.clone());
                }
                walk(child, path, out);
                path.pop();
            }
        }
    }
    let mut out = Vec::new();
    walk(doc.as_table(), &mut Vec::new(), &mut out);
    out
}

/// Belgedeki değer (salt okunur).
pub(crate) fn value_at<'a>(doc: &'a Document<&str>, path: &[String]) -> Option<&'a Value> {
    let (last, parents) = path.split_last()?;
    let mut cur: &dyn TableLike = doc.as_table();
    for p in parents {
        cur = cur.get(p)?.as_table_like()?;
    }
    cur.get(last)?.as_value()
}

/// Yol belgede var mı (değer ya da tablo)?
pub(crate) fn exists(doc: &Document<&str>, path: &[String]) -> bool {
    let Some((last, parents)) = path.split_last() else { return true };
    let mut cur: &dyn TableLike = doc.as_table();
    for p in parents {
        match cur.get(p).and_then(Item::as_table_like) {
            Some(t) => cur = t,
            None => return false,
        }
    }
    cur.contains_key(last)
}

/// Aralıkları siler; aynı satırdaki yorumlar kendi satırlarında kalır.
pub(crate) fn delete(text: &str, target: &Target) -> String {
    let comments: Vec<(usize, String)> = match target {
        Target::Leaf(s) => trailing(text, s).into_iter().collect(),
        Target::Subtree { stmts, .. } => stmts.iter().filter_map(|s| trailing(text, s)).collect(),
    };
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for mut r in target.ranges() {
        let kept: Vec<&String> = comments.iter().filter(|(pos, _)| r.contains(pos)).map(|(_, c)| c).collect();
        // Silinen satırın iki yanı da boşsa bir boş satır da gider (boşluk birikmesin).
        let blank_before = r.start == 0 || text[line_start(text, r.start - 1)..r.start].trim().is_empty();
        if kept.is_empty() && blank_before && r.end < text.len() {
            let next = line_end(text, r.end);
            if text[r.end..next].trim().is_empty() {
                r.end = next;
            }
        }
        out.push_str(&text[at..r.start.max(at)]);
        for c in kept {
            out.push_str(c);
        }
        at = r.end.max(at);
    }
    out.push_str(&text[at..]);
    out
}

/// İfadenin satır sonu yorumu, kendi satırı olarak (girinti korunur).
fn trailing(text: &str, s: &Stmt) -> Option<(usize, String)> {
    let c = s.comment.as_ref()?;
    let ind = indent(&text[s.lines.start..]);
    Some((s.lines.start, format!("{ind}{}\n", &text[c.clone()])))
}

/// Satırları yoruma alır; üstüne `# {marker}` satırı koyar.
pub(crate) fn comment_out(text: &str, lines: Range<usize>, marker: &str) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    out.push_str(&text[..lines.start]);
    let block = &text[lines.clone()];
    let ind = indent(block);
    out.push_str(&format!("{ind}# {marker}\n"));
    for line in block.split_inclusive('\n') {
        out.push_str(ind);
        out.push_str("# ");
        out.push_str(line.strip_prefix(ind).unwrap_or(line));
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&text[lines.end..]);
    out
}

pub(crate) fn replace(text: &str, range: Range<usize>, new: &str) -> String {
    format!("{}{new}{}", &text[..range.start], &text[range.end..])
}

/// Her satırın başında bir değerin (çok satırlı dizi/tablo/dize) içinde miyiz? Dizeler ve yorumlar
/// içindeki parantezler sayılmaz.
pub(crate) fn continuation_lines(text: &str) -> Vec<bool> {
    #[derive(PartialEq)]
    enum S {
        Code,
        Basic,
        Literal,
        MlBasic,
        MlLiteral,
        Comment,
    }
    let b = text.as_bytes();
    let mut out = vec![false];
    let mut depth: i32 = 0;
    let mut s = S::Code;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match s {
            S::Code => match c {
                b'#' => s = S::Comment,
                b'"' if b[i..].starts_with(b"\"\"\"") => {
                    s = S::MlBasic;
                    i += 2;
                }
                b'\'' if b[i..].starts_with(b"'''") => {
                    s = S::MlLiteral;
                    i += 2;
                }
                b'"' => s = S::Basic,
                b'\'' => s = S::Literal,
                b'[' | b'{' => depth += 1,
                b']' | b'}' => depth = (depth - 1).max(0),
                _ => {}
            },
            S::Basic => match c {
                b'\\' => i += 1,
                b'"' => s = S::Code,
                // Kapanmamış tek satırlık dize satırda biter (hatalı; sonraki satır serbest).
                b'\n' => s = S::Code,
                _ => {}
            },
            S::Literal => {
                if c == b'\'' || c == b'\n' {
                    s = S::Code;
                }
            }
            S::MlBasic => match c {
                b'\\' => i += 1,
                b'"' if b[i..].starts_with(b"\"\"\"") => {
                    s = S::Code;
                    i += 2;
                }
                _ => {}
            },
            S::MlLiteral => {
                if b[i..].starts_with(b"'''") {
                    s = S::Code;
                    i += 2;
                }
            }
            S::Comment => {
                if c == b'\n' {
                    s = S::Code;
                }
            }
        }
        if c == b'\n' {
            // Tek satırlık başlık (`[bar]`) satır sonunda kapanmıştır; yalnızca açık değerler sürer.
            out.push(depth > 0 || matches!(s, S::MlBasic | S::MlLiteral));
        }
        i += 1;
    }
    out
}

/// Satır sınırları (başlangıç ofsetleri; son eleman metnin sonu).
pub(crate) fn line_starts(text: &str) -> Vec<usize> {
    let mut v = vec![0];
    v.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    if *v.last().unwrap_or(&0) != text.len() {
        v.push(text.len());
    }
    v
}

/// Tablo başlığı satırı mı (`[a.b]`, `[[a]]`)? Değer devamı satırları hariç tutulmalı.
pub(crate) fn is_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// Başlık satırının yolu ve dizi tablosu olup olmadığı (satır tek başına ayrıştırılır).
pub(crate) fn header_path(line: &str) -> Option<(Vec<String>, bool)> {
    let doc = Document::parse(line.trim()).ok()?;
    let mut path = Vec::new();
    let mut cur: &Table = doc.as_table();
    loop {
        let mut it = cur.iter();
        let (k, item) = it.next()?;
        if it.next().is_some() {
            return None;
        }
        path.push(k.to_owned());
        match item {
            Item::Table(t) if t.is_implicit() => cur = t,
            Item::Table(_) => return Some((path, false)),
            Item::ArrayOfTables(_) => return Some((path, true)),
            _ => return None,
        }
    }
}

/// Tek satırlık ifadenin anahtar yolu (`a.b = 1` → `[a, b]`).
pub(crate) fn stmt_path(line: &str) -> Option<Vec<String>> {
    let doc = Document::parse(line.trim()).ok()?;
    let mut path = Vec::new();
    let mut cur: &Table = doc.as_table();
    loop {
        let mut it = cur.iter();
        let (k, item) = it.next()?;
        if it.next().is_some() {
            return None;
        }
        path.push(k.to_owned());
        match item {
            Item::Table(t) if t.is_dotted() => cur = t,
            Item::Value(_) => return Some(path),
            _ => return None,
        }
    }
}

/// TOML'da tırnaksız yazılabilen anahtar mı?
pub(crate) fn bare_key(k: &str) -> bool {
    !k.is_empty() && k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// Anahtarın TOML yazımı.
pub(crate) fn key_literal(k: &str) -> String {
    if bare_key(k) { k.to_owned() } else { toml_edit::Value::from(k).to_string().trim().to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Vec<String> {
        s.split('.').map(str::to_owned).collect()
    }

    #[test]
    fn locate_leaf_and_delete_keeps_comments() {
        let text = "# baş\n[bar]\n# boyut\nsize = 30 # not\nedge = \"top\"\n";
        let doc = Document::parse(text).unwrap();
        let t = locate(&doc, &p("bar.size")).unwrap();
        assert_eq!(delete(text, &t), "# baş\n[bar]\n# boyut\n# not\nedge = \"top\"\n");
        let t = locate(&doc, &p("bar.edge")).unwrap();
        assert_eq!(delete(text, &t), "# baş\n[bar]\n# boyut\nsize = 30 # not\n");
    }

    #[test]
    fn inline_members_are_not_owned() {
        let text = "bar = { size = 30, edge = \"top\" }\n";
        let doc = Document::parse(text).unwrap();
        assert_eq!(locate(&doc, &p("bar.size")), None);
        assert!(matches!(locate(&doc, &p("bar")), Some(Target::Leaf(_))));
    }

    #[test]
    fn subtree_and_dotted() {
        let text = "a = 1\nfoo.x = 1\n[foo.y]\nz = [\n  1,\n  2,\n]\n[bar]\nq = 1\n";
        let doc = Document::parse(text).unwrap();
        let t = locate(&doc, &p("foo")).unwrap();
        assert_eq!(delete(text, &t), "a = 1\n[bar]\nq = 1\n");
    }

    #[test]
    fn multiline_comment_out() {
        let text = "[bar]\n  start = [\n    \"a\",\n  ]\nx = 1\n";
        let doc = Document::parse(text).unwrap();
        let Some(Target::Leaf(s)) = locate(&doc, &p("bar.start")) else { panic!() };
        assert_eq!(
            comment_out(text, s.lines, "m"),
            "[bar]\n  # m\n  # start = [\n  #   \"a\",\n  # ]\nx = 1\n"
        );
    }

    #[test]
    fn continuation_tracks_open_values() {
        let text = "a = [\n1,\n]\n[b]\ns = \"\"\"\n[x]\n\"\"\"\nc = \"[\" # [\n";
        assert_eq!(continuation_lines(text), [false, true, true, false, false, true, true, false, false]);
    }

    #[test]
    fn header_and_stmt_paths() {
        assert_eq!(header_path("[ bar.widget . clock ] # x"), Some((p("bar.widget.clock"), false)));
        assert_eq!(header_path("[[a]]"), Some((p("a"), true)));
        assert_eq!(header_path("[a"), None);
        assert_eq!(stmt_path("  bar.\"x y\" = 1 # c"), Some(vec!["bar".into(), "x y".into()]));
        assert_eq!(stmt_path("x = [1,"), None);
    }
}
