//! Shell'in Fluent kataloğu (ADR-0010; `locales/<dil>.ftl`, anahtarlar `shell-…`).
//!
//! Metin `nuxut::i18n::t!("shell-…")` ile istenir; dil değişince süreç geneli katalog
//! yenilenir, metni önbelleğe alan bileşen yeniden kurulmalıdır.

use std::sync::{Arc, Once};

use nuxut::i18n::{Catalog, LanguagePrefs, Source};

/// Gömülü `shell` kaynağı.
pub fn source() -> Source {
    nuxut::i18n::embed!("shell")
}

/// Testler için süreç geneli kataloğu Türkçeye, sayı biçimini tr_TR'ye sabitler ("%45", "1,5";
/// ortam ve kullanıcı çeviri dizini okunmaz). Metin karşılaştıran her test önce çağırır; tekrar
/// çağrı zararsız. Süreç genelinde tek kurucu: başka bir test kataloğu kurulursa sıra bozar.
pub fn use_turkish_for_tests() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let numeric = nuxut::locale::Locale::parse("tr_TR").expect("tr_TR");
        let prefs = LanguagePrefs::fixed(&["tr"]).with_numeric(numeric);
        nuxut::i18n::set_current(Arc::new(Catalog::load(&[source(), nuxut_wallpapers::i18n_source()], &prefs)));
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn turkish_catalog_for_tests() {
        super::use_turkish_for_tests();
        assert_eq!(nuxut::i18n::t!("shell-notifications-title"), "Bildirimler");
    }
}
