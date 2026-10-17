//! Gömülü SVG → tam fiziksel çözünürlükte opak RGBA (cover, ortadan kırpma) + bant kırıcı
//! dither ([`crate::deband`]).
//!
//! `nuxut_image::render_svg` kullanılmaz: o yol güvenilmeyen SVG içindir (denetim bütçeleri) ve
//! `Cover`'da kırpmadan, kutudan büyük tuval üretir; burada kaynak gömülü ve güvenilir, çıktı ise
//! tam olarak hedef boyuttur (kaydırmalı dönüşümle kırpma; fazladan piksel çizilmez).

use resvg::{tiny_skia, usvg};

use crate::{Builtin, ColorThemeId, Variant};

/// Çıktı kenar ve piksel sınırları (framework SVG yoluyla aynı).
pub const MAX_SIDE: u32 = 16_384;
pub const MAX_PIXELS: u64 = 64_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// Bilinmeyen kimlik ([`render_preview`]).
    Unknown(String),
    /// 0 ya da sınır dışı boyut.
    Size { width: u32, height: u32 },
    /// Gömülü SVG ayrıştırılamadı (testler her SVG'yi doğrular; olmamalı).
    Svg(String),
    /// Render thread'i açılamadı ([`crate::Renderer`]).
    Thread(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "bilinmeyen yerleşik duvar kağıdı: {id}"),
            Self::Size { width, height } => write!(f, "geçersiz render boyutu {width}x{height}"),
            Self::Svg(e) => write!(f, "gömülü SVG ayrıştırılamadı: {e}"),
            Self::Thread(e) => write!(f, "render thread'i açılamadı: {e}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Opak, düz (premultiplied değil; alfa her yerde 255) RGBA8.
#[derive(Clone, PartialEq, Eq)]
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Rendered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rendered").field("width", &self.width).field("height", &self.height).finish_non_exhaustive()
    }
}

/// Nuxut mavisiyle (tasarımdaki renkler) [`render_themed`].
pub fn render(builtin: &Builtin, variant: Variant, width: u32, height: u32) -> Result<Rendered, RenderError> {
    render_themed(builtin, variant, ColorThemeId::Blue, width, height)
}

/// Tam fiziksel boyutta render (cover) + bant kırıcı dither; renkler `theme` renk temasına
/// boyanır ([`Builtin::svg_for`]). Bloklar (1080p'de onlarca ms, 4K'da daha fazla;
/// `docs/design/wallpapers.md` ölçüm): ana thread dışında çağırın.
pub fn render_themed(builtin: &Builtin, variant: Variant, theme: ColorThemeId, width: u32, height: u32) -> Result<Rendered, RenderError> {
    let mut out = rasterize(&builtin.svg_for(variant, theme), width, height)?;
    crate::deband(&mut out.rgba, width, height);
    Ok(out)
}

/// Ayarlar önizlemesi: kimlikle [`render`] (aynı boru hattı; küçük boyutta ucuz).
pub fn render_preview(id: &str, variant: Variant, width: u32, height: u32) -> Result<Rendered, RenderError> {
    render_preview_themed(id, variant, ColorThemeId::Blue, width, height)
}

/// Ayarlar önizlemesi, renk temasına boyanmış ([`render_themed`]).
pub fn render_preview_themed(id: &str, variant: Variant, theme: ColorThemeId, width: u32, height: u32) -> Result<Rendered, RenderError> {
    let builtin = crate::find(id).ok_or_else(|| RenderError::Unknown(id.chars().take(48).collect()))?;
    render_themed(builtin, variant, theme, width, height)
}

/// Yalnızca rasterleştirme, dither yok (ölçüm, testler; ürün [`render`] kullanır).
pub fn rasterize(svg: &[u8], width: u32, height: u32) -> Result<Rendered, RenderError> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(RenderError::Size { width, height });
    }
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).map_err(|e| RenderError::Svg(e.to_string()))?;
    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    let (tw, th) = (width as f32, height as f32);
    // Cover: kısa kenar dolar, taşan kısım ortadan kırpılır (SVG'lerin preserveAspectRatio
    // "xMidYMid slice" niyeti; güvenli bölge tasarım notlarında).
    let s = (tw / sw).max(th / sh);
    let (dx, dy) = ((tw - sw * s) / 2.0, (th - sh * s) / 2.0);
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(RenderError::Size { width, height })?;
    resvg::render(&tree, tiny_skia::Transform::from_row(s, 0.0, 0.0, s, dx, dy), &mut pixmap.as_mut());
    let mut rgba = pixmap.take();
    // Zemin dikdörtgeni tuvali kaplar: alfa her yerde 255 (premultiplied = düz). Kenarda
    // kenar yumuşatma kalıntısı olsa bile duvar kağıdı opaktır.
    for px in rgba.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Ok(Rendered { width, height, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BUILTINS, Variant};

    #[test]
    fn every_svg_parses_and_renders_opaque() {
        for b in &BUILTINS {
            for v in Variant::ALL {
                for (w, h) in [(64, 36), (48, 64), (90, 20)] {
                    let r = render(b, v, w, h).unwrap_or_else(|e| panic!("{} {v:?}: {e}", b.id));
                    assert_eq!((r.width, r.height, r.rgba.len()), (w, h, (w * h * 4) as usize));
                    // Gerçekten çizildi: düz siyah/saydam değil, kipine uygun parlaklık.
                    let mean = r.rgba.chunks_exact(4).map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])).sum::<u32>() / (w * h * 3);
                    match v {
                        Variant::Dark => assert!((5..90).contains(&mean), "{} koyu ort. {mean}", b.id),
                        Variant::Light => assert!(mean > 180, "{} açık ort. {mean}", b.id),
                    }
                }
            }
        }
    }

    #[test]
    fn every_theme_recolors_every_svg() {
        // Renk teması (docs/design/color-themes.md): her yerleşik × varyant × tema ayrıştırılır,
        // kipin parlaklığı korunur (ton aynen), Nuxut mavisinde baytlar değişmez.
        for b in &BUILTINS {
            for v in Variant::ALL {
                assert!(matches!(b.svg_for(v, ColorThemeId::Blue), std::borrow::Cow::Borrowed(_)));
                let blue = render(b, v, 48, 27).unwrap();
                let mean = |r: &Rendered| r.rgba.chunks_exact(4).map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])).sum::<u32>() / (48 * 27 * 3);
                for t in ColorThemeId::ALL {
                    let r = render_themed(b, v, t, 48, 27).unwrap_or_else(|e| panic!("{} {v:?} {}: {e}", b.id, t.name()));
                    let (m, mb) = (mean(&r), mean(&blue));
                    assert!(m.abs_diff(mb) <= 12, "{} {v:?} {}: parlaklık {m} / {mb}", b.id, t.name());
                    let svg = b.svg_for(v, t);
                    assert_eq!(svg.iter().filter(|c| **c == b'<').count(), b.svg(v).iter().filter(|c| **c == b'<').count());
                }
            }
        }
    }

    #[test]
    fn svgs_use_only_supported_features() {
        // resvg'nin metin/raster özellikleri kapalı; filtre/maske de tasarım sözleşmesi dışı.
        for b in &BUILTINS {
            for v in Variant::ALL {
                let s = std::str::from_utf8(b.svg(v)).unwrap();
                for bad in ["<image", "<text", "<filter", "filter=", "<mask", "<foreignObject", "<script", "href=\"http", "href=\"data:"] {
                    assert!(!s.contains(bad), "{} {v:?}: {bad}", b.id);
                }
            }
        }
    }

    #[test]
    fn mascots_are_drawn() {
        // Maskot gövdesi (#5d8fe8) `<use>` ile yerleştiriliyor: render'da görünmeli (iki kipte aynı renk).
        for id in ["cloud-rest", "far-glide", "chase"] {
            let b = crate::find(id).unwrap();
            for v in Variant::ALL {
                let r = rasterize(b.svg(v), 1280, 720).unwrap();
                let body = r.rgba.chunks_exact(4).filter(|p| p[0].abs_diff(0x5d) < 4 && p[1].abs_diff(0x8f) < 4 && p[2].abs_diff(0xe8) < 4).count();
                assert!(body > 50, "{id} {v:?}: gövde pikseli {body}");
            }
        }
    }

    #[test]
    fn cover_crops_center_without_stretch() {
        // Aynı SVG 16:9 ve 32:9'da: geniş tuvalde merkez satırın ortası 16:9'unkine yakın
        // (ölçek kısa kenardan değil uzun kenardan; ortalı kırpma).
        let b = crate::find("silhouette").unwrap();
        let a = rasterize(b.svg(Variant::Dark), 160, 90).unwrap();
        let wide = rasterize(b.svg(Variant::Dark), 320, 90).unwrap();
        assert_eq!(wide.width, 320);
        let px = |r: &Rendered, x: u32, y: u32| {
            let i = ((y * r.width + x) * 4) as usize;
            [r.rgba[i], r.rgba[i + 1], r.rgba[i + 2]]
        };
        // 320x90 cover'da ölçek 320/2560: 160x90'ın iki katı genişlik → üst/alt kırpılır.
        // Yalnızca boyutun ve opaklığın doğru olduğunu, bozulmadığını denetle:
        assert!(px(&wide, 160, 45).iter().zip(px(&a, 80, 45)).all(|(x, y)| x.abs_diff(y) < 40));
    }

    #[test]
    fn rejects_bad_sizes() {
        let svg = BUILTINS[0].svg(Variant::Dark);
        assert!(matches!(rasterize(svg, 0, 10), Err(RenderError::Size { .. })));
        assert!(matches!(rasterize(svg, MAX_SIDE + 1, 10), Err(RenderError::Size { .. })));
        assert!(matches!(render_preview("yok", Variant::Dark, 8, 8), Err(RenderError::Unknown(_))));
        assert_eq!(render_preview("aurora", Variant::Light, 32, 18).unwrap().width, 32);
    }
}
