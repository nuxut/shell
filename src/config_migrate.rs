//! `shell.toml` sürüm göçü (ADR-0002, `docs/adr/0002-config-version-migration.md`).
//!
//! - Dosyanın üstündeki `config_version` tam sayısı; yoksa **1**. Kodun sürümü
//!   [`CURRENT_CONFIG_VERSION`].
//! - [`MIGRATIONS`]: sıralı, **saf** (yalnızca belge; ortam/saat/dosya sistemi yok) ve
//!   **koşullu** adımlar. `toml_edit` ile çalışır: dokunulmayan her şey bayt bayt aynı kalır,
//!   taşınan anahtar yorumunu ve yerini korur.
//! - İdempotentlik çalıştırıcı düzeyindedir ([`migrate`]): bir adım yalnızca sürümü kendi
//!   `to`'sundan küçük belgeye uygulanır, göçen belgeye `config_version` yazılır; ikinci çalışma
//!   [`Migration::Current`] döner. (Adımların bazıları — ör. `workspaces-page-count` — v2 dosyada
//!   v1 anlamıyla okunamayacağı için tek başına idempotent değildir; bu yüzden sürüm damgası
//!   zorunludur.)
//! - Var olan sürümsüz dosyaya hiçbir adım bir şey değiştirmese de shell bir kez
//!   `config_version` damgası yazar ([`stamp_version`]; karar). Yoksa Ayarlar'ın ya da
//!   kullanıcının sonradan v2 anlamıyla yazdığı bir anahtar (ör. `[bar.widget.workspaces]
//!   count = 15`) bir sonraki açılışta v1 sayılıp göç ederdi. Damga yazılamazsa (salt okunur
//!   dosya, ikincil örnek) dosya o oturum boyunca bellekte güncel sürüm sayılır.
//! - Dosya koddan yeniyse (sürüm düşürme) göç çalışmaz, dosya yazılmaz.
//! - Yazım ince bir arayüzün ([`ConfigWriter`]) arkasındadır; uygulaması [`ApplyConfigWriter`]
//!   (framework `nuxut-apply`, ADR-0001): tek atomik yazıcı (symlink hedefine, izin korunur,
//!   dışarıdan yönetilen/salt okunur dosyaya yazılmaz, `rename`'den hemen önce dış değişim
//!   denetimi), ortam kapısı ve göçte snapshot (`migration.shell`).
//! - Diske yalnızca tek-örnek kilidini almış asıl shell yazar ([`WritePolicy`]); ikinci çağrı ve
//!   `NUXUT_SHELL_INSTANCE`'lı test örneği göçü yalnızca bellekte uygular. (Framework'ün ortam
//!   kapısı ayrıca test örneğinin kullanıcının dosyalarına yazmasını engeller; buradaki kural
//!   shell'e özgü kısımdır: kilit sahibi varsayılan örnek.)
//! - Geri al ([`MigrationUndo`]): yalnızca kullanıcının değerini değiştiren (`Transform`/`Drop`)
//!   anahtarlara, eski değeri **yeni anlamıyla** geri koyar; `config_version` geri alınmaz
//!   (yoksa bir sonraki açılış yeniden göç ederdi). Göç snapshot'ı bu anlamla kurulur
//!   ([`ApplyConfigWriter`]): parçalar yalnızca geri alınabilir anahtarlardır, "önce" değerleri
//!   yeni anlamlı eski değerler. Böylece `config undo-migration` = o snapshot'ın geri yüklenmesi
//!   (`snapshot restore <id>` ile aynı). Bildirim kaydı süreç belleğindedir.

use std::fmt;
use std::io::Read;
use std::path::Path;

use nuxut_apply::{
    Applier, ApplyError, Change, Env, Expect, FragValue, Fragment, Outcome, RestoreMode, RestoreOutcome, SnapshotId, Source,
};
use nuxut_config::{ConfigError, MAX_CONFIG_BYTES};
use toml_edit::{DocumentMut, InlineTable, Item, Key, Table, TableLike, Value};

use nuxut::i18n::t;

/// Bu kodun yazdığı/okuduğu `shell.toml` sürümü.
pub const CURRENT_CONFIG_VERSION: u32 = 2;
/// Sürüm anahtarı (dosyanın kökünde).
pub const VERSION_KEY: &str = "config_version";
/// Bildirim gövdesinde en çok bu kadar ayar adı (fazlası "…").
const MAX_NOTICE_NAMES: usize = 3;
/// v2 workspace sayfa boyu üst sınırı. Göç adımları dondurulmuştur: sonradan değişen
/// `MAX_WORKSPACE_COUNT`'u izlemez, v1 → v2 anlamını taşır.
const V2_MAX_WORKSPACE_PAGE: i64 = 30;

/// Noktalı anahtar yolu (`bar.widget.workspaces.count`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPath(Vec<String>);

impl KeyPath {
    pub fn new(parts: &[&str]) -> Self {
        Self(parts.iter().map(|&p| p.to_owned()).collect())
    }

    pub fn parts(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("."))
    }
}

/// Bir göç değişikliğinin türü: kullanıcıya bildirilip bildirilmeyeceğini belirler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditKind {
    /// Değer aynen yeni anahtara taşındı — sessiz.
    Rename { from: KeyPath },
    /// Zaten etkisiz olan bir değer kaldırıldı — sessiz.
    DropDead,
    /// Kullanıcının değeri değişti (birim, anlam, sınır) — bildirim.
    Transform,
    /// Etkili bir değer kaldırıldı — bildirim.
    Drop,
}

impl EditKind {
    /// Kullanıcıya bildirilir mi (davranış değişti mi)?
    pub fn notifies(&self) -> bool {
        matches!(self, Self::Transform | Self::Drop)
    }
}

/// Geri al: `key`'e `value` (TOML değer yazımı, ör. `12`) yeni anlamıyla yazılır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restore {
    pub key: KeyPath,
    pub value: String,
}

/// Göçün bir anahtara yaptığı değişiklik.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Değişiklikten sonraki anahtar (silinmişse silinen anahtar).
    pub key: KeyPath,
    pub kind: EditKind,
    /// Kullanıcıya gösterilen ayar adı (bildirim gövdesi, Ayarlar "Bu sürümde yeni").
    pub note: String,
    /// Yalnızca bildirilen değişikliklerde: geri alınca ne yazılacağı.
    pub restore: Option<Restore>,
}

/// Bir göç adımı. `run` saf ve koşulludur (eski biçim yoksa hiçbir şey yapmaz).
#[derive(Debug)]
pub struct Step {
    /// Bu adımdan sonra belgenin sürümü.
    pub to: u32,
    pub name: &'static str,
    pub run: fn(&mut DocumentMut) -> Vec<Edit>,
}

/// Sıralı göç adımları. Yeni adım sona eklenir; eklenmiş adım değiştirilmez.
pub const MIGRATIONS: &[Step] = &[
    Step { to: 2, name: "bar-start-end", run: bar_start_end },
    Step { to: 2, name: "widget-tables", run: widget_tables },
    Step { to: 2, name: "frame-inner-radius", run: frame_inner_radius },
    Step { to: 2, name: "workspaces-page-count", run: workspaces_page_count },
];

/// [`migrate`] sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migration {
    /// Sürüm güncel: yapılacak bir şey yok.
    Current,
    /// Dosya bu koddan yeni bir sürüme ait: göç yok, dosya yazılmaz.
    Newer(u32),
    /// Eski sürüm ama hiçbir adım bir şey değiştirmedi: dosya yazılmaz.
    Unchanged { from: u32 },
    Migrated(Migrated),
    /// `toml_edit` ayrıştıramadı: göç yok; olağan ayrıştırma hatayı bildirir.
    Unparsable,
}

/// Göçmüş metin ve değişiklikler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migrated {
    pub from: u32,
    pub to: u32,
    pub text: String,
    pub edits: Vec<Edit>,
}

/// Belgedeki sürüm. Yok, tam sayı değil ya da < 1 ise 1 (ilk biçim).
pub fn file_version(doc: &DocumentMut) -> u32 {
    match doc.get(VERSION_KEY).and_then(Item::as_integer) {
        Some(v) if v >= 1 => u32::try_from(v).unwrap_or(u32::MAX),
        _ => 1,
    }
}

/// Metni güncel sürüme göç ettirir (bellekte; yazım çağıranın).
pub fn migrate(text: &str) -> Migration {
    migrate_with(text, MIGRATIONS, CURRENT_CONFIG_VERSION)
}

fn migrate_with(text: &str, steps: &[Step], current: u32) -> Migration {
    let Ok(mut doc) = text.parse::<DocumentMut>() else { return Migration::Unparsable };
    let from = file_version(&doc);
    if from > current {
        return Migration::Newer(from);
    }
    if from == current {
        return Migration::Current;
    }
    let mut edits = Vec::new();
    for step in steps.iter().filter(|s| s.to > from && s.to <= current) {
        let e = (step.run)(&mut doc);
        if !e.is_empty() {
            tracing::debug!(step = step.name, edits = e.len(), "shell.toml göç adımı");
        }
        edits.extend(e);
    }
    if edits.is_empty() {
        return Migration::Unchanged { from };
    }
    set_version(&mut doc, current);
    Migration::Migrated(Migrated { from, to: current, text: doc.to_string(), edits })
}

/// Metin sürümsüz mü (sürüm anahtarı yok ya da geçersiz → v1 sayılıyor)? Ayrıştırılamazsa `false`.
pub fn is_unversioned(text: &str) -> bool {
    text.parse::<DocumentMut>().is_ok_and(|doc| file_version(&doc) == 1)
}

/// Göç adımlarının bir şey değiştirmediği eski sürümlü metne yalnızca güncel sürüm damgası
/// (dosya başlığı yorumu üstte kalır). Sürüm zaten güncel/yeni ya da metin ayrıştırılamıyorsa
/// `None`. Saf; yazım çağıranın.
pub fn stamp_version(text: &str) -> Option<String> {
    let mut doc = text.parse::<DocumentMut>().ok()?;
    if file_version(&doc) >= CURRENT_CONFIG_VERSION {
        return None;
    }
    set_version(&mut doc, CURRENT_CONFIG_VERSION);
    Some(doc.to_string())
}

/// Bu süreç `shell.toml`'a göç/damga yazabilir mi?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritePolicy {
    /// Tek-örnek kilidini almış asıl shell: göç ve damga dosyaya yazılır.
    Disk,
    /// İkincil çağrı, IPC'siz çalışma ya da `NUXUT_SHELL_INSTANCE`'lı test örneği: yalnızca
    /// bellekte (kullanıcının dosyasına dokunulmaz).
    #[default]
    MemoryOnly,
}

impl WritePolicy {
    /// `primary`: tek-örnek kilidi alındı; `app`: etkin IPC adı ([`crate::ipc::app_name`]).
    /// Yalnızca varsayılan adla çalışan asıl örnek diske yazar.
    pub fn for_instance(primary: bool, app: &str) -> Self {
        if primary && app == crate::ipc::APP_NAME { Self::Disk } else { Self::MemoryOnly }
    }
}

/// `config_version`'ı yazar: varsa yerinde (yorumu korunur), yoksa kökün en üstüne.
fn set_version(doc: &mut DocumentMut, version: u32) {
    let new = Value::from(i64::from(version));
    if let Some(item) = doc.get_mut(VERSION_KEY) {
        replace_value(item, new);
        return;
    }
    // Dosyanın baş yorumu (ilk öğenin önündeki, boş satırla ayrılmış blok) sürümün üstünde kalır.
    let header = take_file_header(doc);
    let root = doc.as_table_mut();
    let mut key = Key::new(VERSION_KEY);
    if let Some(h) = header {
        key.leaf_decor_mut().set_prefix(h);
    }
    root.insert_formatted(&key, Item::Value(new));
    // Kararlı sıralama: yalnızca sürüm anahtarı başa geçer, diğerlerinin sırası aynı kalır.
    root.sort_values_by(|a, _, b, _| (a.get() != VERSION_KEY).cmp(&(b.get() != VERSION_KEY)));
}

/// İlk görüntülenen öğenin önündeki yorumdan, son boş satıra kadar olan dosya başlığını ayırır
/// (o öğeye ait yorum öğede kalır, önüne boş satır konur). Kökte noktalı tablo varsa ya da ilk
/// tablo iç içe ise dokunulmaz (`None`).
fn take_file_header(doc: &mut DocumentMut) -> Option<String> {
    let root = doc.as_table_mut();
    if root.iter().any(|(_, item)| item.as_table().is_some_and(Table::is_dotted)) {
        return None;
    }
    let split = |prefix: &str| -> Option<(String, String)> {
        let at = prefix.rfind("\n\n")?;
        Some((prefix[..at + 2].to_owned(), format!("\n{}", &prefix[at + 2..])))
    };
    let first_value = root.iter().find(|(_, item)| item.is_value()).map(|(k, _)| k.to_owned());
    if let Some(first) = first_value {
        let mut key = root.key_mut(&first)?;
        let decor = key.leaf_decor_mut();
        let (header, rest) = split(decor.prefix()?.as_str()?)?;
        decor.set_prefix(rest);
        return Some(header);
    }
    // Değer yok: en küçük konumlu tablo kökün doğrudan, başlıklı tablosuysa onun yorumu.
    let mut first: Option<(isize, Option<String>)> = None;
    visit_tables(root, None, &mut |pos, root_key| {
        if first.as_ref().is_none_or(|(p, _)| pos < *p) {
            first = Some((pos, root_key));
        }
    });
    let name = first?.1?;
    let table = root.get_mut(&name)?.as_table_mut()?;
    let decor = table.decor_mut();
    let (header, rest) = split(decor.prefix()?.as_str()?)?;
    decor.set_prefix(rest);
    Some(header)
}

/// Başlıklı (örtük olmayan) her tablonun konumu; kökün doğrudan çocuğuysa adıyla.
fn visit_tables(table: &Table, root_key: Option<&str>, f: &mut dyn FnMut(isize, Option<String>)) {
    for (k, item) in table.iter() {
        let Some(t) = item.as_table() else { continue };
        let direct = root_key.is_none().then(|| k.to_owned());
        if !t.is_implicit()
            && let Some(pos) = t.position()
        {
            f(pos, direct.clone());
        }
        visit_tables(t, Some(root_key.unwrap_or(k)), f);
    }
}

/// Değeri değiştirir, satır süslemesini (satır sonu yorumu) korur.
fn replace_value(item: &mut Item, mut new: Value) {
    if let Item::Value(old) = item {
        *new.decor_mut() = old.decor().clone();
        *old = new;
    } else {
        *item = Item::Value(new);
    }
}

/// Değerin süslemesiz TOML yazımı (geri al kaydı için).
fn value_literal(v: &Value) -> String {
    let mut v = v.clone();
    v.decor_mut().clear();
    v.to_string()
}

/// Kökten `path` boyunca tablo-benzeri öğe (standart, noktalı ya da satır içi tablo).
fn table_like_mut<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> Option<&'a mut dyn TableLike> {
    let mut cur: &'a mut dyn TableLike = doc.as_table_mut();
    for part in path {
        let next = cur;
        cur = next.get_mut(part)?.as_table_like_mut()?;
    }
    Some(cur)
}

/// `old` anahtarını yerinde `new` olarak yeniden adlandırır: anahtarın süslemesi (önündeki
/// yorum) ve değerin süslemesi taşınır, tablodaki sırası korunur. `new` boş olmalı.
fn rename_in_place(t: &mut dyn TableLike, old: &str, new: &str) {
    let keys: Vec<String> = t.iter().map(|(k, _)| k.to_owned()).collect();
    let Some(idx) = keys.iter().position(|k| k == old) else { return };
    let Some(old_key) = t.key(old).cloned() else { return };
    let Some(item) = t.remove(old) else { return };
    let new_key = Key::new(new)
        .with_leaf_decor(old_key.leaf_decor().clone())
        .with_dotted_decor(old_key.dotted_decor().clone());
    // Sonraki anahtarlar çıkarılıp yeniden eklenir ki yeni anahtar eski yerinde kalsın.
    // (Alt tabloların görüntü yeri kendi `position`'larındadır; etkilenmez.)
    let tail: Vec<(Key, Item)> =
        keys[idx + 1..].iter().filter_map(|k| Some((t.key(k)?.clone(), t.remove(k)?))).collect();
    // Satır içi tabloda öğeler zaten değerdir; `insert` paniklemez.
    t.entry_format(&new_key).or_insert(item);
    for (k, item) in tail {
        t.entry_format(&k).or_insert(item);
    }
}

// --- v2 adımları --------------------------------------------------------------------------

/// `[bar] left/right` → `start/end`. Eski kod `left`'i `start`'ın üzerine yazardı; ikisi de
/// varsa `start`'ın değeri zaten etkisizdi (DropDead), `left` yerine geçer (davranış aynı).
fn bar_start_end(doc: &mut DocumentMut) -> Vec<Edit> {
    let mut edits = Vec::new();
    let Some(bar) = table_like_mut(doc, &["bar"]) else { return edits };
    for (old, new) in [("left", "start"), ("right", "end")] {
        if !bar.contains_key(old) {
            continue;
        }
        if bar.remove(new).is_some() {
            edits.push(Edit { key: KeyPath::new(&["bar", new]), kind: EditKind::DropDead, note: t!("shell-config-migrate-note-bar-section"), restore: None });
        }
        rename_in_place(bar, old, new);
        edits.push(Edit {
            key: KeyPath::new(&["bar", new]),
            kind: EditKind::Rename { from: KeyPath::new(&["bar", old]) },
            note: t!("shell-config-migrate-note-bar-section"),
            restore: None,
        });
    }
    edits
}

/// `[workspaces]`, `[clock]` → `[bar.widget.*]`. Yeni tablo zaten varsa eski kod eskisini yok
/// sayardı: silinir (DropDead).
fn widget_tables(doc: &mut DocumentMut) -> Vec<Edit> {
    let mut edits = Vec::new();
    for name in ["workspaces", "clock"] {
        if !doc.get(name).is_some_and(Item::is_table_like) {
            continue;
        }
        let from = KeyPath::new(&[name]);
        let to = KeyPath::new(&["bar", "widget", name]);
        if table_like_mut(doc, &["bar", "widget", name]).is_some() {
            doc.as_table_mut().remove(name);
            edits.push(Edit { key: from, kind: EditKind::DropDead, note: t!("shell-config-migrate-note-widget-settings"), restore: None });
            continue;
        }
        // Hedefin üst tabloları tablo değilse (ör. `bar = 1`) eski dosya zaten yüklenmezdi; dokunma.
        if !ensure_tables(doc, &["bar", "widget"]) {
            continue;
        }
        let Some(item) = doc.as_table_mut().remove(name) else { continue };
        let Some(parent) = doc.get_mut("bar").and_then(|b| b.get_mut("widget")) else { continue };
        insert_child(parent, name, item);
        edits.push(Edit { key: to, kind: EditKind::Rename { from }, note: t!("shell-config-migrate-note-widget-settings"), restore: None });
    }
    edits
}

/// Kökten `path` boyunca tabloları sağlar: eksik olanlar örtük (başlıksız) tablo olarak
/// yaratılır. Yoldaki bir öğe tablo değilse `false`.
fn ensure_tables(doc: &mut DocumentMut, path: &[&str]) -> bool {
    let mut cur: &mut Item = doc.as_item_mut();
    for (i, part) in path.iter().enumerate() {
        let next = cur;
        let child = match next {
            Item::Table(t) => t.entry(part).or_insert_with(|| {
                let mut t = Table::new();
                t.set_implicit(true);
                Item::Table(t)
            }),
            // Satır içi tablonun altında yalnızca satır içi tablolar olabilir.
            Item::Value(Value::InlineTable(t)) => return ensure_inline(t, &path[i..]),
            _ => return false,
        };
        if !child.is_table_like() {
            return false;
        }
        cur = child;
    }
    true
}

fn ensure_inline(t: &mut InlineTable, path: &[&str]) -> bool {
    let Some((first, rest)) = path.split_first() else { return true };
    if !t.contains_key(first) {
        t.insert(*first, Value::InlineTable(InlineTable::new()));
    }
    match t.get_mut(first) {
        Some(Value::InlineTable(child)) => ensure_inline(child, rest),
        _ => false,
    }
}

/// `parent`'a (standart ya da satır içi tablo) `name = item` ekler. Satır içi ebeveyne tablo
/// satır içi tabloya çevrilerek, standart ebeveyne satır içi tablo standart tabloya çevrilerek
/// girer (başlık `[bar.widget.<ad>]` olur).
fn insert_child(parent: &mut Item, name: &str, item: Item) {
    match parent {
        Item::Table(t) => {
            let item = match item {
                Item::Value(Value::InlineTable(it)) => Item::Table(it.into_table()),
                Item::Table(mut tab) => {
                    tab.set_dotted(false);
                    Item::Table(tab)
                }
                other => other,
            };
            t.insert(name, item);
        }
        Item::Value(Value::InlineTable(t)) => {
            if let Ok(v) = item.into_value() {
                t.insert(name, v);
            }
        }
        _ => {}
    }
}

/// `[frame] inner_radius`: ADR-0008'den beri etkisiz → silinir.
fn frame_inner_radius(doc: &mut DocumentMut) -> Vec<Edit> {
    let removed = table_like_mut(doc, &["frame"]).and_then(|frame| frame.remove("inner_radius"));
    match removed {
        Some(_) => vec![Edit {
            key: KeyPath::new(&["frame", "inner_radius"]),
            kind: EditKind::DropDead,
            note: t!("shell-config-migrate-note-frame-inner-corner"),
            restore: None,
        }],
        None => Vec::new(),
    }
}

/// `[bar.widget.workspaces]`: v1'de `count` yalnızca `show_empty = true` iken etkiliydi; v2'de
/// `count` sayfa boyudur (1..=30, her zaman bu kadar yuva) ve `show_empty` yoktur.
///
/// | v1 | v2 | tür |
/// |---|---|---|
/// | `show_empty = true`, `count ≤ 30` / yok | `count` aynı | `show_empty` DropDead |
/// | `show_empty = true`, `count > 30` | `count = 30` | Transform |
/// | `show_empty` false/yok, `count` yazılı | `count` silinir (varsayılan 10) | Transform |
/// | `show_empty = false` | silinir | DropDead |
fn workspaces_page_count(doc: &mut DocumentMut) -> Vec<Edit> {
    let mut edits = Vec::new();
    let Some(ws) = table_like_mut(doc, &["bar", "widget", "workspaces"]) else { return edits };
    let key = |k: &str| KeyPath::new(&["bar", "widget", "workspaces", k]);
    let note = t!("shell-bar-workspaces-count-label");
    let show_empty = ws.get("show_empty").and_then(Item::as_bool) == Some(true);
    let old_count = ws.get("count").and_then(Item::as_value).map(|v| (value_literal(v), v.as_integer()));
    match old_count {
        Some((literal, Some(n))) if show_empty && n > V2_MAX_WORKSPACE_PAGE => {
            if let Some(item) = ws.get_mut("count") {
                replace_value(item, Value::from(V2_MAX_WORKSPACE_PAGE));
            }
            edits.push(Edit {
                key: key("count"),
                kind: EditKind::Transform,
                note: note.clone(),
                restore: Some(Restore { key: key("count"), value: literal }),
            });
        }
        Some(_) if show_empty => {}
        Some((literal, _)) => {
            // Görünmeyen bir sayı yeni anlamda birden "her zaman N yuva" olurdu: silinir.
            ws.remove("count");
            edits.push(Edit {
                key: key("count"),
                kind: EditKind::Transform,
                note: note.clone(),
                restore: Some(Restore { key: key("count"), value: literal }),
            });
        }
        None => {}
    }
    if ws.remove("show_empty").is_some() {
        edits.push(Edit { key: key("show_empty"), kind: EditKind::DropDead, note: t!("shell-config-migrate-note-show-empty"), restore: None });
    }
    edits
}

// --- kayıt, bildirim, geri al -------------------------------------------------------------

/// Bir yüklemede yapılan göç.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationRecord {
    pub from: u32,
    pub to: u32,
    pub edits: Vec<Edit>,
    /// Dosyaya yazıldı mı (yazılamadıysa yalnızca bellekte uygulandı; sonraki açılış yeniden dener).
    pub written: bool,
    /// Göçün `migration.shell` snapshot'ı (geri alınabilir değer varsa ve depo kullanılabildiyse).
    /// `None`: geri alma bellekteki kayıtla ([`undo_text`]) yapılır.
    pub snapshot: Option<SnapshotId>,
}

/// Kullanıcıya gösterilecek tek sessiz bildirim (pop-up yok; merkeze düşer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationNotice {
    pub summary: String,
    /// Değişen ayarların adları (en çok 3 + "…").
    pub body: String,
    pub action_label: String,
    /// Eylemin karşılığı olan shell komutu (`nuxut-shell msg …`).
    pub action_command: &'static str,
}

/// `config undo-migration` komutunun metni.
pub const UNDO_COMMAND: &str = "config undo-migration";

impl MigrationRecord {
    fn notified(&self) -> impl Iterator<Item = &Edit> {
        self.edits.iter().filter(|e| e.kind.notifies())
    }

    /// Yalnızca dosyaya yazılmış ve davranışı değiştiren bir göçte bildirim.
    pub fn notice(&self) -> Option<MigrationNotice> {
        if !self.written {
            return None;
        }
        let mut names: Vec<String> = Vec::new();
        for e in self.notified() {
            if !names.contains(&e.note) {
                names.push(e.note.clone());
            }
        }
        if names.is_empty() {
            return None;
        }
        let more = names.len() > MAX_NOTICE_NAMES;
        names.truncate(MAX_NOTICE_NAMES);
        let mut body = names.join(", ");
        if more {
            body.push_str(", …");
        }
        Some(MigrationNotice {
            summary: t!("shell-config-migrated-title"),
            body,
            action_label: t!("shell-config-migrated-undo"),
            action_command: UNDO_COMMAND,
        })
    }

    fn has_restore(&self) -> bool {
        self.notified().any(|e| e.restore.is_some())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("geri alınacak bir göç yok")]
    Nothing,
    #[error("config okunamadı: {0}")]
    Read(String),
    #[error("config ayrıştırılamadı; geri alma yapılmadı")]
    Parse,
    #[error("{0} artık bir tablo değil; geri alma yapılmadı")]
    Conflict(String),
    #[error(transparent)]
    Write(#[from] WriteError),
    /// Snapshot geri yüklenemedi (ADR-0001 §10.3 adı: `not_found`, `busy`, `skipped`, …).
    #[error("göç snapshot'ı geri yüklenemedi ({0})")]
    Restore(&'static str),
}

/// Göç kaydının değerlerini metne geri koyar (saf). Değişiklik yoksa `None`.
pub fn undo_text(text: &str, record: &MigrationRecord) -> Result<Option<String>, UndoError> {
    let mut doc: DocumentMut = text.parse().map_err(|_| UndoError::Parse)?;
    let mut changed = false;
    for r in record.notified().filter_map(|e| e.restore.as_ref()) {
        let value: Value = r.value.parse().map_err(|_| UndoError::Parse)?;
        set_path(&mut doc, &r.key, value)?;
        changed = true;
    }
    Ok(changed.then(|| doc.to_string()))
}

/// `path`'e değeri yazar; eksik üst tablolar örtük yaratılır, var olan değerin yorumu korunur.
fn set_path(doc: &mut DocumentMut, path: &KeyPath, value: Value) -> Result<(), UndoError> {
    let parts = path.parts();
    let Some((leaf, parents)) = parts.split_last() else { return Ok(()) };
    let names: Vec<&str> = parents.iter().map(String::as_str).collect();
    if !ensure_tables(doc, &names) {
        return Err(UndoError::Conflict(KeyPath::new(&names).to_string()));
    }
    let parent = table_like_mut(doc, &names).ok_or_else(|| UndoError::Conflict(KeyPath::new(&names).to_string()))?;
    match parent.get_mut(leaf) {
        Some(item) => replace_value(item, value),
        None => {
            parent.insert(leaf, Item::Value(value));
        }
    }
    Ok(())
}

/// Son göçün geri alma kaydı (bildirim ve `setup status` `pending_notice` için süreç belleğinde).
/// Geri alma göçün `migration.shell` snapshot'ının geri yüklenmesidir ([`undo_record`]).
#[derive(Debug, Default)]
pub struct MigrationUndo {
    last: Option<MigrationRecord>,
}

impl MigrationUndo {
    /// Yazılmış ve geri alınabilir değişiklik içeren kaydı saklar.
    pub fn remember(&mut self, record: &MigrationRecord) {
        if record.written && record.has_restore() {
            self.last = Some(record.clone());
        }
    }

    pub fn pending(&self) -> Option<&MigrationRecord> {
        self.last.as_ref()
    }

    /// Kayıt bu snapshot'ın göçüyse unutur (snapshot başka yoldan — `msg snapshot restore`,
    /// Ayarlar — geri yüklendi).
    pub fn forget_snapshot(&mut self, id: &SnapshotId) {
        if self.last.as_ref().and_then(|r| r.snapshot.as_ref()) == Some(id) {
            self.last = None;
        }
    }

    /// Son göçü geri alır ([`undo_record`]) ve kaydı unutur.
    pub fn undo(&mut self, path: &Path, env: &Env) -> Result<(), UndoError> {
        let record = self.last.as_ref().ok_or(UndoError::Nothing)?;
        undo_record(record, path, env)?;
        self.last = None;
        Ok(())
    }
}

/// Bir göçü geri alır. Snapshot'ı varsa onu **zorla** geri yükler (kullanıcı anahtarı sonradan
/// değiştirmiş olsa da eski değer yeni anlamıyla yazılır — "Geri al"ın önceki davranışı);
/// yoksa dosyayı okuyup [`undo_text`] ile aynı sonucu yazar. Bloklayan IO (kilit ≤ 5 sn).
pub fn undo_record(record: &MigrationRecord, path: &Path, env: &Env) -> Result<(), UndoError> {
    if let Some(id) = &record.snapshot {
        let app = Applier::open(env.clone()).with_source(Source::Shell);
        return match app.restore(id, RestoreMode::Force) {
            RestoreOutcome::Restored { .. } | RestoreOutcome::Partial { .. } => Ok(()),
            other => Err(UndoError::Restore(other.name())),
        };
    }
    let text = read_config(path).map_err(|e| UndoError::Read(e.log_summary()))?.unwrap_or_default();
    if let Some(new) = undo_text(&text, record)? {
        ApplyConfigWriter::new(env.clone()).replace(path, &text, &new, WriteLabel::MigrationUndo, &record.edits)?;
    }
    Ok(())
}

// --- yazım --------------------------------------------------------------------------------

/// Yazımın nedeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteLabel {
    /// Sürüm göçü: snapshot `migration.shell` (ADR-0001 §9.3).
    Migration,
    /// Snapshot'sız göçün bellekteki kayıtla geri alınması (snapshot yok).
    MigrationUndo,
    /// Yalnızca sürüm damgası (değer değişmedi; snapshot yok).
    VersionStamp,
    /// Kendini onarma (`config_heal`): önce tam dosya yedeği alınır, snapshot yok.
    Heal,
}

impl WriteLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Migration => "migration.shell",
            Self::MigrationUndo => "migration.shell.undo",
            Self::VersionStamp => "migration.shell.stamp",
            Self::Heal => "heal.shell",
        }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("dosya bu arada değişti; yazılmadı")]
    Changed,
    #[error("yeni metin boyut sınırını aşıyor")]
    TooLarge,
    #[error("yazılamadı ({0:?})")]
    Io(std::io::ErrorKind),
    /// `nuxut-apply` reddi (ADR-0001 §10.3 adı: `externally_managed`, `skipped`, `not_found`, …).
    #[error("yazılamadı ({0})")]
    Apply(&'static str),
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.kind())
    }
}

impl From<ApplyError> for WriteError {
    fn from(e: ApplyError) -> Self {
        match e {
            ApplyError::Changed => Self::Changed,
            ApplyError::TooLarge(_) => Self::TooLarge,
            ApplyError::Io(k) => Self::Io(k),
            other => Self::Apply(other.kind_name()),
        }
    }
}

/// Config dosyasının tek yazım noktası. Dönüş: alındıysa snapshot kimliği.
pub trait ConfigWriter {
    /// Dosya hâlâ `expected` içeriğindeyse `new` ile atomik değiştirir; değilse
    /// [`WriteError::Changed`] (çağıran bir sonraki yüklemede yeniden dener).
    fn replace(
        &self,
        path: &Path,
        expected: &str,
        new: &str,
        label: WriteLabel,
        edits: &[Edit],
    ) -> Result<Option<SnapshotId>, WriteError>;
}

/// `nuxut-apply` üzerinden yazıcı. Damga ve snapshot'sız geri alma: tek atomik yazım
/// (`write_file`, okunan hâlin ön koşuluyla). Göç (`WriteLabel::Migration`), geri alınabilir
/// değer varsa iki adım:
///
/// 1. Göçmüş metin + geri alma değerleri (yeni anlamıyla; [`undo_text`]) snapshot'sız yazılır.
/// 2. `Applier::apply`: yalnızca o anahtarlar (`TomlKey`) göçmüş değerlerine — snapshot
///    `migration.shell`, "önce" = yeni anlamlı eski değer. Biçim farkı kalırsa göçmüş metin aynen
///    yazılır (değerler aynı; snapshot'ın "sonra"sı geçerli kalır).
///
/// İkinci adım başarısızsa dosya ilk adımdan önceki hâline döner; `Changed` dışında göç snapshot'sız
/// yazılır (geri alma bellekteki kayıtla). Geri alınabilir değer yoksa (yalnızca sessiz
/// değişiklikler) snapshot alınmaz.
#[derive(Debug, Clone)]
pub struct ApplyConfigWriter {
    env: Env,
}

impl ApplyConfigWriter {
    pub fn new(env: Env) -> Self {
        Self { env }
    }

    fn write(&self, path: &Path, text: &str, expect: &Expect) -> Result<(), WriteError> {
        nuxut_apply::write_file(&self.env, path, text.as_bytes(), Some(expect))?;
        Ok(())
    }

    fn migrate(&self, path: &Path, expect: &Expect, old: &str, new: &str, edits: &[Edit]) -> Result<Option<SnapshotId>, WriteError> {
        let record = MigrationRecord { from: 0, to: 0, edits: edits.to_vec(), written: true, snapshot: None };
        let staged = undo_text(new, &record).ok().flatten();
        let change = staged.as_ref().and_then(|_| migration_change(path, old, new, &record));
        let (Some(staged), Some(change)) = (staged, change) else {
            self.write(path, new, expect)?;
            return Ok(None);
        };
        self.write(path, &staged, expect)?;
        let outcome = Applier::open(self.env.clone()).apply(change);
        let id = match outcome {
            Outcome::Applied { id } => Some(id),
            Outcome::Noop => None,
            other => {
                // Geri dön: dosya hâlâ 1. adımdaysa eski hâline.
                let back = nuxut_apply::read_file(path).ok().filter(|(t, _)| *t == staged);
                let Some((_, at_stage)) = back else {
                    tracing::warn!(result = other.name(), "shell.toml göç snapshot'ı alınamadı ve dosya bu arada değişti");
                    return Err(WriteError::Changed);
                };
                self.write(path, old, &at_stage)?;
                if other == Outcome::Changed {
                    return Err(WriteError::Changed);
                }
                tracing::warn!(result = other.name(), "shell.toml göç snapshot'ı alınamadı; göç snapshot'sız yazılıyor");
                let (_, now) = nuxut_apply::read_file(path)?;
                self.write(path, new, &now)?;
                return Ok(None);
            }
        };
        // Değerler aynı, yalnızca biçim farklı olabilir: göçün metni aynen kalsın.
        match nuxut_apply::read_file(path) {
            Ok((t, e)) if t != new => {
                if let Err(err) = self.write(path, new, &e) {
                    tracing::warn!("shell.toml göç biçimi eşitlenemedi: {err}");
                }
            }
            Ok(_) => {}
            Err(err) => tracing::warn!("shell.toml göçten sonra okunamadı: {}", nuxut_apply::ApplyError::kind_name(&err)),
        }
        Ok(id)
    }
}

/// Göç snapshot'ının değişikliği: her geri alınabilir anahtar → göçmüş değeri (yoksa `Absent`).
/// Göçmüş metinde değer tablo vb. ise ya da yol mutlak değilse `None` (snapshot'sız yazılır).
fn migration_change(path: &Path, old: &str, new: &str, record: &MigrationRecord) -> Option<Change> {
    if !path.is_absolute() {
        return None;
    }
    let doc: DocumentMut = new.parse().ok()?;
    let from = old.parse::<DocumentMut>().map_or(1, |d| file_version(&d));
    let label = t!("shell-config-migrate-snapshot-label", from = from, to = file_version(&doc));
    let mut change = Change::new(crate::services::apply::setting(crate::services::apply::MIGRATION_SHELL), label, Source::Migration);
    for r in record.notified().filter_map(|e| e.restore.as_ref()) {
        let key = nuxut_apply::KeyPath::new(r.key.parts().iter().cloned()).ok()?;
        let fragment = Fragment::TomlKey { file: path.to_path_buf(), key };
        if change.writes.iter().any(|(f, _)| *f == fragment) {
            continue;
        }
        let value = match lookup(&doc, r.key.parts()) {
            None => FragValue::Absent,
            Some(Item::Value(v)) => FragValue::Text(value_literal(v)),
            Some(_) => return None,
        };
        change = change.write(fragment, value);
    }
    (!change.writes.is_empty()).then_some(change)
}

/// Kökten `parts` boyunca öğe (salt okunur).
fn lookup<'a>(doc: &'a DocumentMut, parts: &[String]) -> Option<&'a Item> {
    let (leaf, parents) = parts.split_last()?;
    let mut cur: &dyn TableLike = doc.as_table();
    for p in parents {
        cur = cur.get(p)?.as_table_like()?;
    }
    cur.get(leaf)
}

impl ConfigWriter for ApplyConfigWriter {
    fn replace(
        &self,
        path: &Path,
        expected: &str,
        new: &str,
        label: WriteLabel,
        edits: &[Edit],
    ) -> Result<Option<SnapshotId>, WriteError> {
        if new.len() as u64 > MAX_CONFIG_BYTES {
            return Err(WriteError::TooLarge);
        }
        let (current, expect) = nuxut_apply::read_file(path)?;
        if current != expected {
            return Err(WriteError::Changed);
        }
        match label {
            WriteLabel::Migration => self.migrate(path, &expect, expected, new, edits),
            WriteLabel::MigrationUndo | WriteLabel::VersionStamp | WriteLabel::Heal => {
                self.write(path, new, &expect)?;
                Ok(None)
            }
        }
    }
}

/// Config metnini sınırla okur; dosya yoksa `None`.
pub fn read_config(path: &Path) -> Result<Option<String>, ConfigError> {
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
    let mut text = String::new();
    // Ölçümle okuma arasında dosya büyüyebilir; okuma da sınırlı.
    file.take(MAX_CONFIG_BYTES).read_to_string(&mut text).map_err(io)?;
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ShellConfig;

    fn migrated(text: &str) -> Migrated {
        match migrate(text) {
            Migration::Migrated(m) => m,
            other => panic!("göç beklenirdi: {other:?}\n{text}"),
        }
    }

    /// Çıktı uyarısız ayrışır ve ikinci göç bir şey yapmaz.
    fn assert_clean(m: &Migrated) {
        let (_, w) = ShellConfig::parse(&m.text).unwrap_or_else(|e| panic!("{e}\n{}", m.text));
        assert!(w.is_empty(), "{w:?}\n{}", m.text);
        assert_eq!(migrate(&m.text), Migration::Current, "idempotent:\n{}", m.text);
    }

    fn kinds(m: &Migrated) -> Vec<(String, EditKind)> {
        m.edits.iter().map(|e| (e.key.to_string(), e.kind.clone())).collect()
    }

    #[test]
    fn current_newer_and_unchanged_files_are_not_written() {
        assert_eq!(migrate(""), Migration::Unchanged { from: 1 });
        assert_eq!(migrate("[bar]\nsize = 40\n"), Migration::Unchanged { from: 1 });
        assert_eq!(migrate("config_version = 2\n[bar]\nleft = []\n"), Migration::Current);
        assert_eq!(migrate("config_version = 7\n"), Migration::Newer(7));
        assert_eq!(migrate("config_version = 99999999999\n"), Migration::Newer(u32::MAX));
        assert_eq!(migrate("[bar\n"), Migration::Unparsable);
        // Geçersiz sürüm 1 sayılır; göç gerekirse düzeltilir.
        let m = migrated("config_version = \"x\"\n[frame]\ninner_radius = 3\n");
        assert!(m.text.starts_with("config_version = 2\n"), "{}", m.text);
    }

    /// v2 geliştirmesinde eklenen tablolar (`[launcher]`, `[compositor]`, `[windows]`,
    /// `[keybinds]`, workspaces `style`/`scroll`) yalnızca yeni anahtardır: v1 dosyada yoktular,
    /// varsayılanları koddan gelir — göç adımı gerekmez, sürümsüz dosyada dokunulmaz.
    #[test]
    fn new_v2_tables_need_no_step() {
        let text = "[compositor]\n[windows]\n[keybinds]\n[launcher]\n[bar.widget.workspaces]\nstyle = \"dots\"\nscroll = \"occupied\"\n";
        assert_eq!(migrate(text), Migration::Unchanged { from: 1 });
    }

    /// Beklenmedik biçimler panik yapmaz; taşınamayan eski tablo yerinde kalır.
    #[test]
    fn odd_shapes_do_not_panic() {
        for text in [
            "bar = 1\n[clock]\nformat = \"%R\"\n",
            "[[bar]]\nleft = []\n",
            "[[workspaces]]\ncount = 1\n",
            "workspaces = 5\n",
            "bar.widget = 1\n[clock]\nformat = \"%R\"\n",
            "[bar]\nleft = 1\nstart = 2\n",
            "frame = []\n",
            "[bar.widget]\nworkspaces = 3\n[workspaces]\ncount = 2\n",
            "bar = { widget = 1 }\n[clock]\nformat = \"%R\"\n",
            "[bar.widget.workspaces]\ncount = \"many\"\nshow_empty = \"yes\"\n",
            "config_version = -4\n",
            "config_version = 1.5\n[frame]\ninner_radius = 1\n",
        ] {
            let _ = migrate(text);
            let _ = ShellConfig::parse(text);
        }
        let m = migrated("bar = 1\n[clock]\nformat = \"%R\"\n[frame]\ninner_radius = 1\n");
        assert!(m.text.contains("[clock]"), "{}", m.text);
    }

    /// Boş satırla ayrılmış dosya başlığı sürümün üstünde kalır; öğenin kendi yorumu öğede.
    #[test]
    fn version_goes_below_file_header() {
        let m = migrated("# baş\n\n# çerçeve\n[frame]\ninner_radius = 1\n");
        assert_eq!(m.text, "# baş\n\nconfig_version = 2\n\n# çerçeve\n[frame]\n");
        let m = migrated("# baş\n\nfoo = 1 # x\n[frame]\ninner_radius = 1\n");
        assert_eq!(m.text, "# baş\n\nconfig_version = 2\n\nfoo = 1 # x\n[frame]\n");
        // İlk tablo iç içeyse başlık yerinde kalır (sürüm en üstte).
        let m = migrated("# baş\n\n[bar.widget.clock]\nformat = \"%R\"\n[frame]\ninner_radius = 1\n");
        assert!(m.text.starts_with("config_version = 2\n# baş\n\n[bar.widget.clock]"), "{}", m.text);
    }

    #[test]
    fn bar_left_right_rename_in_place_with_comments() {
        let input = "\
# başlık yorumu
[bar]
edge = \"top\"
# sol taraf
left = [\"clock\"] # saat
center = []
right = [\"workspaces\"]
size = 30
";
        let m = migrated(input);
        assert_eq!(
            m.text,
            "\
config_version = 2
# başlık yorumu
[bar]
edge = \"top\"
# sol taraf
start = [\"clock\"] # saat
center = []
end = [\"workspaces\"]
size = 30
"
        );
        assert_eq!(
            kinds(&m),
            [
                ("bar.start".into(), EditKind::Rename { from: KeyPath::new(&["bar", "left"]) }),
                ("bar.end".into(), EditKind::Rename { from: KeyPath::new(&["bar", "right"]) }),
            ]
        );
        assert_clean(&m);
        let (c, _) = ShellConfig::parse(&m.text).unwrap();
        assert_eq!(c.bar.start, ["clock"]);
    }

    /// Eski kod `left`'i `start`'ın üzerine yazardı: davranış korunur, sessiz.
    #[test]
    fn bar_left_wins_over_existing_start() {
        let m = migrated("[bar]\nstart = [\"tray\"]\nleft = [\"active_window\"]\n");
        assert_eq!(m.text, "config_version = 2\n[bar]\nstart = [\"active_window\"]\n");
        assert!(m.edits.iter().all(|e| !e.kind.notifies()));
        assert_clean(&m);
    }

    #[test]
    fn dotted_and_inline_bar_forms() {
        let m = migrated("bar.left = [\"active_window\"]\n");
        assert_eq!(m.text, "config_version = 2\nbar.start = [\"active_window\"]\n");
        assert_clean(&m);
        let m = migrated("bar = { left = [\"active_window\"], size = 30 }\n");
        assert_eq!(m.text, "config_version = 2\nbar = { start = [\"active_window\"], size = 30 }\n");
        assert_clean(&m);
    }

    #[test]
    fn legacy_tables_move_under_bar_widget() {
        let input = "\
[bar]
size = 30

# saat biçimi
[clock]
format = \"%T\" # saniyeli

[osd]
timeout_ms = 900
";
        let m = migrated(input);
        assert_eq!(
            m.text,
            "\
config_version = 2
[bar]
size = 30

# saat biçimi
[bar.widget.clock]
format = \"%T\" # saniyeli

[osd]
timeout_ms = 900
"
        );
        assert_eq!(kinds(&m), [("bar.widget.clock".into(), EditKind::Rename { from: KeyPath::new(&["clock"]) })]);
        assert_clean(&m);
        let (c, _) = ShellConfig::parse(&m.text).unwrap();
        assert_eq!(c.bar.widgets.clock.format.as_deref(), Some("%T"));
    }

    #[test]
    fn legacy_table_without_bar_and_inline_forms() {
        let m = migrated("[clock]\nformat = \"%R\"\n");
        assert_eq!(m.text, "config_version = 2\n[bar.widget.clock]\nformat = \"%R\"\n");
        assert_clean(&m);
        let m = migrated("clock = { format = \"%R\" }\n");
        assert_clean(&m);
        let (c, _) = ShellConfig::parse(&m.text).unwrap();
        assert_eq!(c.bar.widgets.clock.format.as_deref(), Some("%R"));
        let m = migrated("bar = { size = 30 }\n[clock]\nformat = \"%R\"\n");
        assert_clean(&m);
        assert_eq!(ShellConfig::parse(&m.text).unwrap().0.bar.widgets.clock.format.as_deref(), Some("%R"));
    }

    /// Hedef tablo varsa eski kod eskisini yok sayardı: silinir, sessiz.
    #[test]
    fn legacy_table_with_existing_target_is_dropped() {
        let m = migrated("[clock]\nformat = \"%R\"\n[bar.widget.clock]\nformat = \"%T\"\n");
        assert_eq!(m.text, "config_version = 2\n[bar.widget.clock]\nformat = \"%T\"\n");
        assert_eq!(kinds(&m), [("clock".into(), EditKind::DropDead)]);
        assert_clean(&m);
    }

    #[test]
    fn inner_radius_is_dropped_silently() {
        for value in ["20", "1e9", "\"x\"", "[1, 2]", "inf"] {
            let m = migrated(&format!("[frame]\nthickness = 4\ninner_radius = {value}\n"));
            assert_eq!(m.text, "config_version = 2\n[frame]\nthickness = 4\n", "{value}");
            assert_eq!(kinds(&m), [("frame.inner_radius".into(), EditKind::DropDead)]);
            assert!(m.edits[0].restore.is_none());
            assert_clean(&m);
        }
    }

    fn ws(body: &str) -> Migrated {
        migrated(&format!("[bar.widget.workspaces]\n{body}"))
    }

    #[test]
    fn workspaces_show_empty_keeps_count() {
        let m = ws("count = 5\nshow_empty = true\n");
        assert_eq!(m.text, "config_version = 2\n[bar.widget.workspaces]\ncount = 5\n");
        assert_eq!(kinds(&m), [("bar.widget.workspaces.show_empty".into(), EditKind::DropDead)]);
        assert!(m.edits.iter().all(|e| !e.kind.notifies()));
        assert_clean(&m);
        let m = ws("show_empty = true\n");
        assert_eq!(m.text, "config_version = 2\n[bar.widget.workspaces]\n");
        assert!(m.edits.iter().all(|e| !e.kind.notifies()));
    }

    #[test]
    fn workspaces_large_count_is_clamped_with_notice() {
        let m = ws("count = 45 # çok\nshow_empty = true\n");
        assert_eq!(m.text, "config_version = 2\n[bar.widget.workspaces]\ncount = 30 # çok\n");
        let e = &m.edits[0];
        assert_eq!(e.kind, EditKind::Transform);
        assert_eq!(e.restore, Some(Restore { key: KeyPath::new(&["bar", "widget", "workspaces", "count"]), value: "45".into() }));
        assert_clean(&m);
    }

    /// `show_empty` kapalıyken `count` görünmüyordu: silinir + bildirim (karar).
    #[test]
    fn workspaces_invisible_count_is_removed_with_notice() {
        for body in ["count = 7\n", "count = 7\nshow_empty = false\n"] {
            let m = ws(body);
            assert_eq!(m.text, "config_version = 2\n[bar.widget.workspaces]\n", "{body}");
            assert_eq!(m.edits[0].kind, EditKind::Transform);
            assert_eq!(m.edits[0].restore.as_ref().map(|r| r.value.as_str()), Some("7"));
            assert_clean(&m);
        }
        let m = ws("show_empty = false\nstyle = \"dots\"\n");
        assert_eq!(kinds(&m), [("bar.widget.workspaces.show_empty".into(), EditKind::DropDead)]);
    }

    /// Eski `[workspaces]` tablosu önce taşınır, sonra sayfa anlamına çevrilir.
    #[test]
    fn steps_chain_in_order() {
        let m = migrated("[workspaces]\ncount = 5\n");
        assert_eq!(m.text, "config_version = 2\n[bar.widget.workspaces]\n");
        assert_eq!(
            kinds(&m),
            [
                ("bar.widget.workspaces".into(), EditKind::Rename { from: KeyPath::new(&["workspaces"]) }),
                ("bar.widget.workspaces.count".into(), EditKind::Transform),
            ]
        );
    }

    /// Zincir: tüm v1 biçimlerini taşıyan örnek dosya güncel ve uyarısız olur; yorumlar ve
    /// bilinmeyen tablolar korunur.
    #[test]
    fn v1_fixture_migrates_cleanly() {
        let input = include_str!("../tests/fixtures/config/v1-full.toml");
        let m = migrated(input);
        assert_clean(&m);
        assert_eq!(m.text, include_str!("../tests/fixtures/config/v1-full.expected.toml"));
        let (c, _) = ShellConfig::parse(&m.text).unwrap();
        assert_eq!(c.bar.start, ["clock"]);
        assert_eq!(c.bar.widgets.clock.format.as_deref(), Some("%H:%M:%S"));
        assert_eq!(c.bar.widgets.workspaces.count, 8);
        assert_eq!(c.audio.max_volume, 120);
    }

    /// Yeni widget varsayılan listeye girer: göç listelere dokunmaz, yazılmamış liste
    /// koddaki varsayılanı izler.
    #[test]
    fn migration_never_touches_lists() {
        let m = migrated("[bar]\nleft = [\"clock\"]\n");
        let (c, _) = ShellConfig::parse(&m.text).unwrap();
        assert_eq!(c.bar.end, ShellConfig::default().bar.end, "yazılmamış liste varsayılan");
        assert!(!m.text.contains("end"));
    }

    #[test]
    fn notice_lists_only_behaviour_changes() {
        crate::i18n::use_turkish_for_tests();
        let m = migrated("[bar]\nleft = []\n[frame]\ninner_radius = 2\n");
        let r = MigrationRecord { from: m.from, to: m.to, edits: m.edits, written: true, snapshot: None };
        assert_eq!(r.notice(), None, "yeniden adlandırma ve ölü anahtar sessiz");
        let m = ws("count = 7\n");
        let mut r = MigrationRecord { from: m.from, to: m.to, edits: m.edits, written: true, snapshot: None };
        let n = r.notice().unwrap();
        assert_eq!((n.summary.as_str(), n.body.as_str(), n.action_command), ("Ayarların yeni sürüme taşındı", "Sayfa başına workspace", UNDO_COMMAND));
        r.written = false;
        assert_eq!(r.notice(), None, "yazılamayan göç bildirilmez (sonraki açılış yeniden dener)");
        let many = MigrationRecord {
            from: 1,
            to: 2,
            written: true,
            snapshot: None,
            edits: ["a", "b", "c", "d", "a"]
                .into_iter()
                .map(|note| Edit { key: KeyPath::new(&[note]), kind: EditKind::Drop, note: note.to_string(), restore: None })
                .collect(),
        };
        assert_eq!(many.notice().unwrap().body, "a, b, c, …");
    }

    #[test]
    fn undo_restores_value_with_new_meaning_and_keeps_version() {
        let m = ws("count = 7 # benim\n");
        let r = MigrationRecord { from: m.from, to: m.to, edits: m.edits.clone(), written: true, snapshot: None };
        let undone = undo_text(&m.text, &r).unwrap().unwrap();
        assert_eq!(undone, "config_version = 2\n[bar.widget.workspaces]\ncount = 7\n");
        assert_eq!(migrate(&undone), Migration::Current, "geri al yeniden göçe yol açmaz");
        let (c, w) = ShellConfig::parse(&undone).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.bar.widgets.workspaces.count, 7);
        // Kullanıcı bu arada tabloyu silmişse yeniden yaratılır; değeri değiştirdiyse üzerine yazılır.
        assert_eq!(
            undo_text("config_version = 2\n", &r).unwrap().unwrap(),
            "config_version = 2\n\n[bar.widget.workspaces]\ncount = 7\n"
        );
        assert!(matches!(undo_text("config_version = 2\n[bar]\nwidget = 1\n", &r), Err(UndoError::Conflict(_))));
        // Bildirilmeyen değişiklik geri alınmaz.
        let silent = migrated("[frame]\ninner_radius = 2\n");
        let r = MigrationRecord { from: 1, to: 2, edits: silent.edits, written: true, snapshot: None };
        assert_eq!(undo_text(&silent.text, &r).unwrap(), None);
    }

    /// Geçici kök + test ortamı + `<kök>/config/nuxut/shell.toml` (kapı yalnızca kökü açar).
    fn fixture() -> (tempfile::TempDir, Env, std::path::PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let env = Env::for_test(t.path());
        let dir = t.path().join("config/nuxut");
        std::fs::create_dir_all(&dir).unwrap();
        (t, env, dir.join("shell.toml"))
    }

    #[test]
    fn apply_writer_follows_symlink_and_refuses_changed_file() {
        use std::os::unix::fs::PermissionsExt;
        let (_t, env, link) = fixture();
        let real = link.with_file_name("real.toml");
        std::fs::write(&real, "a = 1\n").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let w = ApplyConfigWriter::new(env.clone());
        assert_eq!(w.replace(&link, "a = 2\n", "b = 1\n", WriteLabel::VersionStamp, &[]), Err(WriteError::Changed));
        assert_eq!(w.replace(&link, "a = 1\n", "b = 1\n", WriteLabel::VersionStamp, &[]), Ok(None));
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "symlink korunur");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "b = 1\n");
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o640);
        let big = "#".repeat(MAX_CONFIG_BYTES as usize + 1);
        assert_eq!(w.replace(&link, "b = 1\n", &big, WriteLabel::Migration, &[]), Err(WriteError::TooLarge));
        let leftovers = std::fs::read_dir(link.parent().unwrap())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        // Kapı: kökün dışındaki dosyaya (test örneği) yazılmaz.
        let outside = tempfile::tempdir().unwrap();
        let other = outside.path().join("shell.toml");
        std::fs::write(&other, "a = 1\n").unwrap();
        assert_eq!(w.replace(&other, "a = 1\n", "b = 1\n", WriteLabel::VersionStamp, &[]), Err(WriteError::Apply("skipped")));
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "a = 1\n");
    }

    /// Yükleme: göç yazılır, `migration.shell` snapshot'ı alınır, ikinci yükleme dosyaya
    /// dokunmaz; geri al = snapshot'ın geri yüklenmesi (değer yeni anlamıyla döner).
    #[test]
    fn load_migrates_file_once_and_undo_restores() {
        let (_t, env, path) = fixture();
        std::fs::write(&path, "# benim\n[workspaces]\ncount = 7\n").unwrap();
        let loaded = ShellConfig::load_from(&path, &env).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let record = loaded.migration.expect("göç kaydı");
        assert!(record.written && record.notice().is_some());
        let id = record.snapshot.clone().expect("göç snapshot'ı");
        let text = std::fs::read_to_string(&path).unwrap();
        // Göçün boşalttığı `[bar.widget.workspaces]` başlığını aynı yazımda onarım kaldırır.
        assert_eq!(text, "config_version = 2\n# benim\n");
        let app = Applier::open(env.clone());
        let list = app.list(&nuxut_apply::Filter::default()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].id.clone(), list[0].setting.as_str()), (id.clone(), "migration.shell"));
        let again = ShellConfig::load_from(&path, &env).unwrap();
        assert!(again.migration.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        let mut undo = MigrationUndo::default();
        undo.remember(&record);
        undo.undo(&path, &env).unwrap();
        assert!(undo.pending().is_none());
        assert!(matches!(undo.undo(&path, &env), Err(UndoError::Nothing)));
        let after = ShellConfig::load_from(&path, &env).unwrap();
        assert!(after.migration.is_none() && after.warnings.is_empty(), "{:?}", after.warnings);
        assert_eq!(after.config.bar.widgets.workspaces.count, 7);
        assert!(app.show(&id).unwrap().is_some_and(|d| d.restored_by.is_some()), "göç kaydı geri yüklendi olarak işaretli");
    }

    /// Snapshot'sız kayıt (eski oturum, depo yok): bellekteki kayıtla aynı sonuç.
    #[test]
    fn undo_without_snapshot_uses_record() {
        let (_t, env, path) = fixture();
        std::fs::write(&path, "[workspaces]\ncount = 7\n").unwrap();
        let mut record = ShellConfig::load_from(&path, &env).unwrap().migration.unwrap();
        record.snapshot = None;
        undo_record(&record, &path, &env).unwrap();
        assert_eq!(ShellConfig::load_from(&path, &env).unwrap().config.bar.widgets.workspaces.count, 7);
    }

    /// Yazım başarısızsa göç bellekte uygulanır, uyarı verilir, kayıt `written = false`.
    #[test]
    fn failed_write_still_applies_in_memory() {
        struct Refuse;
        impl ConfigWriter for Refuse {
            fn replace(&self, _: &Path, _: &str, _: &str, _: WriteLabel, _: &[Edit]) -> Result<Option<SnapshotId>, WriteError> {
                Err(WriteError::Changed)
            }
        }
        let (_t, _env, path) = fixture();
        std::fs::write(&path, "[bar]\nleft = [\"active_window\"]\n").unwrap();
        let loaded = ShellConfig::load_with(&path, Some(&Refuse), false).unwrap();
        assert_eq!(loaded.config.bar.start, ["active_window"]);
        assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
        assert!(!loaded.migration.unwrap().written);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[bar]\nleft = [\"active_window\"]\n");
    }

    #[test]
    fn newer_file_warns_and_is_not_written() {
        let (c, w) = ShellConfig::parse("config_version = 9\n[bar]\nsize = 40\n").unwrap();
        assert_eq!(c.bar.size, 40.0);
        assert!(w.len() == 1 && w[0].contains("daha yeni"), "{w:?}");
    }

    /// Her yazımda paniğe düşen yazıcı: "dosyaya dokunulmaz" testleri için.
    struct NoWrite;
    impl ConfigWriter for NoWrite {
        fn replace(&self, path: &Path, _: &str, new: &str, label: WriteLabel, _: &[Edit]) -> Result<Option<SnapshotId>, WriteError> {
            panic!("beklenmeyen yazım ({}) {}:\n{new}", label.as_str(), path.display());
        }
    }

    fn load_state(path: &Path, env: &Env, unversioned_is_current: bool) -> crate::config::LoadedConfig {
        ShellConfig::load_with(path, Some(&ApplyConfigWriter::new(env.clone())), unversioned_is_current).unwrap()
    }

    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn stamp_version_only_for_old_files() {
        assert_eq!(stamp_version("# baş\n\n[bar]\nsize = 40\n").as_deref(), Some("# baş\n\nconfig_version = 2\n\n[bar]\nsize = 40\n"));
        assert_eq!(stamp_version("").as_deref(), Some("config_version = 2\n"));
        assert_eq!(stamp_version("config_version = \"x\"\n").as_deref(), Some("config_version = 2\n"));
        assert_eq!(stamp_version("config_version = 2\n"), None);
        assert_eq!(stamp_version("config_version = 9\n"), None);
        assert_eq!(stamp_version("[bar\n"), None);
        assert!(is_unversioned("[bar]\n") && is_unversioned("config_version = 0\n"));
        assert!(!is_unversioned("config_version = 2\n") && !is_unversioned("[bar\n"));
    }

    /// ENGELLEYİCİ senaryo: sürümsüz dosya adımlardan değişmeden geçse de bir kez damgalanır
    /// (başlık yorumu üstte, snapshot yok); sonra Ayarlar'ın v2 anlamıyla yazdığı `count = 15`
    /// korunur.
    #[test]
    fn unversioned_file_is_stamped_and_later_v2_count_is_kept() {
        let (_t, env, path) = fixture();
        std::fs::write(&path, "# benim\n\n[bar]\nsize = 40\n").unwrap();
        let loaded = ShellConfig::load_from(&path, &env).unwrap();
        assert!(loaded.warnings.is_empty() && loaded.migration.is_none(), "{:?}", loaded.warnings);
        assert!(!loaded.unversioned_is_current);
        let stamped = std::fs::read_to_string(&path).unwrap();
        assert_eq!(stamped, "# benim\n\nconfig_version = 2\n\n[bar]\nsize = 40\n");
        assert!(Applier::open(env.clone()).list(&nuxut_apply::Filter::default()).unwrap().is_empty(), "damga snapshot almaz");
        // Ayarlar sayfa boyunu yazar.
        let edited = format!("{stamped}\n[bar.widget.workspaces]\ncount = 15\n");
        std::fs::write(&path, &edited).unwrap();
        let again = ShellConfig::load_with(&path, Some(&NoWrite), false).unwrap();
        assert!(again.migration.is_none() && again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.config.bar.widgets.workspaces.count, 15);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    }

    /// Ayarlar'ın yeni dosya biçimi (`config_version` + v2 anahtarları) göçten değişmeden geçer.
    #[test]
    fn settings_written_file_passes_unchanged() {
        let (_t, _env, path) = fixture();
        let text = "config_version = 2\n\n[bar.widget.workspaces]\ncount = 15\nstyle = \"dots\"\n";
        std::fs::write(&path, text).unwrap();
        let loaded = ShellConfig::load_with(&path, Some(&NoWrite), false).unwrap();
        assert!(loaded.migration.is_none() && loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.config.bar.widgets.workspaces.count, 15);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    /// Damga yazılamazsa (salt okunur) uyarı; dosya bu oturumda v2 sayılır: sonradan yazılan
    /// sürümsüz `count = 15` silinmez.
    #[test]
    fn unstamped_file_counts_as_current_in_memory() {
        let (_t, env, path) = fixture();
        std::fs::write(&path, "[bar]\nsize = 40\n").unwrap();
        set_mode(&path, 0o444);
        let loaded = ShellConfig::load_from(&path, &env).unwrap();
        assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
        assert!(loaded.warnings[0].contains("damgası yazılamadı"), "{:?}", loaded.warnings);
        assert!(loaded.unversioned_is_current);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[bar]\nsize = 40\n");
        set_mode(&path, 0o644);
        std::fs::write(&path, "[bar]\nsize = 40\n[bar.widget.workspaces]\ncount = 15\n").unwrap();
        set_mode(&path, 0o444);
        let again = load_state(&path, &env, loaded.unversioned_is_current);
        assert!(again.migration.is_none(), "v1 sayılmaz");
        assert!(again.unversioned_is_current);
        assert_eq!(again.config.bar.widgets.workspaces.count, 15);
        // Bayraksız (yeni oturum) aynı dosya v1 sayılır: bayrağın etkisi bu.
        let fresh = load_state(&path, &env, false);
        assert_eq!(fresh.config.bar.widgets.workspaces.count, ShellConfig::default().bar.widgets.workspaces.count);
        set_mode(&path, 0o644);
    }

    /// Salt okunur dosya dışarıdan yönetilir (`nuxut-apply`): göç yazılmaz, snapshot da
    /// alınmaz; bellekte + uyarı.
    #[test]
    fn readonly_file_is_not_migrated_on_disk() {
        let (_t, env, path) = fixture();
        let text = "[bar]\nleft = [\"active_window\"]\n";
        std::fs::write(&path, text).unwrap();
        set_mode(&path, 0o444);
        assert_eq!(
            ApplyConfigWriter::new(env.clone()).replace(&path, text, "x = 1\n", WriteLabel::VersionStamp, &[]),
            Err(WriteError::Apply("externally_managed"))
        );
        let loaded = ShellConfig::load_from(&path, &env).unwrap();
        assert_eq!(loaded.config.bar.start, ["active_window"]);
        assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
        assert!(!loaded.migration.unwrap().written);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(Applier::open(env).list(&nuxut_apply::Filter::default()).unwrap().is_empty());
        set_mode(&path, 0o644);
    }

    /// İkincil/test örneği (`writer = None`): göç ve damga yalnızca bellekte, uyarısız.
    #[test]
    fn memory_only_load_never_writes() {
        let (_t, _env, path) = fixture();
        let text = "[workspaces]\ncount = 7\n";
        std::fs::write(&path, text).unwrap();
        let loaded = ShellConfig::load_with(&path, None, false).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let record = loaded.migration.expect("bellekte göç");
        assert!(!record.written && record.notice().is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        std::fs::write(&path, "[bar]\nsize = 40\n").unwrap();
        let loaded = ShellConfig::load_with(&path, None, false).unwrap();
        assert!(loaded.warnings.is_empty() && loaded.unversioned_is_current, "{:?}", loaded.warnings);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[bar]\nsize = 40\n");
    }

    /// Diske yalnızca varsayılan adlı asıl örnek yazar.
    #[test]
    fn only_primary_default_instance_writes() {
        use crate::ipc::APP_NAME;
        assert_eq!(WritePolicy::for_instance(true, APP_NAME), WritePolicy::Disk);
        assert_eq!(WritePolicy::for_instance(false, APP_NAME), WritePolicy::MemoryOnly);
        assert_eq!(WritePolicy::for_instance(true, &format!("{APP_NAME}-test-config")), WritePolicy::MemoryOnly);
        assert_eq!(WritePolicy::for_instance(false, &format!("{APP_NAME}-test-config")), WritePolicy::MemoryOnly);
        assert_eq!(WritePolicy::default(), WritePolicy::MemoryOnly, "güvenli varsayılan");
    }
}
