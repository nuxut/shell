//! Limine teması: `limine.conf`'a (Limine 8+ `anahtar: değer` sözdizimi) yönetilen bir renk
//! bloğu.
//!
//! Yalnız renkler ve marka satırı; duvar kağıdı yok. Neden: Limine menüsü ekran ortasına değil
//! terminal ızgarasına çizilir, çözünürlüğü bilmeden konan logo menüyle çakışır; görsel dosya
//! ayrıca yapılandırmanın bulunduğu bölümde olmalı ve Secure Boot + kayıtlı sağlama toplamında
//! hash ister. Renk bloğu bölüm bağımsız ve her kurulumda güvenli. Karar: 2026-10-16.
//!
//! Düzenleme kuralları (`apply_block`):
//! - Blok dosyanın en başına konur; işaretleri `# >>> nuxut-system-theme` / `# <<< …`.
//! - Blok dışındaki aynı anahtarlar (Limine'de genel seçenekler dosyanın her yerinde olabilir)
//!   `#nuxut-disabled# ` önekiyle yoruma alınır; geri almada aynen açılır.
//! - Önce tüm dosya ayrıştırılır; tanımadığımız tek satır varsa (eski `limine.cfg` sözdizimi,
//!   yeni bir özellik) dokunulmaz.
//! - Limine EFI ikilisine yapılandırma sağlama toplamı kaydedilmişse (`limine enroll-config`)
//!   dosya değişince Limine açılmaz (panik): [`config_checksum_enrolled`] ile algılanır, o
//!   kurulumda Limine'ye dokunulmaz.

use crate::colors::BootColors;

pub const BEGIN: &str = "# >>> nuxut-system-theme";
pub const END: &str = "# <<< nuxut-system-theme";
pub const DISABLED: &str = "#nuxut-disabled# ";
/// Dosya boyutu sınırı (Limine'nin kendi sınırı da küçük; kullanıcının dosyası KB düzeyinde).
pub const MAX_CONF: usize = 256 * 1024;

/// Bloğun yazdığı anahtarlar (küçük harf; `colour`/`color` eşanlamlı).
const KEYS: &[&str] = &[
    "interface_branding",
    "interface_branding_colour",
    "interface_help_colour",
    "interface_help_colour_bright",
    "term_palette",
    "term_palette_bright",
    "term_background",
    "term_foreground",
    "term_background_bright",
    "term_foreground_bright",
];

fn canonical_key(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace("_color", "_colour")
}

/// Satır türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line<'a> {
    Blank,
    Comment,
    Entry,
    Macro,
    Option { key: &'a str, value: &'a str },
}

/// Bir satırı sınıflandırır; tanınmayan sözdiziminde `None`.
pub fn classify(line: &str) -> Option<Line<'_>> {
    let t = line.trim();
    if t.is_empty() {
        return Some(Line::Blank);
    }
    if t.starts_with('#') {
        return Some(Line::Comment);
    }
    if t.starts_with('/') {
        return Some(Line::Entry);
    }
    if t.starts_with("${") && t.contains("}=") {
        return Some(Line::Macro);
    }
    let (key, value) = t.split_once(':')?;
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    Some(Line::Option { key, value: value.trim() })
}

/// Düzenleme hatası.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    TooLarge,
    NotUtf8,
    /// Tanınmayan satır (1 tabanlı numara).
    Unknown(usize),
    /// Blok işaretleri bozuk (yalnız biri ya da birden çok blok).
    Markers,
    NoEntries,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "limine.conf çok büyük"),
            Self::NotUtf8 => write!(f, "limine.conf UTF-8 değil"),
            Self::Unknown(n) => write!(f, "limine.conf {n}. satır tanınmıyor (eski sözdizimi ya da yeni bir özellik); dokunulmadı"),
            Self::Markers => write!(f, "limine.conf'taki Nuxut blok işaretleri bozuk"),
            Self::NoEntries => write!(f, "limine.conf'ta önyükleme girişi yok"),
        }
    }
}

impl std::error::Error for EditError {}

/// Bayttan metne; boyut ve kodlama denetimi.
pub fn decode(bytes: &[u8]) -> Result<&str, EditError> {
    if bytes.len() > MAX_CONF {
        return Err(EditError::TooLarge);
    }
    std::str::from_utf8(bytes).map_err(|_| EditError::NotUtf8)
}

/// Dosyanın tamamı tanınıyor ve en az bir giriş var mı.
pub fn validate(conf: &str) -> Result<(), EditError> {
    let mut entries = 0;
    for (i, line) in conf.lines().enumerate() {
        match classify(line) {
            None => return Err(EditError::Unknown(i + 1)),
            Some(Line::Entry) => entries += 1,
            _ => {}
        }
    }
    if entries == 0 {
        return Err(EditError::NoEntries);
    }
    Ok(())
}

/// Yönetilen blok (işaretler dahil, sonda `\n`).
pub fn block(c: &BootColors) -> String {
    let pal = |xs: [crate::Rgb; 8]| xs.iter().map(|c| c.hex()).collect::<Vec<_>>().join(";");
    let mut s = String::new();
    s.push_str(BEGIN);
    s.push_str(" (nuxut-system-helper yönetir; geri almak için: nuxut-shell system-theme revert)\n");
    s.push_str("interface_branding: Nuxut\n");
    s.push_str(&format!("interface_branding_colour: {}\n", c.primary.hex()));
    s.push_str(&format!("interface_help_colour: {}\n", c.on_surface_variant.hex()));
    s.push_str(&format!("interface_help_colour_bright: {}\n", c.on_surface.hex()));
    s.push_str(&format!(
        "term_palette: {}\n",
        pal([c.surface_container, c.error, c.tertiary, c.secondary, c.primary, c.tertiary, c.secondary, c.on_surface_variant])
    ));
    s.push_str(&format!(
        "term_palette_bright: {}\n",
        pal([c.outline_variant, c.error, c.tertiary, c.secondary, c.primary, c.tertiary, c.secondary, c.on_surface])
    ));
    // TTRRGGBB: TT saydamlık, 00 = opak.
    s.push_str(&format!("term_background: 00{}\n", c.surface.hex()));
    s.push_str(&format!("term_foreground: {}\n", c.on_surface.hex()));
    s.push_str(&format!("term_background_bright: {}\n", c.surface_container_highest.hex()));
    s.push_str(&format!("term_foreground_bright: {}\n", c.on_surface.hex()));
    s.push_str(END);
    s.push('\n');
    s
}

/// Bloğun yerini bulur: `(başlangıç satırı, bitiş satırı)` (dahil). Blok yoksa `None`.
fn find_block(lines: &[&str]) -> Result<Option<(usize, usize)>, EditError> {
    let begins: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.trim_start().starts_with(BEGIN)).map(|(i, _)| i).collect();
    let ends: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.trim() == END).map(|(i, _)| i).collect();
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([b], [e]) if b < e => Ok(Some((*b, *e))),
        _ => Err(EditError::Markers),
    }
}

/// Blok dışındaki satırlar; bizim yoruma aldıklarımız açılmış hâlde.
fn without_block(conf: &str) -> Result<Vec<String>, EditError> {
    let lines: Vec<&str> = conf.lines().collect();
    let range = find_block(&lines)?;
    Ok(lines
        .iter()
        .enumerate()
        .filter(|(i, _)| range.is_none_or(|(b, e)| *i < b || *i > e))
        .map(|(_, l)| l.strip_prefix(DISABLED).unwrap_or(l).to_owned())
        .collect())
}

/// Bloğu yazar (varsa yeniler). Girdi önce tam doğrulanır; çıktı da doğrulanır.
pub fn apply_block(conf: &str, c: &BootColors) -> Result<String, EditError> {
    let base = without_block(conf)?;
    validate(&base.join("\n"))?;
    let mut out = block(c);
    for line in &base {
        let disable = matches!(classify(line), Some(Line::Option { key, .. }) if KEYS.contains(&canonical_key(key).as_str()));
        if disable {
            out.push_str(DISABLED);
        }
        out.push_str(line);
        out.push('\n');
    }
    validate(&out)?;
    debug_assert_eq!(remove_block(&out).ok().as_deref().map(normalize), Some(normalize(&base.join("\n"))));
    Ok(out)
}

/// Bloğu kaldırır, yoruma alınmış satırları açar. Blok yoksa metin olduğu gibi döner.
pub fn remove_block(conf: &str) -> Result<String, EditError> {
    let base = without_block(conf)?;
    let mut out = base.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

/// Bizim bloğumuz var mı.
pub fn has_block(conf: &str) -> bool {
    conf.lines().any(|l| l.trim_start().starts_with(BEGIN))
}

fn normalize(s: &str) -> String {
    s.lines().collect::<Vec<_>>().join("\n")
}

/// Limine EFI/BIOS ikilisinde yapılandırma sağlama toplamı kayıtlı mı.
/// `Some(true)`: kayıtlı (dosyaya dokunmak açılışı bozar); `Some(false)`: kayıtsız (128 × `0`);
/// `None`: işaret yok (Limine ikilisi değil ya da tanımadığımız sürüm → güvenli tarafta kalınır).
pub fn config_checksum_enrolled(binary: &[u8]) -> Option<bool> {
    const MARK: &[u8] = b"++CONFIG_B2SUM_SIGNATURE++";
    let pos = binary.windows(MARK.len()).position(|w| w == MARK)?;
    let hash = binary.get(pos + MARK.len()..pos + MARK.len() + 128)?;
    if !hash.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    Some(!hash.iter().all(|&b| b == b'0'))
}

/// Limine'nin yapılandırmayı aradığı yollar (bir birimin köküne göre, öncelik sırasıyla).
/// EFI'de önce ikilinin dizini (`EFI/<x>/limine.conf`) gelir; o yol EFI dizini taramasıyla bulunur.
pub const VOLUME_CANDIDATES: &[&str] = &["boot/limine/limine.conf", "boot/limine.conf", "limine/limine.conf", "limine.conf"];

#[cfg(test)]
mod tests {
    use super::*;

    const CONF: &str = "timeout: 5\ninterface_branding: Benim\n\n/Arch Linux\n    protocol: linux\n    path: boot():/vmlinuz-linux\n    cmdline: root=UUID=x rw\n    module_path: boot():/initramfs-linux.img\n";

    #[test]
    fn classify_lines() {
        assert_eq!(classify("  # x"), Some(Line::Comment));
        assert_eq!(classify("//+Sub"), Some(Line::Entry));
        assert_eq!(classify("${ROOT}=root=UUID=x"), Some(Line::Macro));
        assert_eq!(classify("    path: boot():/vmlinuz"), Some(Line::Option { key: "path", value: "boot():/vmlinuz" }));
        assert_eq!(classify("TIMEOUT=5"), None, "limine.cfg (v7) sözdizimi");
        assert_eq!(classify(":Arch"), None);
    }

    #[test]
    fn apply_and_remove_round_trip() {
        let c = BootColors::fallback();
        let out = apply_block(CONF, &c).unwrap();
        assert!(out.starts_with(BEGIN));
        assert!(out.contains("interface_branding_colour: 9ec5ff\n"));
        assert!(out.contains("term_background: 0012141a\n"));
        assert!(out.contains("#nuxut-disabled# interface_branding: Benim\n"));
        assert!(out.contains("\ntimeout: 5\n"), "ilgisiz genel seçenek korunur");
        validate(&out).unwrap();
        // Yeniden uygulama tek blok bırakır (yinelenmez), sonuç aynıdır.
        let again = apply_block(&out, &c).unwrap();
        assert_eq!(again, out);
        // Geri alma özgün metni verir.
        assert_eq!(remove_block(&out).unwrap(), CONF);
    }

    #[test]
    fn color_alias_is_disabled_too() {
        let conf = "interface_branding_color: ff0000\n/Arch\n    protocol: linux\n";
        let out = apply_block(conf, &BootColors::fallback()).unwrap();
        assert!(out.contains("#nuxut-disabled# interface_branding_color: ff0000"));
    }

    #[test]
    fn refuses_unknown_and_broken() {
        let c = BootColors::fallback();
        assert_eq!(apply_block("TIMEOUT=5\n:Arch\n", &c), Err(EditError::Unknown(1)));
        assert_eq!(apply_block("timeout: 5\n", &c), Err(EditError::NoEntries));
        let broken = format!("{BEGIN}\ntimeout: 1\n/Arch\n");
        assert_eq!(apply_block(&broken, &c), Err(EditError::Markers));
        let two = format!("{}{}/A\n", block(&c), block(&c));
        assert_eq!(apply_block(&two, &c), Err(EditError::Markers));
        assert_eq!(decode(&vec![b'a'; MAX_CONF + 1]), Err(EditError::TooLarge));
        assert_eq!(decode(&[0xff]), Err(EditError::NotUtf8));
    }

    #[test]
    fn enrollment_detection() {
        let mut bin = b"xx++CONFIG_B2SUM_SIGNATURE++".to_vec();
        bin.extend(std::iter::repeat_n(b'0', 128));
        bin.push(0);
        assert_eq!(config_checksum_enrolled(&bin), Some(false));
        let start = bin.len() - 129;
        bin[start] = b'a';
        assert_eq!(config_checksum_enrolled(&bin), Some(true));
        assert_eq!(config_checksum_enrolled(b"no marker"), None);
        assert_eq!(config_checksum_enrolled(b"++CONFIG_B2SUM_SIGNATURE++00"), None, "kısa");
    }
}
