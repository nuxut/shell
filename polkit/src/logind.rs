//! Ajanın kaydolacağı logind oturumu (`unix-session` öznesi).
//!
//! Sıra:
//! 1. `GetSessionByPID(kendi pid)` — ajan oturumun kapsamında (ör. Hyprland `exec-once`).
//! 2. `$XDG_SESSION_ID` — systemd kullanıcı birimi oturum kapsamında değildir; compositor
//!    ortamı aktardıysa. Oturum logind'de var ve **bizim uid'imize** ait olmalı.
//! 3. Kullanıcının `Display` oturumu (logind'in seçtiği grafik oturum).
//!
//! polkitd ayrıca "User of caller and user of subject differs" denetimini yapar; başka
//! kullanıcının oturumuna kayıt zaten reddedilir.

use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const LOGIND: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
const USER: &str = "org.freedesktop.login1.User";

/// Oturum kimliği kabul edilebilir mi (logind kimlikleri kısa ASCII; polkitd'ye aynen gider).
pub fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}

async fn proxy<'a>(conn: &Connection, path: &'a str, iface: &'a str) -> zbus::Result<zbus::Proxy<'a>> {
    zbus::Proxy::new(conn, LOGIND, path, iface).await
}

async fn session_id_at(conn: &Connection, path: &OwnedObjectPath) -> zbus::Result<(String, u32)> {
    let p = proxy(conn, path.as_str(), SESSION).await?;
    let id: String = p.get_property("Id").await?;
    let (uid, _): (u32, OwnedObjectPath) = p.get_property("User").await?;
    Ok((id, uid))
}

/// Kendi oturumumuzun kimliği; nedenleriyle birlikte hata.
pub async fn own_session(
    conn: &Connection,
    pid: u32,
    uid: u32,
    env_session: Option<&str>,
) -> Result<String, String> {
    let manager = proxy(conn, MANAGER_PATH, MANAGER)
        .await
        .map_err(|e| format!("logind: {e}"))?;
    let mut tried = Vec::new();

    match manager
        .call::<_, _, OwnedObjectPath>("GetSessionByPID", &(pid,))
        .await
    {
        Ok(path) => match session_id_at(conn, &path).await {
            Ok((id, owner)) if owner == uid && valid_session_id(&id) => return Ok(id),
            Ok(_) => tried.push("pid oturumu başka kullanıcıya ait".to_owned()), // i18n: tanı kayıt hatası logu
            Err(e) => tried.push(format!("pid oturumu: {e}")),
        },
        Err(e) => tried.push(format!("GetSessionByPID: {e}")),
    }

    if let Some(env) = env_session.filter(|s| valid_session_id(s)) {
        match manager.call::<_, _, OwnedObjectPath>("GetSession", &(env,)).await {
            Ok(path) => match session_id_at(conn, &path).await {
                Ok((id, owner)) if owner == uid && id == env => return Ok(id),
                Ok(_) => tried.push("XDG_SESSION_ID başka kullanıcıya ait".to_owned()), // i18n: tanı kayıt hatası logu
                Err(e) => tried.push(format!("XDG_SESSION_ID: {e}")),
            },
            Err(e) => tried.push(format!("GetSession({env}): {e}")),
        }
    }

    match manager.call::<_, _, OwnedObjectPath>("GetUser", &(uid,)).await {
        Ok(user_path) => {
            let display = async {
                let p = proxy(conn, user_path.as_str(), USER).await?;
                p.get_property::<OwnedValue>("Display").await
            }
            .await;
            match display.map(<(String, OwnedObjectPath)>::try_from) {
                Ok(Ok((id, _))) if valid_session_id(&id) => return Ok(id),
                Ok(_) => tried.push("Display oturumu yok".to_owned()),
                Err(e) => tried.push(format!("Display: {e}")),
            }
        }
        Err(e) => tried.push(format!("GetUser: {e}")),
    }
    Err(tried.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids() {
        for ok in ["2", "c1", "self-1", "a_b.c"] {
            assert!(valid_session_id(ok), "{ok}");
        }
        for bad in ["", "a b", "1\n", "ş", &"x".repeat(65)] {
            assert!(!valid_session_id(bad), "{bad:?}");
        }
    }
}
