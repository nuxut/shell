//! Sistem dosyasına dokunmadan önizleme: üretilen tema dosyalarını (aynı PNG'ler, aynı PF2
//! fontlar, aynı düzen sabitleri) GRUB / Plymouth two-step / Limine'nin yerleşim kurallarına göre
//! bir tuvale çizer.
//!
//! **Sınır:** bu bir yeniden çizimdir, önyükleyicinin kendisi değil. Yerleşim kuralları
//! (hizalama formülleri, 9 parçalı kutu, metin taban çizgisi) belgelerden ve kaynak koddan
//! alınmıştır; piksel düzeyinde fark olabilir (ör. Plymouth metni FreeType ile kenar
//! yumuşatmalı çizer, burada PF2 bitmap'iyle çizilir). Gerçek doğrulama VM'de yapılır
//! (`docs/design/boot-theme.md` "Nasıl denenir").

use crate::colors::{BootColors, Rgb};
use crate::files::ThemeFiles;
use crate::pf2::Font;
use crate::raster::{Image, decode_png};
use crate::{grub, plymouth};

/// RGBA tuval (opak zemin üstüne alfa karışımı).
#[derive(Debug, Clone)]
pub struct Canvas {
    pub img: Image,
}

impl Canvas {
    pub fn new(w: u32, h: u32, bg: Rgb) -> Self {
        Self { img: crate::art::solid(w, h, bg) }
    }

    fn blend_px(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.img.width as i32 || y >= self.img.height as i32 || c[3] == 0 {
            return;
        }
        let i = ((y as u32 * self.img.width + x as u32) * 4) as usize;
        let a = u32::from(c[3]);
        for (k, &src) in c.iter().take(3).enumerate() {
            let d = u32::from(self.img.rgba[i + k]);
            self.img.rgba[i + k] = ((u32::from(src) * a + d * (255 - a)) / 255) as u8;
        }
        self.img.rgba[i + 3] = 255;
    }

    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.blend_px(xx, yy, [c.0, c.1, c.2, 255]);
            }
        }
    }

    /// Görüntüyü olduğu gibi (ölçeksiz) yerleştirir.
    pub fn blit(&mut self, src: &Image, x: i32, y: i32) {
        self.blit_scaled(src, x, y, src.width, src.height);
    }

    /// En yakın komşu ölçekleme ile (9 parçalı kutunun kenar/orta parçaları için).
    pub fn blit_scaled(&mut self, src: &Image, x: i32, y: i32, w: u32, h: u32) {
        if src.width == 0 || src.height == 0 {
            return;
        }
        for yy in 0..h {
            for xx in 0..w {
                let sx = xx * src.width / w.max(1);
                let sy = yy * src.height / h.max(1);
                let i = ((sy * src.width + sx) * 4) as usize;
                let p = &src.rgba[i..i + 4];
                self.blend_px(x + xx as i32, y + yy as i32, [p[0], p[1], p[2], p[3]]);
            }
        }
    }

    /// GRUB biçimi 9 parçalı kutu: köşeler doğal boyutta, kenarlar ve orta gerilir.
    pub fn nine(&mut self, parts: &NineParts, x: i32, y: i32, w: u32, h: u32) {
        let (l, t) = (parts.nw.width, parts.nw.height);
        let (r, b) = (parts.se.width, parts.se.height);
        let cw = w.saturating_sub(l + r);
        let ch = h.saturating_sub(t + b);
        let (xi, yi) = (x + l as i32, y + t as i32);
        let (xr, yb) = (xi + cw as i32, yi + ch as i32);
        self.blit(&parts.nw, x, y);
        self.blit_scaled(&parts.n, xi, y, cw, t);
        self.blit(&parts.ne, xr, y);
        self.blit_scaled(&parts.w, x, yi, l, ch);
        self.blit_scaled(&parts.c, xi, yi, cw, ch);
        self.blit_scaled(&parts.e, xr, yi, r, ch);
        self.blit(&parts.sw, x, yb);
        self.blit_scaled(&parts.s, xi, yb, cw, b);
        self.blit(&parts.se, xr, yb);
    }

    /// PF2 metni; `baseline` taban çizgisi y'si. Bilinmeyen karakter `?`.
    pub fn text(&mut self, font: &Font, x: i32, baseline: i32, s: &str, c: Rgb) {
        let mut pen = x;
        for ch in s.chars() {
            let Some(g) = font.glyph(ch).or_else(|| font.glyph('?')) else { continue };
            let top = baseline - g.y_offset - g.height as i32;
            for yy in 0..g.height {
                for xx in 0..g.width {
                    if g.pixel(xx, yy) {
                        self.blend_px(pen + g.x_offset + xx as i32, top + yy as i32, [c.0, c.1, c.2, 255]);
                    }
                }
            }
            pen += g.advance;
        }
    }

    pub fn text_centered(&mut self, font: &Font, cx: i32, baseline: i32, s: &str, c: Rgb) {
        let w = font.measure(s);
        self.text(font, cx - w / 2, baseline, s, c);
    }
}

/// 9 parçalı kutunun çözülmüş parçaları.
#[derive(Debug, Clone)]
pub struct NineParts {
    pub nw: Image,
    pub n: Image,
    pub ne: Image,
    pub w: Image,
    pub c: Image,
    pub e: Image,
    pub sw: Image,
    pub s: Image,
    pub se: Image,
}

impl NineParts {
    pub fn load(files: &ThemeFiles, prefix: &str) -> Option<Self> {
        let get = |p: &str| files.get(&format!("{prefix}_{p}.png")).and_then(|b| decode_png(b).ok());
        Some(Self {
            nw: get("nw")?,
            n: get("n")?,
            ne: get("ne")?,
            w: get("w")?,
            c: get("c")?,
            e: get("e")?,
            sw: get("sw")?,
            s: get("s")?,
            se: get("se")?,
        })
    }
}

fn png(files: &ThemeFiles, name: &str) -> Option<Image> {
    files.get(name).and_then(|b| decode_png(b).ok())
}

/// two-step hizalaması: öğenin sol üstü = (ekran − öğe) × hizalama.
fn align(screen: u32, item: u32, a: f32) -> i32 {
    ((screen as f32 - item as f32) * a).round() as i32
}

/// Plymouth sahnesi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlymouthScene {
    /// Açılış: logo + ilerleme çubuğu (oran ‰).
    Boot { progress_permille: u32 },
    /// LUKS parola sorusu (`bullets` nokta yazılmış).
    Password { bullets: u32 },
}

/// Plymouth two-step önizlemesi. `font`: istem metni için (yoksa metin çizilmez).
pub fn plymouth(files: &ThemeFiles, c: &BootColors, w: u32, h: u32, scene: PlymouthScene, prompt: &str, font: Option<&Font>) -> Image {
    let l = plymouth::LAYOUT;
    let mut cv = Canvas::new(w, h, c.surface);
    if let Some(logo) = png(files, "watermark.png") {
        cv.blit(&logo, align(w, logo.width, 0.5), align(h, logo.height, l.logo_y));
    }
    match scene {
        PlymouthScene::Boot { progress_permille } => {
            let x = align(w, l.progress_w, 0.5);
            let y = align(h, l.progress_h, l.progress_y);
            cv.fill(x, y, l.progress_w as i32, l.progress_h as i32, c.surface_container_highest);
            let filled = (l.progress_w * progress_permille.min(1000) / 1000) as i32;
            cv.fill(x, y, filled, l.progress_h as i32, c.primary);
        }
        PlymouthScene::Password { bullets } => {
            let (Some(lock), Some(entry), Some(bullet)) = (png(files, "lock.png"), png(files, "entry.png"), png(files, "bullet.png")) else {
                return cv.img;
            };
            // two-step: kilit + alan yan yana, birlikte hizalanır; istem metni üstte ortalı.
            let gap = 8;
            let dw = lock.width + gap + entry.width;
            let dh = entry.height.max(lock.height);
            let x = align(w, dw, 0.5);
            let y = align(h, dh, l.dialog_y);
            cv.blit(&lock, x, y + (dh as i32 - lock.height as i32) / 2);
            let ex = x + (lock.width + gap) as i32;
            cv.blit(&entry, ex, y);
            let pad = 14;
            for i in 0..bullets.min(24) {
                let bx = ex + pad + (i * (bullet.width + 6)) as i32;
                cv.blit(&bullet, bx, y + (entry.height as i32 - bullet.height as i32) / 2);
            }
            if let Some(f) = font {
                cv.text_centered(f, (w / 2) as i32, y - 14, prompt, c.on_surface);
            }
        }
    }
    cv.img
}

/// GRUB menüsünün fontları (ada göre: `Nuxut Regular 16` …).
#[derive(Debug, Default)]
pub struct GrubFonts {
    pub fonts: Vec<(String, Font)>,
}

impl GrubFonts {
    pub fn get(&self, name: &str) -> Option<&Font> {
        self.fonts.iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }
}

/// GRUB önizlemesinin metinleri: girdiler (ilki seçili), geri sayım ve kısayol satırı.
#[derive(Debug, Clone, Copy)]
pub struct GrubText<'a> {
    pub entries: &'a [&'a str],
    pub timeout: &'a str,
    pub help: &'a str,
}

/// GRUB önizlemesi: ilk giriş seçili, geri sayım ve kısayol satırı.
pub fn grub(files: &ThemeFiles, c: &BootColors, scale: u32, fonts: &GrubFonts, w: u32, h: u32, text: GrubText<'_>) -> Image {
    let GrubText { entries, timeout, help } = text;
    let l = grub::LAYOUT.scaled(scale);
    let mut cv = Canvas::new(w, h, c.surface);
    let cy = (h / 2) as i32;
    let block_top = cy - l.half_block() as i32;
    if let Some(logo) = png(files, "logo.png") {
        cv.blit(&logo, (w / 2) as i32 - (l.logo / 2) as i32, block_top);
    }
    let menu_left = (w / 2) as i32 - (l.menu_w / 2) as i32;
    let menu_top = block_top + (l.logo + l.gap) as i32;
    let item_font = fonts.get(&format!("{} Regular {}", grub::FAMILY, l.item_font));
    let small = fonts.get(&format!("{} Regular {}", grub::FAMILY, l.small_font));
    let sel = NineParts::load(files, "select");
    for (i, title) in entries.iter().take(l.visible_items as usize).enumerate() {
        let top = menu_top + (i as u32 * (l.item_h + l.item_spacing)) as i32;
        let selected = i == 0;
        if selected && let Some(parts) = &sel {
            cv.nine(parts, menu_left, top, l.menu_w, l.item_h);
        }
        if let Some(f) = item_font {
            let pad = l.radius as i32 + l.item_padding as i32;
            let baseline = top + (l.item_h as i32 - f.height()) / 2 + f.ascent;
            cv.text(f, menu_left + pad, baseline, title, if selected { c.on_primary_container } else { c.on_surface });
        }
    }
    if let Some(f) = small {
        let t_top = menu_top + (l.menu_h() + l.timeout_gap) as i32;
        let base = |top: i32| top + (l.label_h as i32 - f.height()) / 2 + f.ascent;
        cv.text_centered(f, (w / 2) as i32, base(t_top), timeout, c.on_surface_variant);
        cv.text_centered(f, (w / 2) as i32, base(h as i32 - l.help_bottom as i32), help, c.on_surface_variant);
    }
    cv.img
}

/// Limine önizlemesi (yalnız renkler: marka, giriş listesi, seçili giriş, yardım satırı).
/// Limine'nin terminal fontu yerine verilen eş aralıklı PF2 kullanılır; yerleşim yaklaşıktır.
pub fn limine(c: &BootColors, w: u32, h: u32, font: Option<&Font>, entries: &[&str], help: &str) -> Image {
    let mut cv = Canvas::new(w, h, c.surface);
    let Some(f) = font else { return cv.img };
    let line = f.height() + 4;
    let cx = (w / 2) as i32;
    cv.text_centered(f, cx, line + f.ascent, "Nuxut", c.primary);
    cv.text_centered(f, cx, 2 * line + f.ascent, help, c.on_surface_variant);
    let col = f.measure("M").max(1);
    let left = cx - 24 * col;
    for (i, e) in entries.iter().enumerate() {
        let top = (4 + i as i32) * line;
        if i == 0 {
            cv.fill(left - col, top, 48 * col, line, c.surface_container_highest);
        }
        cv.text(f, left, top + 2 + f.ascent, e, c.on_surface);
    }
    cv.img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::ThemeOptions;

    fn px(img: &Image, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * img.width + x) * 4) as usize;
        [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]]
    }

    #[test]
    fn plymouth_boot_draws_logo_and_progress() {
        let o = ThemeOptions::sample();
        let files = plymouth::generate(&o).unwrap();
        let c = &o.colors;
        let img = plymouth(&files, c, 800, 600, PlymouthScene::Boot { progress_permille: 500 }, "", None);
        assert_eq!(px(&img, 0, 0), [c.surface.0, c.surface.1, c.surface.2]);
        let l = plymouth::LAYOUT;
        let y = align(600, l.progress_h, l.progress_y) as u32 + 1;
        let x0 = align(800, l.progress_w, 0.5) as u32;
        assert_eq!(px(&img, x0 + 10, y), [c.primary.0, c.primary.1, c.primary.2], "dolu kısım");
        let t = c.surface_container_highest;
        assert_eq!(px(&img, x0 + l.progress_w - 10, y), [t.0, t.1, t.2], "boş iz");
        // Logo bölgesinde zemin dışı piksel var.
        let ly = align(600, l.logo, l.logo_y) as u32;
        let has_logo = (ly..ly + l.logo).any(|y| (336..464).any(|x| px(&img, x, y) != [c.surface.0, c.surface.1, c.surface.2]));
        assert!(has_logo);
    }

    #[test]
    fn plymouth_password_draws_entry() {
        let o = ThemeOptions::sample();
        let files = plymouth::generate(&o).unwrap();
        let img = plymouth(&files, &o.colors, 800, 600, PlymouthScene::Password { bullets: 5 }, "x", None);
        let l = plymouth::LAYOUT;
        let y = align(600, l.entry_h, l.dialog_y) as u32 + l.entry_h / 2;
        let x = 400;
        let e = o.colors.surface_container_highest;
        assert_eq!(px(&img, x + 100, y), [e.0, e.1, e.2], "alan zemini");
    }

    #[test]
    fn grub_selected_item_uses_container() {
        let o = ThemeOptions::sample();
        let files = grub::generate(&o).unwrap();
        let img = grub(&files, &o.colors, 1, &GrubFonts::default(), 1280, 800, GrubText { entries: &["Arch Linux", "Advanced"], timeout: "", help: "" });
        let l = grub::LAYOUT;
        let top = 400 - l.half_block() + l.logo + l.gap;
        let pc = o.colors.primary_container;
        assert_eq!(px(&img, 640, top + l.item_h / 2), [pc.0, pc.1, pc.2]);
        let s = o.colors.surface;
        assert_eq!(px(&img, 640, top + l.item_h + l.item_spacing + l.item_h / 2), [s.0, s.1, s.2], "seçili olmayan zemin");
    }

    #[test]
    fn nine_slice_corners_are_round() {
        let o = ThemeOptions::sample();
        let files = grub::generate(&o).unwrap();
        let parts = NineParts::load(&files, "select").unwrap();
        let mut cv = Canvas::new(100, 50, Rgb(0, 0, 0));
        cv.nine(&parts, 0, 0, 100, 50);
        assert_eq!(px(&cv.img, 0, 0), [0, 0, 0], "köşe yuvarlak");
        let pc = o.colors.primary_container;
        assert_eq!(px(&cv.img, 50, 25), [pc.0, pc.1, pc.2]);
        assert_eq!(px(&cv.img, 99, 25), [pc.0, pc.1, pc.2], "sağ kenar dolu");
    }
}
