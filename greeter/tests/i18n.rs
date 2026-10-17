//! Giriş ekranı kataloğu kapısı (ADR-0010 §4): tr/en tam, yorum, değişken, önek.

#[test]
fn i18n_catalog() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/locales");
    nuxut::i18n::check::catalog_embedded(&nuxut_greeter::i18n::source(), dir, "shell").assert_ok();
}
