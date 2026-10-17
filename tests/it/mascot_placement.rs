//! Bekçi: maskot Fiyu (`nuxut::ui::Mascot`, `Mood`) yalnızca onaylı boş durum / sonuç yok / hata
//! yüzeylerinde (docs/design/mascot-placement.md; framework docs/design/mascot.md). ASLA bar,
//! kilit ekranı, parola/kimlik doğrulama (polkit) ve uygulama ikonunda.
//!
//! Kural izin listesiyle: [`ALLOWED`] dışındaki her `.rs` dosyasında (`src/`, `lock/`,
//! `polkit/`, `portal/`, `examples/`) maskot tanımlayıcısı yasaktır. Yeni bir yerleşim, önce
//! yerleşim belgesine yazılıp buraya eklenir. Kilit ve polkit ayrıca adıyla da denetlenir: izin
//! listesine yanlışlıkla eklenseler bile test kırılır.

use std::path::{Path, PathBuf};

use crate::support::repo;

/// Maskotu kullanabilen dosyalar (kök göreli).
const ALLOWED: &[&str] = &[
    "src/widgets/notifications_panel.rs",
    "src/launcher_view.rs",
    // "Güncelleme bitti" bildirimi (Celebrate): dahili bildirim modeli, kart çizimi, gönderen.
    "src/services/notifications/internal.rs",
    "src/widgets/notification_cards.rs",
    "src/app/update_notice.rs",
    // Dosya seçici: boş klasör, aramada sonuç yok, okunamayan klasör (görev 2026-10-15).
    "src/filechooser_view.rs",
];

/// Maskotun asla girmeyeceği ağaçlar: kilit ekranı, polkit kimlik doğrulama ajanı, bar
/// widget'ları (bar yüzeyi `src/bar_*.rs` ve bar'da duran widget dosyaları).
const FORBIDDEN: &[&str] = &[
    "lock/",
    "polkit/",
    "crates/nuxut-pam/",
    "src/bar_damage.rs",
    "src/bar_edge.rs",
    "src/widgets/workspaces/",
    "src/widgets/clock.rs",
    "src/widgets/status.rs",
    "src/widgets/status/",
    "src/widgets/tray.rs",
    "src/widgets/active_window.rs",
    "src/widgets/logo.rs",
    "src/widgets/icon_button.rs",
];

/// Maskot kullanımını ele veren tanımlayıcılar.
const NEEDLES: &[&str] = &["Mascot", "mascot_image", "mascot_colors", "Mood::"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn uses_mascot(src: &str) -> bool {
    NEEDLES.iter().any(|n| src.contains(n))
}

#[test]
fn mascot_only_on_allowed_surfaces() {
    let root = &repo::manifest_dir();
    let mut files = Vec::new();
    for dir in ["src", "lock", "polkit", "portal", "examples", "crates"] {
        rust_files(&root.join(dir), &mut files);
    }
    repo::assert_enough_sources(root, files.len(), repo::MIN_SHELL_SOURCES);
    let mut bad = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        if !uses_mascot(&src) {
            continue;
        }
        if FORBIDDEN.iter().any(|p| rel.starts_with(p)) || !ALLOWED.contains(&rel.as_str()) {
            bad.push(rel);
        }
    }
    assert!(bad.is_empty(), "maskot onaysız yüzeyde (docs/design/mascot-placement.md): {bad:?}");
    for allowed in ALLOWED {
        assert!(!FORBIDDEN.iter().any(|p| allowed.starts_with(p)), "{allowed} yasak ağaçta");
        let src = std::fs::read_to_string(root.join(allowed)).expect("izinli dosya var");
        assert!(uses_mascot(&src), "{allowed} artık maskot kullanmıyor: izin listesinden çıkar");
    }
}

#[test]
fn lock_and_polkit_never_use_mascot() {
    let root = &repo::manifest_dir();
    for dir in ["lock", "polkit"] {
        let mut files = Vec::new();
        rust_files(&root.join(dir), &mut files);
        assert!(!files.is_empty(), "{dir} taranamadı");
        for f in files {
            let src = std::fs::read_to_string(&f).unwrap();
            assert!(!uses_mascot(&src), "{} maskot kullanıyor", f.display());
        }
    }
}
