//! Portal ↔ `nuxut-filechooser` sözleşmesi (docs/design/file-chooser.md "Portal ↔ seçici
//! sözleşmesi"): stdin'de tek satır JSON [`ChooserRequest`], stdout'ta tek satır JSON
//! [`ChooserResult`]. Yollar ve dosya adları yüzde kodlu bayt dizisidir ([`encode_path`]): UTF-8
//! olmayan adlar kayıpsız taşınır.
//!
//! Sınırlar burada tek yerde; portal girdiyi bunlara göre kırpar, seçici çıktıyı bunlara göre
//! üretir, portal sonucu yeniden doğrular.

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// İstek satırının en büyük boyu (bayt).
pub const MAX_REQUEST_BYTES: usize = 1 << 20;
/// Sonuç satırının en büyük boyu (bayt).
pub const MAX_RESULT_BYTES: usize = 8 << 20;
/// Dönen en çok yol.
pub const MAX_PATHS: usize = 2048;
/// Bir yolun en büyük boyu (bayt, `PATH_MAX`).
pub const MAX_PATH_BYTES: usize = 4096;
/// Dosya adının en büyük boyu (bayt, `NAME_MAX`).
pub const MAX_NAME_BYTES: usize = 255;
pub const MAX_TITLE_BYTES: usize = 256;
pub const MAX_LABEL_BYTES: usize = 128;
pub const MAX_FILTERS: usize = 64;
pub const MAX_RULES: usize = 64;
pub const MAX_PATTERN_BYTES: usize = 256;
pub const MAX_CHOICES: usize = 32;
pub const MAX_CHOICE_OPTIONS: usize = 64;
pub const MAX_ID_BYTES: usize = 128;
/// `SaveFiles` `files` en çok.
pub const MAX_FILES: usize = 1024;
/// `parent_window` tanıtıcısının en büyük boyu (`wayland:` öneki hariç).
pub const MAX_HANDLE_BYTES: usize = 128;

/// Seçici çıkış kodları.
pub const EXIT_SELECTED: i32 = 0;
pub const EXIT_CANCELLED: i32 = 1;
pub const EXIT_ERROR: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Open,
    Save,
    SaveFiles,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "pattern", rename_all = "snake_case")]
pub enum Rule {
    Glob(String),
    Mime(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    pub name: String,
    pub rules: Vec<Rule>,
}

/// Seçim kutusu: `options` boşsa onay kutusu (`"true"`/`"false"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub options: Vec<(String, String)>,
    pub initial: String,
}

impl Choice {
    pub fn is_toggle(&self) -> bool {
        self.options.is_empty()
    }

    /// `value` bu seçim için geçerli mi.
    pub fn accepts(&self, value: &str) -> bool {
        if self.is_toggle() { value == "true" || value == "false" } else { self.options.iter().any(|(id, _)| id == value) }
    }

    /// Başlangıç değeri: geçerliyse `initial`, değilse onay kutusunda `"false"`, listede ilk seçenek.
    pub fn initial_value(&self) -> String {
        if self.accepts(&self.initial) {
            self.initial.clone()
        } else if self.is_toggle() {
            "false".to_owned()
        } else {
            self.options.first().map(|(id, _)| id.clone()).unwrap_or_default()
        }
    }
}

/// Seçiciye giden istek. Metinler portalda temizlenmiş ve kırpılmıştır.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChooserRequest {
    pub mode: Mode,
    pub title: String,
    pub app_id: String,
    /// Doğrulanmış Wayland tanıtıcısı (`wayland:` öneki atılmış) ya da boş.
    pub parent: String,
    pub accept_label: Option<String>,
    pub modal: bool,
    pub multiple: bool,
    pub directory: bool,
    pub filters: Vec<Filter>,
    /// `filters` içindeki seçili filtre.
    pub current_filter: Option<usize>,
    pub choices: Vec<Choice>,
    /// Kodlu yol ([`encode_path`]).
    pub current_folder: Option<String>,
    /// Kaydetmede önerilen ad (UTF-8 metin; dosya adı kurallarıyla temizlenmiş).
    pub current_name: Option<String>,
    /// `SaveFiles`: kodlu dosya adları.
    pub files: Vec<String>,
}

impl std::fmt::Debug for ChooserRequest {
    /// Yalnızca kip, bayrak ve sayılar (yol, ad, başlık kullanıcı içeriği: loglanmaz).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChooserRequest")
            .field("mode", &self.mode)
            .field("multiple", &self.multiple)
            .field("directory", &self.directory)
            .field("modal", &self.modal)
            .field("parent", &!self.parent.is_empty())
            .field("filters", &self.filters.len())
            .field("choices", &self.choices.len())
            .field("files", &self.files.len())
            .finish_non_exhaustive()
    }
}

/// Seçicinin sonucu.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChooserResult {
    /// Kodlu mutlak yollar.
    pub paths: Vec<String>,
    /// (seçim kimliği, değer).
    pub choices: Vec<(String, String)>,
    /// Seçili filtrenin `filters` indisi.
    pub filter: Option<usize>,
}

impl std::fmt::Debug for ChooserResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChooserResult")
            .field("paths", &self.paths.len())
            .field("choices", &self.choices.len())
            .field("filter", &self.filter)
            .finish()
    }
}

impl ChooserRequest {
    /// Tek satır JSON (sonunda `\n`).
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }

    /// Satırı ayrıştırır; boyut sınırı ve içerik sınırları yeniden denetlenir (seçici portala
    /// güvenir ama bozuk/elle verilmiş girdiyle panik olmasın).
    pub fn from_line(line: &str) -> Result<Self, &'static str> {
        if line.len() > MAX_REQUEST_BYTES {
            return Err("istek çok uzun");
        }
        let mut r: Self = serde_json::from_str(line.trim_end()).map_err(|_| "istek ayrıştırılamadı")?;
        r.filters.truncate(MAX_FILTERS);
        for f in &mut r.filters {
            f.rules.truncate(MAX_RULES);
        }
        r.choices.truncate(MAX_CHOICES);
        r.files.truncate(MAX_FILES);
        if r.current_filter.is_some_and(|i| i >= r.filters.len()) {
            r.current_filter = None;
        }
        r.validate()?;
        Ok(r)
    }
}

impl ChooserRequest {
    /// Yol/ad alanlarının katı denetimi (savunma derinliği; portal da denetler): `files` her ad
    /// [`valid_name`], `current_name` [`valid_name`], `current_folder` [`valid_path`]. Biri bile
    /// geçersizse istek reddedilir.
    pub fn validate(&self) -> Result<(), &'static str> {
        for f in &self.files {
            if !decode_bytes(f).is_some_and(|b| valid_name(&b)) {
                return Err("geçersiz dosya adı");
            }
        }
        if self.current_name.as_deref().is_some_and(|n| !valid_name(n.as_bytes())) {
            return Err("geçersiz önerilen ad");
        }
        if self.current_folder.as_deref().is_some_and(|f| !decode_path(f).is_some_and(|p| valid_path(&p))) {
            return Err("geçersiz başlangıç klasörü");
        }
        Ok(())
    }
}

impl ChooserResult {
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_RESULT_BYTES {
            return Err("sonuç çok uzun");
        }
        let text = std::str::from_utf8(bytes).map_err(|_| "sonuç UTF-8 değil")?;
        let line = text.trim_end_matches('\n');
        if line.contains('\n') {
            return Err("sonuç tek satır değil");
        }
        serde_json::from_str(line).map_err(|_| "sonuç ayrıştırılamadı")
    }
}

/// Yüzde kodlamada olduğu gibi kalan baytlar (RFC 3986 unreserved + `/`).
fn plain(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/')
}

/// Bayt dizisini yüzde kodlar (yol ya da ad).
pub fn encode_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if plain(b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn encode_path(path: &Path) -> String {
    encode_bytes(path.as_os_str().as_bytes())
}

/// Katı çözüm: yalnızca [`plain`] baytlar ve geçerli `%XX`; NUL reddedilir.
pub fn decode_bytes(s: &str) -> Option<Vec<u8>> {
    let src = s.as_bytes();
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        let b = src[i];
        if b == b'%' {
            let hex = src.get(i + 1..i + 3)?;
            let v = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            if v == 0 {
                return None;
            }
            out.push(v);
            i += 3;
        } else if plain(b) {
            out.push(b);
            i += 1;
        } else {
            return None;
        }
    }
    Some(out)
}

pub fn decode_path(s: &str) -> Option<PathBuf> {
    decode_bytes(s).map(|b| PathBuf::from(OsString::from_vec(b)))
}

/// `file://` URI (host boş).
pub fn file_uri(path: &Path) -> String {
    format!("file://{}", encode_path(path))
}

/// Yol sözleşmeye uygun mu: mutlak, ≤ [`MAX_PATH_BYTES`], `.`/`..` bileşeni yok, NUL yok.
pub fn valid_path(path: &Path) -> bool {
    let bytes = path.as_os_str().as_bytes();
    path.is_absolute()
        && bytes.len() <= MAX_PATH_BYTES
        && !bytes.contains(&0)
        && path.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
        // `components` aradaki ve sondaki `.` bileşenini yutar; ham baytta da bakılır (`..` bileşen
        // olarak kalır ve yukarıda reddedilir).
        && !bytes.windows(3).any(|w| w == b"/./")
        && !bytes.ends_with(b"/.")
}

/// Dosya adı geçerli mi: boş değil, `.`/`..` değil, `/` ve NUL yok, ≤ [`MAX_NAME_BYTES`].
pub fn valid_name(name: &[u8]) -> bool {
    !name.is_empty() && name != b"." && name != b".." && name.len() <= MAX_NAME_BYTES && !name.contains(&b'/') && !name.contains(&0)
}

/// Görünecek metinden denetim, yön ve sıfır genişlikli karakterleri atar; `max` bayta karakter
/// sınırında kırpar.
pub fn clean_text(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max));
    for c in s.chars() {
        if c.is_control() || is_format(c) {
            continue;
        }
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

/// Yön denetimleri ve sıfır genişlikli karakterler (Trojan Source; görünmez ad).
fn is_format(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{061C}' | '\u{FEFF}' | '\u{00AD}')
}

/// Mnemonik alt çizgileri atar (`_Aç` → `Aç`, `__` → `_`), GTK etiketi gibi.
pub fn strip_mnemonic(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// `wayland:<tanıtıcı>` → tanıtıcı (yalnızca `[A-Za-z0-9_-]`, ≤ [`MAX_HANDLE_BYTES`]); diğer her
/// şey (x11, boş, bozuk) boş dize.
pub fn wayland_handle(parent_window: &str) -> String {
    match parent_window.strip_prefix("wayland:") {
        Some(h) if !h.is_empty() && h.len() <= MAX_HANDLE_BYTES && h.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') => {
            h.to_owned()
        }
        _ => String::new(),
    }
}

/// Bayt dizisinden NUL sonu ve sonrası atılmış yol (`ay` seçenekleri NUL sonludur).
pub fn path_from_nul_bytes(bytes: &[u8]) -> Option<PathBuf> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let raw = &bytes[..end];
    (!raw.is_empty()).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(raw)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_codec_roundtrip_non_utf8() {
        let raw = b"/home/u/r\xe9sum\xe9 100%.txt".to_vec();
        let p = PathBuf::from(OsString::from_vec(raw.clone()));
        let enc = encode_path(&p);
        assert!(enc.is_ascii());
        assert_eq!(enc, "/home/u/r%E9sum%E9%20100%25.txt");
        assert_eq!(decode_path(&enc).unwrap(), p);
        assert_eq!(file_uri(Path::new("/a b/ç")), "file:///a%20b/%C3%A7");
    }

    #[test]
    fn decode_is_strict() {
        for bad in ["/a b", "%", "%4", "%zz", "/a%00b", "/a?b", "/a#b", "é"] {
            assert!(decode_bytes(bad).is_none(), "{bad}");
        }
        assert_eq!(decode_bytes("%2f%2F").unwrap(), b"//");
    }

    #[test]
    fn path_validation() {
        for ok in ["/", "/a", "/a/b.c", "/a/.gizli", "/a/..b"] {
            assert!(valid_path(Path::new(ok)), "{ok}");
        }
        for bad in ["", "a", "./a", "/a/../b", "/a/./b", "/a/.", "/a/..", "/a//./b"] {
            assert!(!valid_path(Path::new(bad)), "{bad}");
        }
        assert!(!valid_path(Path::new(&format!("/{}", "a".repeat(MAX_PATH_BYTES)))));
    }

    #[test]
    fn names() {
        assert!(valid_name(b"a.txt"));
        assert!(valid_name(b".gizli"));
        for bad in [&b""[..], b".", b"..", b"a/b", b"a\0", &[b'a'; 256]] {
            assert!(!valid_name(bad));
        }
    }

    #[test]
    fn text_cleaning() {
        assert_eq!(clean_text("a\u{202E}b\nc\u{200B}d\u{7}", 100), "abcd");
        assert_eq!(clean_text("çççç", 5), "çç");
        assert_eq!(strip_mnemonic("_Aç"), "Aç");
        assert_eq!(strip_mnemonic("a__b_c"), "a_bc");
    }

    #[test]
    fn handles() {
        assert_eq!(wayland_handle("wayland:abc-1_2"), "abc-1_2");
        for bad in ["", "x11:1a", "wayland:", "wayland:a b", "wayland:a;b", &format!("wayland:{}", "a".repeat(129))] {
            assert_eq!(wayland_handle(bad), "", "{bad}");
        }
    }

    #[test]
    fn nul_terminated_bytes() {
        assert_eq!(path_from_nul_bytes(b"/tmp\0junk").unwrap(), PathBuf::from("/tmp"));
        assert_eq!(path_from_nul_bytes(b"/tmp").unwrap(), PathBuf::from("/tmp"));
        assert!(path_from_nul_bytes(b"\0").is_none());
    }

    #[test]
    fn choice_values() {
        let toggle = Choice { id: "a".into(), label: "A".into(), options: vec![], initial: "yes".into() };
        assert!(toggle.accepts("true") && !toggle.accepts("yes"));
        assert_eq!(toggle.initial_value(), "false");
        let list = Choice {
            id: "enc".into(),
            label: "Kodlama".into(),
            options: vec![("utf8".into(), "UTF-8".into()), ("latin1".into(), "Latin-1".into())],
            initial: "latin1".into(),
        };
        assert_eq!(list.initial_value(), "latin1");
        assert!(!list.accepts("x"));
    }

    #[test]
    fn lines_roundtrip_and_limits() {
        let req = ChooserRequest {
            mode: Mode::Save,
            title: "Kaydet".into(),
            app_id: "org.example".into(),
            parent: String::new(),
            accept_label: None,
            modal: true,
            multiple: false,
            directory: false,
            filters: vec![Filter { name: "PNG".into(), rules: vec![Rule::Glob("*.png".into()), Rule::Mime("image/png".into())] }],
            current_filter: Some(5),
            choices: vec![],
            current_folder: Some(encode_path(Path::new("/tmp"))),
            current_name: Some("a.png".into()),
            files: vec![],
        };
        let line = req.to_line();
        assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'));
        let back = ChooserRequest::from_line(&line).unwrap();
        // Geçersiz filtre indisi atılır.
        assert_eq!(back.current_filter, None);
        assert_eq!(back.filters, req.filters);
        assert!(ChooserRequest::from_line("{").is_err());
        assert!(ChooserRequest::from_line(&" ".repeat(MAX_REQUEST_BYTES + 1)).is_err());

        let res = ChooserResult { paths: vec!["/a".into()], choices: vec![("x".into(), "true".into())], filter: Some(0) };
        assert_eq!(ChooserResult::from_bytes(res.to_line().as_bytes()).unwrap(), res);
        assert!(ChooserResult::from_bytes(b"{}\n{}\n").is_err());
        assert!(ChooserResult::from_bytes(b"\xff").is_err());
        // Debug kullanıcı içeriği göstermez.
        assert!(!format!("{req:?} {res:?}").contains("/tmp") && !format!("{res:?}").contains("/a"));

        // Düşmanca yol/ad alanları: satır tümden reddedilir.
        let base = ChooserRequest { mode: Mode::SaveFiles, current_filter: None, ..req };
        assert!(ChooserRequest::from_line(&base.to_line()).is_ok());
        for files in [vec!["..%2Fx"], vec!["a", "%2Fabs"], vec!["%2E%2E"], vec!["."], vec![""], vec!["a%00b"], vec!["../x"]] {
            let r = ChooserRequest { files: files.iter().map(|s| (*s).to_owned()).collect(), ..base.clone() };
            assert!(ChooserRequest::from_line(&r.to_line()).is_err(), "{files:?}");
        }
        for name in ["../x", "a/b", "..", "", &"a".repeat(256)] {
            let r = ChooserRequest { current_name: Some(name.to_owned()), ..base.clone() };
            assert!(ChooserRequest::from_line(&r.to_line()).is_err(), "{name}");
        }
        for folder in ["relative", "%2Fa%2F..%2Fb", "%2Fa%2F.", ""] {
            let r = ChooserRequest { current_folder: Some(folder.to_owned()), ..base.clone() };
            assert!(ChooserRequest::from_line(&r.to_line()).is_err(), "{folder}");
        }
    }
}
