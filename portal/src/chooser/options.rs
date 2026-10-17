//! D-Bus seçenekleri (`a{sv}`) → [`ChooserRequest`] ve seçici sonucu → yanıt `a{sv}`.
//!
//! Girdi uygulamadan gelir (güvenilmez): her metin temizlenir ve kırpılır, liste uzunlukları
//! sınırlanır, yanlış türdeki anahtar yok sayılır (GTK arka ucu gibi; istek düşmez). Sonuç
//! seçiciden gelir ama yine doğrulanır ([`finish`]).

use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use zbus::zvariant::{OwnedValue, Value};

use super::protocol::{
    self as p, ChooserRequest, ChooserResult, Choice, Filter, MAX_CHOICE_OPTIONS, MAX_CHOICES, MAX_FILES, MAX_FILTERS, MAX_ID_BYTES,
    MAX_LABEL_BYTES, MAX_NAME_BYTES, MAX_PATHS, MAX_PATTERN_BYTES, MAX_RULES, MAX_TITLE_BYTES, Mode, Rule,
};

pub type Options = HashMap<String, OwnedValue>;
pub type Results = HashMap<String, OwnedValue>;

/// D-Bus filtre biçimi `(sa(us))`.
pub type WireFilter = (String, Vec<(u32, String)>);
type WireChoice = (String, String, Vec<(String, String)>, String);

fn get<T>(options: &Options, key: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    let v = options.get(key)?.try_clone().ok()?;
    match T::try_from(v) {
        Ok(t) => Some(t),
        Err(_) => {
            tracing::warn!(key, "seçenek beklenmeyen türde; yok sayıldı");
            None
        }
    }
}

/// Bir filtreyi temizler; kuralı kalmayan filtre atılır.
fn filter_from_wire((name, rules): WireFilter) -> Option<Filter> {
    let rules: Vec<Rule> = rules
        .into_iter()
        .filter_map(|(kind, pattern)| {
            let pattern = p::clean_text(&pattern, MAX_PATTERN_BYTES);
            if pattern.is_empty() {
                return None;
            }
            match kind {
                0 => Some(Rule::Glob(pattern)),
                1 => Some(Rule::Mime(pattern.to_ascii_lowercase())),
                _ => None,
            }
        })
        .take(MAX_RULES)
        .collect();
    (!rules.is_empty()).then(|| Filter { name: p::clean_text(&name, MAX_LABEL_BYTES), rules })
}

pub fn filter_to_wire(f: &Filter) -> WireFilter {
    let rules = f
        .rules
        .iter()
        .map(|r| match r {
            Rule::Glob(s) => (0, s.clone()),
            Rule::Mime(s) => (1, s.clone()),
        })
        .collect();
    (f.name.clone(), rules)
}

fn choice_from_wire((id, label, options, initial): WireChoice) -> Option<Choice> {
    if id.is_empty() || id.len() > MAX_ID_BYTES {
        return None;
    }
    let options = options
        .into_iter()
        .filter(|(oid, _)| !oid.is_empty() && oid.len() <= MAX_ID_BYTES)
        .take(MAX_CHOICE_OPTIONS)
        .map(|(oid, olabel)| (oid, p::clean_text(&olabel, MAX_LABEL_BYTES)))
        .collect();
    Some(Choice { id, label: p::clean_text(&label, MAX_LABEL_BYTES), options, initial: initial.chars().take(MAX_ID_BYTES).collect() })
}

/// Önerilen ad: temizlenmiş, `/` yok, ≤ [`MAX_NAME_BYTES`]; geçersizse `None`.
fn clean_name(name: &str) -> Option<String> {
    let s = p::clean_text(name, MAX_NAME_BYTES).replace('/', "_");
    p::valid_name(s.as_bytes()).then_some(s)
}

/// `SaveFiles` `files` katı denetimi: her ad (NUL sonu atılır) boş değil, `/` ve NUL içermez, `.`/`..`
/// değil, ≤ 255 bayt; en çok [`MAX_FILES`]. Biri bile geçersizse istek TÜMDEN reddedilir (sessizce ad
/// atlamak uygulamaya eksik URI listesi verirdi; `../x`, `/abs` kum havuzu kaçışıdır). UTF-8 olmayan
/// ad geçerlidir (Linux dosya adı bayttır; yüzde kodlu taşınır). Diğer kiplerde her zaman `Ok`.
pub fn validate(mode: Mode, options: &Options) -> Result<(), &'static str> {
    if mode != Mode::SaveFiles {
        return Ok(());
    }
    let Some(v) = options.get("files") else { return Ok(()) };
    let files = Vec::<Vec<u8>>::try_from(v.try_clone().map_err(|_| "files okunamadı")?).map_err(|_| "files türü yanlış")?;
    if files.len() > MAX_FILES {
        return Err("çok fazla dosya adı");
    }
    for b in &files {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        if !p::valid_name(&b[..end]) {
            return Err("geçersiz dosya adı");
        }
    }
    Ok(())
}

/// İsteği kurar. `title` boşsa seçici kipine göre kendi başlığını koyar.
pub fn request(mode: Mode, app_id: &str, parent_window: &str, title: &str, options: &Options) -> ChooserRequest {
    let accept_label = get::<String>(options, "accept_label")
        .map(|s| p::clean_text(&p::strip_mnemonic(&s), MAX_LABEL_BYTES))
        .filter(|s| !s.is_empty());
    let modal = get::<bool>(options, "modal").unwrap_or(true);
    let (multiple, directory) = match mode {
        Mode::Open => (get::<bool>(options, "multiple").unwrap_or(false), get::<bool>(options, "directory").unwrap_or(false)),
        _ => (false, false),
    };

    let mut filters: Vec<Filter> = match mode {
        Mode::SaveFiles => Vec::new(),
        _ => get::<Vec<WireFilter>>(options, "filters").unwrap_or_default().into_iter().filter_map(filter_from_wire).take(MAX_FILTERS).collect(),
    };
    let mut current_filter = None;
    if mode != Mode::SaveFiles
        && let Some(cf) = get::<WireFilter>(options, "current_filter").and_then(filter_from_wire)
    {
        // Listede varsa o; yoksa (liste boşken de) eklenir — GTK arka ucu gibi.
        current_filter = match filters.iter().position(|f| *f == cf) {
            Some(i) => Some(i),
            None if filters.len() < MAX_FILTERS => {
                filters.push(cf);
                Some(filters.len() - 1)
            }
            None => None,
        };
    }

    let choices = get::<Vec<WireChoice>>(options, "choices").unwrap_or_default().into_iter().filter_map(choice_from_wire).take(MAX_CHOICES).collect();

    let mut current_folder = get::<Vec<u8>>(options, "current_folder").and_then(|b| p::path_from_nul_bytes(&b)).filter(|p| p::valid_path(p));
    let mut current_name = match mode {
        Mode::Save => get::<String>(options, "current_name").and_then(|s| clean_name(&s)),
        _ => None,
    };
    if mode == Mode::Save
        && let Some(file) = get::<Vec<u8>>(options, "current_file").and_then(|b| p::path_from_nul_bytes(&b)).filter(|p| p::valid_path(p))
    {
        // Var olan dosyayı kaydetmek: klasör onun dizini, ad onun adı.
        if let Some(parent) = file.parent() {
            current_folder = Some(parent.to_path_buf());
        }
        if let Some(name) = file.file_name() {
            current_name = clean_name(&name.to_string_lossy()).or(current_name);
        }
    }

    let files = match mode {
        Mode::SaveFiles => get::<Vec<Vec<u8>>>(options, "files")
            .unwrap_or_default()
            .into_iter()
            .take(MAX_FILES)
            .filter_map(|b| {
                let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                let name = &b[..end];
                p::valid_name(name).then(|| p::encode_bytes(name))
            })
            .collect(),
        _ => Vec::new(),
    };

    ChooserRequest {
        mode,
        title: p::clean_text(title, MAX_TITLE_BYTES),
        app_id: p::clean_text(app_id, MAX_TITLE_BYTES),
        parent: p::wayland_handle(parent_window),
        accept_label,
        modal,
        multiple,
        directory,
        filters,
        current_filter,
        choices,
        current_folder: current_folder.as_deref().map(p::encode_path),
        current_name,
        files,
    }
}

/// Dosya sistemi sorgusu (testte sahte).
pub trait Fs {
    /// `Some(true)` klasör, `Some(false)` klasör olmayan var olan girdi, `None` yok.
    fn kind(&self, path: &Path) -> Option<bool>;
}

/// Gerçek dosya sistemi (bağlar izlenir: uygulama bağın hedefini açar).
#[derive(Debug, Clone, Copy)]
pub struct RealFs;

impl Fs for RealFs {
    fn kind(&self, path: &Path) -> Option<bool> {
        std::fs::metadata(path).ok().map(|m| m.is_dir())
    }
}

/// Seçicinin sonucunu doğrular ve yanıt sözlüğünü kurar. `Err`: neden (yol içermez).
pub fn finish(req: &ChooserRequest, res: &ChooserResult, fs: &dyn Fs) -> Result<Results, &'static str> {
    if res.paths.is_empty() || res.paths.len() > MAX_PATHS {
        return Err("yol sayısı geçersiz");
    }
    let paths: Vec<PathBuf> = res.paths.iter().map(|s| p::decode_path(s).filter(|p| p::valid_path(p)).ok_or("geçersiz yol")).collect::<Result<_, _>>()?;
    match req.mode {
        Mode::Open => {
            if !req.multiple && paths.len() != 1 {
                return Err("tekli seçimde birden çok yol");
            }
            for path in &paths {
                match fs.kind(path) {
                    Some(is_dir) if is_dir == req.directory => {}
                    Some(_) => return Err("seçilen öğenin türü yanlış"),
                    None => return Err("seçilen öğe yok"),
                }
            }
        }
        Mode::Save => {
            if paths.len() != 1 {
                return Err("kaydetmede tam bir yol gerekir");
            }
            let path = &paths[0];
            if fs.kind(path) == Some(true) {
                return Err("kaydetme hedefi klasör");
            }
            if path.parent().is_none_or(|d| fs.kind(d) != Some(true)) {
                return Err("kaydetme klasörü yok");
            }
            if !path.file_name().is_some_and(|n| p::valid_name(n.as_bytes())) {
                return Err("geçersiz ad");
            }
        }
        Mode::SaveFiles => {
            if paths.len() != req.files.len() {
                return Err("dosya sayısı istekle uyuşmuyor");
            }
            // Hepsi aynı (seçilen) klasörde, adları geçerli: seçici başka yere yazdıramaz.
            let dir = paths[0].parent();
            if paths.iter().any(|p| p.parent() != dir || !p.file_name().is_some_and(|n| p::valid_name(n.as_bytes()))) {
                return Err("kaydetme hedefleri tek klasörde değil");
            }
            for path in &paths {
                if path.parent().is_none_or(|d| fs.kind(d) != Some(true)) || fs.kind(path) == Some(true) {
                    return Err("kaydetme hedefi geçersiz");
                }
            }
        }
    }

    let mut out = Results::new();
    let uris: Vec<String> = paths.iter().map(|p| p::file_uri(p)).collect();
    out.insert("uris".to_owned(), owned(Value::from(uris))?);

    let mut choices: Vec<(String, String)> = Vec::new();
    for (id, value) in &res.choices {
        let Some(c) = req.choices.iter().find(|c| &c.id == id) else { return Err("bilinmeyen seçim") };
        if !c.accepts(value) {
            return Err("geçersiz seçim değeri");
        }
        if !choices.iter().any(|(i, _)| i == id) {
            choices.push((id.clone(), value.clone()));
        }
    }
    if !req.choices.is_empty() {
        // Seçici bir seçimi yazmadıysa başlangıç değeri döner (uygulama her kimliği bekler).
        for c in &req.choices {
            if !choices.iter().any(|(i, _)| *i == c.id) {
                choices.push((c.id.clone(), c.initial_value()));
            }
        }
        out.insert("choices".to_owned(), owned(Value::from(choices))?);
    }

    if req.mode != Mode::SaveFiles
        && let Some(i) = res.filter
    {
        let f = req.filters.get(i).ok_or("filtre indisi geçersiz")?;
        out.insert("current_filter".to_owned(), owned(Value::from(filter_to_wire(f)))?);
    }
    Ok(out)
}

fn owned(v: Value<'_>) -> Result<OwnedValue, &'static str> {
    OwnedValue::try_from(v).map_err(|_| "değer kurulamadı")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn opt(pairs: Vec<(&str, Value<'_>)>) -> Options {
        pairs.into_iter().map(|(k, v)| (k.to_owned(), OwnedValue::try_from(v).unwrap())).collect()
    }

    fn png() -> WireFilter {
        ("Resimler".into(), vec![(0, "*.png".into()), (1, "Image/PNG".into())])
    }

    #[test]
    fn open_options() {
        let o = opt(vec![
            ("accept_label", Value::from("_Aç")),
            ("multiple", Value::from(true)),
            ("directory", Value::from(false)),
            ("modal", Value::from(false)),
            ("filters", Value::from(vec![png(), ("Boş".into(), vec![(7, "x".to_owned())])])),
            ("current_filter", Value::from(png())),
            ("current_folder", Value::from(b"/tmp\0".to_vec())),
            ("choices", Value::from(vec![("ro".to_owned(), "Salt okunur".to_owned(), Vec::<(String, String)>::new(), "true".to_owned())])),
        ]);
        let r = request(Mode::Open, "org.x", "wayland:h1", "Aç\u{202E}", &o);
        assert_eq!(r.accept_label.as_deref(), Some("Aç"));
        assert!(r.multiple && !r.directory && !r.modal);
        // Bilinmeyen kural türü olan filtre düşer; MIME küçük harfe iner.
        assert_eq!(r.filters.len(), 1);
        assert_eq!(r.filters[0].rules[1], Rule::Mime("image/png".into()));
        assert_eq!(r.current_filter, Some(0));
        assert_eq!(r.current_folder.as_deref(), Some("/tmp"));
        assert_eq!(r.parent, "h1");
        assert_eq!(r.title, "Aç");
        assert_eq!(r.choices.len(), 1);
    }

    #[test]
    fn current_filter_appended_and_wrong_types_ignored() {
        let o = opt(vec![("current_filter", Value::from(png())), ("multiple", Value::from("evet")), ("current_folder", Value::from("relative"))]);
        let r = request(Mode::Open, "", "x11:1f", "", &o);
        assert_eq!(r.filters.len(), 1);
        assert_eq!(r.current_filter, Some(0));
        assert!(!r.multiple);
        assert!(r.current_folder.is_none());
        assert_eq!(r.parent, "");
    }

    #[test]
    fn save_current_file_overrides_name() {
        let o = opt(vec![
            ("current_name", Value::from("a/b.txt")),
            ("current_file", Value::from(b"/home/u/belge.odt\0".to_vec())),
            ("multiple", Value::from(true)),
        ]);
        let r = request(Mode::Save, "", "", "", &o);
        assert_eq!(r.current_folder.as_deref(), Some("/home/u"));
        assert_eq!(r.current_name.as_deref(), Some("belge.odt"));
        assert!(!r.multiple);
        let r = request(Mode::Save, "", "", "", &opt(vec![("current_name", Value::from("a/b.txt"))]));
        assert_eq!(r.current_name.as_deref(), Some("a_b.txt"));
        let r = request(Mode::Save, "", "", "", &opt(vec![("current_name", Value::from(".."))]));
        assert!(r.current_name.is_none());
    }

    #[test]
    fn save_files_names() {
        let o = opt(vec![
            ("files", Value::from(vec![b"a.txt\0".to_vec(), b"../x\0".to_vec(), b"r\xe9.txt\0".to_vec(), b"\0".to_vec()])),
            ("filters", Value::from(vec![png()])),
        ]);
        let r = request(Mode::SaveFiles, "", "", "", &o);
        assert_eq!(r.files, vec!["a.txt".to_owned(), "r%E9.txt".to_owned()]);
        assert!(r.filters.is_empty());
    }

    struct FakeFs {
        dirs: HashSet<&'static str>,
        files: HashSet<&'static str>,
    }

    impl Fs for FakeFs {
        fn kind(&self, path: &Path) -> Option<bool> {
            let s = path.to_str()?;
            if self.dirs.contains(s) {
                Some(true)
            } else if self.files.contains(s) {
                Some(false)
            } else {
                None
            }
        }
    }

    fn fs() -> FakeFs {
        FakeFs { dirs: ["/d", "/d/sub"].into(), files: ["/d/a.png", "/d/b.png"].into() }
    }

    fn uris(r: &Results) -> Vec<String> {
        Vec::<String>::try_from(r["uris"].try_clone().unwrap()).unwrap()
    }

    fn res(paths: &[&str]) -> ChooserResult {
        ChooserResult { paths: paths.iter().map(|s| (*s).to_owned()).collect(), ..Default::default() }
    }

    #[test]
    fn finish_open() {
        let o = opt(vec![
            ("filters", Value::from(vec![png()])),
            ("choices", Value::from(vec![("ro".to_owned(), "RO".to_owned(), Vec::<(String, String)>::new(), "true".to_owned())])),
        ]);
        let req = request(Mode::Open, "", "", "", &o);
        let mut ok = res(&["/d/a.png"]);
        ok.filter = Some(0);
        let r = finish(&req, &ok, &fs()).unwrap();
        assert_eq!(uris(&r), vec!["file:///d/a.png".to_owned()]);
        // Seçici yazmadıysa seçim başlangıç değeriyle döner.
        let ch = Vec::<(String, String)>::try_from(r["choices"].try_clone().unwrap()).unwrap();
        assert_eq!(ch, vec![("ro".to_owned(), "true".to_owned())]);
        let f = WireFilter::try_from(r["current_filter"].try_clone().unwrap()).unwrap();
        assert_eq!(f.0, "Resimler");

        for bad in [res(&[]), res(&["/d/a.png", "/d/b.png"]), res(&["/d/yok"]), res(&["/d/sub"]), res(&["d/a.png"]), res(&["/d/../d/a.png"])] {
            assert!(finish(&req, &bad, &fs()).is_err(), "{bad:?}");
        }
        let mut bad = res(&["/d/a.png"]);
        bad.filter = Some(3);
        assert!(finish(&req, &bad, &fs()).is_err());
        let mut bad = res(&["/d/a.png"]);
        bad.choices = vec![("ro".into(), "belki".into())];
        assert!(finish(&req, &bad, &fs()).is_err());
        bad.choices = vec![("baska".into(), "true".into())];
        assert!(finish(&req, &bad, &fs()).is_err());
    }

    #[test]
    fn finish_open_directory_multiple() {
        let req = request(Mode::Open, "", "", "", &opt(vec![("directory", Value::from(true)), ("multiple", Value::from(true))]));
        assert!(finish(&req, &res(&["/d", "/d/sub"]), &fs()).is_ok());
        assert!(finish(&req, &res(&["/d/a.png"]), &fs()).is_err());
        assert!(!finish(&req, &res(&["/d"]), &fs()).unwrap().contains_key("choices"));
    }

    #[test]
    fn finish_save() {
        let req = request(Mode::Save, "", "", "", &Options::new());
        assert!(finish(&req, &res(&["/d/yeni.txt"]), &fs()).is_ok());
        // Üzerine yazma (onay seçicide) geçerli.
        assert!(finish(&req, &res(&["/d/a.png"]), &fs()).is_ok());
        for bad in [res(&["/d/sub"]), res(&["/yok/a.txt"]), res(&["/d/x", "/d/y"])] {
            assert!(finish(&req, &bad, &fs()).is_err());
        }
        let req = request(Mode::SaveFiles, "", "", "", &opt(vec![("files", Value::from(vec![b"a\0".to_vec(), b"b\0".to_vec()]))]));
        assert!(finish(&req, &res(&["/d/a", "/d/b"]), &fs()).is_ok());
        assert!(finish(&req, &res(&["/d/a"]), &fs()).is_err());
        assert!(finish(&req, &res(&["/d/a", "/d/sub"]), &fs()).is_err());
        // Hedefler tek klasörde olmalı; `..`, göreli yol ve alt klasöre kaçış reddedilir.
        let fs2 = FakeFs { dirs: ["/d", "/d/sub", "/e"].into(), files: HashSet::new() };
        for bad in [res(&["/d/a", "/d/sub/b"]), res(&["/d/a", "/e/b"]), res(&["/d/a", "/d/../e/b"]), res(&["/d/a", "d/b"]), res(&["/d/a", "/d/."])] {
            assert!(finish(&req, &bad, &fs2).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn save_files_validate_rejects_whole_request() {
        let files = |names: &[&[u8]]| opt(vec![("files", Value::from(names.iter().map(|n| n.to_vec()).collect::<Vec<_>>()))]);
        assert!(validate(Mode::SaveFiles, &files(&[b"a.txt\0", b"r\xe9.txt\0", b"b"])).is_ok());
        assert!(validate(Mode::SaveFiles, &Options::new()).is_ok());
        let long = [vec![b'a'; 256], vec![0]].concat();
        for bad in [&b"../x\0"[..], b"/abs\0", b"a/b\0", b".\0", b"..\0", b"\0", b"", &long] {
            assert!(validate(Mode::SaveFiles, &files(&[b"iyi\0", bad])).is_err(), "{bad:?}");
        }
        // NUL'dan sonrası atılır: `a\0/../x` → `a` (geçerli, kaçış yok).
        assert!(validate(Mode::SaveFiles, &files(&[b"a\0/../x"])).is_ok());
        let many: Vec<&[u8]> = vec![b"a"; MAX_FILES + 1];
        assert!(validate(Mode::SaveFiles, &files(&many)).is_err());
        // Yanlış tür (dizgi) de reddedilir; diğer kiplerde `files` yok sayılır.
        let wrong = opt(vec![("files", Value::from("../x"))]);
        assert!(validate(Mode::SaveFiles, &wrong).is_err());
        assert!(validate(Mode::Save, &files(&[b"../x"])).is_ok());
    }
}
