//! Bekçi: shell kodunda koda gömülü renk yok. Her renk temadan (`theme.colors`) gelir;
//! varsayılanlar yalnızca nuxut-theme'de ve Ayarlar'dan değiştirilebilir. Shell'e özgü bir renk
//! gerekiyorsa framework'ten yeni bir tema rolü istenir.
//!
//! Taranan: `src/` ve `examples/` altındaki `.rs` dosyaları. Hariç: yorum satırları,
//! `#[cfg(test)]` ile başlayan test modülleri (dosyanın o satırından sonrası),
//! `// renk-matematiği:` ile işaretli satırlar (ör. karıştırma katsayısı) ve
//! `// renk-verisi:` ile işaretli satırlar (dış veriden ayrıştırılan renk, ör. takvim rengi).

use std::path::{Path, PathBuf};

use crate::support::repo;

const FORBIDDEN: &[&str] =
    &["Color::rgb(", "Color::rgba(", "Color::from_rgba8(", "Color::from_hex(", "from_rgba8(", "Color::WHITE", "Color::BLACK"];
const ALLOW_MARKS: [&str; 2] = ["// renk-matematiği:", "// renk-verisi:"];

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

/// `"#rrggbb"` ya da `"#rrggbbaa"` kalıbı (tırnak içinde, 6 veya 8 onaltılık rakam).
fn has_hex_color_literal(line: &str) -> bool {
    let b = line.as_bytes();
    (0..b.len()).any(|i| {
        if b[i] != b'"' || b.get(i + 1) != Some(&b'#') {
            return false;
        }
        let digits = b[i + 2..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
        (digits == 6 || digits == 8) && b.get(i + 2 + digits) == Some(&b'"')
    })
}

fn violations(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("#[cfg(test)]") {
            break;
        }
        if t.starts_with("//") || ALLOW_MARKS.iter().any(|m| line.contains(m)) {
            continue;
        }
        // Satır sonu yorumu tarama dışı.
        let code = line.split("//").next().unwrap_or(line);
        if FORBIDDEN.iter().any(|p| code.contains(p)) || has_hex_color_literal(code) {
            found.push((n + 1, line.trim().to_owned()));
        }
    }
    found
}

#[test]
fn no_hardcoded_colors_in_shell_sources() {
    let root = &repo::manifest_dir();
    let mut files = Vec::new();
    for dir in ["src", "examples"] {
        rust_files(&root.join(dir), &mut files);
    }
    repo::assert_enough_sources(root, files.len(), repo::MIN_SHELL_SOURCES);
    let mut report = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        for (line, code) in violations(&text) {
            report.push(format!("{}:{line}: {code}", f.strip_prefix(root).unwrap_or(&f).display()));
        }
    }
    assert!(
        report.is_empty(),
        "koda gömülü renk bulundu (temadan oku ya da framework'ten tema rolü iste):\n{}",
        report.join("\n")
    );
}

#[test]
fn guard_catches_and_allows_correctly() {
    assert_eq!(violations("let c = Color::rgb(1.0, 0.0, 0.0);").len(), 1);
    assert_eq!(violations("let c = Color::from_hex(\"#ff0000\");").len(), 1);
    assert_eq!(violations("let s = \"#12141a\";").len(), 1);
    assert_eq!(violations("let s = \"#12141aff\";").len(), 1);
    assert!(violations("// Color::rgb(1.0, 0.0, 0.0)").is_empty());
    assert!(violations("let k = 0.55; // renk-matematiği: soluk metin oranı").is_empty());
    assert!(violations("Color::from_hex(&e.color).ok() // renk-verisi: daemon").is_empty());
    assert!(violations("let id = \"#workspace\";").is_empty());
    assert!(violations("let s = \"#123\";").is_empty(), "3 haneli değil, kural 6/8");
    assert!(violations("x\n#[cfg(test)]\nlet c = Color::WHITE;").is_empty());
    assert!(violations("let c = theme.colors.primary; // \"#ffffff\" değil").is_empty());
}
