//! Son giriş yapan kullanıcı ve oturum: greeter'ın durum dizininde (`state.toml`). Yazılamazsa
//! (dizin yok, izin yok) sessizce geçilir; okunan değerler yalnızca listede varsa kullanılır.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 4096;
const FILE: &str = "state.toml";

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Remembered {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// Durum dizini: `NUXUT_GREETER_STATE_DIR` (test), yoksa `/var/lib/nuxut-greeter/state` (greeter kullanıcısının; tmpfiles.d kurar).
pub fn state_dir() -> PathBuf {
    std::env::var_os("NUXUT_GREETER_STATE_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/var/lib/nuxut-greeter/state"))
}

pub fn load(dir: &Path) -> Remembered {
    let mut text = String::new();
    let ok = std::fs::File::open(dir.join(FILE))
        .and_then(|f| f.take(MAX_BYTES).read_to_string(&mut text))
        .is_ok();
    if !ok {
        return Remembered::default();
    }
    toml::from_str(&text).unwrap_or_default()
}

/// Atomik yazım (geçici dosya + rename).
pub fn save(dir: &Path, value: &Remembered) -> std::io::Result<()> {
    let text = toml::to_string(value).map_err(std::io::Error::other)?;
    let tmp = dir.join(".state.toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_garbage() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(load(d.path()), Remembered::default());
        let v = Remembered { user: Some("ayse".into()), session: Some("nuxut".into()) };
        save(d.path(), &v).unwrap();
        assert_eq!(load(d.path()), v);
        std::fs::write(d.path().join(FILE), "user = [").unwrap();
        assert_eq!(load(d.path()), Remembered::default());
        assert!(save(Path::new("/yok/dizin"), &v).is_err());
    }
}
