//! `mimeapps.list` zinciri ve `mimeinfo.cache` yedeği: "bu MIME'ı ne açıyor?" (`xdg-mime query
//! default` eşdeğeri; sözleşme defaults-v1 "Mevcut seçimin çözümü").
//!
//! Sıra (her dizinde önce `<masaüstü>-mimeapps.list` — `$XDG_CURRENT_DESKTOP` öğeleri, küçük
//! harf, sırasıyla — sonra `mimeapps.list`): `$XDG_CONFIG_HOME` → `$XDG_CONFIG_DIRS` →
//! `$XDG_DATA_HOME/applications` → `$XDG_DATA_DIRS/applications`. Bir dosyanın
//! `[Default Applications]` girdisindeki ilk **kurulu** kimlik kazanır. Bir dosyanın
//! `[Removed Associations]`'ı ondan **sonraki** dosyaların varsayılanlarını ve önbellek yedeğini
//! etkiler; aynı dosyadaki açık varsayılan kaldırmadan güçlüdür (karar: docs/design/defaults.md).
//! Hiçbir dosyanın varsayılanı yoksa `[Added Associations]` (dosya sırasıyla, ilk kurulu), o da yoksa
//! `mimeinfo.cache`'lerdeki (veri dizini sırasıyla) ilk kurulu aday.
//!
//! Okuma salt okunurdur ve boyut sınırlıdır (`nuxut_apply::read_file`); yazım [`super::set`]'te.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Bir zincir dosyasının kaynağı (`msg defaults status` `source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `$XDG_CONFIG_HOME/mimeapps.list` (ve eski konum `$XDG_DATA_HOME/applications/`).
    User,
    /// `$XDG_CONFIG_HOME/<masaüstü>-mimeapps.list`.
    DesktopUser,
    /// Sistem dizinleri (`$XDG_CONFIG_DIRS`, `$XDG_DATA_DIRS`).
    System,
    /// `mimeinfo.cache` yedeği.
    SystemCache,
    None,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::User => "user",
            Source::DesktopUser => "desktop_user",
            Source::System => "system",
            Source::SystemCache => "system_cache",
            Source::None => "none",
        }
    }
}

/// Zincirdeki bir dosyanın okunmuş hâli (yalnızca iki bölüm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListFile {
    pub path: PathBuf,
    pub source: Source,
    /// `(mime, kimlikler)`; dosyadaki sırayla. Aynı anahtar yinelenirse sonuncusu (INI kuralı).
    pub defaults: Vec<(String, Vec<String>)>,
    pub removed: Vec<(String, Vec<String>)>,
    /// `[Added Associations]` (varsayılan yoksa tercih sırası).
    pub added: Vec<(String, Vec<String>)>,
}

impl ListFile {
    pub fn default_for(&self, mime: &str) -> Option<&[String]> {
        self.defaults.iter().rev().find(|(m, _)| m == mime).map(|(_, ids)| ids.as_slice())
    }

    fn added_for(&self, mime: &str) -> impl Iterator<Item = &str> {
        self.added.iter().rev().find(|(m, _)| m == mime).into_iter().flat_map(|(_, ids)| ids.iter().map(String::as_str))
    }

    fn removed_for(&self, mime: &str) -> impl Iterator<Item = &str> {
        self.removed.iter().filter(move |(m, _)| m == mime).flat_map(|(_, ids)| ids.iter().map(String::as_str))
    }
}

/// `;` ayrılmış kimlik listesi (boşlar atılır).
pub fn split_ids(value: &str) -> Vec<String> {
    value.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

/// `mimeapps.list` metninden iki bölümü okur (diğerleri yok sayılır).
pub fn parse(path: &Path, source: Source, text: &str) -> ListFile {
    let mut out = ListFile { path: path.to_path_buf(), source, defaults: Vec::new(), removed: Vec::new(), added: Vec::new() };
    let mut section = "";
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with(';') {
            continue;
        }
        if let Some(h) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            section = match h {
                "Default Applications" => "d",
                "Removed Associations" => "r",
                "Added Associations" => "a",
                _ => "",
            };
            continue;
        }
        let Some((k, v)) = t.split_once('=') else { continue };
        let item = (k.trim().to_owned(), split_ids(v));
        match section {
            "d" => out.defaults.push(item),
            "r" => out.removed.push(item),
            "a" => out.added.push(item),
            _ => {}
        }
    }
    out
}

/// Zincirin dosya yolları ve kaynakları, öncelik sırasıyla (var olup olmadıklarına bakılmaz).
pub fn chain_paths(dirs: &super::Dirs) -> Vec<(PathBuf, Source)> {
    let mut out = Vec::new();
    let mut push_dir = |dir: &Path, user: bool, desktop_user: bool| {
        for d in &dirs.desktops_lower() {
            let src = if desktop_user { Source::DesktopUser } else if user { Source::User } else { Source::System };
            out.push((dir.join(format!("{d}-mimeapps.list")), src));
        }
        out.push((dir.join("mimeapps.list"), if user { Source::User } else { Source::System }));
    };
    if let Some(c) = &dirs.config_home {
        push_dir(c, true, true);
    }
    for c in &dirs.config_dirs {
        push_dir(c, false, false);
    }
    if let Some(d) = &dirs.data_home {
        // Eski (kullanımdan kalkmış) kullanıcı konumu: kullanıcının dosyası sayılır.
        push_dir(&d.join("applications"), true, false);
    }
    for d in &dirs.data_dirs {
        push_dir(&d.join("applications"), false, false);
    }
    out
}

/// Var olan zincir dosyalarını okur (okunamayan / çok büyük dosya atlanır).
pub fn load_chain(dirs: &super::Dirs) -> Vec<ListFile> {
    chain_paths(dirs)
        .into_iter()
        .filter_map(|(p, src)| match nuxut_apply::read_file(&p) {
            Ok((text, _)) => Some(parse(&p, src, &text)),
            Err(nuxut_apply::ApplyError::NotFound(_)) => None,
            Err(e) => {
                tracing::debug!(error = e.kind_name(), "mimeapps.list okunamadı; atlandı");
                None
            }
        })
        .collect()
}

/// `mimeinfo.cache` (veri dizini sırasıyla) — yalnızca istenen MIME'lar okunur.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MimeCache {
    entries: Vec<(String, Vec<String>)>,
}

impl MimeCache {
    pub fn load(dirs: &super::Dirs, wanted: &HashSet<&str>) -> Self {
        let mut out = Self::default();
        for d in dirs.data_home.iter().chain(&dirs.data_dirs) {
            let p = d.join("applications/mimeinfo.cache");
            match nuxut_apply::read_file(&p) {
                Ok((text, _)) => out.add(&text, wanted),
                Err(nuxut_apply::ApplyError::NotFound(_)) => {}
                Err(e) => tracing::debug!(error = e.kind_name(), "mimeinfo.cache okunamadı; atlandı"),
            }
        }
        out
    }

    /// Bir önbellek dosyasının `[MIME Cache]` bölümünden istenenleri ekler.
    pub fn add(&mut self, text: &str, wanted: &HashSet<&str>) {
        let mut in_section = false;
        for line in text.lines() {
            let t = line.trim();
            if let Some(h) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                in_section = h == "MIME Cache";
                continue;
            }
            if !in_section {
                continue;
            }
            if let Some((k, v)) = t.split_once('=')
                && wanted.contains(k.trim())
            {
                self.entries.push((k.trim().to_owned(), split_ids(v)));
            }
        }
    }

    fn ids(&self, mime: &str) -> impl Iterator<Item = &str> {
        self.entries.iter().filter(move |(m, _)| m == mime).flat_map(|(_, ids)| ids.iter().map(String::as_str))
    }
}

/// Bir MIME'ın çözümü.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub id: Option<String>,
    pub source: Source,
}

/// `installed(id)`: kimlik kurulu mu (indekste ve `Hidden` değil).
pub fn resolve(chain: &[ListFile], cache: &MimeCache, mime: &str, installed: &dyn Fn(&str) -> bool) -> Resolved {
    let mut removed: HashSet<&str> = HashSet::new();
    for f in chain {
        if let Some(ids) = f.default_for(mime)
            && let Some(id) = ids.iter().find(|id| !removed.contains(id.as_str()) && installed(id))
        {
            return Resolved { id: Some(id.clone()), source: f.source };
        }
        removed.extend(f.removed_for(mime));
    }
    // Hiçbir dosya varsayılan vermediyse `[Added Associations]` (dosya sırasıyla; kaldırmalar
    // varsayılanlardaki kuralla: bir dosyanınki yalnızca sonrakileri etkiler).
    let mut removed_added: HashSet<&str> = HashSet::new();
    for f in chain {
        if let Some(id) = f.added_for(mime).find(|id| !removed_added.contains(id) && installed(id)) {
            return Resolved { id: Some(id.to_owned()), source: f.source };
        }
        removed_added.extend(f.removed_for(mime));
    }
    match cache.ids(mime).find(|id| !removed.contains(id) && installed(id)) {
        Some(id) => Resolved { id: Some(id.to_owned()), source: Source::SystemCache },
        None => Resolved { id: None, source: Source::None },
    }
}
