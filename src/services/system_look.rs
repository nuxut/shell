//! `nuxut-shell system-look status|apply|revert [--json] [--dry-run] [--no-boot] [--no-login]`:
//! açılış görünümü (`system_theme`) ve giriş ekranı (`login_screen`) tek yönetici onayıyla — kök
//! yardımcının `system-look` alanı (`crates/nuxut-system-helper/src/system_look.rs`, kendi polkit
//! eylemi). Ayarlar'ın Hoş geldin "Sistem görünümü" adımı bunu çağırır (2026-10-17).
//!
//! - `status --json`: `{"boot": <system-theme status>, "login": <login-screen status>}` (kök gerekmez).
//! - `apply`: istenen ve uygulanacak işi olan parçalar (açılışta uygulanabilir hedef; giriş ekranı
//!   etkin değil ve greetd kurulu). Yardımcı yoksa ya da eskiyse önce hazırlanmış sistem
//!   dosyalarını kurar (`system_theme::ensure_helper`).
//! - `revert`: yalnız uygulanmış parçalar.
//!
//! Olaylar yardımcınınki: her parçanın önünde `domain` (`boot-theme` | `greeter`). Çıkış kodu 0
//! başarı, 1 bir parça başarısız, 2 kullanım/ortam, 3 yönetici onayı verilmedi.

use std::path::Path;
use std::process::ExitCode;

use nuxut_boot_theme::plan::State;
use nuxut_system_helper::{greeter, system_look};

use crate::services::{login_screen, system_bundle, system_theme};

const USAGE: &str = "kullanım: nuxut-shell system-look status [--json] | apply [--json] [--dry-run] [--no-boot] [--no-login] | revert [--json] [--dry-run] [--no-boot] [--no-login]";

/// Geri alınacak parçalar: (açılış, giriş ekranı); hiçbiri yoksa `None`.
fn revert_parts(boot_applied: bool, login_recorded: bool, boot: bool, login: bool) -> Option<(bool, bool)> {
    let parts = (boot && boot_applied, login && login_recorded);
    (parts.0 || parts.1).then_some(parts)
}

pub fn run_cli(args: &[String]) -> ExitCode {
    let Some(verb) = args.first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let (mut json, mut dry, mut boot, mut login) = (false, false, true, true);
    for a in &args[1..] {
        match a.as_str() {
            "--json" => json = true,
            "--dry-run" if verb != "status" => dry = true,
            "--no-boot" if verb != "status" && boot => boot = false,
            "--no-login" if verb != "status" && login => login = false,
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let theme = match nuxut::Theme::load() {
        Ok(l) => l.value,
        Err(e) => {
            eprintln!("framework.toml okunamadı ({e}); varsayılan tema kullanılıyor");
            nuxut::Theme::default()
        }
    };
    let boot_status = system_theme::status(&theme);
    let login_status = login_screen::status_at(Path::new("/"));
    let argv = match verb.as_str() {
        "status" => {
            let installable = system_bundle::stage_dir().is_some_and(|s| system_bundle::stage_state(&s) == system_bundle::Stage::Ready);
            let v = serde_json::json!({
                "boot": boot_status,
                "login": { "status": login_status, "helper": boot_status.helper, "helper_installable": installable },
            });
            if json {
                println!("{v}");
            } else {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            return ExitCode::SUCCESS;
        }
        "apply" => {
            let req = system_theme::request(&theme, None, &boot_status.plan);
            let boot_req = (boot && !req.targets.is_empty()).then_some(req);
            let login_req = (login && login_status.greetd_installed && !login_status.active)
                .then(|| greeter::Request { keymap: login_screen::hyprland_keymap(), screen_reader: false, theme: true });
            if boot_req.is_none() && login_req.is_none() {
                eprintln!("uygulanacak iş yok");
                return ExitCode::SUCCESS;
            }
            system_look::apply_args(boot_req.as_ref(), login_req.as_ref())
        }
        "revert" => {
            let boot_applied = boot_status.plan.targets.iter().any(|t| t.state == State::Applied);
            match revert_parts(boot_applied, login_status.previous.is_some(), boot, login) {
                Some((b, l)) => system_look::revert_args(b, l),
                None => {
                    eprintln!("geri alınacak iş yok");
                    return ExitCode::SUCCESS;
                }
            }
        }
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
        && let Err(code) = system_theme::ensure_helper()
    {
        return code;
    }
    system_theme::run_helper(&argv, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revert_only_what_was_applied() {
        assert_eq!(revert_parts(true, true, true, true), Some((true, true)));
        assert_eq!(revert_parts(true, false, true, true), Some((true, false)), "giriş ekranı kaydı yok: dokunulmaz");
        assert_eq!(revert_parts(false, true, true, true), Some((false, true)));
        assert_eq!(revert_parts(true, true, false, true), Some((false, true)), "--no-boot");
        assert_eq!(revert_parts(false, false, true, true), None);
        assert_eq!(revert_parts(true, true, false, false), None);
    }
}
