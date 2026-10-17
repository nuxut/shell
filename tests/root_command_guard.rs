//! Kök komutu bekçisi (kullanıcı kararı 2026-10-17): ürün kullanıcıya elle `sudo`, `chown`,
//! `pkexec` ya da `doas` komutu yazdırmaz. Kök yetkisi gereken her düzeltme bir düğmeyle yapılır
//! (Ayarlar, doctor ya da bildirim → pkexec + nuxut-system-helper, Nuxut polkit penceresi); kök
//! gerektirmeyen çözüm (kullanıcının bağını/dosyasını devralmak) her zaman önce gelir.
//!
//! Taranan kullanıcı metinleri:
//! - `locales/*.ftl`: yorum dışı her satır (çeviri metni).
//! - `install.sh`: yorum dışı satırlardaki tırnaklı metinler (`note`/`warn`/`say`/`die`/`ask`
//!   iletileri). Betiğin kendi çalıştırdığı komut (`sudo pacman -S …`) tırnak dışında kalır.
//! - `src/` ve `crates/` Rust kaynakları: test dışı dizge sabitleri. Çalıştırılan ikilinin yolu
//!   (`Command::new("/usr/bin/pkexec")`) kelimeden sonra boşluk taşımadığı için geçer.

use std::path::{Path, PathBuf};

const WORDS: [&str; 4] = ["sudo", "chown", "pkexec", "doas"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `text` içinde kelime olarak (öncesi harf/rakam değil) geçen ve ardından boşluk gelen yasak
/// komut adı: "sudo pacman", "chown $USER". Yol ya da tanımlayıcı içindeki geçiş (`/usr/bin/pkexec"`,
/// `pkexec_uid`) komut önerisi değildir.
fn command_word(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    WORDS.into_iter().find(|w| {
        lower.match_indices(w).any(|(i, _)| {
            let before = lower[..i].chars().next_back();
            let after = lower[i + w.len()..].chars().next();
            !before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '/' || c == '-') && after == Some(' ')
        })
    })
}

/// Satırdaki çift tırnaklı parçalar (kaçışlı tırnak atlanır).
fn quoted(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    let mut esc = false;
    for c in line.chars() {
        match (&mut cur, c) {
            (Some(s), _) if esc => {
                s.push(c);
                esc = false;
            }
            (Some(_), '\\') => esc = true,
            (Some(_), '"') => out.extend(cur.take()),
            (Some(s), _) => s.push(c),
            (None, '"') => cur = Some(String::new()),
            (None, _) => {}
        }
    }
    out
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if !matches!(name, "target" | "tests" | "examples" | "benches") {
                rust_files(&p, out);
            }
        } else if name.ends_with(".rs") && !name.ends_with("tests.rs") && name != "test.rs" {
            out.push(p);
        }
    }
}

#[test]
fn command_word_matches_suggestions_only() {
    assert_eq!(command_word("çalıştırın: sudo pacman -S mpv"), Some("sudo"));
    assert_eq!(command_word("Sudo chown $USER a"), Some("sudo"));
    assert_eq!(command_word("pkexec /usr/lib/nuxut/x"), Some("pkexec"));
    assert_eq!(command_word("/usr/bin/pkexec"), None);
    assert_eq!(command_word("PKEXEC_UID"), None);
    assert_eq!(command_word("pseudo terminal"), None);
    assert_eq!(quoted(r#"warn "a \"b\" sudo x" || note "c""#), vec![r#"a "b" sudo x"#.to_owned(), "c".to_owned()]);
}

#[test]
fn locales_do_not_suggest_root_commands() {
    for name in ["locales/tr.ftl", "locales/en.ftl"] {
        let text = std::fs::read_to_string(root().join(name)).unwrap();
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            assert!(command_word(line).is_none(), "{name}:{}: {line}", i + 1);
        }
    }
}

#[test]
fn install_script_does_not_suggest_root_commands() {
    let text = std::fs::read_to_string(root().join("install.sh")).unwrap();
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        for q in quoted(line) {
            assert!(command_word(&q).is_none(), "install.sh:{}: {line}", i + 1);
        }
    }
}

/// Test modülü (`#[cfg(test)]` sonrası) taranmaz: testler yasak metni örnek olarak içerebilir.
#[test]
fn rust_sources_do_not_suggest_root_commands() {
    let mut files = Vec::new();
    rust_files(&root().join("src"), &mut files);
    rust_files(&root().join("crates"), &mut files);
    assert!(files.len() > 50);
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        for (i, line) in text.lines().enumerate() {
            if line.trim_start() == "#[cfg(test)]" {
                break;
            }
            if line.trim_start().starts_with("//") {
                continue;
            }
            for q in quoted(line) {
                assert!(command_word(&q).is_none(), "{}:{}: {line}", f.display(), i + 1);
            }
        }
    }
}
