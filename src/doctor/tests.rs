use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::gather::{self, Env, LiveReply, Probes};
use super::*;

const HOME: &str = "/home/u";

fn healthy() -> Facts {
    Facts {
        home: Some(PathBuf::from(HOME)),
        install: Install {
            bin_dir: Some(PathBuf::from("/home/u/.local/bin")),
            total_bins: 5,
            total_helpers: 4,
            in_path: true,
            version: "0.1.0".into(),
            commit: Some("abc1234".into()),
            install_sh: PathBuf::from("/home/u/.local/src/nuxut/shell/install.sh"),
            ..Default::default()
        },
        live: Live::Running(Box::new(Running {
            pid: 42,
            version: "0.1.0".into(),
            commit: Some("abc1234".into()),
            instance: "nuxut-shell".into(),
            service: Service::Unit,
            instances: 1,
            session: SessionRes::ByPid,
            hypr: Some(json!({"source": "present", "config": "/home/u/.config/hypr/hyprland.conf", "conflicts": []})),
            look: Some(json!({"writer": "nuxut-apply", "unified": true, "conflicts": [], "last_apply": {"external": [], "paused": []}})),
            audio: Some(json!({"available": true, "backend": "wpctl", "sink": "route", "source": "props"})),
            lock_listening: Some(true),
            clipboard: Some(json!({"enabled": true, "data_control": true, "watching": true, "bridge": "auto", "bridge_running": true, "bridge_active": false})),
        })),
        packs: Vec::new(),
        hypr_config: Some(PathBuf::from("/home/u/.config/hypr/hyprland.conf")),
        portal: Portal {
            total_files: 4,
            desktop_has_nuxut: Some(true),
            portals_conf: ConfState::Managed { file_chooser_nuxut: true },
            portals_conf_path: PathBuf::from("/home/u/.config/xdg-desktop-portal/nuxut-portals.conf"),
            sharepicker_block: Some(true),
            ..Default::default()
        },
        lock: Lock { binary: PathBuf::from("/home/u/.local/bin/nuxut-lock"), binary_ok: true, listen_logind: true },
        disk_free: Some(50 << 30),
        fonts: vec![Font { role: "ui", family: "Inter".into(), installed: true }],
        packages: Some(Packages { total: 5, ..Default::default() }),
        competitors: Vec::new(),
        clip_tools: Vec::new(),
    }
}

fn running(f: &mut Facts) -> &mut Running {
    match &mut f.live {
        Live::Running(r) => r,
        _ => unreachable!(),
    }
}

fn find<'a>(r: &'a Report, section: &str, id: &str) -> &'a Check {
    r.sections.iter().find(|s| s.id == section).and_then(|s| s.checks.iter().find(|c| c.id == id)).expect("denetim")
}

#[test]
fn healthy_report_is_clean_and_exits_zero() {
    crate::i18n::use_turkish_for_tests();
    let r = evaluate(&healthy());
    let bad: Vec<_> = r.sections.iter().flat_map(|s| &s.checks).filter(|c| c.level >= Level::Warn).map(|c| c.id).collect();
    assert!(bad.is_empty(), "{bad:?}");
    assert_eq!(r.exit_code(), 0);
    let ids: Vec<_> = r.sections.iter().map(|s| s.id).collect();
    assert_eq!(ids, ["install", "shell", "hyprland", "look", "audio", "portal", "lock", "clipboard", "system"]);
    let text = r.render();
    assert!(text.starts_with("Nuxut durum raporu (nuxut-shell 0.1.0)"), "{text}");
    assert!(text.contains("  ✓ İkililer kurulu: ~/.local/bin (5)"), "{text}");
    assert!(text.contains("Ayar dosyası ~/.config/hypr/hyprland.conf: Nuxut ayarları dahil"), "{text}");
    assert!(text.contains("Özet:"), "{text}");
    let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
    assert_eq!(v["v"], 1);
    assert_eq!(v["sections"][0]["checks"][0]["id"], "path");
    assert_eq!(v["sections"][0]["checks"][0]["level"], "ok");
}

#[test]
fn shell_not_running_fails_and_live_checks_wait() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    f.live = Live::NotRunning;
    let r = evaluate(&f);
    assert_eq!(find(&r, "shell", "running").level, Level::Fail);
    assert!(find(&r, "shell", "running").hint.as_deref().unwrap().contains("systemctl --user start nuxut-shell"));
    for (s, id) in [("hyprland", "source"), ("look", "writer"), ("audio", "pipewire")] {
        assert_eq!(find(&r, s, id).level, Level::Info, "{s}.{id}");
    }
    assert_eq!(r.exit_code(), 1);
    // Kabuk yokken de dosya taraması sonuç verir.
    assert_eq!(find(&r, "hyprland", "theme_pack").level, Level::Ok);
    f.live = Live::Unreadable;
    assert_eq!(find(&evaluate(&f), "shell", "running").level, Level::Warn);
}

#[test]
fn install_path_and_binaries() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    f.install.in_path = false;
    f.install.missing_bins = vec!["nuxut-settings"];
    f.install.missing_helpers = vec!["nuxut-sharepicker"];
    f.install.commit = None;
    let r = evaluate(&f);
    let path = find(&r, "install", "path");
    assert_eq!(path.level, Level::Warn);
    assert_eq!(path.hint.as_deref(), Some("sh ~/.local/src/nuxut/shell/install.sh --fix-path çalıştırın, sonra oturumu kapatıp açın"));
    assert_eq!(find(&r, "install", "binaries").level, Level::Fail);
    assert!(find(&r, "install", "binaries").text.contains("nuxut-settings"));
    assert_eq!(find(&r, "install", "helpers").level, Level::Fail, "seçici kurulumun zorunlu saydığı ikili");
    assert!(find(&r, "install", "version").text.contains("bilinmiyor"));
    f.install.missing_helpers = vec!["nuxut-image-worker"];
    assert_eq!(find(&evaluate(&f), "install", "helpers").level, Level::Warn, "görsel işçisi uyarı");
}

#[test]
fn path_is_first_line_and_hint_follows_login_shell() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    f.install.in_path = false;
    f.install.login_shell = LoginShell::Fish;
    let r = evaluate(&f);
    // Başlıktan sonraki ilk denetim satırı PATH.
    assert_eq!(r.sections[0].checks[0].id, "path");
    let first = r.render().lines().find(|l| l.starts_with("  ")).unwrap().to_owned();
    assert!(first.contains("⚠") && first.contains("PATH"), "{first}");
    let path = find(&r, "install", "path");
    assert_eq!(path.hint.as_deref(), Some("fish_add_path ~/.local/bin çalıştırın (yeni terminallerde hemen geçerli)"));
    assert_eq!(path.data["shell"], "fish");
    f.install.login_shell = LoginShell::Bash;
    let path = find(&evaluate(&f), "install", "path").clone();
    assert!(path.hint.unwrap().starts_with("sh ~/.local/src/nuxut/shell/install.sh --fix-path"));
}

#[test]
fn login_shell_from_var_uses_name_only() {
    use std::ffi::OsStr;
    assert_eq!(LoginShell::from_shell_var(Some(OsStr::new("/usr/bin/fish"))), LoginShell::Fish);
    assert_eq!(LoginShell::from_shell_var(Some(OsStr::new("/bin/bash"))), LoginShell::Bash);
    assert_eq!(LoginShell::from_shell_var(Some(OsStr::new("zsh"))), LoginShell::Zsh);
    assert_eq!(LoginShell::from_shell_var(Some(OsStr::new("/opt/fishy/bin/nu"))), LoginShell::Other);
    assert_eq!(LoginShell::from_shell_var(None), LoginShell::Other);
}

#[test]
fn service_session_and_instances() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    let live = running(&mut f);
    live.service = Service::AppScope;
    live.session = SessionRes::Auto;
    live.instances = 2;
    let r = evaluate(&f);
    assert_eq!(find(&r, "shell", "service").level, Level::Warn);
    assert_eq!(find(&r, "shell", "service").data["kind"], "app_scope");
    assert_eq!(find(&r, "shell", "session").level, Level::Warn);
    assert_eq!(find(&r, "shell", "instances").level, Level::Warn);
    running(&mut f).session = SessionRes::Missing;
    assert_eq!(find(&evaluate(&f), "shell", "session").level, Level::Fail);
    running(&mut f).service = Service::LoginSession;
    assert_eq!(find(&evaluate(&f), "shell", "service").level, Level::Ok);
}

#[test]
fn hyprland_source_conflicts_and_packs() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    running(&mut f).hypr = Some(json!({
        "source": "removed_by_user",
        "config": "/home/u/.config/hypr/hyprland.conf",
        "conflicts": [
            {"action": "panel_right", "combo": "SUPER+A", "file": "/home/u/.config/hypr/binds.conf", "line": 12},
            {"action": "launcher", "combo": "SUPER+Space"}
        ]
    }));
    f.packs = vec![
        PackHit { pack: "end4", file: PathBuf::from("/home/u/.config/hypr/hyprland.conf"), line: 3 },
        PackHit { pack: "end4", file: PathBuf::from("/home/u/.config/hypr/custom/x.conf"), line: 7 },
    ];
    let r = evaluate(&f);
    let src = find(&r, "hyprland", "source");
    assert_eq!(src.level, Level::Warn);
    assert!(src.text.contains("kullanıcı tarafından kaldırılmış"), "{}", src.text);
    assert!(src.hint.as_deref().unwrap().contains("hypr add-source"));
    let c = find(&r, "hyprland", "conflicts");
    assert_eq!(c.level, Level::Warn);
    assert!(c.text.contains("SUPER+A panel_right (~/.config/hypr/binds.conf:12)"), "{}", c.text);
    assert!(c.text.contains("SUPER+Space launcher"), "{}", c.text);
    let p = find(&r, "hyprland", "theme_pack");
    assert_eq!(p.level, Level::Warn);
    assert!(p.text.contains("end4") && p.text.contains("~/.config/hypr/custom/x.conf:7"), "{}", p.text);
    assert_eq!(p.data["packs"], json!(["end4"]), "aynı paket tek kez");
    running(&mut f).hypr = Some(json!({"source": "error", "conflicts": []}));
    assert_eq!(find(&evaluate(&f), "hyprland", "source").level, Level::Fail);
}

#[test]
fn look_reports_names_and_paths_but_no_values() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    running(&mut f).look = Some(json!({
        "writer": "nuxut-apply",
        "unified": true,
        "conflicts": [{"kind": "env", "name": "QT_STYLE_OVERRIDE", "value": "gizli-deger", "source": "session_env"}],
        "last_apply": {
            "external": [{"setting": "look.mode", "path": "/home/u/.config/gtk-3.0/gtk.css", "reason": "read_only"}],
            "paused": ["look.icons"]
        }
    }));
    let r = evaluate(&f);
    let c = find(&r, "look", "conflicts");
    assert_eq!(c.level, Level::Warn);
    assert!(c.text.contains("QT_STYLE_OVERRIDE (session_env)"), "{}", c.text);
    let e = find(&r, "look", "external");
    assert_eq!(e.level, Level::Warn);
    assert!(e.text.contains("look.mode → ~/.config/gtk-3.0/gtk.css (read_only)"), "{}", e.text);
    assert_eq!(find(&r, "look", "paused").level, Level::Warn);
    let all = format!("{}{}", r.render(), r.to_json());
    assert!(!all.contains("gizli-deger"), "ortam değeri rapora girmez");
}

#[test]
fn audio_paths() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    running(&mut f).audio = Some(json!({"available": true, "backend": "native", "sink": "card_props", "source": "none"}));
    let r = evaluate(&f);
    assert_eq!(find(&r, "audio", "pipewire").level, Level::Ok);
    assert_eq!(find(&r, "audio", "sink").level, Level::Warn);
    assert!(find(&r, "audio", "sink").text.starts_with("Varsayılan çıkış: yalnızca yazılım"), "{}", find(&r, "audio", "sink").text);
    assert_eq!(find(&r, "audio", "source").level, Level::Fail);
    assert!(find(&r, "audio", "source").hint.as_deref().unwrap().contains("audio-diagnose"));
    running(&mut f).audio = Some(json!({"available": false, "backend": "wpctl"}));
    let r = evaluate(&f);
    assert_eq!(find(&r, "audio", "pipewire").level, Level::Fail);
    assert!(r.sections.iter().find(|s| s.id == "audio").unwrap().checks.len() == 1, "bağlı değilken yol denetimi yok");
    running(&mut f).audio = None;
    assert_eq!(find(&evaluate(&f), "audio", "pipewire").level, Level::Fail);
}

#[test]
fn portal_lock_disk_fonts() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    f.portal.missing_files = vec![PathBuf::from("/home/u/.local/share/xdg-desktop-portal/portals/nuxut.portal")];
    f.portal.desktop_has_nuxut = None;
    f.portal.portals_conf = ConfState::Foreign;
    f.portal.sharepicker_block = None;
    f.lock.binary_ok = false;
    f.lock.listen_logind = false;
    f.disk_free = Some(5 << 30);
    f.fonts = vec![
        Font { role: "ui", family: "Inter".into(), installed: false },
        Font { role: "display", family: "Inter".into(), installed: false },
        Font { role: "document", family: "Source Serif 4".into(), installed: false },
        Font { role: "mono", family: "Özel Font".into(), installed: false },
    ];
    let r = evaluate(&f);
    assert_eq!(find(&r, "portal", "files").level, Level::Warn);
    assert!(find(&r, "portal", "files").text.contains("~/.local/share/xdg-desktop-portal/portals/nuxut.portal"));
    assert_eq!(find(&r, "portal", "desktop").level, Level::Warn);
    assert_eq!(find(&r, "portal", "portals_conf").level, Level::Info);
    assert_eq!(find(&r, "portal", "sharepicker").level, Level::Warn);
    assert_eq!(find(&r, "lock", "binary").level, Level::Fail);
    assert_eq!(find(&r, "lock", "listener").level, Level::Warn);
    assert_eq!(find(&r, "system", "disk").level, Level::Warn);
    let fonts = find(&r, "system", "fonts");
    assert_eq!(fonts.text, "Kurulu olmayan font: Inter, Source Serif 4, Özel Font");
    assert_eq!(fonts.hint.as_deref(), Some("Kurun: inter-font adobe-source-serif-fonts"));
    f.disk_free = Some(1 << 30);
    assert_eq!(find(&evaluate(&f), "system", "disk").level, Level::Fail);
    // Dinleme açık ama canlı dinleyici yok.
    f.lock.listen_logind = true;
    running(&mut f).lock_listening = Some(false);
    assert_eq!(find(&evaluate(&f), "lock", "listener").level, Level::Fail);
    running(&mut f).lock_listening = None;
    assert_eq!(find(&evaluate(&f), "lock", "listener").level, Level::Info);
}

#[test]
fn home_never_appears_absolute() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    f.install.in_path = false;
    f.packs = vec![PackHit { pack: "caelestia", file: PathBuf::from("/home/u/.config/hypr/hyprland.conf"), line: 1 }];
    let r = evaluate(&f);
    assert!(!r.render().contains("/home/u"), "{}", r.render());
    assert!(!r.to_json().contains("/home/u"), "{}", r.to_json());
}

// ---------------------------------------------------------------------------------------
// Toplama (sahte ortam)

fn exe(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

struct Fake {
    _dir: tempfile::TempDir,
    env: Env,
}

fn fake() -> Fake {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let bin = home.join(".local/bin");
    let env = Env {
        config_home: home.join(".config"),
        data_home: home.join(".local/share"),
        exe: Some(bin.join("nuxut-shell")),
        path: Some(std::env::join_paths([Path::new("/usr/bin"), &bin]).unwrap()),
        shell: Some("/usr/bin/fish".into()),
        current_desktop: Some("Nuxut:Hyprland".into()),
        proc_root: dir.path().join("proc"),
        pacman_db: dir.path().join("pacman"),
        src: home.join(".local/src/nuxut"),
        version: "0.1.0".into(),
        commit: None,
        home: Some(home),
    };
    Fake { _dir: dir, env }
}

fn run_gather(env: &Env, live: LiveReply, session: SessionRes, installed: &[&str]) -> Facts {
    run_gather_with(env, live, session, installed, &|| None)
}

fn run_gather_with(
    env: &Env,
    live: LiveReply,
    session: SessionRes,
    installed: &[&str],
    systemd_env: &dyn Fn() -> Option<Vec<String>>,
) -> Facts {
    let session = move |_| session;
    let mut font = |f: &str| installed.contains(&f);
    let families = vec![("ui", "Inter".into()), ("document", "Source Serif 4".into())];
    gather::gather(env, live, Probes { session: &session, systemd_env, font: &mut font, families })
}

#[test]
fn gather_install_portal_lock_and_fonts() {
    let f = fake();
    let home = f.env.home.clone().unwrap();
    let bin = home.join(".local/bin");
    for b in gather::BINARIES.iter().filter(|b| **b != "nuxut-calendard") {
        exe(&bin.join(b));
    }
    std::fs::write(bin.join("nuxut-calendard"), "").unwrap(); // çalıştırılamaz: eksik sayılır
    let libexec = home.join(".local/lib/nuxut");
    for h in ["nuxut-image-worker", "xdg-desktop-portal-nuxut", "nuxut-sharepicker"] {
        exe(&libexec.join(h));
    }
    write(&home.join(".local/share/xdg-desktop-portal/portals/nuxut.portal"), "[portal]\n");
    write(
        &home.join(".config/xdg-desktop-portal/nuxut-portals.conf"),
        "# nuxut-managed: portals — x\n[preferred]\ndefault=hyprland;gtk\norg.freedesktop.impl.portal.FileChooser = nuxut;gtk\n",
    );
    write(&home.join(".config/hypr/xdph.conf"), "screencopy {\n}\n# >>> nuxut-sharepicker (yönetilen)\n");
    write(&home.join(".config/nuxut/shell.toml"), "[lock]\nlisten_logind = false\n");

    let facts = run_gather(&f.env, LiveReply::NotRunning, SessionRes::ByPid, &["Inter"]);
    assert_eq!(facts.install.missing_bins, ["nuxut-calendard"]);
    assert_eq!(facts.install.missing_helpers, ["nuxut-filechooser"]);
    assert!(facts.install.in_path);
    assert_eq!(facts.install.login_shell, LoginShell::Fish);
    assert_eq!(facts.install.install_sh, home.join(".local/src/nuxut/shell/install.sh"));
    assert!(matches!(facts.live, Live::NotRunning));
    // nuxut.portal ve arka uç ikilisi var; D-Bus servisi ve systemd birimi eksik.
    assert_eq!(facts.portal.missing_files.len(), 2, "{:?}", facts.portal.missing_files);
    assert_eq!(facts.portal.desktop_has_nuxut, Some(true));
    assert_eq!(facts.portal.portals_conf, ConfState::Managed { file_chooser_nuxut: true });
    assert_eq!(facts.portal.sharepicker_block, Some(true));
    assert!(facts.lock.binary_ok);
    assert!(!facts.lock.listen_logind);
    assert_eq!(facts.fonts, [
        Font { role: "ui", family: "Inter".into(), installed: true },
        Font { role: "document", family: "Source Serif 4".into(), installed: false },
    ]);
    assert!(facts.disk_free.is_some());
}

#[test]
fn gather_portal_conf_states_and_defaults() {
    let f = fake();
    let home = f.env.home.clone().unwrap();
    let facts = run_gather(&f.env, LiveReply::NotRunning, SessionRes::ByPid, &[]);
    assert_eq!(facts.portal.portals_conf, ConfState::Missing);
    assert_eq!(facts.portal.sharepicker_block, None);
    assert!(facts.lock.listen_logind, "ayar yoksa varsayılan açık");
    assert!(!facts.lock.binary_ok);
    write(&home.join(".config/xdg-desktop-portal/nuxut-portals.conf"), "[preferred]\ndefault=gtk\n");
    write(&home.join(".config/hypr/xdph.conf"), "screencopy {}\n");
    write(&home.join(".config/nuxut/shell.toml"), "bozuk = [\n");
    let facts = run_gather(&f.env, LiveReply::NotRunning, SessionRes::ByPid, &[]);
    assert_eq!(facts.portal.portals_conf, ConfState::Foreign);
    assert_eq!(facts.portal.sharepicker_block, Some(false));
    assert!(facts.lock.listen_logind, "bozuk ayar: varsayılan");
    let mut env = f.env.clone();
    env.current_desktop = Some("Hyprland".into());
    env.path = Some("/usr/bin".into());
    let facts = run_gather(&env, LiveReply::NotRunning, SessionRes::ByPid, &[]);
    assert_eq!(facts.portal.desktop_has_nuxut, Some(false));
    assert!(!facts.install.in_path);
}

#[test]
fn gather_theme_packs_follow_one_level_of_sources() {
    let f = fake();
    let home = f.env.home.clone().unwrap();
    let hypr = home.join(".config/hypr");
    write(
        &hypr.join("hyprland.conf"),
        "# ayarlar\nsource = ~/.config/hypr/custom/rules.conf\nSOURCE=./local.conf # yorum\nsource = ~/.config/hypr/*.conf\n$x = 1 # Caelestia teması\n",
    );
    write(&hypr.join("custom/rules.conf"), "a\nb\nexec-once = ~/.config/quickshell/illogical-impulse/start\nsource = ~/.config/hypr/deeper.conf\n");
    write(&hypr.join("local.conf"), "ımlı satır İ\nomarchy\n");
    write(&hypr.join("deeper.conf"), "ml4w\n");
    let facts = run_gather(&f.env, LiveReply::NotRunning, SessionRes::ByPid, &[]);
    assert_eq!(facts.hypr_config.as_deref(), Some(hypr.join("hyprland.conf").as_path()));
    let hits: Vec<(&str, PathBuf, usize)> = facts.packs.iter().map(|p| (p.pack, p.file.clone(), p.line)).collect();
    assert_eq!(hits, [
        ("caelestia", hypr.join("hyprland.conf"), 5),
        ("end4", hypr.join("custom/rules.conf"), 3),
        ("omarchy", hypr.join("local.conf"), 2),
    ], "yalnız bir düzey source; glob atlanır");
}

#[test]
fn gather_live_reply_proc_and_session() {
    let f = fake();
    let proc = &f.env.proc_root;
    write(&proc.join("77/cgroup"), "0::/user.slice/user-1000.slice/user@1000.service/app.slice/nuxut-shell.service\n");
    std::fs::create_dir_all(proc.join("78")).unwrap();
    std::fs::create_dir_all(proc.join("self")).unwrap();
    std::os::unix::fs::symlink("/opt/nuxut-shell", proc.join("77/exe")).unwrap();
    std::os::unix::fs::symlink("/opt/nuxut-shell", proc.join("78/exe")).unwrap();
    let text = json!({
        "v": 1, "pid": 77, "version": "0.1.0", "commit": null, "instance": "nuxut-shell",
        "hypr": {"source": "present"}, "look": {"writer": "disabled"},
        "audio": {"available": true, "backend": "wpctl", "sink": "route", "source": "route"},
        "lock": {"listening": true}
    })
    .to_string();
    let facts = run_gather(&f.env, LiveReply::Text(text), SessionRes::Auto, &[]);
    let Live::Running(r) = &facts.live else { panic!("{:?}", facts.live) };
    assert_eq!(r.pid, 77);
    assert_eq!(r.service, Service::Unit);
    assert_eq!(r.instances, 2);
    assert_eq!(r.session, SessionRes::Auto);
    assert_eq!(r.lock_listening, Some(true));
    assert_eq!(r.hypr.as_ref().unwrap()["source"], "present");
    for (reply, name) in [
        (LiveReply::Text("error: bilinmeyen komut".into()), "eski kabuk"),
        (LiveReply::Text("{\"pid\":0}".into()), "pid yok"),
        (LiveReply::Failed, "zaman aşımı"),
    ] {
        let facts = run_gather(&f.env, reply, SessionRes::ByPid, &[]);
        assert!(matches!(facts.live, Live::Unreadable), "{name}");
    }
}

#[test]
fn service_kinds_from_cgroup() {
    let dir = tempfile::tempdir().unwrap();
    for (pid, line, want) in [
        (1, "0::/user.slice/user-1000.slice/session-2.scope", Service::LoginSession),
        (2, "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-kitty-123.scope", Service::AppScope),
        (3, "0::/user.slice/user-1000.slice/user@1000.service/app.slice/nuxut-shell.service", Service::Unit),
        (4, "1:name=x:/\n", Service::Unknown),
    ] {
        write(&dir.path().join(pid.to_string()).join("cgroup"), line);
        assert_eq!(gather::service(dir.path(), pid), want, "{line}");
    }
    assert_eq!(gather::service(dir.path(), 99), Service::Unknown);
}

#[test]
fn packages_and_competitors() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    f.packages = None;
    assert_eq!(find(&evaluate(&f), "system", "packages").level, Level::Info, "Arch dışı");
    f.packages = Some(Packages { total: 5, missing: Vec::new(), missing_optional: vec!["mpv"] });
    let c = find(&evaluate(&f), "system", "packages").clone();
    assert_eq!((c.level, c.text.as_str()), (Level::Info, "İsteğe bağlı, kurulu değil: mpv"));
    f.packages = Some(Packages { total: 5, missing: vec!["papirus-icon-theme", "xdg-desktop-portal-gtk"], missing_optional: vec![] });
    let c = find(&evaluate(&f), "system", "packages").clone();
    assert_eq!(c.level, Level::Warn);
    assert_eq!(c.hint.as_deref(), Some("Paket yöneticisiyle kurun: papirus-icon-theme xdg-desktop-portal-gtk"));
    f.competitors = vec!["dunst", "waybar"];
    let r = evaluate(&f);
    let c = find(&r, "shell", "competitors");
    assert_eq!((c.level, c.text.as_str()), (Level::Warn, "Çakışabilecek programlar çalışıyor: dunst, waybar"));
    // Kabuk çalışmasa da denetlenir.
    f.live = Live::NotRunning;
    assert_eq!(find(&evaluate(&f), "shell", "competitors").level, Level::Warn);
}

#[test]
fn gather_packages_from_pacman_db_and_competitors_from_proc() {
    let f = fake();
    assert_eq!(gather::packages(&f.env.pacman_db), None, "veritabanı yok");
    for e in ["papirus-icon-theme-20250501-1", "xdg-desktop-portal-1.20.3-1", "xdg-desktop-portal-gtk-1.15.3-1", "mpv-1:0.40.0-3", "ALPM_DB_VERSION"] {
        std::fs::create_dir_all(f.env.pacman_db.join(e)).unwrap();
    }
    let p = gather::packages(&f.env.pacman_db).unwrap();
    assert_eq!(p.total, gather::PACKAGES.len());
    // "xdg-desktop-portal" kurulu sayılır ama "-hyprland" ile karışmaz.
    assert_eq!(p.missing, ["vimix-cursors", "xdg-desktop-portal-hyprland"]);
    assert_eq!(p.missing_optional, ["ddcutil"]);
    let proc = &f.env.proc_root;
    for (pid, comm) in [(10, "waybar\n"), (11, "dunst\n"), (12, "waybar\n"), (13, "waybar-helper\n"), (14, "kitty\n")] {
        write(&proc.join(format!("{pid}/comm")), comm);
    }
    write(&proc.join("self/comm"), "eww\n");
    assert_eq!(gather::competitors(proc), ["dunst", "waybar"]);
    let facts = run_gather(&f.env, LiveReply::NotRunning, SessionRes::ByPid, &[]);
    assert_eq!(facts.competitors, ["dunst", "waybar"]);
    assert!(facts.packages.is_some());
}

#[test]
fn gather_session_path_reads_effective_environments() {
    let f = fake();
    let home = f.env.home.clone().unwrap();
    let bin = home.join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let proc = &f.env.proc_root;
    // Çalışan kabuk PATH'te görür, Hyprland görmez; systemd de görmez ama dosya var.
    write(&proc.join("77/environ"), &format!("HOME=/x\0PATH=/usr/bin:{}\0LANG=C\0", bin.display()));
    write(&proc.join("90/comm"), "Hyprland\n");
    write(&proc.join("90/environ"), "PATH=/usr/bin\0");
    write(&f.env.config_home.join(gather::ENVD_PATH_FILE), "PATH=x\n");
    let text = json!({"v": 1, "pid": 77, "version": "0.1.0", "instance": "nuxut-shell"}).to_string();
    let systemd = || Some(vec!["LANG=C".to_owned(), "PATH=/usr/bin".to_owned()]);
    let facts = run_gather_with(&f.env, LiveReply::Text(text), SessionRes::ByPid, &[], &systemd);
    assert_eq!(
        facts.install.session_path,
        SessionPath { systemd: Some(false), hyprland: Some(false), shell: Some(true), envd_file: true }
    );
    // Hiçbiri okunamazsa bilinmiyor; dosya yok.
    let g = fake();
    let facts = run_gather(&g.env, LiveReply::NotRunning, SessionRes::ByPid, &[]);
    assert_eq!(facts.install.session_path, SessionPath::default());
}

#[test]
fn session_path_check_and_fix_lines() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    let with = |f: &mut Facts, sp: SessionPath| {
        f.install.session_path = sp;
        find(&evaluate(f), "install", "session_path").clone()
    };
    let unknown = with(&mut f, SessionPath::default());
    assert_eq!(unknown.level, Level::Info);
    let ok = with(&mut f, SessionPath { systemd: Some(true), hyprland: Some(true), shell: None, envd_file: true });
    assert_eq!(ok.level, Level::Ok);
    assert!(ok.text.contains("systemd, Hyprland"), "{}", ok.text);
    // Gerçek makine durumu: dosya var, ortamlar görmüyor → yeniden giriş.
    let relogin = with(&mut f, SessionPath { systemd: Some(false), hyprland: Some(false), shell: Some(false), envd_file: true });
    assert_eq!(relogin.level, Level::Warn);
    assert!(relogin.text.contains("systemd, Hyprland, nuxut-shell"), "{}", relogin.text);
    assert!(relogin.hint.as_deref().unwrap().starts_with("Oturumu tamamen kapatıp açın"));
    let no_file = with(&mut f, SessionPath { systemd: Some(false), hyprland: None, shell: None, envd_file: false });
    assert!(no_file.hint.as_deref().unwrap().contains("install.sh --fix-path"));
    let hypr = with(&mut f, SessionPath { systemd: Some(true), hyprland: Some(false), shell: None, envd_file: true });
    assert!(hypr.hint.as_deref().unwrap().contains("uwsm"));
    // Raporda her öneri "Çözüm:" satırıdır.
    let render = evaluate(&f).render();
    assert!(render.contains("      Çözüm: Hyprland'i uwsm ile"), "{render}");
}

#[test]
fn every_problem_has_a_fix_line() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    f.install.in_path = false;
    f.install.missing_bins = vec!["nuxut-settings"];
    f.lock.binary_ok = false;
    f.portal.portals_conf = ConfState::Managed { file_chooser_nuxut: false };
    f.install.session_path = SessionPath { systemd: Some(false), hyprland: None, shell: None, envd_file: true };
    f.clip_tools = vec![
        ClipTool { tool: "hypr-bridge", kind: ClipKind::Bridge, at: None },
        ClipTool { tool: "copyq", kind: ClipKind::Manager, at: None },
    ];
    set_clipboard(&mut f, json!({"enabled": true, "data_control": true, "watching": false, "bridge": "on", "bridge_running": false, "bridge_active": false}));
    running(&mut f).look = Some(json!({"writer": "nuxut-apply", "unified": true, "conflicts": [], "last_apply": {"external": [
        {"setting": "gtk3", "path": "/home/u/.config/gtk-3.0/settings.ini", "reason": "not_owned"},
    ], "paused": []}}));
    let r = evaluate(&f);
    for s in &r.sections {
        for c in &s.checks {
            if matches!(c.level, Level::Warn | Level::Fail) {
                assert!(c.hint.is_some(), "{}.{} çözümsüz: {}", s.id, c.id, c.text);
            }
        }
    }
    let chooser = find(&r, "portal", "portals_conf");
    assert!(chooser.hint.as_deref().unwrap().contains("install.sh --write-portals-conf"));
}

// ---------------------------------------------------------------------------------------
// Pano ve sahipsiz görünüm dosyaları (2026-10-16 gerçek makine bulguları)

fn set_clipboard(f: &mut Facts, v: serde_json::Value) {
    running(f).clipboard = Some(v);
}

#[test]
fn clipboard_history_and_bridge_states() {
    crate::i18n::use_turkish_for_tests();
    let r = evaluate(&healthy());
    assert_eq!(find(&r, "clipboard", "history").level, Level::Ok);
    assert_eq!(find(&r, "clipboard", "bridge").level, Level::Info, "konuk ajanı yok: beklemede");
    assert_eq!(find(&r, "clipboard", "external").level, Level::Ok);

    let mut f = healthy();
    set_clipboard(&mut f, json!({"enabled": true, "data_control": false, "watching": false, "bridge": "auto", "bridge_running": false, "bridge_active": false}));
    let r = evaluate(&f);
    assert_eq!(find(&r, "clipboard", "history").level, Level::Warn);
    assert!(find(&r, "clipboard", "history").hint.as_deref().unwrap().contains("Hyprland"));
    assert_eq!(find(&r, "clipboard", "bridge").level, Level::Info, "neden geçmiş satırında");

    set_clipboard(&mut f, json!({"enabled": true, "data_control": true, "watching": false, "bridge": "on", "bridge_running": false, "bridge_active": false}));
    let r = evaluate(&f);
    assert_eq!(find(&r, "clipboard", "history").level, Level::Warn);
    assert_eq!(find(&r, "clipboard", "bridge").level, Level::Warn);
    assert!(find(&r, "clipboard", "bridge").hint.as_deref().unwrap().contains("journalctl"));

    set_clipboard(&mut f, json!({"enabled": false, "data_control": true, "watching": true, "bridge": "on", "bridge_running": true, "bridge_active": true}));
    let r = evaluate(&f);
    assert_eq!(find(&r, "clipboard", "history").level, Level::Info);
    assert_eq!(find(&r, "clipboard", "bridge").level, Level::Ok);

    set_clipboard(&mut f, json!({"enabled": true, "data_control": true, "watching": true, "bridge": "off", "bridge_running": false, "bridge_active": false}));
    assert_eq!(find(&evaluate(&f), "clipboard", "bridge").level, Level::Info);

    // Eski kabuk (alan yok) ve kabuk yok.
    running(&mut f).clipboard = None;
    assert_eq!(find(&evaluate(&f), "clipboard", "history").level, Level::Info);
    f.live = Live::NotRunning;
    assert_eq!(find(&evaluate(&f), "clipboard", "history").level, Level::Info);
}

#[test]
fn external_clipboard_tools_get_a_fix_and_no_home() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    f.clip_tools = vec![
        ClipTool { tool: "hypr-bridge", kind: ClipKind::Bridge, at: Some((PathBuf::from("/home/u/.config/hypr/hyprland.conf"), 12)) },
        ClipTool { tool: "xclip", kind: ClipKind::Bridge, at: None },
        ClipTool { tool: "cliphist", kind: ClipKind::Manager, at: Some((PathBuf::from("/home/u/.config/hypr/autostart.conf"), 3)) },
    ];
    let r = evaluate(&f);
    let sync = find(&r, "clipboard", "sync_tools");
    assert_eq!(sync.level, Level::Warn);
    assert!(sync.text.contains("hypr-bridge (~/.config/hypr/hyprland.conf:12)"), "{}", sync.text);
    assert!(sync.text.contains("xclip (çalışıyor)"), "{}", sync.text);
    assert!(sync.hint.as_deref().unwrap().contains('#'));
    let managers = find(&r, "clipboard", "managers");
    assert_eq!(managers.level, Level::Warn);
    assert!(managers.hint.as_deref().unwrap().contains("[clipboard] enabled = false"));
    assert!(r.sections.iter().find(|s| s.id == "clipboard").unwrap().checks.iter().all(|c| c.id != "external"));
    assert!(!r.render().contains("/home/u") && !r.to_json().contains("/home/u"));
    let v: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
    let clip = v["sections"].as_array().unwrap().iter().find(|s| s["id"] == "clipboard").unwrap();
    let sync = clip["checks"].as_array().unwrap().iter().find(|c| c["id"] == "sync_tools").unwrap();
    assert_eq!(sync["data"]["tools"][0], json!({"tool": "hypr-bridge", "file": "~/.config/hypr/hyprland.conf", "line": 12}));

    // Nuxut köprüsü kapalıyken harici eşitleyici kullanıcının seçimi; geçmiş kapalıyken yönetici de.
    set_clipboard(&mut f, json!({"enabled": false, "data_control": true, "watching": false, "bridge": "off", "bridge_running": false, "bridge_active": false}));
    let r = evaluate(&f);
    assert_eq!(find(&r, "clipboard", "sync_tools").level, Level::Info);
    assert_eq!(find(&r, "clipboard", "managers").level, Level::Info);
}

#[test]
fn look_not_owned_files_suggest_takeover() {
    crate::i18n::use_turkish_for_tests();
    let mut f = healthy();
    running(&mut f).look = Some(json!({"writer": "nuxut-apply", "unified": true, "conflicts": [], "last_apply": {"external": [
        {"setting": "gtk3", "path": "/home/u/.config/gtk-3.0/settings.ini", "reason": "not_owned"},
        {"setting": "qt6", "path": "/home/u/.config/qt6ct/qt6ct.conf", "reason": "symlink_broken"},
    ], "paused": []}}));
    let r = evaluate(&f);
    let owned = find(&r, "look", "not_owned");
    assert_eq!(owned.level, Level::Warn);
    assert_eq!(owned.hint.as_deref(), Some("Nuxut bu dosyayı devralabilir: Ayarlar › Görünüm (ya da nuxut-shell look apply); eski hâli yedeklenir"));
    let external = find(&r, "look", "external");
    assert!(external.text.contains("qt6ct.conf") && !external.text.contains("gtk-3.0"), "{}", external.text);
}

#[test]
fn gather_clipboard_tools_from_hypr_exec_lines_and_processes() {
    let f = fake();
    let home = f.env.home.clone().unwrap();
    let hypr = home.join(".config/hypr");
    write(
        &hypr.join("hyprland.conf"),
        "source = ./autostart.conf\n# exec-once = cliphist store\nexec-once = ~/hypr-bridge.sh\n$term = xclip # değişken, exec değil\n",
    );
    write(&hypr.join("autostart.conf"), "exec-once = wl-paste --type text --watch cliphist store\n");
    let proc = &f.env.proc_root;
    write(&proc.join("20/comm"), "wl-paste\n");
    write(&proc.join("20/cmdline"), "wl-paste\0--type\0text\0--watch\0xclip\0-selection\0clipboard\0");
    write(&proc.join("21/comm"), "wl-paste\n");
    write(&proc.join("21/cmdline"), "wl-paste\0--no-newline\0"); // tek seferlik: sayılmaz
    write(&proc.join("22/comm"), "copyq\n");
    write(&proc.join("23/comm"), "hypr-bridge.sh\n");
    let facts = run_gather(&f.env, LiveReply::NotRunning, SessionRes::ByPid, &[]);
    type Got = (&'static str, ClipKind, Option<(PathBuf, usize)>);
    let got: Vec<Got> = facts.clip_tools.iter().map(|c| (c.tool, c.kind, c.at.clone())).collect();
    assert_eq!(got, [
        ("hypr-bridge", ClipKind::Bridge, Some((hypr.join("hyprland.conf"), 3))),
        ("cliphist", ClipKind::Manager, Some((hypr.join("autostart.conf"), 1))),
        ("copyq", ClipKind::Manager, None),
        ("hypr-bridge", ClipKind::Bridge, None),
        ("xclip", ClipKind::Bridge, None),
    ], "yorum ve exec olmayan satır sayılmaz; tek seferlik wl-paste sayılmaz");
}

/// Raporu üreten istemci kabukla aynı ikilidir (2026-10-16 canlı test: normal kurulumda hep
/// "2 süreç" uyarısı veriyordu): sayılmaz.
#[test]
fn instances_skip_the_reporting_client() {
    let f = fake();
    let proc = &f.env.proc_root;
    let exe = proc.join("bin/nuxut-shell");
    write(&exe, "");
    for pid in [42, 43, 44] {
        std::fs::create_dir_all(proc.join(pid.to_string())).unwrap();
        std::os::unix::fs::symlink(&exe, proc.join(format!("{pid}/exe"))).unwrap();
    }
    assert_eq!(gather::instances(proc, 42, 43), 2, "kabuk + bir fazla süreç; istemci sayılmaz");
    std::fs::remove_file(proc.join("44/exe")).unwrap();
    assert_eq!(gather::instances(proc, 42, 43), 1);
}

/// Doctor hiçbir yerde yetki yükseltme ya da sahiplik komutu önermez (2026-10-16): `not_owned`
/// dosyası root'a ait bir temaya sembolik bağ olabilir, `chown` bağı izleyip sistem dosyasını
/// değiştirir. Her sorun yolu açıkken rapor taranır; iki dilin katalogunda da yok.
#[test]
fn never_suggests_sudo_or_chown() {
    let mut f = healthy();
    f.install.in_path = false;
    f.install.missing_bins = vec!["nuxut-lock"];
    f.install.missing_helpers = vec!["nuxut-filechooser"];
    f.install.session_path = SessionPath { systemd: Some(false), hyprland: Some(false), shell: Some(false), envd_file: false };
    f.lock.binary_ok = false;
    f.lock.listen_logind = false;
    f.portal.portals_conf = ConfState::Managed { file_chooser_nuxut: false };
    f.disk_free = Some(1 << 30);
    f.fonts = vec![Font { role: "ui", family: "Inter".into(), installed: false }];
    f.packages = Some(Packages { total: 5, missing: vec!["papirus-icon-theme"], ..Default::default() });
    f.competitors = vec!["waybar"];
    f.packs = vec![PackHit { pack: "end4", file: PathBuf::from("/home/u/.config/hypr/hyprland.conf"), line: 1 }];
    f.clip_tools = vec![
        ClipTool { tool: "hypr-bridge", kind: ClipKind::Bridge, at: None },
        ClipTool { tool: "copyq", kind: ClipKind::Manager, at: None },
    ];
    set_clipboard(&mut f, json!({"enabled": true, "data_control": false, "watching": false, "bridge": "on", "bridge_running": false, "bridge_active": false}));
    running(&mut f).look = Some(json!({"writer": "nuxut-apply", "unified": true, "conflicts": [{"name": "QT_STYLE_OVERRIDE", "source": "session_env"}],
        "last_apply": {"external": [
            {"setting": "gtk3", "path": "/home/u/.config/gtk-3.0/settings.ini", "reason": "not_owned"},
            {"setting": "qt6", "path": "/home/u/.config/qt6ct/qt6ct.conf", "reason": "symlink_broken"},
        ], "paused": ["icons"]}}));
    running(&mut f).session = SessionRes::Auto;
    running(&mut f).instances = 2;
    running(&mut f).service = Service::AppScope;
    // Süreç geneli katalog Türkçe (dil değiştirmek paralel testleri bozar); İngilizce metinler
    // aşağıda katalogdan taranır.
    crate::i18n::use_turkish_for_tests();
    let r = evaluate(&f);
    assert!(r.sections.iter().flat_map(|s| &s.checks).filter(|c| c.level >= Level::Warn).count() >= 15, "sorun yolları açık");
    for out in [r.render(), r.to_json()] {
        let lower = out.to_lowercase();
        for bad in ["sudo", "chown", "doas", "pkexec"] {
            assert!(!lower.contains(bad), "{bad} önerilmez:\n{out}");
        }
    }
    for catalog in ["locales/tr.ftl", "locales/en.ftl"] {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(catalog)).unwrap();
        for line in text.lines().filter(|l| l.starts_with("shell-doctor-")) {
            let lower = line.to_lowercase();
            assert!(!lower.contains("sudo") && !lower.contains("chown"), "{catalog}: {line}");
        }
    }
}
