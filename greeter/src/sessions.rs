//! Oturum listesi: `wayland-sessions/*.desktop` (XDG veri dizinleri). Dosyalar root'un ama yine
//! de sınırlı okunur: dosya başına [`MAX_FILE_BYTES`], en çok [`MAX_SESSIONS`] oturum.
//!
//! `Exec` satırı Desktop Entry kurallarıyla argümanlara bölünür (çift tırnak, `\` kaçışı);
//! alan kodları (`%f`…) atılır. Kabuk yorumlanmaz: greetd komutu doğrudan `exec` eder.

use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_FILE_BYTES: u64 = 16 * 1024;
pub const MAX_SESSIONS: usize = 32;
/// Varsayılan oturumun dosya adı (kök): Nuxut paketinin kurduğu `nuxut.desktop`.
pub const NUXUT_SESSION: &str = "nuxut";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Dosya adı kökü (`hyprland`); hatırlanan seçim bununla eşleşir.
    pub id: String,
    pub name: String,
    pub cmd: Vec<String>,
    /// `XDG_SESSION_DESKTOP` / `XDG_CURRENT_DESKTOP` için (`DesktopNames`, yoksa kimlik).
    pub desktop_names: String,
}

impl Session {
    /// `start_session`'a giden ortam.
    pub fn env(&self) -> Vec<String> {
        let mut env = vec![
            "XDG_SESSION_TYPE=wayland".to_owned(),
            format!("XDG_SESSION_DESKTOP={}", self.id),
        ];
        if !self.desktop_names.is_empty() {
            env.push(format!("XDG_CURRENT_DESKTOP={}", self.desktop_names));
        }
        env
    }
}

/// Arama dizinleri: `$XDG_DATA_DIRS` (yoksa `/usr/local/share:/usr/share`) altında
/// `wayland-sessions`. Göreli yollar atılır.
pub fn search_dirs(xdg_data_dirs: Option<&str>) -> Vec<PathBuf> {
    let dirs = xdg_data_dirs.filter(|s| !s.trim().is_empty()).unwrap_or("/usr/local/share:/usr/share");
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs.split(':').map(Path::new).filter(|p| p.is_absolute()) {
        let p = d.join("wayland-sessions");
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// Dizinlerdeki oturumlar; aynı kimlik ilk dizinde bulunanla kalır. Ada göre sıralı, Nuxut
/// en başta.
pub fn load(dirs: &[PathBuf]) -> Vec<Session> {
    let mut sessions: Vec<Session> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            .collect();
        paths.sort();
        for path in paths {
            if sessions.len() >= MAX_SESSIONS {
                break;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else { continue };
            if sessions.iter().any(|s| s.id == id) {
                continue;
            }
            let mut text = String::new();
            let read = std::fs::File::open(&path)
                .and_then(|f| f.take(MAX_FILE_BYTES).read_to_string(&mut text));
            if read.is_err() {
                continue;
            }
            if let Some(s) = parse(&id, &text) {
                sessions.push(s);
            }
        }
    }
    sessions.sort_by(|a, b| {
        (a.id != NUXUT_SESSION, a.name.to_lowercase()).cmp(&(b.id != NUXUT_SESSION, b.name.to_lowercase()))
    });
    sessions
}

/// `[Desktop Entry]` grubundan `Name`, `Exec`, `DesktopNames`; `Hidden=true`/`NoDisplay=true`
/// ya da `Exec`'siz giriş `None`. Yerelleştirilmiş adlar (`Name[tr]`) kullanılmaz: oturum adları
/// özel isimdir.
pub fn parse(id: &str, text: &str) -> Option<Session> {
    let mut in_entry = false;
    let (mut name, mut exec, mut desktop_names) = (None, None, String::new());
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "Name" => name = Some(value.to_owned()),
            "Exec" => exec = Some(value.to_owned()),
            "DesktopNames" => desktop_names = value.trim_end_matches(';').to_owned(),
            "Hidden" | "NoDisplay" if value == "true" => return None,
            _ => {}
        }
    }
    let cmd = split_exec(&exec?)?;
    let name = crate::protocol::sanitize(&name.unwrap_or_else(|| id.to_owned()));
    if desktop_names.chars().any(|c| c.is_control() || c == '\0') {
        desktop_names.clear();
    }
    Some(Session { id: id.to_owned(), name, cmd, desktop_names })
}

/// Desktop Entry `Exec` bölmesi. Kapanmamış tırnak ya da boş sonuç `None`.
pub fn split_exec(exec: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut chars = exec.chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            '\\' if quoted => cur.push(chars.next()?),
            ' ' | '\t' if !quoted => {
                if has {
                    args.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            // Alan kodları (`%f`, `%U`…) oturumda anlamsız; yalnızca `%%` kalır.
            '%' => {
                if chars.next()? == '%' {
                    cur.push('%');
                    has = true;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if has {
        args.push(cur);
    }
    (!args.is_empty() && args.len() <= crate::protocol::MAX_ARGS).then_some(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_splitting() {
        assert_eq!(split_exec("/usr/bin/start-hyprland").unwrap(), ["/usr/bin/start-hyprland"]);
        assert_eq!(split_exec(r#"sh -c "echo \"a b\"" %U"#).unwrap(), ["sh", "-c", r#"echo "a b""#]);
        assert_eq!(split_exec("a  100%% \"\"").unwrap(), ["a", "100%", ""]);
        assert!(split_exec("\"open").is_none());
        assert!(split_exec("   ").is_none());
    }

    #[test]
    fn parse_entry() {
        let s = parse(
            "hyprland",
            "[Desktop Entry]\nName=Hyprland\nName[tr]=X\nExec=/usr/bin/start-hyprland\nDesktopNames=Hyprland;\n[Action x]\nExec=bad\n",
        )
        .unwrap();
        assert_eq!(s.name, "Hyprland");
        assert_eq!(s.cmd, ["/usr/bin/start-hyprland"]);
        assert_eq!(
            s.env(),
            ["XDG_SESSION_TYPE=wayland", "XDG_SESSION_DESKTOP=hyprland", "XDG_CURRENT_DESKTOP=Hyprland"]
        );
        assert!(parse("x", "[Desktop Entry]\nName=X\nExec=x\nNoDisplay=true\n").is_none());
        assert!(parse("x", "[Desktop Entry]\nName=X\n").is_none());
        assert!(parse("x", "Exec=x\n").is_none(), "grup dışı");
    }

    #[test]
    fn load_dedups_and_puts_nuxut_first() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("sway.desktop"), "[Desktop Entry]\nName=Sway\nExec=sway\n").unwrap();
        std::fs::write(b.path().join("sway.desktop"), "[Desktop Entry]\nName=Başka\nExec=x\n").unwrap();
        std::fs::write(b.path().join("nuxut.desktop"), "[Desktop Entry]\nName=Nuxut\nExec=nuxut-session\n").unwrap();
        std::fs::write(b.path().join("bozuk.desktop"), "\u{0}\u{1}").unwrap();
        std::fs::write(b.path().join("readme.txt"), "x").unwrap();
        let s = load(&[a.path().into(), b.path().into(), "/yok".into()]);
        let names: Vec<_> = s.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Nuxut", "Sway"]);
    }

    #[test]
    fn dirs_from_env() {
        assert_eq!(
            search_dirs(Some("/a:rel:/a:/b/")),
            [PathBuf::from("/a/wayland-sessions"), PathBuf::from("/b/wayland-sessions")]
        );
        assert_eq!(search_dirs(None).len(), 2);
    }
}
