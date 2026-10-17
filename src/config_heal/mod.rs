//! Ayar dosyalarının kendini onarması: shell.toml tam, framework.toml yalnızca sözdizimi
//! (`docs/design/config-migrations.md`; saf çekirdek `crates/nuxut-config-heal`).
//!
//! - **Açılışta** (ve `msg config heal` ile) asıl örnek onarımı diske yazar: önce yedek
//!   (`$XDG_STATE_HOME/nuxut/backups/config/<dosya>.<UTC>`), sonra tek atomik yazım; tek sessiz
//!   bildirim "Ayarlarında N satır düzeltildi · Geri al".
//! - **Dosya değişince** onarım yalnızca bellekte uygulanır (geçersiz bir değer bütün dosyayı
//!   düşürmez) ve diske yazılmaz: dosyayı az önce bir insan ya da araç yazdı, açık bir düzenleyicinin
//!   altından satır silmek kullanıcının işini bozar. Değişiklik varsa "Onar" eylemli bildirim.
//! - Geri al: onarımın yazdığı metin hâlâ dosyadaysa onarım öncesi metin geri yazılır ve onarımın
//!   sildiği/taşıdığı anahtarlar "koru" listesine girer (`$XDG_STATE_HOME/nuxut/config-heal.toml`):
//!   sonraki açılışlar onlara dokunmaz. Bozuk satır ve geçersiz değer korunamaz.
//! - Yazım hatası önerisi dosyayı değiştirmez; "Düzelt" (`config heal --fix-typos`) ya da
//!   "Yok say" (`config heal --ignore-typos`, koru listesine ekler).

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use nuxut::i18n::t;
use nuxut_apply::{Env, Expect, WriteOptions};
use nuxut_config::{ConfigError, MAX_CONFIG_BYTES};
use nuxut_config_heal::{Action, ActionKind, Class, Healed, Labels, Loaded, Rules, Schema};

use crate::config::ShellConfig;
use crate::config_migrate::VERSION_KEY;
use crate::schema::{SettingKind, SettingSchema, SettingValue};

mod diag;
pub use diag::{StatusInput, describe, status, summary_line};

/// shell.toml'un dosya adı (yedek ve koru listesi anahtarı).
pub const SHELL_FILE: &str = "shell.toml";
/// framework.toml (yalnızca sözdizimi onarımı; şeması framework'te).
pub const FRAMEWORK_FILE: &str = "framework.toml";

/// Eski ad → yeni ad. Sürüm göçü (`config_migrate::MIGRATIONS`) eski sürümlü dosyayı zaten taşır;
/// bu tablo güncel sürümlü dosyaya sonradan yapıştırılmış eski adları yakalar. Yeni yeniden
/// adlandırma buraya da eklenir (bekçi testi: eski ad şemada olmamalı, yeni ad olmalı).
pub const RENAMES: &[(&str, &str)] = &[
    ("bar.left", "bar.start"),
    ("bar.right", "bar.end"),
    ("workspaces", "bar.widget.workspaces"),
    ("clock", "bar.widget.clock"),
];

/// Kaldırılmış anahtarlar: dosyada görülürse sessizce silinir, yazım hatası önerisi verilmez.
/// Şemadan bir anahtar kaldıran her değişiklik buraya ekler (bekçi testi zorlar).
pub const RETIRED: &[&str] = &["frame.inner_radius", "bar.widget.workspaces.show_empty"];

/// Koru listesinde dosya başına en çok bu kadar anahtar.
const MAX_KEEP: usize = 256;
/// Koru listesi dosyasının boyut sınırı.
const MAX_KEEP_BYTES: u64 = 64 * 1024;

/// Bildirim düğmelerinin komutları (`nuxut-shell msg …`).
pub const HEAL_COMMAND: &str = "config heal";
pub const UNDO_COMMAND: &str = "config heal --undo";
pub const FIX_TYPOS_COMMAND: &str = "config heal --fix-typos";
pub const IGNORE_TYPOS_COMMAND: &str = "config heal --ignore-typos";
/// Bildirim gövdesinde en çok bu kadar öneri.
const MAX_NOTICE_SUGGESTIONS: usize = 3;

/// shell.toml onarım kuralları (şema `ShellConfig`'in `Deserialize`'ından; varsayılanlar ve
/// aralıklar Ayarlar şemasından).
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let mut schema_list = Vec::new();
        ShellConfig::schema(&mut schema_list);
        let mut rules = Rules {
            schema: Schema::of::<ShellConfig>(),
            renames: RENAMES.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect(),
            retired: RETIRED.iter().map(|r| (*r).to_owned()).collect(),
            protected: vec![VERSION_KEY.to_owned()],
            ..Rules::default()
        };
        for s in &schema_list {
            add_schema_rule(&mut rules, s);
        }
        rules
    })
}

fn add_schema_rule(rules: &mut Rules, s: &SettingSchema) {
    let literal = match &s.default {
        SettingValue::Bool(b) => b.to_string(),
        SettingValue::Int(i) => i.to_string(),
        SettingValue::Float(f) => format!("{f:?}"),
        SettingValue::Text(t) => toml_edit::Value::from(t.as_str()).to_string().trim().to_owned(),
        SettingValue::List(v) => v.to_string(),
    };
    rules.defaults.insert(s.key.clone(), literal);
    if matches!(s.kind, SettingKind::Int | SettingKind::Float)
        && let (Some(min), Some(max)) = (s.min, s.max)
    {
        rules.limits.insert(s.key.clone(), (min.as_f64(), max.as_f64()));
    }
}

/// Eşitlik kâhini: ayrıştır + doğrula (`ShellConfig::validate`, uyarılar atılır).
fn load(text: &str) -> Loaded<ShellConfig> {
    match nuxut_config_heal::load_serde::<ShellConfig>(text) {
        Loaded::Ok { mut config, unknown } => {
            config.validate();
            Loaded::Ok { config, unknown }
        }
        other => other,
    }
}

/// Dosyaya yazılan yorum işaretleri (etkin dilde, yerel tarihle).
pub fn labels() -> Labels {
    Labels {
        stamp: chrono::Local::now().format("%Y-%m-%d").to_string(),
        broken: t!("shell-config-heal-mark-broken"),
        duplicate: t!("shell-config-heal-mark-duplicate"),
        invalid: t!("shell-config-heal-mark-invalid"),
        clamped: t!("shell-config-heal-mark-clamped"),
    }
}

/// shell.toml metninin tam onarımı (saf; `keep`: dokunulmayacak yollar, `renames`: ek taşımalar).
pub fn heal_shell(text: &str, keep: &BTreeSet<String>, labels: &Labels, extra_renames: &[(String, String)]) -> Healed {
    if extra_renames.is_empty() {
        return nuxut_config_heal::heal(text, rules(), keep, labels, &load);
    }
    let mut r = rules().clone();
    r.renames.extend(extra_renames.iter().cloned());
    nuxut_config_heal::heal(text, &r, keep, labels, &load)
}

/// Sınırlı okuma; UTF-8 olmayan bayt `U+FFFD` olur (ikinci değer `true`). Dosya yoksa `None`.
pub fn read_lossy(path: &Path) -> Result<Option<(String, bool)>, ConfigError> {
    let io = |source| ConfigError::Io { path: path.to_owned(), source };
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(e)),
    };
    let size = file.metadata().map_err(io)?.len();
    if size > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge { path: path.to_owned(), size });
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES).read_to_end(&mut bytes).map_err(io)?;
    Ok(Some(match String::from_utf8(bytes) {
        Ok(s) => (s, false),
        Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
    }))
}

/// Yedek dizini (`$XDG_STATE_HOME/nuxut/backups/config`).
pub fn backup_dir(env: &Env) -> Option<PathBuf> {
    Some(env.state_home.as_ref()?.join("nuxut/backups/config"))
}

fn utc_stamp() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// `bytes`'ı ortam kapısından, var olan dosyanın üzerine yazmadan yedekler (0600, dizin 0700).
pub fn write_backup(env: &Env, dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    use nuxut_config_heal::backup;
    let stamp = utc_stamp();
    let opts = WriteOptions::PRIVATE;
    for n in 1..=backup::MAX_SUFFIX {
        let path = dir.join(backup::file_name(name, &stamp, n));
        match nuxut_apply::write_file_with(env, &path, bytes, Some(&Expect::Absent), &opts) {
            Ok(_) => {
                backup::prune(dir, name, backup::DEFAULT_KEEP);
                return Ok(path);
            }
            Err(nuxut_apply::ApplyError::Changed) => continue,
            Err(e) => return Err(e.kind_name().to_owned()),
        }
    }
    Err("yedek adı bulunamadı".into())
}

/// Onarımın diske yazdığı bir dosya (geri al için).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealWritten {
    pub path: PathBuf,
    pub backup: PathBuf,
    /// Geri alınca yazılacak metin (onarım öncesi; aynı yüklemede sürüm göçü yazıldıysa göçmüş metin).
    pub restore: String,
    /// Onarımın yazdığı metin (geri al yalnızca dosya hâlâ buysa yapılır).
    pub written: String,
}

/// Bir dosyanın onarım raporu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealReport {
    /// `shell.toml` / `framework.toml`.
    pub file: &'static str,
    pub actions: Vec<Action>,
    /// Diske yazıldıysa; `None`: yalnızca bellekte (bekleyen onarım).
    pub written: Option<HealWritten>,
}

/// Kullanıcıya gösterilecek tek sessiz bildirim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealNotice {
    pub summary: String,
    pub body: String,
    /// (etiket, `msg` komutu).
    pub actions: Vec<(String, &'static str)>,
}

impl HealReport {
    fn count(&self, class: Class) -> usize {
        self.actions.iter().filter(|a| a.kind.class() == class).count()
    }

    /// Bildirilen değişiklik sayısı (varsayılana eşit satır ve öneri hariç).
    pub fn notable(&self) -> usize {
        self.count(Class::Repaired) + self.count(Class::Reset) + self.count(Class::Cleaned)
    }

    /// Geri alınca koru listesine girecek anahtarlar.
    pub fn keepable(&self) -> Vec<String> {
        self.actions.iter().filter(|a| a.kind.keepable() && !a.key.is_empty()).map(|a| a.key.clone()).collect()
    }

    pub fn suggestions(&self) -> Vec<(String, String)> {
        self.actions
            .iter()
            .filter_map(|a| match &a.kind {
                ActionKind::Suggest { candidate } => Some((a.key.clone(), candidate.clone())),
                _ => None,
            })
            .collect()
    }

    /// Aynı sorun kümesi için bildirim yinelenmesin diye parmak izi.
    pub fn fingerprint(&self) -> String {
        let mut v: Vec<String> = self.actions.iter().map(|a| format!("{}:{:?}", a.key, a.kind)).collect();
        v.sort();
        format!("{}|{}", self.file, v.join(","))
    }
}

/// Birden çok dosyanın raporundan tek bildirim: yazıldıysa "düzeltildi · Geri al", yazılmadıysa
/// "düzeltilebilir · Onar". Yalnızca sessiz değişiklik varsa bildirim yok.
pub fn notice(reports: &[HealReport]) -> Option<HealNotice> {
    let sum = |class: Class| reports.iter().map(|r| r.count(class)).sum::<usize>();
    let (repaired, reset, cleaned) = (sum(Class::Repaired), sum(Class::Reset), sum(Class::Cleaned));
    let total = repaired + reset + cleaned;
    if total == 0 {
        return None;
    }
    let written = reports.iter().any(|r| r.written.is_some() && r.notable() > 0);
    let mut parts = Vec::new();
    if cleaned > 0 {
        parts.push(t!("shell-config-heal-part-cleaned", count = cleaned));
    }
    if reset > 0 {
        parts.push(t!("shell-config-heal-part-reset", count = reset));
    }
    if repaired > 0 {
        parts.push(t!("shell-config-heal-part-repaired", count = repaired));
    }
    let only_cleaned = total == cleaned;
    let (summary, action) = match (written, only_cleaned) {
        (true, true) => (t!("shell-config-healed-cleaned-title", count = cleaned), (t!("shell-config-heal-undo"), UNDO_COMMAND)),
        (true, false) => (t!("shell-config-healed-title", count = total), (t!("shell-config-heal-undo"), UNDO_COMMAND)),
        (false, _) => (t!("shell-config-heal-pending-title", count = total), (t!("shell-config-heal-fix"), HEAL_COMMAND)),
    };
    let body = if only_cleaned && written { String::new() } else { parts.join(" · ") };
    Some(HealNotice { summary, body, actions: vec![action] })
}

/// Yazım hatası önerileri bildirimi.
pub fn suggestion_notice(reports: &[HealReport]) -> Option<HealNotice> {
    let all: Vec<(String, String)> = reports.iter().flat_map(HealReport::suggestions).collect();
    if all.is_empty() {
        return None;
    }
    let mut body: Vec<String> = all
        .iter()
        .take(MAX_NOTICE_SUGGESTIONS)
        .map(|(key, cand)| t!("shell-config-heal-typo-item", key = key.clone(), candidate = cand.clone()))
        .collect();
    if all.len() > MAX_NOTICE_SUGGESTIONS {
        body.push("…".into());
    }
    Some(HealNotice {
        summary: t!("shell-config-heal-typo-title", count = all.len()),
        body: body.join(", "),
        actions: vec![
            (t!("shell-config-heal-typo-fix"), FIX_TYPOS_COMMAND),
            (t!("shell-config-heal-typo-ignore"), IGNORE_TYPOS_COMMAND),
        ],
    })
}

// --- koru listesi -------------------------------------------------------------------------

fn keep_path(env: &Env) -> Option<PathBuf> {
    Some(env.state_home.as_ref()?.join("nuxut/config-heal.toml"))
}

/// Dosyanın koru listesi (okunamazsa boş).
pub fn keep_set(env: &Env, file: &str) -> BTreeSet<String> {
    let Some(path) = keep_path(env) else { return BTreeSet::new() };
    read_keep(&path).and_then(|doc| keep_of(&doc, file)).unwrap_or_default()
}

fn read_keep(path: &Path) -> Option<toml::Table> {
    let f = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    f.take(MAX_KEEP_BYTES).read_to_string(&mut text).ok()?;
    text.parse().ok()
}

fn keep_of(doc: &toml::Table, file: &str) -> Option<BTreeSet<String>> {
    let list = doc.get("keep")?.get(file)?.as_array()?;
    Some(list.iter().filter_map(|v| v.as_str()).filter(|s| s.len() <= 512).take(MAX_KEEP).map(str::to_owned).collect())
}

/// Koru listesine ekler (ortam kapısından, atomik).
pub fn add_keep(env: &Env, file: &str, keys: &[String]) -> Result<(), String> {
    if keys.is_empty() {
        return Ok(());
    }
    let path = keep_path(env).ok_or("durum dizini yok")?;
    let (current, expect) = match nuxut_apply::read_optional(&path) {
        Ok(Some((t, e))) => (t, e),
        Ok(None) => (String::new(), Expect::Absent),
        Err(e) => return Err(e.kind_name().to_owned()),
    };
    let mut doc: toml::Table = current.parse().unwrap_or_default();
    let mut set = keep_of(&doc, file).unwrap_or_default();
    set.extend(keys.iter().cloned());
    let list: Vec<toml::Value> = set.into_iter().take(MAX_KEEP).map(toml::Value::String).collect();
    let keep = doc.entry("keep").or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if !keep.is_table() {
        *keep = toml::Value::Table(toml::Table::new());
    }
    if let Some(t) = keep.as_table_mut() {
        t.insert(file.to_owned(), toml::Value::Array(list));
    }
    let header = diag::KEEP_HEADER;
    let text = format!("{header}{}", toml::to_string(&doc).map_err(|e| e.to_string())?);
    nuxut_apply::write_file(env, &path, text.as_bytes(), Some(&expect)).map_err(|e| e.kind_name().to_owned())?;
    Ok(())
}

// --- geri al -------------------------------------------------------------------------------

/// Onarımı geri alır: dosya hâlâ onarımın yazdığı metindeyse onarım öncesi metin yazılır.
pub fn undo(env: &Env, w: &HealWritten) -> Result<(), String> {
    let (current, expect) = nuxut_apply::read_file(&w.path).map_err(|e| e.kind_name().to_owned())?;
    if current != w.written {
        return Err(format!("{} onarımdan sonra değişti; yedek: {}", w.path.display(), w.backup.display()));
    }
    nuxut_apply::write_file(env, &w.path, w.restore.as_bytes(), Some(&expect)).map_err(|e| e.kind_name().to_owned())?;
    Ok(())
}

// --- kalıcı geri al kaydı -----------------------------------------------------------------
//
// Son yazılmış onarımın geri al kaydı `$XDG_STATE_HOME/nuxut/config-heal-undo.toml`'da da durur:
// `msg config undo` kabuk yeniden başladıktan sonra da (güncelleme, oturum) çalışır. Kayıt geri
// alınca boşalır; dosya değiştiyse geri al reddedilir (yedeğin yolu söylenir).

/// Kayıt dosyasının boyut sınırı (iki tam metin taşır).
const MAX_UNDO_BYTES: u64 = 4 * MAX_CONFIG_BYTES;

fn undo_path(env: &Env) -> Option<PathBuf> {
    Some(env.state_home.as_ref()?.join("nuxut/config-heal-undo.toml"))
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct StoredUndo {
    #[serde(default)]
    entries: Vec<StoredEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredEntry {
    file: String,
    path: PathBuf,
    backup: PathBuf,
    restore: String,
    written: String,
    #[serde(default)]
    keep: Vec<String>,
}

fn known_file(name: &str) -> Option<&'static str> {
    [SHELL_FILE, FRAMEWORK_FILE].into_iter().find(|f| *f == name)
}

/// Onarılan dosyanın gerçek yolu (`~/.config/nuxut/<dosya>`).
pub fn file_path(file: &str) -> Option<PathBuf> {
    match file {
        SHELL_FILE => nuxut_config::config_path(crate::config::CONFIG_NAME),
        FRAMEWORK_FILE => nuxut_config::config_path("framework"),
        _ => None,
    }
}

/// Geri al kaydını diske yazar (boş liste: kaydı temizler).
pub fn save_undo(env: &Env, entries: &[UndoEntry]) -> Result<(), String> {
    let path = undo_path(env).ok_or("durum dizini yok")?;
    let stored = StoredUndo {
        entries: entries
            .iter()
            .map(|e| StoredEntry {
                file: e.file.to_owned(),
                path: e.written.path.clone(),
                backup: e.written.backup.clone(),
                restore: e.written.restore.clone(),
                written: e.written.written.clone(),
                keep: e.keep.clone(),
            })
            .collect(),
    };
    let text = format!("{}{}", diag::UNDO_HEADER, toml::to_string(&stored).map_err(|e| e.to_string())?);
    let expect = match nuxut_apply::read_optional(&path) {
        Ok(Some((_, e))) => e,
        Ok(None) => Expect::Absent,
        Err(e) => return Err(e.kind_name().to_owned()),
    };
    nuxut_apply::write_file_with(env, &path, text.as_bytes(), Some(&expect), &WriteOptions::PRIVATE)
        .map_err(|e| e.kind_name().to_owned())?;
    Ok(())
}

/// Diskteki geri al kaydı (yoksa, bozuksa ya da sınırı aşıyorsa boş).
pub fn load_undo(env: &Env) -> Vec<UndoEntry> {
    let Some(path) = undo_path(env) else { return Vec::new() };
    let Ok(f) = std::fs::File::open(&path) else { return Vec::new() };
    let mut text = String::new();
    if f.take(MAX_UNDO_BYTES + 1).read_to_string(&mut text).is_err() || text.len() as u64 > MAX_UNDO_BYTES {
        tracing::warn!("{}: okunamadı ya da çok büyük; yok sayılıyor", path.display());
        return Vec::new();
    }
    let stored: StoredUndo = match toml::from_str(&text) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("{}: bozuk ({e}); yok sayılıyor", path.display());
            return Vec::new();
        }
    };
    stored
        .entries
        .into_iter()
        .filter_map(|e| {
            Some(UndoEntry {
                file: known_file(&e.file)?,
                written: HealWritten { path: e.path, backup: e.backup, restore: e.restore, written: e.written },
                keep: e.keep.into_iter().filter(|k| k.len() <= 512).take(MAX_KEEP).collect(),
            })
        })
        .collect()
}

// --- framework.toml -------------------------------------------------------------------------

/// framework.toml'un sözdizimi onarımı (açılışta, asıl örnek). Şeması framework'e ait olduğu için
/// anlamsal adım yok; bozuk satır yoruma alınır, okunabilen ayarlar kalır.
pub fn heal_framework(env: &Env, write: bool) -> Option<HealReport> {
    let path = nuxut_config::config_path("framework")?;
    let (raw, lossy) = read_lossy(&path).ok()??;
    let healed = nuxut_config_heal::repair_syntax(&raw, &labels());
    let mut actions = healed.actions;
    if lossy {
        actions.insert(0, Action { key: String::new(), kind: ActionKind::Broken, line: 0 });
    }
    if actions.is_empty() {
        return None;
    }
    let written = if write { write_healed(env, &path, FRAMEWORK_FILE, &raw, &healed.text, raw.clone()) } else { None };
    Some(HealReport { file: FRAMEWORK_FILE, actions, written })
}

/// Yedek + atomik yazım (dosya hâlâ `raw` ise). Başarısızsa `None` (onarım bellekte kalır).
pub fn write_healed(env: &Env, path: &Path, name: &str, raw: &str, new: &str, restore: String) -> Option<HealWritten> {
    let dir = backup_dir(env)?;
    let (current, expect) = nuxut_apply::read_file(path).ok()?;
    if current != raw {
        return None;
    }
    let backup = match write_backup(env, &dir, name, raw.as_bytes()) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("{name} onarımı yazılmadı: yedek alınamadı ({e})");
            return None;
        }
    };
    match nuxut_apply::write_file(env, path, new.as_bytes(), Some(&expect)) {
        Ok(_) => Some(HealWritten { path: path.to_owned(), backup, restore, written: new.to_owned() }),
        Err(e) => {
            tracing::warn!("{name} onarımı yazılamadı ({})", e.kind_name());
            None
        }
    }
}

// --- süreç durumu ---------------------------------------------------------------------------

/// Yazılmış bir onarımın geri al kaydı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoEntry {
    pub file: &'static str,
    pub written: HealWritten,
    /// Geri alınınca koru listesine girecek anahtarlar.
    pub keep: Vec<String>,
}

/// Onarım bildirimlerinin ve geri al kaydının süreç içi durumu (UI'dan bağımsız).
#[derive(Debug, Default)]
pub struct HealState {
    undo: Vec<UndoEntry>,
    /// Son bekleyen-onarım bildiriminin parmak izi (aynı sorunlar için yeniden bildirilmez).
    pending_seen: Option<String>,
    typos_seen: Option<String>,
    suggestions: Vec<(String, String)>,
    /// Geri al'dan sonraki ilk yükleme bekleyen-onarım bildirimi vermez (kullanıcı az önce istedi).
    quiet_once: bool,
}

impl HealState {
    /// Bir yüklemenin raporları → gösterilecek bildirimler (en çok iki: onarım, öneri).
    pub fn observe(&mut self, reports: &[HealReport]) -> Vec<HealNotice> {
        let mut out = Vec::new();
        let written: Vec<UndoEntry> = reports
            .iter()
            .filter_map(|r| r.written.clone().map(|w| UndoEntry { file: r.file, written: w, keep: r.keepable() }))
            .collect();
        let wrote_notable = reports.iter().any(|r| r.written.is_some() && r.notable() > 0);
        if !written.is_empty() {
            self.undo = written;
        }
        let fingerprint: String = reports.iter().map(HealReport::fingerprint).collect::<Vec<_>>().join(";");
        let quiet = std::mem::take(&mut self.quiet_once);
        if let Some(n) = notice(reports)
            && (wrote_notable || (!quiet && self.pending_seen.as_deref() != Some(fingerprint.as_str())))
        {
            out.push(n);
        }
        self.pending_seen = (!wrote_notable).then_some(fingerprint);
        self.suggestions = reports.iter().flat_map(HealReport::suggestions).collect();
        let typos = format!("{:?}", self.suggestions);
        if self.typos_seen.as_deref() != Some(typos.as_str())
            && let Some(n) = suggestion_notice(reports)
        {
            out.push(n);
        }
        self.typos_seen = Some(typos);
        out
    }

    /// Son yazılmış onarımlar (geri al); alınınca kayıt boşalır ve sonraki yükleme sessizdir.
    pub fn take_undo(&mut self) -> Vec<UndoEntry> {
        let v = std::mem::take(&mut self.undo);
        if !v.is_empty() {
            self.quiet_once = true;
        }
        v
    }

    /// Geri alınamadıysa kaydı geri koyar.
    pub fn restore_undo(&mut self, entries: Vec<UndoEntry>) {
        self.undo = entries;
        self.quiet_once = false;
    }

    pub fn has_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Bellekteki geri al kaydı (diske [`save_undo`] ile yazılır).
    pub fn undo_entries(&self) -> &[UndoEntry] {
        &self.undo
    }

    /// Son yüklemenin yazım hatası önerileri (yol, aday).
    pub fn suggestions(&self) -> &[(String, String)] {
        &self.suggestions
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config_migrate::{CURRENT_CONFIG_VERSION, MIGRATIONS};

    fn labels() -> Labels {
        Labels {
            stamp: "2026-10-16".into(),
            broken: "okunamadı".into(),
            duplicate: "tekrar".into(),
            invalid: "geçersiz".into(),
            clamped: "sınır dışı".into(),
        }
    }

    fn heal(text: &str) -> Healed {
        let h = heal_shell(text, &BTreeSet::new(), &labels(), &[]);
        let again = heal_shell(&h.text, &BTreeSet::new(), &labels(), &[]);
        assert!(!again.edited(), "idempotent değil:\n{}\n{:?}", h.text, again.actions);
        h
    }

    // --- şema bekçisi ----------------------------------------------------------------------

    const SNAPSHOT_HEADER: &str = "# shell.toml şeması (Deserialize'dan çıkarılır; elle düzenlenmez).\n\
        # Güncellemek: NUXUT_UPDATE_CONFIG_SCHEMA=1 cargo test config_schema_snapshot\n\
        # retired / renamed satırları kaldırılmış anahtarların kaydıdır (config_heal RETIRED/RENAMES).\n";

    fn snapshot_path(version: u32) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("docs/config-schema/shell-v{version}.txt"))
    }

    fn parse_snapshot(text: &str) -> BTreeMap<String, String> {
        text.lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .filter_map(|l| l.split_once(' ').map(|(k, v)| (k.to_owned(), v.to_owned())))
            .collect()
    }

    fn covered(path: &str) -> bool {
        let hit = |p: &str| path == p || path.starts_with(&format!("{p}."));
        RETIRED.iter().any(|r| hit(r)) || RENAMES.iter().any(|(from, _)| hit(from))
    }

    /// `enum(a|b)` → `enum(a|b|c)`: yalnızca varyant eklendi (eski değerler geçerli).
    fn enum_extended(old: &str, new: &str) -> bool {
        let variants = |s: &str| -> Option<Vec<String>> {
            Some(s.strip_prefix("enum(")?.strip_suffix(')')?.split('|').map(str::to_owned).collect())
        };
        matches!((variants(old), variants(new)), (Some(o), Some(n)) if o.iter().all(|v| n.contains(v)))
    }

    /// Şema anlık görüntüsü (`docs/config-schema/shell-v<sürüm>.txt`) ile karşılaştırma: kaldırılan
    /// anahtar `RETIRED`/`RENAMES`'te olmalı; tür değişikliği (enum'a varyant eklemek dışında)
    /// `config_version` artışı + `MIGRATIONS` adımı ister; yeni anahtar anlık görüntüye işlenir.
    #[test]
    fn config_schema_snapshot() {
        let current: BTreeMap<String, String> = rules().schema.iter().map(|(k, v)| (k.to_owned(), v.to_string())).collect();
        let mut expected = current.clone();
        for r in RETIRED {
            expected.insert((*r).to_owned(), "retired".into());
        }
        for (from, to) in RENAMES {
            expected.insert((*from).to_owned(), format!("renamed {to}"));
        }
        let rendered: String =
            format!("{SNAPSHOT_HEADER}{}", expected.iter().map(|(k, v)| format!("{k} {v}\n")).collect::<String>());
        let path = snapshot_path(CURRENT_CONFIG_VERSION);
        let on_disk = std::fs::read_to_string(&path).ok();
        // Bu sürümün dosyası yoksa (sürüm yeni artırıldı) önceki sürümle karşılaştırılır: tür
        // değişikliği serbesttir (göç adımı şart), kaldırılanlar yine kayıtlı olmalı.
        let (old, new_version) = match &on_disk {
            Some(t) => (Some(t.clone()), false),
            None => (std::fs::read_to_string(snapshot_path(CURRENT_CONFIG_VERSION - 1)).ok(), true),
        };
        let mut errors = Vec::new();
        if new_version && !MIGRATIONS.iter().any(|s| s.to == CURRENT_CONFIG_VERSION) {
            errors.push(format!("config_version {CURRENT_CONFIG_VERSION}: MIGRATIONS'ta bu sürüme adım yok"));
        }
        if let Some(old) = &old {
            for (key, kind) in parse_snapshot(old) {
                if kind == "retired" || kind.starts_with("renamed ") {
                    if !covered(&key) {
                        errors.push(format!("{key}: RETIRED/RENAMES'ten çıkarılmış (yalnızca eklenir)"));
                    }
                    continue;
                }
                match current.get(&key) {
                    None if !covered(&key) => errors.push(format!(
                        "{key} şemadan kalktı: config_heal RETIRED'a (kaldırıldıysa) ya da RENAMES'e (adı değiştiyse) ekle"
                    )),
                    Some(k) if *k != kind && !new_version && !enum_extended(&kind, k) => errors.push(format!(
                        "{key} türü değişti ({kind} → {k}): config_version'ı artır ve MIGRATIONS'a dönüştürme adımı ekle"
                    )),
                    _ => {}
                }
            }
        }
        if errors.is_empty() && on_disk.as_deref() != Some(rendered.as_str()) {
            if std::env::var_os("NUXUT_UPDATE_CONFIG_SCHEMA").is_some() {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &rendered).unwrap();
            } else {
                errors.push(format!(
                    "{} güncel değil (yeni anahtar?). Güncelle: NUXUT_UPDATE_CONFIG_SCHEMA=1 cargo test config_schema_snapshot",
                    path.display()
                ));
            }
        }
        assert!(errors.is_empty(), "shell.toml şema bekçisi:\n  {}", errors.join("\n  "));
    }

    #[test]
    fn renames_and_retired_point_the_right_way() {
        let s = &rules().schema;
        for (from, to) in RENAMES {
            let parts = |p: &str| p.split('.').map(str::to_owned).collect::<Vec<_>>();
            assert!(s.resolve(&parts(from)).is_none(), "{from} hâlâ şemada");
            assert!(s.resolve(&parts(to)).is_some(), "{to} şemada yok");
        }
        for r in RETIRED {
            assert!(s.get(r).is_none(), "{r} hâlâ şemada");
        }
        // Ayarlar şemasının her genel anahtarı serde şemasında da var (varsayılan/aralık eşlemesi;
        // serbest tablolardaki anahtarlar — `input.keyboard.*` — `*` ile çözülür).
        for key in rules().defaults.keys() {
            let parts: Vec<String> = key.split('.').map(str::to_owned).collect();
            assert!(s.resolve(&parts).is_some(), "Ayarlar şemasındaki {key} serde şemasında yok");
        }
    }

    // --- gerçek dosyalar ---------------------------------------------------------------------

    /// 2026-10-04 belgelenmiş örnek: v1 → v2 göçü + onarım. Yorumlu satırlar kalır.
    #[test]
    fn real_old_documented_file_loads_clean() {
        let (_t, env, path) = fixture();
        let raw = include_str!("../../tests/fixtures/config/doc-2026-10-04.toml");
        std::fs::write(&path, raw).unwrap();
        let loaded = ShellConfig::load_from(&path, &env).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# 2026-10-04"), "{text}");
        assert!(!text.contains("[bar.widget.workspaces]"), "göçün boşalttığı başlık gider:\n{text}");
        assert!(!text.ends_with("\n\n"), "{text}");
        assert!(text.contains("config_version = 2"), "{text}");
        assert!(text.contains("start    = [\"workspaces\"]"), "{text}");
        assert!(!text.contains("inner_radius"), "{text}");
        assert!(!text.contains("show_empty"), "{text}");
        assert!(text.contains("thickness    = 6.0      # tüm kenarlar"), "yorumlu varsayılan kalır:\n{text}");
        assert!(!text.contains("enabled      = true\n"), "yorumsuz varsayılan gider:\n{text}");
        // Yük sonrası uyarı yok; ikinci açılış hiçbir şey yazmaz.
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let again = ShellConfig::load_from(&path, &env).unwrap();
        assert!(again.heal.as_ref().is_none_or(|h| !h.actions.iter().any(|a| a.kind.edits())), "{:?}", again.heal);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert_eq!(again.config, loaded.config);
    }

    /// 2026-10-10 belgelenmiş tam örnek (her satır yorumlu): değer değişmez, yer tutucu tablo
    /// (`[notifications.apps."<app_name>"]`) harita girdisi olarak kalır.
    #[test]
    fn real_full_documented_file_keeps_commented_values() {
        let raw = include_str!("../../tests/fixtures/config/doc-2026-10-10.toml");
        let crate::config_migrate::Migration::Migrated(m) = crate::config_migrate::migrate(raw) else { panic!("göç beklenirdi") };
        let h = heal(&m.text);
        let lost: Vec<_> = h.actions.iter().filter(|a| a.kind.edits() && a.kind != ActionKind::Redundant).collect();
        assert!(lost.is_empty(), "{lost:?}");
        let (before, _) = ShellConfig::parse(&m.text).unwrap();
        let (after, _) = ShellConfig::parse(&h.text).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn broken_file_is_backed_up_repaired_and_undoable() {
        crate::i18n::use_turkish_for_tests();
        let (_t, env, path) = fixture();
        let raw = "config_version = 2\n[bar\nsize = 40\nspacnig = 3\n[frame]\nthickness = \"kalın\"\ninner_radius = 3\n[osd]\ntimeout_ms = 999999\n";
        std::fs::write(&path, raw).unwrap();
        let loaded = ShellConfig::load_from(&path, &env).unwrap();
        assert_eq!(loaded.config.bar.size, 40.0, "okunabilen kısım kurtarıldı");
        let report = loaded.heal.expect("onarım raporu");
        let written = report.written.clone().expect("yazıldı");
        assert_eq!(std::fs::read_to_string(&written.backup).unwrap(), raw, "önce yedek");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[bar]\nsize = 40\n"), "{text}");
        assert!(text.contains("spacnig = 3"), "öneri: dokunulmaz\n{text}");
        assert!(text.contains("# thickness = \"kalın\""), "{text}");
        assert!(!text.contains("inner_radius"), "{text}");
        assert!(text.contains("timeout_ms = 10000"), "{text}");
        assert_eq!(report.suggestions(), [("bar.spacnig".to_owned(), "spacing".to_owned())]);
        // Bildirimler: yazılmış onarım (Geri al) + öneri (Düzelt / Yok say).
        crate::i18n::use_turkish_for_tests();
        let mut state = HealState::default();
        let notices = state.observe(std::slice::from_ref(&report));
        assert_eq!(notices.len(), 2, "{notices:?}");
        assert_eq!(notices[0].actions[0].1, UNDO_COMMAND);
        assert_eq!(notices[1].actions.iter().map(|a| a.1).collect::<Vec<_>>(), [FIX_TYPOS_COMMAND, IGNORE_TYPOS_COMMAND]);
        // Geri al: dosya eski hâline, onarımın sildiği anahtar korunur.
        let entries = state.take_undo();
        assert_eq!(entries.len(), 1);
        undo(&env, &entries[0].written).unwrap();
        add_keep(&env, SHELL_FILE, &entries[0].keep).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        assert!(keep_set(&env, SHELL_FILE).contains("frame.inner_radius"));
        // Sonraki açılış korunan anahtara dokunmaz (bozuk satır ve geçersiz değer yine onarılır).
        let heal_opts = crate::config::HealOptions { keep: keep_set(&env, SHELL_FILE), write: Some(&env), ..Default::default() };
        let w = crate::config_migrate::ApplyConfigWriter::new(env.clone());
        ShellConfig::load_full(&path, Some(&w), false, &heal_opts).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("inner_radius = 3"), "{text}");
        assert!(text.contains("[bar]\n"), "{text}");
    }

    /// Geri al kaydı diskte de durur: kabuk yeniden başladıktan sonra `config undo` çalışır;
    /// geri alınca kayıt boşalır.
    #[test]
    fn undo_record_survives_restart() {
        let (_t, env, path) = fixture();
        let raw = "config_version = 2\n# not\n[frame]\ninner_radius = 3\n";
        std::fs::write(&path, raw).unwrap();
        let report = ShellConfig::load_from(&path, &env).unwrap().heal.unwrap();
        let mut state = HealState::default();
        state.observe(std::slice::from_ref(&report));
        save_undo(&env, state.undo_entries()).unwrap();
        let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(undo_path(&env).unwrap()).unwrap().permissions());
        assert_eq!(mode & 0o777, 0o600);
        // "Yeniden başlatma": bellek boş, kayıt diskten.
        let loaded = load_undo(&env);
        assert_eq!(loaded, state.undo_entries());
        undo(&env, &loaded[0].written).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        save_undo(&env, &[]).unwrap();
        assert!(load_undo(&env).is_empty());
    }

    #[test]
    fn broken_undo_record_is_ignored() {
        let (_t, env, _path) = fixture();
        let p = undo_path(&env).unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        for junk in ["[[entries]]\nfile = 3\n", "\u{0}\u{1}garbage", "[[entries]]\nfile = \"evil.toml\"\npath = \"/etc/x\"\nbackup = \"/b\"\nrestore = \"\"\nwritten = \"\"\n"] {
            std::fs::write(&p, junk).unwrap();
            assert!(load_undo(&env).is_empty(), "{junk:?}");
        }
        std::fs::write(&p, "a".repeat(MAX_UNDO_BYTES as usize + 10)).unwrap();
        assert!(load_undo(&env).is_empty());
    }

    /// `config status` kuru çalışır: bekleyen onarımı ve göçü listeler, dosyaya dokunmaz.
    #[test]
    fn status_is_read_only_and_lists_pending() {
        let (_t, env, path) = fixture();
        let raw = "[workspaces]\ncount = 5\nshow_empty = true\n[bar]\nsiez = 3\nedge = 9\n";
        std::fs::write(&path, raw).unwrap();
        let s = diag::shell_status(&env, &path, false);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw, "salt okunur");
        assert_eq!(s["version"], serde_json::Value::Null, "{s}");
        assert_eq!(s["migration"]["to"], CURRENT_CONFIG_VERSION, "{s}");
        let kinds: Vec<&str> = s["pending"].as_array().unwrap().iter().map(|a| a["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"invalid"), "{s}");
        assert_eq!(s["suggestions"][0]["candidate"], "size", "{s}");
        assert_eq!(s["backups"]["count"], 0);
    }

    #[test]
    fn undo_refuses_when_file_changed_after_heal() {
        let (_t, env, path) = fixture();
        std::fs::write(&path, "config_version = 2\n[frame]\ninner_radius = 3\n").unwrap();
        let w = ShellConfig::load_from(&path, &env).unwrap().heal.unwrap().written.unwrap();
        std::fs::write(&path, "config_version = 2\n# elle\n").unwrap();
        let err = undo(&env, &w).unwrap_err();
        assert!(err.contains("değişti"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "config_version = 2\n# elle\n");
    }

    /// Dosya değişince (yazım yok) onarım bellekte: geçersiz değer bütün dosyayı düşürmez, dosya
    /// aynen kalır, bildirim "Onar" ister ve aynı sorun için tekrarlanmaz.
    #[test]
    fn watch_reload_heals_in_memory_only() {
        crate::i18n::use_turkish_for_tests();
        let (_t, _env, path) = fixture();
        let raw = "config_version = 2\n[bar]\nsize = 40\nedge = 5\n";
        std::fs::write(&path, raw).unwrap();
        let loaded = ShellConfig::load_with(&path, None, false).unwrap();
        assert_eq!(loaded.config.bar.size, 40.0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        let report = loaded.heal.unwrap();
        assert!(report.written.is_none());
        let mut state = HealState::default();
        let n = state.observe(std::slice::from_ref(&report));
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].actions[0].1, HEAL_COMMAND);
        assert!(state.observe(std::slice::from_ref(&report)).is_empty(), "aynı sorun yeniden bildirilmez");
    }

    #[test]
    fn only_redundant_changes_are_silent() {
        crate::i18n::use_turkish_for_tests();
        let h = heal("config_version = 2\n[bar]\nsize = 34\n");
        assert_eq!(h.text, "config_version = 2\n");
        let report = HealReport { file: SHELL_FILE, actions: h.actions, written: None };
        assert!(notice(&[report]).is_none());
    }

    #[test]
    fn notice_texts() {
        crate::i18n::use_turkish_for_tests();
        let a = |kind| Action { key: "x".into(), kind, line: 1 };
        let w = HealWritten { path: "/x".into(), backup: "/b".into(), restore: String::new(), written: String::new() };
        let r = HealReport { file: SHELL_FILE, actions: vec![a(ActionKind::Unused), a(ActionKind::Retired), a(ActionKind::Unused)], written: Some(w.clone()) };
        let n = notice(&[r]).unwrap();
        assert_eq!(n.summary, "Ayarlarında 3 eski satır temizlendi");
        assert_eq!(n.actions[0].0, "Geri al");
        let r = HealReport { file: SHELL_FILE, actions: vec![a(ActionKind::Unused), a(ActionKind::Invalid), a(ActionKind::Broken)], written: Some(w) };
        let n = notice(&[r]).unwrap();
        assert_eq!(n.summary, "Ayarlarında 3 satır düzeltildi");
        assert_eq!(n.body, "1 eski satır temizlendi · 1 geçersiz değer varsayılana döndü · 1 bozuk satır yoruma alındı");
    }

    #[test]
    fn heal_is_fast_enough_for_startup() {
        let raw = include_str!("../../tests/fixtures/config/doc-2026-10-10.toml");
        let t = std::time::Instant::now();
        let _ = heal_shell(raw, &BTreeSet::new(), &labels(), &[]);
        // Hata ayıklama derlemesinde bile açılışı belirgin geciktirmemeli.
        assert!(t.elapsed() < std::time::Duration::from_millis(1500), "{:?}", t.elapsed());
    }

    /// Geçici kök + test ortamı + `<kök>/config/nuxut/shell.toml` (kapı yalnızca kökü açar).
    fn fixture() -> (tempfile::TempDir, Env, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let env = Env::for_test(t.path());
        let dir = t.path().join("config/nuxut");
        std::fs::create_dir_all(&dir).unwrap();
        (t, env, dir.join("shell.toml"))
    }
}
