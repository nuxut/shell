//! Bekçi (ADR-0006): shell kodunda gömülü font ailesi, ikon fontu glifi ya da ikon adı dizgisi
//! yok. Metin stili yalnızca `text_style(theme, TextRole::…)` ile (gerekirse
//! `.with_tabular_numbers()`), ikon yalnızca `Icon::named(IconName::…)` ile alınır; aileler ve
//! glifler framework'te (nuxut-theme, `icon_names.rs`). Config'ten gelen ikon adı
//! `parse::<IconName>()` ile çözülür.
//!
//! Taranan: `src/` ve `examples/` altındaki `.rs` dosyaları. Aile/glif kuralında hariç: yorum
//! satırları, `#[cfg(test)]` ardından gelen `mod … { }` bloğu, `tests.rs` adlı test modülü dosyaları,
//! `// font-istisnası:` işaretli satır (ya da hemen önceki satırda bu yorum) ve dosya başında
//! `//! font-istisnası-dosya: <gerekçe>` taşıyan dosya (framework karşılığı gelene kadar; ör.
//! uygulama marka glifleri). `Icon::symbol(` (geçiş kısa yolu) testler dahil her yerde yasaktır.

use std::path::{Path, PathBuf};

use crate::support::repo;

const ALLOW_MARK: &str = "// font-istisnası:";
/// Dosyanın tamamını aile/glif kuralından çıkaran işaret (gerekçeyle, satır başında).
const FILE_ALLOW_MARK: &str = "//! font-istisnası-dosya:";
/// Aile adını dizgi sabitiyle kuran kalıplar.
const FAMILY_SETTERS: &[&str] = &["TextStyle::new(\"", ".family = \"", "family: \""];
/// Dizgi sabiti içinde görülmemesi gereken aile adları (genel aileler dahil).
const FAMILY_NAMES: &[&str] = &[
    "\"sans-serif\"",
    "\"serif\"",
    "\"monospace\"",
    "Inter\"",
    "Source Serif",
    "JetBrains",
    "Nerd Font",
    "Material Symbols",
    "Noto Sans",
    "Roboto",
    "DejaVu",
    "Cantarell",
    "Adwaita Sans",
];
const SYMBOL_SHORTCUT: &str = "Icon::symbol(";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Satırda gömülü aile adı ya da Unicode özel kullanım alanı (ikon fontu glifi) var mı.
fn font_violation(line: &str) -> Option<&'static str> {
    if FAMILY_SETTERS.iter().any(|p| line.contains(p)) || FAMILY_NAMES.iter().any(|n| line.contains(n)) {
        return Some("gömülü font ailesi");
    }
    // `\u{e000}`–`\u{f8ff}` ve `\u{f0000}`+ kaçışları ya da doğrudan PUA karakteri.
    let escaped_pua = line.match_indices("\\u{").any(|(i, _)| {
        let hex: String = line[i + 3..].chars().take_while(char::is_ascii_hexdigit).collect();
        u32::from_str_radix(&hex, 16).is_ok_and(is_pua)
    });
    if escaped_pua || line.chars().any(|c| is_pua(c as u32)) {
        return Some("gömülü ikon glifi");
    }
    None
}

fn is_pua(c: u32) -> bool {
    (0xE000..=0xF8FF).contains(&c) || c >= 0xF0000
}

/// Dosya başındaki `//!` bloğu aile/glif kuralından çıkarma işareti taşıyor mu.
fn file_allowed(text: &str) -> bool {
    text.lines().take_while(|l| l.starts_with("//!")).any(|l| l.starts_with(FILE_ALLOW_MARK))
}

/// Dosyadaki ihlaller: (satır no, tür, satır). `font_rule`: aile/glif kuralı uygulanır mı
/// (`Icon::symbol` her zaman).
fn violations(text: &str) -> Vec<(usize, &'static str, String)> {
    violations_with(text, !file_allowed(text))
}

/// Satırdaki `{` − `}` (dizgi/karakter içindekiler de sayılır; kaynakta dengeli varsayılır).
fn brace_delta(line: &str) -> i32 {
    line.chars().map(|c| match c {
        '{' => 1,
        '}' => -1,
        _ => 0,
    }).sum()
}

fn violations_with(text: &str, font_rule: bool) -> Vec<(usize, &'static str, String)> {
    let mut out = Vec::new();
    // Muafiyet yalnızca `#[cfg(test)]`'in hemen ardından gelen `mod … {` bloğuna; süslü parantez
    // derinliğiyle izlenir, blok kapanınca biter (`#[cfg(test)] fn …` sonrasını muaf tutmaz).
    let mut cfg_test_pending = false;
    let mut test_depth: i32 = 0;
    let mut prev_allowed = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        let is_comment = t.starts_with("//");
        let was_in_tests = test_depth > 0;
        let mut item = t;
        if let Some(rest) = t.strip_prefix("#[cfg(test)]") {
            cfg_test_pending = true;
            item = rest.trim_start();
        }
        if test_depth > 0 {
            test_depth += brace_delta(line);
        } else if cfg_test_pending && !item.is_empty() && !item.starts_with("//") && !item.starts_with("#[") {
            cfg_test_pending = false;
            let is_mod = ["mod ", "pub mod ", "pub(crate) mod ", "pub(super) mod "].iter().any(|p| item.starts_with(p));
            if is_mod && item.contains('{') {
                test_depth = brace_delta(item);
            }
        }
        let in_tests = was_in_tests || test_depth > 0;
        if !is_comment && line.contains(SYMBOL_SHORTCUT) {
            out.push((i + 1, "Icon::symbol (Icon::named(IconName::…) kullanın)", line.trim().to_owned()));
        }
        let allowed = line.contains(ALLOW_MARK);
        if font_rule
            && !in_tests
            && !allowed
            && !prev_allowed
            && !is_comment
            && let Some(kind) = font_violation(line)
        {
            out.push((i + 1, kind, line.trim().to_owned()));
        }
        prev_allowed = allowed && is_comment;
    }
    out
}

#[test]
fn no_hardcoded_fonts_or_icon_strings_in_shell_sources() {
    let root = &repo::manifest_dir();
    let mut files = Vec::new();
    for dir in ["src", "examples"] {
        rust_files(&root.join(dir), &mut files);
    }
    files.sort();
    repo::assert_enough_sources(root, files.len(), repo::MIN_SHELL_SOURCES);
    let mut report = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        // `tests.rs` adlı dosya bir modülün `#[cfg(test)] mod tests;` gövdesidir.
        let font_rule = f.file_name().is_none_or(|n| n != "tests.rs") && !file_allowed(&text);
        for (n, kind, line) in violations_with(&text, font_rule) {
            report.push(format!("{}:{n}: {kind}: {line}", f.strip_prefix(root).unwrap_or(&f).display()));
        }
    }
    assert!(
        report.is_empty(),
        "fontu `text_style(theme, TextRole::…)`, ikonu `Icon::named(IconName::…)` ile alın \
         (gerçekten istisnaysa satıra `{ALLOW_MARK} <gerekçe>`):\n{}",
        report.join("\n")
    );
}

#[test]
fn detector_self_check() {
    let one = |s: &str| violations(s).len();
    assert_eq!(one(r#"TextStyle::new("Material Symbols Rounded".to_owned(), 14.0, 400)"#), 1);
    assert_eq!(one(r#"let f = "monospace";"#), 1);
    assert_eq!(one(r#"style.family = "Hack".into();"#), 1);
    assert_eq!(one(r#"label.set_text("\u{f0156}")"#), 1);
    assert_eq!(one(r#"label.set_text("\u{e5cd}")"#), 1);
    assert_eq!(one(r#"IconView::new(t, Icon::symbol("wifi"))"#), 1);
    assert_eq!(one("#[cfg(test)]\nmod tests {\n    let i = Icon::symbol(\"wifi\");\n}"), 1, "testte de yasak");
    assert_eq!(one("#[cfg(test)]\nmod tests {\n    let f = \"monospace\";\n}"), 0, "aile kuralı test modülünde yok");
    assert_eq!(one("#[cfg(test)]\nfn h() {}\nlet f = \"monospace\";"), 1, "yalnızca mod bloğu muaf");
    assert_eq!(one("#[cfg(test)]\nmod tests {\n    fn a() {\n    }\n}\nlet f = \"monospace\";"), 1, "blok kapanınca muafiyet biter");
    assert_eq!(one("#[cfg(test)] mod tests {\n    let f = \"monospace\";\n}"), 0, "aynı satırda mod");
    assert_eq!(one("#[cfg(test)]\nmod tests;\nlet f = \"monospace\";"), 1, "dış dosya modülü muaf tutmaz");
    assert_eq!(one("// font-istisnası: gerekçe\nlet f = \"monospace\";"), 0);
    assert_eq!(one(r#"TextStyle::new(family, size, weight)"#), 0);
    assert_eq!(one(r#"label.set_text("\u{2026}")"#), 0);
    assert_eq!(one(r#"Icon::symbolic(texture)"#), 0);
    assert_eq!(one(r#"let s = "Seriler";"#), 0);
    let allowed_file = "//! Uygulama glifleri.\n//! font-istisnası-dosya: marka glifleri\n\nconst G: char = '\\u{f0239}';";
    assert_eq!(one(allowed_file), 0, "dosya işareti");
    assert_eq!(one("//! font-istisnası-dosya: x\nlet i = Icon::symbol(\"wifi\");"), 1, "Icon::symbol dosya işaretinde de yasak");
    assert_eq!(one("fn f() {}\n//! font-istisnası-dosya: geç\nlet f = \"monospace\";"), 1, "işaret yalnızca dosya başında");
}
