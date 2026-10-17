//! `com.canonical.dbusmenu` menü ağacı: düz, sınırlı bir Rust ağacı. Framework'ün genel menü
//! modeline dönüştürülmek üzere tasarlandı (UI tipine bağımlılık yok).
//!
//! Sınırlar: en fazla [`MAX_MENU_NODES`] düğüm, kök dahil [`MAX_MENU_DEPTH`] seviye; aşan
//! kısım kırpılır.

use std::collections::HashMap;
use std::sync::Arc;

use zbus::zvariant::{OwnedValue, Value};

use super::item::Pixmap;
use super::value::{as_array, as_bool, as_bytes, as_fields, as_i64, as_str, clip_line, peel};

pub const MAX_MENU_NODES: usize = 512;
pub const MAX_MENU_DEPTH: usize = 8;
pub const MAX_LABEL_CHARS: usize = 256;
/// `icon-data` PNG baytlarının sınırı.
pub const MAX_MENU_ICON_BYTES: usize = 256 * 1024;
const MAX_SHORTCUTS: usize = 4;
const MAX_SHORTCUT_KEYS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuItemKind {
    #[default]
    Standard,
    Separator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleKind {
    Checkmark,
    Radio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToggleState {
    Off,
    On,
    #[default]
    Indeterminate,
}

/// Menüde bir düğüm. Varsayılanlar dbusmenu belirtimindekiler (etkin, görünür, standart).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuNode {
    /// dbusmenu öğe kimliği; olay gönderirken kullanılır. Kökün kimliği 0'dır.
    pub id: i32,
    pub kind: MenuItemKind,
    /// Mnemonik alt çizgileri temizlenmiş etiket (`_Dosya` → `Dosya`, `__` → `_`).
    pub label: String,
    /// Etikette `_` ile işaretlenmiş erişim harfi (küçük harfe çevrilmiş).
    pub mnemonic: Option<char>,
    pub enabled: bool,
    pub visible: bool,
    pub icon_name: String,
    /// `icon-data`: PNG dosya baytları (imza doğrulanmış).
    pub icon_png: Option<Arc<[u8]>>,
    /// Çizime hazır ikon (düz RGBA): servis `icon-data`/`icon-name`'i güvenli çözücüyle arka
    /// planda çözer; hazır olunca yeni `MenuLayout` gelir. Yoksa `None`.
    pub image: Option<Pixmap>,
    /// `None`: işaretlenebilir öğe değil.
    pub toggle_kind: Option<ToggleKind>,
    pub toggle_state: ToggleState,
    /// `children-display = "submenu"`. Çocuklar henüz boşsa alt menü açılırken
    /// `menu_about_to_show` ile istenmelidir.
    pub submenu: bool,
    /// Kısayollar; her biri tuş adları dizisi, ör. `[["Control", "q"]]`.
    pub shortcuts: Vec<Vec<String>>,
    pub children: Vec<MenuNode>,
}

impl MenuNode {
    pub fn new(id: i32) -> Self {
        Self {
            id,
            kind: MenuItemKind::Standard,
            label: String::new(),
            mnemonic: None,
            enabled: true,
            visible: true,
            icon_name: String::new(),
            icon_png: None,
            image: None,
            toggle_kind: None,
            toggle_state: ToggleState::Indeterminate,
            submenu: false,
            shortcuts: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn is_separator(&self) -> bool {
        self.kind == MenuItemKind::Separator
    }

    pub fn has_submenu(&self) -> bool {
        self.submenu || !self.children.is_empty()
    }

    /// İşaretli mi (onay kutusu / radyo düğmesi açık).
    pub fn is_checked(&self) -> bool {
        self.toggle_kind.is_some() && self.toggle_state == ToggleState::On
    }

    /// Bu düğüm ve altındakiler arasında `id`'yi arar.
    pub fn find(&self, id: i32) -> Option<&MenuNode> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    pub fn find_mut(&mut self, id: i32) -> Option<&mut MenuNode> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter_mut().find_map(|c| c.find_mut(id))
    }

    /// Alt ağaçtaki düğüm sayısı (kendisi dahil).
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(MenuNode::count).sum::<usize>()
    }

    /// Bir özelliği uygular. Yanlış tip o özelliği varsayılanına döndürür; bilinmeyen anahtar
    /// yok sayılır.
    pub(crate) fn set_property(&mut self, key: &str, value: &Value<'_>) {
        match key {
            "type" => {
                self.kind = match as_str(value) {
                    Some("separator") => MenuItemKind::Separator,
                    _ => MenuItemKind::Standard,
                }
            }
            "label" => (self.label, self.mnemonic) = parse_label(as_str(value).unwrap_or_default()),
            "enabled" => self.enabled = as_bool(value).unwrap_or(true),
            "visible" => self.visible = as_bool(value).unwrap_or(true),
            "icon-name" => self.icon_name = clip_line(as_str(value).unwrap_or_default(), MAX_LABEL_CHARS),
            "icon-data" => {
                self.icon_png = as_bytes(value, MAX_MENU_ICON_BYTES)
                    .filter(|b| b.starts_with(b"\x89PNG\r\n\x1a\n"))
                    .map(Into::into)
            }
            "toggle-type" => {
                self.toggle_kind = match as_str(value) {
                    Some("checkmark") => Some(ToggleKind::Checkmark),
                    Some("radio") => Some(ToggleKind::Radio),
                    _ => None,
                }
            }
            "toggle-state" => {
                self.toggle_state = match as_i64(value) {
                    Some(0) => ToggleState::Off,
                    Some(1) => ToggleState::On,
                    _ => ToggleState::Indeterminate,
                }
            }
            "children-display" => self.submenu = as_str(value) == Some("submenu"),
            "shortcut" => self.shortcuts = parse_shortcuts(value),
            _ => {}
        }
    }

    /// Özelliği varsayılanına döndürür (`ItemsPropertiesUpdated` "removed" listesi).
    pub(crate) fn reset_property(&mut self, key: &str) {
        let d = MenuNode::new(self.id);
        match key {
            "type" => self.kind = d.kind,
            "label" => (self.label, self.mnemonic) = (d.label, d.mnemonic),
            "enabled" => self.enabled = d.enabled,
            "visible" => self.visible = d.visible,
            "icon-name" => self.icon_name = d.icon_name,
            "icon-data" => self.icon_png = d.icon_png,
            "toggle-type" => self.toggle_kind = d.toggle_kind,
            "toggle-state" => self.toggle_state = d.toggle_state,
            "children-display" => self.submenu = d.submenu,
            "shortcut" => self.shortcuts = d.shortcuts,
            _ => {}
        }
    }

    /// Ağacı sınırlara indirir: derinlik ve toplam düğüm. `budget` kalan düğüm hakkıdır.
    fn enforce_limits(&mut self, depth: usize, budget: &mut usize) {
        *budget = budget.saturating_sub(1);
        if depth + 1 >= MAX_MENU_DEPTH {
            self.children.clear();
            return;
        }
        let mut keep = 0;
        for child in &mut self.children {
            if *budget == 0 {
                break;
            }
            child.enforce_limits(depth + 1, budget);
            keep += 1;
        }
        self.children.truncate(keep);
    }
}

/// `_` mnemonik işaretini temizler: `_Aç` → (`Aç`, Some('a')), `__` → `_`; sondaki tek `_`
/// olduğu gibi kalır. Denetim karakterleri atılır, uzunluk sınırlanır.
pub fn parse_label(raw: &str) -> (String, Option<char>) {
    let mut label = String::new();
    let mut mnemonic = None;
    let mut chars = raw.chars().filter(|c| !c.is_control()).peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            match chars.next() {
                Some('_') => label.push('_'),
                Some(next) => {
                    if mnemonic.is_none() {
                        mnemonic = next.to_lowercase().next();
                    }
                    label.push(next);
                }
                None => label.push('_'),
            }
        } else {
            label.push(c);
        }
        if label.chars().count() >= MAX_LABEL_CHARS {
            break;
        }
    }
    (label, mnemonic)
}

fn parse_shortcuts(value: &Value<'_>) -> Vec<Vec<String>> {
    let Some(list) = as_array(value) else { return Vec::new() };
    list.iter()
        .take(MAX_SHORTCUTS)
        .filter_map(|keys| {
            let keys: Vec<String> = as_array(keys)?
                .iter()
                .take(MAX_SHORTCUT_KEYS)
                .filter_map(as_str)
                .map(|k| clip_line(k, 32))
                .filter(|k| !k.is_empty())
                .collect();
            (!keys.is_empty()).then_some(keys)
        })
        .collect()
}

/// Bir menünün anlık görüntüsü.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuTree {
    /// `GetLayout` revizyonu.
    pub revision: u32,
    pub root: MenuNode,
}

/// `GetLayout` cevabının gövdesi: `(u (ia{sv}av))`.
pub(crate) type LayoutReply = (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>));
/// `ItemsPropertiesUpdated` gövdesi: `(a(ia{sv}) a(ias))`.
pub(crate) type PropertiesUpdate = (Vec<(i32, HashMap<String, OwnedValue>)>, Vec<(i32, Vec<String>)>);

/// Bir `(ia{sv}av)` yapısından düğüm kurar. `budget` biterse çocuklar kırpılır.
fn node_from_parts<'a, 'v: 'a>(
    id: i32,
    props: impl Iterator<Item = (&'a str, &'a Value<'v>)>,
    children: impl Iterator<Item = &'a Value<'v>>,
    depth: usize,
    budget: &mut usize,
) -> MenuNode {
    *budget = budget.saturating_sub(1);
    let mut node = MenuNode::new(id);
    for (key, value) in props {
        node.set_property(key, value);
    }
    if depth + 1 < MAX_MENU_DEPTH {
        for child in children {
            if *budget == 0 {
                break;
            }
            if let Some(child) = node_from_value(child, depth + 1, budget) {
                node.children.push(child);
            }
        }
    }
    node
}

fn node_from_value(value: &Value<'_>, depth: usize, budget: &mut usize) -> Option<MenuNode> {
    let [id, props, children] = as_fields(value)? else { return None };
    let id = i32::try_from(as_i64(id)?).ok()?;
    let props: Vec<(&str, &Value<'_>)> = match peel(props) {
        Value::Dict(d) => d.iter().filter_map(|(k, v)| Some((as_str(k)?, v))).collect(),
        _ => Vec::new(),
    };
    let children = as_array(children).unwrap_or_default();
    Some(node_from_parts(id, props.into_iter(), children.iter(), depth, budget))
}

impl MenuTree {
    /// `GetLayout` cevabından ağaç kurar.
    pub(crate) fn from_layout(reply: &LayoutReply) -> Self {
        let (revision, (id, props, children)) = reply;
        let mut budget = MAX_MENU_NODES;
        let root = node_from_parts(
            *id,
            props.iter().map(|(k, v)| (k.as_str(), &**v)),
            children.iter().map(|v| &**v),
            0,
            &mut budget,
        );
        if budget == 0 {
            tracing::warn!(limit = MAX_MENU_NODES, "tepsi menüsü çok büyük, kırpıldı");
        }
        Self { revision: *revision, root }
    }

    pub fn find(&self, id: i32) -> Option<&MenuNode> {
        self.root.find(id)
    }

    /// Bir alt ağacı (`GetLayout(parent)` cevabı) yerine koyar. `parent` bulunamazsa
    /// `false` döner (çağıran tam yeniden okuma yapmalı).
    pub(crate) fn splice(&mut self, subtree: MenuTree) -> bool {
        let id = subtree.root.id;
        let Some(node) = self.root.find_mut(id) else { return false };
        *node = subtree.root;
        self.revision = subtree.revision;
        let mut budget = MAX_MENU_NODES;
        self.root.enforce_limits(0, &mut budget);
        true
    }

    /// `ItemsPropertiesUpdated` / `GetGroupProperties` değişikliklerini uygular. Değişiklik
    /// olduysa `true`.
    pub(crate) fn patch(&mut self, update: &PropertiesUpdate) -> bool {
        let before = self.clone();
        let (updated, removed) = update;
        for (id, props) in updated.iter().take(MAX_MENU_NODES) {
            if let Some(node) = self.root.find_mut(*id) {
                for (key, value) in props {
                    node.set_property(key, value);
                }
            }
        }
        for (id, keys) in removed.iter().take(MAX_MENU_NODES) {
            if let Some(node) = self.root.find_mut(*id) {
                for key in keys.iter().take(32) {
                    node.reset_property(key);
                }
            }
        }
        *self != before
    }
}

#[cfg(test)]
mod tests {
    use zbus::zvariant::{Array, Dict, Signature, Structure};

    use super::*;

    fn dict(entries: &[(&str, Value<'static>)]) -> Dict<'static, 'static> {
        let mut d = Dict::new(&Signature::Str, &Signature::Variant);
        for (k, v) in entries {
            d.append(Value::from(k.to_string()), Value::Value(Box::new(v.try_clone().unwrap()))).unwrap();
        }
        d
    }

    fn node(id: i32, entries: &[(&str, Value<'static>)], children: Vec<Value<'static>>) -> Value<'static> {
        let mut arr = Array::new(&Signature::Variant);
        for c in children {
            arr.append(Value::Value(Box::new(c))).unwrap();
        }
        Value::Structure(
            zbus::zvariant::StructureBuilder::new()
                .add_field(id)
                .append_field(Value::Dict(dict(entries)))
                .append_field(Value::Array(arr))
                .build()
                .unwrap(),
        )
    }

    fn reply(children: Vec<Value<'static>>) -> LayoutReply {
        let children = children.into_iter().map(|v| OwnedValue::try_from(Value::Value(Box::new(v))).unwrap()).collect();
        (7, (0, HashMap::from([("children-display".into(), OwnedValue::from(zbus::zvariant::Str::from("submenu")))]), children))
    }

    #[test]
    fn labels_lose_mnemonics() {
        assert_eq!(parse_label("_Aç"), ("Aç".into(), Some('a')));
        assert_eq!(parse_label("Save __as_X"), ("Save _asX".into(), Some('x')));
        assert_eq!(parse_label("end_"), ("end_".into(), None));
        assert_eq!(parse_label("a\u{0}b"), ("ab".into(), None));
        assert_eq!(parse_label(&"y".repeat(5000)).0.chars().count(), MAX_LABEL_CHARS);
    }

    #[test]
    fn layout_is_parsed_with_properties() {
        let png = {
            let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
            b.extend([1, 2, 3]);
            b
        };
        let shortcut = Value::from(vec![vec!["Control", "q"]]);
        let tree = MenuTree::from_layout(&reply(vec![
            node(
                1,
                &[
                    ("label", Value::from("_Wi-Fi")),
                    ("toggle-state", Value::from(1i32)),
                    ("toggle-type", Value::from("checkmark")),
                    ("icon-data", Value::Array(Array::from(png.clone()))),
                    ("shortcut", shortcut),
                ],
                vec![],
            ),
            node(2, &[("type", Value::from("separator"))], vec![]),
            node(
                3,
                &[("label", Value::from("Ağlar")), ("children-display", Value::from("submenu")), ("enabled", Value::from(false))],
                vec![node(4, &[("label", Value::from("ev")), ("toggle-type", Value::from("radio")), ("visible", Value::from(false))], vec![])],
            ),
            node(5, &[("icon-data", Value::Array(Array::from(vec![1u8, 2])))], vec![]),
        ]));
        assert_eq!(tree.revision, 7);
        assert!(tree.root.has_submenu());
        assert_eq!(tree.root.children.len(), 4);
        let wifi = tree.find(1).unwrap();
        assert_eq!((wifi.label.as_str(), wifi.mnemonic), ("Wi-Fi", Some('w')));
        assert_eq!(wifi.toggle_kind, Some(ToggleKind::Checkmark));
        assert!(wifi.is_checked());
        assert_eq!(wifi.icon_png.as_deref(), Some(&png[..]));
        assert_eq!(wifi.shortcuts, [["Control", "q"]]);
        assert!(tree.find(2).unwrap().is_separator());
        let nets = tree.find(3).unwrap();
        assert!(!nets.enabled && nets.submenu);
        let home = tree.find(4).unwrap();
        assert!(!home.visible);
        assert_eq!((home.toggle_kind, home.toggle_state), (Some(ToggleKind::Radio), ToggleState::Indeterminate));
        assert_eq!(tree.find(5).unwrap().icon_png, None, "PNG olmayan ikon verisi atılır");
    }

    fn chain(depth: usize) -> Value<'static> {
        let mut v = node(depth as i32 + 100, &[], vec![]);
        for d in (1..depth).rev() {
            v = node(d as i32 + 100, &[], vec![v]);
        }
        v
    }

    #[test]
    fn depth_and_node_count_are_limited() {
        let tree = MenuTree::from_layout(&reply(vec![chain(30)]));
        let mut depth = 0;
        let mut n = &tree.root;
        while let Some(c) = n.children.first() {
            depth += 1;
            n = c;
        }
        assert_eq!(depth + 1, MAX_MENU_DEPTH);

        let wide = (1..5000).map(|i| node(i, &[], vec![])).collect();
        let tree = MenuTree::from_layout(&reply(wide));
        assert_eq!(tree.root.count(), MAX_MENU_NODES);
    }

    #[test]
    fn malformed_children_are_skipped() {
        let mut r = reply(vec![node(1, &[], vec![])]);
        r.1.2.push(OwnedValue::from(5i32));
        r.1.2.push(OwnedValue::try_from(Value::Structure(Structure::from((1i32, 2i32)))).unwrap());
        let tree = MenuTree::from_layout(&r);
        assert_eq!(tree.root.children.len(), 1);
    }

    #[test]
    fn patch_and_splice() {
        let mut tree = MenuTree::from_layout(&reply(vec![
            node(1, &[("label", Value::from("a")), ("enabled", Value::from(false))], vec![]),
            node(3, &[("children-display", Value::from("submenu"))], vec![]),
        ]));
        let update: PropertiesUpdate = (
            vec![(1, HashMap::from([("label".into(), OwnedValue::from(zbus::zvariant::Str::from("_b")))])), (99, HashMap::new())],
            vec![(1, vec!["enabled".into()])],
        );
        assert!(tree.patch(&update));
        let one = tree.find(1).unwrap();
        assert_eq!((one.label.as_str(), one.enabled), ("b", true));
        assert!(!tree.patch(&update), "aynı değişiklik ikinci kez değişiklik sayılmaz");

        let sub = MenuTree::from_layout(&(9, (3, HashMap::new(), vec![OwnedValue::try_from(node(30, &[("label", Value::from("x"))], vec![])).unwrap()])));
        assert!(tree.splice(sub));
        assert_eq!(tree.revision, 9);
        assert_eq!(tree.find(30).unwrap().label, "x");
        let missing = MenuTree { revision: 1, root: MenuNode::new(77) };
        assert!(!tree.splice(missing));
    }
}
