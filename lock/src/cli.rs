//! Komut satırı ve tek örnek adı (saf; testli).

/// Tek örnek IPC'sinde uygulama adı (`$XDG_RUNTIME_DIR/nuxut/nuxut-lock.sock`).
pub const APP_NAME: &str = "nuxut-lock";
/// Test örneği adı: tanımlıysa IPC adı `nuxut-lock-<ad>` (kullanıcının kilidine komut gitmesin).
pub const INSTANCE_ENV: &str = "NUXUT_LOCK_INSTANCE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Kilitle; `locked`'tan sonra bu fd'ye "locked\n".
    Lock { ready_fd: Option<i32> },
    /// Çalışan örneğe komut (`status`).
    Msg(Vec<String>),
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Lock { ready_fd: None }),
        [flag, n] if flag == "--ready-fd" => n
            .parse::<i32>()
            .ok()
            .filter(|n| *n > 2)
            .map(|n| Command::Lock { ready_fd: Some(n) })
            .ok_or_else(|| "--ready-fd: 2'den büyük bir tamsayı bekleniyor".to_owned()),
        [msg, rest @ ..] if msg == "msg" && !rest.is_empty() => Ok(Command::Msg(rest.to_vec())),
        _ => Err("kullanım: nuxut-lock [--ready-fd <n>] | nuxut-lock msg status".to_owned()),
    }
}

/// Geçersiz örnek adı hatadır; sessizce varsayılana düşmek kullanıcının kilidine komut
/// göndermek olurdu.
pub fn app_name(instance: Option<&str>) -> Result<String, String> {
    match instance {
        None | Some("") => Ok(APP_NAME.to_owned()),
        Some(s)
            if s.len() <= 16
                && s.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                }) =>
        {
            Ok(format!("{APP_NAME}-{s}"))
        }
        Some(_) => Err(format!("{INSTANCE_ENV} geçersiz ([a-z0-9_-], en çok 16)")),
    }
}

/// Sahte doğrulayıcı yalnızca adlandırılmış test örneğinde: varsayılan kilit (kullanıcının
/// gerçek kilidi) ortamda sızan bir test değişkeniyle şifresiz açılabilir hâle gelmesin.
pub fn fake_auth_allowed(app_name: &str) -> bool {
    app_name != APP_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn commands() {
        assert_eq!(parse(&[]), Ok(Command::Lock { ready_fd: None }));
        assert_eq!(
            parse(&args(&["--ready-fd", "7"])),
            Ok(Command::Lock { ready_fd: Some(7) })
        );
        for bad in [
            &["--ready-fd", "1"][..],
            &["--ready-fd", "x"],
            &["--ready-fd"],
            &["msg"],
            &["--unlock"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
        assert_eq!(
            parse(&args(&["msg", "status"])),
            Ok(Command::Msg(args(&["status"])))
        );
    }

    #[test]
    fn instance_names() {
        assert_eq!(app_name(None).unwrap(), "nuxut-lock");
        assert_eq!(app_name(Some("test-lock")).unwrap(), "nuxut-lock-test-lock");
        assert!(app_name(Some("../x")).is_err());
        assert!(app_name(Some("Upper")).is_err());
        assert!(app_name(Some("a-very-long-instance-name")).is_err());
    }

    #[test]
    fn fake_auth_needs_instance() {
        assert!(!fake_auth_allowed(&app_name(None).unwrap()));
        assert!(!fake_auth_allowed(&app_name(Some("")).unwrap()));
        assert!(fake_auth_allowed(&app_name(Some("test-lock")).unwrap()));
    }
}
