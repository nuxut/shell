//! Görüntü ve geometri: RGBA tampon, ölçekli kesim, çoklu çıkış birleştirme.
//!
//! Koordinat kuralı: çıkış düzeni ve seçim **mantıksal** (compositor uzayı); görüntü
//! **fiziksel** piksel. Bir çıkışın ölçeği görüntüden türetilir (`görüntü genişliği / mantıksal
//! genişlik`): kesirli ölçekte (1.5) compositor'ın bildirdiği tamsayı ölçek yanlıştır, grim ise
//! çıkışın gerçek tamponunu verir.

use std::sync::Arc;

/// Düz (straight) alfa RGBA8, satırlar sıkı. grim çıktısı opaktır; birleştirmedeki boşluklar
/// alfa 0'dır (premultiplied ile aynı bayt: doku yüklemesi kopyasız).
#[derive(Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

impl std::fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RgbaImage({}x{})", self.width, self.height)
    }
}

impl RgbaImage {
    /// Boyut ve bayt sayısı tutarlıysa.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        let expected = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        (width > 0 && height > 0 && pixels.len() == expected).then(|| Self { width, height, pixels: pixels.into() })
    }

    /// Şeffaf tuval.
    pub fn transparent(width: u32, height: u32) -> Option<Self> {
        let len = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        Self::new(width, height, vec![0; len])
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2], self.pixels[i + 3]]
    }

    /// Tüm pikseller opak mı (PNG'de alfa kanalı atlanır).
    pub fn is_opaque(&self) -> bool {
        self.pixels.chunks_exact(4).all(|p| p[3] == 255)
    }

    /// Fiziksel dikdörtgenin kopyası (`rect` görüntü içinde olmalı; [`crop_rect`] bunu sağlar).
    pub fn crop(&self, rect: PixelRect) -> Option<RgbaImage> {
        if rect.w == 0 || rect.h == 0 || rect.x.checked_add(rect.w).is_none_or(|r| r > self.width) || rect.y.checked_add(rect.h).is_none_or(|b| b > self.height) {
            return None;
        }
        let stride = self.width as usize * 4;
        let row = rect.w as usize * 4;
        let mut out = Vec::with_capacity(row * rect.h as usize);
        for y in rect.y..rect.y + rect.h {
            let start = y as usize * stride + rect.x as usize * 4;
            out.extend_from_slice(&self.pixels[start..start + row]);
        }
        RgbaImage::new(rect.w, rect.h, out)
    }
}

/// Mantıksal tamsayı dikdörtgen (çıkış düzeni, pencere geometrisi).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogicalRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl LogicalRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.w)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h)
    }

    /// Kesişim (boşsa `None`).
    pub fn intersect(&self, other: &LogicalRect) -> Option<LogicalRect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        (r > x && b > y).then(|| LogicalRect::new(x, y, r.saturating_sub(x), b.saturating_sub(y)))
    }

    /// Kaplayan dikdörtgen.
    pub fn union(&self, other: &LogicalRect) -> LogicalRect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        LogicalRect::new(x, y, self.right().max(other.right()).saturating_sub(x), self.bottom().max(other.bottom()).saturating_sub(y))
    }

    /// `x,y wxh` (grim `-g` biçimi).
    pub fn to_grim(&self) -> String {
        format!("{},{} {}x{}", self.x, self.y, self.w, self.h)
    }
}

/// Fiziksel piksel dikdörtgeni (görüntü içinde).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Çıkışa göre mantıksal seçimi (kayan nokta; çıkışın sol üstü 0,0) görüntünün fiziksel
/// dikdörtgenine çevirir. Ölçek eksen başına görüntüden türetilir; kenarlar ayrı ayrı
/// yuvarlanır (bitişik seçimler üst üste binmez), görüntü sınırına kırpılır. Boşsa `None`.
pub fn crop_rect(selection: (f32, f32, f32, f32), logical: (f32, f32), image: (u32, u32)) -> Option<PixelRect> {
    let (x, y, w, h) = selection;
    let (lw, lh) = logical;
    if ![x, y, w, h, lw, lh].iter().all(|v| v.is_finite()) || lw <= 0.0 || lh <= 0.0 || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let sx = image.0 as f32 / lw;
    let sy = image.1 as f32 / lh;
    let edge = |v: f32, s: f32, max: u32| ((v * s).round().max(0.0) as u32).min(max);
    let x0 = edge(x, sx, image.0);
    let y0 = edge(y, sy, image.1);
    let x1 = edge(x + w, sx, image.0);
    let y1 = edge(y + h, sy, image.1);
    (x1 > x0 && y1 > y0).then(|| PixelRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// Seçimin fiziksel boyutu (etiket için; [`crop_rect`] ile aynı yuvarlama).
pub fn physical_size(selection: (f32, f32, f32, f32), logical: (f32, f32), image: (u32, u32)) -> (u32, u32) {
    crop_rect(selection, logical, image).map_or((0, 0), |r| (r.w, r.h))
}

/// Birleştirmenin bir parçası: çıkışın mantıksal düzen dikdörtgeni ve yakalanan görüntüsü.
#[derive(Debug, Clone)]
pub struct Tile<'a> {
    pub layout: LogicalRect,
    pub image: &'a RgbaImage,
}

/// Birleştirilmiş görüntünün en büyük kenarı (bellek sınırı: 16384² × 4 = 1 GiB'ı aşmasın).
pub const MAX_STITCH_SIDE: u32 = 16384;

/// Çıkışları düzenlerine göre tek görüntüde birleştirir. Tuval ölçeği çıkışların en büyük
/// ölçeğidir (grim'in varsayılanı gibi: hiçbir çıkış ayrıntı kaybetmez); daha düşük ölçekli
/// çıkış çift doğrusal büyütülür. Çıkışların kaplamadığı alan şeffaftır. Boş liste, geçersiz
/// düzen ya da sınırı aşan tuval `None`.
pub fn stitch(tiles: &[Tile<'_>]) -> Option<RgbaImage> {
    let first = tiles.first()?;
    if tiles.iter().any(|t| t.layout.is_empty()) {
        return None;
    }
    if tiles.len() == 1 {
        return Some(first.image.clone());
    }
    let bounds = tiles.iter().skip(1).fold(first.layout, |acc, t| acc.union(&t.layout));
    // Kesirli ölçekte mantıksal boyut yuvarlanmıştır (1280 / 1.5 = 853): oran ölçek protokolünün
    // 1/120 adımına oturtulur, yoksa tuval bir iki piksel kayar.
    let scale = tiles.iter().map(|t| t.image.width as f64 / t.layout.w as f64).fold(0.0_f64, f64::max);
    let scale = (scale * 120.0).round() / 120.0;
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let px = |v: i32| (f64::from(v) * scale).round();
    let (cw, ch) = (px(bounds.w), px(bounds.h));
    if cw < 1.0 || ch < 1.0 || cw > f64::from(MAX_STITCH_SIDE) || ch > f64::from(MAX_STITCH_SIDE) {
        return None;
    }
    let (cw, ch) = (cw as u32, ch as u32);
    let mut canvas = vec![0u8; cw as usize * ch as usize * 4];
    for t in tiles {
        let ox = px(t.layout.x.saturating_sub(bounds.x)) as u32;
        let oy = px(t.layout.y.saturating_sub(bounds.y)) as u32;
        let tw = (px(t.layout.w) as u32).min(cw - ox.min(cw));
        let th = (px(t.layout.h) as u32).min(ch - oy.min(ch));
        if tw == 0 || th == 0 {
            continue;
        }
        let scaled;
        let src = if (t.image.width, t.image.height) == (tw, th) {
            t.image
        } else {
            scaled = resize_bilinear(t.image, tw, th)?;
            &scaled
        };
        let row = tw as usize * 4;
        for y in 0..th as usize {
            let s = y * src.width as usize * 4;
            let d = ((oy as usize + y) * cw as usize + ox as usize) * 4;
            canvas[d..d + row].copy_from_slice(&src.pixels[s..s + row]);
        }
    }
    RgbaImage::new(cw, ch, canvas)
}

/// Çift doğrusal yeniden örnekleme (yalnızca birleştirmede, düşük ölçekli çıkışı büyütmek
/// için; piksel merkezleri hizalı).
pub fn resize_bilinear(src: &RgbaImage, width: u32, height: u32) -> Option<RgbaImage> {
    if width == 0 || height == 0 {
        return None;
    }
    let (sw, sh) = (src.width as usize, src.height as usize);
    let fx = sw as f32 / width as f32;
    let fy = sh as f32 / height as f32;
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let sy = ((y as f32 + 0.5) * fy - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let wy = sy - y0 as f32;
        for x in 0..width as usize {
            let sx = ((x as f32 + 0.5) * fx - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let wx = sx - x0 as f32;
            for c in 0..4 {
                let p = |xx: usize, yy: usize| f32::from(src.pixels[(yy * sw + xx) * 4 + c]);
                let top = p(x0, y0) * (1.0 - wx) + p(x1, y0) * wx;
                let bottom = p(x0, y1) * (1.0 - wx) + p(x1, y1) * wx;
                out.push((top * (1.0 - wy) + bottom * wy).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    RgbaImage::new(width, height, out)
}

/// En-boy korunarak en uzun kenarı `max_side`'a sığan boyut; zaten sığıyorsa aynı boyut
/// (büyütmez). Kısa kenar en az 1 piksel.
pub fn fit_within(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= max_side || max_side == 0 {
        return (width, height);
    }
    let scale = |v: u32| ((u64::from(v) * u64::from(max_side) + u64::from(long) / 2) / u64::from(long)).max(1) as u32;
    (scale(width), scale(height))
}

/// Küçük resim: en uzun kenar `max_side`'ı geçmez ([`fit_within`], [`downscale_area`]).
/// Zaten sığan görüntü kopyalanmadan döner.
pub fn thumbnail(src: &RgbaImage, max_side: u32) -> Option<RgbaImage> {
    let (w, h) = fit_within(src.width, src.height, max_side);
    if (w, h) == (src.width, src.height) {
        return Some(src.clone());
    }
    downscale_area(src, w, h)
}

/// Bir eksende hedef pikselin kapladığı kaynak aralığı ve kesirli örtüşme ağırlıkları
/// (ağırlıkların toplamı 1).
fn area_weights(src_len: u32, dst_len: u32) -> Vec<Vec<(usize, f32)>> {
    let scale = f64::from(src_len) / f64::from(dst_len);
    (0..dst_len)
        .map(|i| {
            let lo = f64::from(i) * scale;
            let hi = (f64::from(i + 1) * scale).min(f64::from(src_len));
            let first = lo.floor() as u32;
            let last = (hi.ceil() as u32).min(src_len);
            (first..last)
                .filter_map(|j| {
                    let overlap = hi.min(f64::from(j + 1)) - lo.max(f64::from(j));
                    (overlap > 1e-9).then_some((j as usize, (overlap / scale) as f32))
                })
                .collect()
        })
        .collect()
}

/// Alan ortalamalı küçültme (kutu süzgeci, kesirli kenar örtüşmesi dahil): her hedef piksel
/// kapladığı kaynak alanın ortalamasıdır; bilinear'ın küçültmedeki örtüşme (aliasing) bozulması
/// olmaz. Renkler alfa ağırlıklıdır (düz alfa: şeffaf pikselin rengi karışmaz). Ayrılabilir iki
/// geçiş (önce yatay, sonra dikey). Hedef kaynaktan büyük ya da 0 ise `None` (büyütme
/// [`resize_bilinear`]'ın işi).
pub fn downscale_area(src: &RgbaImage, width: u32, height: u32) -> Option<RgbaImage> {
    if width == 0 || height == 0 || width > src.width || height > src.height {
        return None;
    }
    let (sw, sh, dw, dh) = (src.width as usize, src.height as usize, width as usize, height as usize);
    let xw = area_weights(src.width, width);
    let yw = area_weights(src.height, height);
    // Yatay geçiş: dw x sh, önçarpılmış (r·a, g·a, b·a, a), a ∈ [0, 1].
    let mut tmp = vec![0f32; dw * sh * 4];
    for y in 0..sh {
        let row = &src.pixels[y * sw * 4..(y + 1) * sw * 4];
        for (x, weights) in xw.iter().enumerate() {
            let mut acc = [0f32; 4];
            for &(sx, w) in weights {
                let p = &row[sx * 4..sx * 4 + 4];
                let a = f32::from(p[3]) / 255.0 * w;
                acc[0] += f32::from(p[0]) * a;
                acc[1] += f32::from(p[1]) * a;
                acc[2] += f32::from(p[2]) * a;
                acc[3] += a;
            }
            tmp[(y * dw + x) * 4..(y * dw + x) * 4 + 4].copy_from_slice(&acc);
        }
    }
    // Dikey geçiş ve önçarpmanın geri alınması.
    let mut out = Vec::with_capacity(dw * dh * 4);
    for weights in &yw {
        for x in 0..dw {
            let mut acc = [0f32; 4];
            for &(sy, w) in weights {
                let p = &tmp[(sy * dw + x) * 4..(sy * dw + x) * 4 + 4];
                for (a, v) in acc.iter_mut().zip(p) {
                    *a += v * w;
                }
            }
            let a = acc[3];
            for v in &acc[..3] {
                out.push(if a > 0.0 { (v / a).round().clamp(0.0, 255.0) as u8 } else { 0 });
            }
            out.push((a * 255.0).round().clamp(0.0, 255.0) as u8);
        }
    }
    RgbaImage::new(width, height, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> RgbaImage {
        RgbaImage::new(w, h, rgba.repeat((w * h) as usize)).unwrap()
    }

    #[test]
    fn image_rejects_bad_sizes() {
        assert!(RgbaImage::new(0, 1, vec![]).is_none());
        assert!(RgbaImage::new(2, 2, vec![0; 15]).is_none());
        assert!(RgbaImage::new(2, 2, vec![0; 16]).is_some());
    }

    #[test]
    fn crop_unscaled_and_scaled() {
        // 1x: mantıksal = fiziksel.
        assert_eq!(crop_rect((10.0, 20.0, 30.0, 40.0), (100.0, 100.0), (100, 100)), Some(PixelRect { x: 10, y: 20, w: 30, h: 40 }));
        // 1.5x: 1920x1080 mantıksal → 2880x1620 fiziksel.
        let r = crop_rect((100.0, 50.0, 201.0, 101.0), (1920.0, 1080.0), (2880, 1620)).unwrap();
        assert_eq!(r, PixelRect { x: 150, y: 75, w: 302, h: 152 }, "kenarlar ayrı yuvarlanır (226.5 → 227)");
        // 2x.
        assert_eq!(crop_rect((1.0, 1.0, 2.0, 2.0), (10.0, 10.0), (20, 20)), Some(PixelRect { x: 2, y: 2, w: 4, h: 4 }));
    }

    #[test]
    fn crop_clamps_to_image_and_rejects_empty() {
        let r = crop_rect((-10.0, -10.0, 30.0, 30.0), (100.0, 100.0), (200, 200)).unwrap();
        assert_eq!(r, PixelRect { x: 0, y: 0, w: 40, h: 40 });
        let r = crop_rect((90.0, 90.0, 50.0, 50.0), (100.0, 100.0), (100, 100)).unwrap();
        assert_eq!(r, PixelRect { x: 90, y: 90, w: 10, h: 10 });
        assert_eq!(crop_rect((10.0, 10.0, 0.0, 5.0), (100.0, 100.0), (100, 100)), None);
        assert_eq!(crop_rect((200.0, 10.0, 5.0, 5.0), (100.0, 100.0), (100, 100)), None, "tamamen dışarıda");
        assert_eq!(crop_rect((10.0, 10.0, f32::NAN, 5.0), (100.0, 100.0), (100, 100)), None);
        assert_eq!(crop_rect((0.0, 0.0, 0.2, 0.2), (100.0, 100.0), (100, 100)), None, "fiziksel 0 piksel");
        assert_eq!(physical_size((0.0, 0.0, 10.0, 10.0), (100.0, 100.0), (150, 150)), (15, 15));
    }

    #[test]
    fn crop_copies_rows() {
        let mut px = Vec::new();
        for y in 0..3u8 {
            for x in 0..4u8 {
                px.extend_from_slice(&[x, y, 0, 255]);
            }
        }
        let img = RgbaImage::new(4, 3, px).unwrap();
        let c = img.crop(PixelRect { x: 1, y: 1, w: 2, h: 2 }).unwrap();
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(c.pixel(0, 0), [1, 1, 0, 255]);
        assert_eq!(c.pixel(1, 1), [2, 2, 0, 255]);
        assert!(img.crop(PixelRect { x: 3, y: 0, w: 2, h: 1 }).is_none());
    }

    #[test]
    fn rect_ops() {
        let a = LogicalRect::new(0, 0, 100, 50);
        let b = LogicalRect::new(50, 25, 100, 100);
        assert_eq!(a.intersect(&b), Some(LogicalRect::new(50, 25, 50, 25)));
        assert_eq!(a.union(&b), LogicalRect::new(0, 0, 150, 125));
        assert_eq!(a.intersect(&LogicalRect::new(100, 0, 10, 10)), None, "kenar teması kesişim değil");
        assert_eq!(LogicalRect::new(-5, 10, 3, 4).to_grim(), "-5,10 3x4");
    }

    #[test]
    fn stitch_side_by_side_same_scale() {
        let a = solid(4, 2, [255, 0, 0, 255]);
        let b = solid(2, 2, [0, 0, 255, 255]);
        let out = stitch(&[
            Tile { layout: LogicalRect::new(0, 0, 4, 2), image: &a },
            Tile { layout: LogicalRect::new(4, 0, 2, 2), image: &b },
        ])
        .unwrap();
        assert_eq!((out.width, out.height), (6, 2));
        assert_eq!(out.pixel(3, 1), [255, 0, 0, 255]);
        assert_eq!(out.pixel(4, 0), [0, 0, 255, 255]);
        assert!(out.is_opaque());
    }

    #[test]
    fn stitch_mixed_scale_uses_largest_and_leaves_gaps_transparent() {
        // Sol: 1x 4x4 mantıksal; sağ: 2x, 2x2 mantıksal (4x4 fiziksel), alt hizalı değil.
        let a = solid(4, 4, [10, 20, 30, 255]);
        let b = solid(4, 4, [200, 100, 50, 255]);
        let out = stitch(&[
            Tile { layout: LogicalRect::new(0, 0, 4, 4), image: &a },
            Tile { layout: LogicalRect::new(4, 0, 2, 2), image: &b },
        ])
        .unwrap();
        // Tuval ölçeği 2: 6x4 mantıksal → 12x8.
        assert_eq!((out.width, out.height), (12, 8));
        assert_eq!(out.pixel(0, 0), [10, 20, 30, 255], "1x çıkış 2x'e büyütüldü");
        assert_eq!(out.pixel(7, 7), [10, 20, 30, 255]);
        assert_eq!(out.pixel(8, 0), [200, 100, 50, 255]);
        assert_eq!(out.pixel(11, 3), [200, 100, 50, 255]);
        assert_eq!(out.pixel(9, 6), [0, 0, 0, 0], "kaplanmayan alan şeffaf");
        assert!(!out.is_opaque());
    }

    #[test]
    fn stitch_fractional_scale_snaps_to_protocol_step() {
        // 1280x720 @1.5 → sway mantıksal 853x480 (yuvarlanmış); yanında 1x 4x480.
        let a = solid(4, 480, [1, 1, 1, 255]);
        let b = solid(1280, 720, [2, 2, 2, 255]);
        let out = stitch(&[
            Tile { layout: LogicalRect::new(0, 0, 4, 480), image: &a },
            Tile { layout: LogicalRect::new(4, 0, 853, 480), image: &b },
        ])
        .unwrap();
        assert_eq!((out.width, out.height), (1286, 720), "ölçek 1.5 (1.50059 değil)");
        assert_eq!(out.pixel(5, 719), [1, 1, 1, 255]);
        assert_eq!(out.pixel(6, 0), [2, 2, 2, 255]);
        assert_eq!(out.pixel(1285, 719), [2, 2, 2, 255]);
    }

    #[test]
    fn stitch_negative_positions_and_edge_cases() {
        let a = solid(2, 2, [1, 1, 1, 255]);
        let b = solid(2, 2, [2, 2, 2, 255]);
        let out = stitch(&[
            Tile { layout: LogicalRect::new(0, 0, 2, 2), image: &a },
            Tile { layout: LogicalRect::new(-2, 2, 2, 2), image: &b },
        ])
        .unwrap();
        assert_eq!((out.width, out.height), (4, 4));
        assert_eq!(out.pixel(2, 0), [1, 1, 1, 255]);
        assert_eq!(out.pixel(0, 3), [2, 2, 2, 255]);
        assert!(stitch(&[]).is_none());
        let one = stitch(&[Tile { layout: LogicalRect::new(5, 5, 2, 2), image: &a }]).unwrap();
        assert_eq!(one, a, "tek çıkış olduğu gibi");
        assert!(stitch(&[Tile { layout: LogicalRect::new(0, 0, 0, 2), image: &a }, Tile { layout: LogicalRect::new(0, 0, 2, 2), image: &b }]).is_none());
        let huge = [
            Tile { layout: LogicalRect::new(0, 0, 2, 2), image: &a },
            Tile { layout: LogicalRect::new(20_000, 0, 2, 2), image: &b },
        ];
        assert!(stitch(&huge).is_none(), "tuval sınırı");
    }

    #[test]
    fn bilinear_resize_keeps_solid_color_and_interpolates() {
        let a = solid(2, 2, [100, 150, 200, 255]);
        let r = resize_bilinear(&a, 5, 3).unwrap();
        assert!(r.pixels.chunks_exact(4).all(|p| p == [100, 150, 200, 255]));
        let grad = RgbaImage::new(2, 1, vec![0, 0, 0, 255, 200, 200, 200, 255]).unwrap();
        let r = resize_bilinear(&grad, 4, 1).unwrap();
        assert_eq!(r.pixel(0, 0)[0], 0);
        assert_eq!(r.pixel(3, 0)[0], 200);
        assert!(r.pixel(1, 0)[0] > 0 && r.pixel(1, 0)[0] < r.pixel(2, 0)[0]);
    }

    #[test]
    fn fit_within_keeps_aspect_and_never_upscales() {
        assert_eq!(fit_within(3840, 2160, 256), (256, 144), "geniş");
        assert_eq!(fit_within(1080, 1920, 256), (144, 256), "uzun");
        assert_eq!(fit_within(256, 100, 256), (256, 100), "tam sınırda");
        assert_eq!(fit_within(40, 30, 256), (40, 30), "küçük büyütülmez");
        assert_eq!(fit_within(10_000, 3, 256), (256, 1), "kısa kenar en az 1");
        let big = solid(1000, 500, [1, 2, 3, 255]);
        let t = thumbnail(&big, 256).unwrap();
        assert_eq!((t.width, t.height), (256, 128));
        let small = solid(20, 10, [1, 2, 3, 255]);
        let t = thumbnail(&small, 256).unwrap();
        assert!(Arc::ptr_eq(&t.pixels, &small.pixels), "sığan görüntü kopyalanmaz");
    }

    #[test]
    fn area_downscale_averages_colors() {
        // Dama tahtası 2x küçültmede tam ortalamaya iner (bilinear burada örtüşürdü).
        let mut px = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                px.extend_from_slice(if (x + y) % 2 == 0 { &[0, 0, 0, 255] } else { &[200, 100, 50, 255] });
            }
        }
        let checker = RgbaImage::new(8, 8, px).unwrap();
        let r = downscale_area(&checker, 4, 4).unwrap();
        assert!(r.pixels.chunks_exact(4).all(|p| p == [100, 50, 25, 255]), "{:?}", &r.pixels[..8]);
        // Kesirli oran (3 → 2): orta piksel iki hedefe yarı yarıya düşer.
        let row = RgbaImage::new(3, 1, vec![0, 0, 0, 255, 90, 90, 90, 255, 180, 180, 180, 255]).unwrap();
        let r = downscale_area(&row, 2, 1).unwrap();
        assert_eq!(r.pixel(0, 0), [30, 30, 30, 255]);
        assert_eq!(r.pixel(1, 0), [150, 150, 150, 255]);
        // Düz renk korunur; şeffaf pikselin rengi karışmaz, alfa ortalanır.
        let s = downscale_area(&solid(7, 5, [10, 20, 30, 255]), 3, 2).unwrap();
        assert!(s.pixels.chunks_exact(4).all(|p| p == [10, 20, 30, 255]));
        let half = RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 0]).unwrap();
        assert_eq!(downscale_area(&half, 1, 1).unwrap().pixel(0, 0), [255, 0, 0, 128]);
        // Büyütme ve sıfır reddedilir.
        assert!(downscale_area(&row, 4, 1).is_none());
        assert!(downscale_area(&row, 0, 1).is_none());
    }
}
