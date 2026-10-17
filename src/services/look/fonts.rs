//! Yazı tipi dönüşümleri (ADR-0004 §5.5): framework `[typography]` rolleri → Pango (GTK, dconf)
//! ve QFont (kdeglobals) dizgeleri. Boy hesabı framework'ün `nuxut_palette::text_scale`'inden
//! (docs/design/accessibility.md §3, t124): metin ölçeği her hedefe TAM BİR kez girer.
//!
//! - GTK adı (`font-name`, `gtk-font-name`) REFERANS boyda (`gtk_font_pt`, 13 px × rol çarpanı);
//!   ölçek ayrıca `text-scaling-factor` (dconf) ve `gtk-xft-dpi` (`settings.ini`) ile verilir
//!   ([`text_scale`]). Önceki sürüm tam ölçekli pt'yi ada koyuyordu (ölçek yalnız sistem fontunu
//!   kullananlara ulaşıyordu); sonraki yazım bunu referans boya çevirir.
//! - Qt kdeglobals QFont boyu ölçek katılmış (`qt_font_pt`); `QT_FONT_DPI` bu yolda yok.
//!
//! Dizge biçimleri framework'te yok; tek tüketici shell olduğundan burada kalır (ADR-0004 §17-6).

use nuxut::theme::{FontRole, Typography};
use nuxut_palette::text_scale::{self, QtFonts, TextScale};

/// Aile adı dizgelere güvenle girer mi (QFont virgülle ayırır; denetim karakteri yok)?
pub fn valid_family(f: &str) -> bool {
    !f.is_empty() && f.len() <= 128 && !f.contains(',') && !f.chars().any(char::is_control) && !f.contains(['\'', '"', '\\'])
}

fn pt_text(pt: f32) -> String {
    if pt.fract() == 0.0 { format!("{pt:.0}") } else { format!("{pt:.1}") }
}

/// Pango ağırlık adı (400 = adsız).
fn pango_weight(w: u16) -> Option<&'static str> {
    Some(match w {
        0..=149 => "Thin",
        150..=249 => "Ultra-Light",
        250..=349 => "Light",
        350..=449 => return None,
        450..=549 => "Medium",
        550..=649 => "Semi-Bold",
        650..=749 => "Bold",
        750..=849 => "Ultra-Bold",
        _ => "Heavy",
    })
}

/// `"Inter Medium 10"` (dconf `font-name`, `settings.ini gtk-font-name`). Boy REFERANS boyda
/// (`text_scale::gtk_font_pt`): metin ölçeği GTK'ya `text-scaling-factor`/`gtk-xft-dpi` ile girer.
pub fn pango(role: &FontRole) -> Option<String> {
    if !valid_family(&role.family) {
        return None;
    }
    let pt = pt_text(text_scale::gtk_font_pt(role.scale));
    Some(match pango_weight(role.weight) {
        Some(w) => format!("{} {w} {pt}", role.family),
        None => format!("{} {pt}", role.family),
    })
}

/// Qt6 `QFont::toString` biçimi (Plasma 6'nın kdeglobals'a yazdığı):
/// `aile,pt,-1,5,ağırlık,0,0,0,0,0,0,0,0,0,0,1`. Boy ölçek katılmış (`text_scale::qt_font_pt`).
pub fn qfont(role: &FontRole, size_px: f32) -> Option<String> {
    if !valid_family(&role.family) {
        return None;
    }
    let pt = pt_text(text_scale::qt_font_pt(size_px, role.scale));
    let weight = role.weight.clamp(100, 900);
    Some(format!("{},{pt},-1,5,{weight},0,0,0,0,0,0,0,0,0,0,1", role.family))
}

/// Dış hedeflere giden yazı tipi dizgeleri ve metin ölçeği değerleri.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fonts {
    pub ui_pango: Option<String>,
    pub document_pango: Option<String>,
    pub mono_pango: Option<String>,
    pub ui_qt: Option<String>,
    pub small_qt: Option<String>,
    pub mono_qt: Option<String>,
    /// dconf `text-scaling-factor` (GVariant `d`, 0.01 adım, `dconf read` biçiminde: `1.0`, `1.23`;
    /// [`super::gtk::gvariant_double`]).
    pub text_scaling_factor: String,
    /// `settings.ini gtk-xft-dpi` (DPI × 1024).
    pub xft_dpi: String,
}

/// En küçük okunur yazı (`smallestReadableFont`) taban boyun bu katı.
const SMALL_SCALE: f32 = 0.8;

pub fn fonts(t: &Typography) -> Fonts {
    let small = FontRole { scale: t.ui.scale * SMALL_SCALE, ..t.ui.clone() };
    let gtk = text_scale::gtk(t);
    Fonts {
        ui_pango: pango(&t.ui),
        document_pango: pango(&t.document),
        mono_pango: pango(&t.mono),
        ui_qt: qfont(&t.ui, t.size),
        small_qt: qfont(&small, t.size),
        mono_qt: qfont(&t.mono, t.size),
        text_scaling_factor: super::gtk::gvariant_double((gtk.text_scaling_factor * 100.0).round() / 100.0),
        xft_dpi: gtk.xft_dpi.to_string(),
    }
}

/// qt6ct yolunun tek ölçek aktarımı: `QT_FONT_DPI` (oturum ortamı). Yalnızca qt6ct etkin ve ölçek
/// ≠ 1 iken `Some`; kdeglobals yolunda (ölçek QFont pt'sinde) ve Qt platform teması yokken `None`.
pub fn qt_font_dpi(t: &Typography, qt6ct: bool) -> Option<u32> {
    if !qt6ct {
        return None;
    }
    TextScale::of(t).qt_font_dpi(QtFonts::NotWritten)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(family: &str, weight: u16) -> FontRole {
        FontRole { family: family.into(), weight, scale: 1.0 }
    }

    fn typo(text: &str) -> Typography {
        nuxut::Theme::parse(text).unwrap().0.typography
    }

    #[test]
    fn converts_sizes_and_weights() {
        assert_eq!(pango(&role("Inter", 500)).as_deref(), Some("Inter Medium 10"));
        assert_eq!(pango(&FontRole { scale: 14.0 / 13.0, ..role("Source Serif 4", 400) }).as_deref(), Some("Source Serif 4 10.5"));
        assert_eq!(qfont(&role("Inter", 500), 13.0).as_deref(), Some("Inter,10,-1,5,500,0,0,0,0,0,0,0,0,0,0,1"));
        assert_eq!(qfont(&role("Inter", 400), 14.0).as_deref(), Some("Inter,10.5,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"));
    }

    #[test]
    fn unsafe_families_are_skipped() {
        for bad in ["", "A,B", "x'y", "a\nb"] {
            assert!(pango(&role(bad, 400)).is_none(), "{bad:?}");
            assert!(qfont(&role(bad, 400), 13.0).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn default_typography_maps_every_role() {
        let t = nuxut::Theme::default().typography;
        let f = fonts(&t);
        for v in [&f.ui_pango, &f.document_pango, &f.mono_pango, &f.ui_qt, &f.small_qt, &f.mono_qt] {
            assert!(v.is_some());
        }
        assert_eq!(f.text_scaling_factor, "1.0", "GVariant çift, dconf read biçimi (1.00 her tur yeniden yazılıyordu)");
        assert_eq!(f.xft_dpi, "98304");
        assert_eq!(qt_font_dpi(&t, true), None, "ölçek 1: QT_FONT_DPI yok");
    }

    /// Ölçek her hedefe tek kez: GTK adı referans boyda + çarpan; Qt kdeglobals pt'si ölçekli;
    /// QT_FONT_DPI yalnızca qt6ct yolunda.
    #[test]
    fn text_scale_enters_each_target_once() {
        let t = typo("[typography]\nsize = 16\n");
        let f = fonts(&t);
        let ui = f.ui_pango.as_deref().unwrap();
        assert!(ui.ends_with(" 10"), "GTK adı referans boyda (12 değil): {ui}");
        assert_eq!(f.text_scaling_factor, "1.23");
        assert_eq!(f.xft_dpi, "120914");
        assert!(f.ui_qt.as_deref().unwrap().contains(",12,"), "kdeglobals ölçekli: {:?}", f.ui_qt);
        assert_eq!(qt_font_dpi(&t, false), None, "kdeglobals yolunda QT_FONT_DPI yok");
        assert_eq!(qt_font_dpi(&t, true), Some(118));
    }
}
