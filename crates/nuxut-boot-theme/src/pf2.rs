//! GRUB PF2 font okuyucu (yalnız önizleme): GRUB'un çizeceği glifleri birebir görmek için
//! `grub-mkfont` çıktısını okur. Biçim (GRUB `font.c`): `FILE`/`NAME`/…/`CHIX`/`DATA` bölümleri,
//! her biri 4 bayt ad + 4 bayt büyük-endian uzunluk; `CHIX` girdisi 9 bayt (kod noktası u32,
//! saklama bayrağı u8, glif ofseti u32); glif: genişlik, yükseklik (u16), x/y ofseti ve ilerleme
//! (i16), ardından satır satır, MSB önce, dolgusuz 1 bit/piksel.
//!
//! Girdi kendi ürettiğimiz dosya olsa da her erişim sınır denetimli; bozuk dosya panik değil hata.

/// Okuma hatası.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pf2Error(pub &'static str);

impl std::fmt::Display for Pf2Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PF2 okunamadı: {}", self.0)
    }
}

impl std::error::Error for Pf2Error {}

/// En çok glif (Unicode düzlemi 0 yeter; grub-mkfont tam font için ~3–4 bin).
const MAX_CHARS: usize = 65_536;
const MAX_FILE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Font {
    data: Vec<u8>,
    pub name: String,
    pub ascent: i32,
    pub descent: i32,
    index: Vec<(u32, usize)>,
}

/// Bir glif (bitmap `data` içinde).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph<'a> {
    pub width: u32,
    pub height: u32,
    pub x_offset: i32,
    /// Taban çizgisinden bitmap'in alt kenarına (yukarı pozitif).
    pub y_offset: i32,
    pub advance: i32,
    pub bits: &'a [u8],
}

impl Glyph<'_> {
    pub fn pixel(&self, x: u32, y: u32) -> bool {
        let i = (y * self.width + x) as usize;
        self.bits.get(i / 8).is_some_and(|b| b & (0x80 >> (i % 8)) != 0)
    }
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

impl Font {
    pub fn parse(data: Vec<u8>) -> Result<Self, Pf2Error> {
        if data.len() > MAX_FILE {
            return Err(Pf2Error("çok büyük"));
        }
        let mut pos = 0;
        let mut name = String::new();
        let (mut ascent, mut descent) = (0, 0);
        let mut index = Vec::new();
        let mut first = true;
        while pos + 8 <= data.len() {
            let tag = &data[pos..pos + 4];
            let len = be32(&data, pos + 4).ok_or(Pf2Error("bölüm"))? as usize;
            let body = pos + 8;
            if tag == b"DATA" {
                break;
            }
            let end = body.checked_add(len).filter(|e| *e <= data.len()).ok_or(Pf2Error("bölüm taşıyor"))?;
            let sect = &data[body..end];
            match tag {
                b"FILE" if first && sect == b"PFF2" => {}
                _ if first => return Err(Pf2Error("PFF2 imzası yok")),
                b"NAME" => name = String::from_utf8_lossy(sect.split(|b| *b == 0).next().unwrap_or(&[])).into_owned(),
                b"ASCE" => ascent = i32::from(be16(sect, 0).ok_or(Pf2Error("ASCE"))?),
                b"DESC" => descent = i32::from(be16(sect, 0).ok_or(Pf2Error("DESC"))?),
                b"CHIX" => {
                    if !len.is_multiple_of(9) || len / 9 > MAX_CHARS {
                        return Err(Pf2Error("CHIX"));
                    }
                    for e in sect.chunks_exact(9) {
                        let cp = u32::from_be_bytes([e[0], e[1], e[2], e[3]]);
                        let off = u32::from_be_bytes([e[5], e[6], e[7], e[8]]) as usize;
                        index.push((cp, off));
                    }
                }
                _ => {}
            }
            first = false;
            pos = end;
        }
        if first || index.is_empty() {
            return Err(Pf2Error("glif dizini yok"));
        }
        index.sort_unstable_by_key(|(cp, _)| *cp);
        Ok(Self { data, name, ascent, descent, index })
    }

    pub fn glyph(&self, ch: char) -> Option<Glyph<'_>> {
        let i = self.index.binary_search_by_key(&(ch as u32), |(cp, _)| *cp).ok()?;
        let off = self.index[i].1;
        let d = &self.data;
        let width = u32::from(be16(d, off)?);
        let height = u32::from(be16(d, off + 2)?);
        let x_offset = i32::from(be16(d, off + 4)? as i16);
        let y_offset = i32::from(be16(d, off + 6)? as i16);
        let advance = i32::from(be16(d, off + 8)? as i16);
        let n = (width as usize * height as usize).div_ceil(8);
        let bits = d.get(off + 10..off + 10 + n)?;
        Some(Glyph { width, height, x_offset, y_offset, advance, bits })
    }

    /// Metnin piksel genişliği (bilinmeyen karakter `?` ile).
    pub fn measure(&self, text: &str) -> i32 {
        text.chars().filter_map(|c| self.glyph(c).or_else(|| self.glyph('?'))).map(|g| g.advance).sum()
    }

    pub fn height(&self) -> i32 {
        self.ascent + self.descent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Elle kurulmuş küçük bir PF2: 'A' 2×2 dolu, ilerleme 3.
    fn tiny() -> Vec<u8> {
        let mut v = Vec::new();
        let sect = |v: &mut Vec<u8>, tag: &[u8], body: &[u8]| {
            v.extend_from_slice(tag);
            v.extend_from_slice(&(body.len() as u32).to_be_bytes());
            v.extend_from_slice(body);
        };
        sect(&mut v, b"FILE", b"PFF2");
        sect(&mut v, b"NAME", b"Nuxut Regular 2\0");
        sect(&mut v, b"ASCE", &2u16.to_be_bytes());
        sect(&mut v, b"DESC", &1u16.to_be_bytes());
        let chix_at = v.len();
        let glyph_at = chix_at + 8 + 9 + 8;
        let mut chix = Vec::new();
        chix.extend_from_slice(&('A' as u32).to_be_bytes());
        chix.push(0);
        chix.extend_from_slice(&(glyph_at as u32).to_be_bytes());
        sect(&mut v, b"CHIX", &chix);
        v.extend_from_slice(b"DATA");
        v.extend_from_slice(&u32::MAX.to_be_bytes());
        for x in [2u16, 2, 0, 0, 3] {
            v.extend_from_slice(&x.to_be_bytes());
        }
        v.push(0b1111_0000);
        v
    }

    #[test]
    fn parses_tiny_font() {
        let f = Font::parse(tiny()).unwrap();
        assert_eq!(f.name, "Nuxut Regular 2");
        assert_eq!((f.ascent, f.descent), (2, 1));
        let g = f.glyph('A').unwrap();
        assert_eq!((g.width, g.height, g.advance), (2, 2, 3));
        assert!(g.pixel(0, 0) && g.pixel(1, 1));
        assert!(f.glyph('B').is_none());
        assert_eq!(f.measure("AA"), 6);
    }

    #[test]
    fn rejects_garbage_without_panic() {
        assert!(Font::parse(vec![]).is_err());
        assert!(Font::parse(b"FILE\0\0\0\x04XXXX".to_vec()).is_err());
        let mut t = tiny();
        let n = t.len();
        t.truncate(n - 3); // glif kesik
        let f = Font::parse(t).unwrap();
        assert!(f.glyph('A').is_none());
        let mut bad = tiny();
        bad[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Font::parse(bad).is_err());
    }

    #[test]
    fn reads_real_grub_font_if_present() {
        // GRUB kuruluysa paketle gelen unicode.pf2 okunabilmeli (biçim uyumu).
        let Ok(bytes) = std::fs::read("/usr/share/grub/unicode.pf2") else { return };
        let f = Font::parse(bytes).unwrap();
        assert!(f.glyph('A').is_some_and(|g| g.advance > 0));
        assert!(f.measure("Nuxut") > 0);
    }
}
