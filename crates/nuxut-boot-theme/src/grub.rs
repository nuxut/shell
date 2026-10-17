//! GRUB teması (`/boot/grub/themes/nuxut/`) ve `grub-mkconfig` sonrası denetimler.
//!
//! Tema yolu `/etc/default/grub.d/90-nuxut-theme.cfg` damla dosyasıyla verilir: kullanıcının
//! `/etc/default/grub`'ı hiç değişmez (grub-mkconfig damlaları ondan sonra okur; son atama
//! geçerli). Tema `/boot` altında: GRUB kök dosya sistemini (şifreli/btrfs alt birimi) okuyamayabilir,
//! `/boot` her zaman okunur.
//!
//! Fontlar PF2 ister; PF2'yi yardımcı GRUB'un kendi aracıyla (`grub-mkfont`) sistem fontundan
//! üretir — biçim GRUB'un kendi kodlayıcısından çıkar. Ölçek: GRUB çözünürlüğe uyamaz; kullanıcı
//! tarafı en büyük ekrana göre 1–3 seçer, tüm ölçüler onunla çarpılır.

use crate::art;
use crate::files::{Lang, ThemeFiles, ThemeOptions};
use crate::raster::RasterError;

/// Tema dizininin `/boot/grub` altındaki adı.
pub const THEME_SUBDIR: &str = "themes/nuxut";
/// Damla dosyası.
pub const DROPIN_DIR: &str = "/etc/default/grub.d";
pub const DROPIN_NAME: &str = "90-nuxut-theme.cfg";
/// PF2 aile adları (`grub-mkfont -n`); GRUB adı `<aile> Regular <boyut>` kurar.
pub const FAMILY: &str = "Nuxut";
pub const MONO_FAMILY: &str = "Nuxut Mono";
/// `grub-mkfont -r` aralıkları: Latin + Türkçe harfler, noktalama, oklar, kutu çizgileri (GRUB
/// düzenleyici/komut satırı çerçevesi; yoksa `?` kutuları görünür — QEMU, 2026-10-17), üçgenler.
pub const PF2_RANGES: &str = "0x20-0x7E,0xA0-0x24F,0x2010-0x2027,0x2030-0x205E,0x2190-0x21FF,0x2500-0x257F,0x25B2-0x25BC";

/// Bir PF2 fontu: dosya adı, aile ve piksel boyutu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontSpec {
    pub file: String,
    pub family: &'static str,
    pub mono: bool,
    pub size: u32,
}

impl FontSpec {
    /// theme.txt'deki ad (grub-mkfont: aile + biçem + boyut; biçem hep `Regular`).
    pub fn grub_name(&self) -> String {
        format!("{} Regular {}", self.family, self.size)
    }
}

/// Düzen (1× mantıksal px; ölçekle çarpılır). Önizleme aynı değerleri kullanır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub logo: u32,
    pub gap: u32,
    pub menu_w: u32,
    pub item_h: u32,
    pub item_spacing: u32,
    pub visible_items: u32,
    pub item_padding: u32,
    pub radius: u32,
    pub item_font: u32,
    pub small_font: u32,
    pub mono_font: u32,
    pub timeout_gap: u32,
    pub help_bottom: u32,
    pub label_h: u32,
    pub scrollbar_w: u32,
}

pub const LAYOUT: Layout = Layout {
    logo: 96,
    gap: 28,
    menu_w: 600,
    item_h: 40,
    item_spacing: 6,
    visible_items: 6,
    item_padding: 14,
    // Hap köşesi = framework `radius_small` (8). GRUB öğe kutusu hap − 2 × köşe; 16 px fontun satır
    // yüksekliğine (≈ 22) sığmalı, yoksa GRUB alt uzantıları keser (QEMU, 2026-10-17).
    radius: 8,
    item_font: 16,
    small_font: 14,
    mono_font: 14,
    timeout_gap: 18,
    help_bottom: 48,
    label_h: 24,
    scrollbar_w: 4,
};

impl Layout {
    pub fn scaled(self, s: u32) -> Self {
        Self {
            logo: self.logo * s,
            gap: self.gap * s,
            menu_w: self.menu_w * s,
            item_h: self.item_h * s,
            item_spacing: self.item_spacing * s,
            visible_items: self.visible_items,
            item_padding: self.item_padding * s,
            radius: self.radius * s,
            item_font: self.item_font * s,
            small_font: self.small_font * s,
            mono_font: self.mono_font * s,
            timeout_gap: self.timeout_gap * s,
            help_bottom: self.help_bottom * s,
            label_h: self.label_h * s,
            scrollbar_w: self.scrollbar_w * s,
        }
    }

    pub fn menu_h(&self) -> u32 {
        self.visible_items * self.item_h + (self.visible_items - 1) * self.item_spacing
    }

    /// Logo + boşluk + menü bloğunun yarısı: blok ekranın ortasına oturur.
    pub fn half_block(&self) -> u32 {
        (self.logo + self.gap + self.menu_h()) / 2
    }
}

/// Geri sayım etiketi (`__timeout__`; GRUB `%d`'yi saniyeyle değiştirir). GRUB 2.16'da
/// `@TIMEOUT_NOTIFICATION_*@` bu etikette çevrilmeden göründü (QEMU, 2026-10-17); GRUB'un Türkçe
/// kataloğu da çoğu kurulumda yok. Metin tema dilinde ve PF2 aralığında (Türkçe harfler dahil).
pub fn timeout_text(lang: Lang) -> &'static str {
    match lang {
        Lang::Tr => "%d sn içinde başlıyor",
        Lang::En => "Starting in %d s",
    }
}

/// Alt yardım satırı (GRUB'un kendi tuşları; `@KEYMAP_SHORT@` İngilizce kalıyordu).
pub fn keymap_text(lang: Lang) -> &'static str {
    match lang {
        Lang::Tr => "Enter başlat  ·  e düzenle  ·  c komut satırı",
        Lang::En => "Enter boot  ·  e edit  ·  c command line",
    }
}

/// Üretilecek fontlar.
pub fn fonts(scale: u32) -> Vec<FontSpec> {
    let l = LAYOUT.scaled(scale);
    let mut v: Vec<FontSpec> = Vec::new();
    for (size, mono) in [(l.item_font, false), (l.small_font, false), (l.mono_font, true)] {
        let family = if mono { MONO_FAMILY } else { FAMILY };
        let file = format!("{}-{size}.pf2", if mono { "nuxut-mono" } else { "nuxut" });
        if !v.iter().any(|f| f.file == file) {
            v.push(FontSpec { file, family, mono, size });
        }
    }
    v
}

/// `theme.txt`.
pub fn theme_txt(o: &ThemeOptions) -> String {
    let c = &o.colors;
    let l = LAYOUT.scaled(o.scale);
    let item = format!("{} Regular {}", FAMILY, l.item_font);
    let small = format!("{} Regular {}", FAMILY, l.small_font);
    let mono = format!("{} Regular {}", MONO_FAMILY, l.mono_font);
    let half = l.half_block();
    let menu_top_from_center = l.logo + l.gap; // blok tepesinden menüye
    // GRUB ifadeleri: "50%-N" / "50%+N" ve "100%-N" (gfxmenu orantılı biçimi).
    let off = |v: i64| if v >= 0 { format!("50%+{v}") } else { format!("50%-{}", -v) };
    let block_top = -(i64::from(half));
    let menu_top = block_top + i64::from(menu_top_from_center);
    let timeout_top = menu_top + i64::from(l.menu_h()) + i64::from(l.timeout_gap);
    format!(
        r#"# Nuxut GRUB teması. nuxut-system-helper üretir; elle değiştirmeyin.
title-text: ""
desktop-color: "{bg}"
message-font: "{item}"
message-color: "{fg}"
message-bg-color: "{bg}"
terminal-font: "{mono}"
terminal-box: "terminal_box_*.png"

+ image {{
	left = 50%-{logo_half}
	top = {logo_top}
	width = {logo}
	height = {logo}
	file = "logo.png"
}}

+ boot_menu {{
	left = 50%-{menu_half}
	top = {menu_top}
	width = {menu_w}
	height = {menu_h}
	item_font = "{item}"
	selected_item_font = "{item}"
	item_color = "{fg}"
	selected_item_color = "{sel_fg}"
	item_height = {item_h}
	item_padding = {item_padding}
	item_spacing = {item_spacing}
	icon_width = 0
	icon_height = 0
	item_icon_space = 0
	selected_item_pixmap_style = "select_*.png"
	scrollbar = true
	scrollbar_width = {scrollbar_w}
	scrollbar_thumb = "scrollbar_thumb_*.png"
}}

+ label {{
	id = "__timeout__"
	left = 0
	top = {timeout_top}
	width = 100%
	height = {label_h}
	align = "center"
	font = "{small}"
	color = "{muted}"
	text = "{timeout_text}"
}}

+ label {{
	left = 0
	top = 100%-{help_bottom}
	width = 100%
	height = {label_h}
	align = "center"
	font = "{small}"
	color = "{muted}"
	text = "{keymap_text}"
}}
"#,
        bg = c.surface.css(),
        fg = c.on_surface.css(),
        sel_fg = c.on_primary_container.css(),
        muted = c.on_surface_variant.css(),
        logo = l.logo,
        logo_half = l.logo / 2,
        logo_top = off(block_top),
        menu_half = l.menu_w / 2,
        menu_top = off(menu_top),
        menu_w = l.menu_w,
        menu_h = l.menu_h(),
        // GRUB öğe kutusu = hap − dikey pay; aralık + iki pay: hap yüksekliği `item_h`, haplar arası
        // `item_spacing` (QEMU'da gerçek GRUB 2.16 ile doğrulandı, 2026-10-17).
        item_h = l.item_h.saturating_sub(2 * l.radius).max(1),
        item_padding = 0,
        item_spacing = l.item_spacing + l.item_h - l.item_h.saturating_sub(2 * l.radius).max(1),
        timeout_text = timeout_text(o.lang),
        keymap_text = keymap_text(o.lang),
        scrollbar_w = l.scrollbar_w,
        timeout_top = off(timeout_top),
        label_h = l.label_h,
        help_bottom = l.help_bottom,
    )
}

/// Tema dosyaları (PF2 fontlar hariç: onları yardımcı `grub-mkfont` ile ekler, bkz. [`fonts`]).
pub fn generate(o: &ThemeOptions) -> Result<ThemeFiles, RasterError> {
    o.validate().map_err(RasterError)?;
    let c = &o.colors;
    let l = LAYOUT.scaled(o.scale);
    let mut f = ThemeFiles::default();
    f.push("theme.txt", theme_txt(o).into_bytes());
    f.push("logo.png", art::logo_png(c, l.logo)?);
    // Seçili hap: GRUB kutuyu içeriğin dışına parça boyu kadar büyütür; yatay pay = köşe +
    // iç boşluk (metin hap kenarından içeride), dikey pay = köşe (bkz. [`theme_txt`]).
    for (part, png) in art::nine_slice_padded(l.radius, l.radius + l.item_padding, c.primary_container, None)? {
        f.push(format!("select_{part}.png"), png);
    }
    for (part, png) in art::nine_slice(l.radius, c.surface_container, Some(c.outline_variant))? {
        f.push(format!("terminal_box_{part}.png"), png);
    }
    for (part, png) in art::nine_slice((l.scrollbar_w / 2).max(1), c.outline_variant, None)? {
        f.push(format!("scrollbar_thumb_{part}.png"), png);
    }
    Ok(f)
}

/// Damla dosyasının içeriği. `theme_path`: theme.txt'nin mutlak yolu (sabit, yardımcıdan).
pub fn dropin(theme_path: &str) -> String {
    format!(
        "# Nuxut önyükleme teması: nuxut-system-helper yazdı.\n# Geri almak için: nuxut-shell system-theme revert\nGRUB_THEME=\"{theme_path}\"\n"
    )
}

/// grub-mkconfig'in ürettiği bir grub.cfg mi (elle yazılmış dosyanın üstüne asla yazılmaz).
pub fn is_generated(cfg: &str) -> bool {
    cfg.lines().take(8).any(|l| l.contains("automatically generated by grub-mkconfig"))
}

/// `menuentry`/`submenu` sayısı (satır başında, girintili olabilir).
pub fn count_entries(cfg: &str) -> usize {
    cfg.lines()
        .map(str::trim_start)
        .filter(|l| l.starts_with("menuentry ") || l.starts_with("menuentry\t") || l.starts_with("submenu "))
        .count()
}

/// grub.cfg temamızı yüklüyor mu (`set theme=…/themes/nuxut/theme.txt`).
pub fn loads_our_theme(cfg: &str) -> bool {
    cfg.lines().any(|l| {
        let l = l.trim();
        l.starts_with("set theme=") && l.ends_with("/themes/nuxut/theme.txt")
    })
}

/// Yeni grub.cfg'nin kabul denetimi (grub-script-check'e ek): üretilmiş başlık, en az bir giriş,
/// eskisinden az giriş yok (kaybolan çekirdek girişi = kurulum durur), temamız yükleniyor.
pub fn check_new_cfg(old: Option<&str>, new: &str, expect_theme: bool) -> Result<(), String> {
    if !is_generated(new) {
        return Err("yeni grub.cfg grub-mkconfig başlığını taşımıyor".into());
    }
    let n = count_entries(new);
    if n == 0 {
        return Err("yeni grub.cfg'de hiç önyükleme girişi yok".into());
    }
    if let Some(old) = old {
        let o = count_entries(old);
        if n < o {
            return Err(format!("yeni grub.cfg'de giriş sayısı azaldı ({o} → {n})"));
        }
    }
    if expect_theme != loads_our_theme(new) {
        return Err(if expect_theme {
            "yeni grub.cfg Nuxut temasını yüklemiyor (GRUB_THEME başka bir yerde eziliyor olabilir)".into()
        } else {
            "yeni grub.cfg hâlâ Nuxut temasını yüklüyor".into()
        });
    }
    Ok(())
}

/// Kabuk atamalarından bir değişkenin son değeri (`/etc/default/grub` + damlalar sırasıyla).
/// Yalnız düz atamalar: `AD=değer`, `AD="değer"`, `AD='değer'`, önünde `export` olabilir.
/// Değerde `$`/`` ` `` varsa çözülemez → `Some(None)` (bilinmiyor). Hiç atanmamışsa `None`.
pub fn shell_var(texts: &[&str], name: &str) -> Option<Option<String>> {
    let mut found = None;
    for text in texts {
        for line in text.lines() {
            let t = line.trim();
            let t = t.strip_prefix("export ").map(str::trim_start).unwrap_or(t);
            let Some(rest) = t.strip_prefix(name).and_then(|r| r.strip_prefix('=')) else { continue };
            let v = if let Some(q) = rest.strip_prefix('"') {
                q.split_once('"').map(|(v, _)| v)
            } else if let Some(q) = rest.strip_prefix('\'') {
                q.split_once('\'').map(|(v, _)| v)
            } else {
                Some(rest.split(|c: char| c.is_whitespace() || c == '#').next().unwrap_or(""))
            };
            found = Some(v.filter(|v| !v.contains('$') && !v.contains('`')).map(str::to_owned));
        }
    }
    found
}

/// GRUB grafik terminalde mi (tema yalnız `gfxterm`'de görünür).
pub fn graphical_terminal(texts: &[&str]) -> bool {
    match shell_var(texts, "GRUB_TERMINAL_OUTPUT") {
        Some(Some(v)) => v.split_whitespace().any(|t| t == "gfxterm"),
        Some(None) => true, // çözülemedi: engel sayma, yalnız not
        None => match shell_var(texts, "GRUB_TERMINAL") {
            Some(Some(v)) => !v.split_whitespace().any(|t| t == "console"),
            _ => true,
        },
    }
}

/// grub-mkconfig `/etc/default/grub.d/*.cfg` damlalarını okuyor mu (GRUB 2.12+ Arch).
pub fn mkconfig_reads_dropins(script: &str) -> bool {
    script.contains("/default/grub.d/") && script.contains(".cfg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_txt_well_formed() {
        let o = ThemeOptions::sample();
        let t = theme_txt(&o);
        assert!(t.contains("desktop-color: \"#12141a\""));
        assert!(t.contains("item_font = \"Nuxut Regular 16\""));
        assert!(t.contains("terminal-font: \"Nuxut Mono Regular 14\""));
        // Açılan ve kapanan süslü parantez sayısı eşit, her bileşen kapanıyor.
        assert_eq!(t.matches('{').count(), t.matches('}').count());
        // Her satır: yorum, boş, `anahtar: "değer"`, `+ bileşen {`, `}` ya da `anahtar = değer`.
        for line in t.lines() {
            let s = line.trim();
            let ok = s.is_empty()
                || s.starts_with('#')
                || s == "}"
                || (s.starts_with("+ ") && s.ends_with('{'))
                || s.contains(": ")
                || s.contains(" = ");
            assert!(ok, "{line}");
        }
        // Blok ortalı: logo tepesi 50%-197 (96+28+270)/2.
        assert!(t.contains("top = 50%-197"), "{t}");
        let t2 = theme_txt(&ThemeOptions { scale: 2, ..o });
        assert!(t2.contains("item_font = \"Nuxut Regular 32\""));
        assert!(t2.contains("top = 50%-394"));
    }

    #[test]
    fn fonts_cover_theme_names() {
        for s in 1..=3 {
            let o = ThemeOptions { scale: s, ..ThemeOptions::sample() };
            let t = theme_txt(&o);
            let fonts = fonts(s);
            for line in t.lines().filter(|l| l.contains("font")) {
                let name = line.split('"').nth(1).unwrap();
                assert!(fonts.iter().any(|f| f.grub_name() == name), "{name} için PF2 yok");
            }
        }
    }

    #[test]
    fn generate_has_pixmaps() {
        let f = generate(&ThemeOptions::sample()).unwrap();
        for p in ["nw", "n", "ne", "w", "c", "e", "sw", "s", "se"] {
            assert!(f.get(&format!("select_{p}.png")).is_some());
            assert!(f.get(&format!("terminal_box_{p}.png")).is_some());
        }
        assert!(f.get("logo.png").is_some() && f.get("theme.txt").is_some());
    }

    const CFG: &str = "#\n# DO NOT EDIT THIS FILE\n#\n# It is automatically generated by grub-mkconfig using templates\n#\nset theme=($root)/boot/grub/themes/nuxut/theme.txt\nmenuentry 'Arch Linux' {\n}\nsubmenu 'Advanced' {\n\tmenuentry 'Arch fallback' {\n\t}\n}\n";

    #[test]
    fn cfg_checks() {
        assert!(is_generated(CFG));
        assert_eq!(count_entries(CFG), 3);
        assert!(loads_our_theme(CFG));
        assert!(check_new_cfg(Some(CFG), CFG, true).is_ok());
        let fewer = CFG.replace("\tmenuentry 'Arch fallback' {\n\t}\n", "");
        assert!(check_new_cfg(Some(CFG), &fewer, true).is_err());
        assert!(check_new_cfg(None, "menuentry 'x' {}\n", true).is_err(), "elle yazılmış");
        assert!(check_new_cfg(None, CFG, false).is_err(), "geri almada tema kalmamalı");
        let none = "# It is automatically generated by grub-mkconfig\n";
        assert!(check_new_cfg(None, none, false).is_err(), "giriş yok");
    }

    #[test]
    fn shell_vars() {
        let base = "GRUB_TIMEOUT=5\n#GRUB_THEME=\"/x\"\nGRUB_TERMINAL_OUTPUT=console\n";
        let drop = "export GRUB_THEME='/boot/grub/themes/nuxut/theme.txt'\n";
        assert_eq!(shell_var(&[base, drop], "GRUB_THEME"), Some(Some("/boot/grub/themes/nuxut/theme.txt".into())));
        assert_eq!(shell_var(&[base], "GRUB_THEME"), None);
        assert_eq!(shell_var(&["GRUB_THEME=$X/t"], "GRUB_THEME"), Some(None));
        assert_eq!(shell_var(&["GRUB_TIMEOUT=5 # yorum"], "GRUB_TIMEOUT"), Some(Some("5".into())));
        assert!(!graphical_terminal(&[base]));
        assert!(graphical_terminal(&["GRUB_TERMINAL_OUTPUT=\"gfxterm serial\""]));
        assert!(graphical_terminal(&[""]));
        assert!(!graphical_terminal(&["GRUB_TERMINAL=console"]));
    }

    #[test]
    fn dropin_sets_theme() {
        let d = dropin("/boot/grub/themes/nuxut/theme.txt");
        assert_eq!(shell_var(&[&d], "GRUB_THEME"), Some(Some("/boot/grub/themes/nuxut/theme.txt".into())));
    }
}
