//! Onarım öncesi yedek: `<dizin>/<dosya adı>.<damga>` (çakışırsa `.2`, `.3` …), 0600, dizin 0700.
//! Var olan dosyanın üzerine asla yazılmaz (`create_new`). En yeni `keep` yedek kalır.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Bir dosyanın yedek sayısı üst sınırı (aynı dosya için).
pub const DEFAULT_KEEP: usize = 10;
/// Aynı damgalı en çok bu kadar yedek denenir.
pub const MAX_SUFFIX: u32 = 100;

/// `n`. yedeğin adı (1: sonek yok). Başka bir yazıcıyla (ör. ortam kapılı) yazan çağıranlar için.
pub fn file_name(name: &str, stamp: &str, n: u32) -> String {
    if n <= 1 { format!("{name}.{stamp}") } else { format!("{name}.{stamp}.{n}") }
}

/// `bytes`'ı `dir` altına yedekler; yedeğin yolu. `stamp`: sıralanabilir UTC damgası
/// (`20261016T101500Z`).
pub fn write(dir: &Path, name: &str, stamp: &str, bytes: &[u8], keep: usize) -> io::Result<PathBuf> {
    if name.contains('/') || stamp.contains('/') || name.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "yedek adı geçersiz"));
    }
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let mut path = None;
    for n in 1..=MAX_SUFFIX {
        let p = dir.join(file_name(name, stamp, n));
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&p) {
            Ok(mut f) => {
                f.write_all(bytes)?;
                f.sync_all()?;
                path = Some(p);
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let path = path.ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "yedek adı bulunamadı"))?;
    prune(dir, name, keep.max(1));
    Ok(path)
}

/// `name`'in yedekleri, en yeniden eskiye.
pub fn list(dir: &Path, name: &str) -> Vec<PathBuf> {
    let prefix = format!("{name}.");
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let n = e.file_name().into_string().ok()?;
            n.starts_with(&prefix).then(|| (sort_key(&n[prefix.len()..]), e.path()))
        })
        .collect();
    v.sort();
    v.into_iter().rev().map(|(_, p)| p).collect()
}

/// `20261016T101500Z.12` → sıralama anahtarı (sonek sayısal).
fn sort_key(rest: &str) -> String {
    let (stamp, n) = rest.split_once('.').map_or((rest, 1), |(s, n)| (s, n.parse().unwrap_or(0)));
    format!("{stamp}.{n:05}")
}

/// En yeni `keep` yedek dışındakileri siler.
pub fn prune(dir: &Path, name: &str, keep: usize) {
    for old in list(dir, name).into_iter().skip(keep) {
        let _ = fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_and_pruning() {
        let dir = std::env::temp_dir().join(format!("nuxut-heal-backup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let a = write(&dir, "shell.toml", "20261016T100000Z", b"a", 3).unwrap();
        let b = write(&dir, "shell.toml", "20261016T100000Z", b"b", 3).unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read(&a).unwrap(), b"a");
        write(&dir, "shell.toml", "20261016T110000Z", b"c", 3).unwrap();
        write(&dir, "framework.toml", "20261016T110000Z", b"x", 3).unwrap();
        let d = write(&dir, "shell.toml", "20261016T120000Z", b"d", 3).unwrap();
        let l = list(&dir, "shell.toml");
        assert_eq!(l.len(), 3);
        assert_eq!(l[0], d);
        assert!(!a.exists(), "en eski silinir");
        assert_eq!(list(&dir, "framework.toml").len(), 1);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(write(&dir, "../x", "s", b"", 1).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
