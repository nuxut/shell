//! Temaların görsel varlıkları: logo, yuvarlatılmış kutular (9 parça), küçük ikonlar ve dönen
//! gösterge kareleri. Hepsi burada kurulan SVG'lerden çizilir; renkler [`BootColors`]'tan.

use crate::colors::{BootColors, Rgb};
use crate::raster::{self, Image, RasterError};

/// Framework'ün renkli marka SVG'si (Kara Sağan; geometri `.team/logo/r3/10-kara-sagan.md`).
/// Kopya yok: tek kaynak framework'teki varlık.
const BRAND_SVG: &str = include_str!("../../../../framework/crates/nuxut-ui/assets/brand/kara-sagan.svg");
/// Renkli SVG'deki sabit roller: gövde tonu ve ön kanat (framework `brand.rs` aynı değerleri belgeler).
const BRAND_SHADE: &str = "#5d8fe8";
const BRAND_PRIMARY: &str = "#9ec5ff";

/// Karosuz iki tonlu logo SVG'si (zemin şeffaf). Grup dönüşümü korunur: işaret karedeki
/// yerini (0,86 ölçek, ortada) tutar, kenarlarda doğal bir boşluk kalır.
pub fn logo_svg(c: &BootColors) -> Result<String, RasterError> {
    let mut out = String::with_capacity(BRAND_SVG.len());
    let mut tile = 0;
    for line in BRAND_SVG.lines() {
        if line.trim_start().starts_with("<rect") {
            tile += 1;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    let shade = out.matches(BRAND_SHADE).count();
    let primary = out.matches(BRAND_PRIMARY).count();
    // Framework varlığı değişirse sessizce yanlış renkli logo üretmeyelim.
    if tile != 1 || shade != 1 || primary != 1 {
        return Err(RasterError(format!("marka SVG'si beklenen biçimde değil (karo {tile}, gövde {shade}, kanat {primary})")));
    }
    Ok(out.replace(BRAND_SHADE, &c.logo_shade.css()).replace(BRAND_PRIMARY, &c.primary.css()))
}

/// Logo PNG'si (`side` piksel kare).
pub fn logo_png(c: &BootColors, side: u32) -> Result<Vec<u8>, RasterError> {
    raster::svg_png(&logo_svg(c)?, side, side)
}

fn svg(w: f32, h: f32, body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">{body}</svg>"#)
}

/// Yuvarlatılmış dikdörtgen (isteğe bağlı 1 px iç kenar çizgisi).
pub fn rounded_rect_svg(w: u32, h: u32, radius: f32, fill: Rgb, stroke: Option<Rgb>) -> String {
    let (wf, hf) = (w as f32, h as f32);
    let r = radius.min(wf / 2.0).min(hf / 2.0);
    let mut body = format!(r#"<rect width="{wf}" height="{hf}" rx="{r}" fill="{}"/>"#, fill.css());
    if let Some(s) = stroke {
        body.push_str(&format!(
            r#"<rect x="0.5" y="0.5" width="{}" height="{}" rx="{}" fill="none" stroke="{}" stroke-width="1"/>"#,
            wf - 1.0,
            hf - 1.0,
            (r - 0.5).max(0.0),
            s.css()
        ));
    }
    svg(wf, hf, &body)
}

/// GRUB'un 9 parçalı kutu biçimi (`<önek>_{nw,n,ne,w,c,e,sw,s,se}.png`): köşeler `radius`
/// kenarlı kare, kenarlar ve orta 1 px (GRUB onları gerer). Dönen: `(sonek, PNG)`.
pub fn nine_slice(radius: u32, fill: Rgb, stroke: Option<Rgb>) -> Result<Vec<(&'static str, Vec<u8>)>, RasterError> {
    nine_slice_padded(radius, radius, fill, stroke)
}

/// [`nine_slice`], köşe/yan parçaları `hpad` genişliğinde (`hpad ≥ radius`; köşenin yuvarlağı
/// dış `radius` pikselde, kalanı düz dolgu). GRUB seçili kutuyu içeriğin dışına parçaların boyu
/// kadar büyütür: yatay pay metni hap kenarından içeri alır (2026-10-17, QEMU'da gerçek GRUB'la
/// ölçüldü).
pub fn nine_slice_padded(radius: u32, hpad: u32, fill: Rgb, stroke: Option<Rgb>) -> Result<Vec<(&'static str, Vec<u8>)>, RasterError> {
    let r = radius.max(1);
    let p = hpad.max(r);
    let (w, h) = (2 * p + 1, 2 * r + 1);
    let img = raster::to_image(&raster::render_svg(&rounded_rect_svg(w, h, r as f32, fill, stroke), w, h)?);
    let parts: [(&str, u32, u32, u32, u32); 9] = [
        ("nw", 0, 0, p, r),
        ("n", p, 0, 1, r),
        ("ne", p + 1, 0, p, r),
        ("w", 0, r, p, 1),
        ("c", p, r, 1, 1),
        ("e", p + 1, r, p, 1),
        ("sw", 0, r + 1, p, r),
        ("s", p, r + 1, 1, r),
        ("se", p + 1, r + 1, p, r),
    ];
    parts.iter().map(|&(name, x, y, w, h)| Ok((name, raster::encode_png(&raster::crop(&img, x, y, w, h))?))).collect()
}

/// Parola alanı zemini (Plymouth `entry.png`): kap rengi, ince çerçeve.
pub fn svg_entry(w: u32, h: u32, c: &BootColors) -> Result<Vec<u8>, RasterError> {
    raster::svg_png(&rounded_rect_svg(w, h, 12.0, c.surface_container_highest, Some(c.outline_variant)), w, h)
}

/// Dolu daire (parola noktası).
pub fn dot_png(side: u32, fill: Rgb) -> Result<Vec<u8>, RasterError> {
    let s = side as f32;
    raster::svg_png(&svg(s, s, &format!(r#"<circle cx="{}" cy="{}" r="{}" fill="{}"/>"#, s / 2.0, s / 2.0, s / 2.0, fill.css())), side, side)
}

/// Kilit ikonu (24 birimlik ızgara, `side` piksel).
pub fn lock_png(side: u32, color: Rgb) -> Result<Vec<u8>, RasterError> {
    let c = color.css();
    let body = format!(
        r#"<path d="M8 10.5V8a4 4 0 0 1 8 0v2.5" fill="none" stroke="{c}" stroke-width="2" stroke-linecap="round"/><rect x="5" y="10" width="14" height="11" rx="2.5" fill="{c}"/>"#
    );
    raster::svg_png(&svg(24.0, 24.0, &body), side, side)
}

/// Büyük harf kilidi uyarısı (yukarı ok + çizgi).
pub fn capslock_png(side: u32, color: Rgb) -> Result<Vec<u8>, RasterError> {
    let c = color.css();
    let body = format!(
        r#"<path d="M12 3.5 4.5 11.5H9V16h6v-4.5h4.5z" fill="none" stroke="{c}" stroke-width="1.8" stroke-linejoin="round"/><rect x="9" y="18.5" width="6" height="2" rx="1" fill="{c}"/>"#
    );
    raster::svg_png(&svg(24.0, 24.0, &body), side, side)
}

/// Klavye düzeni ikonu.
pub fn keyboard_png(side: u32, color: Rgb) -> Result<Vec<u8>, RasterError> {
    let c = color.css();
    let mut body = format!(r#"<rect x="2.5" y="6" width="19" height="12" rx="2.5" fill="none" stroke="{c}" stroke-width="1.6"/>"#);
    for (x, y) in [(6.0, 9.5), (9.5, 9.5), (13.0, 9.5), (16.5, 9.5)] {
        body.push_str(&format!(r#"<rect x="{x}" y="{y}" width="1.6" height="1.6" fill="{c}"/>"#));
    }
    body.push_str(&format!(r#"<rect x="8" y="13.5" width="8" height="1.6" rx=".8" fill="{c}"/>"#));
    raster::svg_png(&svg(24.0, 24.0, &body), side, side)
}

/// Dönen göstergenin `frame`/`frames` karesi: soluk iz halkası üstünde 100°'lik yay.
/// Ölçülü: tek renk, sabit hız, parlama/iz yok.
pub fn throbber_png(side: u32, frame: u32, frames: u32, arc: Rgb, track: Rgb) -> Result<Vec<u8>, RasterError> {
    let s = side as f32;
    let stroke = (s / 10.0).max(2.0);
    let r = (s - stroke) / 2.0;
    let (cx, cy) = (s / 2.0, s / 2.0);
    let start = 360.0 * frame as f32 / frames.max(1) as f32 - 90.0;
    let end = start + 100.0;
    let pt = |deg: f32| {
        let rad = deg.to_radians();
        (cx + r * rad.cos(), cy + r * rad.sin())
    };
    let (x0, y0) = pt(start);
    let (x1, y1) = pt(end);
    let body = format!(
        r#"<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{}" stroke-width="{stroke}"/><path d="M{x0} {y0}A{r} {r} 0 0 1 {x1} {y1}" fill="none" stroke="{}" stroke-width="{stroke}" stroke-linecap="round"/>"#,
        track.css(),
        arc.css()
    );
    raster::svg_png(&svg(s, s, &body), side, side)
}

/// Düz renk dolu görüntü (önizleme zemini vb.).
pub fn solid(w: u32, h: u32, c: Rgb) -> Image {
    Image { width: w, height: h, rgba: [c.0, c.1, c.2, 255].repeat((w * h) as usize) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::decode_png;

    #[test]
    fn logo_is_recolored_without_tile() {
        let mut c = BootColors::fallback();
        c.primary = Rgb(0x11, 0x22, 0x33);
        c.logo_shade = Rgb(0x44, 0x55, 0x66);
        let s = logo_svg(&c).unwrap();
        assert!(!s.contains("<rect"));
        assert!(s.contains("#112233") && s.contains("#445566"));
        assert!(!s.contains(BRAND_PRIMARY) && !s.contains(BRAND_SHADE));
        let img = decode_png(&logo_png(&c, 64).unwrap()).unwrap();
        // Köşe şeffaf (karo yok), ortada kuşun bir kısmı boyalı.
        assert_eq!(img.rgba[3], 0);
        assert!(img.rgba.chunks_exact(4).any(|p| p[3] == 255 && p[0] == 0x11 && p[1] == 0x22));
    }

    #[test]
    fn nine_slice_parts_have_expected_sizes() {
        let parts = nine_slice(6, Rgb(10, 20, 30), Some(Rgb(200, 200, 200))).unwrap();
        assert_eq!(parts.len(), 9);
        for (name, png) in parts {
            let img = decode_png(&png).unwrap();
            let want = match name {
                "nw" | "ne" | "sw" | "se" => (6, 6),
                "n" | "s" => (1, 6),
                "w" | "e" => (6, 1),
                "c" => (1, 1),
                _ => unreachable!(),
            };
            assert_eq!((img.width, img.height), want, "{name}");
            if name == "c" {
                assert_eq!(img.rgba, vec![10, 20, 30, 255]);
            }
            if name == "nw" {
                assert_eq!(img.rgba[3], 0, "köşe yuvarlak: sol üst piksel şeffaf");
            }
        }
    }

    #[test]
    fn icons_and_frames_render() {
        let c = BootColors::fallback();
        for png in [
            lock_png(24, c.on_surface).unwrap(),
            capslock_png(24, c.error).unwrap(),
            keyboard_png(24, c.on_surface).unwrap(),
            dot_png(8, c.on_surface).unwrap(),
            throbber_png(28, 3, 24, c.primary, c.surface_container_highest).unwrap(),
        ] {
            let img = decode_png(&png).unwrap();
            assert!(img.rgba.chunks_exact(4).any(|p| p[3] > 200), "boş ikon");
        }
        // Kareler birbirinden farklı (dönüyor).
        assert_ne!(throbber_png(28, 0, 24, c.primary, c.surface).unwrap(), throbber_png(28, 6, 24, c.primary, c.surface).unwrap());
    }
}
