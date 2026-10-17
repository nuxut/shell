//! Önyükleme temasının renkleri: seçili Nuxut paletinin **koyu** kipinden bir avuç rol.
//!
//! Neden hep koyu: önyükleme ekranı karanlık ürün yazılımı (firmware) ekranının ardından ve çoğu
//! zaman karanlık odada gelir; açık zemin gece göz kamaştırır, LUKS sorusu da bu ekranda. Karar:
//! 2026-10-16 (`docs/design/boot-theme.md`).
//!
//! Kök yetkili yardımcıya renkler yalnız `rol=#rrggbb` argümanları olarak geçer; tüm roller
//! zorunlu, bilinmeyen rol ve biçim reddedilir (yardımcı hiçbir kullanıcı dosyası okumaz).

use std::fmt;

/// 8 bit sRGB renk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// `#rrggbb` (büyük/küçük harf). Başka biçim yok: `#rgb`, alfa ve adlar reddedilir.
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#')?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some(Self((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// `rrggbb` (küçük harf, `#` yok): Limine ve GRUB/Plymouth biçimlerinin ortak çekirdeği.
    pub fn hex(self) -> String {
        format!("{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// `#rrggbb`.
    pub fn css(self) -> String {
        format!("#{}", self.hex())
    }

    /// `0xrrggbb` (Plymouth two-step).
    pub fn plymouth(self) -> String {
        format!("0x{}", self.hex())
    }

    /// İki renk arasında doğrusal karışım (`t` = 0 → self).
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.css())
    }
}

macro_rules! roles {
    ($($field:ident),* $(,)?) => {
        /// Temanın kullandığı roller (koyu kip). Alan adları Material rol adlarıdır
        /// (`nuxut_palette` snake_case), `logo_shade` hariç: logonun gövde tonu
        /// (framework `brand_shade(primary)`).
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct BootColors {
            $(pub $field: Rgb,)*
        }

        /// Argüman adları (sıra kararlı).
        pub const ROLE_NAMES: &[&str] = &[$(stringify!($field)),*];

        impl BootColors {
            fn slot(&mut self, name: &str) -> Option<&mut Rgb> {
                match name {
                    $(stringify!($field) => Some(&mut self.$field),)*
                    _ => None,
                }
            }

            /// `(ad, renk)` çiftleri, [`ROLE_NAMES`] sırasıyla.
            pub fn pairs(&self) -> Vec<(&'static str, Rgb)> {
                vec![$((stringify!($field), self.$field)),*]
            }
        }
    };
}

roles!(
    surface,
    surface_container,
    surface_container_highest,
    on_surface,
    on_surface_variant,
    primary,
    on_primary,
    primary_container,
    on_primary_container,
    secondary,
    tertiary,
    outline_variant,
    error,
    logo_shade,
);

/// Renk argümanı hatası.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorError {
    /// `ad=#rrggbb` biçiminde değil.
    Syntax(String),
    UnknownRole(String),
    BadColor(String),
    Duplicate(String),
    Missing(&'static str),
}

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(s) => write!(f, "renk argümanı ad=#rrggbb değil: {s:?}"),
            Self::UnknownRole(s) => write!(f, "bilinmeyen renk rolü: {s:?}"),
            Self::BadColor(s) => write!(f, "geçersiz renk: {s:?}"),
            Self::Duplicate(s) => write!(f, "renk rolü iki kez verildi: {s}"),
            Self::Missing(s) => write!(f, "renk rolü eksik: {s}"),
        }
    }
}

impl std::error::Error for ColorError {}

/// Argüman uzunluğu sınırı (`surface_container_highest=#rrggbb` 33 bayt).
const MAX_ARG: usize = 64;

impl BootColors {
    /// Nuxut varsayılan koyu paleti (framework `Palette::default` değerlerine yakın; testler ve
    /// önizleme örnekleri için). Ürün yolu renkleri her zaman seçili paletten alır.
    pub fn fallback() -> Self {
        Self {
            surface: Rgb(0x12, 0x14, 0x1a),
            surface_container: Rgb(0x1e, 0x20, 0x26),
            surface_container_highest: Rgb(0x33, 0x35, 0x3b),
            on_surface: Rgb(0xe2, 0xe2, 0xe9),
            on_surface_variant: Rgb(0xc4, 0xc6, 0xd0),
            primary: Rgb(0x9e, 0xc5, 0xff),
            on_primary: Rgb(0x00, 0x31, 0x5d),
            primary_container: Rgb(0x1d, 0x47, 0x7f),
            on_primary_container: Rgb(0xd5, 0xe3, 0xff),
            secondary: Rgb(0xbc, 0xc7, 0xdc),
            tertiary: Rgb(0xda, 0xbd, 0xe2),
            outline_variant: Rgb(0x44, 0x47, 0x4e),
            error: Rgb(0xff, 0xb4, 0xab),
            logo_shade: Rgb(0x5d, 0x8f, 0xe8),
        }
    }

    /// `ad=#rrggbb` argümanlarından; her rol tam bir kez.
    pub fn from_args<'a>(args: impl IntoIterator<Item = &'a str>) -> Result<Self, ColorError> {
        let mut out = Self::fallback();
        let mut seen = vec![false; ROLE_NAMES.len()];
        for arg in args {
            if arg.len() > MAX_ARG {
                return Err(ColorError::Syntax(arg.chars().take(MAX_ARG).collect()));
            }
            let (name, value) = arg.split_once('=').ok_or_else(|| ColorError::Syntax(arg.to_owned()))?;
            let idx = ROLE_NAMES.iter().position(|r| *r == name).ok_or_else(|| ColorError::UnknownRole(name.to_owned()))?;
            if std::mem::replace(&mut seen[idx], true) {
                return Err(ColorError::Duplicate(name.to_owned()));
            }
            let color = Rgb::parse(value).ok_or_else(|| ColorError::BadColor(value.to_owned()))?;
            if let Some(slot) = out.slot(name) {
                *slot = color;
            }
        }
        if let Some(i) = seen.iter().position(|s| !s) {
            return Err(ColorError::Missing(ROLE_NAMES[i]));
        }
        Ok(out)
    }

    /// [`Self::from_args`]'ın tersi.
    pub fn to_args(&self) -> Vec<String> {
        self.pairs().into_iter().map(|(n, c)| format!("{n}={}", c.css())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parse_is_strict() {
        assert_eq!(Rgb::parse("#9ec5FF"), Some(Rgb(0x9e, 0xc5, 0xff)));
        for bad in ["9ec5ff", "#9ec5f", "#9ec5fff", "#9ec5fg", "#+ec5ff", "", "#", "#9ec5ff\n"] {
            assert_eq!(Rgb::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn args_round_trip() {
        let c = BootColors::fallback();
        let args = c.to_args();
        assert_eq!(args.len(), ROLE_NAMES.len());
        let back = BootColors::from_args(args.iter().map(String::as_str)).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn args_reject_missing_unknown_duplicate() {
        let mut args = BootColors::fallback().to_args();
        args.pop();
        assert!(matches!(BootColors::from_args(args.iter().map(String::as_str)), Err(ColorError::Missing("logo_shade"))));
        args.push("logo_shade=#000000".into());
        args.push("logo_shade=#000000".into());
        assert!(matches!(BootColors::from_args(args.iter().map(String::as_str)), Err(ColorError::Duplicate(_))));
        assert!(matches!(BootColors::from_args(["shadow=#000000"]), Err(ColorError::UnknownRole(_))));
        assert!(matches!(BootColors::from_args(["primary"]), Err(ColorError::Syntax(_))));
        assert!(matches!(BootColors::from_args(["primary=red"]), Err(ColorError::BadColor(_))));
        let long = format!("primary=#{}", "0".repeat(100));
        assert!(matches!(BootColors::from_args([long.as_str()]), Err(ColorError::Syntax(_))));
    }

    #[test]
    fn mix_endpoints() {
        let a = Rgb(0, 0, 0);
        let b = Rgb(200, 100, 50);
        assert_eq!(a.mix(b, 0.0), a);
        assert_eq!(a.mix(b, 1.0), b);
        assert_eq!(a.mix(b, 0.5), Rgb(100, 50, 25));
    }
}
