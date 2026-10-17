//! Varsayılan uygulamalar: çözüm sırası, kaldırılan ilişkiler, gölge, karışık, aday süzgeçleri,
//! yazım (koruma, yaratma, değişiklik yok), hata kodları, snapshot geri yükleme, terminal.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use nuxut_apply::{Env, RestoreMode, SnapshotId};

use super::*;
use crate::services::apply;
use crate::services::launcher::desktop::Locale;

struct Fx {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    env: Env,
    dirs: Dirs,
}

impl Fx {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let env = Env::for_test(&root);
        let data_home = root.join("data_home");
        let scan = ScanConfig {
            data_dirs: vec![data_home.clone(), root.join("sys")],
            desktops: vec!["Hyprland".into()],
            locale: Locale::default(),
            path: vec![root.join("bin")],
        };
        let dirs = Dirs::new(scan, env.config_home.clone(), vec![root.join("etc")], Some(data_home));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        Self { _tmp: tmp, root, env, dirs }
    }

    fn write(&self, rel: &str, body: &str) -> PathBuf {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        p
    }

    /// Sistem veri dizinine uygulama.
    fn app(&self, id: &str, name: &str, extra: &str) {
        self.write(&format!("sys/applications/{id}"), &format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\n{extra}"));
    }

    fn exe(&self, name: &str) {
        let p = self.write(&format!("bin/{name}"), "#!/bin/sh\n");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn index(&self) -> Index {
        Index::new(Scanner::new().scan_index(&self.dirs.scan))
    }

    fn status(&self) -> Status {
        status(&self.env, &self.dirs, &self.index())
    }

    fn role(&self, name: &str) -> RoleStatus {
        self.status().roles.into_iter().find(|r| r.role == name).unwrap()
    }

    fn set(&self, role: &str, id: &str) -> Result<SetOk, SetError> {
        set(&self.env, &self.dirs, &self.index(), role, id)
    }

    fn user_list(&self) -> PathBuf {
        self.root.join("config/mimeapps.list")
    }

    fn snapshots(&self) -> usize {
        let v: serde_json::Value = serde_json::from_str(&apply::snapshot_list(&self.env, None, 200).unwrap()).unwrap();
        v.as_array().unwrap().len()
    }
}

fn cur(r: &RoleStatus) -> Option<&str> {
    r.current.as_ref().map(|c| c.id.as_str())
}

#[test]
fn resolution_order_and_installed_check() {
    let fx = Fx::new();
    for id in ["a", "b", "c", "d", "e", "f"] {
        fx.app(&format!("{id}.desktop"), id, "MimeType=application/pdf;\n");
    }
    let m = |d: &str| format!("[Default Applications]\napplication/pdf={d}.desktop;\n");
    // Önbellek yedeği: hiçbir liste yokken.
    fx.write("sys/applications/mimeinfo.cache", "[MIME Cache]\napplication/pdf=missing.desktop;f.desktop;\n");
    let r = fx.role("pdf");
    assert_eq!((cur(&r), r.source), (Some("f.desktop"), "system_cache"), "kurulu olmayan atlanır");
    fx.write("sys/applications/mimeapps.list", &m("e"));
    assert_eq!(cur(&fx.role("pdf")), Some("e.desktop"));
    fx.write("data_home/applications/mimeapps.list", &m("d"));
    assert_eq!((cur(&fx.role("pdf")), fx.role("pdf").source), (Some("d.desktop"), "user"));
    fx.write("etc/mimeapps.list", &m("c"));
    assert_eq!((cur(&fx.role("pdf")), fx.role("pdf").source), (Some("c.desktop"), "system"));
    fx.write("config/mimeapps.list", "[Default Applications]\napplication/pdf=gone.desktop;b.desktop;\n");
    assert_eq!((cur(&fx.role("pdf")), fx.role("pdf").source), (Some("b.desktop"), "user"), "ilk KURULU giriş");
    fx.write("config/hyprland-mimeapps.list", &m("a"));
    let r = fx.role("pdf");
    assert_eq!((cur(&r), r.source), (Some("a.desktop"), "desktop_user"));
    assert!(r.shadowed_by.as_deref().is_some_and(|p| p.ends_with("config/hyprland-mimeapps.list")), "{r:?}");
    // Hiçbir şey: current null, source none.
    let fx2 = Fx::new();
    let r = fx2.role("pdf");
    assert_eq!((r.current.clone(), r.source), (None, "none"));
}

#[test]
fn removed_associations_apply_to_later_files_and_cache() {
    let fx = Fx::new();
    for id in ["a", "b"] {
        fx.app(&format!("{id}.desktop"), id, "MimeType=text/plain;\n");
    }
    fx.write("sys/applications/mimeapps.list", "[Default Applications]\ntext/plain=a.desktop;\n");
    fx.write("sys/applications/mimeinfo.cache", "[MIME Cache]\ntext/plain=a.desktop;b.desktop;\n");
    assert_eq!(cur(&fx.role("text_editor")), Some("a.desktop"));
    fx.write("config/mimeapps.list", "# yorum\n[Removed Associations]\ntext/plain=a.desktop;\n");
    let r = fx.role("text_editor");
    assert_eq!((cur(&r), r.source), (Some("b.desktop"), "system_cache"), "sistem varsayılanı ve önbellekteki a kaldırıldı");
    // Aynı dosyadaki açık varsayılan kaldırmadan güçlü.
    fx.write("config/mimeapps.list", "[Removed Associations]\ntext/plain=a.desktop;\n[Default Applications]\ntext/plain=a.desktop;\n");
    assert_eq!(cur(&fx.role("text_editor")), Some("a.desktop"));
}

#[test]
fn mixed_and_set_unifies() {
    let fx = Fx::new();
    fx.app("img.desktop", "Img", "MimeType=image/png;image/jpeg;image/x-other;\n");
    fx.app("other.desktop", "Other", "MimeType=image/jpeg;\n");
    fx.write(
        "config/mimeapps.list",
        "[Default Applications]\nimage/png=img.desktop;\nimage/jpeg=other.desktop;\n",
    );
    let r = fx.role("image");
    assert_eq!(cur(&r), Some("img.desktop"));
    assert!(r.mixed);
    let ok = fx.set("image", "img.desktop").unwrap();
    assert_eq!(ok.written, ["image/jpeg"], "yalnızca değişen ve uygulamanın bildirdiği");
    assert!(ok.snapshot.is_some());
    assert!(!fx.role("image").mixed);
    let text = std::fs::read_to_string(fx.user_list()).unwrap();
    assert!(!text.contains("image/x-other"), "küme dışı MIME yazılmaz");
}

#[test]
fn candidate_filters_and_order() {
    let fx = Fx::new();
    let pdf = "MimeType=application/pdf;\n";
    fx.app("zeta.desktop", "Zeta", pdf);
    fx.app("alpha.desktop", "alpha", pdf);
    fx.app("handler.desktop", "Aaa Handler", &format!("{pdf}NoDisplay=true\n"));
    fx.app("hidden.desktop", "Hidden", &format!("{pdf}Hidden=true\n"));
    fx.app("tryno.desktop", "TryNo", &format!("{pdf}TryExec=not-installed-prog\n"));
    fx.app("tryok.desktop", "TryOk", &format!("{pdf}TryExec=okprog\n"));
    fx.exe("okprog");
    fx.app("nomime.desktop", "NoMime", "MimeType=text/plain;\n");
    fx.app("masked.desktop", "Masked", pdf);
    // Kullanıcı veri dizinindeki Hidden girdi sistemdekini siler.
    fx.write("data_home/applications/masked.desktop", "[Desktop Entry]\nType=Application\nName=Masked\nHidden=true\n");
    let r = fx.role("pdf");
    let ids: Vec<(&str, bool)> = r.candidates.iter().map(|c| (c.id.as_str(), c.hidden)).collect();
    assert_eq!(
        ids,
        [("alpha.desktop", false), ("tryok.desktop", false), ("zeta.desktop", false), ("handler.desktop", true)],
        "ad sırası (küçük harf), NoDisplay sonda"
    );
    assert_eq!(fx.set("pdf", "hidden.desktop"), Err(SetError::UnknownApp));
    assert_eq!(fx.set("pdf", "masked.desktop"), Err(SetError::UnknownApp));
    assert!(fx.set("pdf", "handler.desktop").is_ok(), "NoDisplay aday olabilir");
}

#[test]
fn write_preserves_everything_else_and_noop_takes_no_snapshot() {
    let fx = Fx::new();
    fx.app("firefox.desktop", "Firefox", "Icon=firefox\nMimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n");
    let original = "\
# kullanıcı yorumu
[Added Associations]
text/html=other.desktop;

; noktalı virgül yorumu
[Default Applications]
image/png=viewer.desktop;
x-scheme-handler/http=old.desktop;
# arada yorum
text/html=old.desktop;

[Özel]
anahtar=değer
";
    fx.write("config/mimeapps.list", original);
    let ok = fx.set("browser", "firefox.desktop").unwrap();
    assert_eq!(ok.written, ["x-scheme-handler/http", "x-scheme-handler/https", "text/html"]);
    let text = std::fs::read_to_string(fx.user_list()).unwrap();
    let expected = original
        .replace("x-scheme-handler/http=old.desktop;", "x-scheme-handler/http=firefox.desktop;")
        .replace("text/html=old.desktop;\n", "text/html=firefox.desktop;\nx-scheme-handler/https=firefox.desktop;\n");
    assert_eq!(text, expected);
    let r = fx.role("browser");
    assert_eq!(r.current, Some(AppRef { id: "firefox.desktop".into(), name: "Firefox".into(), icon: Some("firefox".into()) }));
    assert_eq!(r.source, "user");
    assert!(r.shadowed_by.is_none() && !r.mixed && r.available);

    // Değişiklik yok: ok, boş written, snapshot yok.
    let before = fx.snapshots();
    assert_eq!(fx.set("browser", "firefox.desktop"), Ok(SetOk::default()));
    assert_eq!(fx.snapshots(), before);
    assert_eq!(set_json(&Ok(SetOk::default())), r#"{"ok":true,"snapshot":null,"written":[]}"#);

    // Geri yükleme: eski hâl bayt bayt.
    let id = SnapshotId::parse(ok.snapshot.as_deref().unwrap()).unwrap();
    let r = apply::snapshot_restore(&fx.env, &id, RestoreMode::Safe, apply::known_setting("defaults.browser").as_ref());
    assert!(r.json.starts_with("{\"result\":\"restored\""), "{}", r.json);
    assert_eq!(std::fs::read_to_string(fx.user_list()).unwrap(), original);
}

#[test]
fn missing_file_is_created_and_restore_removes_it() {
    let fx = Fx::new();
    fx.app("ev.desktop", "Evince", "MimeType=application/pdf;\n");
    assert!(!fx.user_list().exists());
    let ok = fx.set("pdf", "ev.desktop").unwrap();
    assert_eq!(std::fs::read_to_string(fx.user_list()).unwrap(), "[Default Applications]\napplication/pdf=ev.desktop;\n");
    let json: serde_json::Value = serde_json::from_str(&set_json(&Ok(ok.clone()))).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["written"][0], "application/pdf");
    let id = SnapshotId::parse(ok.snapshot.as_deref().unwrap()).unwrap();
    let r = apply::snapshot_restore(&fx.env, &id, RestoreMode::Safe, None);
    assert!(r.json.starts_with("{\"result\":\"restored\""), "{}", r.json);
    assert!(!fx.user_list().exists(), "Nuxut'un yarattığı ve boş kalan dosya silinir");
}

#[test]
fn shadowed_write_still_happens() {
    let fx = Fx::new();
    fx.app("a.desktop", "A", "MimeType=application/pdf;\n");
    fx.app("b.desktop", "B", "MimeType=application/pdf;\n");
    let shadow = "[Default Applications]\napplication/pdf=a.desktop;\n";
    fx.write("config/hyprland-mimeapps.list", shadow);
    assert!(fx.set("pdf", "b.desktop").unwrap().snapshot.is_some());
    let r = fx.role("pdf");
    assert_eq!(cur(&r), Some("a.desktop"));
    assert!(r.shadowed_by.is_some());
    assert_eq!(std::fs::read_to_string(fx.root.join("config/hyprland-mimeapps.list")).unwrap(), shadow, "dokunulmaz");
    assert!(std::fs::read_to_string(fx.user_list()).unwrap().contains("application/pdf=b.desktop;"));
}

#[test]
fn error_codes() {
    let fx = Fx::new();
    fx.app("a.desktop", "A", "MimeType=application/pdf;\nCategories=System;TerminalEmulator;\n");
    assert_eq!(fx.set("nope", "a.desktop"), Err(SetError::UnknownRole));
    assert_eq!(fx.set("pdf", "b.desktop"), Err(SetError::UnknownApp));
    assert_eq!(fx.set("pdf", "../a.desktop"), Err(SetError::UnknownApp));
    assert_eq!(fx.set("pdf", "a"), Err(SetError::UnknownApp));
    assert_eq!(fx.set("image", "a.desktop"), Err(SetError::UnknownApp), "birincil MIME'ı bildirmiyor");
    assert_eq!(fx.set(TERMINAL, "a.desktop"), Err(SetError::Unavailable));
    let off = Env { disabled: true, ..fx.env.clone() };
    assert_eq!(set(&off, &fx.dirs, &fx.index(), "pdf", "a.desktop"), Err(SetError::WriterDisabled));
    // Test örneği kapısı: kök dışındaki yapılandırma dizinine yazılmaz.
    let mut outside = fx.dirs.clone();
    outside.config_home = Some(fx.root.join("elsewhere"));
    assert_eq!(set(&fx.env, &outside, &fx.index(), "pdf", "a.desktop"), Err(SetError::WriterDisabled));
    assert!(!fx.root.join("elsewhere").exists());
    assert_eq!(set_json(&Err(SetError::UnknownRole)), r#"{"error":"unknown_role","ok":false}"#);
    let wf: serde_json::Value = serde_json::from_str(&set_json(&Err(SetError::WriteFailed("busy".into())))).unwrap();
    assert_eq!((wf["error"].as_str(), wf["detail"].as_str()), (Some("write_failed"), Some("busy")));
}

#[test]
fn status_json_shape() {
    let fx = Fx::new();
    let v: serde_json::Value = serde_json::from_str(&fx.status().to_json()).unwrap();
    assert_eq!(v["v"], 1);
    let roles: Vec<&str> = v["roles"].as_array().unwrap().iter().map(|r| r["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ROLES.map(|r| r.name));
    let t = &v["roles"][3];
    assert_eq!(t["role"], "terminal");
    assert_eq!(t["available"], false, "xdg-terminal-exec yok");
    for key in ["current", "source", "mixed", "shadowed_by", "available", "candidates"] {
        assert!(v["roles"][0].get(key).is_some(), "{key}");
    }
    assert!(v["roles"][0]["current"].is_null() && v["roles"][0]["shadowed_by"].is_null());
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

#[test]
fn terminal_list_write_and_restore() {
    let fx = Fx::new();
    let term = |id: &str| fx.app(id, id, "Categories=System;TerminalEmulator;\n");
    term("foot.desktop");
    term("kitty.desktop");
    term("alacritty.desktop");
    fx.app("notterm.desktop", "x", "Categories=Utility;\n");
    fx.exe(terminal::TOOL);
    let r = fx.role(TERMINAL);
    assert!(r.available);
    assert_eq!(r.candidates.len(), 3);
    assert_eq!((r.current.clone(), r.source), (None, "none"));

    // Sistem listesi okunur.
    fx.write("sys/xdg-terminal-exec/xdg-terminals.list", "# sistem\nalacritty.desktop\n");
    assert_eq!((cur(&fx.role(TERMINAL)), fx.role(TERMINAL).source), (Some("alacritty.desktop"), "system"));

    // Dosya yok → yaratılır; diğer satırlar korunur.
    let list = fx.root.join("config/xdg-terminals.list");
    let ok = fx.set(TERMINAL, "foot.desktop").unwrap();
    assert_eq!(ok.written, [terminal::LIST]);
    assert_eq!(read(&list), "foot.desktop\n");
    std::fs::write(&list, "foot.desktop\n# yorum\nmissing.desktop\n").unwrap();
    fx.set(TERMINAL, "kitty.desktop").unwrap();
    assert_eq!(read(&list), "kitty.desktop\nfoot.desktop\n# yorum\nmissing.desktop\n");
    assert_eq!((cur(&fx.role(TERMINAL)), fx.role(TERMINAL).source), (Some("kitty.desktop"), "user"));
    // Değişiklik yok.
    assert_eq!(fx.set(TERMINAL, "kitty.desktop"), Ok(SetOk::default()));

    // Aşağıdaki satır başa taşınır; son snapshot geri yüklenince önceki ilk girdi geçerli olur.
    let ok = fx.set(TERMINAL, "foot.desktop").unwrap();
    assert_eq!(read(&list), "foot.desktop\nkitty.desktop\n# yorum\nmissing.desktop\n");
    let id = SnapshotId::parse(ok.snapshot.as_deref().unwrap()).unwrap();
    let r = apply::snapshot_restore(&fx.env, &id, RestoreMode::Safe, apply::known_setting("defaults.terminal").as_ref());
    assert!(r.json.starts_with("{\"result\":\"restored\""), "{}", r.json);
    assert_eq!(cur(&fx.role(TERMINAL)), Some("kitty.desktop"));
    assert_eq!(read(&list), "kitty.desktop\n# yorum\nmissing.desktop\n");

    // Masaüstüne özgü kullanıcı listesi gölgeler.
    fx.write("config/hyprland-xdg-terminals.list", "alacritty.desktop\n");
    let r = fx.role(TERMINAL);
    assert_eq!((cur(&r), r.source), (Some("alacritty.desktop"), "desktop_user"));
    assert!(r.shadowed_by.as_deref().is_some_and(|p| p.ends_with("hyprland-xdg-terminals.list")));
}

#[test]
fn terminal_plan_and_entry_ids() {
    assert_eq!(terminal::entry_id("  foot.desktop:new-window "), Some("foot.desktop"));
    assert_eq!(terminal::entry_id("# x"), None);
    assert_eq!(terminal::entry_id(""), None);
    assert_eq!(terminal::plan("", "a.desktop"), terminal::Plan::Prepend);
    assert_eq!(terminal::plan("# y\na.desktop:x\n", "a.desktop"), terminal::Plan::Unchanged);
    assert_eq!(terminal::plan("b.desktop\na.desktop\n", "a.desktop"), terminal::Plan::MoveToTop);
    assert_eq!(terminal::plan("b.desktop\na.desktop:x\n", "a.desktop"), terminal::Plan::Prepend);
}

#[test]
fn mimeapps_parse_and_chain() {
    let f = mimeapps::parse(
        Path::new("/x"),
        mimeapps::Source::User,
        "[Default Applications]\na/b = x.desktop; y.desktop ;;\n[Added Associations]\na/b=z.desktop;\n[Default Applications]\na/b=w.desktop\n",
    );
    assert_eq!(f.default_for("a/b"), Some(&["w.desktop".to_owned()][..]), "yinelenen anahtarın sonuncusu");
    let fx = Fx::new();
    let names: Vec<String> = mimeapps::chain_paths(&fx.dirs)
        .into_iter()
        .map(|(p, s)| format!("{}:{}", p.strip_prefix(&fx.root).unwrap().display(), s.name()))
        .collect();
    assert_eq!(
        names,
        [
            "config/hyprland-mimeapps.list:desktop_user",
            "config/mimeapps.list:user",
            "etc/hyprland-mimeapps.list:system",
            "etc/mimeapps.list:system",
            "data_home/applications/hyprland-mimeapps.list:user",
            "data_home/applications/mimeapps.list:user",
            "sys/applications/hyprland-mimeapps.list:system",
            "sys/applications/mimeapps.list:system",
        ]
    );
}

#[test]
fn every_role_has_a_snapshot_setting() {
    let names: Vec<&str> = ROLES.iter().map(|r| r.name).collect();
    let ids: Vec<String> = names.iter().map(|n| format!("defaults.{n}")).collect();
    assert_eq!(ids, apply::DEFAULTS_SETTINGS, "rol tablosu ve kapalı liste aynı sırada");
    for r in &ROLES {
        assert!(setting_for(r).is_ok(), "{}", r.name);
    }
}

#[test]
fn added_associations_after_defaults_before_cache() {
    let fx = Fx::new();
    for id in ["a", "b", "c", "d"] {
        fx.app(&format!("{id}.desktop"), id, "MimeType=application/pdf;\n");
    }
    fx.write("sys/applications/mimeinfo.cache", "[MIME Cache]\napplication/pdf=d.desktop;\n");
    assert_eq!(cur(&fx.role("pdf")), Some("d.desktop"));
    // Added (sistem) önbellekten önce; kurulu olmayan atlanır.
    fx.write("sys/applications/mimeapps.list", "[Added Associations]\napplication/pdf=missing.desktop;c.desktop;\n");
    let r = fx.role("pdf");
    assert_eq!((cur(&r), r.source), (Some("c.desktop"), "system"));
    // Kullanıcı dosyasının Added'ı önce; aynı dosyadaki kaldırma sonraki dosyaları etkiler.
    fx.write("config/mimeapps.list", "[Added Associations]\napplication/pdf=b.desktop;\n[Removed Associations]\napplication/pdf=c.desktop;\n");
    assert_eq!((cur(&fx.role("pdf")), fx.role("pdf").source), (Some("b.desktop"), "user"));
    fx.write("config/mimeapps.list", "[Removed Associations]\napplication/pdf=c.desktop;d.desktop;\n");
    assert_eq!(cur(&fx.role("pdf")), None, "kaldırılan Added ve önbellek adayı yok");
    // Herhangi bir dosyadaki varsayılan Added'dan güçlü (sistem varsayılanı > kullanıcı Added).
    fx.write("config/mimeapps.list", "[Added Associations]\napplication/pdf=b.desktop;\n");
    fx.write("etc/mimeapps.list", "[Default Applications]\napplication/pdf=a.desktop;\n");
    assert_eq!((cur(&fx.role("pdf")), fx.role("pdf").source), (Some("a.desktop"), "system"));
}

#[test]
fn duplicate_defaults_section_is_not_written() {
    let fx = Fx::new();
    fx.app("a.desktop", "a", "MimeType=application/pdf;\n");
    fx.app("b.desktop", "b", "MimeType=application/pdf;\n");
    let text = "[Default Applications]\napplication/pdf=a.desktop;\n\n[Other]\nx=y\n\n[Default Applications]\napplication/pdf=b.desktop;\n";
    let p = fx.write("config/mimeapps.list", text);
    // Okuma GLib gibi sonraki değeri görür.
    assert_eq!(cur(&fx.role("pdf")), Some("b.desktop"));
    assert_eq!(fx.set("pdf", "a.desktop"), Err(SetError::WriteFailed("duplicate_section".into())));
    assert_eq!(read(&p), text, "dosyaya dokunulmadı");
    assert_eq!(fx.snapshots(), 0);
    assert!(set_json(&fx.set("pdf", "a.desktop")).contains("\"detail\":\"duplicate_section\""));
}

#[test]
fn candidates_have_settable_ids() {
    let fx = Fx::new();
    fx.app("good.desktop", "g", "MimeType=application/pdf;\n");
    fx.app("bad id.desktop", "b", "MimeType=application/pdf;\n");
    fx.app("çok.desktop", "c", "MimeType=application/pdf;\n");
    let r = fx.role("pdf");
    let ids: Vec<&str> = r.candidates.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["good.desktop"]);
    for c in &r.candidates {
        assert!(fx.set("pdf", &c.id).is_ok(), "listedeki aday yazılabilir: {}", c.id);
    }
}

#[test]
fn terminal_move_failure_puts_line_back() {
    let fx = Fx::new();
    for id in ["foot.desktop", "kitty.desktop"] {
        fx.app(id, id, "Categories=TerminalEmulator;\n");
    }
    fx.exe(terminal::TOOL);
    let list = fx.root.join("config/xdg-terminals.list");
    fx.write("config/xdg-terminals.list", "kitty.desktop\n# yorum\nfoot.desktop\n");
    let role = role(TERMINAL).unwrap();
    for (failure, name) in [(Outcome::Busy, "busy"), (Outcome::Changed, "changed"), (Outcome::Error { kind: "io" }, "io")] {
        let r = set_terminal_with(&fx.env, &fx.dirs, role, "foot.desktop", "t", |_| failure);
        assert!(matches!(&r, Err(SetError::WriteFailed(d)) if *d == name), "{r:?}");
        let text = read(&list);
        assert!(text.lines().any(|l| l == "foot.desktop"), "satır kaybolmadı: {text:?}");
        assert!(text.starts_with("kitty.desktop\n"), "etkin terminal aynı: {text:?}");
        assert_eq!(cur(&fx.role(TERMINAL)), Some("kitty.desktop"));
    }
    // Başarılı taşıma yine çalışır.
    fx.set(TERMINAL, "foot.desktop").unwrap();
    assert!(read(&list).starts_with("foot.desktop\nkitty.desktop\n"), "{}", read(&list));
    assert_eq!(read(&list).lines().filter(|l| *l == "foot.desktop").count(), 1);
}
