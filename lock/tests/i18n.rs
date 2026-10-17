//! Kilit kataloğu kapısı (ADR-0010 §4): tr/en tam, yorum, değişken, önek. Gömülü metin bekçisi
//! kök shell testinde (`lock/src` de taranır).

#[test]
fn i18n_catalog() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/locales");
    nuxut::i18n::check::catalog_embedded(&nuxut_lock::i18n::source(), dir, "shell").assert_ok();
}
