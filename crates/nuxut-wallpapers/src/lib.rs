//! Nuxut'un yerleşik duvar kağıtları (kullanıcı kararı, 2026-10-13;
//! `docs/design/wallpapers.md`).
//!
//! - Dokuz tasarım (altısı maskotsuz, üçü maskotlu) × koyu/açık SVG derleme anında gömülüdür
//!   (`include_bytes!`). Kaynak güvenilirdir (repo içi, kullanıcı dosyası değil): süreç içinde
//!   `resvg` ile render serbesttir.
//! - Kaynak sözdizimi `nuxut:<id>` (tema kipini izler) ya da `nuxut:<id>/dark|light` (sabit);
//!   bkz. [`parse_source`]. Bilinmeyen kimlik ya da varyant (eski `:dark` biçimi dahil) uyarı +
//!   [`DEFAULT_ID`].
//! - [`render()`]: tam fiziksel çözünürlükte cover (ortadan kırpma) + bant kırıcı sıralı dither.
//! - [`Renderer`]: arka plan thread'inde bellek içi render, aynı anahtarlar birleşir; sonuç
//!   [`Frame`] (paylaşılan RGBA) duvar kağıdı bileşenine `Wallpaper::set_image` ile dosyasız
//!   verilir (disk önbelleği ve yalıtılmış PNG turu yok). [`Slot`]: yüzey başına istek/sonuç
//!   (shell ve kilit ekranı ortak).
//! - Material You tohumu render'dan çıkarılmaz: tasarım notlarındaki sabit tablo
//!   ([`Builtin::seed`]).
//! - Görünen adlar crate'in kendi Fluent kataloğundadır ([`i18n_source`], anahtarlar
//!   `wallpapers-…`); tüketici kataloğunu kurarken ekler.

mod dither;
mod render;
mod renderer;

pub use dither::deband;
pub use render::{MAX_PIXELS, MAX_SIDE, RenderError, Rendered, rasterize, render, render_preview, render_preview_themed, render_themed};
pub use renderer::{Frame, PIPELINE, Renderer, Slot, image_key};
pub use nuxut_theme::ColorThemeId;

use std::borrow::Cow;

/// Yerleşiklerin boyanacağı renk teması: seçili Nuxut teması; renkler duvar kâğıdından
/// geliyorsa (tema yok) tasarımdaki Nuxut mavisi. Kullanıcı görselleri hiç boyanmaz.
pub fn color_theme_for(theme: &nuxut_theme::Theme) -> ColorThemeId {
    theme.color_theme.unwrap_or(ColorThemeId::Blue)
}

/// Yerleşik kaynakların öneki.
pub const SCHEME: &str = "nuxut:";
/// Varsayılan duvar kağıdı (taze kurulum ve yapılandırılmamış `[wallpaper] path`).
pub const DEFAULT_ID: &str = "pair-flight";
/// [`DEFAULT_ID`]'nin kaynak dizgesi.
pub const DEFAULT_SOURCE: &str = "nuxut:pair-flight";

/// Varyant (koyu/açık). Kaynakta yazılmazsa tema kipini izler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Variant {
    Dark,
    Light,
}

impl Variant {
    pub const ALL: [Variant; 2] = [Variant::Dark, Variant::Light];

    pub fn name(self) -> &'static str {
        match self {
            Variant::Dark => "dark",
            Variant::Light => "light",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "dark" => Some(Variant::Dark),
            "light" => Some(Variant::Light),
            _ => None,
        }
    }
}

/// Bir yerleşik duvar kağıdı.
#[derive(Debug)]
pub struct Builtin {
    /// Kalıcı kimlik (İngilizce kebab; `shell.toml`'a yazılır, değişmez).
    pub id: &'static str,
    dark: &'static [u8],
    light: &'static [u8],
    /// Material You tohumu, `[koyu, açık]` (`#rrggbb`; tasarım notlarının "tohum/vurgu" sütunu).
    seed: [&'static str; 2],
    /// Fluent anahtarı (`wallpapers-<id>`).
    name_key: &'static str,
}

impl Builtin {
    /// Varyantın SVG baytları.
    pub fn svg(&self, v: Variant) -> &'static [u8] {
        match v {
            Variant::Dark => self.dark,
            Variant::Light => self.light,
        }
    }

    /// Varyantın SVG'si `theme` renk temasına boyanmış: tasarımlar Nuxut mavisiyle çizildi;
    /// başka temada her renk özniteliği tonu korunarak temaya taşınır (framework
    /// `ColorTheme::recolor`, docs/design/color-themes.md). Nuxut mavisinde baytlar aynen döner.
    pub fn svg_for(&self, v: Variant, theme: ColorThemeId) -> Cow<'static, [u8]> {
        let svg = self.svg(v);
        if theme == ColorThemeId::Blue {
            return Cow::Borrowed(svg);
        }
        let target = theme.theme();
        Cow::Owned(nuxut_theme::recolor_svg(svg, |c| target.recolor(&nuxut_theme::color_theme::BLUE, c)))
    }

    /// Material You tohum rengi (`#rrggbb`, küçük harf). Render'dan çıkarılmaz: ortalama renk
    /// tasarımda bilinçli olarak soluk, vurgu rengi tasarımcının seçtiği tohumdur.
    pub fn seed(&self, v: Variant) -> &'static str {
        match v {
            Variant::Dark => self.seed[0],
            Variant::Light => self.seed[1],
        }
    }

    /// Kaynak dizgesi (`nuxut:<id>`).
    pub fn source(&self) -> String {
        format!("{SCHEME}{}", self.id)
    }

    pub fn is_default(&self) -> bool {
        self.id == DEFAULT_ID
    }

    /// Görünen ad (süreç geneli katalogdan; katalog [`i18n_source`]'u içermiyorsa anahtar döner).
    pub fn name(&self) -> String {
        nuxut_i18n::current().text(self.name_key).into_owned()
    }
}

macro_rules! builtin {
    ($id:literal, $dark:literal, $light:literal) => {
        Builtin {
            id: $id,
            dark: include_bytes!(concat!("../assets/", $id, "-dark.svg")),
            light: include_bytes!(concat!("../assets/", $id, "-light.svg")),
            seed: [$dark, $light],
            name_key: concat!("wallpapers-", $id),
        }
    };
}

/// Bütün yerleşikler (ayarlardaki sıra). Kaynak dosyalar: `.team/wallpapers/final/` (aurora = C,
/// buyuk-siluet = A, es-ucus = b5, goc = D v2, kanat-ucu = b1, tirmanis = b3) ve maskotlu
/// `.team/wallpapers/mascot/` (cloud-rest = B, far-glide = A, chase = C; kullanıcı kararı
/// 2026-10-13). Maskotlu tohumlar: README'deki iz/vurgu rengi (eş uçuşla aynı iz dili).
pub static BUILTINS: [Builtin; 9] = [
    builtin!("pair-flight", "#9ec5ff", "#3f6fc6"),
    builtin!("wingtip", "#9ec5ff", "#3f6fc6"),
    builtin!("climb", "#9ec5ff", "#3f6fc6"),
    builtin!("migration", "#9ec5ff", "#2f5fb8"),
    builtin!("silhouette", "#9ec5ff", "#3f6fc6"),
    builtin!("aurora", "#5d8fe8", "#8fb6f6"),
    builtin!("cloud-rest", "#9ec5ff", "#3f6fc6"),
    builtin!("far-glide", "#9ec5ff", "#3f6fc6"),
    builtin!("chase", "#9ec5ff", "#3f6fc6"),
];

/// Kimliğe göre yerleşik.
pub fn find(id: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.id == id)
}

/// Varsayılan yerleşik.
pub fn default_builtin() -> &'static Builtin {
    find(DEFAULT_ID).unwrap_or(&BUILTINS[0])
}

/// Çözülmüş yerleşik seçimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub builtin: &'static Builtin,
    /// `None`: tema kipini izler.
    pub variant: Option<Variant>,
}

impl PartialEq for Builtin {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Builtin {}

impl Selection {
    /// Gösterilecek varyant: sabitse o, değilse tema kipi (`dark`).
    pub fn variant_for(&self, theme_dark: bool) -> Variant {
        self.variant.unwrap_or(if theme_dark { Variant::Dark } else { Variant::Light })
    }

    /// Kaynak dizgesi (`nuxut:<id>[/<varyant>]`).
    pub fn source(&self) -> String {
        match self.variant {
            Some(v) => format!("{SCHEME}{}/{}", self.builtin.id, v.name()),
            None => self.builtin.source(),
        }
    }
}

/// [`parse_source`]'un sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// `nuxut:` öneki yok: dosya yolu (çağıran yorumlar).
    NotBuiltin,
    Builtin(Selection),
    /// `nuxut:` önekli ama tanınmayan: varsayılan seçim + uyarı (log/config uyarısı; çevrilmez).
    Invalid { fallback: Selection, warning: SourceWarning },
}

/// Tanınmayan yerleşik kaynak (gösterilen parça kırpılmış, denetim karaktersiz).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceWarning {
    UnknownId(String),
    UnknownVariant(String),
}

impl std::fmt::Display for SourceWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownId(id) => write!(f, "yerleşik duvar kağıdı \"{id}\" bilinmiyor; {DEFAULT_SOURCE} kullanılıyor"),
            Self::UnknownVariant(v) => write!(f, "yerleşik duvar kağıdı varyantı \"{v}\" tanınmıyor (dark|light); {DEFAULT_SOURCE} kullanılıyor"),
        }
    }
}

/// `nuxut:<id>` / `nuxut:<id>/dark|light`. Önek dışında boşluk/büyük harf kabul edilmez
/// (kimlikler sabit). Bilinmeyen kimlik ya da varyant → [`Parsed::Invalid`] (varsayılan).
pub fn parse_source(s: &str) -> Parsed {
    let Some(rest) = s.strip_prefix(SCHEME) else { return Parsed::NotBuiltin };
    let fallback = Selection { builtin: default_builtin(), variant: None };
    let shown = |t: &str| t.chars().take(48).filter(|c| !c.is_control()).collect::<String>();
    let (id, variant) = match rest.split_once('/') {
        Some((id, v)) => match Variant::from_name(v) {
            Some(v) => (id, Some(v)),
            None => return Parsed::Invalid { fallback, warning: SourceWarning::UnknownVariant(shown(v)) },
        },
        None => (rest, None),
    };
    match find(id) {
        Some(builtin) => Parsed::Builtin(Selection { builtin, variant }),
        None => Parsed::Invalid { fallback, warning: SourceWarning::UnknownId(shown(id)) },
    }
}

/// Crate'in Fluent kaynağı (`locales/<dil>.ftl`, alan `wallpapers`). Ad gösteren tüketici
/// (shell, ayarlar) kataloğunu kurarken kendi kaynağının yanına ekler.
pub fn i18n_source() -> nuxut_i18n::Source {
    nuxut_i18n::embed!("wallpapers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_unique_and_default_present() {
        let mut ids: Vec<_> = BUILTINS.iter().map(|b| b.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), BUILTINS.len());
        assert_eq!(default_builtin().id, "pair-flight");
        assert_eq!(DEFAULT_SOURCE, format!("{SCHEME}{DEFAULT_ID}"));
        assert!(BUILTINS.iter().filter(|b| b.is_default()).count() == 1);
        // Kimlik listesi sözleşmedir (wallpaper-builtins-v1 KARARLAR + maskotlu üçlü):
        // `[a-z0-9-]{1,32}`, sıra = seçicideki sıra.
        let ids: Vec<_> = BUILTINS.iter().map(|b| b.id).collect();
        assert_eq!(
            ids,
            ["pair-flight", "wingtip", "climb", "migration", "silhouette", "aurora", "cloud-rest", "far-glide", "chase"]
        );
        for id in ids {
            assert!((1..=32).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'), "{id}");
        }
    }

    #[test]
    fn parse_sources() {
        assert_eq!(parse_source("/home/u/a.png"), Parsed::NotBuiltin);
        assert_eq!(parse_source("~/nuxut:aurora"), Parsed::NotBuiltin);
        let Parsed::Builtin(s) = parse_source("nuxut:aurora") else { panic!() };
        assert_eq!((s.builtin.id, s.variant), ("aurora", None));
        let Parsed::Builtin(s) = parse_source("nuxut:migration/light") else { panic!() };
        assert_eq!((s.builtin.id, s.variant), ("migration", Some(Variant::Light)));
        assert_eq!(s.source(), "nuxut:migration/light");
        for bad in ["nuxut:", "nuxut:yok", "nuxut:Aurora", "nuxut:aurora/dim", "nuxut:aurora/", "nuxut:aurora/dark/x", "nuxut:aurora:dark", "nuxut:pair-flight:light"] {
            let Parsed::Invalid { fallback, warning } = parse_source(bad) else { panic!("{bad}") };
            assert_eq!(fallback.builtin.id, DEFAULT_ID, "{bad}");
            assert_eq!(fallback.variant, None);
            assert!(warning.to_string().contains(DEFAULT_SOURCE), "{warning}");
        }
    }

    #[test]
    fn variant_follows_theme_unless_fixed() {
        let s = Selection { builtin: default_builtin(), variant: None };
        assert_eq!(s.variant_for(true), Variant::Dark);
        assert_eq!(s.variant_for(false), Variant::Light);
        let s = Selection { variant: Some(Variant::Dark), ..s };
        assert_eq!(s.variant_for(false), Variant::Dark);
    }

    #[test]
    fn seeds_are_hex() {
        for b in &BUILTINS {
            for v in Variant::ALL {
                let s = b.seed(v);
                assert!(s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{s}");
            }
        }
    }

    #[test]
    fn i18n_catalog_complete() {
        // Dizin ÇALIŞMA ZAMANINDA çözülür: cargo paket yolu değişince test ikilisini yeniden
        // derlemez, derleme anındaki `env!` bayat/başka bir ağacı gösterebilir (yalnızca yedek;
        // docs/ARCHITECTURE.md "Testler").
        let root = std::env::var_os("CARGO_MANIFEST_DIR")
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_MANIFEST_DIR").into());
        let dir = root.join("locales");
        assert!(dir.is_dir(), "katalog dizini yok: {} (ikili silinmiş bir yoldan mı derlendi?)", dir.display());
        nuxut_i18n::check::catalog_embedded(&i18n_source(), dir, "wallpapers").assert_ok();
        for lang in ["tr", "en"] {
            let catalog = nuxut_i18n::Catalog::load(&[i18n_source()], &nuxut_i18n::LanguagePrefs::fixed(&[lang]));
            for b in &BUILTINS {
                let key = format!("wallpapers-{}", b.id);
                assert!(catalog.has(&key), "{lang}: {key}");
            }
        }
    }
}
