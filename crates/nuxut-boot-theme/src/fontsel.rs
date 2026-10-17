//! Önyükleme fontu seçimi: yalnız sistem font dizinindeki (paket yöneticisinin, kökün sahip
//! olduğu) sabit dosyalardan. Kullanıcı font dizinleri ve fontconfig kullanılmaz: kök yardımcı
//! kullanıcının yazabildiği bir dosyayı `grub-mkfont`'a vermez.
//!
//! Sıra: Nuxut'un arayüz ailesi Inter (DESIGN.md), yoksa yaygın sans aileleri. Değişken fontlarda
//! grub-mkfont varsayılan örneği (Regular) kullanır.

use crate::detect::Probe;

/// `(aile adı, köke göre dosya yolları)`.
pub const UI_FONTS: &[(&str, &[&str])] = &[
    ("Inter", &["usr/share/fonts/inter/Inter-Regular.otf", "usr/share/fonts/inter/InterVariable.ttf", "usr/share/fonts/TTF/Inter-Regular.ttf"]),
    ("Noto Sans", &["usr/share/fonts/noto/NotoSans-Regular.ttf"]),
    ("Adwaita Sans", &["usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf"]),
    ("DejaVu Sans", &["usr/share/fonts/TTF/DejaVuSans.ttf"]),
    ("Liberation Sans", &["usr/share/fonts/liberation/LiberationSans-Regular.ttf"]),
];

pub const MONO_FONTS: &[(&str, &[&str])] = &[
    ("JetBrains Mono", &["usr/share/fonts/TTF/JetBrainsMono-Regular.ttf"]),
    ("JetBrainsMono Nerd Font", &["usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf"]),
    ("Noto Sans Mono", &["usr/share/fonts/noto/NotoSansMono-Regular.ttf"]),
    ("Adwaita Mono", &["usr/share/fonts/Adwaita/AdwaitaMono-Regular.ttf"]),
    ("DejaVu Sans Mono", &["usr/share/fonts/TTF/DejaVuSansMono.ttf"]),
    ("Liberation Mono", &["usr/share/fonts/liberation/LiberationMono-Regular.ttf"]),
];

/// Seçilen font: aile ve köke göre dosya yolu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    pub family: &'static str,
    pub path: &'static str,
}

fn choose(p: &dyn Probe, list: &'static [(&'static str, &'static [&'static str])]) -> Option<Chosen> {
    list.iter().find_map(|(family, paths)| paths.iter().find(|path| p.exists(path)).map(|path| Chosen { family, path }))
}

/// Arayüz ve eş aralıklı font; ikisi de bulunmazsa `None`.
pub fn choose_fonts(p: &dyn Probe) -> Option<(Chosen, Chosen)> {
    Some((choose(p, UI_FONTS)?, choose(p, MONO_FONTS)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::MapProbe;
    use crate::files::valid_family;

    #[test]
    fn prefers_inter_then_falls_back() {
        let p = MapProbe::default()
            .with("usr/share/fonts/noto/NotoSans-Regular.ttf", "")
            .with("usr/share/fonts/TTF/DejaVuSansMono.ttf", "");
        let (ui, mono) = choose_fonts(&p).unwrap();
        assert_eq!(ui.family, "Noto Sans");
        assert_eq!(mono.family, "DejaVu Sans Mono");
        let p = p.with("usr/share/fonts/inter/InterVariable.ttf", "");
        assert_eq!(choose_fonts(&p).unwrap().0.family, "Inter");
        assert_eq!(choose_fonts(&MapProbe::default()), None);
    }

    #[test]
    fn families_are_valid_config_values() {
        for (f, paths) in UI_FONTS.iter().chain(MONO_FONTS) {
            assert!(valid_family(f), "{f}");
            assert!(paths.iter().all(|p| p.starts_with("usr/share/fonts/") && !p.contains("..")));
        }
    }
}
