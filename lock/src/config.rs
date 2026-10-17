//! `~/.config/nuxut/lock.toml`. Bozuk dosya, yanlış tip ya da sınır dışı değer **kilidi asla
//! engellemez**: alan varsayılana döner ve uyarı üretilir.

use crate::assets::WallpaperSource;

use chrono::format::{Item, StrftimeItems};

#[derive(Debug, Clone, PartialEq)]
pub struct LockConfig {
    /// Boş: shell.toml `[wallpaper] path` (masaüstüyle aynı; yerleşik varsayılan dahil), o da
    /// boşsa `palette.toml [meta] image`, o da yoksa düz yüzey ([`crate::assets::resolve_wallpaper`]).
    pub wallpaper: Option<WallpaperSource>,
    pub blur: bool,
    /// Duvar kağıdı üstü karartma, 0..=0.9.
    pub dim: f32,
    /// `None` (yazılmamış ya da `""`) = otomatik: `framework.toml [locale] hour_cycle` (t119);
    /// yazılmış strftime kalıbı kazanır.
    pub clock_format: Option<String>,
    pub date_format: String,
    pub fingerprint: bool,
    /// 1..=10.
    pub fingerprint_tries: u32,
    /// Gizlilik: varsayılan kapalı.
    pub show_media: bool,
    pub show_notifications: bool,
}

impl Default for LockConfig {
    fn default() -> Self {
        Self {
            wallpaper: None,
            blur: true,
            dim: 0.55,
            clock_format: None,
            date_format: "%A, %e %B".into(),
            fingerprint: true,
            fingerprint_tries: 3,
            show_media: false,
            show_notifications: false,
        }
    }
}

/// Dosya boyutu üst sınırı; fazlası okunmaz (varsayılanlar + uyarı).
pub const MAX_CONFIG_BYTES: usize = 64 * 1024;

impl LockConfig {
    /// Config dizinindeki `lock.toml` (`nuxut::config::config_dir`); okunamaz/bozuksa
    /// varsayılanlar, uyarılar loglanır (kilit asla engellenmez). Giriş ekranı da aynı dosyayı
    /// kendi yayımlanmış config dizininden okur.
    pub fn load() -> Self {
        use std::io::Read as _;
        let Some(path) = nuxut::config::config_dir().map(|d| d.join("lock.toml")) else {
            return Self::default();
        };
        let mut text = String::new();
        let read = std::fs::File::open(&path)
            .and_then(|f| f.take(MAX_CONFIG_BYTES as u64 + 1).read_to_string(&mut text));
        match read {
            Ok(_) => {
                let (config, warnings) = Self::parse(&text);
                for w in warnings {
                    tracing::warn!("{w}");
                }
                config
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                tracing::warn!("lock.toml okunamadı ({:?}); varsayılanlar", e.kind());
                Self::default()
            }
        }
    }

    /// TOML metnini çözer. Hiçbir girdi hata döndürmez.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut cfg = Self::default();
        let mut warnings = Vec::new();
        if text.len() > MAX_CONFIG_BYTES {
            warnings.push(format!(
                "lock.toml {MAX_CONFIG_BYTES} baytı aşıyor; varsayılanlar"
            ));
            return (cfg, warnings);
        }
        let table = match text.parse::<toml::Table>() {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!(
                    "lock.toml okunamadı ({}); varsayılanlar",
                    e.message()
                ));
                return (cfg, warnings);
            }
        };
        for (key, value) in &table {
            let mut bad = |what: &str| {
                warnings.push(format!(
                    "lock.toml `{key}`: {what}; varsayılan kullanılıyor"
                ))
            };
            match key.as_str() {
                "wallpaper" => match value.as_str() {
                    Some("") => cfg.wallpaper = None,
                    Some(s) if s.len() <= 4096 && !s.contains('\0') => {
                        let mut note = None;
                        cfg.wallpaper = WallpaperSource::parse(s, &mut |w| note = Some(w));
                        if let Some(w) = note {
                            bad(&w);
                        }
                    }
                    _ => bad("geçerli bir yol değil"),
                },
                "blur" => set_bool(value, &mut cfg.blur, &mut bad),
                "dim" => match value
                    .as_float()
                    .or_else(|| value.as_integer().map(|i| i as f64))
                {
                    Some(v) if v.is_finite() && (0.0..=0.9).contains(&v) => cfg.dim = v as f32,
                    _ => bad("0..=0.9 aralığında sayı olmalı"),
                },
                "clock_format" => match value.as_str() {
                    Some(s) if s.trim().is_empty() => cfg.clock_format = None,
                    Some(s) if valid_strftime(s) => cfg.clock_format = Some(s.to_owned()),
                    _ => bad("geçerli bir strftime biçimi değil"),
                },
                "date_format" => set_format(value, &mut cfg.date_format, &mut bad),
                "fingerprint" => set_bool(value, &mut cfg.fingerprint, &mut bad),
                "fingerprint_tries" => match value.as_integer() {
                    Some(n @ 1..=10) => cfg.fingerprint_tries = n as u32,
                    _ => bad("1..=10 tamsayı olmalı"),
                },
                "show_media" => set_bool(value, &mut cfg.show_media, &mut bad),
                "show_notifications" => set_bool(value, &mut cfg.show_notifications, &mut bad),
                _ => warnings.push(format!("lock.toml: bilinmeyen anahtar `{key}` yok sayıldı")),
            }
        }
        (cfg, warnings)
    }
}

fn set_bool(value: &toml::Value, slot: &mut bool, bad: &mut impl FnMut(&str)) {
    match value.as_bool() {
        Some(b) => *slot = b,
        None => bad("true/false olmalı"),
    }
}

fn set_format(value: &toml::Value, slot: &mut String, bad: &mut impl FnMut(&str)) {
    match value.as_str() {
        Some(s) if valid_strftime(s) => *slot = s.to_owned(),
        _ => bad("geçerli bir strftime biçimi değil"),
    }
}

/// Boş değil, ≤ 64 bayt ve chrono'nun panik etmeden biçimlendirebileceği bir dizgi.
pub fn valid_strftime(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && StrftimeItems::new(s).all(|i| !matches!(i, Item::Error))
}

/// Kilit saatinin metni: kalıp yazılmadıysa locale'in saat:dakikası (`Locale::format_hm`,
/// `[locale] hour_cycle`'a uyar), yazıldıysa kalıp. Biçimlenemezse boş dizgi.
pub fn clock_text<Tz: chrono::TimeZone>(
    locale: &nuxut::locale::Locale,
    pattern: Option<&str>,
    now: &chrono::DateTime<Tz>,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Timelike;
    match pattern {
        Some(p) => locale.format_datetime(now, p).unwrap_or_default(),
        None => locale.format_hm(now.hour(), now.minute()).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_default() {
        let (c, w) = LockConfig::parse("");
        assert_eq!(c, LockConfig::default());
        assert!(w.is_empty());
        assert!(
            !c.show_media && !c.show_notifications,
            "gizlilik varsayılanı"
        );
    }

    #[test]
    fn valid_values() {
        let (c, w) = LockConfig::parse(
            "wallpaper = \"/tmp/w.png\"\nblur = false\ndim = 0.3\nclock_format = \"%H:%M:%S\"\n\
             fingerprint = false\nfingerprint_tries = 5\nshow_media = true\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(
            c.wallpaper,
            Some(WallpaperSource::File("/tmp/w.png".into()))
        );
        assert!(!c.blur && !c.fingerprint && c.show_media);
        assert_eq!((c.dim, c.fingerprint_tries), (0.3, 5));
        assert_eq!(c.clock_format.as_deref(), Some("%H:%M:%S"));
    }

    #[test]
    fn builtin_wallpaper() {
        let (c, w) = LockConfig::parse("wallpaper = \"nuxut:wingtip/dark\"\n");
        assert!(w.is_empty(), "{w:?}");
        let sel = nuxut_wallpapers::Selection { builtin: nuxut_wallpapers::find("wingtip").unwrap(), variant: Some(nuxut_wallpapers::Variant::Dark) };
        assert_eq!(c.wallpaper, Some(WallpaperSource::Builtin(sel)));
        let (c, w) = LockConfig::parse("wallpaper = \"nuxut:yok\"\n");
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(c.wallpaper, Some(WallpaperSource::Builtin(nuxut_wallpapers::Selection { builtin: nuxut_wallpapers::default_builtin(), variant: None })));
    }

    #[test]
    fn bad_values_fall_back_per_field() {
        let (c, w) = LockConfig::parse(
            "dim = 2.0\nblur = \"evet\"\nclock_format = \"%Q\"\nfingerprint_tries = 0\nbilinmeyen = 1\nshow_notifications = true\n",
        );
        assert_eq!(w.len(), 5, "{w:?}");
        assert_eq!(c.dim, 0.55);
        assert!(c.blur);
        assert_eq!(c.clock_format, None, "geçersiz kalıp → otomatik");
        assert_eq!(c.fingerprint_tries, 3);
        assert!(c.show_notifications, "geçerli alan yine uygulanır");
    }

    #[test]
    fn garbage_and_huge_files_give_defaults() {
        let (c, w) = LockConfig::parse("this is = = not toml [[");
        assert_eq!(c, LockConfig::default());
        assert_eq!(w.len(), 1);
        let (c, w) = LockConfig::parse(&"#".repeat(MAX_CONFIG_BYTES + 1));
        assert_eq!(c, LockConfig::default());
        assert_eq!(w.len(), 1);
    }

    /// t119: yazılmamış kalıp `hour_cycle`'a uyar; yazılmış kalıp kazanır; "" = otomatik.
    #[test]
    fn clock_follows_hour_cycle_unless_written() {
        use chrono::TimeZone;
        use nuxut::locale::Locale;
        let t = chrono::FixedOffset::east_opt(3 * 3600).unwrap().with_ymd_and_hms(2026, 10, 13, 13, 5, 0).unwrap();
        let tr = Locale::parse("tr_TR").unwrap();
        let en = Locale::parse("en_US").unwrap();
        assert_eq!(clock_text(&tr, None, &t), "13:05");
        assert_eq!(clock_text(&en, None, &t), "1:05 PM");
        assert_eq!(clock_text(&en.clone().with_12_hour(Some(false)), None, &t), "13:05");
        assert!(clock_text(&tr.clone().with_12_hour(Some(true)), None, &t).starts_with("1:05"));
        assert_eq!(clock_text(&en.with_12_hour(Some(true)), Some("%H:%M"), &t), "13:05");
        let (c, w) = LockConfig::parse("clock_format = \"\"\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.clock_format, None);
    }

    #[test]
    fn strftime_validation() {
        assert!(valid_strftime("%A, %e %B"));
        assert!(!valid_strftime(""));
        assert!(!valid_strftime("%"));
        assert!(!valid_strftime(&"%H".repeat(40)));
    }
}
