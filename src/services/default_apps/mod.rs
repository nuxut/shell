//! Varsayılan uygulamalar (sözleşme defaults-v1; docs/design/defaults.md): `msg defaults
//! status|set`.
//!
//! UI'dan bağımsız, saf ve test edilebilir:
//! - Aday ve "kurulu mu": launcher'ın XDG masaüstü indeksi ([`Scanner::scan_index`]; kimlik
//!   başına ilk bulunan girdi, `MimeType`/`Categories` aynı ayrıştırıcıdan).
//! - Mevcut seçim: [`mimeapps`] zinciri + `mimeinfo.cache` yedeği; terminal: [`terminal`] listeleri.
//! - Yazım: yalnızca `$XDG_CONFIG_HOME/mimeapps.list` `[Default Applications]` (`nuxut-apply`
//!   `IniKey` parçaları, diğer her şey korunur) ve terminal için `xdg-terminals.list` (`Line`).
//!   Snapshot ayar kimliği `defaults.<rol>` ([`crate::services::apply::DEFAULTS_SETTINGS`]).
//!
//! Bloklayan IO (dizin taraması, dosya okuma, snapshot kilidi): yalnızca [`Worker`]'ın thread'inde.
//! Worker taramalar arası ayrıştırma önbelleğini tutar (değişmeyen dosya yeniden okunmaz); ilk
//! tarama dışında `status` yalnızca `stat` + birkaç küçük dosya okur.

pub mod mimeapps;
pub mod terminal;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nuxut::i18n::t;
use nuxut_apply::{Applier, Change, Env, FragValue, Fragment, Outcome, Position};
use serde::Serialize;

use crate::services::launcher::catalog::find_executable;
use crate::services::launcher::{DesktopEntry, ScanConfig, Scanner};
use mimeapps::{ListFile, MimeCache, Source};

/// Bir rol: birincil MIME (mevcut seçim buradan) ve yazılan küme (uygulamanın bildirdikleriyle
/// kesişim). Terminalde ikisi de yok.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Role {
    pub name: &'static str,
    pub primary: Option<&'static str>,
    pub mimes: &'static [&'static str],
}

/// Terminal rolünün adı.
pub const TERMINAL: &str = "terminal";

/// Roller, sözleşme tablosu sırasıyla (status hepsini bu sırayla döndürür).
pub const ROLES: [Role; 9] = [
    Role {
        name: "browser",
        primary: Some("x-scheme-handler/https"),
        mimes: &["x-scheme-handler/http", "x-scheme-handler/https", "text/html", "application/xhtml+xml"],
    },
    Role { name: "email", primary: Some("x-scheme-handler/mailto"), mimes: &["x-scheme-handler/mailto"] },
    Role { name: "file_manager", primary: Some("inode/directory"), mimes: &["inode/directory"] },
    Role { name: TERMINAL, primary: None, mimes: &[] },
    Role { name: "text_editor", primary: Some("text/plain"), mimes: &["text/plain"] },
    Role {
        name: "image",
        primary: Some("image/png"),
        mimes: &[
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/bmp",
            "image/tiff",
            "image/svg+xml",
            "image/avif",
            "image/jxl",
        ],
    },
    Role {
        name: "video",
        primary: Some("video/mp4"),
        mimes: &[
            "video/mp4",
            "video/webm",
            "video/x-matroska",
            "video/quicktime",
            "video/mpeg",
            "video/x-msvideo",
            "video/ogg",
        ],
    },
    Role {
        name: "music",
        primary: Some("audio/mpeg"),
        mimes: &[
            "audio/mpeg",
            "audio/flac",
            "audio/ogg",
            "audio/x-vorbis+ogg",
            "audio/opus",
            "audio/wav",
            "audio/x-wav",
            "audio/mp4",
            "audio/aac",
        ],
    },
    Role { name: "pdf", primary: Some("application/pdf"), mimes: &["application/pdf"] },
];

pub fn role(name: &str) -> Option<&'static Role> {
    ROLES.iter().find(|r| r.name == name)
}

/// `mimeapps.list` bölümü.
pub const DEFAULTS_SECTION: &str = "Default Applications";

/// Okuma dizinleri (ortamdan ya da testte sahte köklerden).
#[derive(Debug, Clone, Default)]
pub struct Dirs {
    /// Veri dizinleri (ilki `$XDG_DATA_HOME`), masaüstleri, yerel ayar, `$PATH`.
    pub scan: ScanConfig,
    /// Yazım hedefinin dizini (`nuxut_apply::Env::config_home` ile aynı).
    pub config_home: Option<PathBuf>,
    pub config_dirs: Vec<PathBuf>,
    pub data_home: Option<PathBuf>,
    /// `$XDG_DATA_DIRS` (veri ev dizini hariç).
    pub data_dirs: Vec<PathBuf>,
}

impl Dirs {
    /// Süreç ortamından; yazım dizini `env`'inkiyle aynı.
    pub fn from_env(env: &Env) -> Self {
        let scan = ScanConfig::from_env();
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| env.home.as_ref().map(|h| h.join(".local/share")));
        let config_dirs = std::env::var_os("XDG_CONFIG_DIRS")
            .filter(|v| !v.is_empty())
            .map(|v| std::env::split_paths(&v).filter(|p| p.is_absolute()).collect())
            .unwrap_or_else(|| vec![PathBuf::from("/etc/xdg")]);
        Self::new(scan, env.config_home.clone(), config_dirs, data_home)
    }

    /// `scan.data_dirs` veri ev dizini + sistem veri dizinleridir; burada ikisi ayrılır.
    pub fn new(scan: ScanConfig, config_home: Option<PathBuf>, config_dirs: Vec<PathBuf>, data_home: Option<PathBuf>) -> Self {
        let data_dirs = scan.data_dirs.iter().filter(|d| Some(*d) != data_home.as_ref()).cloned().collect();
        Self { scan, config_home, config_dirs, data_home, data_dirs }
    }

    /// `$XDG_CURRENT_DESKTOP` öğeleri, küçük harf (dosya adı öneki; yalnızca güvenli karakterler).
    pub fn desktops_lower(&self) -> Vec<String> {
        self.scan
            .desktops
            .iter()
            .map(|d| d.to_ascii_lowercase())
            .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) && !d.starts_with('.'))
            .collect()
    }

    /// Yazılan `mimeapps.list`.
    pub fn user_mimeapps(&self) -> Option<PathBuf> {
        self.config_home.as_ref().map(|c| c.join("mimeapps.list"))
    }
}

/// Masaüstü indeksi: kimlik → girdi.
#[derive(Debug, Default)]
pub struct Index {
    entries: Vec<Arc<DesktopEntry>>,
    by_id: HashMap<String, usize>,
}

impl Index {
    pub fn new(entries: Vec<Arc<DesktopEntry>>) -> Self {
        let mut by_id = HashMap::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            by_id.entry(e.id.clone()).or_insert(i);
        }
        Self { entries, by_id }
    }

    pub fn get(&self, id: &str) -> Option<&DesktopEntry> {
        self.by_id.get(id).map(|&i| &*self.entries[i])
    }

    fn get_arc(&self, id: &str) -> Option<Arc<DesktopEntry>> {
        self.by_id.get(id).map(|&i| Arc::clone(&self.entries[i]))
    }

    /// Kurulu: indekste, uygulama ve `Hidden` değil (gizli girdi silinmiş sayılır).
    pub fn installed(&self, id: &str) -> bool {
        self.get(id).is_some_and(|e| e.kind == crate::services::launcher::desktop::EntryType::Application && !e.hidden)
    }

    /// Rolün adayları: uygulama, `Hidden` değil, `TryExec` (varsa) bulunuyor, birincil MIME'ı
    /// bildiriyor (terminal: `Categories` `TerminalEmulator`). Ada göre (küçük harf), `NoDisplay`
    /// olanlar sonda.
    pub fn candidates(&self, role: &Role, path: &[PathBuf]) -> Vec<&DesktopEntry> {
        let mut exec_ok: HashMap<&str, bool> = HashMap::new();
        let mut out: Vec<&DesktopEntry> = self
            .entries
            .iter()
            .map(|e| &**e)
            .filter(|e| e.kind == crate::services::launcher::desktop::EntryType::Application && !e.hidden)
            // `set`'in kabul ettiği kimlikler: listede görünüp yazılamayan aday olmasın.
            .filter(|e| valid_desktop_id(&e.id))
            .filter(|e| match role.primary {
                Some(p) => e.mime_types.iter().any(|m| m == p),
                None => e.categories.iter().any(|c| c == "TerminalEmulator"),
            })
            .filter(|e| match e.try_exec.as_deref() {
                Some(t) => *exec_ok.entry(t).or_insert_with(|| find_executable(t, path).is_some()),
                None => true,
            })
            .collect();
        out.sort_by_cached_key(|e| (e.no_display, e.name.to_lowercase(), e.id.clone()));
        out
    }
}

// ---------------------------------------------------------------------------------------------
// status

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppRef {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    /// `NoDisplay=true` (işleyici; listede sonda).
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoleStatus {
    pub role: &'static str,
    pub current: Option<AppRef>,
    pub source: &'static str,
    pub mixed: bool,
    pub shadowed_by: Option<String>,
    pub available: bool,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub v: u32,
    pub roles: Vec<RoleStatus>,
}

impl Status {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"v\":1,\"roles\":[]}".into())
    }
}

fn app_ref(index: &Index, id: &str) -> AppRef {
    match index.get(id) {
        Some(e) => AppRef { id: id.to_owned(), name: e.name.clone(), icon: e.icon.clone() },
        // Kurulu değil: ad yerine kimlik.
        None => AppRef { id: id.to_owned(), name: id.to_owned(), icon: None },
    }
}

/// `msg defaults open <rol>` (temel katman kısayolları: terminal, dosya yöneticisi, tarayıcı —
/// ADR-0006): rolün geçerli varsayılanı; yoksa ya da kurulu değilse görünür ilk aday (ada göre).
/// Dönüş: (girdi, yedek mi). Hata: `unknown_role` ya da `no_app`.
pub fn open_target(dirs: &Dirs, index: &Index, role_name: &str) -> Result<(Arc<DesktopEntry>, bool), &'static str> {
    let role = role(role_name).ok_or("unknown_role")?;
    let installed = |id: &str| index.installed(id);
    let current = match role.primary {
        None => terminal::current(dirs, &installed).0,
        Some(primary) => {
            let wanted: HashSet<&str> = std::iter::once(primary).collect();
            mimeapps::resolve(&mimeapps::load_chain(dirs), &MimeCache::load(dirs, &wanted), primary, &installed).id
        }
    };
    if let Some(e) = current.as_deref().filter(|id| installed(id)).and_then(|id| index.get_arc(id)) {
        return Ok((e, false));
    }
    let first = index.candidates(role, &dirs.scan.path).into_iter().find(|e| !e.no_display).map(|e| e.id.clone());
    first.and_then(|id| index.get_arc(&id)).map(|e| (e, true)).ok_or("no_app")
}

/// `xdg-terminal-exec` PATH'te mi?
pub fn terminal_available(dirs: &Dirs) -> bool {
    find_executable(terminal::TOOL, &dirs.scan.path).is_some()
}

/// Bütün rollerin durumu. `env`: yolları `~` ile kısaltmak için.
pub fn status(env: &Env, dirs: &Dirs, index: &Index) -> Status {
    let chain = mimeapps::load_chain(dirs);
    let wanted: HashSet<&str> = ROLES.iter().flat_map(|r| r.mimes.iter().copied()).collect();
    let cache = MimeCache::load(dirs, &wanted);
    let terminal_ok = terminal_available(dirs);
    let roles = ROLES.iter().map(|r| role_status(env, dirs, index, &chain, &cache, r, terminal_ok)).collect();
    Status { v: 1, roles }
}

fn role_status(
    env: &Env,
    dirs: &Dirs,
    index: &Index,
    chain: &[ListFile],
    cache: &MimeCache,
    role: &Role,
    terminal_ok: bool,
) -> RoleStatus {
    let installed = |id: &str| index.installed(id);
    let candidates = index
        .candidates(role, &dirs.scan.path)
        .into_iter()
        .map(|e| Candidate { id: e.id.clone(), name: e.name.clone(), icon: e.icon.clone(), hidden: e.no_display })
        .collect();
    let Some(primary) = role.primary else {
        let (id, source, shadow) = terminal::current(dirs, &installed);
        return RoleStatus {
            role: role.name,
            current: id.map(|id| app_ref(index, &id)),
            source: source.name(),
            mixed: false,
            shadowed_by: shadow.map(|p| env.display_path(&p)),
            available: terminal_ok,
            candidates,
        };
    };
    let cur = mimeapps::resolve(chain, cache, primary, &installed);
    // Karışık: geçerli uygulamanın bildirdiği, yazılan kümedeki MIME'lar birden çok yere gidiyor.
    let mixed = cur.id.as_deref().and_then(|id| index.get(id)).is_some_and(|e| {
        let targets: HashSet<Option<String>> = role
            .mimes
            .iter()
            .filter(|m| e.mime_types.iter().any(|x| x == *m))
            .map(|m| mimeapps::resolve(chain, cache, m, &installed).id)
            .collect();
        targets.len() > 1
    });
    // Gölge: kullanıcının masaüstüne özgü dosyası kümedeki bir MIME'ı tanımlıyor.
    let shadowed_by = chain
        .iter()
        .filter(|f| f.source == Source::DesktopUser)
        .find(|f| role.mimes.iter().any(|m| f.default_for(m).is_some()))
        .map(|f| env.display_path(&f.path));
    RoleStatus {
        role: role.name,
        current: cur.id.as_deref().map(|id| app_ref(index, id)),
        source: cur.source.name(),
        mixed,
        shadowed_by,
        available: true,
        candidates,
    }
}

// ---------------------------------------------------------------------------------------------
// set

/// `msg defaults set` hata kodları (sözleşme).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    UnknownRole,
    UnknownApp,
    Unavailable,
    WriterDisabled,
    /// Ayrıntı: `nuxut-apply` sonuç/hata adı (yol ve değer içermez).
    WriteFailed(String),
}

impl SetError {
    pub fn code(&self) -> &'static str {
        match self {
            SetError::UnknownRole => "unknown_role",
            SetError::UnknownApp => "unknown_app",
            SetError::Unavailable => "unavailable",
            SetError::WriterDisabled => "writer_disabled",
            SetError::WriteFailed(_) => "write_failed",
        }
    }
}

/// Başarılı yazım: snapshot (değişiklik yoksa `None`) ve yazılan MIME'lar (terminalde dosya adı).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetOk {
    pub snapshot: Option<String>,
    pub written: Vec<String>,
}

pub fn set_json(result: &Result<SetOk, SetError>) -> String {
    let v = match result {
        Ok(ok) => serde_json::json!({ "ok": true, "snapshot": ok.snapshot, "written": ok.written }),
        Err(SetError::WriteFailed(detail)) => serde_json::json!({ "ok": false, "error": "write_failed", "detail": detail }),
        Err(e) => serde_json::json!({ "ok": false, "error": e.code() }),
    };
    v.to_string()
}

/// Masaüstü kimliği biçimi (`a.b-c_d.desktop`, en çok 255 bayt).
fn valid_desktop_id(id: &str) -> bool {
    id.len() <= 255
        && id.ends_with(".desktop")
        && id.len() > ".desktop".len()
        && !id.starts_with(['.', '-'])
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}

/// Snapshot ayar kimliği. Her rolün kimliği kapalı listededir (test); yine de eksikse panik
/// yerine hata — worker thread'i ölmesin.
fn setting_for(role: &Role) -> Result<nuxut_apply::SettingId, SetError> {
    crate::services::apply::known_setting(&format!("defaults.{}", role.name)).ok_or_else(|| SetError::WriteFailed("unknown_setting".into()))
}

/// `[Default Applications]` başlığı birden çok kez var mı? GLib yinelenen bölümleri birleştirir
/// (sonraki değer kazanır) ama `nuxut-apply` `IniKey` yalnızca ilk bölümü düzenler: böyle dosyaya
/// yazım etkisiz kalabilir, bu yüzden yazılmaz (`write_failed`/`duplicate_section`).
fn duplicate_defaults_section(text: &str) -> bool {
    let header = format!("[{DEFAULTS_SECTION}]");
    text.lines().filter(|l| l.trim() == header).count() > 1
}

fn outcome(out: Outcome, written: Vec<String>) -> Result<SetOk, SetError> {
    match out {
        Outcome::Applied { id } => Ok(SetOk { snapshot: Some(id.as_str().to_owned()), written }),
        Outcome::Noop => Ok(SetOk::default()),
        Outcome::Skipped { .. } => Err(SetError::WriterDisabled),
        other => Err(SetError::WriteFailed(match &other {
            Outcome::Error { kind } => (*kind).to_owned(),
            o => o.name().to_owned(),
        })),
    }
}

/// Rolün varsayılanını `id` yapar. Bloklayan IO (snapshot kilidi ≤ 5 sn).
pub fn set(env: &Env, dirs: &Dirs, index: &Index, role_name: &str, id: &str) -> Result<SetOk, SetError> {
    let role = role(role_name).ok_or(SetError::UnknownRole)?;
    if !valid_desktop_id(id) {
        return Err(SetError::UnknownApp);
    }
    let entry = index.candidates(role, &dirs.scan.path).into_iter().find(|e| e.id == id).ok_or(SetError::UnknownApp)?;
    if role.primary.is_none() && !terminal_available(dirs) {
        return Err(SetError::Unavailable);
    }
    if env.disabled {
        return Err(SetError::WriterDisabled);
    }
    let label = t!("shell-defaults-snapshot-label", role = role.name);
    if role.primary.is_none() {
        return set_terminal(env, dirs, role, id, &label);
    }
    let file = dirs.user_mimeapps().ok_or_else(|| SetError::WriteFailed("no_config_home".into()))?;
    let current = read_optional(&file)?;
    if duplicate_defaults_section(&current) {
        return Err(SetError::WriteFailed("duplicate_section".into()));
    }
    let setting = setting_for(role)?;
    let value = format!("{id};");
    // Yalnızca değişen MIME'lar (önceki değer tamamen değişir; aynıysa dokunulmaz).
    let changed: Vec<String> = role
        .mimes
        .iter()
        .filter(|m| entry.mime_types.iter().any(|x| x == *m))
        .filter(|m| {
            nuxut_apply::edit::ini::get(&current, DEFAULTS_SECTION, m).map(|v| v.trim().to_owned()).as_deref() != Some(value.as_str())
        })
        .map(|m| (*m).to_owned())
        .collect();
    if changed.is_empty() {
        return Ok(SetOk::default());
    }
    let mut change = Change::new(setting, label, nuxut_apply::Source::Settings);
    for m in &changed {
        change = change.write(
            Fragment::IniKey { file: file.clone(), section: DEFAULTS_SECTION.into(), key: m.clone() },
            FragValue::Text(value.clone()),
        );
    }
    outcome(Applier::open(env.clone()).with_source(nuxut_apply::Source::Settings).apply(change), changed)
}

/// Dosya metni; yoksa boş.
fn read_optional(file: &Path) -> Result<String, SetError> {
    match nuxut_apply::read_file(file) {
        Ok((text, _)) => Ok(text),
        Err(nuxut_apply::ApplyError::NotFound(_)) => Ok(String::new()),
        Err(e) => Err(SetError::WriteFailed(e.kind_name().to_owned())),
    }
}

fn set_terminal(env: &Env, dirs: &Dirs, role: &Role, id: &str, label: &str) -> Result<SetOk, SetError> {
    set_terminal_with(env, dirs, role, id, label, |change| Applier::open(env.clone()).with_source(nuxut_apply::Source::Settings).apply(change))
}

/// [`set_terminal`]; `add`: başa ekleme işlemini uygular (testte başarısızlık benzetimi için).
fn set_terminal_with(
    env: &Env,
    dirs: &Dirs,
    role: &Role,
    id: &str,
    label: &str,
    add: impl FnOnce(Change) -> Outcome,
) -> Result<SetOk, SetError> {
    let file = dirs.config_home.as_ref().map(|c| c.join(terminal::LIST)).ok_or_else(|| SetError::WriteFailed("no_config_home".into()))?;
    let text = read_optional(&file)?;
    let setting = setting_for(role)?;
    let line = |at| Fragment::Line { file: file.clone(), marker: id.to_owned(), at };
    let written = vec![terminal::LIST.to_owned()];
    let mut removed: Option<nuxut_apply::SnapshotId> = None;
    match terminal::plan(&text, id) {
        terminal::Plan::Unchanged => return Ok(SetOk::default()),
        terminal::Plan::Prepend => {}
        terminal::Plan::MoveToTop => {
            // `Line` satır taşımaz: aşağıdaki satır önce kaldırılır. Kaynak farklı (Shell) ki
            // ardından gelen ekleme (Settings) bu işleme birleşmesin: son snapshot'ın geri
            // yüklenmesi eklemeyi geri alır ve önceki ilk girdi yeniden geçerli olur.
            let remove = Change::new(setting.clone(), label, nuxut_apply::Source::Shell).write(line(Position::End), FragValue::Absent);
            removed = outcome(Applier::open(env.clone()).with_source(nuxut_apply::Source::Shell).apply(remove), Vec::new())?
                .snapshot
                .and_then(|s| nuxut_apply::SnapshotId::parse(&s).ok());
        }
    }
    let change = Change::new(setting, label, nuxut_apply::Source::Settings).write_creating(line(Position::Start), FragValue::Text(id.to_owned()));
    let result = outcome(add(change), written);
    // Telafi: ekleme düştüyse kaldırılan satır geri konur (kaldırma snapshot'ı geri yüklenir;
    // satır listenin sonuna döner — ilk girdi değişmediği için etkin terminal aynı kalır).
    if result.is_err()
        && let Some(snap) = removed
    {
        let back = Applier::open(env.clone()).with_source(nuxut_apply::Source::Shell).restore(&snap, nuxut_apply::RestoreMode::Safe);
        if !matches!(back, nuxut_apply::RestoreOutcome::Restored { .. }) {
            tracing::warn!(result = back.name(), "terminal satırı geri konamadı");
        }
    }
    result
}

// ---------------------------------------------------------------------------------------------
// Worker

/// Taramalar arası ayrıştırma önbelleğini tutan iş birimi; tek thread'de sırayla çalışır
/// (eşzamanlı iki `set` aynı dosyayı yarıştırmaz).
#[derive(Debug)]
pub struct Worker {
    env: Env,
    dirs: Dirs,
    scanner: Scanner,
}

impl Worker {
    pub fn new(env: Env, dirs: Dirs) -> Self {
        Self { env, dirs, scanner: Scanner::new() }
    }

    fn index(&mut self) -> Index {
        Index::new(self.scanner.scan_index(&self.dirs.scan))
    }

    /// `msg defaults status` yanıtı (tek satır JSON).
    pub fn status_json(&mut self) -> String {
        let index = self.index();
        status(&self.env, &self.dirs, &index).to_json()
    }

    /// `msg defaults open <rol>`: yanıt (tek satır JSON) ve başlatılacak girdi.
    pub fn open(&mut self, role: &str) -> (String, Option<Arc<DesktopEntry>>) {
        let index = self.index();
        match open_target(&self.dirs, &index, role) {
            Ok((entry, fallback)) => {
                let json = serde_json::json!({ "ok": true, "id": entry.id, "fallback": fallback }).to_string();
                (json, Some(entry))
            }
            Err(code) => (serde_json::json!({ "ok": false, "error": code }).to_string(), None),
        }
    }

    /// `msg defaults set <rol> <kimlik>` yanıtı (tek satır JSON).
    pub fn set_json(&mut self, role: &str, id: &str) -> String {
        let index = self.index();
        let r = set(&self.env, &self.dirs, &index, role, id);
        match &r {
            Ok(ok) => tracing::info!(role, written = ok.written.len(), "varsayılan uygulama yazıldı"),
            Err(e) => tracing::info!(role, error = e.code(), "varsayılan uygulama yazılmadı"),
        }
        set_json(&r)
    }
}
