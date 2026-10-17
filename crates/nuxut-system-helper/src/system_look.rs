//! `system-look` alanı: açılış görünümü (`boot-theme`) ve giriş ekranı (`greeter`) tek yönetici
//! onayıyla (Hoş geldin "Sistem görünümü" adımı, 2026-10-17). Kendi polkit eylemi
//! (`org.nuxut.system-helper.system-look`); içeride iki alan sırayla ve kendi kurallarıyla çalışır.
//!
//! ```text
//! nuxut-system-helper system-look apply  [--no-boot | <boot-theme apply argümanları>] [--no-login | <greeter apply argümanları>]
//! nuxut-system-helper system-look revert [--no-boot | --targets …] [--no-login]
//! ```
//!
//! Argümanlar alanların kendi ayrıştırıcılarından geçer (seçenek adları çakışmaz); dosya yolu ya
//! da içerik kabul edilmez. Her parçanın önüne `domain` olayı yazılır; ardından o alanın olayları
//! değişmeden gelir (alan başına bir `done`). Bir parça başarısız olursa öbürü yine çalışır ve
//! kendi işlemiyle bağımsız kalır; çıkış kodu 1.

use nuxut_boot_theme::plan::Target;

use crate::boot_theme::{ApplyRequest, Helper};
use crate::events::Event;
use crate::greeter::{self, Greeter};

/// `domain` olayındaki adlar.
pub const DOMAIN_BOOT: &str = "boot-theme";
pub const DOMAIN_LOGIN: &str = "greeter";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `None` = o parça istenmedi (`--no-boot` / `--no-login`).
    Apply { boot: Option<ApplyRequest>, login: Option<greeter::Request> },
    Revert { boot: Option<Vec<Target>>, login: bool },
}

const BOOT_VALUE: [&str; 4] = ["--targets", "--scale", "--lang", "--color"];
const LOGIN_VALUE: [&str; 3] = ["--layout", "--variant", "--options"];
const LOGIN_FLAG: [&str; 2] = ["--screen-reader", "--no-theme"];

/// `system-look …` argümanları (alan adı çıkarılmış). Genel sınırlar (`args::parse`) önceden
/// uygulanmıştır.
pub fn parse<'a>(mut it: impl Iterator<Item = &'a str>) -> Result<Command, String> {
    let verb = it.next().ok_or("eylem eksik")?;
    if verb != "apply" && verb != "revert" {
        return Err(format!("bilinmeyen eylem: {verb}"));
    }
    let (mut boot, mut login) = (vec!["boot-theme".to_owned(), verb.to_owned()], vec![verb]);
    let (mut no_boot, mut no_login) = (false, false);
    while let Some(opt) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{opt} değer bekliyor")); // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
        match opt {
            "--no-boot" if !no_boot => no_boot = true,
            "--no-login" if !no_login => no_login = true,
            o if BOOT_VALUE.contains(&o) => {
                let v = value()?;
                boot.extend([o.to_owned(), v.to_owned()]);
            }
            o if LOGIN_VALUE.contains(&o) => {
                let v = value()?;
                login.extend([o, v]);
            }
            o if LOGIN_FLAG.contains(&o) => login.push(o),
            _ => return Err(format!("beklenmeyen argüman: {opt}")), // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
        }
    }
    if (no_boot && boot.len() > 2) || (no_login && login.len() > 1) {
        return Err("--no-boot/--no-login ile o alanın argümanı verilemez".into()); // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
    }
    if no_boot && no_login {
        return Err("yapılacak iş yok".into()); // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
    }
    use crate::args::Command as Boot;
    use greeter::Command as Login;
    let boot = if no_boot { None } else { Some(crate::args::parse(&boot)?) };
    let login = if no_login { None } else { Some(greeter::parse(login.into_iter())?) };
    if verb == "apply" {
        Ok(Command::Apply {
            boot: match boot {
                Some(Boot::Apply(r)) => Some(r),
                Some(_) => return Err("beklenmeyen önyükleme isteği".into()), // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
                None => None,
            },
            login: match login {
                Some(Login::Apply(r)) => Some(r),
                Some(_) => return Err("beklenmeyen giriş ekranı isteği".into()), // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
                None => None,
            },
        })
    } else {
        Ok(Command::Revert {
            boot: match boot {
                Some(Boot::Revert(t)) => Some(t),
                Some(_) => return Err("beklenmeyen önyükleme isteği".into()), // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
                None => None,
            },
            login: login.is_some(),
        })
    }
}

/// [`parse`]'ın tersi (kullanıcı tarafı pkexec argümanlarını bununla kurar).
pub fn apply_args(boot: Option<&ApplyRequest>, login: Option<&greeter::Request>) -> Vec<String> {
    let mut v = vec!["system-look".to_owned(), "apply".to_owned()];
    match boot {
        Some(r) => v.extend(crate::args::apply_args(r).into_iter().skip(2)),
        None => v.push("--no-boot".into()),
    }
    match login {
        Some(r) => v.extend(greeter::apply_args(r).into_iter().skip(2)),
        None => v.push("--no-login".into()),
    }
    v
}

pub fn revert_args(boot: bool, login: bool) -> Vec<String> {
    let mut v = vec!["system-look".to_owned(), "revert".to_owned()];
    if !boot {
        v.push("--no-boot".into());
    }
    if !login {
        v.push("--no-login".into());
    }
    v
}

/// Komutu çalıştırır: önce açılış görünümü, sonra giriş ekranı; ikisi de başarılıysa `true`.
pub fn run(helper: &Helper, caller: Option<u32>, cmd: &Command) -> bool {
    let greeter = Greeter { ops: &helper.ops, bins: &helper.bins, events: &helper.events, caller };
    let domain = |d: &str| helper.events.emit(Event::Domain { domain: d.to_owned() });
    let mut ok = true;
    match cmd {
        Command::Apply { boot, login } => {
            if let Some(r) = boot {
                domain(DOMAIN_BOOT);
                ok &= helper.apply(r);
            }
            if let Some(r) = login {
                domain(DOMAIN_LOGIN);
                ok &= greeter.apply(r);
            }
        }
        Command::Revert { boot, login } => {
            if let Some(t) = boot {
                domain(DOMAIN_BOOT);
                ok &= helper.revert(t);
            }
            if *login {
                domain(DOMAIN_LOGIN);
                ok &= greeter.revert();
            }
        }
    }
    ok
}

#[cfg(test)]
mod tests {
    use nuxut_boot_theme::{BootColors, Lang};

    use super::*;

    fn parse_all(v: &[String]) -> Result<Command, String> {
        crate::args::parse(v).and_then(|c| match c {
            crate::args::Command::SystemLook(c) => Ok(c),
            _ => Err("alan değil".into()),
        })
    }

    #[test]
    fn round_trip_and_limits() {
        let boot = ApplyRequest { targets: vec![Target::Grub, Target::Plymouth], colors: BootColors::fallback(), scale: 2, lang: Lang::Tr };
        let login = greeter::Request { keymap: Some(greeter::Keymap::new("tr,us", "", "grp:alt_shift_toggle").unwrap()), screen_reader: true, theme: true };
        let args = apply_args(Some(&boot), Some(&login));
        assert!(args.len() <= crate::args::MAX_ARGS, "{}", args.len());
        assert_eq!(parse_all(&args), Ok(Command::Apply { boot: Some(boot.clone()), login: Some(login.clone()) }));
        assert_eq!(parse_all(&apply_args(None, Some(&login))), Ok(Command::Apply { boot: None, login: Some(login) }));
        assert_eq!(parse_all(&apply_args(Some(&boot), None)), Ok(Command::Apply { boot: Some(boot), login: None }));
        assert_eq!(parse_all(&revert_args(true, true)), Ok(Command::Revert { boot: Some(Target::ALL.to_vec()), login: true }));
        assert_eq!(parse_all(&revert_args(false, true)), Ok(Command::Revert { boot: None, login: true }));
    }

    #[test]
    fn rejects_bad_input() {
        let a = |v: &[&str]| parse(v.iter().copied());
        assert!(a(&["status"]).is_err(), "durum alanların kendisinde");
        assert!(a(&["apply", "--no-boot", "--no-login"]).is_err());
        assert!(a(&["revert", "--no-boot", "--no-login"]).is_err());
        assert!(a(&["apply", "--no-boot", "--scale", "1", "--layout", "us"]).is_err(), "kapalı alanın argümanı");
        assert!(a(&["apply", "--no-login", "--no-login"]).is_err());
        assert!(a(&["apply", "--no-boot", "--layout", "us;rm"]).is_err(), "greeter doğrulaması");
        assert!(a(&["apply", "--no-boot", "--path", "/etc/shadow"]).is_err(), "yol yok");
        assert!(a(&["apply", "--no-login", "--scale", "1"]).is_err(), "--lang eksik: boot doğrulaması");
        assert!(a(&["revert", "--no-boot", "--layout", "us"]).is_err());
        assert!(a(&["revert", "--scale", "1"]).is_err());
        assert!(parse_all(&["system-look".to_owned(), "apply".to_owned(), "x".repeat(200)]).is_err());
    }

    /// Kendi polkit eylemi: yalnız `system-look` ilk argümanıyla, `auth_admin`.
    #[test]
    fn polkit_action_matches_domain() {
        let xml = include_str!("../../../dist/polkit/org.nuxut.system-helper.policy");
        let start = xml.find(&format!("<action id=\"{}\">", crate::ACTION_SYSTEM_LOOK)).unwrap();
        let action = &xml[start..start + xml[start..].find("</action>").unwrap()];
        assert!(action.contains("<annotate key=\"org.freedesktop.policykit.exec.argv1\">system-look</annotate>"));
        assert!(action.contains(&format!("<annotate key=\"org.freedesktop.policykit.exec.path\">{}</annotate>", crate::INSTALL_PATH)));
        assert!(action.contains("<allow_active>auth_admin</allow_active>") && !action.contains("keep"));
    }
}
