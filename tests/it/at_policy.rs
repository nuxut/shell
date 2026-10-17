//! Bekçi (güvenlik, 2026-10-17): erişim veren yüzeyler AT-SPI eylemine açık değil.
//!
//! a11y bus'ı oturumdaki her sürece (Flatpak'te varsayılan izinli) açık; AT eylemi gerçek
//! kullanıcı niyeti değildir (`docs/design/screen-reader.md` "Güvenlik"). Polkit onayı ve ekran
//! paylaşım seçicisi bugün ağaç ve eylem yayımlamaz (`App::accessibility` /
//! `accessibility_action` yok → framework içeriksiz kök verir, eylem uygulamaya ulaşmaz); kilit
//! ekranı yalnız okunan ağaç yayımlar, her eylemi reddeder ("Kilit ekranı", `AT_POLICY:`). Biri
//! eklenirse bu test kırılır: eklenen kod ilgili tasarım belgesindeki AT kuralını (yalnız okuma ve
//! odak; onay, parola ve metin yalnız gerçek girdiyle) uyguladığını `AT_POLICY:` işaretiyle
//! belgelemeli ve düşmanca uçtan uca testi (`examples/at_probe`) eklemeli.

use crate::support::repo::repo_path;

fn sources(dir: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![repo_path(dir)];
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            for e in std::fs::read_dir(&p).unwrap().flatten() {
                stack.push(e.path());
            }
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push((p.display().to_string(), std::fs::read_to_string(&p).unwrap()));
        }
    }
    out
}

#[test]
fn granting_surfaces_publish_no_assistive_actions() {
    let mut files = sources("polkit/src");
    files.extend(sources("lock/src"));
    files.push((
        "src/sharepicker_view.rs".into(),
        std::fs::read_to_string(repo_path("src/sharepicker_view.rs")).unwrap(),
    ));
    assert!(files.len() > 10, "kaynaklar okunamadı: {}", files.len());
    for (path, text) in &files {
        for hook in ["fn accessibility(", "fn accessibility_action("] {
            assert!(
                !text.contains(hook) || text.contains("AT_POLICY:"),
                "{path}: {hook} eklendi ama AT kuralı belgelenmedi (AT_POLICY:); docs/design/screen-reader.md \"Güvenlik\""
            );
        }
    }
}

/// Ekran klavyesi gerçek girdi üretir (sanal klavye odaktaki yüzeye, kilitliyse kilit ekranına
/// yazar): tuşları AT'ye her zaman eylemsiz (`docs/design/osk.md` "Ekran okuyucu ve gerçek girdi").
#[test]
fn osk_keys_never_take_assistive_actions() {
    let view = std::fs::read_to_string(repo_path("src/osk_view.rs")).unwrap();
    let start = view.find("pub fn access(&self, tree: &mut AccessTree, osk: &Osk)").expect("KeyboardView::access yok");
    let body = &view[start..start + view[start..].find("\n    }\n").expect("gövde sonu")];
    for action in ["clickable", ".action(", "Action::Click", "Increment"] {
        assert!(!body.contains(action), "ekran klavyesi tuşu AT eylemi sunuyor: {action}");
    }
    // AT eylemi işleyicisi klavye yüzeyine bir şey yapmaz: ekran klavyesi modülünde AT isteği yok.
    let app = std::fs::read_to_string(repo_path("src/app/osk.rs")).unwrap();
    assert!(!app.contains("AccessRequest") && !app.contains("accessibility_action"), "ekran klavyesi AT eylemi işliyor");
}
