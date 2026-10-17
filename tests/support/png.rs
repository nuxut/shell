//! Test fikstürü: kodlayıcı bağımlılığı olmadan geçerli PNG. 1 bit gri (siyah) ve
//! sıkıştırmasız deflate blokları: 4000×4000 bile ≈ 2 MB (güvenilmeyen kaynaklar artık
//! yalnızca raster çözdüğü için SVG fikstürlerinin yerini alır).

// Her test ikilisi yalnızca ihtiyacı olanı kullanır.
#![allow(dead_code)]

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Sıkıştırmasız zlib akışı (en fazla 65535 baytlık stored bloklar).
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    if blocks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(block) = blocks.next() {
        out.push(u8::from(blocks.peek().is_none()));
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in raw {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

/// `width × height` siyah PNG.
pub fn png(width: u32, height: u32) -> Vec<u8> {
    let row = (width as usize).div_ceil(8);
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for _ in 0..height {
        raw.push(0); // filtre: yok
        raw.resize(raw.len() + row, 0);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[1, 0, 0, 0, 0]); // 1 bit, gri
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// `width × height`, tek renk 8 bit RGBA PNG (renk denetimi gereken küçük ikonlar için).
pub fn solid_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    // Satır: filtre baytı (0 = yok) + pikseller.
    let row = [&[0u8][..], &rgba.repeat(width as usize)].concat();
    let raw = row.repeat(height as usize);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bit, RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}
