//! GTK hedefleri (ADR-0004 §5.2; saf).
//!
//! CSS içeriği framework'ten (`nuxut_palette::look`): GTK4/libadwaita **CSS değişkenleri** iki kip
//! `@media (prefers-color-scheme)` bloklarıyla (B1, B4), GTK3 etkin kip `@define-color`; yönetilen
//! dosya başlığı dahil. Burada yalnızca `settings.ini` ve dconf anahtarları.

use nuxut_palette::Mode;

use super::palette::ExportPalette;
use super::plan::IniKey;

/// Yönetilen dosya başlığının işareti (`nuxut-apply` `edit::managed::HEADER_MARK`). Framework
/// CSS'i (9300dd1) ilk satırı zaten `/* Nuxut — birleşik görünüm, GTK … · elle düzenleme bir
/// sonraki uygulamada ezilir */` olarak üretir; shell ayrıca başlık eklemez.
pub const CSS_HEADER_PREFIX: &str = "/* Nuxut — ";

/// `gtk.css` sonuna eklenen satır (`nuxut.css` aynı dizinde; `nuxut-apply` `Line`, dosya sonu;
/// kullanıcı `nuxut.css`'i başka biçimde anmışsa — yorumlanmış satır dahil — eklenmez).
pub const GTK_IMPORT: &str = "@import url(\"nuxut.css\");";

/// `gtk-3.0/nuxut.css`: etkin kip.
pub fn gtk3_css(p: &ExportPalette, mode: Mode) -> String {
    nuxut_palette::look::gtk3_css(p.get(mode), mode)
}

/// `gtk-4.0/nuxut.css`: iki kip (etkin kip `@media`'sız varsayılan blokta da).
pub fn gtk4_css(p: &ExportPalette, mode: Mode) -> String {
    nuxut_palette::look::gtk4_css(&p.dark, &p.light, mode)
}

/// `settings.ini` `[Settings]` anahtarları (GTK3 ve GTK4 aynı).
///
/// Erişilebilirlik (framework `docs/design/accessibility.md` §2–3, t124): `gtk-xft-dpi` metin
/// ölçeğinin GTK'ya tek girişi (yazı tipi adı referans boyda; [`super::fonts`]),
/// `gtk-enable-animations` = `!Theme::reduced_motion()` (`1`/`0`).
#[allow(clippy::too_many_arguments)]
pub fn settings_ini(
    gtk_theme: &str,
    icon: &str,
    cursor: &str,
    cursor_size: u32,
    font: Option<&str>,
    xft_dpi: &str,
    animations: bool,
    mode: Mode,
) -> Vec<IniKey> {
    let s = "Settings";
    let mut keys = vec![
        IniKey::new(s, "gtk-theme-name", gtk_theme),
        IniKey::new(s, "gtk-icon-theme-name", icon),
        IniKey::new(s, "gtk-cursor-theme-name", cursor),
        IniKey::new(s, "gtk-cursor-theme-size", cursor_size.to_string()),
    ];
    if let Some(f) = font {
        keys.push(IniKey::new(s, "gtk-font-name", f));
    }
    keys.push(IniKey::new(s, "gtk-xft-dpi", xft_dpi));
    keys.push(IniKey::new(s, "gtk-enable-animations", if animations { "1" } else { "0" }));
    keys.push(IniKey::new(s, "gtk-application-prefer-dark-theme", if mode == Mode::Dark { "true" } else { "false" }));
    keys
}

/// Yalnızca GTK 4 `settings.ini`: `gtk-interface-color-scheme` (GTK ≥ 4.20). Ayarlar portalı
/// yokken düz GTK 4 uygulamaları (libadwaita'sız) `prefers-color-scheme`'i bundan okur; aksi
/// hâlde açık kipe düşer (canlı doğrulandı: pavucontrol). GTK 3 bu anahtarı tanımaz.
pub fn gtk4_color_scheme(mode: Mode) -> IniKey {
    IniKey::new("Settings", "gtk-interface-color-scheme", if mode == Mode::Dark { "dark" } else { "light" })
}

/// GVariant dizge değişmezi (`'…'`, `'` ve `\` kaçışlı).
pub fn gvariant_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// GVariant `d` değişmezi, `dconf read`'in yazdırdığı biçimde (GLib `g_variant_print`:
/// `g_ascii_dtostr` = `%.17g`, nokta/üs yoksa `.0` eklenir). `nuxut-apply` dconf değerini metin
/// olarak karşılaştırır: yazılan metin okunanla bayt-eşit olmazsa her tur "değişti" sayılır ve
/// yeniden yazılır (2026-10-16 hatası: `1.00` → okunan `1.0`, look.fonts saniyede bir yazılıyordu).
/// Ölçülen GLib çıktıları testte. Sonlu olmayan değer `1.0`.
pub fn gvariant_double(x: f64) -> String {
    if !x.is_finite() {
        return "1.0".into();
    }
    let sci = format!("{x:.16e}");
    let exp: i32 = sci.rsplit_once('e').and_then(|(_, e)| e.parse().ok()).unwrap_or(0);
    let strip = |s: &str| -> String {
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s.to_owned() }
    };
    let mut out = if (-4..17).contains(&exp) {
        strip(&format!("{x:.*}", (16 - exp) as usize))
    } else {
        let (mantissa, _) = sci.split_once('e').unwrap_or((&sci, ""));
        format!("{}e{}{:02}", strip(mantissa), if exp < 0 { '-' } else { '+' }, exp.unsigned_abs())
    };
    if !out.contains(['.', 'e']) {
        out.push_str(".0");
    }
    out
}

/// GNOME'un adlandırılmış vurgu renkleri (`org.gnome.desktop.interface accent-color`, GNOME ≥ 47)
/// ve libadwaita'daki karşılıkları.
const GNOME_ACCENTS: [(&str, u32); 8] = [
    ("blue", 0x3584e4),
    ("teal", 0x2190a4),
    ("green", 0x3a944a),
    ("yellow", 0xc88800),
    ("orange", 0xed5b00),
    ("red", 0xe62d42),
    ("pink", 0xd56199),
    ("purple", 0x9141ac),
];

/// Bu OKLCH kromasının altındaki vurgu renksiz sayılır: `slate`.
const SLATE_CHROMA: f32 = 0.04;

/// sRGB → OKLab (a, b).
fn oklab_ab(c: nuxut_core::Color) -> (f32, f32) {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let (r, g, b) = (lin(c.r), lin(c.g), lin(c.b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    (1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s, 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s)
}

/// Nuxut vurgusuna (palet `primary`) OKLCH tonu en yakın GNOME vurgu adı; renksiz vurgu `slate`.
/// Portal `accent-color`'ı tam rengi verir; bu ad yalnız gsettings okuyan uygulamalar için
/// (2026-10-17: kullanıcıda 'blue' kalmıştı).
pub fn gnome_accent(c: nuxut_core::Color) -> &'static str {
    let (a, b) = oklab_ab(c);
    if a.hypot(b) < SLATE_CHROMA {
        return "slate";
    }
    let hue = b.atan2(a);
    let dist = |rgb: u32| {
        let (ga, gb) = oklab_ab(nuxut_core::Color::from_rgba8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)); // renk-verisi: GNOME vurgu tablosundaki başvuru rengi (tema rengi değil)
        let d = (gb.atan2(ga) - hue).rem_euclid(std::f32::consts::TAU);
        d.min(std::f32::consts::TAU - d)
    };
    GNOME_ACCENTS.iter().min_by(|x, y| dist(x.1).total_cmp(&dist(y.1))).map_or("blue", |(name, _)| name)
}

/// dconf `/org/gnome/desktop/interface/` anahtarları: (anahtar yolu, GVariant metni).
#[derive(Debug)]
pub struct DconfInput<'a> {
    pub mode: Mode,
    pub gtk_theme: &'a str,
    pub icon: &'a str,
    pub cursor: &'a str,
    pub cursor_size: u32,
    pub ui_font: Option<&'a str>,
    pub document_font: Option<&'a str>,
    pub mono_font: Option<&'a str>,
    /// `text-scaling-factor` (GVariant `d`; [`super::fonts::Fonts::text_scaling_factor`]).
    pub text_scaling_factor: &'a str,
    /// Nuxut vurgusu (açık palet `primary`); `accent-color` [`gnome_accent`] ile en yakın ada eşlenir.
    pub accent: nuxut_core::Color,
    /// `enable-animations` = `!Theme::reduced_motion()`.
    pub animations: bool,
}

pub const DCONF_DIR: &str = "/org/gnome/desktop/interface/";

pub fn dconf(i: &DconfInput<'_>) -> Vec<(String, String)> {
    let scheme = if i.mode == Mode::Dark { "prefer-dark" } else { "prefer-light" };
    let mut out = vec![
        ("color-scheme", gvariant_str(scheme)),
        ("gtk-theme", gvariant_str(i.gtk_theme)),
        ("icon-theme", gvariant_str(i.icon)),
        ("cursor-theme", gvariant_str(i.cursor)),
        ("cursor-size", i.cursor_size.to_string()),
    ];
    for (key, font) in [("font-name", i.ui_font), ("document-font-name", i.document_font), ("monospace-font-name", i.mono_font)] {
        if let Some(f) = font {
            out.push((key, gvariant_str(f)));
        }
    }
    out.push(("text-scaling-factor", i.text_scaling_factor.to_owned()));
    out.push(("enable-animations", i.animations.to_string()));
    out.push(("accent-color", gvariant_str(gnome_accent(i.accent))));
    out.into_iter().map(|(k, v)| (format!("{DCONF_DIR}{k}"), v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: nuxut_core::Color) -> String {
        c.to_hex()[..7].to_owned()
    }

    /// Tek başlık satırı ve `nuxut-apply` `is_managed` biçimi (`/* Nuxut — … · elle düzenleme … */`).
    fn assert_managed(css: &str) {
        let first = css.lines().next().unwrap();
        assert!(first.starts_with(CSS_HEADER_PREFIX) && first.ends_with(" · elle düzenleme bir sonraki uygulamada ezilir */"), "{first}");
        assert_eq!(css.matches(CSS_HEADER_PREFIX).count(), 1, "çift başlık");
    }

    #[test]
    fn css_files_carry_header_and_theme_surface() {
        let theme = nuxut::theme::Palette::default();
        let p = crate::services::look::palette::build(&theme, crate::services::look::palette::Seed::new(&theme, None), Mode::Dark, nuxut_palette::ContrastMode::Standard);
        let css = gtk4_css(&p, Mode::Dark);
        assert_managed(&css);
        assert!(!css.contains("@define-color"), "libadwaita @define-color'ı yok sayar (B1)");
        let dark = css.find("@media (prefers-color-scheme: dark)").unwrap();
        let light = css.find("@media (prefers-color-scheme: light)").unwrap();
        let bg = format!("--window-bg-color: {};", hex(theme.surface));
        assert!(css[..light].contains(&bg), "varsayılan blok etkin (koyu) kip");
        assert!(css[dark..].contains(&bg));
        assert!(!css[light..dark].contains(&bg), "açık kip temanın koyu yüzeyini almaz");
        assert!(css.contains("--success-bg-color:") && css.contains("--warning-bg-color:"));
        let gtk3 = gtk3_css(&p, Mode::Dark);
        assert_managed(&gtk3);
        assert!(gtk3.contains(&format!("@define-color window_bg_color {};", hex(theme.surface))));
        assert!(!gtk3.contains("@media"));
    }

    #[test]
    fn gvariant_escapes() {
        assert_eq!(gvariant_str("a'b\\c"), "'a\\'b\\\\c'");
    }

    /// `dconf write /k <v>; dconf read /k` çıktıları (yalıtılmış dconf, GLib 2.86, 2026-10-16).
    /// Yazılan metin okunana eşit olmalı: yoksa her tur yeniden yazılır.
    #[test]
    fn gvariant_double_matches_dconf_read() {
        for (v, read) in [(1.0, "1.0"), (1.25, "1.25"), (1.23, "1.23"), (1.1, "1.1000000000000001"), (0.9, "0.90000000000000002"), (2.0, "2.0")] {
            assert_eq!(gvariant_double(v), read, "{v}");
        }
        assert_eq!(gvariant_double(1e20), "1e+20");
        assert_eq!(gvariant_double(f64::NAN), "1.0");
    }

    #[test]
    fn settings_and_dconf_keys() {
        let k = settings_ini("adw-gtk3-dark", "Papirus-Dark", "Bibata-Modern-Classic", 24, Some("Inter Medium 10"), "120914", true, Mode::Dark);
        assert_eq!(k.len(), 8);
        assert_eq!(k[7].value, "true");
        let get = |k: &[IniKey], name: &str| k.iter().find(|x| x.key == name).map(|x| x.value.clone());
        assert_eq!(get(&k, "gtk-xft-dpi").as_deref(), Some("120914"));
        assert_eq!(get(&k, "gtk-enable-animations").as_deref(), Some("1"));
        // Azaltılmış hareket: GTK geçişleri kapalı.
        let still = settings_ini("adw-gtk3", "Papirus", "Adwaita", 24, None, "98304", false, Mode::Light);
        assert_eq!(get(&still, "gtk-enable-animations").as_deref(), Some("0"));
        let d = dconf(&DconfInput {
            mode: Mode::Light,
            gtk_theme: "adw-gtk3",
            icon: "Papirus-Light",
            cursor: "Bibata-Modern-Classic",
            cursor_size: 32,
            ui_font: Some("Inter Medium 10"),
            document_font: None,
            mono_font: None,
            text_scaling_factor: "1.23",
            animations: false,
            accent: nuxut_core::Color::from_rgba8(0xff, 0xb4, 0xa5, 255),
        });
        assert_eq!(d[0], ("/org/gnome/desktop/interface/color-scheme".into(), "'prefer-light'".into()));
        assert_eq!(d[4].1, "32");
        assert_eq!(d.len(), 9);
        assert_eq!(d[6], ("/org/gnome/desktop/interface/text-scaling-factor".into(), "1.23".into()));
        assert_eq!(d[7], ("/org/gnome/desktop/interface/enable-animations".into(), "false".into()));
        assert_eq!(d[8], ("/org/gnome/desktop/interface/accent-color".into(), "'red'".into()));
    }

    #[test]
    fn gnome_accent_names_follow_hue() {
        let c = |hex: &str| nuxut_core::Color::from_hex(hex).unwrap();
        for (hex, name) in [
            ("#3584e4", "blue"),
            ("#aac7ff", "blue"),
            ("#2190a4", "teal"),
            ("#3a944a", "green"),
            ("#c88800", "yellow"),
            ("#ed5b00", "orange"),
            ("#ffb4a5", "red"),
            ("#d56199", "pink"),
            ("#9141ac", "purple"),
            ("#cdb9fb", "purple"),
            ("#8a8a8a", "slate"),
            ("#ffffff", "slate"),
        ] {
            assert_eq!(gnome_accent(c(hex)), name, "{hex}");
        }
    }
}
