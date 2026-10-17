//! Testlerde depo yolu: ÇALIŞMA ZAMANINDA `CARGO_MANIFEST_DIR` (docs/ARCHITECTURE.md "Testler").
//!
//! Neden: cargo, paketin mutlak yolu değişince (worktree taşınması, sembolik bağlı kum havuzu,
//! `--manifest-path` ile başka yoldan derleme) test ikilisini yeniden derlemez; derleme anında
//! `env!("CARGO_MANIFEST_DIR")` ile gömülen yol bayatlar. Bayat ikili ya ölü yolu ya da BAŞKA
//! bir worktree'nin ağacını kullanır: kaynak tarayan bekçiler (renk, yazı tipi, maskot, i18n)
//! yanlış ağacı tarayıp sessizce geçer (kök neden: e413de1). `cargo test` değişkeni test
//! sürecine verir; derleme anındaki değer yalnızca ikili cargo dışında çalıştırılınca yedektir.

// Her test ikilisi yalnızca ihtiyacı olanı kullanır.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Kabuk paketinin kökü (`Cargo.toml`'un dizini). Kök yoksa ya da paket kökü değilse panik.
pub fn manifest_dir() -> PathBuf {
    let root = std::env::var_os("CARGO_MANIFEST_DIR")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    assert!(
        root.join("Cargo.toml").is_file(),
        "paket kökü yok: {} (ikili silinmiş bir yoldan mı derlendi?)",
        root.display()
    );
    root
}

/// Paket köküne göre `rel`; yol yoksa açık mesajla panik.
pub fn repo_path(rel: &str) -> PathBuf {
    let p = manifest_dir().join(rel);
    assert!(p.exists(), "depo yolu yok: {}", p.display());
    p
}

/// Bekçinin taradığı ağacın en az bu kadar `.rs` içermesi beklenir (ağaç şu an ≈200 dosya).
/// Daha azı yanlış/boş ağacın taranması demektir: bekçi sessizce yeşil vermesin.
pub const MIN_SHELL_SOURCES: usize = 100;

/// Taranan dosya sayısı `min`'in altındaysa panik (sessiz yeşil yok).
pub fn assert_enough_sources(root: &Path, count: usize, min: usize) {
    assert!(
        count >= min,
        "{} altında taranan .rs çok az ({count} < {min}): yanlış ya da boş ağaç taranıyor",
        root.display()
    );
}

/// `dir` altındaki `.rs` dosyalarının sayısı (özyinelemeli; `target` atlanır).
pub fn count_rust_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .map(|p| {
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "target") { 0 } else { count_rust_files(&p) }
            } else {
                usize::from(p.extension().is_some_and(|x| x == "rs"))
            }
        })
        .sum()
}
