//! Komut satırı: kök yetkiyle çalışan sürecin tek girdisi. Sıkı ve sınırlı: en çok 64 argüman,
//! her biri en çok 128 bayt ASCII; bilinmeyen seçenek, yinelenen seçenek ve eksik değer hata.
//!
//! ```text
//! nuxut-system-helper boot-theme status
//! nuxut-system-helper boot-theme apply  [--targets grub,limine,plymouth] --scale N --lang tr|en --color rol=#rrggbb …
//! nuxut-system-helper boot-theme revert [--targets …]
//! nuxut-system-helper greeter status|apply|revert …   (bkz. crate::greeter)
//! ```

use nuxut_boot_theme::plan::Target;
use nuxut_boot_theme::{BootColors, Lang, ThemeOptions};

use crate::boot_theme::ApplyRequest;

pub const MAX_ARGS: usize = 64;
pub const MAX_ARG_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Status,
    Apply(ApplyRequest),
    Revert(Vec<Target>),
    /// Giriş ekranı alanı.
    Greeter(crate::greeter::Command),
}

fn targets(v: &str) -> Result<Vec<Target>, String> {
    let mut out = Vec::new();
    for name in v.split(',') {
        let t = Target::parse(name).ok_or_else(|| format!("bilinmeyen hedef: {name:?}"))?;
        if !out.contains(&t) {
            out.push(t);
        }
    }
    Ok(out)
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    if args.len() > MAX_ARGS {
        return Err("çok fazla argüman".into());
    }
    if let Some(a) = args.iter().find(|a| a.len() > MAX_ARG_LEN || !a.is_ascii() || a.chars().any(|c| c.is_ascii_control())) {
        return Err(format!("geçersiz argüman: {:?}", a.chars().take(32).collect::<String>()));
    }
    let mut it = args.iter().map(String::as_str);
    match it.next() {
        Some("boot-theme") => {}
        Some("greeter") => return crate::greeter::parse(it).map(Command::Greeter),
        _ => return Err("kullanım: nuxut-system-helper boot-theme|greeter status|apply|revert …".into()),
    }
    let verb = it.next().ok_or("eylem eksik")?;
    let mut tgt: Option<Vec<Target>> = None;
    let mut scale: Option<u32> = None;
    let mut lang: Option<Lang> = None;
    let mut colors: Vec<&str> = Vec::new();
    while let Some(opt) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{opt} değer bekliyor"));
        match opt {
            "--targets" if tgt.is_none() => tgt = Some(targets(value()?)?),
            "--scale" if scale.is_none() && verb == "apply" => {
                let v = value()?;
                scale = Some(v.parse().ok().filter(|s| (1..=ThemeOptions::MAX_SCALE).contains(s)).ok_or_else(|| format!("geçersiz ölçek: {v}"))?);
            }
            "--lang" if lang.is_none() && verb == "apply" => {
                lang = Some(match value()? {
                    "tr" => Lang::Tr,
                    "en" => Lang::En,
                    v => return Err(format!("bilinmeyen dil: {v}")),
                })
            }
            "--color" if verb == "apply" => colors.push(value()?),
            _ => return Err(format!("beklenmeyen argüman: {opt}")),
        }
    }
    let targets = tgt.unwrap_or_else(|| Target::ALL.to_vec());
    match verb {
        "status" => Ok(Command::Status),
        "revert" => Ok(Command::Revert(targets)),
        "apply" => Ok(Command::Apply(ApplyRequest {
            targets,
            colors: BootColors::from_args(colors).map_err(|e| e.to_string())?,
            scale: scale.ok_or("--scale eksik")?,
            lang: lang.ok_or("--lang eksik")?,
        })),
        v => Err(format!("bilinmeyen eylem: {v}")),
    }
}

/// [`parse`]'ın tersi (kullanıcı tarafı pkexec argümanlarını bununla kurar).
pub fn apply_args(req: &ApplyRequest) -> Vec<String> {
    let mut v = vec!["boot-theme".to_owned(), "apply".to_owned()];
    v.push("--targets".into());
    v.push(req.targets.iter().map(|t| t.name()).collect::<Vec<_>>().join(","));
    v.push("--scale".into());
    v.push(req.scale.to_string());
    v.push("--lang".into());
    v.push(req.lang.code().into());
    for c in req.colors.to_args() {
        v.push("--color".into());
        v.push(c);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn apply_round_trip() {
        let req = ApplyRequest { targets: vec![Target::Grub, Target::Plymouth], colors: BootColors::fallback(), scale: 2, lang: Lang::Tr };
        let args = apply_args(&req);
        assert!(args.len() <= MAX_ARGS);
        assert_eq!(parse(&args), Ok(Command::Apply(req)));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&s(&["boot-theme"])).is_err());
        assert!(parse(&s(&["other", "status"])).is_err());
        assert!(parse(&s(&["boot-theme", "status", "--scale", "2"])).is_err());
        assert!(parse(&s(&["boot-theme", "revert", "--targets", "grub,evil"])).is_err());
        assert!(parse(&s(&["boot-theme", "revert", "--targets", "grub", "--targets", "limine"])).is_err());
        assert!(parse(&s(&["boot-theme", "revert", "--targets"])).is_err());
        assert!(parse(&s(&["boot-theme", "revert", "--targets", "grub\nx"])).is_err());
        assert!(parse(&s(&["boot-theme", "revert", &"a".repeat(200)])).is_err());
        let mut args = apply_args(&ApplyRequest { targets: vec![Target::Grub], colors: BootColors::fallback(), scale: 1, lang: Lang::En });
        args[5] = "9".into();
        assert!(parse(&args).is_err(), "ölçek sınırı");
        let many = vec!["x".to_owned(); MAX_ARGS + 1];
        assert!(parse(&many).is_err());
        assert_eq!(parse(&s(&["boot-theme", "revert"])), Ok(Command::Revert(Target::ALL.to_vec())));
    }
}
