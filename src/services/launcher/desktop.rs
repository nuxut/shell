//! `.desktop` dosyası ayrıştırıcı (Desktop Entry Specification 1.5).
//!
//! Yalnızca `[Desktop Entry]` ve `Actions=` ile listelenen `[Desktop Action <id>]` grupları
//! okunur. Değer kaçışları (`\s \n \t \r \\`, listelerde `\;`) çözülür; yerelleştirilmiş
//! anahtarlar (`Name[tr_TR]`) [`Locale`] eşleme kuralına göre seçilir. Dosya kullanıcının veya
//! sistemin veri dizininden gelir; yine de sınırlıdır: dosya [`MAX_FILE_BYTES`], değer
//! [`MAX_VALUE`] bayt, en çok [`MAX_ACTIONS`] eylem ve [`MAX_LIST`] liste öğesi.

use std::path::{Path, PathBuf};

/// Okunan `.desktop` dosyasının en büyük boyutu.
pub const MAX_FILE_BYTES: u64 = 64 << 10;
/// Tek bir değerin en büyük uzunluğu (bayt); aşan değer kırpılır.
pub const MAX_VALUE: usize = 4096;
/// En çok kaç masaüstü eylemi okunur.
pub const MAX_ACTIONS: usize = 16;
/// Liste değerlerinde (Keywords, OnlyShowIn ...) en çok öğe.
pub const MAX_LIST: usize = 64;
/// `MimeType=` değerinin en büyük uzunluğu ve öğe sayısı (medya oynatıcılar yüzlerce tür bildirir;
/// VLC ~6 KiB).
pub const MAX_MIME_VALUE: usize = 32 << 10;
pub const MAX_MIME_LIST: usize = 1024;

/// İleti yerel ayarı (`LC_ALL` → `LC_MESSAGES` → `LANG`); eşleme anahtarları öncelik sırasıyla:
/// `lang_COUNTRY@MOD`, `lang_COUNTRY`, `lang@MOD`, `lang`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Locale {
    keys: Vec<String>,
}

impl Locale {
    /// `lang_COUNTRY.ENCODING@MODIFIER` biçiminden (kodlama yok sayılır). `C`/`POSIX` → boş.
    pub fn parse(value: &str) -> Self {
        let (rest, modifier) = match value.split_once('@') {
            Some((r, m)) => (r, Some(m)),
            None => (value, None),
        };
        let rest = rest.split_once('.').map_or(rest, |(r, _)| r);
        let (lang, country) = match rest.split_once('_') {
            Some((l, c)) => (l, Some(c)),
            None => (rest, None),
        };
        if lang.is_empty() || lang == "C" || lang == "POSIX" {
            return Self::default();
        }
        let mut keys = Vec::with_capacity(4);
        if let (Some(c), Some(m)) = (country, modifier) {
            keys.push(format!("{lang}_{c}@{m}"));
        }
        if let Some(c) = country {
            keys.push(format!("{lang}_{c}"));
        }
        if let Some(m) = modifier {
            keys.push(format!("{lang}@{m}"));
        }
        keys.push(lang.to_owned());
        Self { keys }
    }

    /// Ortam değişkenlerinden.
    pub fn from_env() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .map(|v| Self::parse(&v))
            .unwrap_or_default()
    }

    /// Yerel ayar etiketinin önceliği (küçük = iyi); eşleşmezse `None`.
    fn rank(&self, tag: &str) -> Option<usize> {
        self.keys.iter().position(|k| k == tag)
    }
}

/// Dosyanın `Type` değeri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    Application,
    /// `Link`, `Directory` ya da bilinmeyen tür: başlatıcıda gösterilmez.
    Other,
}

/// `[Desktop Action <id>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    /// Kaçışları çözülmüş ham `Exec` (alan kodları henüz açılmamış).
    pub exec: Option<String>,
}

/// Ayrıştırılmış masaüstü girdisi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    /// Masaüstü dosya kimliği (`org.gnome.Nautilus.desktop`).
    pub id: String,
    pub path: PathBuf,
    pub kind: EntryType,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub keywords: Vec<String>,
    pub icon: Option<String>,
    pub exec: Option<String>,
    pub try_exec: Option<String>,
    /// `Path=`: çalışma dizini.
    pub working_dir: Option<String>,
    pub terminal: bool,
    pub dbus_activatable: bool,
    pub no_display: bool,
    pub hidden: bool,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
    pub startup_wm_class: Option<String>,
    /// `MimeType=`: bildirilen MIME türleri (varsayılan uygulamalar; `services::default_apps`).
    pub mime_types: Vec<String>,
    /// `Categories=` (ör. `TerminalEmulator`).
    pub categories: Vec<String>,
    pub actions: Vec<DesktopAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("[Desktop Entry] grubu yok")]
    NoMainGroup,
    #[error("zorunlu anahtar eksik: {0}")]
    Missing(&'static str),
}

/// Yerelleştirilebilir bir anahtarın en iyi değeri.
#[derive(Debug, Default)]
struct Localized {
    /// (öncelik, değer); öncelik `usize::MAX` = yerelleştirilmemiş.
    best: Option<(usize, String)>,
}

impl Localized {
    fn offer(&mut self, rank: usize, value: &str) {
        if self.best.as_ref().is_none_or(|(r, _)| rank < *r) {
            self.best = Some((rank, value.to_owned()));
        }
    }

    fn take(self) -> Option<String> {
        self.best.map(|(_, v)| v)
    }
}

const UNLOCALIZED: usize = usize::MAX;

#[derive(Debug, Default)]
struct GroupBuilder {
    name: Localized,
    generic_name: Localized,
    comment: Localized,
    keywords: Localized,
    icon: Option<String>,
    exec: Option<String>,
    try_exec: Option<String>,
    path: Option<String>,
    kind: Option<String>,
    terminal: Option<String>,
    dbus: Option<String>,
    no_display: Option<String>,
    hidden: Option<String>,
    only_show_in: Option<String>,
    not_show_in: Option<String>,
    wm_class: Option<String>,
    mime_types: Option<String>,
    categories: Option<String>,
    actions: Option<String>,
}

impl GroupBuilder {
    fn set(&mut self, key: &str, rank: usize, value: &str) {
        let first = |slot: &mut Option<String>| {
            if rank == UNLOCALIZED && slot.is_none() {
                *slot = Some(value.to_owned());
            }
        };
        match key {
            "Name" => self.name.offer(rank, value),
            "GenericName" => self.generic_name.offer(rank, value),
            "Comment" => self.comment.offer(rank, value),
            "Keywords" => self.keywords.offer(rank, value),
            "Icon" => first(&mut self.icon),
            "Exec" => first(&mut self.exec),
            "TryExec" => first(&mut self.try_exec),
            "Path" => first(&mut self.path),
            "Type" => first(&mut self.kind),
            "Terminal" => first(&mut self.terminal),
            "DBusActivatable" => first(&mut self.dbus),
            "NoDisplay" => first(&mut self.no_display),
            "Hidden" => first(&mut self.hidden),
            "OnlyShowIn" => first(&mut self.only_show_in),
            "NotShowIn" => first(&mut self.not_show_in),
            "StartupWMClass" => first(&mut self.wm_class),
            "MimeType" => first(&mut self.mime_types),
            "Categories" => first(&mut self.categories),
            "Actions" => first(&mut self.actions),
            _ => {}
        }
    }
}

/// Dosyayı okuyup ayrıştırır (boyut sınırlı, FIFO/aygıt açılmaz). Okuma bloklayan IO'dur.
pub fn read_entry(id: &str, path: &Path, locale: &Locale) -> std::io::Result<Result<DesktopEntry, ParseError>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32 | rustix::fs::OFlags::NOCTTY.bits() as i32)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "normal dosya değil ya da çok büyük"));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_FILE_BYTES).read_to_end(&mut bytes)?;
    Ok(parse(id, path, &String::from_utf8_lossy(&bytes), locale))
}

/// Metni ayrıştırır.
pub fn parse(id: &str, path: &Path, text: &str, locale: &Locale) -> Result<DesktopEntry, ParseError> {
    enum Group {
        Main,
        Action(usize),
        Skip,
    }
    let mut main: Option<GroupBuilder> = None;
    // Eylem grupları dosyada `Actions=`'tan önce de gelebilir: hepsi toplanır, sonra süzülür.
    let mut actions: Vec<(String, GroupBuilder)> = Vec::new();
    let mut group = Group::Skip;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = if header == "Desktop Entry" {
                if main.is_some() {
                    Group::Skip // tekrarlanan grup: ilki geçerli
                } else {
                    main = Some(GroupBuilder::default());
                    Group::Main
                }
            } else if let Some(action) = header.strip_prefix("Desktop Action ") {
                if actions.len() < MAX_ACTIONS * 2 && !actions.iter().any(|(a, _)| a == action) {
                    actions.push((action.to_owned(), GroupBuilder::default()));
                    Group::Action(actions.len() - 1)
                } else {
                    Group::Skip
                }
            } else {
                Group::Skip
            };
            continue;
        }
        let builder = match group {
            Group::Main => main.as_mut(),
            Group::Action(i) => actions.get_mut(i).map(|(_, b)| b),
            Group::Skip => None,
        };
        let Some(builder) = builder else { continue };
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim_end(), value.trim_start());
        let (key, rank) = match key.split_once('[') {
            Some((base, tag)) => match tag.strip_suffix(']').and_then(|t| locale.rank(t)) {
                Some(rank) => (base, rank),
                None => continue, // başka dil
            },
            None => (key, UNLOCALIZED),
        };
        let limit = if key == "MimeType" { MAX_MIME_VALUE } else { MAX_VALUE };
        builder.set(key, rank, clip(value, limit));
    }

    let main = main.ok_or(ParseError::NoMainGroup)?;
    let kind = match main.kind.as_deref() {
        Some("Application") => EntryType::Application,
        Some(_) => EntryType::Other,
        None => return Err(ParseError::Missing("Type")),
    };
    let name = main.name.take().map(|v| unescape(&v)).ok_or(ParseError::Missing("Name"))?;
    let listed: Vec<String> = main.actions.as_deref().map(split_list).unwrap_or_default();
    let mut out_actions = Vec::new();
    for action_id in listed {
        if out_actions.len() >= MAX_ACTIONS {
            break;
        }
        let Some(pos) = actions.iter().position(|(a, _)| *a == action_id) else { continue };
        let (aid, b) = actions.swap_remove(pos);
        let Some(aname) = b.name.take() else { continue }; // Name zorunlu
        out_actions.push(DesktopAction {
            id: aid,
            name: unescape(&aname),
            icon: b.icon.map(|v| unescape(&v)).filter(|s| !s.is_empty()),
            exec: b.exec.map(|v| unescape(&v)).filter(|s| !s.is_empty()),
        });
    }

    Ok(DesktopEntry {
        id: id.to_owned(),
        path: path.to_path_buf(),
        kind,
        name,
        generic_name: main.generic_name.take().map(|v| unescape(&v)).filter(|s| !s.is_empty()),
        comment: main.comment.take().map(|v| unescape(&v)).filter(|s| !s.is_empty()),
        keywords: main.keywords.take().map(|v| split_list(&v)).unwrap_or_default(),
        icon: main.icon.map(|v| unescape(&v)).filter(|s| !s.is_empty()),
        exec: main.exec.map(|v| unescape(&v)).filter(|s| !s.trim().is_empty()),
        try_exec: main.try_exec.map(|v| unescape(&v)).filter(|s| !s.is_empty()),
        working_dir: main.path.map(|v| unescape(&v)).filter(|s| !s.is_empty()),
        terminal: parse_bool(main.terminal.as_deref()),
        dbus_activatable: parse_bool(main.dbus.as_deref()),
        no_display: parse_bool(main.no_display.as_deref()),
        hidden: parse_bool(main.hidden.as_deref()),
        only_show_in: main.only_show_in.as_deref().map(split_list).unwrap_or_default(),
        not_show_in: main.not_show_in.as_deref().map(split_list).unwrap_or_default(),
        startup_wm_class: main.wm_class.map(|v| unescape(&v)).filter(|s| !s.is_empty()),
        mime_types: main.mime_types.as_deref().map(|v| split_list_max(v, MAX_MIME_LIST)).unwrap_or_default(),
        categories: main.categories.as_deref().map(split_list).unwrap_or_default(),
        actions: out_actions,
    })
}

fn clip(value: &str, limit: usize) -> &str {
    if value.len() <= limit {
        return value;
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// `true`/`false`; eski dosyalardaki `1`/`0` da kabul edilir.
fn parse_bool(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("true" | "1"))
}

/// `string`/`localestring` değer kaçışları: `\s \n \t \r \\`; bilinmeyen kaçış olduğu gibi kalır.
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// `;` ayrılmış liste (`\;` kaçışı korunur, değer kaçışları çözülür, boş öğeler atılır).
pub fn split_list(value: &str) -> Vec<String> {
    split_list_max(value, MAX_LIST)
}

/// [`split_list`], en çok `max` öğe.
pub fn split_list_max(value: &str, max: usize) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(';') => current.push(';'),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => current.push('\\'),
            },
            ';' => {
                let item = unescape(current.trim());
                if !item.is_empty() && items.len() < max {
                    items.push(item);
                }
                current.clear();
            }
            c => current.push(c),
        }
    }
    let item = unescape(current.trim());
    if !item.is_empty() && items.len() < max {
        items.push(item);
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr() -> Locale {
        Locale::parse("tr_TR.UTF-8")
    }

    #[test]
    fn locale_keys() {
        assert_eq!(Locale::parse("sr_RS.UTF-8@latin").keys, ["sr_RS@latin", "sr_RS", "sr@latin", "sr"]);
        assert_eq!(Locale::parse("tr_TR.UTF-8").keys, ["tr_TR", "tr"]);
        assert_eq!(Locale::parse("de").keys, ["de"]);
        assert!(Locale::parse("C.UTF-8").keys.is_empty());
        assert!(Locale::parse("POSIX").keys.is_empty());
    }

    #[test]
    fn parses_localized_entry_with_actions() {
        let text = "\
# yorum
[Desktop Action new-window]
Name=New Window
Name[tr]=Yeni Pencere
Exec=firefox --new-window %u

[Desktop Entry]
Version=1.5
Type=Application
Name=Firefox
Name[de]=Feuerfuchs
Name[tr]=Ateştilki
Name[tr_TR]=Firefox (TR)
GenericName=Web Browser
GenericName[tr]=Web Tarayıcı
Keywords=web;browser\\;x;internet;;
Keywords[tr]=web;tarayıcı;
Exec=firefox %u
Icon=firefox
Terminal=false
Actions=new-window;missing;private;
StartupWMClass=firefox
OnlyShowIn=GNOME;Hyprland;

[Desktop Action private]
Name=Private\\sWindow
Exec=firefox --private-window %u

[Other Group]
Name=Yanlış
";
        let e = parse("firefox.desktop", Path::new("/x/firefox.desktop"), text, &tr()).unwrap();
        assert_eq!(e.kind, EntryType::Application);
        assert_eq!(e.name, "Firefox (TR)", "tr_TR, tr'den önce");
        assert_eq!(e.generic_name.as_deref(), Some("Web Tarayıcı"));
        assert_eq!(e.keywords, ["web", "tarayıcı"]);
        assert_eq!(e.exec.as_deref(), Some("firefox %u"));
        assert_eq!(e.only_show_in, ["GNOME", "Hyprland"]);
        assert_eq!(e.startup_wm_class.as_deref(), Some("firefox"));
        assert_eq!(e.actions.len(), 2, "listede olmayan grup yok sayılır, eksik eylem atlanır");
        assert_eq!(e.actions[0].id, "new-window");
        assert_eq!(e.actions[0].name, "Yeni Pencere");
        assert_eq!(e.actions[1].name, "Private Window");

        let en = parse("firefox.desktop", Path::new("/x"), text, &Locale::default()).unwrap();
        assert_eq!(en.name, "Firefox");
        assert_eq!(en.keywords, ["web", "browser;x", "internet"]);
    }

    #[test]
    fn required_keys_and_flags() {
        let p = Path::new("/x");
        assert_eq!(parse("a", p, "Type=Application\nName=A", &tr()), Err(ParseError::NoMainGroup));
        assert_eq!(parse("a", p, "[Desktop Entry]\nName=A", &tr()), Err(ParseError::Missing("Type")));
        assert_eq!(parse("a", p, "[Desktop Entry]\nType=Application", &tr()), Err(ParseError::Missing("Name")));
        let e = parse(
            "a",
            p,
            "[Desktop Entry]\nType = Link\nName=A\nNoDisplay=true\nHidden=1\nTerminal=yes\nDBusActivatable=true\nExec=  ",
            &tr(),
        )
        .unwrap();
        assert_eq!(e.kind, EntryType::Other);
        assert!(e.no_display && e.hidden && e.dbus_activatable);
        assert!(!e.terminal, "yalnızca true/1");
        assert_eq!(e.exec, None, "boş Exec yok sayılır");
    }

    #[test]
    fn first_key_wins_and_duplicate_group_ignored() {
        let text = "[Desktop Entry]\nType=Application\nName=A\nExec=a\nExec=b\n[Desktop Entry]\nExec=c\n";
        let e = parse("a", Path::new("/x"), text, &tr()).unwrap();
        assert_eq!(e.exec.as_deref(), Some("a"));
    }

    #[test]
    fn mime_types_and_categories() {
        let many: String = (0..300).map(|i| format!("video/x-tur{i};")).collect();
        let text = format!("[Desktop Entry]\nType=Application\nName=V\nCategories=AudioVideo;Player;\nMimeType=text/plain;{many}video/mp4;\n");
        assert!(text.len() > MAX_VALUE, "uzun MimeType genel değer sınırını aşar");
        let e = parse("v.desktop", Path::new("/x"), &text, &tr()).unwrap();
        assert_eq!(e.categories, ["AudioVideo", "Player"]);
        assert_eq!(e.mime_types.len(), 302);
        assert_eq!(e.mime_types.last().map(String::as_str), Some("video/mp4"));
    }

    #[test]
    fn escapes_and_limits() {
        assert_eq!(unescape(r"a\sb\\c\nd\q\"), "a b\\c\nd\\q\\");
        assert_eq!(split_list(r"a;b\;c;d\\;"), ["a", "b;c", "d\\"]);
        let many = "x;".repeat(MAX_LIST + 10);
        assert_eq!(split_list(&many).len(), MAX_LIST);
        let long = format!("[Desktop Entry]\nType=Application\nName={}", "ş".repeat(MAX_VALUE));
        let e = parse("a", Path::new("/x"), &long, &tr()).unwrap();
        assert!(e.name.len() <= MAX_VALUE);
    }

    #[test]
    fn read_entry_rejects_large_and_special_files() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.desktop");
        std::fs::write(&big, vec![b'#'; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(read_entry("big.desktop", &big, &tr()).is_err());
        let fifo = dir.path().join("fifo.desktop");
        rustix::fs::mknodat(rustix::fs::CWD, &fifo, rustix::fs::FileType::Fifo, rustix::fs::Mode::from_raw_mode(0o600), 0)
            .unwrap();
        let started = std::time::Instant::now();
        assert!(read_entry("fifo.desktop", &fifo, &tr()).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        let ok = dir.path().join("ok.desktop");
        std::fs::write(&ok, "[Desktop Entry]\nType=Application\nName=Ok\nExec=ok\n").unwrap();
        assert_eq!(read_entry("ok.desktop", &ok, &tr()).unwrap().unwrap().name, "Ok");
    }
}
