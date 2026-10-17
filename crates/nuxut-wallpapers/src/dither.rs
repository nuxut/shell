//! Bant kırıcı: eşikli kutu bulanıklığı + 8×8 Bayer sıralı dither.
//!
//! `tiny-skia` gradyanları doğrudan 8 bite yuvarlar; geniş, yumuşak ışık alanlarında (aurora,
//! özellikle açık sürüm) 1 LSB'lik basamaklar kontur halkası olarak görünür. 8 bit sonuçtan
//! basamağı geri kazanmak için her pikselin `r` yarıçaplı kutu ortalaması kesirli (1/256 LSB)
//! hesaplanır; ortalama özgün değerden kanal başına en çok [`THRESHOLD`] uzaktaysa piksel
//! "yumuşak bölge"dir ve ortalama, sıralı dither ile 8 bite indirilir. Aksi hâlde (kuş kenarı,
//! iz) piksel olduğu gibi kalır. Böylece hiçbir piksel eşikten (+1 LSB dither) fazla değişmez;
//! keskin kenarlar korunur, düz alanlar birebir aynı kalır (ortalama tam sayıysa dither onu
//! değiştirmez).
//!
//! Bellek O(genişlik × yarıçap): satır yatay toplamları halka tamponda tutulur, dikey toplam
//! kayan pencereyle güncellenir (4K'da ~3 MB). Süre O(piksel), yarıçaptan bağımsız.

/// Kanal başına eşik, 1/256 LSB biriminde (1,5 LSB).
const THRESHOLD: u32 = 384;

/// 8×8 Bayer matrisi (0..64).
const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// Çözünürlüğe göre yarıçap: 1080p'de 15 px (basamak genişliği onlarca piksel), sınırlı.
fn radius(height: u32) -> usize {
    (height as usize / 72).clamp(4, 32)
}

/// Opak RGBA8'i yerinde debant eder (alfa dokunulmaz). Boyut uyuşmazsa bir şey yapmaz.
pub fn deband(rgba: &mut [u8], width: u32, height: u32) {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return;
    }
    let r = radius(height).min(w.max(h));
    let span = 2 * r + 1;
    let n = (span * span) as u32;
    // Halka: pencere satırlarının yatay toplamları (özgün piksellerden; satır y yazılmadan önce
    // y+r+1 okunur, y-r halkadan çıkarılır — yazılmış satır yeniden okunmaz).
    let mut ring = vec![0u32; span * w * 3];
    let mut col = vec![0u32; w * 3];
    let clamp_row = |y: isize| y.clamp(0, h as isize - 1) as usize;
    let hsum = |src: &[u8], y: usize, out: &mut [u32]| {
        let row = &src[y * w * 4..(y + 1) * w * 4];
        let at = |x: isize, c: usize| u32::from(row[x.clamp(0, w as isize - 1) as usize * 4 + c]);
        for c in 0..3 {
            let mut s: u32 = (-(r as isize)..=r as isize).map(|x| at(x, c)).sum();
            for x in 0..w {
                out[x * 3 + c] = s;
                let xi = x as isize;
                s = s + at(xi + r as isize + 1, c) - at(xi - r as isize, c);
            }
        }
    };
    for (slot, yy) in (-(r as isize)..=r as isize).enumerate() {
        let buf = &mut ring[slot * w * 3..(slot + 1) * w * 3];
        hsum(rgba, clamp_row(yy), buf);
        for (c, v) in col.iter_mut().zip(buf.iter()) {
            *c += v;
        }
    }
    // Halkada en eski satırın (y - r) yuvası.
    let mut oldest = 0usize;
    let mut next = vec![0u32; w * 3];
    for y in 0..h {
        // y+r+1'in yatay toplamı, satır y yazılmadan önce (özgün).
        let incoming = clamp_row(y as isize + r as isize + 1);
        hsum(rgba, incoming, &mut next);
        let row = &mut rgba[y * w * 4..(y + 1) * w * 4];
        let bayer = &BAYER[y & 7];
        for x in 0..w {
            let px = &mut row[x * 4..x * 4 + 4];
            let avg = [0, 1, 2].map(|c| col[x * 3 + c] * 256 / n);
            let smooth = (0..3).all(|c| avg[c].abs_diff(u32::from(px[c]) * 256) <= THRESHOLD);
            if smooth {
                let t = u32::from(bayer[x & 7]) * 4 + 2;
                for c in 0..3 {
                    px[c] = ((avg[c] + t) >> 8).min(255) as u8;
                }
            }
        }
        let old = &mut ring[oldest * w * 3..(oldest + 1) * w * 3];
        for ((c, o), nv) in col.iter_mut().zip(old.iter_mut()).zip(next.iter()) {
            *c = *c - *o + nv;
            *o = *nv;
        }
        oldest = (oldest + 1) % span;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = f(x, y);
                v.extend_from_slice(&[r, g, b, 255]);
            }
        }
        v
    }

    #[test]
    fn flat_stays_identical() {
        let src = image(40, 30, |_, _| [17, 200, 3]);
        let mut out = src.clone();
        deband(&mut out, 40, 30);
        assert_eq!(out, src);
    }

    #[test]
    fn sharp_edges_untouched_and_changes_bounded() {
        // Sol yarı 20, sağ yarı 120: kenar korunur; hiçbir piksel 2 LSB'den fazla değişmez.
        let src = image(64, 40, |x, _| if x < 32 { [20; 3] } else { [120; 3] });
        let mut out = src.clone();
        deband(&mut out, 64, 40);
        for (a, b) in src.iter().zip(&out) {
            assert!(a.abs_diff(*b) <= 2);
        }
        assert_eq!(out, src, "keskin kenarlı iki düz alan: değişiklik yok");
    }

    #[test]
    fn staircase_becomes_smooth_ramp() {
        // 1 LSB.lik 10 px basamaklar (bant; yarıçap 4). Dither sonrası 8×8 blok ortalamaları gerçek rampa
        // (x/10) daha yakın olmalı.
        let (w, h) = (400u32, 64u32);
        let src = image(w, h, |x, _| [(100 + x / 10) as u8; 3]);
        let mut out = src.clone();
        deband(&mut out, w, h);
        let err = |img: &[u8]| {
            let mut e = 0.0f64;
            let mut k = 0;
            for bx in (40..w - 40).step_by(8) {
                let mut s = 0.0;
                for y in 16..24 {
                    for x in bx..bx + 8 {
                        s += f64::from(img[((y * w + x) * 4) as usize]);
                    }
                }
                let ideal = 100.0 + (f64::from(bx) + 3.5) / 10.0 - 0.45;
                e += (s / 64.0 - ideal).abs();
                k += 1;
            }
            e / f64::from(k)
        };
        let (before, after) = (err(&src), err(&out));
        assert!(after < before * 0.6, "önce {before:.3}, sonra {after:.3}");
        assert!(src.iter().zip(&out).all(|(a, b)| a.abs_diff(*b) <= 2));
    }

    #[test]
    fn wrong_size_is_noop() {
        let mut v = vec![1u8; 10];
        deband(&mut v, 4, 4);
        assert_eq!(v, vec![1u8; 10]);
    }
}
