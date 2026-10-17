//! Kapat / yeniden başlat: logind `Manager.PowerOff` / `Reboot` (etkileşimsiz). greeter'ın
//! oturumu koltuktaki etkin oturumdur; polkit varsayılanı (`allow_active=yes`) izin verir.
//! Düğmeler yalnızca `CanPowerOff`/`CanReboot` `yes` dönerse gösterilir. Çağrılar ayrı
//! thread'de (sistem bus'ı yavaş olabilir).

use std::time::Duration;

const LOGIN1: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    PowerOff,
    Reboot,
}

impl PowerAction {
    fn method(self) -> &'static str {
        match self {
            Self::PowerOff => "PowerOff",
            Self::Reboot => "Reboot",
        }
    }

    fn can_method(self) -> &'static str {
        match self {
            Self::PowerOff => "CanPowerOff",
            Self::Reboot => "CanReboot",
        }
    }
}

/// Test kipi listesi (`NUXUT_GREETER_POWER=poweroff,reboot`): bilinmeyen sözcükler atlanır.
pub fn parse_list(list: &str) -> Vec<PowerAction> {
    let mut out = Vec::new();
    for word in list.split(',').map(str::trim) {
        let action = match word {
            "poweroff" => PowerAction::PowerOff,
            "reboot" => PowerAction::Reboot,
            _ => continue,
        };
        if !out.contains(&action) {
            out.push(action);
        }
    }
    out
}

/// Yalnızca `"yes"` izinli sayılır (`challenge` greeter'da parola soramaz).
pub fn allowed(answer: &str) -> bool {
    answer == "yes"
}

fn connect(bus_address: Option<&str>) -> zbus::Result<zbus::blocking::Connection> {
    let builder = match bus_address {
        Some(addr) => zbus::blocking::connection::Builder::address(addr)?,
        None => zbus::blocking::connection::Builder::system()?,
    };
    builder.method_timeout(TIMEOUT).build()
}

/// Hangi eylemler izinli (bloklar; arka plan thread'inden).
pub fn query(bus_address: Option<&str>) -> Vec<PowerAction> {
    let Ok(conn) = connect(bus_address) else { return vec![] };
    let Ok(proxy) = zbus::blocking::Proxy::new(&conn, LOGIN1, PATH, MANAGER) else { return vec![] };
    [PowerAction::PowerOff, PowerAction::Reboot]
        .into_iter()
        .filter(|a| {
            proxy
                .call::<_, _, String>(a.can_method(), &())
                .is_ok_and(|s| allowed(&s))
        })
        .collect()
}

/// Eylemi ister (bloklar).
pub fn request(bus_address: Option<&str>, action: PowerAction) -> zbus::Result<()> {
    let conn = connect(bus_address)?;
    let proxy = zbus::blocking::Proxy::new(&conn, LOGIN1, PATH, MANAGER)?;
    proxy.call::<_, _, ()>(action.method(), &(false,))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_yes_is_allowed() {
        assert!(allowed("yes"));
        for s in ["challenge", "no", "na", ""] {
            assert!(!allowed(s));
        }
        assert_eq!(PowerAction::Reboot.method(), "Reboot");
    }

    #[test]
    fn test_list_parses_known_words_once() {
        assert_eq!(parse_list("reboot, poweroff,reboot,suspend"), [PowerAction::Reboot, PowerAction::PowerOff]);
        assert!(parse_list("").is_empty());
    }
}
