use std::cell::RefCell;
use std::fs;

use nuxut_apply::Env;

use super::*;
use crate::services::hypr_config::base;
use crate::services::hypr_config::sync::{SyncEnv, SyncRequest, Syncer};

struct Setup {
    _tmp: tempfile::TempDir,
    hypr: PathBuf,
    env: SyncEnv,
}

fn setup(files: &[(&str, &str)]) -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let xdg = home.join("xdg-config");
    let hypr = xdg.join("hypr");
    fs::create_dir_all(&hypr).unwrap();
    for (name, text) in files {
        let p = hypr.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    let env = SyncEnv {
        apply: Env { config_home: Some(xdg), state_home: Some(home.join("state")), hyprland: None, ..Env::for_test(tmp.path()) },
        runtime_dir: Some(tmp.path().join("run")),
        isolated_control: false,
        under_systemd: false,
    };
    Setup { _tmp: tmp, hypr, env }
}

/// Sahte doğrulayıcı: aday dosyanın içeriğini kaydeder; sonucu verilir.
#[derive(Default)]
struct Fake {
    seen: RefCell<Vec<(PathBuf, String)>>,
    fail: Option<&'static str>,
}

impl Verifier for Fake {
    fn verify(&self, config: &Path) -> Result<(), String> {
        self.seen.borrow_mut().push((config.to_path_buf(), fs::read_to_string(config).unwrap()));
        self.fail.map_or(Ok(()), |e| Err(e.to_owned()))
    }
}

const END4: &str = "\
# end4 dots-hyprland
$mainMod = SUPER
source = ~/xdg-config/hypr/hyprland/default/keybinds.conf
monitor = ,preferred,auto,1
$ext = DP-2
monitor = $ext, 2560x1440@144, 1920x0, 1
monitor = $nope_unset_var_is_kept_out
";

/// end4 benzeri kurgu: yedek benzersiz ve özgünle bayt bayt aynı; aynı dakikadaki eski yedek
/// ezilmez (-2); yeni dosya işaretle başlar, temel katman başta, monitörler taşınır, Nuxut
/// katmanı ve en sonda kullanıcı dosyası; end4'ün dosyalarına dokunulmaz.
#[test]
fn end4_like_takeover_with_unique_backup() {
    let s = setup(&[
        ("hyprland.conf", END4),
        ("hyprland/default/keybinds.conf", "bind = SUPER, Return, exec, kitty\nbind = , XF86AudioMute, exec, wpctl x\n"),
        ("hyprland.conf.nuxut-yedek-2026-10-16-1430", "daha önceki yedek\n"),
    ]);
    let fake = Fake::default();
    let r = takeover(&s.env, &fake, "/usr/bin/nuxut-shell", Some("2026-10-16-1430"));
    assert_eq!(r.result, "taken_over", "{r:?}");
    assert_eq!(r.backup.as_deref(), Some("~/xdg-config/hypr/hyprland.conf.nuxut-yedek-2026-10-16-1430-2"));
    assert_eq!(fs::read_to_string(s.hypr.join("hyprland.conf.nuxut-yedek-2026-10-16-1430")).unwrap(), "daha önceki yedek\n", "eski yedek ezilmez");
    assert_eq!(fs::read_to_string(s.hypr.join("hyprland.conf.nuxut-yedek-2026-10-16-1430-2")).unwrap(), END4);
    let main = fs::read_to_string(s.hypr.join("hyprland.conf")).unwrap();
    assert!(main.starts_with(&format!("{MARKER_CONF}\n")), "{main}");
    assert!(is_managed_main(&main, ConfigKind::Hyprlang));
    let base_line = format!("source = ~/xdg-config/hypr/{}  {}", base::CONF_NAME, base::CONF_NOTE);
    let pos = |needle: &str| main.find(needle).unwrap_or_else(|| panic!("{needle}\n{main}"));
    assert!(pos(&base_line) < pos("monitor = ,preferred,auto,1"));
    assert!(pos("monitor = DP-2, 2560x1440@144, 1920x0, 1") < pos("source = ~/xdg-config/hypr/nuxut.conf"));
    assert!(pos("source = ~/xdg-config/hypr/nuxut.conf") < pos("source = ~/xdg-config/hypr/user.conf"));
    assert!(main.trim_end().ends_with("source = ~/xdg-config/hypr/user.conf"), "kullanıcı dosyası en son");
    assert!(!main.contains("nope_unset") && !main.contains("keybinds.conf"), "end4 artık yüklenmez");
    assert_eq!(r.monitors, 2);
    assert!(fs::read_to_string(s.hypr.join("hyprland/default/keybinds.conf")).unwrap().contains("kitty"), "end4 dosyasına dokunulmaz");
    // Kullanıcı dosyası ve varsayılanlarla Nuxut katmanları oluştu (kabuk hiç çalışmamış olsa da
    // oturum kısayolsuz kalmaz, kabuk sonraki girişte başlar); aday doğrulandı ve kaldırıldı.
    assert!(fs::read_to_string(s.hypr.join("user.conf")).unwrap().starts_with("# Kendi Hyprland"));
    let seeded_base = fs::read_to_string(s.hypr.join("nuxut-base.conf")).unwrap();
    assert!(seeded_base.contains("bind") && seeded_base.contains("/usr/bin/nuxut-shell msg"), "{seeded_base}");
    let seeded = fs::read_to_string(s.hypr.join("nuxut.conf")).unwrap();
    assert!(seeded.contains("exec-once = ") && seeded.contains("/usr/bin/nuxut-shell"), "{seeded}");
    let seen = fake.seen.borrow();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].1, main, "doğrulanan içerik yerine konan");
    assert!(!seen[0].0.exists(), "aday dosya silindi");
    // İkinci çalışma: zaten Nuxut'un; yedek yok, dosya aynı.
    let r = takeover(&s.env, &fake, "/usr/bin/nuxut-shell", Some("2026-10-16-1431"));
    assert_eq!(r.result, "already_managed");
    assert!(!s.hypr.join("hyprland.conf.nuxut-yedek-2026-10-16-1431").exists());
    // Eşitleme yeni dosyayı tanır: temel katman ve sondaki satır "present", satır eklenmez.
    let st = Syncer::new(SyncEnv { apply: Env { hyprland: Some("sahte".into()), ..s.env.apply.clone() }, ..s.env.clone() }).run(&request(), None);
    assert_eq!((st.source, st.base), ("present", Some("present")), "{st:?}");
    assert_eq!(fs::read_to_string(s.hypr.join("hyprland.conf")).unwrap(), main);
}

fn request() -> SyncRequest {
    use crate::services::hypr_config::generate::{BlurLook, LookGroup, MotionGroup, WindowsGroup};
    use crate::services::hypr_config::settings::{CompositorConfig, KeybindsConfig};
    let theme = nuxut::Theme::default();
    SyncRequest {
        compositor: CompositorConfig::default(),
        look: Some(LookGroup::from_theme(&theme, Some(BlurLook { size: 8, passes: 2 }))),
        motion: Some(MotionGroup::from_theme(&theme)),
        windows: Some(WindowsGroup::from_theme(&theme, 4, 8, 2)),
        keybinds: Some(KeybindsConfig::default().map),
        key_owner: Default::default(),
        hide_defaults: true,
        shell_exe: "/usr/bin/nuxut-shell".into(),
        session_env: Vec::new(),
        cursor_env: Vec::new(),
        input: None,
        force_add: false,
    }
}

/// Doğrulama geçmezse eski dosya yerinde kalır (yedek durur, zararsız); hata raporlanır.
#[test]
fn failed_verification_keeps_original() {
    let s = setup(&[("hyprland.conf", END4)]);
    let fake = Fake { fail: Some("Config error ... line 3"), ..Fake::default() };
    let r = takeover(&s.env, &fake, "/usr/bin/nuxut-shell", Some("2026-10-16-1500"));
    assert_eq!(r.result, "error");
    assert!(r.errors[0].contains("doğrulamasından geçmedi") && r.errors[0].contains("line 3"), "{r:?}");
    assert_eq!(fs::read_to_string(s.hypr.join("hyprland.conf")).unwrap(), END4);
    assert!(fs::read_dir(&s.hypr).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains("nuxut-aday")), "aday kalmaz");
}

/// Lua: `hyprland.lua` varsa Hyprland onu seçer (`.conf` yok sayılır; başvuru §1); üst düzey
/// `hl.monitor(…)` çağrıları taşınır; kullanıcı dosyası `require("user")` ile en son.
#[test]
fn lua_variant_wins_and_moves_monitor_calls() {
    let lua = "-- kullanıcının\nhl.monitor({ output = \"DP-1\", mode = \"2560x1440@144\", position = \"0x0\", scale = 1 })\n-- hl.monitor({ output = \"YORUM\" })\nhl.config({ general = { gaps_in = 3 } })\nhl.bind(\"SUPER + Q\", hl.dsp.exec_cmd(\"kitty\"))\n";
    let s = setup(&[("hyprland.lua", lua), ("hyprland.conf", "monitor = ,preferred,auto,1\n")]);
    let r = takeover(&s.env, &Fake::default(), "/usr/bin/nuxut-shell", Some("2026-10-16-1600"));
    assert_eq!(r.result, "taken_over", "{r:?}");
    assert_eq!(r.backup.as_deref(), Some("~/xdg-config/hypr/hyprland.lua.nuxut-yedek-2026-10-16-1600"));
    let main = fs::read_to_string(s.hypr.join("hyprland.lua")).unwrap();
    assert!(main.starts_with(&format!("{MARKER_LUA}\n")));
    assert!(main.contains(&format!("{}  {}\n", base::LUA_INCLUDE_LINE, base::LUA_NOTE)));
    assert!(main.contains("hl.monitor({ output = \"DP-1\", mode = \"2560x1440@144\", position = \"0x0\", scale = 1 })\n"));
    assert!(!main.contains("YORUM") && !main.contains("kitty"));
    assert!(main.trim_end().ends_with("require(\"user\")"));
    assert!(s.hypr.join("user.lua").is_file() && s.hypr.join(base::LUA_REL).is_file() && s.hypr.join("nuxut.lua").is_file());
    assert_eq!(fs::read_to_string(s.hypr.join("hyprland.conf")).unwrap(), "monitor = ,preferred,auto,1\n", "etkin olmayan .conf'a dokunulmaz");
}

/// Geri alma: en yeni yedek (zaman damgası, sonra sıra no) geri konur; o anki dosya önce
/// yedeklenir; hiçbir yedek silinmez.
#[test]
fn restore_backup_picks_newest_and_backs_up_current() {
    let s = setup(&[
        ("hyprland.conf", "nuxut dosyası\n"),
        ("hyprland.conf.nuxut-yedek-2026-10-15-0900", "eski\n"),
        ("hyprland.conf.nuxut-yedek-2026-10-16-1430", "yeni\n"),
        ("hyprland.conf.nuxut-yedek-2026-10-16-1430-2", "en yeni\n"),
        ("hyprland.conf.nuxut-yedek-bozuk", "ad uymuyor\n"),
    ]);
    let r = restore_backup(&s.env, Some("2026-10-16-1700"));
    assert_eq!(r.result, "restored", "{r:?}");
    assert_eq!(r.restored.as_deref(), Some("~/xdg-config/hypr/hyprland.conf.nuxut-yedek-2026-10-16-1430-2"));
    assert_eq!(fs::read_to_string(s.hypr.join("hyprland.conf")).unwrap(), "en yeni\n");
    assert_eq!(fs::read_to_string(s.hypr.join("hyprland.conf.nuxut-yedek-2026-10-16-1700")).unwrap(), "nuxut dosyası\n");
    assert_eq!(fs::read_dir(&s.hypr).unwrap().count(), 6, "hiçbir dosya silinmedi");
    let empty = setup(&[("hyprland.conf", "x\n")]);
    assert_eq!(restore_backup(&empty.env, None).result, "no_backup");
}

#[test]
fn names_keys_markers_and_verify_output() {
    assert_eq!(backup_name("hyprland.conf", "2026-10-16-1430", 1), "hyprland.conf.nuxut-yedek-2026-10-16-1430");
    assert_eq!(backup_key("hyprland.conf", "hyprland.conf.nuxut-yedek-2026-10-16-1430-12"), Some(("2026-10-16-1430".into(), 12)));
    assert_eq!(backup_key("hyprland.conf", "hyprland.conf.nuxut-yedek-2026-10-16-1430"), Some(("2026-10-16-1430".into(), 1)));
    assert_eq!(backup_key("hyprland.conf", "hyprland.conf.nuxut-yedek-2026-10-16-14x0"), None);
    assert_eq!(backup_key("hyprland.conf", "hyprland.lua.nuxut-yedek-2026-10-16-1430"), None);
    assert_eq!(backup_key("hyprland.conf", "hyprland.conf.nuxut-yedek-2026-10-16-1430-"), None);
    assert!(is_managed_main(&format!("\n{MARKER_CONF}\nsource = x\n"), ConfigKind::Hyprlang));
    assert!(!is_managed_main(&format!("# x\n{MARKER_CONF}\n"), ConfigKind::Hyprlang), "işaret ilk satırda olmalı");
    assert!(!is_managed_main(MARKER_CONF, ConfigKind::Lua));
    assert!(parse_verify_output("DEBUG …\n\n======== Config parsing result:\n\nconfig ok\n").is_ok());
    let e = parse_verify_output("======== Config parsing result:\n\nConfig error in file /x at line 3: invalid field\n").unwrap_err();
    assert!(e.contains("x satır 3") && !e.contains("invalid field"), "yalnız yer: {e}");
    assert!(parse_verify_output("segfault").is_err());
}

/// Test örneği gerçek ev dizinine yazmaz (kapı); yazılamayan durumda hiçbir dosya oluşmaz.
#[test]
fn test_instance_outside_temp_roots_is_inactive() {
    let s = setup(&[("hyprland.conf", END4)]);
    let mut env = s.env.clone();
    env.apply.config_home = env.apply.real_home.clone().map(|h| h.join(".config"));
    let r = takeover(&env, &Fake::default(), "/usr/bin/nuxut-shell", None);
    assert!(matches!(r.result, "inactive" | "error"), "{r:?}");
}

/// Gerçek `Hyprland --verify-config` (kuruluysa; compositor başlatmaz): devralınan ana dosya,
/// tohum katmanları ve kullanıcı dosyası iki arka uçta da "config ok". Lua adayı `.lua` ile
/// bitmeli — Hyprland sözdizimini uzantıdan seçer.
#[test]
fn real_hyprland_accepts_taken_over_configs() {
    if Command::new("Hyprland").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        eprintln!("Hyprland yok; atlandı");
        return;
    }
    for (name, text) in [("hyprland.conf", END4), ("hyprland.lua", "hl.monitor({ output = \"DP-1\", mode = \"preferred\", position = \"auto\", scale = 1 })\n")] {
        let s = setup(&[(name, text)]);
        let verifier = HyprlandVerifier {
            runtime_dir: s.env.runtime_dir.clone().unwrap(),
            home: s.env.apply.home.clone(),
            config_home: s.env.apply.config_home.clone(),
        };
        let r = takeover(&s.env, &verifier, "/usr/bin/nuxut-shell", Some("2026-10-16-1700"));
        assert_eq!(r.result, "taken_over", "{name}: {r:?}");
        assert_eq!(verifier.verify(&s.hypr.join(name)), Ok(()), "{name}: yerine konan dosya da geçer");
    }
}

#[test]
fn candidate_keeps_extension_and_backups_are_unbounded() {
    assert_eq!(candidate_name("hyprland.lua", 42), ".nuxut-aday-42-hyprland.lua");
    assert_eq!(backup_key("hyprland.conf", "hyprland.conf.nuxut-yedek-2026-10-16-1430-123456"), Some(("2026-10-16-1430".into(), 123_456)));
    assert_eq!(backup_key("hyprland.conf", "hyprland.conf.nuxut-yedek-2026-10-16-1430-01"), None);
    let tmp = tempfile::tempdir().unwrap();
    for n in 1..=12 {
        let p = create_backup(tmp.path(), "hyprland.conf", b"x", "2026-10-16-1430", 0o644).unwrap();
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), backup_name("hyprland.conf", "2026-10-16-1430", n));
    }
}

const HW_MAIN: &str = "\
$kb = tr
source = ~/xdg-config/hypr/hyprland/env.conf
source = ~/xdg-config/hypr/hyprland/general.conf
exec-once = ~/hypr-bridge.sh
exec-once = waybar
";
const HW_ENV: &str = "\
env = LIBVA_DRIVER_NAME,nvidia
env = __GLX_VENDOR_LIBRARY_NAME,nvidia
envd = AQ_DRM_DEVICES,/dev/dri/card1:/dev/dri/card0
env = XDG_DATA_DIRS,$HOME/.local/share:/usr/share
";
const HW_GENERAL: &str = "\
input {
    kb_layout = $kb
    kb_options = grp:alt_shift_toggle # yorum
    touchpad {
        natural_scroll = true
    }
}
cursor {
    no_hardware_cursors = true
}
cursor:inactive_timeout = 5
xwayland {
  force_zero_scaling = true
}
render {
  direct_scanout = 0
}
device {
    name = epic-mouse-v1
    sensitivity = -0.5
}
general {
  gaps_in = 3
}
windowrule = match:class ^(kitty)$, float on
bind = SUPER, Q, killactive,
";
const BRIDGE: &str = "#!/bin/sh\nwl-paste --watch xclip -selection clipboard &\nwhile :; do xclip -o; sleep 0.1; done\n";

fn hw_setup() -> Setup {
    let s = setup(&[("hyprland.conf", HW_MAIN), ("hyprland/env.conf", HW_ENV), ("hyprland/general.conf", HW_GENERAL)]);
    fs::write(s.env.apply.home.clone().unwrap().join("hypr-bridge.sh"), BRIDGE).unwrap();
    s
}

/// Devralma taşınması (kullanıcı onayı 2026-10-17): her env/envd satırı, input (iç içe touchpad),
/// cursor (blok + yollu atama), xwayland, render, device blokları `user-migrated.conf`'a,
/// kullanıcı değişkeni açılmış, yorum atılmış; ortam değişkeni (`$HOME`) hyprlang'a kalır. Ana
/// dosya onu Nuxut ayarlar katmanından ÖNCE okur. Taşınmayanların özeti ve harici pano köprüsü
/// raporda.
#[test]
fn migrates_env_input_device_and_reports_the_rest() {
    let s = hw_setup();
    let r = takeover(&s.env, &Fake::default(), "/usr/bin/nuxut-shell", Some("2026-10-17-0900"));
    assert_eq!(r.result, "taken_over", "{r:?}");
    assert_eq!(
        r.migrated,
        Migrated { env: 4, env_names: vec!["LIBVA_DRIVER_NAME".into(), "__GLX_VENDOR_LIBRARY_NAME".into(), "AQ_DRM_DEVICES".into(), "XDG_DATA_DIRS".into()], settings: 6 }
    );
    assert_eq!(r.not_migrated, NotMigrated { exec: vec!["hypr-bridge.sh".into(), "waybar".into()], binds: 1, rules: 1, other: 1 });
    assert_eq!(r.clipboard_bridge, Some("external_not_migrated"));
    assert_eq!(r.migrated_file.as_deref(), Some("~/xdg-config/hypr/user-migrated.conf"));
    let m = fs::read_to_string(s.hypr.join(MIGRATED_CONF)).unwrap();
    for want in [
        "env = LIBVA_DRIVER_NAME,nvidia\n",
        "env = __GLX_VENDOR_LIBRARY_NAME,nvidia\n",
        "envd = AQ_DRM_DEVICES,/dev/dri/card1:/dev/dri/card0\n",
        "env = XDG_DATA_DIRS,$HOME/.local/share:/usr/share\n",
        "input {\n    kb_layout = tr\n    kb_options = grp:alt_shift_toggle\n    touchpad {\n        natural_scroll = true\n    }\n}\n",
        "cursor {\n    no_hardware_cursors = true\n}\n",
        "cursor:inactive_timeout = 5\n",
        "xwayland {\n    force_zero_scaling = true\n}\n",
        "render {\n    direct_scanout = 0\n}\n",
        "device {\n    name = epic-mouse-v1\n    sensitivity = -0.5\n}\n",
    ] {
        assert!(m.contains(want), "{want}\n---\n{m}");
    }
    assert!(!m.contains("gaps_in") && !m.contains("windowrule") && !m.contains("bind") && !m.contains("exec"), "{m}");
    let main = fs::read_to_string(s.hypr.join("hyprland.conf")).unwrap();
    let pos = |needle: &str| main.find(needle).unwrap_or_else(|| panic!("{needle}\n{main}"));
    assert!(pos("source = ~/xdg-config/hypr/user-migrated.conf") < pos("source = ~/xdg-config/hypr/nuxut.conf"));
    assert!(pos(base::CONF_NAME) < pos("user-migrated.conf"));
    let json = r.to_json();
    assert!(json.contains("\"not_migrated\"") && json.contains("\"clipboard_bridge\": \"external_not_migrated\""), "{json}");
}

/// Önceki bir devralmanın `user-migrated.conf`'u farklıysa önce benzersiz yedeklenir; aynıysa
/// dokunulmaz. Doğrulama geçmezse taşınan dosya da yazılmaz.
#[test]
fn migrated_file_is_backed_up_and_not_written_on_failed_verify() {
    let s = hw_setup();
    let fail = Fake { fail: Some("Config error"), ..Fake::default() };
    let r = takeover(&s.env, &fail, "/usr/bin/nuxut-shell", Some("2026-10-17-0901"));
    assert_eq!(r.result, "error");
    assert!(!s.hypr.join(MIGRATED_CONF).exists(), "doğrulama geçmeden yazılmaz");
    fs::write(s.hypr.join(MIGRATED_CONF), "elle düzenlenmiş\n").unwrap();
    let r = takeover(&s.env, &Fake::default(), "/usr/bin/nuxut-shell", Some("2026-10-17-0902"));
    assert_eq!(r.result, "taken_over", "{r:?}");
    assert_eq!(fs::read_to_string(s.hypr.join("user-migrated.conf.nuxut-yedek-2026-10-17-0902")).unwrap(), "elle düzenlenmiş\n");
    assert!(fs::read_to_string(s.hypr.join(MIGRATED_CONF)).unwrap().contains("kb_layout = tr"));
    assert!(fs::read_dir(&s.hypr).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains("nuxut-aday")), "aday kalmaz");
}

/// Lua: üst düzey `hl.env(…)` çağrıları ana dosyaya, Nuxut ayarlar katmanından önce taşınır.
#[test]
fn lua_env_calls_are_migrated() {
    let s = setup(&[("hyprland.lua", "hl.env(\"LIBVA_DRIVER_NAME\", \"nvidia\")\n-- hl.env(\"X\", \"y\")\nhl.env(\"GBM_BACKEND\", \"nvidia-drm\", true)\n")]);
    let r = takeover(&s.env, &Fake::default(), "/usr/bin/nuxut-shell", Some("2026-10-17-0903"));
    assert_eq!(r.result, "taken_over", "{r:?}");
    assert_eq!(r.migrated.env, 2);
    let main = fs::read_to_string(s.hypr.join("hyprland.lua")).unwrap();
    let pos = |needle: &str| main.find(needle).unwrap_or_else(|| panic!("{needle}\n{main}"));
    assert!(pos("hl.env(\"GBM_BACKEND\", \"nvidia-drm\", true)") < pos("pcall(require, \"nuxut\")"), "{main}");
    assert!(!main.contains("\"X\""));
}

/// Gerçek `Hyprland --verify-config` (kuruluysa): taşınan dosyayla devralınan ana ayar geçer.
#[test]
fn real_hyprland_accepts_migrated_settings() {
    if Command::new("Hyprland").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        eprintln!("Hyprland yok; atlandı");
        return;
    }
    let s = hw_setup();
    let verifier = HyprlandVerifier { runtime_dir: s.env.runtime_dir.clone().unwrap(), home: s.env.apply.home.clone(), config_home: s.env.apply.config_home.clone() };
    let r = takeover(&s.env, &verifier, "/usr/bin/nuxut-shell", Some("2026-10-17-0904"));
    assert_eq!(r.result, "taken_over", "{r:?}");
    assert_eq!(verifier.verify(&s.hypr.join("hyprland.conf")), Ok(()));
}

/// Güvenlik (2026-10-17, sensitive-data-exposure): ortam DEĞERİ ve exec argümanları raporda,
/// rapor dosyasında, bildirimde ve doğrulama hatasında görünmez; yalnızca adlar. Kullanıcı
/// içeriği taşıyan yeni dosyalar (taşınan dosya, yedekler, kullanıcı dosyası, rapor) 0600.
#[test]
fn secrets_never_leave_the_migrated_file_and_files_are_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let s = setup(&[(
        "hyprland.conf",
        "env = SECRET_TOKEN,abc123\nexec-once = SECRET=abc123 mytool --token abc123\nexec-once = sh -c \"other --key=abc123\"\n",
    )]);
    fs::set_permissions(s.hypr.join("hyprland.conf"), fs::Permissions::from_mode(0o644)).unwrap();
    let r = takeover(&s.env, &Fake::default(), "/usr/bin/nuxut-shell", Some("2026-10-17-1200"));
    assert_eq!(r.result, "taken_over", "{r:?}");
    assert_eq!(r.migrated.env_names, ["SECRET_TOKEN"]);
    assert_eq!(r.not_migrated.exec, ["mytool", "other"]);
    let json = r.to_json();
    assert!(!json.contains("abc123"), "{json}");
    let saved = save_report(&s.env, &r).unwrap();
    assert!(!fs::read_to_string(&saved).unwrap().contains("abc123"));
    let summary = load_report(&s.env).unwrap();
    assert!(!format!("{summary:?}").contains("abc123"));
    // Değer yalnız taşınan dosyada (Hyprland'in okuması için) ve dosya yalnız sahibinin.
    let migrated = s.hypr.join(MIGRATED_CONF);
    assert!(fs::read_to_string(&migrated).unwrap().contains("env = SECRET_TOKEN,abc123"));
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&migrated), 0o600);
    assert_eq!(mode(&s.hypr.join("hyprland.conf.nuxut-yedek-2026-10-17-1200")), 0o600, "0644 kaynağın yedeği de 0600");
    assert_eq!(mode(&s.hypr.join(USER_CONF)), 0o600);
    assert_eq!(mode(&saved), 0o600);
    // Doğrulama hatası değeri yansıtsa da rapora yalnız dosya ve satır çıkar.
    let s2 = setup(&[("hyprland.conf", "env = SECRET_TOKEN,abc123\n")]);
    let fail = Fake { fail: Some("x"), ..Fake::default() };
    let r2 = takeover(&s2.env, &fail, "/usr/bin/nuxut-shell", Some("2026-10-17-1201"));
    assert_eq!(r2.result, "error");
    let out = "======== Config parsing result:\n\nConfig error in file /h/.config/hypr/user-migrated.conf at line 4: invalid value abc123\nweird abc123\n";
    let e = parse_verify_output(out).unwrap_err();
    assert!(!e.contains("abc123") && e.contains("user-migrated.conf satır 4"), "{e}");
    assert_eq!(lua_env_name("hl.env(\"SECRET_TOKEN\", \"abc123\")").as_deref(), Some("SECRET_TOKEN"));
}
