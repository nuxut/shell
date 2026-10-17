//! Kendi ürettiğimiz SVG'lerin PNG'ye çizimi ve PNG kodlama/çözme.
//!
//! Girdi SVG'ler bu crate'in kodunda kurulur ya da derlemeye gömülüdür (logo): dış dosya
//! çözülmez. PNG çözme yalnızca önizlemede, yine kendi ürettiğimiz baytlar için kullanılır.

use resvg::{tiny_skia, usvg};

/// Raster hatası (kendi girdimizde olmamalı; testler her varlığı çizer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterError(pub String);

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "görsel üretilemedi: {}", self.0)
    }
}

impl std::error::Error for RasterError {}

/// Düz (ön çarpımsız) RGBA görüntü.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// En büyük kenar (bayt sınırı: 4096² × 4 = 64 MiB; önyükleme varlıkları çok daha küçük).
pub const MAX_SIDE: u32 = 4096;

/// SVG'yi `width`×`height` piksele, viewBox'ı tam kaplayacak biçimde çizer.
pub fn render_svg(svg: &str, width: u32, height: u32) -> Result<tiny_skia::Pixmap, RasterError> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(RasterError(format!("boyut {width}x{height}")));
    }
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).map_err(|e| RasterError(e.to_string()))?;
    let size = tree.size();
    let sx = width as f32 / size.width();
    let sy = height as f32 / size.height();
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or_else(|| RasterError("pixmap".into()))?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(sx, sy), &mut pixmap.as_mut());
    Ok(pixmap)
}

/// Ön çarpımlı pixmap → düz RGBA.
pub fn to_image(pixmap: &tiny_skia::Pixmap) -> Image {
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for p in pixmap.pixels() {
        let c = p.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Image { width: pixmap.width(), height: pixmap.height(), rgba }
}

/// SVG → PNG baytları.
pub fn svg_png(svg: &str, width: u32, height: u32) -> Result<Vec<u8>, RasterError> {
    encode_png(&to_image(&render_svg(svg, width, height)?))
}

/// RGBA → PNG (8 bit, RGBA).
pub fn encode_png(img: &Image) -> Result<Vec<u8>, RasterError> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::High);
        let mut w = enc.write_header().map_err(|e| RasterError(e.to_string()))?;
        w.write_image_data(&img.rgba).map_err(|e| RasterError(e.to_string()))?;
    }
    Ok(out)
}

/// PNG → RGBA (yalnız 8 bit RGBA/RGB; kendi ürettiklerimiz için, boyut sınırlı).
pub fn decode_png(bytes: &[u8]) -> Result<Image, RasterError> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().map_err(|e| RasterError(e.to_string()))?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(RasterError(format!("boyut {w}x{h}")));
    }
    let mut buf = vec![0; reader.output_buffer_size().ok_or_else(|| RasterError("arabellek".into()))?];
    let info = reader.next_frame(&mut buf).map_err(|e| RasterError(e.to_string()))?;
    buf.truncate(info.buffer_size());
    let rgba = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => buf,
        (png::ColorType::Rgb, png::BitDepth::Eight) => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        other => return Err(RasterError(format!("desteklenmeyen PNG biçimi {other:?}"))),
    };
    Ok(Image { width: w, height: h, rgba })
}

/// Görüntünün bir dikdörtgen parçası (sınır dışı kırpılır).
pub fn crop(img: &Image, x: u32, y: u32, w: u32, h: u32) -> Image {
    let w = w.min(img.width.saturating_sub(x));
    let h = h.min(img.height.saturating_sub(y));
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * img.width + x) * 4) as usize;
        rgba.extend_from_slice(&img.rgba[start..start + (w * 4) as usize]);
    }
    Image { width: w, height: h, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 2"><rect width="2" height="2" fill="#ff0000"/></svg>"##;
        let png = svg_png(svg, 4, 2).unwrap();
        let img = decode_png(&png).unwrap();
        assert_eq!((img.width, img.height), (4, 2));
        assert_eq!(&img.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(img.rgba[3 * 4 + 3], 0, "sağ yarı saydam");
    }

    #[test]
    fn rejects_bad_sizes() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"/>"#;
        assert!(render_svg(svg, 0, 1).is_err());
        assert!(render_svg(svg, MAX_SIDE + 1, 1).is_err());
    }

    #[test]
    fn crop_clamps() {
        let img = Image { width: 2, height: 2, rgba: (0..16).collect() };
        let c = crop(&img, 1, 1, 5, 5);
        assert_eq!((c.width, c.height), (1, 1));
        assert_eq!(c.rgba, vec![12, 13, 14, 15]);
    }
}
