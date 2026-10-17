//! Uygulama kataloğu: XDG veri dizinlerindeki `applications/` ağaçlarını tarar, masaüstü dosya
//! kimliklerine göre gölgeleme yapar ve görünür uygulamaları arama için hazırlar.
//!
//! Kurallar (Desktop Entry Spec + Base Directory Spec):
//! - Dizin önceliği: `$XDG_DATA_HOME` (varsayılan `~/.local/share`), sonra `$XDG_DATA_DIRS`
//!   (varsayılan `/usr/local/share:/usr/share`) sırasıyla. Aynı kimliğin ilk bulunanı geçerlidir;
//!   `Hidden=true` olan daha öncelikli dosya alttakini **siler**, ayrıştırılamayan da gölgeler.
//! - Kimlik: `applications/` altındaki göreli yol, `/` → `-` (`kde4/a.desktop` → `kde4-a.desktop`).
//!   Aynı dizinde aynı kimliğe çıkan iki dosyada sıralı ilk.
//! - Görünürlük: `Type=Application`; `NoDisplay`, `Hidden` değil; `OnlyShowIn`/`NotShowIn`
//!   `$XDG_CURRENT_DESKTOP` ile; `TryExec` mutlak ya da `$PATH`'te çalıştırılabilir olmalı;
//!   `Exec` yoksa yalnızca `DBusActivatable=true` ile.
//! - Sınırlar: dizin derinliği [`MAX_DEPTH`], dosya sayısı [`MAX_FILES`].
//! - Önbellek: dosya (cihaz, inode, boyut, mtime) değişmediyse yeniden ayrıştırılmaz; canlı
//!   güncellemede yalnızca değişen dosyalar okunur.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nucleo_matcher::Utf32String;

use super::desktop::{self, DesktopEntry, EntryType, Locale};

/// `applications/` altında en çok kaç dizin derinliğine inilir.
pub const MAX_DEPTH: usize = 6;
/// Tüm dizinlerde en çok kaç `.desktop` dosyası okunur.
pub const MAX_FILES: usize = 20_000;

/// Taramanın girdileri (ortamdan ya da testte sahte dizinlerden).
#[derive(Debug, Clone, Default)]
pub struct ScanConfig {
    /// Öncelik sırasıyla veri dizinleri (her birinin `applications/` alt dizini taranır).
    pub data_dirs: Vec<PathBuf>,
    /// `$XDG_CURRENT_DESKTOP` öğeleri (`Hyprland`).
    pub desktops: Vec<String>,
    pub locale: Locale,
    /// `TryExec` araması için `$PATH`.
    pub path: Vec<PathBuf>,
}

impl ScanConfig {
    /// Ortamdan (XDG değişkenleri, `$PATH`, yerel ayar).
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.map(|h| h.join(".local/share")));
        let dirs = std::env::var_os("XDG_DATA_DIRS")
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| OsString::from("/usr/local/share:/usr/share"));
        let mut data_dirs: Vec<PathBuf> = data_home.into_iter().collect();
        for d in std::env::split_paths(&dirs) {
            if d.is_absolute() && !data_dirs.contains(&d) {
                data_dirs.push(d);
            }
        }
        let desktops = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        let path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        Self { data_dirs, desktops, locale: Locale::from_env(), path }
    }

    /// İzlenecek `applications/` dizinleri.
    pub fn application_dirs(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.data_dirs.iter().map(|d| d.join("applications"))
    }
}

/// Aramaya hazır uygulama.
#[derive(Debug)]
pub struct App {
    pub entry: Arc<DesktopEntry>,
    /// Arama samanlıkları ([`Field`] sırasıyla).
    pub(super) haystacks: [Utf32String; 4],
    /// Ad küçük harfli (boş sorguda alfabetik sıra için).
    pub(super) sort_key: String,
}

/// Arama alanları ve ağırlıkları (yüzde).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name = 0,
    GenericName = 1,
    Keywords = 2,
    /// Program adı ve kimlik (`nautilus`, `org.gnome.Nautilus`).
    Aux = 3,
}

impl App {
    fn new(entry: Arc<DesktopEntry>) -> Self {
        let keywords = entry.keywords.join(" ");
        let mut aux = entry.id.strip_suffix(".desktop").unwrap_or(&entry.id).to_owned();
        if let Some(program) = entry.exec.as_deref().and_then(program_name)
            && !aux.to_lowercase().contains(&program.to_lowercase())
        {
            aux.push(' ');
            aux.push_str(&program);
        }
        let haystacks = [
            Utf32String::from(entry.name.as_str()),
            Utf32String::from(entry.generic_name.as_deref().unwrap_or("")),
            Utf32String::from(keywords),
            Utf32String::from(aux),
        ];
        let sort_key = entry.name.to_lowercase();
        Self { entry, haystacks, sort_key }
    }
}

/// `Exec`'in program adı (argv[0]'ın son bileşeni); `env A=B prog` ve `flatpak run x` gibi
/// sarmalayıcılar ayıklanmaz — kimlik zaten aranır.
fn program_name(exec: &str) -> Option<String> {
    let argv = super::exec::expand(exec, &super::exec::FieldValues::default()).ok()?;
    let name = argv.first()?.rsplit('/').next()?;
    (!name.is_empty() && name.len() <= 64).then(|| name.to_owned())
}

/// Görünür uygulamaların değişmez listesi; her taramada yeni bir `Arc<Catalog>`.
#[derive(Debug, Default)]
pub struct Catalog {
    pub apps: Vec<App>,
    /// Artan sürüm (UI eski aramayı tanır).
    pub generation: u64,
}

impl Catalog {
    pub fn len(&self) -> usize {
        self.apps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&DesktopEntry> {
        self.apps.get(index).map(|a| &*a.entry)
    }

    /// Kimliğe göre.
    pub fn find(&self, id: &str) -> Option<(usize, &DesktopEntry)> {
        self.apps.iter().position(|a| a.entry.id == id).map(|i| (i, &*self.apps[i].entry))
    }

    /// Sahte girdilerden (test ve kıyaslama).
    pub fn from_entries(entries: impl IntoIterator<Item = DesktopEntry>, generation: u64) -> Self {
        Self::from_shared(entries.into_iter().map(Arc::new).collect(), generation)
    }

    fn from_shared(entries: Vec<Arc<DesktopEntry>>, generation: u64) -> Self {
        let mut apps: Vec<App> = entries.into_iter().map(App::new).collect();
        apps.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then_with(|| a.entry.id.cmp(&b.entry.id)));
        Self { apps, generation }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: i64,
    mtime_nsec: i64,
}

impl Stamp {
    fn of(meta: &std::fs::Metadata) -> Self {
        Self { dev: meta.dev(), ino: meta.ino(), size: meta.size(), mtime: meta.mtime(), mtime_nsec: meta.mtime_nsec() }
    }
}

/// Taramalar arası ayrıştırma önbelleği.
#[derive(Debug, Default)]
pub struct Scanner {
    cache: HashMap<PathBuf, (Stamp, Option<Arc<DesktopEntry>>)>,
    generation: u64,
    /// Son taramada kaç dosya yeniden ayrıştırıldı (test/ölçüm).
    pub last_parsed: usize,
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tüm dizinleri tarar. Bloklayan IO: yalnızca arka plan thread'inde.
    pub fn scan(&mut self, config: &ScanConfig) -> Catalog {
        let index = self.scan_index(config);
        let mut exec_cache: HashMap<String, bool> = HashMap::new();
        let visible = index.into_iter().filter(|e| is_visible(e, config, &mut exec_cache)).collect();
        self.generation += 1;
        Catalog::from_shared(visible, self.generation)
    }

    /// Kimlik başına geçerli (ilk bulunan) ve ayrıştırılabilen bütün girdiler, dizin önceliği
    /// sırasıyla — görünürlük süzgeci yok (`Hidden`, `NoDisplay`, `TryExec` çağıranın işi). Daha
    /// öncelikli dizindeki ayrıştırılamayan dosya da alttakini gölgeler. Varsayılan uygulamalar
    /// (`services::default_apps`) aday ve "kurulu mu" için kullanır. Bloklayan IO.
    pub fn scan_index(&mut self, config: &ScanConfig) -> Vec<Arc<DesktopEntry>> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut index: Vec<Arc<DesktopEntry>> = Vec::new();
        let mut next_cache = HashMap::with_capacity(self.cache.len());
        let mut files = 0usize;
        self.last_parsed = 0;

        for apps_dir in config.application_dirs() {
            let mut found: Vec<(String, PathBuf)> = Vec::new();
            collect(&apps_dir, &apps_dir, 0, &mut found, &mut files);
            found.sort();
            for (id, path) in found {
                if !seen.insert(id.clone()) {
                    continue; // daha öncelikli dizinde (ya da aynı dizinde önce) var
                }
                let Ok(meta) = std::fs::metadata(&path) else { continue };
                let stamp = Stamp::of(&meta);
                let entry = match self.cache.remove(&path) {
                    Some((old, entry)) if old == stamp && entry.as_ref().is_none_or(|e| e.id == id) => entry,
                    _ => {
                        self.last_parsed += 1;
                        match desktop::read_entry(&id, &path, &config.locale) {
                            Ok(Ok(e)) => Some(Arc::new(e)),
                            Ok(Err(err)) => {
                                tracing::debug!(error = %err, "masaüstü dosyası geçersiz");
                                None
                            }
                            Err(err) => {
                                tracing::debug!(kind = ?err.kind(), "masaüstü dosyası okunamadı");
                                None
                            }
                        }
                    }
                };
                if let Some(e) = &entry {
                    index.push(e.clone());
                }
                next_cache.insert(path, (stamp, entry));
            }
        }
        if files >= MAX_FILES {
            tracing::warn!(limit = MAX_FILES, "masaüstü dosyası sınırına ulaşıldı; kalanlar yok sayıldı");
        }
        self.cache = next_cache;
        index
    }
}

fn collect(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>, files: &mut usize) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for item in read.flatten() {
        if *files >= MAX_FILES {
            return;
        }
        let path = item.path();
        let Ok(ft) = item.file_type() else { continue };
        // Symlink'ler izlenir (flatpak/nix dışa aktarımları); dizin symlink'i döngüsü derinlikle sınırlı.
        let is_dir = ft.is_dir() || (ft.is_symlink() && path.is_dir());
        if is_dir {
            if depth < MAX_DEPTH {
                collect(root, &path, depth + 1, out, files);
            }
            continue;
        }
        if path.extension().is_none_or(|e| e != "desktop") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else { continue };
        let Some(rel) = rel.to_str() else { continue }; // UTF-8 olmayan ad: kimlik olamaz
        *files += 1;
        out.push((rel.replace('/', "-"), path));
    }
}

fn is_visible(e: &DesktopEntry, config: &ScanConfig, exec_cache: &mut HashMap<String, bool>) -> bool {
    if e.kind != EntryType::Application || e.hidden || e.no_display {
        return false;
    }
    if e.exec.is_none() && !e.dbus_activatable {
        return false;
    }
    let in_desktop = |list: &[String]| list.iter().any(|d| config.desktops.iter().any(|c| c.eq_ignore_ascii_case(d)));
    if !e.only_show_in.is_empty() && !in_desktop(&e.only_show_in) {
        return false;
    }
    if in_desktop(&e.not_show_in) {
        return false;
    }
    if let Some(try_exec) = &e.try_exec {
        let found =
            *exec_cache.entry(try_exec.clone()).or_insert_with(|| find_executable(try_exec, &config.path).is_some());
        if !found {
            return false;
        }
    }
    true
}

/// Mutlak yol ya da `$PATH`'te çalıştırılabilir dosya.
pub fn find_executable(program: &str, path: &[PathBuf]) -> Option<PathBuf> {
    let executable = |p: &Path| {
        p.is_file() && rustix::fs::access(p, rustix::fs::Access::EXEC_OK).is_ok()
    };
    if program.contains('/') {
        let p = Path::new(program);
        return (p.is_absolute() && executable(p)).then(|| p.to_path_buf());
    }
    if program.is_empty() {
        return None;
    }
    path.iter().filter(|d| d.is_absolute()).map(|d| d.join(program)).find(|p| executable(p))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        p
    }

    fn app(name: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec={}\n{extra}", name.to_lowercase())
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        sys: PathBuf,
        bin: PathBuf,
        config: ScanConfig,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home/share");
        let sys = tmp.path().join("usr/share");
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let tool = bin.join("present-tool");
        std::fs::write(&tool, "").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let noexec = bin.join("noexec-tool");
        std::fs::write(&noexec, "").unwrap();
        let config = ScanConfig {
            data_dirs: vec![home.clone(), sys.clone()],
            desktops: vec!["Hyprland".into()],
            locale: Locale::parse("tr_TR.UTF-8"),
            path: vec![bin.clone()],
        };
        Fixture { _tmp: tmp, home, sys, bin, config }
    }

    fn names(c: &Catalog) -> Vec<&str> {
        c.apps.iter().map(|a| a.entry.name.as_str()).collect()
    }

    #[test]
    fn shadowing_visibility_and_ids() {
        let f = fixture();
        write(&f.sys, "applications/firefox.desktop", &app("Firefox", ""));
        write(&f.home, "applications/firefox.desktop", &app("Firefox Kullanıcı", ""));
        write(&f.sys, "applications/gizli.desktop", &app("Gizli", ""));
        write(&f.home, "applications/gizli.desktop", &app("Gizli", "Hidden=true"));
        write(&f.sys, "applications/nodisplay.desktop", &app("NoDisplay", "NoDisplay=true"));
        write(&f.sys, "applications/kde4/kate.desktop", &app("Kate", ""));
        write(&f.sys, "applications/only.desktop", &app("OnlyGnome", "OnlyShowIn=GNOME;"));
        write(&f.sys, "applications/onlyh.desktop", &app("OnlyHypr", "OnlyShowIn=GNOME;hyprland;"));
        write(&f.sys, "applications/not.desktop", &app("NotHypr", "NotShowIn=Hyprland;"));
        write(&f.sys, "applications/try1.desktop", &app("TryOk", "TryExec=present-tool"));
        write(&f.sys, "applications/try2.desktop", &app("TryMissing", "TryExec=missing-tool"));
        write(&f.sys, "applications/try3.desktop", &app("TryNoExec", "TryExec=noexec-tool"));
        let abs = f.bin.join("present-tool");
        write(&f.sys, "applications/try4.desktop", &app("TryAbs", &format!("TryExec={}", abs.display())));
        write(&f.sys, "applications/link.desktop", "[Desktop Entry]\nType=Link\nName=Link\nURL=x");
        write(&f.sys, "applications/noexec.desktop", "[Desktop Entry]\nType=Application\nName=NoExec");
        write(&f.sys, "applications/dbus.desktop", "[Desktop Entry]\nType=Application\nName=Dbus\nDBusActivatable=true");
        write(&f.sys, "applications/broken.desktop", "garbage");
        write(&f.sys, "applications/readme.txt", &app("Txt", ""));

        let mut scanner = Scanner::new();
        let c = scanner.scan(&f.config);
        assert_eq!(names(&c), ["Dbus", "Firefox Kullanıcı", "Kate", "OnlyHypr", "TryAbs", "TryOk"]);
        assert_eq!(c.find("kde4-kate.desktop").unwrap().1.name, "Kate");
        assert!(c.find("gizli.desktop").is_none(), "Hidden alttakini siler");
        assert_eq!(c.generation, 1);
    }

    #[test]
    fn rescan_reuses_unchanged_files() {
        let f = fixture();
        write(&f.sys, "applications/a.desktop", &app("A", ""));
        let b = write(&f.sys, "applications/b.desktop", &app("B", ""));
        let mut scanner = Scanner::new();
        let first = scanner.scan(&f.config);
        assert_eq!(scanner.last_parsed, 2);
        let again = scanner.scan(&f.config);
        assert_eq!(scanner.last_parsed, 0, "değişmeyen dosya yeniden okunmaz");
        assert_eq!(again.generation, first.generation + 1);
        std::fs::write(&b, app("B2", "")).unwrap();
        let changed = scanner.scan(&f.config);
        assert_eq!(scanner.last_parsed, 1);
        assert_eq!(names(&changed), ["A", "B2"]);
        std::fs::remove_file(&b).unwrap();
        assert_eq!(names(&scanner.scan(&f.config)), ["A"]);
        // Kullanıcı dizinine aynı kimlik eklenince gölgeler.
        write(&f.home, "applications/a.desktop", &app("A Kullanıcı", ""));
        assert_eq!(names(&scanner.scan(&f.config)), ["A Kullanıcı"]);
    }

    #[test]
    fn haystacks_include_program_and_id() {
        let e = desktop::parse(
            "org.gnome.Nautilus.desktop",
            Path::new("/x"),
            "[Desktop Entry]\nType=Application\nName=Dosyalar\nExec=/usr/bin/nautilus --new-window\nKeywords=klasör;dosya;",
            &Locale::default(),
        )
        .unwrap();
        let c = Catalog::from_entries([e], 1);
        assert_eq!(c.apps[0].haystacks[Field::Aux as usize].to_string(), "org.gnome.Nautilus");
        assert_eq!(c.apps[0].haystacks[Field::Keywords as usize].to_string(), "klasör dosya");
        assert_eq!(program_name("\"/opt/My App/x\" --y").as_deref(), Some("x"));
        assert_eq!(program_name("env FOO=1 code").as_deref(), Some("env"));
    }
}
