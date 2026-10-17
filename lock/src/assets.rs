//! Avatar ve duvar kağıdı yollarının çözümü (saf; dosya sistemi denetimi parametreyle).

use std::path::{Path, PathBuf};

/// Avatar adayları sırasıyla: `~/.face`, `~/.face.icon`, AccountsService ikonu.
pub fn avatar_candidates(home: &Path, user: &str) -> Vec<PathBuf> {
    let mut v = vec![home.join(".face"), home.join(".face.icon")];
    // Kullanıcı adı yol bileşeni olarak güvenli olmalı (getpwuid'den gelir ama yine de).
    if !user.is_empty() && !user.contains('/') && user != "." && user != ".." {
        v.push(Path::new("/var/lib/AccountsService/icons").join(user));
    }
    v
}

/// İlk var olan normal dosya.
pub fn first_file(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

/// Kilidin duvar kağıdı kaynağı: dosya ya da yerleşik `nuxut:<id>[/dark|light]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WallpaperSource {
    File(PathBuf),
    Builtin(nuxut_wallpapers::Selection),
}

impl WallpaperSource {
    /// Boş dizge `None`; `nuxut:` önekliyse yerleşik (bilinmeyen kimlik → varsayılan + uyarı
    /// `warn`'a), değilse `~/` açılmış dosya yolu.
    pub fn parse(s: &str, warn: &mut dyn FnMut(String)) -> Option<Self> {
        if s.is_empty() || s.contains('\0') {
            return None;
        }
        Some(match nuxut_wallpapers::parse_source(s) {
            nuxut_wallpapers::Parsed::NotBuiltin => Self::File(expand_home(s)),
            nuxut_wallpapers::Parsed::Builtin(sel) => Self::Builtin(sel),
            nuxut_wallpapers::Parsed::Invalid { fallback, warning } => {
                warn(warning.to_string());
                Self::Builtin(fallback)
            }
        })
    }
}

fn expand_home(s: &str) -> PathBuf {
    match s.strip_prefix("~/") {
        Some(rest) => std::env::home_dir().map_or_else(|| PathBuf::from(s), |h| h.join(rest)),
        None => PathBuf::from(s),
    }
}

/// shell.toml'daki genel `[wallpaper] path`'in kilitteki karşılığı (shell ile aynı kural):
/// dosya yoksa ya da anahtar yazılmamışsa yerleşik varsayılan (taze kurulum); `""` → `None`
/// (shell duvar kağıdı çizmiyor); okunamayan/bozuk dosya → varsayılan (shell de varsayılanla
/// açılır). Monitöre özel geçersiz kılmalar (`[wallpaper.outputs]`) kilitte yok sayılır.
pub fn shell_wallpaper(text: Option<&str>, warn: &mut dyn FnMut(String)) -> Option<WallpaperSource> {
    let default = || Some(WallpaperSource::Builtin(nuxut_wallpapers::Selection { builtin: nuxut_wallpapers::default_builtin(), variant: None }));
    let Some(text) = text else { return default() };
    let Ok(table) = text.parse::<toml::Table>() else { return default() };
    match table.get("wallpaper").and_then(|w| w.get("path")) {
        None => default(),
        Some(v) => match v.as_str() {
            Some(s) if s.len() <= 4096 => WallpaperSource::parse(s, warn),
            _ => default(),
        },
    }
}

/// Kilidin duvar kağıdı: `lock.toml wallpaper` → shell.toml `[wallpaper] path` (masaüstüyle aynı
/// kaynak; yerleşik varsayılan dahil) → palet görseli (`palette.toml [meta] image`; shell
/// duvar kağıdı çizmiyorsa) → yok (düz yüzey). Palet görseli de aynı ayrıştırıcıdan geçer:
/// Ayarlar yerleşik seçince `[meta] image = "nuxut:<id>"` yazar (sözleşme wallpaper-builtins-v1
/// §4); bilinmeyen kimlik → varsayılan + `warn`.
pub fn resolve_wallpaper(
    configured: Option<&WallpaperSource>,
    shell: Option<WallpaperSource>,
    palette_image: Option<&str>,
    warn: &mut dyn FnMut(String),
) -> Option<WallpaperSource> {
    configured.cloned().or(shell).or_else(|| palette_image.and_then(|s| WallpaperSource::parse(s, warn)))
}

/// shell.toml metni (yoksa ya da okunamazsa `None`; 1 MiB sınırı).
pub fn shell_toml() -> Option<String> {
    use std::io::Read as _;
    let path = nuxut::config::config_path("shell")?;
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(1 << 20).read_to_string(&mut text).ok()?;
    Some(text)
}

/// `palette.toml`'daki görsel yolu (yoksa ya da okunamazsa `None`).
pub fn palette_image() -> Option<String> {
    let path = nuxut::theme::palette_path()?;
    nuxut::theme::GeneratedPalette::load(&path)
        .ok()?
        .value
        .meta
        .image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_order_and_unsafe_user() {
        let c = avatar_candidates(Path::new("/home/u"), "u");
        assert_eq!(
            c,
            [
                PathBuf::from("/home/u/.face"),
                PathBuf::from("/home/u/.face.icon"),
                PathBuf::from("/var/lib/AccountsService/icons/u")
            ]
        );
        assert_eq!(avatar_candidates(Path::new("/h"), "../x").len(), 2);
        assert_eq!(avatar_candidates(Path::new("/h"), "..").len(), 2);
    }

    #[test]
    fn first_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::write(&b, "x").unwrap();
        assert_eq!(first_file(&[a, b.clone()]), Some(b));
        assert_eq!(first_file(&[]), None);
    }

    #[test]
    fn wallpaper_precedence() {
        let mut warnings = Vec::new();
        let mut warn = |w: String| warnings.push(w);
        let file = |p: &str| WallpaperSource::File(PathBuf::from(p));
        let a = file("/a");
        let shell = Some(file("/s"));
        assert_eq!(resolve_wallpaper(Some(&a), shell.clone(), Some("/b"), &mut warn), Some(file("/a")));
        assert_eq!(resolve_wallpaper(None, shell, Some("/b"), &mut warn), Some(file("/s")), "masaüstüyle aynı kaynak");
        assert_eq!(resolve_wallpaper(None, None, Some("/b"), &mut warn), Some(file("/b")));
        assert_eq!(resolve_wallpaper(None, None, Some(""), &mut warn), None);
        assert_eq!(resolve_wallpaper(None, None, None, &mut warn), None);
        // Palet görseli yerleşik adres olabilir (Ayarlar yazar; §4).
        assert_eq!(resolve_wallpaper(None, None, Some("nuxut:cloud-rest"), &mut warn), Some(builtin("cloud-rest", None)));
        assert_eq!(
            resolve_wallpaper(None, None, Some("nuxut:chase/light"), &mut warn),
            Some(builtin("chase", Some(nuxut_wallpapers::Variant::Light)))
        );
        assert_eq!(resolve_wallpaper(None, None, Some("nuxut:yok"), &mut warn), Some(builtin("pair-flight", None)));
        assert!(warnings.len() == 1 && warnings[0].contains("yok"), "{warnings:?}");
    }

    fn builtin(id: &str, variant: Option<nuxut_wallpapers::Variant>) -> WallpaperSource {
        WallpaperSource::Builtin(nuxut_wallpapers::Selection { builtin: nuxut_wallpapers::find(id).unwrap(), variant })
    }

    #[test]
    fn shell_wallpaper_matches_shell_rules() {
        let mut warnings = Vec::new();
        let mut warn = |w: String| warnings.push(w);
        let default = Some(builtin("pair-flight", None));
        // Taze kurulum (dosya yok), anahtar yok, bozuk dosya: yerleşik varsayılan.
        assert_eq!(shell_wallpaper(None, &mut warn), default);
        assert_eq!(shell_wallpaper(Some("[bar]\nposition = \"top\"\n"), &mut warn), default);
        assert_eq!(shell_wallpaper(Some("[wallpaper\n"), &mut warn), default);
        assert_eq!(shell_wallpaper(Some("[wallpaper]\nfit = \"contain\"\n"), &mut warn), default);
        // Açıkça boş: shell çizmiyor → kilit palet görseline düşer.
        assert_eq!(shell_wallpaper(Some("[wallpaper]\npath = \"\"\n"), &mut warn), None);
        assert_eq!(
            shell_wallpaper(Some("[wallpaper]\npath = \"nuxut:aurora/light\"\n"), &mut warn),
            Some(builtin("aurora", Some(nuxut_wallpapers::Variant::Light)))
        );
        assert_eq!(
            shell_wallpaper(Some("[wallpaper]\npath = \"/srv/a.png\"\n[wallpaper.outputs.\"DP-1\"]\npath = \"nuxut:climb\"\n"), &mut warn),
            Some(WallpaperSource::File("/srv/a.png".into()))
        );
        assert_eq!(shell_wallpaper(Some("[wallpaper]\npath = \"nuxut:yok\"\n"), &mut warn), default);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }
}
