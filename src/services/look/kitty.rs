//! kitty: Nuxut renkleri ve imleç izi (kullanıcı isteği 2026-10-16). Saf.
//!
//! İki yönetilen dosya, kullanıcının `kitty.conf`'una birer `include` satırı (sonda: kitty'de
//! sonraki tanım kazanır, `kitten themes` bloğu da ezilir):
//! - `kitty/nuxut.conf` (`look.mode`): framework'ün kitty şablonu (`nuxut_palette` yerleşik
//!   şablonu; 16 ANSI rengi temanın birincil rengine uyumlandırılmış) seçili paletle.
//! - `kitty/nuxut-motion.conf` (`look.motion`): `cursor_trail` varsayılan açık; azaltılmış
//!   harekette kapalı (`cursor_trail 0`).
//!
//! Açık pencereler `SIGUSR1` ile yapılandırmayı yeniden okur (canlı eylem `kitty_reload`).

use nuxut_palette::Palette;

pub const INCLUDE_COLORS: &str = "include nuxut.conf";
pub const INCLUDE_MOTION: &str = "include nuxut-motion.conf";

fn header(what: &str) -> String {
    format!("# Nuxut — {what} · elle düzenleme bir sonraki uygulamada ezilir\n") // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
}

/// `kitty/nuxut.conf`. Şablon yoksa ya da işlenemezse `None` (dosya yazılmaz).
pub fn colors_conf(palette: &Palette) -> Option<String> {
    let (_, tpl) = nuxut_palette::export::BUILTIN_TEMPLATES.iter().find(|(n, _)| *n == "kitty.conf")?;
    let body = nuxut_palette::template::render(tpl, palette, None).ok()?;
    // Şablonun kendi açıklama satırları (kaynak, kullanım) Nuxut başlığıyla değişir.
    let lines: String = body.lines().filter(|l| !l.starts_with('#')).map(|l| format!("{l}\n")).collect();
    Some(format!("{}{lines}", header("kitty renkleri")))
}

/// `kitty/nuxut-motion.conf`: imleç izi. Değerler kitty belgesindeki önerilen başlangıç
/// (`cursor_trail 3`: 3 ms'den uzun süren imleç hareketinde iz; sönüm 0.1–0.4 s).
pub fn motion_conf(reduced_motion: bool) -> String {
    let body = if reduced_motion { "cursor_trail 0\n" } else { "cursor_trail 3\ncursor_trail_decay 0.1 0.4\n" };
    format!("{}{body}", header("kitty hareketi"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_palette::{ContrastMode, Mode, Variant};

    #[test]
    fn colors_cover_the_terminal() {
        let p = Palette::generate_with(nuxut_core::Color::from_hex("#aac7ff").unwrap(), Variant::TonalSpot, Mode::Dark, 0.0, ContrastMode::Standard);
        let text = colors_conf(&p).unwrap();
        assert!(text.starts_with("# Nuxut — kitty renkleri"), "{text}");
        assert_eq!(text.lines().filter(|l| l.starts_with('#')).count(), 1, "tek başlık");
        for key in ["foreground ", "background ", "cursor ", "selection_background ", "color0 ", "color15 "] {
            assert!(text.lines().any(|l| l.starts_with(key) && l.len() == key.len() + 7 && l.ends_with(|c: char| c.is_ascii_hexdigit())), "{key}: {text}");
        }
        assert!(!text.contains("{{"), "işlenmemiş yer tutucu yok");
    }

    #[test]
    fn motion_follows_reduced_motion() {
        assert!(motion_conf(false).contains("\ncursor_trail 3\n"));
        let reduced = motion_conf(true);
        assert!(reduced.contains("\ncursor_trail 0\n") && !reduced.contains("decay"));
    }
}
