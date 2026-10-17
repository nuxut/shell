//! Ortam değişkenleri (ADR-0004 §5.4, §5.6; saf): üretilen değerler, `environment.d` dosyası ve
//! çakışma taraması (oturum ortamı, kullanıcının diğer `environment.d` dosyaları).
//!
//! `GTK_THEME` **yazılmaz** (canlı kip geçişini kilitler); ortamda varsa çakışma olarak raporlanır.

use std::path::Path;

use serde::Serialize;

use super::selection::{QtBackend, Resolved};

/// Nuxut'un `environment.d` dosyası. `99-` öneki: eski kozmetik dosyaları (`10-`, `95-`) ezilir
/// (systemd dosyaları ad sırasıyla okur, sonraki kazanır).
pub const ENV_FILE: &str = "99-nuxut-look.conf";
/// Önceki sürümün dosya adı: diskte yönetilen başlıkla duruyorsa look.unified işleminde kaldırılır
/// (göç; snapshot uyumlu, kapatınca ikisi de Nuxut öncesine döner).
pub const LEGACY_ENV_FILE: &str = "90-nuxut-look.conf";
/// Yönetilen dosyanın başlığı (`nuxut-apply` `is_managed` ile tanınır).
pub const ENV_HEADER: &str = "# Nuxut — birleşik görünüm · elle düzenleme bir sonraki uygulamada ezilir"; // i18n: tanı üretilen environment.d dosyasının başlık yorumu (dosya içeriği)

/// Görünümün ilgilendiği değişkenler (çakışma taraması bunlara bakar).
pub const WATCHED: [&str; 9] = [
    "QT_QPA_PLATFORMTHEME",
    "QT_STYLE_OVERRIDE",
    "QT_FONT_DPI",
    "QT_PLUGIN_PATH",
    "XCURSOR_THEME",
    "XCURSOR_SIZE",
    "HYPRCURSOR_THEME",
    "HYPRCURSOR_SIZE",
    "GTK_THEME",
];

/// Yazılacak değişkenler, sabit sırayla. `QT_STYLE_OVERRIDE` boş: istenen değer "zorlama yok";
/// boş olmayan her değer (Kvantum/Darkly) çakışma sayılır. Boş değer `environment.d`'ye
/// yazılmaz (bkz. [`environment_d`]). Qt platform teması kurulu değilse yazılmaz.
///
/// `qt_font_dpi`: metin ölçeğinin qt6ct yoluna tek girişi (`QT_FONT_DPI`, framework
/// `docs/design/accessibility.md` §3; [`super::fonts::qt_font_dpi`]). `None` iken yazılmaz —
/// yönetilen dosyadan düşer, canlı tur `SessionUnset` ile oturumdan kaldırır. `QT_SCALE_FACTOR`
/// asla yazılmaz (metni değil bütün arayüzü ölçekler).
pub fn variables(r: &Resolved, qt_font_dpi: Option<u32>) -> Vec<(&'static str, String)> {
    let mut out = Vec::with_capacity(7);
    if let Some(b) = r.qt_backend {
        out.push(("QT_QPA_PLATFORMTHEME", qpa_name(b).to_owned()));
    }
    out.push(("QT_STYLE_OVERRIDE", String::new()));
    // Nuxut Qt stili (ADR-0009): Qt6 ve Qt5 eklenti dizinleri; her sürüm yalnızca kendi
    // ABI'sinin eklentisini yükler.
    if let Some(dir) = r.qt_plugin_dir.as_deref().and_then(|d| d.to_str()).filter(|d| !d.contains(':') && !d.chars().any(char::is_control)) {
        let qt = |v: &str| format!("{dir}/{v}");
        out.push(("QT_PLUGIN_PATH", format!("{}:{}", qt("qt6"), qt("qt5"))));
    }
    if let Some(dpi) = qt_font_dpi {
        out.push(("QT_FONT_DPI", dpi.to_string()));
    }
    out.extend(cursor_variables(r));
    out
}

/// Yalnızca imleç değişkenleri (`hypr_config` üreticisinin `env` girdisi; §5.4).
pub fn cursor_variables(r: &Resolved) -> [(&'static str, String); 4] {
    let size = r.cursor_size.to_string();
    [
        ("XCURSOR_THEME", r.cursor_theme.clone()),
        ("XCURSOR_SIZE", size.clone()),
        // Hyprcursor biçimi yoksa Hyprland XCursor'a düşer; iki değişken aynı adı alır.
        ("HYPRCURSOR_THEME", r.cursor_theme.clone()),
        ("HYPRCURSOR_SIZE", size),
    ]
}

/// `QT_QPA_PLATFORMTHEME` değeri. Qt5 `plasma5-integration` da `kde` adını kullanır; qt6ct yolunda
/// Qt5 uygulamaları (`qt5ct` adı) bu değişkenle eşleşmez — bilinen v1 sınırı.
pub fn qpa_name(b: QtBackend) -> &'static str {
    match b {
        QtBackend::Kde => "kde",
        QtBackend::Qt6ct | QtBackend::Auto => "qt6ct",
    }
}

/// `environment.d/99-nuxut-look.conf` içeriği. Boş değerli değişkenler atlanır: systemd
/// `NAME=` satırını geçersiz atama sayıp yok sayar (günlüğe uyarı düşer, hiçbir şeyi kaldırmaz).
pub fn environment_d(vars: &[(&'static str, String)]) -> String {
    let mut out = format!("{ENV_HEADER}\n");
    for (k, v) in vars.iter().filter(|(_, v)| !v.is_empty()) {
        out.push_str(k);
        out.push('=');
        out.push_str(v);
        out.push('\n');
    }
    out
}

/// Bir görünüm ayarıyla çelişen dış kaynak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    /// `env` (değişken) · `env_file` (eski Nuxut kozmetiği gibi bütün bir dosya).
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// `session_env` ya da `environment.d/<dosya>:<satır>`.
    pub source: String,
}

/// Değer/kaynak sınırları (ADR §9: değer ≤ 256, yol ≤ 512 bayt, liste ≤ 256 öğe).
const MAX_VALUE: usize = 256;
const MAX_SOURCE: usize = 512;
pub const MAX_CONFLICTS: usize = 256;

fn clip(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].chars().filter(|c| !c.is_control()).collect()
}

/// Bu değer bizim yazacağımızla çelişiyor mu? `wanted` = bizim değerimiz (yoksa: hiç yazmıyoruz,
/// yalnızca `GTK_THEME`, boş olmayan `QT_STYLE_OVERRIDE` ve bizimkinden farklı `QT_FONT_DPI`
/// çakışmadır). Bizim yazmadığımız bir `QT_FONT_DPI` metin ölçeğini ikinci kez uygular (kdeglobals
/// yolunda ölçek zaten QFont pt'sinde): çakışmadır.
fn conflicts_with(name: &str, value: &str, ours: &[(&'static str, String)]) -> bool {
    match name {
        "GTK_THEME" => !value.is_empty(),
        "QT_FONT_DPI" => !value.is_empty() && ours.iter().find(|(k, _)| *k == name).is_none_or(|(_, v)| v != value),
        _ => ours.iter().find(|(k, _)| *k == name).is_some_and(|(_, v)| v != value),
    }
}

/// Shell'in kendi süreç ortamı (Hyprland `env` satırları ve oturum başlatıcısının verdikleri
/// buraya yansır).
pub fn scan_session<'a>(env: impl IntoIterator<Item = (&'a str, &'a str)>, ours: &[(&'static str, String)]) -> Vec<Conflict> {
    env.into_iter()
        .filter(|(k, v)| WATCHED.contains(k) && conflicts_with(k, v, ours))
        .take(MAX_CONFLICTS)
        .map(|(k, v)| Conflict {
            kind: "env",
            name: Some((*k).to_owned()),
            value: Some(clip(v, MAX_VALUE)),
            source: "session_env".into(),
        })
        .collect()
}

/// Bir `environment.d` dosyası. Eski Nuxut kozmetiği dosyası bütünüyle çakışma sayılır.
pub fn scan_env_file(file_name: &str, text: &str, ours: &[(&'static str, String)], out: &mut Vec<Conflict>) {
    if file_name == ENV_FILE || file_name == LEGACY_ENV_FILE {
        return;
    }
    let source = |line: usize| clip(&format!("environment.d/{file_name}:{line}"), MAX_SOURCE);
    if file_name.contains("nuxut-cosmetics") {
        out.push(Conflict { kind: "env_file", name: None, value: None, source: clip(&format!("environment.d/{file_name}"), MAX_SOURCE) });
    }
    for (i, line) in text.lines().enumerate() {
        if out.len() >= MAX_CONFLICTS {
            return;
        }
        let t = line.trim();
        if t.starts_with('#') || t.starts_with(';') {
            continue;
        }
        let Some((k, v)) = t.split_once('=') else { continue };
        let k = k.trim();
        let v = v.trim().trim_matches(['"', '\'']);
        if WATCHED.contains(&k) && conflicts_with(k, v, ours) {
            out.push(Conflict { kind: "env", name: Some(k.to_owned()), value: Some(clip(v, MAX_VALUE)), source: source(i + 1) });
        }
    }
}

/// `~/.config/environment.d` dizinini tarar (en çok 64 dosya, dosya başına 64 KiB).
pub fn scan_env_dir(dir: &Path, ours: &[(&'static str, String)]) -> Vec<Conflict> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    let mut names: Vec<String> = entries
        .flatten()
        .take(64)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".conf"))
        .collect();
    names.sort();
    for name in names {
        let path = dir.join(&name);
        if std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= 64 * 1024)
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            scan_env_file(&name, &text, ours, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use nuxut_palette::Mode;

    use super::*;
    use crate::services::look::selection::{Desktop, resolve, tests::FakeProbe};

    fn resolved() -> Resolved {
        resolve(&Desktop::default(), Mode::Dark, &FakeProbe::full())
    }

    #[test]
    fn variables_and_file() {
        let vars = variables(&resolved(), None);
        let names: Vec<&str> = vars.iter().map(|(k, _)| *k).collect();
        assert_eq!(names, ["QT_QPA_PLATFORMTHEME", "QT_STYLE_OVERRIDE", "XCURSOR_THEME", "XCURSOR_SIZE", "HYPRCURSOR_THEME", "HYPRCURSOR_SIZE"]);
        assert!(names.iter().all(|n| WATCHED.contains(n)));
        assert_eq!(vars[0].1, "kde");
        assert_eq!(vars[1].1, "");
        let text = environment_d(&vars);
        assert!(text.starts_with("# Nuxut — "));
        // Boş değer systemd'de geçersiz atama: dosyaya hiç girmez.
        assert!(!text.contains("QT_STYLE_OVERRIDE"), "{text}");
        assert!(text.lines().skip(1).all(|l| l.split_once('=').is_some_and(|(_, v)| !v.is_empty())), "{text}");
        assert!(text.contains("\nQT_QPA_PLATFORMTHEME=kde\nXCURSOR_THEME=Vimix-cursors\nXCURSOR_SIZE=24\n"));
        // Qt platform teması yok: değişken yazılmaz.
        let none = Resolved { qt_backend: None, ..resolved() };
        assert!(variables(&none, None).iter().all(|(k, _)| *k != "QT_QPA_PLATFORMTHEME"));
    }

    #[test]
    fn conflicts_are_found_and_bounded() {
        let ours = variables(&resolved(), None);
        let session = [("QT_STYLE_OVERRIDE", "kvantum"), ("XCURSOR_THEME", "Vimix-cursors"), ("GTK_THEME", "Adwaita:dark"), ("PATH", "/bin")];
        let c = scan_session(session, &ours);
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!(c[0].name.as_deref(), Some("QT_STYLE_OVERRIDE"));
        assert_eq!(c[1].name.as_deref(), Some("GTK_THEME"));

        let mut out = Vec::new();
        scan_env_file("10-nuxut-cosmetics.conf", "# x\nQT_QPA_PLATFORMTHEME=kde\nQT_STYLE_OVERRIDE=\"Darkly\"\n", &ours, &mut out);
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0].kind, "env_file");
        assert_eq!(out[1].source, "environment.d/10-nuxut-cosmetics.conf:3");
        // Kendi dosyamız çakışma değil.
        let mut own = Vec::new();
        scan_env_file(ENV_FILE, "QT_STYLE_OVERRIDE=kvantum\n", &ours, &mut own);
        assert!(own.is_empty());
        // Uzun değer kırpılır.
        let long = format!("XCURSOR_THEME={}\n", "é".repeat(400));
        let mut l = Vec::new();
        scan_env_file("a.conf", &long, &ours, &mut l);
        assert!(l[0].value.as_ref().unwrap().len() <= MAX_VALUE);
    }

    /// `QT_FONT_DPI`: verildiyse dosyaya yazılır, yoksa hiç geçmez; `QT_SCALE_FACTOR` asla.
    /// Bizim yazmadığımız (ya da farklı) bir değer çift ölçek demektir: çakışma.
    #[test]
    fn qt_font_dpi_written_or_absent() {
        let qt6ct = Resolved { qt_backend: Some(QtBackend::Qt6ct), ..resolved() };
        let with = variables(&qt6ct, Some(118));
        assert!(environment_d(&with).contains("\nQT_FONT_DPI=118\n"));
        let without = variables(&qt6ct, None);
        let text = environment_d(&without);
        assert!(!text.contains("QT_FONT_DPI") && !text.contains("QT_SCALE_FACTOR"), "{text}");
        assert!(scan_session([("QT_FONT_DPI", "118")], &with).is_empty());
        assert_eq!(scan_session([("QT_FONT_DPI", "144")], &with).len(), 1);
        assert_eq!(scan_session([("QT_FONT_DPI", "118")], &without).len(), 1, "kdeglobals yolunda çift ölçek");
        assert!(scan_session([("QT_FONT_DPI", "")], &without).is_empty());
    }

    #[test]
    fn env_dir_scan() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("50-x.conf"), "XCURSOR_SIZE=48\n").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), "XCURSOR_SIZE=48\n").unwrap();
        let c = scan_env_dir(tmp.path(), &variables(&resolved(), None));
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].source, "environment.d/50-x.conf:1");
        assert!(scan_env_dir(&tmp.path().join("yok"), &[]).is_empty());
    }
}
