//! Diyalog kataloğu (ADR-0010 §4): tr/en tam, yorum, değişken, [max], `polkit-` öneki.

use std::path::PathBuf;

/// Katalog dizini ÇALIŞMA ZAMANINDA `CARGO_MANIFEST_DIR`'den çözülür; derleme anındaki `env!`
/// yalnızca cargo dışı çalıştırmada yedektir. Cargo paket yolu değişince test ikilisini yeniden
/// derlemez; gömülü yol bayatlayıp başka bir ağacı gösterebilir (docs/ARCHITECTURE.md "Testler").
fn locales_dir() -> PathBuf {
    let root = std::env::var_os("CARGO_MANIFEST_DIR")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let dir = root.join("locales");
    assert!(dir.is_dir(), "katalog dizini yok: {} (ikili silinmiş bir yoldan mı derlendi?)", dir.display());
    dir
}

#[test]
fn i18n_catalog() {
    nuxut::i18n::check::catalog_embedded(&nuxut::i18n::embed!("polkit"), locales_dir(), "polkit").assert_ok();
}
