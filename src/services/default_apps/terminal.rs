//! Terminal rolü: `xdg-terminal-exec` listeleri (`xdg-terminals.list`).
//!
//! MIME türü yoktur; seçim `xdg-terminal-exec`'in okuduğu listelerle yapılır. Okuma sırası (her
//! dizinde önce `<masaüstü>-xdg-terminals.list`, sonra `xdg-terminals.list`): `$XDG_CONFIG_HOME` →
//! `$XDG_CONFIG_DIRS` → `$XDG_DATA_HOME/xdg-terminal-exec` → `$XDG_DATA_DIRS/xdg-terminal-exec`.
//! İlk **kurulu** girdi geçerlidir. Satır biçimi: masaüstü kimliği; `#` yorum; `:eylem` / `/…`
//! ekleri kimlikten ayrılır.
//!
//! Yazım yalnızca `$XDG_CONFIG_HOME/xdg-terminals.list`'e, `nuxut-apply` `Line` parçasıyla
//! (`Position::Start`, dosya yoksa yaratılır); diğer satırlar korunur. `Line` var/yok parçasıdır,
//! satır taşımaz: kimlik dosyada daha aşağıdaysa önce o satır ayrı bir işlemde kaldırılır, sonra
//! başa eklenir (bkz. [`plan`], docs/design/defaults.md).

use std::path::{Path, PathBuf};

use super::mimeapps::Source;

/// `xdg-terminal-exec` program adı (PATH'te aranır).
pub const TOOL: &str = "xdg-terminal-exec";
/// Yazılan dosyanın adı.
pub const LIST: &str = "xdg-terminals.list";

/// Satırın masaüstü kimliği (yorum/boş → `None`).
pub fn entry_id(line: &str) -> Option<&str> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    let id = t.split([':', '/']).next().unwrap_or("").trim();
    (!id.is_empty()).then_some(id)
}

/// Listelerin yolları ve kaynakları, öncelik sırasıyla.
pub fn chain_paths(dirs: &super::Dirs) -> Vec<(PathBuf, Source)> {
    let mut out = Vec::new();
    let mut push_dir = |dir: &Path, src_desktop: Source, src: Source| {
        for d in &dirs.desktops_lower() {
            out.push((dir.join(format!("{d}-{LIST}")), src_desktop));
        }
        out.push((dir.join(LIST), src));
    };
    if let Some(c) = &dirs.config_home {
        push_dir(c, Source::DesktopUser, Source::User);
    }
    for c in &dirs.config_dirs {
        push_dir(c, Source::System, Source::System);
    }
    if let Some(d) = &dirs.data_home {
        push_dir(&d.join("xdg-terminal-exec"), Source::User, Source::User);
    }
    for d in &dirs.data_dirs {
        push_dir(&d.join("xdg-terminal-exec"), Source::System, Source::System);
    }
    out
}

/// Geçerli terminal: `(kimlik, kaynak, gölgeleyen masaüstü dosyası)`. Gölge: kullanıcının
/// `<masaüstü>-xdg-terminals.list`'i kurulu bir girdi taşıyor (bizim yazdığımız dosyadan önce okunur).
pub fn current(dirs: &super::Dirs, installed: &dyn Fn(&str) -> bool) -> (Option<String>, Source, Option<PathBuf>) {
    let mut found: Option<(String, Source)> = None;
    let mut shadow = None;
    for (path, src) in chain_paths(dirs) {
        let Ok((text, _)) = nuxut_apply::read_file(&path) else { continue };
        let first = text.lines().filter_map(entry_id).find(|id| installed(id)).map(str::to_owned);
        if let Some(id) = first {
            if src == Source::DesktopUser && shadow.is_none() {
                shadow = Some(path.clone());
            }
            if found.is_none() {
                found = Some((id, src));
            }
        }
    }
    match found {
        Some((id, src)) => (Some(id), src, shadow),
        None => (None, Source::None, shadow),
    }
}

/// Yazım planı (dosyanın şu anki metni üzerinden).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Kimlik zaten ilk girdi.
    Unchanged,
    /// Satır yok: başa ekle.
    Prepend,
    /// Satır daha aşağıda: önce kaldır (ayrı işlem), sonra başa ekle.
    MoveToTop,
}

/// `text`: `$XDG_CONFIG_HOME/xdg-terminals.list` (yoksa boş).
pub fn plan(text: &str, id: &str) -> Plan {
    // İlk girdi bu uygulama (ekli biçimde `id:eylem` de olabilir): dokunulmaz.
    if text.lines().find_map(entry_id) == Some(id) {
        return Plan::Unchanged;
    }
    if text.lines().any(|l| l.trim() == id) { Plan::MoveToTop } else { Plan::Prepend }
}
