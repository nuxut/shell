//! Giriş yapabilecek kullanıcılar: `/etc/passwd` + `/etc/login.defs` (`UID_MIN`/`UID_MAX`).
//! Sistem kullanıcıları (aralık dışı uid) ve giriş kabuğu olmayanlar (`nologin`, `false`)
//! gizlenir. AccountsService gerekmez (greeter oturumunda olmayabilir); avatar onun ikon
//! dizininden okunur (greeter kullanıcının evine erişemez).

use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_PASSWD_BYTES: u64 = 1024 * 1024;
pub const MAX_USERS: usize = 64;
/// AccountsService'in dünya-okunur ikon dizini.
pub const ICONS_DIR: &str = "/var/lib/AccountsService/icons";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub name: String,
    /// GECOS'un ilk alanı; boşsa kullanıcı adı.
    pub display_name: String,
    pub uid: u32,
}

impl User {
    /// Avatar adayı; kullanıcı adı yol bileşeni olarak güvenli değilse `None`.
    pub fn avatar(&self, icons_dir: &Path) -> Option<PathBuf> {
        valid_name(&self.name).then(|| icons_dir.join(&self.name))
    }
}

/// POSIX taşınabilir kullanıcı adı + yaygın genişletmeler (`.`, `-`, `_`, sonda `$` yok).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && !name.starts_with(['-', '.'])
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UidRange {
    pub min: u32,
    pub max: u32,
}

impl Default for UidRange {
    fn default() -> Self {
        Self { min: 1000, max: 60000 }
    }
}

/// `login.defs`'ten `UID_MIN`/`UID_MAX`; okunamayan değer varsayılanda kalır.
pub fn parse_login_defs(text: &str) -> UidRange {
    let mut r = UidRange::default();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(value)) = (it.next(), it.next()) else { continue };
        let Ok(v) = value.parse::<u32>() else { continue };
        match key {
            "UID_MIN" => r.min = v,
            "UID_MAX" => r.max = v,
            _ => {}
        }
    }
    if r.min > r.max {
        return UidRange::default();
    }
    r
}

fn login_shell(shell: &str) -> bool {
    let base = shell.rsplit('/').next().unwrap_or("");
    !shell.is_empty() && !matches!(base, "nologin" | "false")
}

/// `passwd` satırları → kullanıcılar (uid sırasıyla, en çok [`MAX_USERS`]).
pub fn parse_passwd(text: &str, range: UidRange) -> Vec<User> {
    let mut users: Vec<User> = text
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() != 7 {
                return None;
            }
            let uid = f[2].parse::<u32>().ok()?;
            if uid < range.min || uid > range.max || !valid_name(f[0]) || !login_shell(f[6]) {
                return None;
            }
            let gecos = crate::protocol::sanitize(f[4].split(',').next().unwrap_or(""));
            let display_name = if gecos.is_empty() { f[0].to_owned() } else { gecos };
            Some(User { name: f[0].to_owned(), display_name, uid })
        })
        .collect();
    users.sort_by_key(|u| u.uid);
    users.dedup_by(|a, b| a.name == b.name);
    users.truncate(MAX_USERS);
    users
}

fn read_limited(path: &Path, limit: u64) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(limit).read_to_string(&mut text))
        .ok()?;
    Some(text)
}

/// Sistemdeki kullanıcılar (okunamazsa boş: greeter yine elle ad girişine izin verir).
pub fn load(passwd: &Path, login_defs: &Path) -> Vec<User> {
    let range = read_limited(login_defs, 256 * 1024).map(|t| parse_login_defs(&t)).unwrap_or_default();
    read_limited(passwd, MAX_PASSWD_BYTES).map(|t| parse_passwd(&t, range)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0::/root:/usr/bin/bash
greeter:x:964:964::/var/lib/greeter:/usr/bin/nologin
nobody:x:65534:65534:Kernel Overflow User:/:/usr/bin/nologin
ayse:x:1000:1000:Ayşe Yılmaz,,,:/home/ayse:/usr/bin/zsh
misafir:x:1001:1001::/home/misafir:/bin/bash
servis:x:1002:1002::/srv:/bin/false
-kotu:x:1003:1003::/x:/bin/sh
bozuk:x:abc:1::/x:/bin/sh
eksik:x:1004
";

    #[test]
    fn filters_system_and_nologin() {
        let users = parse_passwd(PASSWD, UidRange::default());
        let names: Vec<_> = users.iter().map(|u| (u.name.as_str(), u.display_name.as_str())).collect();
        assert_eq!(names, [("ayse", "Ayşe Yılmaz"), ("misafir", "misafir")]);
    }

    #[test]
    fn login_defs_range() {
        let r = parse_login_defs("# yorum\nUID_MIN\t\t 500\nUID_MAX 2000\nGID_MIN 10\n");
        assert_eq!(r, UidRange { min: 500, max: 2000 });
        assert_eq!(parse_login_defs("UID_MIN 9\nUID_MAX 1\n"), UidRange::default());
        assert_eq!(parse_passwd(PASSWD, UidRange { min: 1001, max: 1001 }).len(), 1);
    }

    #[test]
    fn avatar_path_is_safe() {
        let u = User { name: "ayse".into(), display_name: "A".into(), uid: 1000 };
        assert_eq!(u.avatar(Path::new(ICONS_DIR)).unwrap(), Path::new("/var/lib/AccountsService/icons/ayse"));
        let bad = User { name: "../etc".into(), display_name: "x".into(), uid: 1 };
        assert!(bad.avatar(Path::new(ICONS_DIR)).is_none());
        assert!(!valid_name("a/b") && !valid_name(".x") && valid_name("ali.veli_1"));
    }
}
