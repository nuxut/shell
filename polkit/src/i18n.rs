//! Diyaloğun Fluent kataloğu (ADR-0010; `polkit/locales/<dil>.ftl` — tr kaynak, en taslak;
//! anahtarlar `polkit-…`).

/// Gömülü `polkit` kaynağı.
pub fn source() -> nuxut::i18n::Source {
    nuxut::i18n::embed!("polkit")
}
