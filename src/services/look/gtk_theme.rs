//! Nuxut GTK teması (canlı GTK3 renkleri; docs/design/look-gtk.md). Saf.
//!
//! GTK3 kullanıcı `gtk.css`'ini yalnızca açılışta okur; tema CSS'ini ise `gtk-theme` adı değişince
//! yeniden yükler (izole sway'de ölçüldü). Bu yüzden renkler kullanıcı dosyasına değil, Nuxut'un
//! kendi temasına (`$XDG_DATA_HOME/themes/Nuxut`) yazılır ve her renk değişiminde ad kısa süre
//! eşdeğer ikize (`Nuxut-reload`) alınıp geri verilir.
//!
//! İkinci engel gölgeleme: kullanıcı sağlayıcısı (`~/.config/gtk-3.0/gtk.css`, ör. matugen
//! çıktısı) aynı adlı `@define-color` tanımlarsa tema renklerini ezer (öncelik sırası; ölçüldü).
//! Kullanıcının dosyasına dokunmadan çözüm: taban temanın (adw-gtk3) CSS'i, bütün renk adları
//! [`PREFIX`] önekiyle yeniden adlandırılarak kopyalanır; tema kuralları yalnızca önekli adlara
//! bakar, kullanıcının adları hiçbir kurala ulaşmaz. Göreli `url()`'ler taban dizine mutlak
//! `file://` olarak çözülür. Kullanıcının kendi kuralları (renk dışı) yine uygulanır.

use std::fmt::Write as _;
use std::path::Path;

/// Yeniden adlandırılmış renklerin öneki.
pub const PREFIX: &str = "nuxut_";
/// Temanın adı ve canlı yeniden yükleme ikizi.
pub const THEME: &str = "Nuxut";
pub const RELOAD_THEME: &str = "Nuxut-reload";
/// Taban CSS üst sınırı (adw-gtk3 ≈ 300 KB; nuxut-apply dosya sınırı 1 MiB).
pub const MAX_BASE_BYTES: usize = 900 * 1024;

/// Yeniden adlandırılmayan @-kuralları.
const AT_RULES: [&str; 8] = ["import", "define-color", "binding-set", "keyframes", "media", "charset", "namespace", "font-face"];

fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// `url()` argümanı göreliyse taban dizine mutlak `file://` URL'si; güvensiz karakterli yolda
/// `None` (olduğu gibi bırakılır).
fn absolute_url(arg: &str, base_dir: &Path) -> Option<String> {
    if arg.is_empty() || arg.contains(':') || arg.starts_with('/') {
        return None;
    }
    let dir = base_dir.to_str()?;
    let bad = |s: &str| s.chars().any(|c| c.is_control() || matches!(c, '"' | '\'' | '(' | ')' | '\\'));
    if bad(dir) || bad(arg) || arg.split('/').any(|p| p == "..") {
        return None;
    }
    Some(format!("file://{dir}/{arg}"))
}

/// Taban tema CSS'ini öneklenmiş renk adlarıyla yeniden yazar. Yorumlar ve dizgeler korunur;
/// `@define-color ad` ve `@ad` başvuruları `@nuxut_ad` olur, göreli `url()` mutlak olur.
/// `@import` içeren taban desteklenmez (`None`: çağıran eski yola düşer).
pub fn rewrite_base(css: &str, base_dir: &Path) -> Option<String> {
    let mut out = String::with_capacity(css.len() + css.len() / 16);
    let mut it = css.char_indices().peekable();
    let bytes = css.as_bytes();
    while let Some((i, c)) = it.next() {
        match c {
            '/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = css[i + 2..].find("*/").map_or(css.len(), |e| i + 2 + e + 2);
                out.push_str(&css[i..end]);
                while it.peek().is_some_and(|(j, _)| *j < end) {
                    it.next();
                }
            }
            '"' | '\'' => {
                let end = css[i + 1..].find(c).map_or(css.len(), |e| i + 1 + e + 1);
                out.push_str(&css[i..end]);
                while it.peek().is_some_and(|(j, _)| *j < end) {
                    it.next();
                }
            }
            'u' if css[i..].starts_with("url(") && !css[..i].ends_with(ident_char) => {
                let close = css[i..].find(')').map(|e| i + e)?;
                let raw = css[i + 4..close].trim();
                let arg = raw.trim_matches(|q| q == '"' || q == '\'');
                match absolute_url(arg, base_dir) {
                    Some(abs) => {
                        let _ = write!(out, "url(\"{abs}\")");
                    }
                    None => out.push_str(&css[i..=close]),
                }
                while it.peek().is_some_and(|(j, _)| *j <= close) {
                    it.next();
                }
            }
            '@' => {
                let start = i + 1;
                let len = css[start..].find(|ch: char| !ident_char(ch)).unwrap_or(css.len() - start);
                let name = &css[start..start + len];
                if name == "import" {
                    return None;
                }
                out.push('@');
                if name == "define-color" {
                    out.push_str(name);
                    // Boşluk + tanımlanan ad.
                    let rest = &css[start + len..];
                    let ws = rest.len() - rest.trim_start().len();
                    out.push_str(&rest[..ws]);
                    let id_start = start + len + ws;
                    let id_len = css[id_start..].find(|ch: char| !ident_char(ch)).unwrap_or(css.len() - id_start);
                    out.push_str(PREFIX);
                    out.push_str(&css[id_start..id_start + id_len]);
                    while it.peek().is_some_and(|(j, _)| *j < id_start + id_len) {
                        it.next();
                    }
                } else {
                    if !name.is_empty() && !AT_RULES.contains(&name) {
                        out.push_str(PREFIX);
                    }
                    out.push_str(name);
                    while it.peek().is_some_and(|(j, _)| *j < start + len) {
                        it.next();
                    }
                }
            }
            _ => out.push(c),
        }
    }
    Some(out)
}

/// Nuxut'un GTK3 renk satırlarını (`@define-color ad değer;`) öneklenmiş adlara çevirir.
pub fn prefixed_colors(gtk3_css: &str) -> String {
    let mut out = String::new();
    for line in gtk3_css.lines() {
        if let Some(rest) = line.strip_prefix("@define-color ") {
            let _ = writeln!(out, "@define-color {PREFIX}{rest}");
        }
    }
    out
}

/// Nuxut temasının tabanı: kurulu adw-gtk3 (kipe göre) ve öneklenmiş GTK 3 CSS'i.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtkBase {
    pub name: String,
    pub dir: std::path::PathBuf,
    /// [`rewrite_base`] çıktısı.
    pub css3: String,
}

impl GtkBase {
    /// Taban temayı okur (bloklayan IO, sınırlı; görünüm thread'i). GTK 3 CSS'i okunamaz, çok
    /// büyük, `@import`'lu ya da GTK 4 CSS'i yoksa `None` (eski yol).
    pub fn load(name: &str, dir: &Path) -> Option<Self> {
        if !base_css_path(dir, true).is_file() {
            return None;
        }
        let path = base_css_path(dir, false);
        let len = std::fs::metadata(&path).ok()?.len();
        if len > MAX_BASE_BYTES as u64 {
            return None;
        }
        let css = std::fs::read_to_string(&path).ok()?;
        let css3 = rewrite_base(&css, &dir.join("gtk-3.0"))?;
        (css3.len() <= MAX_BASE_BYTES).then(|| Self { name: name.to_owned(), dir: dir.to_path_buf(), css3 })
    }
}

/// Önbellek anahtarı: taban dizini, ad ve iki CSS'in değişim zamanı/boyu.
type BaseKey = (std::path::PathBuf, String, [Option<(std::time::SystemTime, u64)>; 2]);
static BASE_CACHE: std::sync::Mutex<Option<(BaseKey, Option<GtkBase>)>> = std::sync::Mutex::new(None);

/// [`GtkBase::load`], taban CSS'leri değişmedikçe önbellekten: `msg look status` her istekte
/// görünüm durumunu yeniden kurar, ≈ 300 KB'lik taban CSS'i her seferinde okunup yeniden
/// yazılmasın.
pub fn load_cached(name: &str, dir: &Path) -> Option<GtkBase> {
    let stamp = |gtk4: bool| std::fs::metadata(base_css_path(dir, gtk4)).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
    let key: BaseKey = (dir.to_path_buf(), name.to_owned(), [stamp(false), stamp(true)]);
    let mut slot = BASE_CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((k, v)) = slot.as_ref()
        && *k == key
    {
        return v.clone();
    }
    let v = GtkBase::load(name, dir);
    *slot = Some((key, v.clone()));
    v
}

/// Yönetilen dosya başlıkları (`nuxut-apply` `is_managed`).
pub fn css_header(what: &str) -> String {
    format!("/* Nuxut — {what} · elle düzenleme bir sonraki uygulamada ezilir */\n") // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
}

pub fn ini_header(what: &str) -> String {
    format!("# Nuxut — {what} · elle düzenleme bir sonraki uygulamada ezilir\n") // i18n: tanı üretilen dosyanın başlık yorumu (dosya içeriği)
}

/// Taban dizinindeki CSS'in GTK 3 ya da 4 dosyası (`gtk-<v>/gtk.css`).
pub fn base_css_path(base_dir: &Path, gtk4: bool) -> std::path::PathBuf {
    base_dir.join(if gtk4 { "gtk-4.0" } else { "gtk-3.0" }).join("gtk.css")
}

/// `themes/Nuxut/gtk-3.0/gtk.css`: yeniden adlandırılmış taban + Nuxut renkleri (sonra gelen
/// tanım kazanır).
pub fn gtk3_theme_css(header: &str, colors: &str) -> String {
    format!("{header}@import url(\"base.css\");\n{colors}")
}

/// `themes/Nuxut/gtk-4.0/gtk.css`: taban GTK 4 CSS'i (renkler libadwaita değişkenleriyle kullanıcı
/// `gtk-4.0/nuxut.css`'ten gelir).
pub fn gtk4_theme_css(header: &str, base_dir: &Path) -> Option<String> {
    let url = absolute_url("gtk-4.0/gtk.css", base_dir)?;
    Some(format!("{header}@import url(\"{url}\");\n"))
}

/// İkiz tema: aynı içeriği asıl temadan alır (ad değişimi GTK'yı yeniden yüklemeye zorlar, görüntü
/// aynı kalır).
pub fn reload_css(header: &str, version: &str) -> String {
    format!("{header}@import url(\"../../{THEME}/gtk-{version}/gtk.css\");\n")
}

/// `index.theme` (tema seçicilerde ad).
pub fn index_theme(header: &str, name: &str) -> String {
    format!("{header}[Desktop Entry]\nType=X-GNOME-Metatheme\nName={name}\nComment=Nuxut\n\n[X-GNOME-Metatheme]\nGtkTheme={name}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = "/usr/share/themes/adw-gtk3-dark/gtk-3.0";

    #[test]
    fn rewrites_names_urls_and_keeps_comments_and_strings() {
        let css = "/* @window_bg_color yorum */\n@define-color window_bg_color #242424;\n@define-color  view_fg_color @window_fg_color;\n\
                   .x { color: shade(@window_bg_color, 0.9); background-image: url(\"assets/a@2.png\"); }\n\
                   .y { -gtk-icon-source: -gtk-recolor(url(assets/b.svg)); font-family: \"@not\"; }\n\
                   @keyframes spin { to { -gtk-icon-transform: rotate(1turn); } }\n.z { background: url(\"resource:///x.png\"); }\n";
        let out = rewrite_base(css, Path::new(DIR)).unwrap();
        assert!(out.starts_with("/* @window_bg_color yorum */\n"), "{out}");
        assert!(out.contains("@define-color nuxut_window_bg_color #242424;"));
        assert!(out.contains("@define-color  nuxut_view_fg_color @nuxut_window_fg_color;"));
        assert!(out.contains("shade(@nuxut_window_bg_color, 0.9)"));
        assert!(out.contains(&format!("url(\"file://{DIR}/assets/a@2.png\")")), "{out}");
        assert!(out.contains(&format!("url(\"file://{DIR}/assets/b.svg\")")));
        assert!(out.contains("\"@not\""), "dizgeye dokunulmaz");
        assert!(out.contains("@keyframes spin"));
        assert!(out.contains("url(\"resource:///x.png\")"), "mutlak URL kalır");
    }

    #[test]
    fn rejects_imports_and_unsafe_paths() {
        assert!(rewrite_base("@import url(\"x.css\");", Path::new(DIR)).is_none());
        assert_eq!(absolute_url("../x.png", Path::new(DIR)), None);
        assert_eq!(absolute_url("a\".png", Path::new(DIR)), None);
        assert_eq!(absolute_url("a.png", Path::new("/t/a)b")), None);
        // Kapanmayan yorum/dizge/url çökme olmadan biter.
        for bad in ["/* yarım", "\"yarım", "url(yarım", "@", "@define-color", "a { color: @"] {
            let _ = rewrite_base(bad, Path::new(DIR));
        }
    }

    #[test]
    fn colors_are_prefixed() {
        let c = prefixed_colors("/* Nuxut — x */\n@define-color window_bg_color #101418;\n@define-color accent_color #aac7ff;\n");
        assert_eq!(c, "@define-color nuxut_window_bg_color #101418;\n@define-color nuxut_accent_color #aac7ff;\n");
    }

    /// Canlı deneme için (elle): `NUXUT_LOOK_EXPORT=<dizin> cargo test export_theme_for_live_check
    /// -- --ignored`: kurulu adw-gtk3-dark'tan `<dizin>/Nuxut` ve `Nuxut-reload` temalarını üretir
    /// (renk satırları `<dizin>/colors.css`'ten).
    #[test]
    #[ignore]
    fn export_theme_for_live_check() {
        let out = Path::new(&std::env::var("NUXUT_LOOK_EXPORT").expect("NUXUT_LOOK_EXPORT")).to_path_buf();
        let base = GtkBase::load("adw-gtk3-dark", Path::new("/usr/share/themes/adw-gtk3-dark")).expect("adw-gtk3-dark");
        let colors = std::fs::read_to_string(out.join("colors.css")).unwrap_or_default();
        let h = css_header("deneme");
        for (rel, text) in [
            ("Nuxut/gtk-3.0/base.css", format!("{h}{}", base.css3)),
            ("Nuxut/gtk-3.0/gtk.css", gtk3_theme_css(&h, &prefixed_colors(&colors))),
            ("Nuxut/gtk-4.0/gtk.css", gtk4_theme_css(&h, &base.dir).unwrap()),
            ("Nuxut/index.theme", index_theme(&ini_header("deneme"), THEME)),
            ("Nuxut-reload/gtk-3.0/gtk.css", reload_css(&h, "3.0")),
            ("Nuxut-reload/gtk-4.0/gtk.css", reload_css(&h, "4.0")),
            ("Nuxut-reload/index.theme", index_theme(&ini_header("deneme"), RELOAD_THEME)),
        ] {
            let p = out.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
    }

    /// Kurulu adw-gtk3 ile: her öneklenmiş başvurunun tanımı var, öneksiz başvuru kalmadı.
    #[test]
    fn real_adw_gtk3_rewrite_is_self_contained() {
        for theme in ["adw-gtk3", "adw-gtk3-dark"] {
            let dir = Path::new("/usr/share/themes").join(theme);
            let Ok(css) = std::fs::read_to_string(base_css_path(&dir, false)) else { continue };
            let out = rewrite_base(&css, &dir.join("gtk-3.0")).expect(theme);
            let defined: std::collections::HashSet<&str> = out
                .match_indices("@define-color ")
                .filter_map(|(i, m)| out[i + m.len()..].split(|c: char| !ident_char(c)).next())
                .collect();
            let mut refs = 0;
            for (i, _) in out.match_indices('@') {
                let name: String = out[i + 1..].chars().take_while(|c| ident_char(*c)).collect();
                if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) || AT_RULES.contains(&name.as_str()) {
                    continue;
                }
                assert!(name.starts_with(PREFIX), "{theme}: öneksiz başvuru @{name}");
                refs += 1;
            }
            assert!(refs > 100 && defined.len() > 50, "{theme}: {refs} {}", defined.len());
            assert!(!out.contains("url(\"assets/"), "{theme}: göreli url kaldı");
        }
    }
}
