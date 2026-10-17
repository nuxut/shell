//! `[night_light]`: gece ışığı ayarları, şeması ve doğrulaması.
//!
//! Kullanıcı kararı: hiçbir eylem varsayılan açık değil (`enabled = false`). Açınca makul
//! değerlerle başlar: 4000 K, gün batımından gün doğumuna, 30 dk geçiş.

use std::time::Duration;

use nuxut::i18n::t;
use serde::Deserialize;

use super::model::{Schedule, parse_hhmm};
use crate::schema::SettingSchema;

/// Gece sıcaklığı sınırı (K).
pub const TEMPERATURE: std::ops::RangeInclusive<u32> = 1500..=6000;
/// Geçiş süresi sınırı (dk).
pub const TRANSITION_MINUTES: std::ops::RangeInclusive<u32> = 0..=120;

/// Zamanlama türü (`schedule`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleKind {
    /// Açıkken sürekli.
    Always,
    /// `from`–`to` yerel saat aralığı.
    Manual,
    /// Gün batımından gün doğumuna.
    #[default]
    Sun,
}

impl ScheduleKind {
    pub const ALL: [ScheduleKind; 3] = [ScheduleKind::Always, ScheduleKind::Manual, ScheduleKind::Sun];

    /// `shell.toml`'daki adı (serde adıyla aynı; şema testi doğrular).
    pub fn name(self) -> &'static str {
        match self {
            ScheduleKind::Always => "always",
            ScheduleKind::Manual => "manual",
            ScheduleKind::Sun => "sun",
        }
    }

    /// Ayarlarda seçeneğin görünen adı.
    pub fn label(self) -> String {
        match self {
            ScheduleKind::Always => t!("shell-schema-night-light-schedule-always"),
            ScheduleKind::Manual => t!("shell-schema-night-light-schedule-manual"),
            ScheduleKind::Sun => t!("shell-schema-night-light-schedule-sun"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct NightLightConfig {
    /// Ana anahtar: kapalıyken zamanlama çalışmaz (hızlı ayardan elle açılabilir).
    pub enabled: bool,
    /// Gece sıcaklığı, Kelvin ([`TEMPERATURE`]).
    pub temperature: u32,
    pub schedule: ScheduleKind,
    /// `manual` aralığı, `HH:MM` yerel saat; `to <= from` gece yarısını aşar.
    pub from: String,
    pub to: String,
    /// `sun` için konum; ikisi de yoksa saat diliminden.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Sınırın etrafındaki geçiş süresi, dakika ([`TRANSITION_MINUTES`]).
    pub transition: u32,
}

impl Default for NightLightConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            temperature: 4000,
            schedule: ScheduleKind::Sun,
            from: "20:00".into(),
            to: "07:00".into(),
            latitude: None,
            longitude: None,
            transition: 30,
        }
    }
}

/// Konum çözülemezse güneş zamanlamasının yerine geçen saatler.
const FALLBACK_FROM: &str = "20:00";
const FALLBACK_TO: &str = "07:00";

impl NightLightConfig {
    /// Şema; başlık ve açıklamalar katalogdan (`shell-config-night-light-*`). Konum anahtarları
    /// isteğe bağlıdır (yoksa saat dilimi); şema biçiminde "boş olabilir" sayı yok, yazılmaz.
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(
            SettingSchema::bool("night_light.enabled", t!("shell-config-night-light-enabled-label"), d.enabled)
                .describe(t!("shell-config-night-light-enabled-description")),
        );
        out.push(
            SettingSchema::int(
                "night_light.temperature",
                t!("shell-config-night-light-temperature-label"),
                d.temperature.into(),
                (*TEMPERATURE.start()).into(),
                (*TEMPERATURE.end()).into(),
            )
            .step(100.0)
            .unit("K"),
        );
        out.push(
            SettingSchema::choice(
                "night_light.schedule",
                t!("shell-config-night-light-schedule-label"),
                d.schedule.name(),
                ScheduleKind::ALL.map(ScheduleKind::name).to_vec(),
            )
            .labels(ScheduleKind::ALL.map(ScheduleKind::label).to_vec()),
        );
        out.push(
            SettingSchema::text("night_light.from", t!("shell-config-night-light-from-label"), d.from)
                .describe(t!("shell-config-night-light-from-description")),
        );
        out.push(SettingSchema::text("night_light.to", t!("shell-config-night-light-to-label"), d.to));
        out.push(
            SettingSchema::int(
                "night_light.transition",
                t!("shell-config-night-light-transition-label"),
                d.transition.into(),
                (*TRANSITION_MINUTES.start()).into(),
                (*TRANSITION_MINUTES.end()).into(),
            )
            .step(5.0)
            .unit("min"),
        );
    }

    /// Değerleri sınırlara çeker; her düzeltme için uyarı. İdempotent.
    pub fn validate(&mut self, w: &mut Vec<String>) {
        let d = Self::default();
        if !TEMPERATURE.contains(&self.temperature) {
            let fixed = self.temperature.clamp(*TEMPERATURE.start(), *TEMPERATURE.end());
            w.push(t!(
                "shell-config-night-light-out-of-range",
                key = "night_light.temperature",
                value = self.temperature,
                fixed = fixed,
                range = "1500..=6000"
            ));
            self.temperature = fixed;
        }
        if !TRANSITION_MINUTES.contains(&self.transition) {
            let fixed = self.transition.min(*TRANSITION_MINUTES.end());
            w.push(t!(
                "shell-config-night-light-out-of-range",
                key = "night_light.transition",
                value = self.transition,
                fixed = fixed,
                range = "0..=120"
            ));
            self.transition = fixed;
        }
        for (value, key, default) in [(&mut self.from, "night_light.from", d.from), (&mut self.to, "night_light.to", d.to)] {
            if parse_hhmm(value).is_none() {
                w.push(t!("shell-config-night-light-bad-time", key = key, value = crate::config::sanitize(value), fixed = default.clone()));
                *value = default;
            }
        }
        match (self.latitude, self.longitude) {
            (Some(lat), Some(lon)) if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) => {}
            (None, None) => {}
            _ => {
                w.push(t!("shell-config-night-light-bad-location"));
                self.latitude = None;
                self.longitude = None;
            }
        }
    }

    /// Durum makinesinin ayarları. `zone_location`: saat diliminden konum (yalnızca `sun` ve
    /// elle konum yokken sorulur). Konum yoksa `sun` → `manual 20:00–07:00` ve uyarı.
    pub fn settings(&self, zone_location: impl FnOnce() -> Option<(f64, f64)>) -> (Settings, Option<String>) {
        let time = |s: &str, fallback: &str| parse_hhmm(s).or_else(|| parse_hhmm(fallback)).unwrap_or_default();
        let manual = |from: &str, to: &str| Schedule::Manual { from: time(from, FALLBACK_FROM), to: time(to, FALLBACK_TO) };
        let mut warning = None;
        let mut location = None;
        let schedule = match self.schedule {
            ScheduleKind::Always => Schedule::Always,
            ScheduleKind::Manual => manual(&self.from, &self.to),
            ScheduleKind::Sun => {
                let found = match self.latitude.zip(self.longitude) {
                    Some(c) => Some((LocationSource::Manual, c)),
                    None => zone_location().map(|c| (LocationSource::Timezone, c)),
                };
                location = Some(match found {
                    Some((source, c)) => Location { source, coords: Some(c) },
                    None => Location { source: LocationSource::None, coords: None },
                });
                match found {
                    Some((_, (lat, lon))) => Schedule::Sun { lat, lon },
                    None => {
                        warning =
                            Some(t!("shell-config-night-light-no-location", from = FALLBACK_FROM, to = FALLBACK_TO));
                        manual(FALLBACK_FROM, FALLBACK_TO)
                    }
                }
            }
        };
        let settings = Settings {
            enabled: self.enabled,
            temperature: self.temperature.clamp(*TEMPERATURE.start(), *TEMPERATURE.end()),
            schedule,
            transition: Duration::from_secs(u64::from(self.transition.min(*TRANSITION_MINUTES.end())) * 60),
            location,
        };
        (settings, warning)
    }
}

/// Güneş zamanlamasının konumu nereden geldi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationSource {
    /// `shell.toml` `latitude` + `longitude` (ikisi birlikte).
    Manual,
    /// Saat diliminden yaklaşık (`location::from_system`).
    Timezone,
    /// Bulunamadı: zamanlama 20:00–07:00'ye düştü.
    None,
}

/// Çözülmüş konum (Ayarlar gösterimi ve `msg night-light status`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    pub source: LocationSource,
    /// (enlem, boylam); `None` yalnızca kaynak `None` iken.
    pub coords: Option<(f64, f64)>,
}

/// Çözülmüş ayarlar (durum makinesinin girdisi).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub temperature: u32,
    pub schedule: Schedule,
    pub transition: Duration,
    /// Yalnızca yapılandırılmış zamanlama `sun` iken.
    pub location: Option<Location>,
}

impl Default for Settings {
    fn default() -> Self {
        NightLightConfig::default().settings(|| None).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_with_sane_values() {
        let d = NightLightConfig::default();
        assert!(!d.enabled, "kullanıcı kararı: varsayılan kapalı");
        let (s, w) = d.settings(|| Some((41.0, 29.0)));
        assert!(w.is_none());
        assert_eq!((s.temperature, s.transition), (4000, Duration::from_secs(1800)));
        assert_eq!(s.schedule, Schedule::Sun { lat: 41.0, lon: 29.0 });
    }

    #[test]
    fn sun_without_location_falls_back_to_hours() {
        crate::i18n::use_turkish_for_tests();
        let (s, w) = NightLightConfig::default().settings(|| None);
        assert_eq!(s.schedule, Schedule::Manual { from: parse_hhmm("20:00").unwrap(), to: parse_hhmm("07:00").unwrap() });
        assert!(w.is_some());
        // Elle konum saat dilimine sorulmadan kullanılır.
        let c = NightLightConfig { latitude: Some(60.0), longitude: Some(10.0), ..NightLightConfig::default() };
        let (s, _) = c.settings(|| panic!("sorulmamalı"));
        assert_eq!(s.schedule, Schedule::Sun { lat: 60.0, lon: 10.0 });
        assert_eq!(s.location, Some(Location { source: LocationSource::Manual, coords: Some((60.0, 10.0)) }));
        // Kaynaklar: saat dilimi, bulunamadı; sun dışında konum yok.
        let (s, _) = NightLightConfig::default().settings(|| Some((41.0, 29.0)));
        assert_eq!(s.location, Some(Location { source: LocationSource::Timezone, coords: Some((41.0, 29.0)) }));
        let (s, _) = NightLightConfig::default().settings(|| None);
        assert_eq!(s.location, Some(Location { source: LocationSource::None, coords: None }));
        let c = NightLightConfig { schedule: ScheduleKind::Always, ..NightLightConfig::default() };
        assert_eq!(c.settings(|| panic!("sorulmamalı")).0.location, None);
    }

    #[test]
    fn validation_clamps_and_is_idempotent() {
        crate::i18n::use_turkish_for_tests();
        let mut c = NightLightConfig {
            temperature: 900,
            transition: 500,
            from: "25:00".into(),
            to: "6:30".into(),
            latitude: Some(95.0),
            longitude: Some(10.0),
            ..NightLightConfig::default()
        };
        let mut w = Vec::new();
        c.validate(&mut w);
        assert_eq!((c.temperature, c.transition), (1500, 120));
        assert_eq!((c.from.as_str(), c.to.as_str()), ("20:00", "6:30"));
        assert_eq!((c.latitude, c.longitude), (None, None));
        assert_eq!(w.len(), 4, "{w:?}");
        let mut again = Vec::new();
        c.validate(&mut again);
        assert!(again.is_empty(), "idempotent: {again:?}");
        // Yalnızca biri yazılmış konum da geçersiz.
        let mut c = NightLightConfig { latitude: Some(41.0), ..NightLightConfig::default() };
        let mut w = Vec::new();
        c.validate(&mut w);
        assert_eq!((w.len(), c.latitude), (1, None));
    }

    #[test]
    fn parses_partial_table() {
        let c: NightLightConfig = toml::from_str("enabled = true\nschedule = \"manual\"\nfrom = \"21:30\"").unwrap();
        assert!(c.enabled && c.schedule == ScheduleKind::Manual && c.from == "21:30" && c.to == "07:00");
        assert!(toml::from_str::<NightLightConfig>("schedule = \"never\"").is_err());
    }
}
