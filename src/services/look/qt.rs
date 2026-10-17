//! Qt/KDE üreticileri (ADR-0004 §5.1, §5.3; saf).
//!
//! - Birincil yol: `kdeglobals` (KDE platform teması `QT_QPA_PLATFORMTHEME=kde` okur; canlı,
//!   `KGlobalSettings.notifyChange`) + KDE Sistem Ayarları'nda görünsün diye `Nuxut.colors`.
//! - Yedek yol: qt6ct/qt5ct renk şeması (QPalette sırası) ve `qt6ct.conf`/`qt5ct.conf` anahtarları.

use std::fmt::Write as _;

use nuxut_core::Color;

use super::fonts::Fonts;
use super::plan::IniKey;
use nuxut_palette::Scheme;

use super::palette::RoleName;

/// Renk şeması adı (kdeglobals `[General] ColorScheme`, `Nuxut.colors`).
pub const SCHEME_NAME: &str = "Nuxut";

/// KColorScheme grupları ve zemin rolleri: (grup, BackgroundNormal, BackgroundAlternate).
const GROUPS: [(&str, &str, &str); 7] = [
    ("View", "surface_container_low", "surface_container"),
    ("Window", "surface", "surface_container"),
    ("Button", "surface_container_high", "surface_container_highest"),
    ("Selection", "primary", "primary_container"),
    ("Tooltip", "inverse_surface", "inverse_surface"),
    ("Complementary", "surface_container_low", "surface_container"),
    ("Header", "surface_container", "surface"),
];

/// Bir grubun ön plan rolleri (KColorScheme'in 10 ön plan/süsleme anahtarı; `success`/`warning`
/// framework'ün Nuxut rolleri).
fn foreground(group: &str) -> [(&'static str, &'static str); 10] {
    match group {
        "Selection" => [
            ("ForegroundNormal", "on_primary"),
            ("ForegroundInactive", "on_primary"),
            ("ForegroundActive", "on_primary"),
            ("ForegroundLink", "on_primary"),
            ("ForegroundVisited", "on_primary"),
            ("ForegroundNegative", "on_primary"),
            ("ForegroundNeutral", "on_primary"),
            ("ForegroundPositive", "on_primary"),
            ("DecorationFocus", "primary"),
            ("DecorationHover", "primary"),
        ],
        "Tooltip" => [
            ("ForegroundNormal", "inverse_on_surface"),
            ("ForegroundInactive", "inverse_on_surface"),
            ("ForegroundActive", "inverse_primary"),
            ("ForegroundLink", "inverse_primary"),
            ("ForegroundVisited", "inverse_primary"),
            ("ForegroundNegative", "error"),
            ("ForegroundNeutral", "warning"),
            ("ForegroundPositive", "success"),
            ("DecorationFocus", "primary"),
            ("DecorationHover", "primary"),
        ],
        _ => [
            ("ForegroundNormal", "on_surface"),
            ("ForegroundInactive", "on_surface_variant"),
            ("ForegroundActive", "primary"),
            ("ForegroundLink", "primary"),
            ("ForegroundVisited", "tertiary"),
            ("ForegroundNegative", "error"),
            ("ForegroundNeutral", "warning"),
            ("ForegroundPositive", "success"),
            ("DecorationFocus", "primary"),
            // Breeze geleneği: üzerine gelme vurgu tonuyla (ADR §5.1'deki outline_variant
            // önerisi Breeze'de görünmez kalıyordu; §17-4).
            ("DecorationHover", "primary"),
        ],
    }
}

/// KDE renk değeri `r,g,b`.
pub fn kde_rgb(c: Color) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{},{},{}", b(c.r), b(c.g), b(c.b))
}

/// `[Colors:*]` + `[ColorEffects:*]` anahtarları (kdeglobals ve `Nuxut.colors` ortak).
pub fn color_keys(roles: &Scheme) -> Vec<IniKey> {
    let mut keys = Vec::new();
    for (group, bg, bg_alt) in GROUPS {
        let sec = format!("Colors:{group}");
        keys.push(IniKey::new(&sec, "BackgroundNormal", kde_rgb(roles.c(bg))));
        keys.push(IniKey::new(&sec, "BackgroundAlternate", kde_rgb(roles.c(bg_alt))));
        for (key, role) in foreground(group) {
            keys.push(IniKey::new(&sec, key, kde_rgb(roles.c(role))));
        }
    }
    // Pasif pencere soluklaştırılmaz (Material'de yok); devre dışı öğeler Breeze katsayılarıyla
    // (renk değil katsayı; ColorAmount 0 → renk karışmaz).
    keys.push(IniKey::new("ColorEffects:Inactive", "Enable", "false"));
    keys.push(IniKey::new("ColorEffects:Inactive", "ChangeSelectionColor", "false"));
    for (k, v) in [
        ("ColorAmount", "0"),
        ("ColorEffect", "0"),
        ("ContrastAmount", "0.65"),
        ("ContrastEffect", "1"),
        ("IntensityAmount", "0.1"),
        ("IntensityEffect", "2"),
    ] {
        keys.push(IniKey::new("ColorEffects:Disabled", k, v));
    }
    keys
}

/// kdeglobals'ın tüm yönetilen anahtarları: renkler + şema adı + yazı tipleri + ikon + stil.
pub fn kdeglobals(roles: &Scheme, fonts: &Fonts, icon_theme: &str, widget_style: &str) -> Vec<IniKey> {
    let mut keys = color_keys(roles);
    keys.push(IniKey::new("General", "ColorScheme", SCHEME_NAME));
    if let Some(f) = &fonts.ui_qt {
        for k in ["font", "menuFont", "toolBarFont"] {
            keys.push(IniKey::new("General", k, f.clone()));
        }
    }
    if let Some(f) = &fonts.small_qt {
        keys.push(IniKey::new("General", "smallestReadableFont", f.clone()));
    }
    if let Some(f) = &fonts.mono_qt {
        keys.push(IniKey::new("General", "fixed", f.clone()));
    }
    keys.push(IniKey::new("Icons", "Theme", icon_theme));
    keys.push(IniKey::new("KDE", "widgetStyle", widget_style));
    keys
}

/// `~/.local/share/color-schemes/Nuxut.colors` içeriği (yönetilen dosya; başlık yorumlu).
pub fn colors_file(roles: &Scheme) -> String {
    let mut keys = vec![IniKey::new("General", "ColorScheme", SCHEME_NAME), IniKey::new("General", "Name", SCHEME_NAME)];
    keys.extend(color_keys(roles));
    format!("{KDE_HEADER}\n{}", ini_text(&keys))
}

/// Anahtarları bölüm başlıklarıyla düz INI metnine çevirir (bölümler ilk geçiş sırasıyla, aralarında
/// boş satır). Yalnızca baştan üretilen dosya için; birleştirme yok.
fn ini_text(keys: &[IniKey]) -> String {
    let mut sections: Vec<&str> = Vec::new();
    for k in keys {
        if !sections.contains(&k.section.as_str()) {
            sections.push(&k.section);
        }
    }
    let mut out = String::new();
    for (i, sec) in sections.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let _ = writeln!(out, "[{sec}]");
        for k in keys.iter().filter(|k| k.section == *sec) {
            let _ = writeln!(out, "{}={}", k.key, k.value);
        }
    }
    out
}

/// INI biçimli yönetilen dosyaların başlığı.
pub const KDE_HEADER: &str = "# Nuxut — birleşik görünüm · elle düzenleme bir sonraki uygulamada ezilir"; // i18n: tanı üretilen KDE INI dosyalarının başlık yorumu (dosya içeriği)

/// QPalette rolleri (qt6ct dosya sırası; Qt ≥ 6.6'da 22. `Accent`). Değer: etkin grubun rolü.
const QPALETTE: [(&str, &str); 22] = [
    ("WindowText", "on_surface"),
    ("Button", "surface_container_high"),
    ("Light", "surface_container_highest"),
    ("Midlight", "surface_container_high"),
    ("Dark", "surface_container_lowest"),
    ("Mid", "outline_variant"),
    ("Text", "on_surface"),
    ("BrightText", "inverse_on_surface"),
    ("ButtonText", "on_surface"),
    ("Base", "surface_container_low"),
    ("Window", "surface"),
    ("Shadow", "shadow"),
    ("Highlight", "primary"),
    ("HighlightedText", "on_primary"),
    ("Link", "primary"),
    ("LinkVisited", "tertiary"),
    ("AlternateBase", "surface_container"),
    ("NoRole", "surface"),
    ("ToolTipBase", "inverse_surface"),
    ("ToolTipText", "inverse_on_surface"),
    ("PlaceholderText", "on_surface_variant"),
    ("Accent", "primary"),
];

fn qt_argb(c: Color) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#ff{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
}

/// QPalette grubu: `inactive` vurgu `primary_container`; `disabled` metinler `outline`.
fn palette_row(roles: &Scheme, group: &str, count: usize) -> String {
    let row: Vec<String> = QPALETTE[..count]
        .iter()
        .map(|(name, role)| {
            let role = match (group, *name) {
                ("inactive", "Highlight") => "primary_container",
                ("inactive", "HighlightedText") => "on_primary_container",
                ("disabled", "WindowText" | "Text" | "ButtonText" | "PlaceholderText") => "outline",
                ("disabled", "Highlight") => "surface_container_highest",
                ("disabled", "HighlightedText") => "on_surface_variant",
                _ => role,
            };
            qt_argb(roles.c(role))
        })
        .collect();
    row.join(", ")
}

/// qt6ct (22 rol) / qt5ct (21 rol) renk şeması dosyası.
pub fn qtct_scheme(roles: &Scheme, qt6: bool) -> String {
    let n = if qt6 { 22 } else { 21 };
    let mut out = format!("{KDE_HEADER}\n[ColorScheme]\n");
    for group in ["active", "disabled", "inactive"] {
        let _ = writeln!(out, "{group}_colors={}", palette_row(roles, group, n));
    }
    out
}

/// `qt6ct.conf`/`qt5ct.conf` anahtarları (yedek yol; yazı tipleri v1'de yazılmaz — §17-4).
pub fn qtct_conf(scheme_path: &str, icon_theme: &str, style: &str) -> Vec<IniKey> {
    vec![
        IniKey::new("Appearance", "custom_palette", "true"),
        IniKey::new("Appearance", "color_scheme_path", scheme_path),
        IniKey::new("Appearance", "icon_theme", icon_theme),
        IniKey::new("Appearance", "style", style),
    ]
}

/// Nuxut Qt stilinin adı (eklentinin anahtarı; kdeglobals `widgetStyle`, qt6ct `style`).
pub const NUXUT_STYLE: &str = "Nuxut";

/// Nuxut Qt stilinin token dosyası (`$XDG_CONFIG_HOME/nuxut/qt-style.conf`; ADR-0009 §2, biçim
/// `qt-style/src/tokens.h`): köşe yarıçapları temadan, odak halkası framework'ün kalınlığı ve
/// birincil rengi, aralık ızgarası. Eklenti dosyayı izler; açık Qt uygulamaları canlı güncellenir.
pub fn style_tokens(radius_small: f32, radius_medium: f32, focus: Color) -> String {
    let px = |v: f32| v.round().max(0.0) as u32;
    format!(
        "# Nuxut — Qt stili · elle düzenleme bir sonraki uygulamada ezilir\nversion=1\nradius_small={}\nradius_medium={}\nfocus_ring={}\nfocus_color={}\nspacing={}\n", // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
        px(radius_small),
        px(radius_medium),
        px(nuxut::ui::style::FOCUS_RING),
        focus.with_alpha(1.0).to_hex(),
        px(nuxut::theme::SPACING_GRID),
    )
}

#[cfg(test)]
pub(crate) fn used_roles() -> impl Iterator<Item = &'static str> {
    let groups = GROUPS.iter().flat_map(|(g, a, b)| {
        let fg: Vec<&'static str> = foreground(g).iter().map(|(_, r)| *r).collect();
        [*a, *b].into_iter().chain(fg)
    });
    groups.chain(QPALETTE.iter().map(|(_, r)| *r)).chain(["primary_container", "on_primary_container", "outline", "surface_container_highest", "on_surface_variant"])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_scheme() -> Scheme {
        let t = nuxut::theme::Palette::default();
        nuxut_palette::Scheme::generate(t.primary, nuxut_palette::Variant::TonalSpot, nuxut_palette::Mode::Dark, 0.0)
    }

    #[test]
    fn kde_values() {
        assert_eq!(kde_rgb(Color::from_hex("#9ec5ff").unwrap()), "158,197,255");
        assert_eq!(qt_argb(Color::from_hex("#9ec5ff").unwrap()), "#ff9ec5ff");
    }

    #[test]
    fn every_group_has_its_keys() {
        let keys = color_keys(&test_scheme());
        for (g, ..) in GROUPS {
            let n = keys.iter().filter(|k| k.section == format!("Colors:{g}")).count();
            assert_eq!(n, 12, "{g}");
        }
    }

    #[test]
    fn colors_file_is_plain_ini() {
        let text = colors_file(&test_scheme());
        assert!(text.starts_with(&format!("{KDE_HEADER}\n[General]\nColorScheme=Nuxut\nName=Nuxut\n\n[Colors:View]\n")));
        assert_eq!(text.matches("[Colors:").count(), GROUPS.len());
        assert_eq!(text.matches("[General]").count(), 1);
        assert!(text.ends_with('\n') && !text.ends_with("\n\n"));
    }

    #[test]
    fn qtct_counts() {
        let s = qtct_scheme(&test_scheme(), true);
        let active = s.lines().find(|l| l.starts_with("active_colors=")).unwrap();
        assert_eq!(active.matches('#').count(), 22);
        let s = qtct_scheme(&test_scheme(), false);
        let active = s.lines().find(|l| l.starts_with("active_colors=")).unwrap();
        assert_eq!(active.matches('#').count(), 21);
    }

    /// Eklentinin ayrıştırıcısıyla aynı biçim (`qt-style/src/tokens.cpp`: `version=1`, sınırlar,
    /// `#rrggbb`).
    #[test]
    fn style_tokens_format() {
        let text = style_tokens(8.4, 14.0, Color::from_hex("#9ec5ff").unwrap().with_alpha(0.5));
        assert!(text.starts_with("# Nuxut — "));
        let kv: Vec<(&str, &str)> = text.lines().skip(1).map(|l| l.split_once('=').unwrap()).collect();
        assert_eq!(kv, [("version", "1"), ("radius_small", "8"), ("radius_medium", "14"), ("focus_ring", "2"), ("focus_color", "#9ec5ff"), ("spacing", "4")]);
    }
}
