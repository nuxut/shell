//! Ayar onarımının tanı çıktısı: `msg config status` JSON'u, `msg config heal` satırları ve
//! durum dosyalarının baş yorumları. Çevrilmez (ADR-0010: msg/CLI yanıtları ve tanı çıktısı).

use std::path::Path;

use nuxut_apply::Env;
use nuxut_config_heal::{Action, ActionKind};

use super::{
    FRAMEWORK_FILE, HealReport, SHELL_FILE, UndoEntry, backup_dir, file_path, heal_framework, keep_set, load_undo, read_lossy,
};
use crate::config::ShellConfig;

/// Koru listesi dosyasının baş yorumu.
pub(super) const KEEP_HEADER: &str =
    "# Ayar onarımının dokunmadığı anahtarlar (Geri al / Yok say ile eklenir; silmek serbest).\n";
/// Geri al kaydı dosyasının baş yorumu.
pub(super) const UNDO_HEADER: &str = "# Son ayar onarımının geri al kaydı (nuxut-shell msg config undo). Elle düzenlenmez.\n";

/// `msg config heal` yanıtında bir dosyanın özet satırı.
pub fn summary_line(r: &HealReport) -> String {
    let done = if r.written.is_some() { "yazıldı" } else { "yazılamadı (bellekte uygulandı)" };
    format!("{}: {} değişiklik, {done}", r.file, r.actions.iter().filter(|a| a.kind.edits()).count())
}



/// `msg config status` girdisi (kabuğun süreç durumu).
#[derive(Debug, Clone, Copy)]
pub struct StatusInput<'a> {
    /// Bu örnek ayar dosyalarına yazar mı (asıl örnek).
    pub disk: bool,
    pub unversioned_is_current: bool,
    /// Bu oturumda geri alınabilir sürüm göçü var mı.
    pub migration_undo: bool,
    /// Bellekteki onarım geri al kaydı (boşsa diskteki okunur).
    pub undo: &'a [UndoEntry],
}

fn action_json(a: &Action) -> serde_json::Value {
    let kind = match &a.kind {
        ActionKind::Broken => "broken",
        ActionKind::Duplicate => "duplicate",
        ActionKind::MergedTable => "merged_table",
        ActionKind::Moved { .. } => "renamed",
        ActionKind::Invalid => "invalid",
        ActionKind::Clamped { .. } => "clamped",
        ActionKind::Retired => "retired",
        ActionKind::Unused => "unused",
        ActionKind::Redundant => "redundant",
        ActionKind::Suggest { .. } => "suggest",
    };
    let mut v = serde_json::json!({ "line": a.line, "key": a.key, "kind": kind });
    match &a.kind {
        ActionKind::Moved { to } | ActionKind::Clamped { to } => v["to"] = to.clone().into(),
        ActionKind::Suggest { candidate } => v["candidate"] = candidate.clone().into(),
        _ => {}
    }
    v
}

fn backups_json(env: &Env, name: &str) -> serde_json::Value {
    let list = backup_dir(env).map(|d| nuxut_config_heal::backup::list(&d, name)).unwrap_or_default();
    serde_json::json!({ "count": list.len(), "latest": list.first().map(|p| p.display().to_string()) })
}

/// Ayar dosyalarının durumu: sürüm, bekleyen göç/onarım (salt okunur kuru çalıştırma), geri al,
/// yedekler, koru listesi. Hiçbir şey yazmaz.
pub fn status(env: &Env, input: StatusInput<'_>) -> serde_json::Value {
    let shell = match file_path(SHELL_FILE) {
        None => serde_json::json!({ "error": "config dizini bulunamadı" }),
        Some(path) => shell_status(env, &path, input.unversioned_is_current),
    };
    let framework = {
        let path = file_path(FRAMEWORK_FILE);
        let pending: Vec<_> = heal_framework(env, false).map(|r| r.actions).unwrap_or_default().iter().map(action_json).collect();
        serde_json::json!({
            "path": path.as_ref().map(|p| p.display().to_string()),
            "exists": path.as_ref().is_some_and(|p| p.exists()),
            "pending": pending,
            "backups": backups_json(env, FRAMEWORK_FILE),
        })
    };
    let undo: Vec<UndoEntry> = if input.undo.is_empty() { load_undo(env) } else { input.undo.to_vec() };
    serde_json::json!({
        "current_version": crate::config_migrate::CURRENT_CONFIG_VERSION,
        "writes": input.disk,
        "shell": shell,
        "framework": framework,
        "undo": {
            "heal": undo.iter().map(|e| serde_json::json!({ "file": e.file, "backup": e.written.backup.display().to_string() })).collect::<Vec<_>>(),
            "migration": input.migration_undo,
        },
        "keep": keep_set(env, SHELL_FILE).into_iter().collect::<Vec<_>>(),
    })
}

pub(super) fn shell_status(env: &Env, path: &Path, unversioned_is_current: bool) -> serde_json::Value {
    use crate::config_migrate::{self, Migration};
    let mut v = serde_json::json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "backups": backups_json(env, SHELL_FILE),
    });
    let raw = match read_lossy(path) {
        Ok(Some((raw, _))) => raw,
        Ok(None) => return v,
        Err(e) => {
            v["error"] = e.log_summary().into();
            return v;
        }
    };
    v["version"] = match config_migrate::migrate(&raw) {
        Migration::Unparsable => serde_json::Value::Null,
        _ if config_migrate::is_unversioned(&raw) => serde_json::Value::Null,
        _ => raw
            .parse::<toml_edit::DocumentMut>()
            .map(|d| serde_json::Value::from(config_migrate::file_version(&d)))
            .unwrap_or(serde_json::Value::Null),
    };
    let heal = crate::config::HealOptions { keep: keep_set(env, SHELL_FILE), ..Default::default() };
    match ShellConfig::load_full(path, None, unversioned_is_current, &heal) {
        Ok(loaded) => {
            let actions = loaded.heal.map(|h| h.actions).unwrap_or_default();
            v["pending"] = actions.iter().filter(|a| a.kind.edits()).map(action_json).collect::<Vec<_>>().into();
            v["suggestions"] =
                actions.iter().filter(|a| matches!(a.kind, ActionKind::Suggest { .. })).map(action_json).collect::<Vec<_>>().into();
            v["migration"] = loaded.migration.map(|m| serde_json::json!({ "from": m.from, "to": m.to })).into();
            v["warnings"] = loaded.warnings.into();
        }
        Err(e) => v["error"] = e.to_string().into(),
    }
    v
}

// --- msg yanıtı ----------------------------------------------------------------------------

/// `config heal --dry-run` yanıtı: her değişiklik bir satır (tanı çıktısı, çevrilmez).
pub fn describe(file: &str, actions: &[Action]) -> String {
    if actions.is_empty() {
        return format!("{file}: onarılacak bir şey yok");
    }
    let mut out = String::new();
    for a in actions {
        let what = match &a.kind {
            ActionKind::Broken => "okunamayan satır yoruma alınır".to_owned(),
            ActionKind::Duplicate => "tekrar eden anahtar: önceki yoruma alınır".to_owned(),
            ActionKind::MergedTable => "tekrar eden tablo birleştirilir".to_owned(),
            ActionKind::Moved { to } => format!("eski ad, {to} olarak taşınır"),
            ActionKind::Invalid => "geçersiz değer yoruma alınır, varsayılan geçerli".to_owned(),
            ActionKind::Clamped { to } => format!("sınır dışı, {to} yazılır"),
            ActionKind::Retired => "kaldırılmış anahtar silinir".to_owned(),
            ActionKind::Unused => "tanınmayan, etkisiz anahtar silinir".to_owned(),
            ActionKind::Redundant => "varsayılana eşit, silinir".to_owned(),
            ActionKind::Suggest { candidate } => format!("tanınmıyor; {candidate} mi demek istediniz? (değişiklik yok)"),
        };
        let key = if a.key.is_empty() { String::new() } else { format!(" {}", a.key) };
        out.push_str(&format!("{file}:{}{key}: {what}\n", a.line));
    }
    out.trim_end().to_owned()
}

