//! Kenar çubuğu yerleri: xdg-user-dirs (`user-dirs.dirs`) ve GTK yer imleri (salt okunur).
//!
//! Dosyalar boyut sınırlı okunur, bozuk satır atlanır, yalnızca mutlak yerel yollar kabul edilir.
//! Var olmayan dizin gösterilmez (denetim çağıranda: `exists`).

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 64 << 10;
pub const MAX_BOOKMARKS: usize = 256;
const MAX_LABEL: usize = 128;

/// Bilinen kullanıcı dizinleri (kenar çubuğu sırası).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserDir {
    Desktop,
    Documents,
    Download,
    Pictures,
    Music,
    Videos,
}

impl UserDir {
    pub const ALL: [UserDir; 6] = [Self::Desktop, Self::Documents, Self::Download, Self::Pictures, Self::Music, Self::Videos];

    fn key(self) -> &'static str {
        match self {
            Self::Desktop => "XDG_DESKTOP_DIR",
            Self::Documents => "XDG_DOCUMENTS_DIR",
            Self::Download => "XDG_DOWNLOAD_DIR",
            Self::Pictures => "XDG_PICTURES_DIR",
            Self::Music => "XDG_MUSIC_DIR",
            Self::Videos => "XDG_VIDEOS_DIR",
        }
    }
}

fn read_limited(path: &Path) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut s = String::new();
    f.take(MAX_FILE_BYTES).read_to_string(&mut s).ok()?;
    Some(s)
}

/// `user-dirs.dirs` ayrıştırma: `XDG_X_DIR="$HOME/Belgeler"` ya da mutlak yol. `$HOME`'a eşit
/// dizin (xdg-user-dirs "devre dışı" anlamında) atlanır.
pub fn parse_user_dirs(text: &str, home: &Path) -> Vec<(UserDir, PathBuf)> {
    let mut out = Vec::new();
    for dir in UserDir::ALL {
        let Some(value) = text.lines().map(str::trim).filter(|l| !l.starts_with('#')).find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == dir.key()).then_some(v.trim())
        }) else {
            continue;
        };
        let Some(v) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else { continue };
        let path = if let Some(rest) = v.strip_prefix("$HOME") {
            let rest = rest.trim_start_matches('/');
            if rest.is_empty() { home.to_path_buf() } else { home.join(rest) }
        } else if v.starts_with('/') {
            PathBuf::from(v)
        } else {
            continue;
        };
        if path != home && !out.iter().any(|(_, p)| *p == path) {
            out.push((dir, path));
        }
    }
    out
}

/// Ortamdaki kullanıcı dizinleri (`$XDG_CONFIG_HOME/user-dirs.dirs`).
pub fn user_dirs(home: &Path) -> Vec<(UserDir, PathBuf)> {
    let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    read_limited(&config.join("user-dirs.dirs")).map(|t| parse_user_dirs(&t, home)).unwrap_or_default()
}

/// Yer imi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub path: PathBuf,
    /// Kullanıcının verdiği ad (yoksa `None`: dizin adı gösterilir).
    pub label: Option<String>,
}

/// Gevşek yüzde çözümü (GTK yer imleri): `%XX` çözülür, diğer baytlar olduğu gibi; NUL reddedilir.
fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    (!out.contains(&0)).then_some(out)
}

/// `file:///yol[ etiket]` satırları; yalnızca yerel (`file://` + boş ya da `localhost` host).
pub fn parse_bookmarks(text: &str) -> Vec<Bookmark> {
    text.lines()
        .filter_map(|l| {
            let (uri, label) = match l.split_once(' ') {
                Some((u, rest)) => (u, Some(rest.trim())),
                None => (l.trim(), None),
            };
            let rest = uri.strip_prefix("file://")?;
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
            if !rest.starts_with('/') || rest.contains(['?', '#']) {
                return None;
            }
            let path = PathBuf::from(OsString::from_vec(percent_decode(rest)?));
            let label = label.filter(|s| !s.is_empty()).map(|s| nuxut_portal_backend::chooser::protocol::clean_text(s, MAX_LABEL)).filter(|s| !s.is_empty());
            Some(Bookmark { path, label })
        })
        .take(MAX_BOOKMARKS)
        .collect()
}

/// `$XDG_CONFIG_HOME/gtk-3.0/bookmarks` (yoksa boş).
pub fn bookmarks(home: &Path) -> Vec<Bookmark> {
    let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    read_limited(&config.join("gtk-3.0/bookmarks")).map(|t| parse_bookmarks(&t)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_file() {
        let home = Path::new("/home/u");
        let text = "# yorum\nXDG_DOCUMENTS_DIR=\"$HOME/Belgeler\"\nXDG_DESKTOP_DIR=\"$HOME/\"\nXDG_DOWNLOAD_DIR=\"/mnt/indir\"\n\
                    XDG_MUSIC_DIR=goreli\nXDG_PICTURES_DIR=\"$HOME/Belgeler\"\nXDG_VIDEOS_DIR=\"$HOME/Videolar\"\n";
        let d = parse_user_dirs(text, home);
        assert_eq!(
            d,
            vec![(UserDir::Documents, PathBuf::from("/home/u/Belgeler")), (UserDir::Download, PathBuf::from("/mnt/indir")), (UserDir::Videos, PathBuf::from("/home/u/Videolar"))]
        );
    }

    #[test]
    fn bookmarks_file() {
        let text = "file:///home/u/Proje%20X Projelerim\nfile:///tmp\nsftp://host/x\nfile://localhost/srv\nfile://baska/x\nfile:///a%00b\nfile:///q?x\nfile:///e%E9 \u{202E}ters\n";
        let b = parse_bookmarks(text);
        assert_eq!(b.len(), 4);
        assert_eq!(b[0], Bookmark { path: "/home/u/Proje X".into(), label: Some("Projelerim".into()) });
        assert_eq!(b[1].label, None);
        assert_eq!(b[2].path, PathBuf::from("/srv"));
        assert_eq!(b[3].path.as_os_str().len(), 3);
        assert_eq!(b[3].label.as_deref(), Some("ters"));
    }
}
