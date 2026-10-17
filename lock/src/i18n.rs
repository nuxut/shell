//! Kilit ekranının Fluent kataloğu (ADR-0010; `lock/locales/<dil>.ftl`). Anahtarlar repo
//! önekiyle `shell-lock-…`; kilit ayrı süreç olduğundan kendi gömülü kaynağı (`lock`) vardır
//! ve shell kataloğunu yüklemez. Dil süreç başında bir kez seçilir.

use std::sync::{Arc, Once};

use nuxut::i18n::{Catalog, LanguagePrefs, Source};

/// Gömülü `lock` kaynağı.
pub fn source() -> Source {
    nuxut::i18n::embed!("lock")
}

/// Testler için süreç geneli kataloğu Türkçeye sabitler (ortam ve kullanıcı çeviri dizini
/// okunmaz). Metin karşılaştıran her test önce çağırır; tekrar çağrı zararsız.
pub fn use_turkish_for_tests() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prefs = LanguagePrefs::fixed(&["tr"]);
        nuxut::i18n::set_current(Arc::new(Catalog::load(&[source()], &prefs)));
    });
}
