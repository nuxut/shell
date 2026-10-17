//! Yerleşik duvar kağıtlarının ölçümü: render, dither, PNG kodlama süresi ve dosya boyutu
//! (1080p, 1440p, 4K); aurora'da bant ölçütü (önce/sonra) ve kontrastı açılmış kırpım PNG'leri.
//!
//! `cargo run --release -p nuxut-wallpapers --example builtin_bench -- <çıktı dizini>`
//!
//! Bant ölçütü: düz bölgelerde (8×8 blokta özgün aralık ≤ 2 LSB) blok ortalamalarının ikinci
//! farkı. Gerçek gradyan doğrusala yakındır (ikinci fark ≈ 0); 8 bit basamaklar her basamakta
//! bir sıçrama üretir. Göz uzaktan blok ortalamasını görür: değer küçüldükçe halka azalır.

use std::path::PathBuf;
use std::time::Instant;

use nuxut_wallpapers::{BUILTINS, Rendered, Variant, deband, find, rasterize};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn luma(img: &Rendered, x: u32, y: u32) -> f64 {
    let i = ((y * img.width + x) * 4) as usize;
    0.2126 * f64::from(img.rgba[i]) + 0.7152 * f64::from(img.rgba[i + 1]) + 0.0722 * f64::from(img.rgba[i + 2])
}

/// (ortalama |ikinci fark|, en büyük |ikinci fark|) — düz bölgelerde, 8×8 bloklarda.
fn banding(raw: &Rendered, img: &Rendered) -> (f64, f64) {
    let (bw, bh) = (img.width / 8, img.height / 8);
    let block = |im: &Rendered, bx: u32, by: u32| {
        let mut s = 0.0;
        for y in by * 8..by * 8 + 8 {
            for x in bx * 8..bx * 8 + 8 {
                s += luma(im, x, y);
            }
        }
        s / 64.0
    };
    let flat = |bx: u32, by: u32| {
        let (mut lo, mut hi) = (255.0f64, 0.0f64);
        for y in by * 8..by * 8 + 8 {
            for x in bx * 8..bx * 8 + 8 {
                let l = luma(raw, x, y);
                lo = lo.min(l);
                hi = hi.max(l);
            }
        }
        hi - lo <= 2.0
    };
    let means: Vec<f64> = (0..bh).flat_map(|by| (0..bw).map(move |bx| (bx, by))).map(|(bx, by)| block(img, bx, by)).collect();
    let m = |bx: u32, by: u32| means[(by * bw + bx) as usize];
    let (mut sum, mut max, mut n) = (0.0, 0.0f64, 0u32);
    for by in 1..bh - 1 {
        for bx in 1..bw - 1 {
            if !(flat(bx - 1, by) && flat(bx, by) && flat(bx + 1, by) && flat(bx, by - 1) && flat(bx, by + 1)) {
                continue;
            }
            let dx = (m(bx - 1, by) - 2.0 * m(bx, by) + m(bx + 1, by)).abs();
            let dy = (m(bx, by - 1) - 2.0 * m(bx, by) + m(bx, by + 1)).abs();
            sum += dx + dy;
            max = max.max(dx).max(dy);
            n += 2;
        }
    }
    (sum / f64::from(n.max(1)), max)
}

/// Kırpım, kontrastı ×`k` açılmış (bant gözle görülsün) PNG olarak.
fn stretched_crop(img: &Rendered, (x0, y0, w, h): (u32, u32, u32, u32), k: f64, out: &PathBuf) {
    let mut lo = [255u8; 3];
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let i = ((y * img.width + x) * 4) as usize;
            for (c, l) in lo.iter_mut().enumerate() {
                *l = (*l).min(img.rgba[i + c]);
            }
        }
    }
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let i = ((y * img.width + x) * 4) as usize;
            for (c, l) in lo.iter().enumerate() {
                rgba.push(((f64::from(img.rgba[i + c] - l)) * k).min(255.0) as u8);
            }
            rgba.push(255);
        }
    }
    let crop = Rendered { width: w, height: h, rgba };
    let f = std::fs::File::create(out).expect("çıktı");
    encode_png(std::io::BufWriter::new(f), &crop).expect("png");
}

fn main() {
    let out: PathBuf = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&out).expect("dizin");
    println!("boyut      id               varyant  render  dither  png     toplam  png-boyut");
    for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
        for b in &BUILTINS {
            for v in Variant::ALL {
                let t = Instant::now();
                let mut img = rasterize(b.svg(v), w, h).expect("render");
                let t_render = ms(t);
                let t = Instant::now();
                deband(&mut img.rgba, w, h);
                let t_dither = ms(t);
                let t = Instant::now();
                let mut buf = Vec::new();
                encode_png(&mut buf, &img).expect("png");
                let t_png = ms(t);
                println!(
                    "{w}x{h:<5} {:<16} {:<8} {t_render:6.1} {t_dither:7.1} {t_png:7.1} {:7.1} {:6.2} MB",
                    b.id,
                    v.name(),
                    t_render + t_dither + t_png,
                    buf.len() as f64 / 1e6
                );
            }
        }
    }
    println!();
    println!("bant ölçütü (1920x1080; düz bölge blok ortalaması ikinci farkı, LSB): önce → sonra");
    for (id, v) in [("aurora", Variant::Light), ("aurora", Variant::Dark), ("pair-flight", Variant::Dark), ("migration", Variant::Dark)] {
        let b = find(id).expect("id");
        let raw = rasterize(b.svg(v), 1920, 1080).expect("render");
        let mut dith = raw.clone();
        deband(&mut dith.rgba, 1920, 1080);
        let (m0, x0) = banding(&raw, &raw);
        let (m1, x1) = banding(&raw, &dith);
        let changed = raw.rgba.iter().zip(&dith.rgba).filter(|(a, b)| a != b).count() as f64 / raw.rgba.len() as f64 * 100.0;
        let maxdiff = raw.rgba.iter().zip(&dith.rgba).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        println!("{id:<12} {:<6} ort {m0:.4} → {m1:.4}   en büyük {x0:.3} → {x1:.3}   değişen kanal %{changed:.1}, en büyük fark {maxdiff}", v.name());
        if id == "aurora" {
            // Sol üst ışık alanının kenarı (2560 tuvalde ~520,260 merkezli elips).
            let rect = (40, 120, 900, 420);
            stretched_crop(&raw, rect, 16.0, &out.join(format!("{id}-{}-once-x16.png", v.name())));
            stretched_crop(&dith, rect, 16.0, &out.join(format!("{id}-{}-sonra-x16.png", v.name())));
            let f = std::fs::File::create(out.join(format!("{id}-{}-sonra.png", v.name()))).expect("çıktı");
            encode_png(std::io::BufWriter::new(f), &dith).expect("png");
        }
    }
}

/// RGB8 PNG, hızlı sıkıştırma (kaldırılan disk önbelleğinin biçimi; "png" sütunu o maliyeti
/// ölçer).
fn encode_png(w: impl std::io::Write, image: &Rendered) -> Result<(), String> {
    let mut enc = png::Encoder::new(w, image.width, image.height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let rgb: Vec<u8> = image.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}
