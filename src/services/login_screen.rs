//! `nuxut-shell login-screen status|apply|revert`: giriş ekranını (greetd + `nuxut-greeter`)
//! sistemde etkinleştirmenin kullanıcı tarafı. Kök işini kök yardımcının `greeter` alanı yapar
//! (`crates/nuxut-system-helper/src/greeter.rs`, ADR-0008), `pkexec` ile ve kendi polkit eylemiyle
//! (`org.nuxut.system-helper.greeter`). Tasarım: `docs/design/greeter.md` "Kurulum ve geri alma".
//!
//! - `status [--json]`: kök gerekmez; aynı dosyalardan (greetd, giriş ekranı dosyaları, kayıt,
//!   `display-manager.service` bağı) hesaplar. Hoş geldin adımı bunu okur.
//! - `apply [--json] [--dry-run] [--no-theme] [--screen-reader]`: klavye düzeni çalışan Hyprland'den
//!   (kullanıcının şu an yazdığı düzen; giriş ekranı şifreyi aynı düzende alsın).
//! - `revert [--json] [--dry-run]`.

use std::path::Path;
use std::process::ExitCode;

use nuxut_system_helper::greeter::{self, Keymap, Request, Status};

use crate::services::system_theme::{HelperState, ensure_helper, helper_state, run_helper};

const USAGE: &str = "kullanım: nuxut-shell login-screen status [--json] | apply [--json] [--dry-run] [--no-theme] [--screen-reader] | revert [--json] [--dry-run]";

/// Kök gerekmeyen durum (`/` kökünde).
pub fn status_at(root: &Path) -> Status {
    let p = |rel: &str| root.join(rel);
    let previous = std::fs::read(p(&format!("{}/{}", greeter::STATE_DIR, greeter::RECORD)))
        .ok()
        .and_then(|b| serde_json::from_slice::<greeter::Record>(&b).ok())
        .map(|r| r.previous_dm);
    let display_manager = std::fs::read_link(p(greeter::DM_LINK))
        .ok()
        .and_then(|t| t.file_name().and_then(|n| n.to_str()).map(str::to_owned))
        .filter(|n| greeter::valid_unit(n));
    Status {
        greetd_installed: p(greeter::GREETD_BIN).is_file(),
        greeter_installed: greeter::REQUIRED.iter().all(|f| p(f).is_file()),
        active: display_manager.as_deref() == Some(greeter::GREETD_UNIT) && previous.is_some(),
        display_manager,
        previous,
    }
}

/// Çalışan Hyprland'in klavye düzeni (`getoption`); doğrulanamazsa `None`.
fn hyprland_keymap() -> Option<Keymap> {
    let paths = crate::services::hyprland::HyprlandPaths::from_env().ok()?;
    let client = crate::services::hyprland::HyprlandClient::new(paths.socket1);
    let get = |opt: &str| -> String {
        client
            .request(&format!("j/getoption {opt}"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("str").and_then(|s| s.as_str()).map(|s| s.trim().to_owned()))
            .unwrap_or_default()
    };
    Keymap::new(&get("input:kb_layout"), &get("input:kb_variant"), &get("input:kb_options")).ok()
}

fn status_text(st: &Status, helper: HelperState) -> String {
    let yes = |b: bool| if b { "evet" } else { "hayır" };
    format!(
        "giriş ekranı: {} · greetd: {} · giriş ekranı dosyaları: {} · yardımcı: {helper:?}\ngiriş yöntemi: {} · önceki: {}",
        if st.active { "Nuxut (greetd)" } else { "Nuxut değil" },
        yes(st.greetd_installed),
        yes(st.greeter_installed),
        st.display_manager.as_deref().unwrap_or("tty"),
        st.previous.as_deref().unwrap_or("-"),
    )
}

pub fn run_cli(args: &[String]) -> ExitCode {
    let Some(verb) = args.first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let (mut json, mut dry, mut no_theme, mut screen_reader) = (false, false, false, false);
    for a in &args[1..] {
        match a.as_str() {
            "--json" => json = true,
            "--dry-run" if verb != "status" => dry = true,
            "--no-theme" if verb == "apply" => no_theme = true,
            "--screen-reader" if verb == "apply" => screen_reader = true,
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let helper = helper_state(Path::new(nuxut_system_helper::INSTALL_PATH));
    let argv: Vec<String> = match verb.as_str() {
        "status" => {
            let st = status_at(Path::new("/"));
            if json {
                let v = serde_json::json!({ "status": st, "helper": helper });
                println!("{v}");
            } else {
                println!("{}", status_text(&st, helper));
            }
            return ExitCode::SUCCESS;
        }
        "apply" => greeter::apply_args(&Request { keymap: hyprland_keymap(), screen_reader, theme: !no_theme }),
        "revert" => vec!["greeter".into(), "revert".into()],
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if dry {
        println!("yönetici onayıyla çalışır: {} {}", nuxut_system_helper::INSTALL_PATH, argv.join(" "));
        return ExitCode::SUCCESS;
    }
    if verb == "apply"
        && let Err(code) = ensure_helper()
    {
        return code;
    }
    run_helper(&argv, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_from_files() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let w = |rel: &str, s: &str| {
            std::fs::create_dir_all(r.join(rel).parent().unwrap()).unwrap();
            std::fs::write(r.join(rel), s).unwrap();
        };
        let st = status_at(r);
        assert!(!st.greetd_installed && !st.greeter_installed && !st.active && st.previous.is_none());
        w(greeter::GREETD_BIN, "x");
        for f in greeter::REQUIRED {
            w(f, "x");
        }
        w(&format!("{}/{}", greeter::STATE_DIR, greeter::RECORD), r#"{"version":1,"previous_dm":"sddm.service","config_existed":true,"config_backup":null}"#);
        std::fs::create_dir_all(r.join("etc/systemd/system")).unwrap();
        std::os::unix::fs::symlink("/usr/lib/systemd/system/greetd.service", r.join(greeter::DM_LINK)).unwrap();
        let st = status_at(r);
        assert!(st.greetd_installed && st.greeter_installed && st.active);
        assert_eq!(st.previous.as_deref(), Some("sddm.service"));
        assert!(status_text(&st, HelperState::Installed).contains("Nuxut (greetd)"));
    }
}
