//! Onarım hattı.

use std::collections::{BTreeMap, BTreeSet};

use toml_edit::{Document, Value};

use crate::text::{self, Stmt, Target};
use crate::{Action, ActionKind, Healed, Labels, Loaded, Schema, syntax, typo};

/// Bir dosyanın onarım kuralları (bir kez kurulur).
#[derive(Debug, Clone, Default)]
pub struct Rules {
    pub schema: Schema,
    /// Eski ad → yeni ad (noktalı yollar). Yeni ad zaten yazılıysa eski satır etkisizdir, silinir.
    pub renames: Vec<(String, String)>,
    /// Kaldırılmış anahtarlar (noktalı yol; altındakiler dahil). Yazım hatası önerisi verilmez.
    pub retired: Vec<String>,
    /// Hiçbir zaman silinmeyen anahtarlar (ör. `config_version`).
    pub protected: Vec<String>,
    /// Şema yolu → varsayılan değerin TOML yazımı. Varsayılana eşit olmayan ama yok sayılan değer
    /// "geçersiz" sayılır (yoruma alınır), eşit olan "gereksiz" (silinir).
    pub defaults: BTreeMap<String, String>,
    /// Şema yolu → sayı aralığı.
    pub limits: BTreeMap<String, (f64, f64)>,
}

/// Yalnızca sözdizimi: bozuk TOML kurtarılır, anlamsal adım yok (şeması başkasına ait dosyalar).
pub fn repair_syntax(text: &str, labels: &Labels) -> Healed {
    let mut actions = Vec::new();
    let text = syntax::repair(text, labels, &mut actions);
    Healed { text, actions }
}

/// Tam onarım. `keep`: kullanıcının geri aldığı, dokunulmayacak anahtar yolları. `load`: çağıranın
/// ayrıştırma + doğrulaması (eşitlik kâhini).
pub fn heal<C: PartialEq>(
    text: &str,
    rules: &Rules,
    keep: &BTreeSet<String>,
    labels: &Labels,
    load: &dyn Fn(&str) -> Loaded<C>,
) -> Healed {
    let mut h = Heal { text: String::new(), actions: Vec::new(), rules, keep, labels, load, skip: BTreeSet::new(), commented: BTreeSet::new() };
    h.text = syntax::repair(text, labels, &mut h.actions);
    // Yorumlu satırlar kullanıcının elinden çıkmıştır: onarımın kendi yorum blokları sayılmasın diye
    // girdi metnine (okunamıyorsa sözdizimi onarılmış metne) bakılır.
    h.commented = commented_leaves(text).or_else(|| commented_leaves(&h.text)).unwrap_or_default();
    if let Some((mut base, unknown)) = h.invalid_values() {
        h.unknown(&mut base, unknown);
        h.clamp(&base);
        h.redundant(&base);
    }
    Healed { text: h.text, actions: h.actions }
}

struct Heal<'a, C> {
    text: String,
    actions: Vec<Action>,
    rules: &'a Rules,
    keep: &'a BTreeSet<String>,
    labels: &'a Labels,
    load: &'a dyn Fn(&str) -> Loaded<C>,
    /// Bu çalıştırmada dokunulmayacak yollar (öneri verilenler, korunanlar).
    skip: BTreeSet<String>,
    /// Girdide yorumu olan yapraklar (varsayılana eşit olsa da silinmez).
    commented: BTreeSet<String>,
}

fn join(path: &[String]) -> String {
    path.join(".")
}

fn split(path: &str) -> Vec<String> {
    path.split('.').map(str::to_owned).collect()
}

impl<C: PartialEq> Heal<'_, C> {
    fn doc(&self) -> Option<Document<&str>> {
        Document::parse(self.text.as_str()).ok()
    }

    fn push(&mut self, key: String, kind: ActionKind, pos: usize) {
        let line = text::line_no(&self.text, pos);
        self.actions.push(Action { key, kind, line });
    }

    /// Aday metin aynı ayarı veriyor mu?
    fn same(&self, candidate: &str, base: &C) -> bool {
        matches!((self.load)(candidate), Loaded::Ok { config, .. } if config == *base)
    }

    /// Anahtar ya da bir üst yolu korunuyor mu?
    fn protected(&self, key: &str) -> bool {
        let hit = |set: &BTreeSet<String>| {
            set.contains(key) || set.iter().any(|p| key.starts_with(p.as_str()) && key[p.len()..].starts_with('.'))
        };
        hit(self.keep) || hit(&self.skip) || self.rules.protected.iter().any(|p| p == key)
    }

    /// Okunamayan değerleri yoruma alır; sonunda yüklenen ayar ve bilinmeyen anahtarlar.
    fn invalid_values(&mut self) -> Option<(C, Vec<Vec<String>>)> {
        let limit = self.text.lines().count() + 8;
        for _ in 0..limit {
            let span = match (self.load)(&self.text) {
                Loaded::Ok { config, unknown } => return Some((config, unknown)),
                Loaded::Rejected { span: Some(span), .. } => span,
                Loaded::Rejected { span: None, .. } => return None,
            };
            let doc = self.doc()?;
            let (path, stmt) = text::leaves(&doc)
                .into_iter()
                .filter(|(_, s)| s.lines.contains(&span.start))
                .min_by_key(|(_, s)| s.lines.len())?;
            drop(doc);
            let key = join(&path);
            self.push(key, ActionKind::Invalid, stmt.lines.start);
            self.text = text::comment_out(&self.text, stmt.lines, &self.labels.marker(&self.labels.invalid));
        }
        None
    }

    fn unknown(&mut self, base: &mut C, mut unknown: Vec<Vec<String>>) {
        unknown.sort_by_key(Vec::len);
        unknown.dedup();
        let mut done: Vec<Vec<String>> = Vec::new();
        for path in unknown {
            if path.is_empty() || done.iter().any(|d| path.starts_with(d)) {
                continue;
            }
            let key = join(&path);
            if self.protected(&key) {
                continue;
            }
            done.push(path.clone());
            if let Some(to) = self.rules.renames.iter().find(|(from, _)| *from == key).map(|(_, to)| split(to)) {
                self.rename(base, &path, &to);
                continue;
            }
            if self.rules.retired.iter().any(|r| key == *r || key.starts_with(&format!("{r}."))) {
                self.delete(base, &path, ActionKind::Retired);
                continue;
            }
            if let Some(candidate) = self.typo(&path) {
                let pos = self.doc().and_then(|d| text::locate(&d, &path)).map_or(0, |t| t.first_line());
                self.push(key.clone(), ActionKind::Suggest { candidate }, pos);
                self.skip.insert(key);
                continue;
            }
            self.delete(base, &path, ActionKind::Unused);
        }
    }

    /// Silme; ayar değişmiyorsa.
    fn delete(&mut self, base: &C, path: &[String], kind: ActionKind) -> bool {
        let Some(doc) = self.doc() else { return false };
        let Some(target) = text::locate(&doc, path) else { return false };
        drop(doc);
        let candidate = text::delete(&self.text, &target);
        if !self.same(&candidate, base) {
            return false;
        }
        self.push(join(path), kind, target.first_line());
        self.text = candidate;
        true
    }

    /// Eski adı yeni adına taşır. Yeni ad yazılıysa eski etkisizdir: silinir.
    fn rename(&mut self, base: &mut C, from: &[String], to: &[String]) {
        let Some(doc) = self.doc() else { return };
        if text::exists(&doc, to) {
            drop(doc);
            self.delete(base, from, ActionKind::Retired);
            return;
        }
        let Some(target) = text::locate(&doc, from) else { return };
        let candidate = match &target {
            Target::Leaf(s) if from[..from.len() - 1] == to[..to.len() - 1] => {
                Some(text::replace(&self.text, s.key.clone(), &text::key_literal(&to[to.len() - 1])))
            }
            Target::Leaf(s) => Some(move_leaf(&self.text, &doc, s, to)),
            Target::Subtree { headers, .. } if headers.len() == 1 => {
                rename_header(&self.text, headers[0].clone(), to)
            }
            Target::Subtree { .. } => None,
        };
        drop(doc);
        let Some(candidate) = candidate else { return };
        // Taşıma davranışı değiştirir (eski ad etkisizdi): okunabilmesi şart, sonraki adımların
        // karşılaştırması yeni ayarla yapılır.
        let Loaded::Ok { config, .. } = (self.load)(&candidate) else { return };
        *base = config;
        self.push(join(from), ActionKind::Moved { to: join(to) }, target.first_line());
        self.text = candidate;
    }

    fn typo(&self, path: &[String]) -> Option<String> {
        let (last, parent) = path.split_last()?;
        let schema_parent = if parent.is_empty() { String::new() } else { self.rules.schema.resolve(parent)? };
        let doc = self.doc()?;
        let present = |name: &str| {
            let mut p = parent.to_vec();
            p.push(name.to_owned());
            text::exists(&doc, &p)
        };
        let candidates: Vec<&str> = self.rules.schema.children(&schema_parent).into_iter().filter(|c| !present(c)).collect();
        typo::suggest(last, &candidates).map(str::to_owned)
    }

    /// Aralık dışı sayıyı, yükleyicinin zaten kullandığı sınıra çeker (davranış aynıysa).
    fn clamp(&mut self, base: &C) {
        let mut tried = BTreeSet::new();
        loop {
            let Some(doc) = self.doc() else { return };
            let next = text::leaves(&doc).into_iter().find_map(|(path, stmt)| {
                let key = join(&path);
                if tried.contains(&key) || self.protected(&key) {
                    return None;
                }
                let (min, max) = *self.rules.limits.get(&self.rules.schema.resolve(&path)?)?;
                let v = text::value_at(&doc, &path)?;
                let (n, int) = match v {
                    Value::Integer(i) => (*i.value() as f64, true),
                    Value::Float(f) => (*f.value(), false),
                    _ => return None,
                };
                let to = n.clamp(min, max);
                (to != n && n.is_finite()).then_some((key, stmt, to, int))
            });
            drop(doc);
            let Some((key, stmt, to, int)) = next else { return };
            tried.insert(key.clone());
            let literal = if int { format!("{}", to as i64) } else { format!("{to:?}") };
            let old = self.text[stmt.lines.clone()].trim_end().to_owned();
            let replaced = text::replace(&self.text, stmt.value.clone(), &literal);
            let ind = indent_of(&self.text, &stmt);
            let note = format!("{ind}# {}: {}\n", self.labels.marker(&self.labels.clamped), old.trim_start());
            let candidate = format!("{}{note}{}", &replaced[..stmt.lines.start], &replaced[stmt.lines.start..]);
            if self.same(&candidate, base) {
                self.push(key, ActionKind::Clamped { to: literal }, stmt.lines.start);
                self.text = candidate;
            }
        }
    }

    /// Varsayılana eşit (ya da yükleyicinin yok saydığı) satırlar.
    fn redundant(&mut self, base: &C) {
        let mut tried = BTreeSet::new();
        loop {
            let Some(doc) = self.doc() else { return };
            let next = text::leaves(&doc).into_iter().find(|(path, _)| {
                let key = join(path);
                !tried.contains(&key) && !self.protected(&key) && !self.commented.contains(&key)
            });
            let Some((path, stmt)) = next else { break };
            let key = join(&path);
            tried.insert(key.clone());
            let default_differs = self
                .rules
                .schema
                .resolve(&path)
                .and_then(|p| self.rules.defaults.get(&p))
                .zip(text::value_at(&doc, &path))
                .is_some_and(|(d, v)| !same_literal(d, v));
            drop(doc);
            let deleted = text::delete(&self.text, &Target::Leaf(stmt.clone()));
            if !self.same(&deleted, base) {
                continue;
            }
            if default_differs {
                // Yükleyici bu değeri yok sayıp varsayılanı kullanıyor: kullanıcının yazdığı yorumda kalsın.
                self.push(key, ActionKind::Invalid, stmt.lines.start);
                self.text = text::comment_out(&self.text, stmt.lines, &self.labels.marker(&self.labels.invalid));
            } else {
                self.push(key, ActionKind::Redundant, stmt.lines.start);
                self.text = deleted;
            }
        }
        self.empty_tables(base);
        self.trim_end();
    }

    /// Boş açık tablo başlıkları (altında ne değer ne yorum; ayar değişmiyorsa). Göçün ya da
    /// onarımın boşalttığı `[bar.widget.workspaces]` gibi başlıklar işe yaramaz satırdır.
    /// Onarım metni değiştirdiyse dosya sonundaki boş satırlar teke iner.
    fn trim_end(&mut self) {
        if !self.actions.iter().any(|a| a.kind.edits()) {
            return;
        }
        let t = self.text.trim_end_matches(['\n', '\r', ' ', '\t']);
        self.text = if t.is_empty() { String::new() } else { format!("{t}\n") };
    }

    fn empty_tables(&mut self, base: &C) {
        let Some(doc) = self.doc() else { return };
        let tables = text::explicit_tables(&doc);
        drop(doc);
        for path in tables.iter().rev() {
            if self.protected(&join(path)) {
                continue;
            }
            let Some(doc) = self.doc() else { return };
            let Some(Target::Subtree { stmts, headers }) = text::locate(&doc, path) else { continue };
            drop(doc);
            let [header] = headers.as_slice() else { continue };
            if !stmts.is_empty() {
                continue;
            }
            // Başlıktan sonraki satırlar bir sonraki başlığa kadar boş olmalı (yorum varsa kalır).
            let rest = &self.text[header.end..];
            let body_end = rest.lines().position(|l| !l.trim().is_empty()).map_or(rest.len(), |n| {
                rest.split_inclusive('\n').take(n).map(str::len).sum()
            });
            let after = rest[body_end..].trim_start();
            if !(after.is_empty() || after.starts_with('[')) {
                continue;
            }
            let target = Target::Subtree { stmts: Vec::new(), headers: vec![header.clone()] };
            let candidate = text::delete(&self.text, &target);
            if self.same(&candidate, base) {
                self.push(join(path), ActionKind::Redundant, header.start);
                self.text = candidate;
            }
        }
    }
}

fn indent_of<'t>(text: &'t str, stmt: &Stmt) -> &'t str {
    let line = &text[stmt.lines.clone()];
    &line[..line.len() - line.trim_start().len()]
}

/// İki TOML değeri anlamca eşit mi (`6` ile `6.0` eşit; biçim farkı önemsiz)?
fn same_literal(default: &str, v: &Value) -> bool {
    let Ok(d) = default.parse::<Value>() else { return false };
    values_equal(&d, v)
}

fn values_equal(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (String(x), String(y)) => x.value() == y.value(),
        (Integer(x), Integer(y)) => x.value() == y.value(),
        (Float(x), Float(y)) => x.value() == y.value(),
        (Integer(x), Float(y)) | (Float(y), Integer(x)) => (*x.value() as f64) == *y.value(),
        (Boolean(x), Boolean(y)) => x.value() == y.value(),
        (Array(x), Array(y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| values_equal(a, b)),
        _ => false,
    }
}

/// Yaprağı başka tabloya taşır: eski satır silinir, yeni satır hedef tablonun başlığının altına
/// (yoksa dosya sonuna yeni başlıkla) yazılır.
fn move_leaf(text: &str, doc: &Document<&str>, stmt: &Stmt, to: &[String]) -> String {
    let (last, table) = to.split_last().expect("hedef yol boş değil");
    let line = format!("{} = {}\n", text::key_literal(last), &text[stmt.value.clone()]);
    let header_end = if table.is_empty() {
        // Kök: ilk başlıktan önce.
        Some(text::line_starts(text).into_iter().find(|&s| text::is_header(&text[s..text::line_end(text, s)])).unwrap_or(text.len()))
    } else {
        match text::locate(doc, table) {
            Some(Target::Subtree { headers, .. }) if headers.len() == 1 => Some(headers[0].end),
            _ => None,
        }
    };
    let removed = stmt.lines.clone();
    match header_end {
        Some(at) => {
            let mut out = String::with_capacity(text.len() + line.len());
            if at <= removed.start {
                out.push_str(&text[..at]);
                out.push_str(&line);
                out.push_str(&text[at..removed.start]);
                out.push_str(&text[removed.end..]);
            } else {
                out.push_str(&text[..removed.start]);
                out.push_str(&text[removed.end..at]);
                out.push_str(&line);
                out.push_str(&text[at..]);
            }
            out
        }
        None => {
            let mut out = format!("{}{}", &text[..removed.start], &text[removed.end..]);
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            let header: Vec<String> = table.iter().map(|k| text::key_literal(k)).collect();
            out.push_str(&format!("\n[{}]\n{line}", header.join(".")));
            out
        }
    }
}

/// Tablo başlığını yeni yola yazar (`[clock]` → `[bar.widget.clock]`), satır sonu yorumu korunur.
fn rename_header(text: &str, header: std::ops::Range<usize>, to: &[String]) -> Option<String> {
    let line = &text[header.clone()];
    let trimmed = line.trim_start();
    if trimmed.starts_with("[[") {
        return None;
    }
    let close = trimmed.find(']')?;
    let ind = &line[..line.len() - trimmed.len()];
    let tail = &trimmed[close + 1..];
    let keys: Vec<String> = to.iter().map(|k| text::key_literal(k)).collect();
    Some(format!("{}{ind}[{}]{tail}{}", &text[..header.start], keys.join("."), &text[header.end..]))
}

/// Yorumu (üstünde ya da satır sonunda) olan yaprakların yolları; okunamazsa `None`.
fn commented_leaves(text: &str) -> Option<BTreeSet<String>> {
    let doc = Document::parse(text).ok()?;
    Some(text::leaves(&doc).into_iter().filter(|(_, s)| s.has_comment(text)).map(|(p, _)| join(&p)).collect())
}
