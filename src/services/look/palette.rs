//! Dış uygulamalar için iki kipli rol kümesi (ADR-0004 §5.1, §12-9).
//!
//! Türetme framework'te: `nuxut_palette::Palette::generate` tohum renkten iki kipin tam
//! [`Scheme`]'ini (57 rol: Material 3 + Nuxut `success`/`warning`) verir. Tohum, duvar kağıdı
//! kaynağında `palette.toml`'un tohumu, statik temada temanın `primary`'si. Nuxut renk teması
//! seçiliyse (`[colors] theme`) tohum yok: iki kip `Scheme::from_color_theme` ile temanın elle
//! ayarlanmış tonal paletlerinden kurulur (2026-10-16). Ardından temanın
//! çözülmüş 11 token'ı (Nuxut'un kendi çizdiği renkler) çözülmüş kipe yazılır: GTK/Qt, Nuxut ile
//! aynı yüzey/vurgu renklerini görür. Kodda renk yok.
//!
//! Yüksek karşıtlık (framework `docs/design/accessibility.md` §1, t124): türetme
//! `Palette::generate_with(…, theme.contrast)` ile yapılır; `High` iken iki kipin 57 rolü de
//! WCAG AA eşiğine itilir (yüzeyler aynı kalır). Temanın 11 token'ı zaten yüksek karşıtlıklıdır
//! ve üstüne yazılır. Canlı değişim ayrı yol istemez: `[colors] contrast` değişince tema değişir,
//! görünüm turu (`app/look.rs`) yeniden plan kurar.

use nuxut::theme::{ColorThemeId, Palette as ThemePalette, PaletteMeta, ThemeMode};
use nuxut_core::Color;
use nuxut_palette::{ContrastMode, Mode, Palette, Role, Scheme, THEME_ROLES, Variant};

/// Ada göre rol rengi (üretici tablolarındaki snake_case adlar). Bilinmeyen ad (olmamalı; testler
/// her adın çözüldüğünü doğrular) `surface`'e düşer.
pub trait RoleName {
    fn c(&self, role: &str) -> Color;
}

impl RoleName for Scheme {
    fn c(&self, role: &str) -> Color {
        self.by_name(role).unwrap_or(self.surface)
    }
}

/// İki kipin rolleri.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportPalette {
    pub dark: Scheme,
    pub light: Scheme,
}

impl ExportPalette {
    pub fn get(&self, mode: Mode) -> &Scheme {
        match mode {
            Mode::Dark => &self.dark,
            Mode::Light => &self.light,
        }
    }
}

/// Türetmenin tohumu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seed {
    pub source: Color,
    pub variant: Variant,
    pub contrast: f64,
    /// Seçili Nuxut renk teması (`[colors] theme`, statik kaynak): roller tohumdan değil, temanın
    /// elle ayarlanmış tonal paletlerinden (`Scheme::from_color_theme`; 2026-10-16).
    pub theme: Option<ColorThemeId>,
}

impl Seed {
    /// Duvar kağıdı paletinin bilgisi geçerliyse onun tohumu/şeması, değilse temanın vurgusu.
    pub fn new(theme: &ThemePalette, generated: Option<&PaletteMeta>) -> Self {
        let meta = generated.and_then(|m| Some((Color::from_hex(m.source.as_deref()?).ok()?, m))); // renk-verisi: palette.toml tohumu
        match meta {
            Some((source, m)) => Self {
                source,
                variant: m.scheme.as_deref().and_then(|s| s.parse().ok()).unwrap_or_default(),
                contrast: m.contrast.map_or(0.0, f64::from),
                theme: None,
            },
            None => Self { source: theme.primary, variant: Variant::TonalSpot, contrast: 0.0, theme: None },
        }
    }

    /// Statik kaynakta seçili renk teması (`nuxut::Theme::color_theme`). Duvar kağıdı tohumu
    /// geçerliyse etkisiz (framework o durumda `None` verir; yine de tohum önceliklidir).
    pub fn with_color_theme(mut self, id: Option<ColorThemeId>, generated: Option<&PaletteMeta>) -> Self {
        let wallpaper = generated.and_then(|m| Color::from_hex(m.source.as_deref()?).ok()).is_some(); // renk-verisi: palette.toml tohumu
        if !wallpaper {
            self.theme = id;
            if let Some(id) = id {
                self.source = id.theme().seed;
            }
        }
        self
    }
}

fn theme_token(p: &ThemePalette, name: &str) -> Option<Color> {
    Some(match name {
        "surface" => p.surface,
        "surface_container" => p.surface_container,
        "surface_bright" => p.surface_bright,
        "on_surface" => p.on_surface,
        "on_surface_variant" => p.on_surface_variant,
        "primary" => p.primary,
        "on_primary" => p.on_primary,
        "error" => p.error,
        "on_error" => p.on_error,
        "outline" => p.outline,
        "shadow" => p.shadow,
        _ => return None,
    })
}

/// Temanın token'ları `mode` kipine uyar mı? Framework kipi zaten çözer (`Theme::mode`) ve
/// `colors` o kipin paletidir; yalnızca `framework.toml`'da açıkça yazılmış bir yüzey sabit kiple
/// çelişebilir (ör. `mode = "light"` + koyu `surface`). Ölçüt framework'ün
/// [`ThemeMode::of_surface`]'ı (WCAG eşiği).
pub fn tokens_apply(theme: &ThemePalette, mode: Mode) -> bool {
    Mode::from(ThemeMode::of_surface(theme.surface)) == mode
}

/// Temanın token'ları çözülmüş `mode` kipine yazılır. Kullanıcı açıkça temanınkinin tersi bir kip
/// seçtiyse (ör. koyu statik temada `mode = "light"`) koyu token'lar açık kipe yazılmaz: o kip
/// türetilmiş rollerle kalır (`msg look status` uyarır). `contrast`: temanın karşıtlık kipi
/// (`Theme::contrast`).
pub fn build(theme: &ThemePalette, seed: Seed, mode: Mode, contrast: ContrastMode) -> ExportPalette {
    let mut out = match seed.theme {
        // Renk teması: iki kip de temanın tanımından (açık kip de birincilden tohumlanmış
        // kaymayı taşımaz); temanın 11 token'ı framework'te zaten aynı.
        Some(id) => {
            let scheme = |m: Mode| {
                let s = Scheme::from_color_theme(id.theme(), m, 0.0);
                if contrast.is_high() { s.high_contrast() } else { s }
            };
            ExportPalette { dark: scheme(Mode::Dark), light: scheme(Mode::Light) }
        }
        None => {
            let generated = Palette::generate_with(seed.source, seed.variant, Mode::Dark, seed.contrast, contrast);
            ExportPalette { dark: *generated.scheme(Mode::Dark), light: *generated.scheme(Mode::Light) }
        }
    };
    if !tokens_apply(theme, mode) {
        return out;
    }
    let target = match mode {
        Mode::Dark => &mut out.dark,
        Mode::Light => &mut out.light,
    };
    let foreign_surfaces = target.surface != theme.surface;
    for (ours, m3) in THEME_ROLES {
        if let (Some(c), Some(role)) = (theme_token(theme, ours), Role::from_name(m3)) {
            target.set(role, c);
        }
    }
    if foreign_surfaces {
        fit_surface_ramp(target, theme, mode);
    }
    out
}

/// Temanın yüzeyleri tohumdan türetilenlerden farklıysa (`framework.toml [colors]` ile elle
/// yazılmış yüzeyler, ör. Dracula `#282a36`) token'ı olmayan yüzey rolleri de temanın üç
/// yüzeyinden kurulur. Yoksa GTK'nin içerik ve kenar çubuğu (`view_bg`/`sidebar_bg` →
/// `surface_container_low`) tohumun nötr tonunda kalır, temanın rengi yalnız pencere zemininde
/// görünür: Nautilus tema değişse de aynı görünüyordu (2026-10-17, gerçek Nautilus ile ölçüldü).
/// Basamaklar Material 3 sırasıyla: `lowest` < `surface` < `low` < `container` < `high` <
/// `highest` (açık kipte aynı sıra, yön tersine; temanın yüzeyleri o yönü zaten taşır).
fn fit_surface_ramp(s: &mut Scheme, theme: &ThemePalette, mode: Mode) {
    let (surface, container, high) = (theme.surface, theme.surface_container, theme.surface_bright);
    // `t` 0–1 dışında doğrusal uzatma; kanal [0, 1]'e kırpılır.
    let at = |a: Color, b: Color, t: f32| {
        let ch = |x: f32, y: f32| (x + (y - x) * t).clamp(0.0, 1.0);
        Color::rgba(ch(a.r, b.r), ch(a.g, b.g), ch(a.b, b.b), 1.0) // renk-matematiği: temanın iki yüzeyi arasında doğrusal uzatma
    };
    let highest = at(container, high, 2.0);
    s.set(Role::SurfaceContainerLowest, at(surface, container, -0.5));
    s.set(Role::SurfaceContainerLow, at(surface, container, 0.5));
    s.set(Role::SurfaceContainerHighest, highest);
    s.set(Role::SurfaceVariant, highest);
    let (dim, bright) = match mode {
        Mode::Dark => (surface, at(container, high, 2.5)),
        Mode::Light => (at(container, high, 2.5), surface),
    };
    s.set(Role::SurfaceDim, dim);
    s.set(Role::SurfaceBright, bright);
}

/// Şablonlu çıktılar (kitty) için framework paleti: rollerin yanında 16 ANSI rengi. Kaynak
/// [`build`] ile aynı (renk teması ya da tohum); temanın elle değiştirilmiş token'ları
/// (`framework.toml [colors]` geçersiz kılmaları) buraya yansımaz.
pub fn full(seed: Seed, mode: Mode, contrast: ContrastMode) -> Palette {
    match seed.theme {
        Some(id) => Palette::from_color_theme(id.theme(), mode, contrast),
        None => Palette::generate_with(seed.source, seed.variant, mode, seed.contrast, contrast),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_tokens_win_in_their_mode() {
        let theme = ThemePalette::default();
        let p = build(&theme, Seed::new(&theme, None), Mode::Dark, ContrastMode::Standard);
        assert!(tokens_apply(&theme, Mode::Dark));
        assert_eq!(p.dark.c("surface"), theme.surface);
        assert_eq!(p.dark.c("surface_container_high"), theme.surface_bright);
        assert_eq!(p.dark.c("outline_variant"), theme.outline);
        assert_eq!(p.dark.c("primary"), theme.primary);
        // Açık kip temanın koyu token'larını almaz; yüzeyi açık.
        assert_ne!(p.light.c("surface"), theme.surface);
        let l = p.light.c("surface");
        assert!(0.2126 * l.r + 0.7152 * l.g + 0.0722 * l.b > 0.8);
    }

    #[test]
    fn wallpaper_seed_is_used_when_valid() {
        let theme = ThemePalette::default();
        let meta = PaletteMeta { source: Some("#ff0000".into()), scheme: Some("vibrant".into()), contrast: Some(0.5), ..Default::default() };
        let s = Seed::new(&theme, Some(&meta));
        assert_eq!(s.source, Color::from_hex("#ff0000").unwrap());
        assert_eq!(s.variant, Variant::Vibrant);
        let bad = PaletteMeta { source: Some("kırmızı".into()), ..Default::default() };
        assert_eq!(Seed::new(&theme, Some(&bad)).source, theme.primary);
    }

    /// Üreticilerin kullandığı her rol adı framework rolüne çözülür (`RoleName::c` yedeğine düşülmez).
    #[test]
    fn every_used_role_name_resolves() {
        for role in super::super::qt::used_roles() {
            assert!(Role::from_name(role).is_some(), "{role}");
        }
    }

    #[test]
    fn deterministic() {
        let theme = ThemePalette::default();
        assert_eq!(build(&theme, Seed::new(&theme, None), Mode::Dark, ContrastMode::Standard), build(&theme, Seed::new(&theme, None), Mode::Dark, ContrastMode::Standard));
    }

    /// Yüksek karşıtlık: türetilmiş roller farklı ve WCAG AA metin eşiğinde (yüzeyler aynı);
    /// standart kip `generate` ile bayt bayt aynı.
    #[test]
    fn high_contrast_changes_derived_roles() {
        let theme = ThemePalette::default();
        let seed = Seed::new(&theme, None);
        let std = build(&theme, seed, Mode::Dark, ContrastMode::Standard);
        let high = build(&theme, seed, Mode::Dark, ContrastMode::High);
        assert_ne!(std, high);
        let plain = Palette::generate(seed.source, seed.variant, Mode::Dark, seed.contrast);
        assert_eq!(std.light, *plain.scheme(Mode::Light), "standart kip değişmedi");
        // Açık kip (türetilmiş, token yazılmaz): yüzey aynı, metin rolleri eşikte.
        assert_eq!(high.light.c("surface"), std.light.c("surface"));
        for kip in [&high.light, &high.dark] {
            for fg in ["on_surface", "on_surface_variant", "primary", "error"] {
                let r = nuxut::theme::contrast_ratio(kip.c(fg), kip.c("surface"));
                assert!(r >= 4.5, "{fg}: {r}");
            }
        }
    }

    /// Renk teması seçiliyken iki kip de temanın tanımından (birincilden tohumlama yok): açık
    /// kipte de framework'ün çizdiği 11 token ile dışa aktarılan roller aynı (2026-10-16).
    #[test]
    fn color_theme_builds_both_modes_from_its_definition() {
        for id in ColorThemeId::ALL {
            for mode in [Mode::Dark, Mode::Light] {
                let tokens = id.theme().palette(match mode {
                    Mode::Dark => ThemeMode::Dark,
                    Mode::Light => ThemeMode::Light,
                });
                let seed = Seed::new(&tokens, None).with_color_theme(Some(id), None);
                let p = build(&tokens, seed, mode, ContrastMode::Standard);
                for m in [Mode::Dark, Mode::Light] {
                    assert_eq!(*p.get(m), Scheme::from_color_theme(id.theme(), m, 0.0), "{} {}", id.name(), m.name());
                }
                assert_eq!(p.get(mode).c("surface"), tokens.surface);
                assert_eq!(p.get(mode).c("primary"), tokens.primary);
            }
            // Duvar kağıdı tohumu varsa tema etkisiz.
            let meta = PaletteMeta { source: Some("#ff0000".into()), ..Default::default() };
            let tokens = id.theme().palette(ThemeMode::Dark);
            assert_eq!(Seed::new(&tokens, Some(&meta)).with_color_theme(Some(id), Some(&meta)).theme, None);
        }
        // Tohumlamadan farklı: nötr temanın açık kipi birincilden türetilenle aynı değil.
        let tokens = ColorThemeId::Neutral.theme().palette(ThemeMode::Light);
        let themed = build(&tokens, Seed::new(&tokens, None).with_color_theme(Some(ColorThemeId::Neutral), None), Mode::Light, ContrastMode::Standard);
        let seeded = build(&tokens, Seed::new(&tokens, None), Mode::Light, ContrastMode::Standard);
        assert_ne!(themed.dark, seeded.dark);
        // Yüksek karşıtlık temanın şemasına da uygulanır.
        let high = build(&tokens, Seed::new(&tokens, None).with_color_theme(Some(ColorThemeId::Neutral), None), Mode::Light, ContrastMode::High);
        assert_ne!(high.dark, themed.dark);
    }

    /// Elle yazılmış yüzeyler (Dracula): GTK içerik/kenar çubuğu rolleri temanın renginde, tohumun
    /// nötründe değil; sıra korunur (2026-10-17).
    #[test]
    fn foreign_surfaces_rebuild_the_container_ramp() {
        let hex = |h: &str| Color::from_hex(h).unwrap();
        let theme = ThemePalette {
            surface: hex("#282a36"),
            surface_container: hex("#343746"),
            surface_bright: hex("#44475a"),
            primary: hex("#bd93f9"),
            ..ThemePalette::default()
        };
        let p = build(&theme, Seed::new(&theme, None), Mode::Dark, ContrastMode::Standard);
        let d = &p.dark;
        let lum = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
        let ramp = ["surface_container_lowest", "surface", "surface_container_low", "surface_container", "surface_container_high", "surface_container_highest"];
        for w in ramp.windows(2) {
            assert!(lum(d.c(w[0])) < lum(d.c(w[1])), "{} < {}", w[0], w[1]);
        }
        // Mavi-mor ton korunur (Dracula yüzeyleri: mavi kanal baskın).
        for r in ramp {
            let c = d.c(r);
            assert!(c.b > c.r && c.b > c.g, "{r}: {}", c.to_hex());
        }
        // Temanın kendi şemasından gelen yüzeylere dokunulmaz (renk teması yolu ayrıca testli).
        let plain = ThemePalette::default();
        let seed = Seed::new(&plain, None);
        let derived = Palette::generate(seed.source, seed.variant, Mode::Dark, seed.contrast);
        let same = ThemePalette { surface: derived.scheme(Mode::Dark).surface, ..plain };
        let q = build(&same, Seed::new(&same, None), Mode::Dark, ContrastMode::Standard);
        assert_eq!(q.dark.c("surface_container_low"), derived.scheme(Mode::Dark).surface_container_low);
    }

    fn light_theme() -> ThemePalette {
        ThemePalette { surface: Color::from_hex("#f8f9ff").unwrap(), ..ThemePalette::default() }
    }

    #[test]
    fn tokens_go_to_resolved_mode_only_when_consistent() {
        let light = light_theme();
        let p = build(&light, Seed::new(&light, None), Mode::Light, ContrastMode::Standard);
        assert_eq!(p.light.c("surface"), light.surface);
        assert_ne!(p.dark.c("surface"), light.surface);
        // Açıkça ters kip: token'lar yazılmaz (açık yüzey koyu kipe girmez).
        let p = build(&light, Seed::new(&light, None), Mode::Dark, ContrastMode::Standard);
        assert_ne!(p.dark.c("surface"), light.surface);
        assert_ne!(p.light.c("surface"), light.surface);
    }
}
