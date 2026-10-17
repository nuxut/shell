//! Saat: strftime biçimi doğrulama, yerelleştirilmiş biçimlendirme ve hizalı tick hesabı.
//!
//! Saf fonksiyonlar; thread yok. Runtime, [`ClockFormat::until_next_tick`] kadar sonrasına
//! tek atımlık timer kurar, uyanınca yeniden biçimlendirir ve timer'ı yeniden kurar. Böylece
//! saat yalnızca gösterilen birim değiştiğinde (dakika ya da saniye sınırında) iş yapar.
//! Biçimlendirme her zaman ayrıştırılmış öğelerle yapılır; geçersiz biçim yüklemede
//! reddedildiği için chrono'nun `Display` hatası (panik eden `to_string`) yoluna girilmez.
//!
//! Ay/gün adları (`%B`, `%a`…) ve `%x`/`%X`/`%c`/`%p` zaman locale'inden ([`time_locale`]).
//!
//! **12/24 saat** (t119, `docs/design/clock-locale.md`): kullanıcı biçim yazmadıysa
//! ([`ClockFormat::auto`]) saat `framework.toml [locale] hour_cycle`'a uyar (`auto` = locale
//! verisi, `h23`, `h12`); yazdıysa ([`ClockFormat::parse_in`]) strftime kalıbı kazanır.

use std::fmt::Write;
use std::time::Duration;

use chrono::format::{Fixed, Item, Numeric, StrftimeItems};
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike};
use nuxut::locale::Locale;

/// Timer'ın sınırdan hemen önce uyanıp eski değeri göstermemesi için eklenen pay.
pub const TICK_SLACK: Duration = Duration::from_millis(5);

/// Etkin saat/tarih locale'i: framework'ün tek kaynağı (`nuxut::i18n::time_locale`):
/// `framework.toml [locale] formats` (`auto` → `LC_ALL` > `LC_TIME` > `LANG`, tanınmazsa
/// `en_US`) + `hour_cycle`. Runtime yapılandırmadan önce (ve testlerde) ortamdan. Ucuz değil
/// (her çağrı ayarı kopyalar, locale'i çözer): saatte bir kez değil, kurulumda ya da
/// `locale_changed`'de alınıp saklanır.
pub fn time_locale() -> Locale {
    nuxut::i18n::time_locale()
}

/// Biçimin gösterdiği en küçük zaman birimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Second,
    Minute,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("geçersiz saat biçimi (strftime)")]
pub struct ClockFormatError;

/// Doğrulanmış ve ayrıştırılmış saat biçimi: kullanıcının strftime kalıbı (locale'e bağlı:
/// `%X` gibi kalıplar locale'in kalıbına açılmış olarak tutulur) ya da kalıp yazılmadıysa
/// locale'in saat:dakikası ([`ClockFormat::auto`], `hour_cycle`'a uyar).
#[derive(Debug, Clone)]
pub struct ClockFormat {
    kind: FormatKind,
    granularity: Granularity,
}

#[derive(Debug, Clone)]
enum FormatKind {
    Pattern { source: String, items: Vec<Item<'static>>, locale: chrono::Locale },
    /// Locale'in saat:dakikası; `vertical`: satırlara bölünmüş (saat / dakika / gün dilimi).
    Auto { locale: Locale, vertical: bool },
}

impl ClockFormat {
    /// Biçimi [`time_locale`] ile ayrıştırır. Bilinmeyen ya da yarım belirteç (`%Q`, sondaki
    /// `%`) hatadır.
    pub fn parse(format: &str) -> Result<Self, ClockFormatError> {
        Self::parse_in(format, &time_locale())
    }

    /// Biçimi verilen locale'le ayrıştırır.
    pub fn parse_in(format: &str, locale: &Locale) -> Result<Self, ClockFormatError> {
        let locale = locale.chrono();
        let items = StrftimeItems::new_with_locale(format, locale).parse_to_owned().map_err(|_| ClockFormatError)?;
        let granularity =
            if items.iter().any(shows_seconds) { Granularity::Second } else { Granularity::Minute };
        Ok(Self { kind: FormatKind::Pattern { source: format.to_owned(), items, locale }, granularity })
    }

    /// Kullanıcı biçim yazmadı: locale'in saat:dakikası, 12/24 saat locale'in
    /// [`Locale::uses_12_hour`]'ından (`[locale] hour_cycle` geçersiz kılması dahil). Yatay:
    /// [`Locale::format_hm`] ("13:05", "1:05 PM", "1:05 ÖS"). Dikey: 24 saatte "13\n05", 12
    /// saatte "1\n05\nPM" (gün dilimi adı locale'den; boşsa AM/PM).
    pub fn auto(locale: &Locale, vertical: bool) -> Self {
        Self { kind: FormatKind::Auto { locale: locale.clone(), vertical }, granularity: Granularity::Minute }
    }

    /// Bar saatinin biçimi: kullanıcı kalıp yazdıysa (`Some`) o — geçersizse (doğrulama
    /// yakalamış olmalı) otomatik —, yazmadıysa otomatik.
    pub fn resolve(explicit: Option<&str>, vertical: bool, locale: &Locale) -> Self {
        explicit.and_then(|p| Self::parse_in(p, locale).ok()).unwrap_or_else(|| Self::auto(locale, vertical))
    }

    /// Kullanıcının kalıbı; otomatik biçimde `None`.
    pub fn pattern(&self) -> Option<&str> {
        match &self.kind {
            FormatKind::Pattern { source, .. } => Some(source),
            FormatKind::Auto { .. } => None,
        }
    }

    pub fn granularity(&self) -> Granularity {
        self.granularity
    }

    pub fn format<Tz: TimeZone>(&self, time: &DateTime<Tz>) -> String
    where
        Tz::Offset: std::fmt::Display,
    {
        match &self.kind {
            FormatKind::Pattern { source, items, locale } => {
                let mut out = String::new();
                if write!(out, "{}", time.format_localized_with_items(items.iter(), *locale)).is_err() {
                    // Ayrıştırılmış biçimle olmaması gerekir; yine de panik yerine boş metin.
                    tracing::warn!(format = %source, "saat biçimlendirilemedi");
                    out.clear();
                }
                out
            }
            FormatKind::Auto { locale, vertical } => auto_text(locale, *vertical, time.hour(), time.minute()),
        }
    }

    pub fn format_now(&self) -> String {
        self.format(&Local::now())
    }

    /// Gösterilen birimin bir sonraki sınırına kalan süre (+ [`TICK_SLACK`]).
    pub fn until_next_tick<T: Timelike>(&self, now: &T) -> Duration {
        until_next_tick(self.granularity, now.second(), now.nanosecond())
    }
}

/// Otomatik saat metni (bkz. [`ClockFormat::auto`]).
fn auto_text(locale: &Locale, vertical: bool, hour: u32, minute: u32) -> String {
    let twelve = locale.uses_12_hour();
    if !vertical {
        // `format_hm` yalnızca geçersiz saatte hata verir; yine de panik yerine sayısal.
        return locale.format_hm(hour, minute).unwrap_or_else(|_| format!("{hour:02}:{minute:02}"));
    }
    if twelve {
        let [am, pm] = locale.day_periods();
        let h = (hour + 11) % 12 + 1;
        format!("{h}\n{minute:02}\n{}", if hour < 12 { am } else { pm })
    } else {
        format!("{hour:02}\n{minute:02}")
    }
}

/// Saatin tek satırlık saat:dakikası (takvim etkinlik satırları, popout'un büyük saati):
/// [`Locale::format_hm`], `hour_cycle`'a uyar.
pub fn format_hm(locale: &Locale, hour: u32, minute: u32) -> String {
    auto_text(locale, false, hour, minute)
}

/// Uzun tarih, ay/gün adları locale'den: "4 Ekim 2026, Pazar" (tr_TR); İngilizce'de
/// "Sunday, October 4, 2026". glibc'de uzun tarih kalıbı yok (`%x` kısa, sayısal); sıra gün-ay-yıl,
/// yalnızca İngilizce ay-gün.
pub fn long_date<D: Datelike>(date: &D, locale: &Locale) -> String {
    let pattern = if locale.language() == "en" { "%A, %B %-d, %Y" } else { "%-d %B %Y, %A" };
    NaiveDate::from_ymd_opt(date.year(), date.month(), date.day())
        .and_then(|d| locale.format_date(d, pattern).ok())
        .unwrap_or_default()
}

/// Tam tarih, gün adı önde: "Pazartesi, 13 Ekim 2026" (tr), "Monday, October 13, 2026"
/// (en_US), "Monday, 13 October 2026" (en_GB ve diğer İngilizce bölgeler), "Montag, 13.
/// Oktober 2026" (de). Bar saatinin ipucu balonu. glibc/nuxut-locale'de CLDR "full" tarih kalıbı
/// yok; sıra burada dile (ve İngilizcede bölgeye) göre seçilir: ay-gün yalnızca ABD
/// İngilizcesinde (bölgesiz `en` de ABD sayılır, `time_locale` varsayılanı `en_US`).
pub fn full_date<D: Datelike>(date: &D, locale: &Locale) -> String {
    let pattern = match (locale.language(), locale.region()) {
        ("en", "" | "US" | "PH" | "UM" | "AS" | "GU" | "MP" | "PR" | "VI") => "%A, %B %-d, %Y",
        ("de", _) => "%A, %-d. %B %Y",
        _ => "%A, %-d %B %Y",
    };
    NaiveDate::from_ymd_opt(date.year(), date.month(), date.day())
        .and_then(|d| locale.format_date(d, pattern).ok())
        .unwrap_or_default()
}

/// `second` 0..=60 (artık saniye), `nanosecond` artık saniyede 1e9'u aşabilir.
fn until_next_tick(granularity: Granularity, second: u32, nanosecond: u32) -> Duration {
    let into_second = Duration::from_nanos(u64::from(nanosecond.min(999_999_999)));
    let remaining = match granularity {
        Granularity::Second => Duration::from_secs(1) - into_second,
        Granularity::Minute => {
            let elapsed = Duration::from_secs(u64::from(second.min(59))) + into_second;
            Duration::from_secs(60) - elapsed
        }
    };
    remaining + TICK_SLACK
}

fn shows_seconds(item: &Item<'_>) -> bool {
    match item {
        Item::Numeric(n, _) => matches!(n, Numeric::Second | Numeric::Nanosecond | Numeric::Timestamp),
        // `Internal` öğeler `%3f` gibi noktasız kesir biçimleridir; temkinli olarak saniye say.
        Item::Fixed(f) => matches!(
            f,
            Fixed::Nanosecond | Fixed::Nanosecond3 | Fixed::Nanosecond6 | Fixed::Nanosecond9 | Fixed::Internal(_)
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, NaiveTime};

    use super::*;

    fn at(h: u32, m: u32, s: u32, nano: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(3 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 4, h, m, s)
            .unwrap()
            .with_nanosecond(nano)
            .unwrap()
    }

    fn loc(name: &str) -> Locale {
        Locale::parse(name).unwrap()
    }

    #[test]
    fn long_date_follows_locale() {
        let tr = loc("tr_TR.UTF-8");
        assert_eq!(long_date(&at(9, 0, 0, 0), &tr), "4 Ekim 2026, Pazar");
        let d = chrono::NaiveDate::from_ymd_opt(2027, 2, 1).unwrap();
        assert_eq!(long_date(&d, &tr), "1 Şubat 2027, Pazartesi");
        assert_eq!(long_date(&d, &Locale::en_us()), "Monday, February 1, 2027");
        assert_eq!(long_date(&d, &loc("de_DE.UTF-8")), "1 Februar 2027, Montag");
    }

    #[test]
    fn full_date_starts_with_weekday() {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        assert_eq!(full_date(&d, &loc("tr_TR.UTF-8")), "Pazartesi, 12 Ekim 2026");
        assert_eq!(full_date(&d, &Locale::en_us()), "Monday, October 12, 2026");
        assert_eq!(full_date(&d, &loc("en_GB.UTF-8")), "Monday, 12 October 2026");
        assert_eq!(full_date(&d, &loc("de_DE.UTF-8")), "Montag, 12. Oktober 2026");
        let d = chrono::NaiveDate::from_ymd_opt(2026, 10, 13).unwrap();
        assert_eq!(full_date(&d, &loc("tr_TR.UTF-8")), "Salı, 13 Ekim 2026");
        assert_eq!(full_date(&at(9, 0, 0, 0), &loc("tr_TR.UTF-8")), "Pazar, 4 Ekim 2026");
    }

    #[test]
    fn clock_names_follow_locale() {
        let t = at(13, 5, 0, 0);
        let f = |p: &str, l: &Locale| ClockFormat::parse_in(p, l).unwrap().format(&t);
        assert_eq!(f("%a %d %b %H:%M", &loc("tr_TR.UTF-8")), "Paz 04 Eki 13:05");
        assert_eq!(f("%a %d %b %H:%M", &Locale::en_us()), "Sun 04 Oct 13:05");
        // `%X` locale kalıbına açılır; en_US 12 saatlik ve saniyeli.
        assert_eq!(f("%X", &Locale::en_us()), "01:05:00 PM");
        assert_eq!(ClockFormat::parse_in("%X", &Locale::en_us()).unwrap().granularity(), Granularity::Second);
        assert_eq!(f("%X", &loc("tr_TR.UTF-8")), "13:05:00");
    }

    /// t119: biçim yazılmadıysa `hour_cycle`'a uyulur; yazıldıysa kalıp kazanır.
    #[test]
    fn auto_format_follows_hour_cycle() {
        let t = at(13, 5, 0, 0);
        let morning = at(0, 7, 0, 0);
        let cyc = |name: &str, o: Option<bool>| loc(name).with_12_hour(o);
        let h = |l: &Locale| ClockFormat::auto(l, false).format(&t);
        let v = |l: &Locale| ClockFormat::auto(l, true).format(&t);
        // auto: dilden (tr 24 s, en_US 12 s).
        assert_eq!(h(&cyc("tr_TR", None)), "13:05");
        assert_eq!(v(&cyc("tr_TR", None)), "13\n05");
        assert_eq!(h(&cyc("en_US", None)), "1:05 PM");
        assert_eq!(v(&cyc("en_US", None)), "1\n05\nPM");
        // h23 her iki dilde.
        assert_eq!(h(&cyc("tr_TR", Some(false))), "13:05");
        assert_eq!(h(&cyc("en_US", Some(false))), "13:05");
        assert_eq!(v(&cyc("en_US", Some(false))), "13\n05");
        // h12 her iki dilde (tr gün dilimi glibc'den).
        assert_eq!(h(&cyc("en_US", Some(true))), "1:05 PM");
        assert_eq!(ClockFormat::auto(&cyc("en_US", Some(true)), false).format(&morning), "12:07 AM");
        assert_eq!(ClockFormat::auto(&cyc("en_US", Some(true)), true).format(&morning), "12\n07\nAM");
        let tr12 = cyc("tr_TR", Some(true));
        let [_, pm] = tr12.day_periods();
        assert_eq!(h(&tr12), format!("1:05 {pm}"));
        assert_eq!(v(&tr12), format!("1\n05\n{pm}"));
        // Gün dilimi adı olmayan locale: AM/PM.
        assert_eq!(v(&cyc("de_DE", Some(true))), "1\n05\nPM");
        // Otomatik biçim dakikalık; kalıbı yok.
        assert_eq!(ClockFormat::auto(&tr12, true).granularity(), Granularity::Minute);
        assert_eq!(ClockFormat::auto(&tr12, false).pattern(), None);
        // Açık kalıp kazanır (h12 zorlansa da %H 24 saattir); geçersiz kalıp → otomatik.
        let r = |p: Option<&str>, l: &Locale| ClockFormat::resolve(p, false, l).format(&t);
        assert_eq!(r(Some("%H:%M"), &cyc("en_US", Some(true))), "13:05");
        assert_eq!(r(Some("%I:%M %p"), &cyc("tr_TR", Some(false))), format!("01:05 {pm}"));
        assert_eq!(r(None, &cyc("en_US", Some(true))), "1:05 PM");
        assert_eq!(r(Some("%Q"), &cyc("tr_TR", None)), "13:05");
        assert_eq!(ClockFormat::resolve(Some("%T"), false, &tr12).pattern(), Some("%T"));
        assert_eq!(format_hm(&cyc("en_US", None), 9, 30), "9:30 AM");
        assert_eq!(format_hm(&cyc("tr_TR", None), 9, 30), "09:30");
    }

    #[test]
    fn rejects_invalid_formats_without_panicking() {
        for bad in ["%Q", "%H:%", "%", "%E", "%:"] {
            assert_eq!(ClockFormat::parse(bad).unwrap_err(), ClockFormatError, "{bad}");
        }
    }

    #[test]
    fn granularity_follows_smallest_unit() {
        let g = |f: &str| ClockFormat::parse(f).unwrap().granularity();
        assert_eq!(g("%H:%M"), Granularity::Minute);
        assert_eq!(g("%a %d %b  %I:%M %p"), Granularity::Minute);
        assert_eq!(g("%H:%M:%S"), Granularity::Second);
        assert_eq!(g("%T"), Granularity::Second, "bileşik belirteç açılır");
        assert_eq!(g("%R"), Granularity::Minute);
        assert_eq!(g("%s"), Granularity::Second);
        assert_eq!(g("%H:%M:%S%.3f"), Granularity::Second);
        assert_eq!(g("%3f"), Granularity::Second);
        assert_eq!(g("tarih: %F"), Granularity::Minute);
    }

    #[test]
    fn formats_time() {
        let t = at(9, 5, 7, 0);
        assert_eq!(ClockFormat::auto(&loc("tr_TR"), false).format(&t), "09:05");
        assert_eq!(ClockFormat::parse("%H:%M:%S").unwrap().format(&t), "09:05:07");
        assert_eq!(ClockFormat::parse("%d.%m.%Y %z").unwrap().format(&t), "04.10.2026 +0300");
        assert_eq!(ClockFormat::parse("100%% %k").unwrap().format(&t), "100%  9");
    }

    #[test]
    fn next_tick_aligns_to_boundary() {
        let minute = ClockFormat::auto(&loc("tr_TR"), false);
        let second = ClockFormat::parse("%T").unwrap();
        let t = NaiveTime::from_hms_nano_opt(12, 30, 15, 250_000_000).unwrap();
        assert_eq!(minute.until_next_tick(&t), Duration::from_millis(44_750) + TICK_SLACK);
        assert_eq!(second.until_next_tick(&t), Duration::from_millis(750) + TICK_SLACK);
        let edge = NaiveTime::from_hms_opt(12, 30, 0).unwrap();
        assert_eq!(minute.until_next_tick(&edge), Duration::from_secs(60) + TICK_SLACK);
        assert_eq!(second.until_next_tick(&edge), Duration::from_secs(1) + TICK_SLACK);
    }

    #[test]
    fn next_tick_survives_leap_second() {
        // Artık saniye: second = 59, nanosecond >= 1e9.
        let d = until_next_tick(Granularity::Minute, 59, 1_500_000_000);
        assert!(d >= TICK_SLACK && d <= Duration::from_millis(1) + TICK_SLACK, "{d:?}");
        let d = until_next_tick(Granularity::Second, 59, 1_999_999_999);
        assert!(d >= TICK_SLACK && d < Duration::from_millis(1) + TICK_SLACK, "{d:?}");
        assert!(until_next_tick(Granularity::Minute, 60, 0) <= Duration::from_secs(1) + TICK_SLACK);
    }

    #[test]
    fn tick_after_waiting_shows_next_value() {
        let f = ClockFormat::auto(&loc("tr_TR"), false);
        let t = at(23, 59, 42, 123_000_000);
        let next = t + chrono::TimeDelta::from_std(f.until_next_tick(&t)).unwrap();
        assert_eq!(f.format(&next), "00:00");
        assert_ne!(f.format(&t), f.format(&next));
    }
}
