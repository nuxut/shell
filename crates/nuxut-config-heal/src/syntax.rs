//! Bozuk TOML'un kurtarılması: okunamayan satır yoruma alınır, okunabilen her şey kalır.
//!
//! - Tekrar eden anahtar: **son yazılan geçerlidir** (Hyprland ve çoğu ayar dosyasındaki alışkanlık:
//!   kullanıcı alta yeni satır ekler). Öncekiler yoruma alınır.
//! - Tekrar eden tablo başlığı (`[bar]` iki kez): ikinci bölümün satırları ilk bölüme taşınır,
//!   ikinci başlık kalkar. Böylece çakışan anahtar varsa yine "son yazılan geçerli" kuralı işler.
//! - Bozuk başlık (`[bar`, `[bar] x`): başlık onarılabiliyorsa onarılır; onarılamıyorsa başlık
//!   **bölümüyle birlikte** yoruma alınır (yoksa altındaki anahtarlar bir önceki tabloya düşerdi).
//! - Kapanmamış çok satırlı değer: değerin başladığı satırdan hatanın görüldüğü satıra kadar.
//! - Diğer: hatalı satır.
//!
//! Her adımdan sonra metin yeniden ayrıştırılır; döngü satır sayısıyla sınırlıdır.

use toml_edit::Document;

use crate::text::{
    comment_out, continuation_lines, header_path, is_header, line_end, line_no, line_starts, locate, stmt_path, Target,
};
use crate::{Action, ActionKind, Labels};

/// Metni ayrıştırılabilir hâle getirir (ya da sınırda vazgeçer). Değişiklikler `actions`'a eklenir.
pub(crate) fn repair(text: &str, labels: &Labels, actions: &mut Vec<Action>) -> String {
    let mut text = text.to_owned();
    let limit = text.lines().count() + 8;
    for _ in 0..limit {
        let err = match Document::parse(text.as_str()) {
            Ok(_) => return text,
            Err(e) => e,
        };
        let pos = err.span().map_or(text.len(), |s| s.start).min(text.len());
        let next = err
            .message()
            .contains("duplicate")
            .then(|| duplicate(&text, pos, labels, actions))
            .flatten()
            .unwrap_or_else(|| generic(&text, pos, labels, actions));
        if next == text {
            break;
        }
        text = next;
    }
    text
}

struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
    cont: Vec<bool>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let starts = line_starts(text);
        let mut cont = continuation_lines(text);
        cont.resize(starts.len(), false);
        Self { text, starts, cont }
    }

    /// Satır sayısı (son satırın sonu dahil değil).
    fn len(&self) -> usize {
        self.starts.len().saturating_sub(1).max(1)
    }

    fn line(&self, i: usize) -> &'a str {
        let s = self.starts[i];
        let e = self.starts.get(i + 1).copied().unwrap_or(self.text.len());
        &self.text[s..e.max(s)]
    }

    fn start(&self, i: usize) -> usize {
        self.starts.get(i).copied().unwrap_or(self.text.len())
    }

    fn index_of(&self, pos: usize) -> usize {
        self.starts.partition_point(|&s| s <= pos).saturating_sub(1).min(self.len() - 1)
    }

    fn is_header(&self, i: usize) -> bool {
        !self.cont[i] && is_header(self.line(i))
    }

    /// `i`'den sonraki ilk başlık satırı (yoksa satır sayısı).
    fn next_header(&self, i: usize) -> usize {
        (i + 1..self.len()).find(|&j| self.is_header(j)).unwrap_or(self.len())
    }
}

fn push(actions: &mut Vec<Action>, kind: ActionKind, key: String, line: usize) {
    actions.push(Action { key, kind, line });
}

/// Tekrar eden tablo ya da anahtar. Çözülemezse `None` (genel yola düşer).
fn duplicate(text: &str, pos: usize, labels: &Labels, actions: &mut Vec<Action>) -> Option<String> {
    let lines = Lines::new(text);
    let l = lines.index_of(pos);
    let line = lines.line(l);
    if lines.is_header(l) {
        let (path, aot) = header_path(line)?;
        if aot {
            return None;
        }
        let first = (0..l).find(|&i| lines.is_header(i) && header_path(lines.line(i)) == Some((path.clone(), false)))?;
        let end1 = lines.next_header(first);
        let end2 = lines.next_header(l);
        let mut body = text[lines.start(l + 1)..lines.start(end2)].to_owned();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        let new = format!(
            "{}{}{}{}",
            &text[..lines.start(end1)],
            body,
            &text[lines.start(end1)..lines.start(l)],
            &text[lines.start(end2)..]
        );
        push(actions, ActionKind::MergedTable, path.join("."), l + 1);
        return Some(new);
    }
    // Anahtar: ikinci satır geçici olarak yoruma alınınca belge okunuyorsa ilki bulunur.
    let temp = format!("{}# {}", &text[..lines.start(l)], &text[lines.start(l)..]);
    let doc = Document::parse(temp.as_str()).ok()?;
    let table = match (0..l).rev().find(|&i| lines.is_header(i)) {
        Some(h) => match header_path(lines.line(h))? {
            (p, false) => p,
            (_, true) => return None,
        },
        None => Vec::new(),
    };
    let mut full = table;
    full.extend(stmt_path(line)?);
    let Target::Leaf(first) = locate(&doc, &full)? else { return None };
    if first.lines.end > lines.start(l) {
        return None;
    }
    push(actions, ActionKind::Duplicate, full.join("."), line_no(text, first.lines.start));
    Some(comment_out(text, first.lines, &labels.marker(&labels.duplicate)))
}

/// Genel hata: hatalı satır (ya da bozuk başlığın bölümü, kapanmamış değerin başı).
fn generic(text: &str, pos: usize, labels: &Labels, actions: &mut Vec<Action>) -> String {
    let lines = Lines::new(text);
    let mut l = lines.index_of(pos);
    // Dosya sonunda ya da boş/yorum satırında görülen hata önceki ifadeye aittir.
    while l > 0 && {
        let t = lines.line(l).trim();
        t.is_empty() || t.starts_with('#')
    } {
        l -= 1;
    }
    let marker = labels.marker(&labels.broken);
    if lines.cont[l] {
        let s = (0..l).rev().find(|&i| !lines.cont[i]).unwrap_or(0);
        let range = lines.start(s)..lines.start(l);
        push(actions, ActionKind::Broken, String::new(), s + 1);
        return comment_out(text, range, &marker);
    }
    let line = lines.line(l);
    if is_header(line) {
        let trimmed = line.trim();
        let aot = trimmed.starts_with("[[");
        let inner = trimmed.trim_start_matches('[');
        let inner = inner.split([']', '#']).next().unwrap_or("").trim();
        let fixed = if aot { format!("[[{inner}]]") } else { format!("[{inner}]") };
        if !inner.is_empty() && fixed != trimmed && header_path(&fixed).is_some() {
            let ind = &line[..line.len() - line.trim_start().len()];
            let eol = if line.ends_with('\n') { "\n" } else { "" };
            push(actions, ActionKind::Broken, inner.to_owned(), l + 1);
            let end = line_end(text, lines.start(l));
            return format!("{}{ind}{fixed}{eol}{}", &text[..lines.start(l)], &text[end..]);
        }
        let end = lines.next_header(l);
        push(actions, ActionKind::Broken, String::new(), l + 1);
        return comment_out(text, lines.start(l)..lines.start(end), &marker);
    }
    push(actions, ActionKind::Broken, String::new(), l + 1);
    comment_out(text, lines.start(l)..line_end(text, lines.start(l)), &marker)
}
