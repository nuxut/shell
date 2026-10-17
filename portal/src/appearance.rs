//! Saf eşleme: framework'ün çözdüğü tema → `org.freedesktop.appearance` değerleri.
//!
//! Kaynak tek: [`nuxut_theme::Theme`] (runtime'ın da kullandığı `ThemeLoader` ile yüklenir). Bu
//! yüzden kip, vurgu ve hareket Nuxut'un o an çizdiğiyle aynıdır; burada renk türetilmez.
//!
//! - `color-scheme` (`u`): `Theme::mode` — `[colors] mode`'un çözülmüş hali (`auto`: palette.toml
//!   kipi → yüzey parlaklığı → koyu; `ColorsConfig::resolve_mode`). Nuxut her zaman bir kipte
//!   çizdiği için "tercih yok" (0) verilmez: koyu → 1, açık → 2.
//! - `accent-color` (`(ddd)`): `Theme::colors.primary`, sRGB 0..1 (8 bit adımına yuvarlanır).
//! - `contrast` (`u`): `Theme::contrast` — `framework.toml [colors] contrast = "high"` → 1, standart
//!   → 0 (bulgu, 2026-10-14). (`palette.toml` `contrast`
//!   Material kontrast düzeyidir, erişilebilirlik isteği değil; eşlenmez.)
//! - `reduced-motion` (`u`): `Theme::motion.enabled` yanlışsa 1 (Ayarlar → Hareket "Kapalı" =
//!   `[motion] preset = "instant"`, ya da `[motion] enabled = false`).
//!
//! Birleşik görünüm kapalıysa (`[desktop] unified = false`, bozuk framework.toml dahil) arka uç
//! saydamdır ([`Appearance::from_theme`] `None`): hiçbir anahtar bilinmez, xdg-desktop-portal
//! değerleri sıradaki arka uçtan (gtk → kullanıcının dconf'u) alır.

use nuxut_theme::{ContrastMode, Theme, ThemeMode};
use zbus::zvariant::{OwnedValue, Value};

/// Desteklenen tek ad alanı.
pub const NAMESPACE: &str = "org.freedesktop.appearance";

pub const COLOR_SCHEME: &str = "color-scheme";
pub const ACCENT_COLOR: &str = "accent-color";
pub const CONTRAST: &str = "contrast";
pub const REDUCED_MOTION: &str = "reduced-motion";

/// Sunulan anahtarlar (sinyal ve `ReadAll` sırası).
pub const KEYS: [&str; 4] = [COLOR_SCHEME, ACCENT_COLOR, CONTRAST, REDUCED_MOTION];

/// `color-scheme` değerleri (portal belirtimi).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ColorScheme {
    NoPreference = 0,
    PreferDark = 1,
    PreferLight = 2,
}

impl From<ThemeMode> for ColorScheme {
    fn from(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::PreferDark,
            ThemeMode::Light => Self::PreferLight,
        }
    }
}

/// Bir anın görünüm değerleri.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Appearance {
    pub color_scheme: ColorScheme,
    /// sRGB, her kanal 0..1.
    pub accent: [f64; 3],
    /// 0 = normal, 1 = yüksek.
    pub contrast: u32,
    /// 0 = tercih yok, 1 = azaltılmış hareket.
    pub reduced_motion: u32,
}

/// Kanal: 0..1'e sınırlanır, 8 bit adımına yuvarlanır (f32 → f64 dönüşüm gürültüsü sinyal
/// üretmesin; tema renkleri zaten 8 bitlik onaltılıktan gelir).
fn channel(v: f32) -> f64 {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    (f64::from(v) * 255.0).round() / 255.0
}

impl Appearance {
    /// Birleşik görünüm kapalıysa `None` (arka uç saydam).
    pub fn from_theme(theme: &Theme) -> Option<Self> {
        if !theme.desktop.unified {
            return None;
        }
        let p = theme.colors.primary;
        Some(Self {
            color_scheme: theme.mode.into(),
            accent: [channel(p.r), channel(p.g), channel(p.b)],
            contrast: u32::from(theme.contrast == ContrastMode::High),
            reduced_motion: u32::from(!theme.motion.enabled),
        })
    }

    /// Anahtarın D-Bus değeri; bilinmeyen anahtar `None`.
    pub fn value(&self, key: &str) -> Option<OwnedValue> {
        let v = match key {
            COLOR_SCHEME => Value::from(self.color_scheme as u32),
            ACCENT_COLOR => Value::from((self.accent[0], self.accent[1], self.accent[2])),
            CONTRAST => Value::from(self.contrast),
            REDUCED_MOTION => Value::from(self.reduced_motion),
            _ => return None,
        };
        // Yalnızca fd içeren değerler sahiplenilemez; burada hiç yok.
        OwnedValue::try_from(v).ok()
    }

    /// Tüm anahtar/değer çiftleri.
    pub fn entries(&self) -> Vec<(&'static str, OwnedValue)> {
        KEYS.iter().filter_map(|k| Some((*k, self.value(k)?))).collect()
    }
}

/// `old` → `new` geçişinde sinyal gerektiren anahtarlar. Saydama geçişte (`new` = `None`) sinyal
/// yok: portal "anahtar kalktı" bildiremez; sıradaki arka ucun değeri yeni okumalarda gelir.
/// Saydamdan etkine geçişte bütün anahtarlar.
pub fn changed_keys(old: Option<&Appearance>, new: Option<&Appearance>) -> Vec<&'static str> {
    let Some(new) = new else { return Vec::new() };
    KEYS.into_iter().filter(|k| old.and_then(|o| o.value(k)) != new.value(k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_theme::{FrameworkConfig, GeneratedPalette, PaletteMeta};

    fn theme(text: &str) -> Theme {
        let (theme, warnings) = Theme::parse(text).unwrap();
        assert!(warnings.is_empty(), "{text}: {warnings:?}");
        theme
    }

    fn of(text: &str) -> Appearance {
        Appearance::from_theme(&theme(text)).unwrap()
    }

    fn u(v: OwnedValue) -> u32 {
        u32::try_from(v).unwrap()
    }

    /// Kip framework'ün çözümüyle aynı: varsayılan auto → koyu; açık yüzey → açık; sabit kip kazanır.
    #[test]
    fn color_scheme_follows_resolved_mode() {
        let cases = [
            ("", ColorScheme::PreferDark),
            ("[colors]\nmode = \"light\"\n", ColorScheme::PreferLight),
            ("[colors]\nmode = \"dark\"\n", ColorScheme::PreferDark),
            ("[colors]\nsurface = \"#f8f9ff\"\n", ColorScheme::PreferLight),
        ];
        for (text, want) in cases {
            let t = theme(text);
            assert_eq!(ColorScheme::from(t.mode), want, "{text}");
            assert_eq!(of(text).color_scheme, want, "{text}");
            assert_eq!(u(of(text).value(COLOR_SCHEME).unwrap()), want as u32);
        }
    }

    /// `auto` + duvar kağıdı paleti: paletin kayıtlı kipi (framework `from_generated`).
    #[test]
    fn auto_mode_uses_wallpaper_palette_mode() {
        let (cfg, _) = nuxut_config::parse_str::<FrameworkConfig>("[colors]\nsource = \"wallpaper\"\n").unwrap();
        let light = GeneratedPalette { meta: PaletteMeta { mode: Some("light".into()), ..Default::default() }, ..Default::default() };
        let (t, _) = Theme::from_generated(&cfg, Some(&light));
        assert_eq!(Appearance::from_theme(&t).unwrap().color_scheme, ColorScheme::PreferLight);
    }

    #[test]
    fn accent_is_theme_primary_in_srgb_unit_range() {
        let a = of("[colors]\nprimary = \"#3366cc\"\n");
        assert_eq!(a.accent, [0x33 as f64 / 255.0, 0x66 as f64 / 255.0, 0xcc as f64 / 255.0]);
        let v = a.value(ACCENT_COLOR).unwrap();
        assert_eq!(v.value_signature().to_string(), "(ddd)");
        let (r, g, b) = <(f64, f64, f64)>::try_from(v).unwrap();
        assert_eq!([r, g, b], a.accent);
        // Varsayılan tema: değer temanın primary'si (kodda renk yok).
        let t = Theme::default();
        let d = Appearance::from_theme(&t).unwrap();
        assert_eq!(d.accent, [channel(t.colors.primary.r), channel(t.colors.primary.g), channel(t.colors.primary.b)]);
        assert!(d.accent.iter().all(|c| (0.0..=1.0).contains(c)));
    }

    #[test]
    fn channel_clamps_and_rejects_nan() {
        assert_eq!(channel(f32::NAN), 0.0);
        assert_eq!(channel(2.0), 1.0);
        assert_eq!(channel(-1.0), 0.0);
    }

    #[test]
    fn reduced_motion_from_off_preset_or_switch() {
        assert_eq!(of("").reduced_motion, 0);
        assert_eq!(of("[motion]\npreset = \"instant\"\n").reduced_motion, 1);
        assert_eq!(of("[motion]\nenabled = false\n").reduced_motion, 1);
        assert_eq!(of("[motion]\npreset = \"lively\"\n").reduced_motion, 0);
        assert_eq!(u(of("[motion]\npreset = \"instant\"\n").value(REDUCED_MOTION).unwrap()), 1);
    }

    /// `contrast`: standart (varsayılan ya da açıkça) → 0, yüksek karşıtlık → 1 (koyu ve açıkta);
    /// değişim sinyal anahtarı üretir.
    #[test]
    fn contrast_follows_theme_contrast_mode() {
        assert_eq!(u(of("").value(CONTRAST).unwrap()), 0);
        assert_eq!(of("[colors]\ncontrast = \"standard\"\n").contrast, 0);
        for mode in ["dark", "light"] {
            let high = of(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"high\"\n"));
            assert_eq!(high.contrast, 1, "{mode}");
            assert_eq!(u(high.value(CONTRAST).unwrap()), 1);
            let std = of(&format!("[colors]\nmode = \"{mode}\"\n"));
            assert!(changed_keys(Some(&std), Some(&high)).contains(&CONTRAST));
        }
    }

    /// Birleşik görünüm kapalı ya da bozuk framework.toml: saydam.
    #[test]
    fn unified_off_is_transparent() {
        assert!(Appearance::from_theme(&theme("[desktop]\nunified = false\n")).is_none());
        assert!(Appearance::from_theme(&Theme::broken_config()).is_none());
        assert!(Appearance::from_theme(&Theme::default()).is_some());
    }

    #[test]
    fn unknown_key_has_no_value() {
        assert!(of("").value("color_scheme").is_none());
        assert_eq!(of("").entries().len(), KEYS.len());
    }

    #[test]
    fn changed_keys_only_differing() {
        let base = of("");
        let instant = of("[motion]\npreset = \"instant\"\n");
        assert_eq!(changed_keys(Some(&base), Some(&base)), Vec::<&str>::new());
        assert_eq!(changed_keys(Some(&base), Some(&instant)), vec![REDUCED_MOTION]);
        assert_eq!(changed_keys(None, Some(&base)), KEYS.to_vec());
        assert!(changed_keys(Some(&base), None).is_empty());
        let accent = of("[colors]\nprimary = \"#3366cc\"\n");
        assert_eq!(changed_keys(Some(&base), Some(&accent)), vec![ACCENT_COLOR]);
    }
}
