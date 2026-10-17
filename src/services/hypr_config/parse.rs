//! Hyprland ayar dosyası (hyprlang alt kümesi) ayrıştırma ve `source` zinciri taraması.
//!
//! Yalnızca okur. Amaç anlamak: Nuxut'un dosyası source ediliyor mu, kullanıcı onu yorum
//! satırıyla kapatmış mı, `nuxut-shell` başka yerden başlatılıyor mu, kullanıcının bind'leri ve
//! Nuxut'un yönettiği anahtarları nerede yazdığı. Hyprland'in tam dilbilgisi değildir; bilinmeyen
//! satırlar atlanır. Tüm girdi sınırlıdır ([`ScanLimits`]); sınır aşılırsa rapor `truncated`
//! olur ve çağıran güvenli tarafta kalır (dosyaya yazmaz).

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use super::keybinds::{Combo, parse_hypr_combo};

/// Tarama sınırları.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanLimits {
    /// İç içe `source` derinliği (ana dosya 0).
    pub depth: usize,
    /// Toplam taranan dosya.
    pub files: usize,
    pub file_bytes: u64,
    pub line_bytes: usize,
    /// Tek bir jokerli `source`'un en fazla eşleşmesi.
    pub glob_matches: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self { depth: 8, files: 64, file_bytes: 1 << 20, line_bytes: 4096, glob_matches: 64 }
    }
}

/// Satırın konumu: [`ChainReport::files`] içindeki dosya sırası ve 1'den başlayan satır no.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub file: usize,
    pub line: usize,
}

/// Kullanıcının zincirindeki bir bind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserBind {
    pub combo: Combo,
    pub at: Location,
}

/// Zincir taramasının sonucu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChainReport {
    /// Taranan dosyalar (ilk = ana dosya). Hedef (Nuxut'un dosyası) taranmaz, burada yok.
    pub files: Vec<PathBuf>,
    /// Hedefi source eden (yorum olmayan) satır.
    pub source_at: Option<Location>,
    /// Hedefi gösteren yorumlanmış `source` satırı (kullanıcı kapatmış).
    pub commented_at: Option<Location>,
    /// Hedef dışında `nuxut-shell`'i başlatan `exec`/`exec-once` satırı.
    pub external_launch: Option<Location>,
    pub binds: Vec<UserBind>,
    /// İzlenen anahtarların (ör. `decoration:rounding`) yazıldığı yerler, Hyprland'in okuma
    /// sırasıyla (`source` yerinde açılır).
    pub assignments: Vec<(String, Location)>,
    /// Hedefi source eden SON satıra gelindiğinde `assignments`'ın uzunluğu: bundan sonraki atamalar
    /// Nuxut'un değerini ezer, öncekiler Nuxut tarafından ezilir.
    pub source_order: Option<usize>,
    /// Hedefi source eden SON satıra gelindiğinde `binds`'ın uzunluğu: öncekileri Nuxut'un ayar
    /// katmanı `unbind` ile devralabilir, sonrakiler onu yeniden bağlar (devralınamaz).
    pub bind_order: Option<usize>,
    /// `monitor = …` satırlarının değerleri (`$değişken` açılmış), okuma sırasıyla; ana ayarı
    /// devralırken yeni dosyaya taşınır (`takeover`). Açılamayan `$` içeren değer atlanır.
    pub monitors: Vec<String>,
    /// Ana ayar devralınırken taşınan/taşınmayan satırlar ([`Migration`]).
    pub migration: Migration,
    /// Bir sınır aşıldı ya da bir dosya okunamadı: rapor eksik olabilir.
    pub truncated: bool,
}

/// Devralmada taşınan üst düzey kategoriler: donanım, klavye/fare, imleç (kullanıcı onayı
/// 2026-10-17; ADR-0007 eki). Blok (`input { … }`) ya da yollu atama (`input:kb_layout = tr`).
pub const MIGRATE_CATEGORIES: [&str; 6] = ["input", "cursor", "xwayland", "render", "opengl", "device"];
/// Taşınan liste ve metin sınırları.
const MAX_MIGRATED_LINES: usize = 2048;
const MAX_EXEC: usize = 64;

/// Ana ayar devralınırken eski zincirden taşınanlar ve taşınmayanların özeti. Taşınan satırlar
/// Hyprland'in okuma sırasıyla, kullanıcı değişkenleri (`$x`) açılmış; açılamayan `$AD` ortam
/// değişkeni olarak kalır (hyprlang açar).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Migration {
    /// Taşınan metin parçaları (`env = …` satırı ya da çok satırlı blok).
    pub lines: Vec<String>,
    /// `env`/`envd` satır sayısı.
    pub env: usize,
    /// Taşınan ortam değişkenlerinin yalnızca ADLARI (değerler rapora, bildirime, günlüğe
    /// girmez: belirteç/anahtar olabilir). En çok [`MAX_EXEC`].
    pub env_names: Vec<String>,
    /// Taşınan blok ve yollu atama sayısı (`input {…}`, `device {…}`, `cursor:… = …`).
    pub settings: usize,
    /// Taşınmayan `exec`/`exec-once` komutları, tam metin (en çok [`MAX_EXEC`]). Yalnız süreç içi
    /// karar için (pano köprüsü tanıma); rapora yalnızca program adı çıkar ([`program_name`]).
    pub exec: Vec<String>,
    /// Taşınmayan bind sayısı.
    pub binds: usize,
    /// Taşınmayan pencere/katman kuralı sayısı.
    pub rules: usize,
    /// Taşınmayan öteki ayar satırları (görünüm, yerleşim, animasyon…; Nuxut'un katmanları karşılar).
    pub other: usize,
}

/// Bir `exec` komutunun yalnızca program adı (rapor ve bildirim için): baştaki `AD=değer` ortam
/// atamaları ve `sh`/`bash` atlanır, yol ve tırnak atılır; argümanlar (belirteç, parola
/// içerebilir) hiç dönmez. Tanınamazsa `?`.
pub fn program_name(command: &str) -> String {
    let mut words = command.split_whitespace().map(|w| w.trim_matches(|c| c == '"' || c == '\''));
    let mut first = words.next().unwrap_or("");
    while first.contains('=') || matches!(first, "env" | "sh" | "bash" | "-c" | "exec") {
        first = words.next().unwrap_or("");
    }
    let base = first.rsplit('/').next().unwrap_or("");
    let ok = !base.is_empty() && base.len() <= 64 && base.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b));
    if ok { base.to_owned() } else { "?".to_owned() }
}

/// Kategori adı (`device:fare`, `input`) taşınan kategorilerden mi?
fn is_migrated_category(name: &str) -> bool {
    let top = name.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
    MIGRATE_CATEGORIES.contains(&top.as_str())
}

/// Taşınan metne bir satır: girinti + hyprlang yorum kaçışı (`#` → `##`).
fn push_migrated(buf: &mut String, depth: usize, text: &str) {
    for _ in 0..depth {
        buf.push_str("    ");
    }
    buf.push_str(&text.replace('#', "##"));
    buf.push('\n');
}

/// Tarama girdisi.
#[derive(Debug, Clone)]
pub struct ScanInput<'a> {
    /// Ana ayar dosyası (mutlak).
    pub root: &'a Path,
    /// Nuxut'un dosyası (mutlak); source edilip edilmediği aranır, içine girilmez.
    pub target: &'a Path,
    /// Nuxut'un öteki dosyaları (temel katman): source edilse de içine girilmez — bind'leri kullanıcının
    /// sayılmaz.
    pub skip: &'a [PathBuf],
    /// `~` genişletmesi için ev dizini.
    pub home: Option<&'a Path>,
    /// Yeri raporlanacak anahtarlar (kategori yolu `:` ile, küçük harf).
    pub watch_keys: &'a [&'a str],
    pub limits: ScanLimits,
}

/// Bir satırın içeriği ve yorumu. `#` yorum başlatır, `##` düz `#`'dir.
pub fn split_comment(line: &str) -> (String, Option<&str>) {
    let mut content = String::with_capacity(line.len());
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '#' {
            if chars.peek().is_some_and(|&(_, n)| n == '#') {
                chars.next();
                content.push('#');
                continue;
            }
            return (content, Some(&line[i + 1..]));
        }
        content.push(c);
    }
    (content, None)
}

/// `$ad` değişkenlerini yerine koyar. Hyprlang gibi en uzun tanımlı ad eşleşir; tanımsız
/// ad olduğu gibi kalır.
pub fn expand_vars(value: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(pos) = rest.find('$') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let ident_len = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
        let found = (1..=ident_len).rev().find_map(|n| vars.get(&after[..n]).map(|v| (n, v)));
        match found {
            Some((n, v)) => {
                out.push_str(v);
                rest = &after[n..];
            }
            None => {
                out.push('$');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `~`, `~/…` ve göreli yolu çözer (göreli yol `base_dir`'e göre). Sözcüksel olarak normalize
/// edilir (`.`/`..`), dosya sistemine bakılmaz.
pub fn resolve_path(raw: &str, base_dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('\0') {
        return None;
    }
    let path = match raw.strip_prefix('~') {
        Some("") => home?.to_path_buf(),
        Some(rest) if rest.starts_with('/') => home?.join(rest.trim_start_matches('/')),
        Some(_) => return None, // `~kullanıcı` desteklenmez
        None if raw.starts_with('/') => PathBuf::from(raw),
        None => base_dir.join(raw),
    };
    Some(normalize(&path))
}

/// Sözcüksel normalleştirme: `.` atılır, `..` bir üstü siler (kökte kalır).
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Bileşen başına joker eşleşmesi (`*`, `?`, `[abc]`, `[a-z]`, `[!x]`). glob(3) gibi `*`/`?`
/// baştaki `.`'yı eşlemez.
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    match_from(&p, &n)
}

fn match_from(p: &[char], n: &[char]) -> bool {
    // Yinelemeli geri izleme: tek `*` noktası yeter (klasik iki işaretçili algoritma).
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
                    if let Some((matched, len)) = match_class(&p[pi..], n[ni])
                        && matched
                    {
                        pi += len;
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

/// `[...]` sınıfı: (eşleşti mi, desen uzunluğu). Kapanmamış sınıf `None` (düz `[`).
fn match_class(p: &[char], c: char) -> Option<(bool, usize)> {
    let mut i = 1;
    let negate = matches!(p.get(i), Some('!' | '^'));
    if negate {
        i += 1;
    }
    let start = i;
    let mut matched = false;
    while i < p.len() {
        if p[i] == ']' && i > start {
            return Some((matched != negate, i + 1));
        }
        if p.get(i + 1) == Some(&'-') && p.get(i + 2).is_some_and(|&e| e != ']') {
            if (p[i]..=p[i + 2]).contains(&c) {
                matched = true;
            }
            i += 3;
        } else {
            if p[i] == c {
                matched = true;
            }
            i += 1;
        }
    }
    None
}

fn has_wildcard(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

/// Jokerli mutlak yolu genişletir; sıralı, en fazla `max` (fazlası varsa `true` döner).
pub fn expand_glob(path: &Path, max: usize) -> (Vec<PathBuf>, bool) {
    let mut current = vec![PathBuf::new()];
    let mut overflow = false;
    for comp in path.components() {
        let comp_str = comp.as_os_str().to_string_lossy();
        if !matches!(comp, Component::Normal(_)) || !has_wildcard(&comp_str) {
            for p in &mut current {
                p.push(comp.as_os_str());
            }
            continue;
        }
        let mut next = Vec::new();
        for dir in &current {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            let mut names: Vec<String> = entries
                .filter_map(Result::ok)
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|name| wildcard_match(&comp_str, name))
                .collect();
            names.sort();
            for name in names {
                if next.len() >= max {
                    overflow = true;
                    break;
                }
                next.push(dir.join(name));
            }
        }
        current = next;
    }
    (current, overflow)
}

/// Komut `nuxut-shell`'i başlatıyor mu (alt komut değil)? `[kural] komut` öneki, sarmalayıcılar
/// (`env X=1`, `uwsm app --`, `sh -c "…"`) ve tırnaklar tolere edilir.
pub fn launches_shell(command: &str) -> bool {
    let mut cmd = command.trim();
    if cmd.starts_with('[')
        && let Some(end) = cmd.find(']')
    {
        cmd = &cmd[end + 1..];
    }
    let tokens: Vec<&str> = cmd
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|'))
        .map(|t| t.trim_matches(['"', '\'']))
        .filter(|t| !t.is_empty())
        .collect();
    // Başlatmayan alt komutlar (`ipc::parse_cli`).
    const NON_LAUNCH: [&str; 7] = ["msg", "schema", "help", "version", "--help", "-h", "--version"];
    tokens.iter().enumerate().any(|(i, t)| {
        let base = t.rsplit('/').next().unwrap_or(t);
        base == "nuxut-shell" && !tokens.get(i + 1).is_some_and(|n| NON_LAUNCH.contains(n))
    })
}

/// Bir satırı `anahtar = değer`e böler (anahtar kırpılmış ve küçük harf).
fn assignment(content: &str) -> Option<(String, &str)> {
    let (key, value) = content.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || key.contains(char::is_whitespace) && !key.starts_with('$') {
        return None;
    }
    Some((key.to_ascii_lowercase(), value.trim()))
}

struct Scanner<'a> {
    input: &'a ScanInput<'a>,
    target_canon: Option<PathBuf>,
    target_norm: PathBuf,
    seen: HashSet<PathBuf>,
    vars: BTreeMap<String, String>,
    report: ChainReport,
}

impl Scanner<'_> {
    fn is_target(&self, path: &Path) -> bool {
        if normalize(path) == self.target_norm {
            return true;
        }
        match (&self.target_canon, path.canonicalize()) {
            (Some(t), Ok(p)) => *t == p,
            _ => false,
        }
    }

    /// `source` değerinin işaret ettiği dosyalar (joker genişletilmiş).
    fn source_targets(&mut self, value: &str, base_dir: &Path) -> Vec<PathBuf> {
        let expanded = expand_vars(value, &self.vars);
        let Some(path) = resolve_path(&expanded, base_dir, self.input.home) else { return Vec::new() };
        if !has_wildcard(&path.to_string_lossy()) {
            return vec![path];
        }
        let (matches, overflow) = expand_glob(&path, self.input.limits.glob_matches);
        if overflow {
            self.report.truncated = true;
        }
        // Henüz var olmayan hedef jokerle eşleşmez ama desen hedefi kapsayabilir.
        if !matches.iter().any(|m| self.is_target(m)) && self.pattern_covers_target(&path) {
            let mut m = matches;
            m.push(self.target_norm.clone());
            return m;
        }
        matches
    }

    fn pattern_covers_target(&self, pattern: &Path) -> bool {
        let p: Vec<_> = pattern.components().collect();
        let t: Vec<_> = self.target_norm.components().collect();
        p.len() == t.len()
            && p.iter().zip(&t).all(|(a, b)| {
                let (a, b) = (a.as_os_str().to_string_lossy(), b.as_os_str().to_string_lossy());
                a == b || wildcard_match(&a, &b)
            })
    }

    fn read(&mut self, path: &Path) -> Option<String> {
        let limit = self.input.limits.file_bytes;
        let file = std::fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take(limit + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > limit {
            self.report.truncated = true;
            return None;
        }
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn scan_file(&mut self, path: &Path, depth: usize) {
        if depth > self.input.limits.depth || self.report.files.len() >= self.input.limits.files {
            self.report.truncated = true;
            return;
        }
        let key = path.canonicalize().unwrap_or_else(|_| normalize(path));
        if !self.seen.insert(key) {
            return; // döngü ya da tekrar
        }
        let Some(text) = self.read(path) else {
            // Kök okunamazsa ya da source edilen dosya yoksa: Hyprland da hata verir; yalnızca
            // kökte rapor eksik sayılır.
            if depth == 0 {
                self.report.truncated = true;
            }
            return;
        };
        let file = self.report.files.len();
        self.report.files.push(path.to_path_buf());
        let base_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut categories: Vec<String> = Vec::new();
        // Taşınan blok (`input { … }`): yalnız bu dosyada açılıp kapanır.
        let mut block: Option<String> = None;

        for (idx, raw) in text.lines().enumerate() {
            let at = Location { file, line: idx + 1 };
            if raw.len() > self.input.limits.line_bytes {
                self.report.truncated = true;
                continue;
            }
            let (content, comment) = split_comment(raw);
            let content = content.trim();
            if content.is_empty() {
                if let Some(comment) = comment
                    && self.report.commented_at.is_none()
                {
                    self.check_commented_source(comment, &base_dir, at);
                }
                continue;
            }
            if let Some(name) = content.strip_suffix('{') {
                let name = name.trim();
                if categories.is_empty() && is_migrated_category(name) {
                    block = Some(String::new());
                }
                if let Some(b) = block.as_mut() {
                    push_migrated(b, categories.len(), &format!("{name} {{"));
                }
                categories.push(name.to_ascii_lowercase());
                continue;
            }
            if content == "}" {
                categories.pop();
                if let Some(b) = block.as_mut() {
                    push_migrated(b, categories.len(), "}");
                    if categories.is_empty() {
                        self.push_migration(block.take().unwrap_or_default(), false);
                    }
                }
                continue;
            }
            let Some((key, value)) = assignment(content) else { continue };
            if let Some(var) = key.strip_prefix('$') {
                // Değişken adı büyük/küçük harfe duyarlı: orijinal metinden al.
                let name = content.split_once('=').map(|(k, _)| k.trim().trim_start_matches('$')).unwrap_or(var);
                let expanded = expand_vars(value, &self.vars);
                self.vars.insert(name.to_owned(), expanded);
                continue;
            }
            let full_key = if categories.is_empty() { key.clone() } else { format!("{}:{key}", categories.join(":")) };
            // Devralma: taşınacak satırlar (özgün anahtar yazımı, değişkenler açılmış).
            let orig_key = content.split_once('=').map_or(key.as_str(), |(k, _)| k.trim());
            let migrated_line = || format!("{orig_key} = {}", expand_vars(value, &self.vars));
            if let Some(b) = block.as_mut() {
                push_migrated(b, categories.len(), &migrated_line());
            } else if categories.is_empty() && matches!(key.as_str(), "env" | "envd") {
                let mut line = String::new();
                push_migrated(&mut line, 0, &migrated_line());
                self.push_migration(line, true);
                let name = expand_vars(value, &self.vars).split(',').next().unwrap_or("").trim().to_owned();
                if !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') && self.report.migration.env_names.len() < MAX_EXEC {
                    self.report.migration.env_names.push(name);
                }
            } else if is_migrated_category(&full_key) && full_key.contains(':') {
                let mut line = String::new();
                push_migrated(&mut line, 0, &format!("{full_key} = {}", expand_vars(value, &self.vars)));
                self.push_migration(line, false);
            } else {
                let m = &mut self.report.migration;
                match key.as_str() {
                    "source" | "monitor" | "env" | "envd" => {}
                    "exec-once" | "exec" | "execr-once" | "execr" => {
                        if m.exec.len() < MAX_EXEC {
                            m.exec.push(expand_vars(value, &self.vars).chars().take(300).collect());
                        }
                    }
                    k if k.starts_with("bind") => m.binds += 1,
                    k if k.starts_with("windowrule") || k.starts_with("layerrule") => m.rules += 1,
                    _ => m.other += 1,
                }
            }
            match key.as_str() {
                "source" => {
                    for target in self.source_targets(value, &base_dir) {
                        if self.is_target(&target) {
                            self.report.source_at.get_or_insert(at);
                            // İki kez source edilirse Hyprland değerleri sonuncusunda uygular.
                            self.report.source_order = Some(self.report.assignments.len());
                            self.report.bind_order = Some(self.report.binds.len());
                        } else if self.input.skip.iter().any(|s| normalize(s) == normalize(&target)) {
                            // Nuxut'un temel katmanı: taranmaz.
                        } else {
                            self.scan_file(&target, depth + 1);
                        }
                    }
                }
                "exec-once" | "exec" | "execr-once" | "execr" => {
                    if self.report.external_launch.is_none() && launches_shell(&expand_vars(value, &self.vars)) {
                        self.report.external_launch = Some(at);
                    }
                }
                "monitor" if categories.is_empty() => {
                    let v = expand_vars(value, &self.vars);
                    if !v.contains('$') && self.report.monitors.len() < 64 {
                        self.report.monitors.push(v);
                    }
                }
                k if k.starts_with("bind") && k[4..].chars().all(|c| c.is_ascii_lowercase()) => {
                    let value = expand_vars(value, &self.vars);
                    let mut fields = value.splitn(3, ',');
                    if let (Some(mods), Some(key)) = (fields.next(), fields.next())
                        && let Some(combo) = parse_hypr_combo(mods, key)
                    {
                        self.report.binds.push(UserBind { combo, at });
                    }
                }
                _ => {
                    if self.input.watch_keys.contains(&full_key.as_str()) {
                        self.report.assignments.push((full_key, at));
                    }
                }
            }
        }
        // Kapanmamış taşınan blok (Hyprland da hata verir): kapatılarak taşınır.
        if let Some(mut b) = block.take() {
            for depth in (0..categories.len()).rev() {
                push_migrated(&mut b, depth, "}");
            }
            self.push_migration(b, false);
        }
    }

    /// Taşınan bir parça (env satırı ya da blok); sınır aşılırsa rapor eksik.
    fn push_migration(&mut self, text: String, env: bool) {
        let m = &mut self.report.migration;
        if m.lines.len() >= MAX_MIGRATED_LINES {
            self.report.truncated = true;
            return;
        }
        m.lines.push(text);
        if env {
            m.env += 1;
        } else {
            m.settings += 1;
        }
    }

    /// Yorum metni `source = …hedef` ise kullanıcı satırı kapatmış demektir.
    fn check_commented_source(&mut self, comment: &str, base_dir: &Path, at: Location) {
        let text = comment.trim_start_matches(['#', ' ', '\t']);
        if let Some((key, value)) = assignment(text)
            && key == "source"
        {
            let expanded = expand_vars(value, &self.vars);
            if let Some(path) = resolve_path(&expanded, base_dir, self.input.home)
                && (self.is_target(&path) || self.pattern_covers_target(&path))
            {
                self.report.commented_at = Some(at);
            }
        }
    }
}

/// Ana dosyadan başlayarak `source` zincirini tarar.
pub fn scan_chain(input: &ScanInput<'_>) -> ChainReport {
    let mut scanner = Scanner {
        input,
        target_canon: input.target.canonicalize().ok(),
        target_norm: normalize(input.target),
        seen: HashSet::new(),
        vars: BTreeMap::new(),
        report: ChainReport::default(),
    };
    scanner.scan_file(input.root, 0);
    scanner.report
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn comments_and_escapes() {
        assert_eq!(split_comment("a = b # c"), ("a = b ".into(), Some(" c")));
        assert_eq!(split_comment("col = ##fff"), ("col = #fff".into(), None));
        assert_eq!(split_comment("# source = x"), (String::new(), Some(" source = x")));
    }

    #[test]
    fn variables_longest_match() {
        let mut vars = BTreeMap::new();
        vars.insert("dir".to_owned(), "/a".to_owned());
        vars.insert("dirs".to_owned(), "/b".to_owned());
        assert_eq!(expand_vars("$dirs/x", &vars), "/b/x");
        assert_eq!(expand_vars("$dir/x $unknown", &vars), "/a/x $unknown");
        assert_eq!(expand_vars("$dirx", &vars), "/ax");
    }

    #[test]
    fn paths_resolve() {
        let home = Path::new("/home/u");
        let base = Path::new("/home/u/.config/hypr");
        assert_eq!(resolve_path("~/x.conf", base, Some(home)), Some("/home/u/x.conf".into()));
        assert_eq!(resolve_path("configs/../a.conf", base, Some(home)), Some("/home/u/.config/hypr/a.conf".into()));
        assert_eq!(resolve_path("/etc/a", base, Some(home)), Some("/etc/a".into()));
        assert_eq!(resolve_path("~root/a", base, Some(home)), None);
        assert_eq!(resolve_path("~/a", base, None), None);
    }

    #[test]
    fn wildcards() {
        assert!(wildcard_match("*.conf", "a.conf"));
        assert!(!wildcard_match("*.conf", ".hidden.conf"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(wildcard_match("[a-c]x", "bx"));
        assert!(!wildcard_match("[!a-c]x", "bx"));
        assert!(wildcard_match("*a*b", "xxaxxb"));
        assert!(!wildcard_match("*a*b", "xxaxxbc"));
    }

    #[test]
    fn launch_detection() {
        assert!(launches_shell("nuxut-shell"));
        assert!(launches_shell("/home/u/.local/bin/nuxut-shell"));
        assert!(launches_shell("[workspace 1 silent] env RUST_LOG=info nuxut-shell"));
        assert!(launches_shell("uwsm app -- nuxut-shell --verbose"));
        assert!(launches_shell("sh -c \"nuxut-shell\""));
        assert!(launches_shell("nuxut-shell & disown"));
        assert!(!launches_shell("nuxut-shell msg volume up"));
        assert!(!launches_shell("qs -c nuxut-shellx"));
        assert!(!launches_shell("~/.config/hypr/starter.sh"));
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn scan(root: &Path, target: &Path, home: &Path) -> ChainReport {
        scan_chain(&ScanInput {
            root,
            target,
            home: Some(home),
            watch_keys: &["decoration:rounding", "general:gaps_in"],
            skip: &[],
            limits: ScanLimits::default(),
        })
    }

    /// Temel katman dosyası başta source edilir ama Nuxut'undur: bind'leri kullanıcının sayılmaz.
    #[test]
    fn nuxut_base_file_is_not_scanned() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let hypr = home.join(".config/hypr");
        let target = hypr.join("nuxut.conf");
        let base = hypr.join("nuxut-base.conf");
        write(&base, "bindd = SUPER, q, nuxut:close_window, killactive,\n");
        write(&hypr.join("hyprland.conf"), "source = ~/.config/hypr/nuxut-base.conf\nbind = SUPER, Return, exec, kitty\n");
        let skip = [base.clone()];
        let r = scan_chain(&ScanInput { root: &hypr.join("hyprland.conf"), target: &target, home: Some(home), watch_keys: &[], skip: &skip, limits: ScanLimits::default() });
        assert_eq!(r.binds.iter().map(|b| b.combo.to_string()).collect::<Vec<_>>(), ["SUPER+return"]);
        assert_eq!(r.files.len(), 1);
        let r = scan(&hypr.join("hyprland.conf"), &target, home);
        assert_eq!(r.binds.len(), 2, "atlanmazsa kendi bind'imiz kullanıcınınki sanılırdı");
    }

    /// Ayar katmanından önceki/sonraki bind'ler ayrılır (devralma sınırı); satır yoksa sınır yok.
    #[test]
    fn bind_order_marks_binds_before_settings_layer() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let hypr = home.join(".config/hypr");
        let target = hypr.join("nuxut.conf");
        write(&hypr.join("keys.conf"), "bindl = , XF86AudioPlay, exec, playerctl play-pause\n");
        write(&hypr.join("hyprland.conf"), "source = keys.conf\nsource = ~/.config/hypr/nuxut.conf\nbind = , Print, exec, grim\n");
        let r = scan(&hypr.join("hyprland.conf"), &target, home);
        assert_eq!(r.binds.len(), 2);
        assert_eq!(r.bind_order, Some(1));
        write(&hypr.join("hyprland.conf"), "source = keys.conf\n");
        assert_eq!(scan(&hypr.join("hyprland.conf"), &target, home).bind_order, None);
    }

    #[test]
    fn finds_source_through_chain_relative_glob_and_vars() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let hypr = home.join(".config/hypr");
        let target = hypr.join("nuxut.conf");
        write(&target, "");
        write(&hypr.join("hyprland.conf"), "$cfg = ~/.config/hypr/configs\nsource = configs/a.conf\nsource=$cfg/conf.d/*.conf\n");
        write(&hypr.join("configs/a.conf"), "decoration {\n  rounding = 4\n}\n");
        write(&hypr.join("configs/conf.d/10.conf"), "general:gaps_in = 2\n");
        write(&hypr.join("configs/conf.d/20.conf"), "source = ~/.config/hypr/nuxut.conf\n");
        let r = scan(&hypr.join("hyprland.conf"), &target, home);
        assert!(!r.truncated);
        assert_eq!(r.files.len(), 4);
        let at = r.source_at.unwrap();
        assert!(r.files[at.file].ends_with("20.conf"));
        let keys: Vec<_> = r.assignments.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["decoration:rounding", "general:gaps_in"]);
        assert_eq!(r.source_order, Some(2), "iki atama da kaynaktan önce okunur");

        // Kaynaktan sonraki atama: okuma sırasında source'tan sonra.
        write(&hypr.join("hyprland.conf"), "general:gaps_in = 1\nsource = ~/.config/hypr/nuxut.conf\ndecoration {\n  rounding = 4\n}\n");
        let r = scan(&hypr.join("hyprland.conf"), &target, home);
        assert_eq!(r.source_order, Some(1));
        assert_eq!(r.assignments[1].0, "decoration:rounding");

        // İki kez source: aradaki atama son source'ta ezilir, yalnız sonrası override.
        write(
            &hypr.join("hyprland.conf"),
            "source = ~/.config/hypr/nuxut.conf\ngeneral:gaps_in = 1\nsource = ~/.config/hypr/nuxut.conf\ndecoration {\n  rounding = 4\n}\n",
        );
        let r = scan(&hypr.join("hyprland.conf"), &target, home);
        assert_eq!(r.source_at, Some(Location { file: 0, line: 1 }), "source_at ilk satır kalır");
        assert_eq!(r.source_order, Some(1), "sınır son source");
        assert_eq!(r.bind_order, Some(0));
        let after: Vec<_> = r.assignments[1..].iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(after, ["decoration:rounding"]);
    }

    #[test]
    fn commented_source_and_external_launch_and_binds() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let hypr = home.join(".config/hypr");
        let target = hypr.join("nuxut.conf");
        write(
            &hypr.join("hyprland.conf"),
            "$mainMod = SUPER\n# source = ~/.config/hypr/nuxut.conf\nsource = old.conf\nbind = $mainMod, N, exec, foo\nbindel = , XF86AudioRaiseVolume, exec, x\n",
        );
        write(&hypr.join("old.conf"), "exec-once = nuxut-shell # eski\n");
        let r = scan(&hypr.join("hyprland.conf"), &target, home);
        assert_eq!(r.source_at, None);
        assert_eq!(r.commented_at, Some(Location { file: 0, line: 2 }));
        assert_eq!(r.external_launch, Some(Location { file: 1, line: 1 }));
        let combos: Vec<String> = r.binds.iter().map(|b| b.combo.to_string()).collect();
        assert_eq!(combos, ["SUPER+n", "xf86audioraisevolume"]);
    }

    #[test]
    fn target_contents_are_not_scanned_and_cycles_stop() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let hypr = home.join(".config/hypr");
        let target = hypr.join("nuxut.conf");
        write(&target, "exec-once = nuxut-shell\nbind = SUPER, N, exec, x\n");
        write(&hypr.join("hyprland.conf"), "source = a.conf\nsource = ~/.config/hypr/nuxut.conf\n");
        write(&hypr.join("a.conf"), "source = hyprland.conf\n");
        let r = scan(&hypr.join("hyprland.conf"), &target, home);
        assert_eq!(r.source_at, Some(Location { file: 0, line: 2 }));
        assert_eq!(r.external_launch, None);
        assert!(r.binds.is_empty());
        assert_eq!(r.files.len(), 2);
        assert!(!r.truncated);
    }

    #[test]
    fn glob_covering_missing_target_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let hypr = home.join(".config/hypr");
        write(&hypr.join("hyprland.conf"), "source = ~/.config/hypr/*.conf\n");
        let r = scan(&hypr.join("hyprland.conf"), &hypr.join("nuxut.conf"), home);
        assert!(r.source_at.is_some());
    }

    #[test]
    fn limits_mark_truncated() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let hypr = home.join(".config/hypr");
        // 12 derinlikte zincir.
        for i in 0..12 {
            write(&hypr.join(format!("{i}.conf")), &format!("source = {}.conf\n", i + 1));
        }
        let r = scan(&hypr.join("0.conf"), &hypr.join("nuxut.conf"), home);
        assert!(r.truncated);
        // Okunamayan kök.
        let r = scan(&hypr.join("yok.conf"), &hypr.join("nuxut.conf"), home);
        assert!(r.truncated);
        // Çok uzun satır.
        write(&hypr.join("long.conf"), &"a".repeat(5000));
        assert!(scan(&hypr.join("long.conf"), &hypr.join("nuxut.conf"), home).truncated);
    }
}
