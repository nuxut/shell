//! Giriş ekranının Fluent kataloğu (`greeter/locales/<dil>.ftl`, anahtarlar `shell-greeter-…`).
//! Durum satırları kilitle ortak olduğundan kilidin kaynağı da yüklenir. Dil greeter'ın
//! yayımlanmış `framework.toml [locale]`'inden, yoksa ortamdan (greetd'nin `LANG`'ı).

use std::sync::{Arc, Once};

use nuxut::i18n::{Catalog, LanguagePrefs, Source};

pub fn source() -> Source {
    nuxut::i18n::embed!("greeter")
}

pub fn sources() -> [Source; 2] {
    [source(), nuxut_lock::i18n::source()]
}

/// Testler için süreç geneli kataloğu Türkçeye sabitler.
pub fn use_turkish_for_tests() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prefs = LanguagePrefs::fixed(&["tr"]);
        nuxut::i18n::set_current(Arc::new(Catalog::load(&sources(), &prefs)));
    });
}
