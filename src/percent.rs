//! Yüzde metni: tek yol katalogdaki `NUMBER($percent, style: "percent")` mesajı. Böylece kodda
//! biçimlenen yüzdeler (kaydırıcı, sysmon, pil) katalog mesajlarındakilerle aynı locale'i
//! (framework.toml `[locale] formats`, yoksa ortam) ve aynı CLDR kalıbını kullanır:
//! tr "%50", en "50%", de "50 %". Dil/biçim çalışırken değişince sonraki çağrı yenisini görür.

/// 0..=100 ölçekli değeri (ör. `45.0`) tamsayıya yuvarlanmış yüzde metni yapar. Sınır yok
/// (ses %150 olabilir); NaN/sonsuz → 0.
pub fn text(percent: f64) -> String {
    let percent = if percent.is_finite() { percent.round() } else { 0.0 };
    nuxut::i18n::t!("shell-bar-status-percent", percent = percent / 100.0)
}

/// Katalogdaki `$percent` değişkenleri oran bekler (`NUMBER(style: "percent")`: 0.45 → "%45").
pub fn ratio(percent: impl Into<f64>) -> f64 {
    let p = percent.into();
    if p.is_finite() { p / 100.0 } else { 0.0 }
}

/// Testler: [`crate::i18n::use_turkish_for_tests`] (kaynak dil Türkçe, sayı biçimi tr_TR).
#[cfg(test)]
pub(crate) fn tr_catalog() {
    crate::i18n::use_turkish_for_tests();
}

#[cfg(test)]
mod tests {
    use nuxut::i18n::{Args, Catalog, LanguagePrefs};

    use super::*;

    fn catalog(lang: &str, numeric: &str) -> Catalog {
        let prefs = LanguagePrefs::fixed(&[lang]).with_numeric(nuxut::locale::Locale::parse(numeric).unwrap());
        Catalog::load(&[nuxut::i18n::embed!("shell")], &prefs)
    }

    #[test]
    fn current_catalog_tr() {
        tr_catalog();
        assert_eq!(text(45.0), "%45");
        assert_eq!(text(23.4), "%23");
        assert_eq!(text(150.0), "%150");
        assert_eq!(text(f64::NAN), "%0");
        // Kayan nokta artığı yazılmaz (0.29 * 100 = 28.999…).
        assert_eq!(text(29.0), "%29");
    }

    /// Kalıp dile değil sayı locale'ine bağlı: tr "%45", en "45%", de "45 %" (U+00A0).
    #[test]
    fn pattern_follows_numeric_locale() {
        let fmt = |c: &Catalog, p: f64| {
            let mut args = Args::new();
            args.set("percent", ratio(p));
            c.format("shell-bar-status-percent", &args).into_owned()
        };
        assert_eq!(fmt(&catalog("tr", "tr_TR"), 45.0), "%45");
        assert_eq!(fmt(&catalog("en", "en_US"), 45.0), "45%");
        assert_eq!(fmt(&catalog("en", "de_DE"), 45.0), "45\u{a0}%");
        assert_eq!(fmt(&catalog("en", "en_US"), 28.999_999_999_999_996), "29%");
    }
}
