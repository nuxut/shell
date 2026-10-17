//! Komut satırı, örnek adı ve test geçersiz kılmaları (saf; testli).

use std::path::PathBuf;

pub const APP_NAME: &str = "nuxut-greeter";
/// Test örneği: IPC adı `nuxut-greeter-<ad>`; yalnızca tanımlıyken test geçersiz kılmaları
/// (`NUXUT_GREETER_PASSWD`, `NUXUT_GREETER_SESSIONS`, `NUXUT_GREETER_ICONS`) okunur ve sistem
/// bus'ı (güç düğmeleri) kapalıdır.
pub const INSTANCE_ENV: &str = "NUXUT_GREETER_INSTANCE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run,
    Msg(Vec<String>),
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Run),
        [msg, rest @ ..] if msg == "msg" && !rest.is_empty() => Ok(Command::Msg(rest.to_vec())),
        _ => Err("kullanım: nuxut-greeter | nuxut-greeter msg status (GREETD_SOCK gerekir)".to_owned()), // i18n: tanı CLI kullanım satırı (stderr)
    }
}

pub fn app_name(instance: Option<&str>) -> Result<String, String> {
    match instance {
        None | Some("") => Ok(APP_NAME.to_owned()),
        Some(s)
            if s.len() <= 16
                && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-') =>
        {
            Ok(format!("{APP_NAME}-{s}"))
        }
        Some(_) => Err(format!("{INSTANCE_ENV} geçersiz ([a-z0-9_-], en çok 16)")), // i18n: tanı test örneği adı hatası (stderr)
    }
}

/// Test kipinde ortamdan mutlak yol; aksi hâlde `None` (üretimde sistem yolları).
pub fn test_path(test_mode: bool, value: Option<String>) -> Option<PathBuf> {
    value.filter(|_| test_mode).map(PathBuf::from).filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_names() {
        assert_eq!(parse(&[]).unwrap(), Command::Run);
        assert_eq!(parse(&["msg".into(), "status".into()]).unwrap(), Command::Msg(vec!["status".into()]));
        assert!(parse(&["x".into()]).is_err());
        assert_eq!(app_name(None).unwrap(), "nuxut-greeter");
        assert_eq!(app_name(Some("greeter-test")).unwrap(), "nuxut-greeter-greeter-test");
        assert!(app_name(Some("A/b")).is_err());
        assert_eq!(test_path(false, Some("/x".into())), None);
        assert_eq!(test_path(true, Some("rel".into())), None);
        assert_eq!(test_path(true, Some("/x".into())), Some(PathBuf::from("/x")));
    }
}
