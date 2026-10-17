//! Üretilen tema dosyaları ve ortak seçenekler.

use crate::colors::BootColors;

/// Bir temanın dosyaları: düz bir dizin (alt dizin yok). Ad kuralı [`valid_file_name`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeFiles {
    pub files: Vec<(String, Vec<u8>)>,
}

impl ThemeFiles {
    pub fn push(&mut self, name: impl Into<String>, bytes: Vec<u8>) {
        let name = name.into();
        debug_assert!(valid_file_name(&name), "{name}");
        self.files.push((name, bytes));
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files.iter().find(|(n, _)| n == name).map(|(_, b)| b.as_slice())
    }

    pub fn text(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(|b| std::str::from_utf8(b).ok())
    }

    /// Toplam bayt.
    pub fn size(&self) -> usize {
        self.files.iter().map(|(_, b)| b.len()).sum()
    }
}

/// Tema dizinindeki dosya adı: 1–64 bayt `[a-z0-9._-]`, `.` ile başlamaz.
pub fn valid_file_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

/// Yapılandırma dosyalarına yazılan font ailesi adı: 1–64 bayt, harf/rakam/boşluk/`-`.
/// (Satır sonu, tırnak ve `=` gibi sözdizimi karakterleri giremez.)
pub fn valid_family(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'-'))
        && !name.starts_with(' ')
        && !name.ends_with(' ')
}

/// Metin dili (yalnız temaya gömülen birkaç dize: Plymouth güncelleme başlıkları).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    Tr,
    #[default]
    En,
}

impl Lang {
    /// `tr`, `tr_TR.UTF-8` → Tr; diğerleri En.
    pub fn from_locale(s: &str) -> Self {
        if s.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("tr")) { Self::Tr } else { Self::En }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Tr => "tr",
            Self::En => "en",
        }
    }
}

/// Üretim seçenekleri.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeOptions {
    pub colors: BootColors,
    /// Önyükleyici menüsü ölçeği (1–3): GRUB/Limine çözünürlüğe uyamaz; kullanıcı tarafı en
    /// büyük ekranın fiziksel yüksekliğinden seçer (`docs/design/boot-theme.md`).
    pub scale: u32,
    pub lang: Lang,
    /// Arayüz font ailesi (Plymouth `Font=`; GRUB'da PF2'ye dönüştürülen ailenin adı).
    pub family: String,
    /// Eş aralıklı aile (GRUB düzenleyici/komut satırı, Plymouth ayrıntı kipi).
    pub mono_family: String,
}

impl ThemeOptions {
    pub const MAX_SCALE: u32 = 3;

    /// Seçenekler kurallara uyuyor mu (yardımcı her girdiyi yeniden denetler).
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=Self::MAX_SCALE).contains(&self.scale) {
            return Err(format!("ölçek 1–{} olmalı: {}", Self::MAX_SCALE, self.scale));
        }
        for f in [&self.family, &self.mono_family] {
            if !valid_family(f) {
                return Err(format!("geçersiz font ailesi: {f:?}"));
            }
        }
        Ok(())
    }

    /// Testler ve önizleme örnekleri için.
    pub fn sample() -> Self {
        Self {
            colors: BootColors::fallback(),
            scale: 1,
            lang: Lang::Tr,
            family: "Noto Sans".into(),
            mono_family: "DejaVu Sans Mono".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_families() {
        assert!(valid_file_name("throbber-0001.png"));
        for bad in ["", ".x", "a/b", "A.png", "x\n", &"a".repeat(65)] {
            assert!(!valid_file_name(bad), "{bad:?}");
        }
        assert!(valid_family("Noto Sans"));
        for bad in ["", " A", "A ", "A\nB", "A=B", "A\"", &"a".repeat(65)] {
            assert!(!valid_family(bad), "{bad:?}");
        }
    }

    #[test]
    fn options_validate() {
        let mut o = ThemeOptions::sample();
        assert!(o.validate().is_ok());
        o.scale = 0;
        assert!(o.validate().is_err());
        o.scale = 4;
        assert!(o.validate().is_err());
        o.scale = 2;
        o.family = "X\nTheme=evil".into();
        assert!(o.validate().is_err());
    }

    #[test]
    fn lang_from_locale() {
        assert_eq!(Lang::from_locale("tr_TR.UTF-8"), Lang::Tr);
        assert_eq!(Lang::from_locale("en_US"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
    }
}
