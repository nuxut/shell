//! Tepsi durumunun UI tarafındaki kopyası: olaylardan beslenen saf veri yapısı.

use std::collections::HashMap;

use super::{ItemId, MenuRequest, MenuTree, Status, TrayEvent, TrayItem, TrayRole};

/// Öğeler kayıt sırasına göre (kararlı) dizilidir; açık menüler öğe kimliğine göre.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrayModel {
    items: Vec<TrayItem>,
    /// Son okunan menü ağacı ve ait olduğu açılış.
    menus: HashMap<ItemId, (MenuRequest, MenuTree)>,
    role: Option<TrayRole>,
    /// Son `MenuRequested` (istek kimliği, öğe).
    menu_request: Option<(MenuRequest, ItemId)>,
}

impl TrayModel {
    /// Olayı uygular; model değiştiyse `true`.
    pub fn apply(&mut self, event: &TrayEvent) -> bool {
        match event {
            TrayEvent::Role(role) => self.role.replace(*role) != Some(*role),
            TrayEvent::Failed(_) => {
                let changed = !self.items.is_empty() || self.role.is_some();
                *self = Self::default();
                changed
            }
            TrayEvent::ItemAdded(item) | TrayEvent::ItemUpdated(item) => self.upsert(item),
            TrayEvent::ItemRemoved(id) => {
                self.menus.remove(id);
                let before = self.items.len();
                self.items.retain(|i| &i.id != id);
                self.items.len() != before
            }
            TrayEvent::MenuLayout { item, request, menu } => {
                if self.get(item).is_none() {
                    return false;
                }
                // Ağaç aynı olsa da yeni açılışın yanıtı değişikliktir: UI bekleyen açılışı
                // ancak bu yanıtla gösterir (aynı menüyü ikinci kez açmak).
                let new = (*request, menu.clone());
                self.menus.insert(item.clone(), new.clone()).as_ref() != Some(&new)
            }
            TrayEvent::MenuRequested { item, request } => {
                if self.get(item).is_none() {
                    return false;
                }
                self.menu_request = Some((*request, item.clone()));
                true
            }
        }
    }

    fn upsert(&mut self, item: &TrayItem) -> bool {
        if let Some(existing) = self.items.iter_mut().find(|i| i.id == item.id) {
            if existing == item {
                return false;
            }
            *existing = item.clone();
            return true;
        }
        let at = self.items.partition_point(|i| i.seq < item.seq);
        self.items.insert(at, item.clone());
        true
    }

    /// Son servis kaynaklı menü açma isteği (`Activate` desteklenmeyen öğe): `(istek, öğe)`.
    /// UI son gördüğü istekten farklıysa menüyü bu istek kimliğiyle açar.
    pub fn menu_request(&self) -> Option<(MenuRequest, &ItemId)> {
        self.menu_request.as_ref().map(|(n, id)| (*n, id))
    }

    pub fn role(&self) -> Option<TrayRole> {
        self.role
    }

    /// Tüm öğeler, kayıt sırasıyla.
    pub fn items(&self) -> &[TrayItem] {
        &self.items
    }

    /// Gösterilecek öğeler (`Passive` olanlar hariç).
    pub fn visible(&self) -> impl Iterator<Item = &TrayItem> {
        self.items.iter().filter(|i| i.status != Status::Passive)
    }

    pub fn get(&self, id: &ItemId) -> Option<&TrayItem> {
        self.items.iter().find(|i| &i.id == id)
    }

    /// Son okunan menü ağacı (menü kapatılsa da kalır; yeniden açılınca güncellenir).
    pub fn menu(&self, id: &ItemId) -> Option<&MenuTree> {
        self.menus.get(id).map(|(_, tree)| tree)
    }

    /// Son okunan menü ağacı ve ait olduğu açılış (`TrayHandle::open_menu` isteği).
    pub fn menu_layout(&self, id: &ItemId) -> Option<(MenuRequest, &MenuTree)> {
        self.menus.get(id).map(|(request, tree)| (*request, tree))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::menu::MenuNode;
    use super::*;

    fn item(owner: &str, seq: u64, status: &str) -> TrayItem {
        let id = ItemId::new(owner, "/SNI");
        let mut props = HashMap::new();
        props.insert("Status".to_owned(), zbus::zvariant::OwnedValue::from(zbus::zvariant::Str::from(status.to_owned())));
        TrayItem::from_properties(id, seq, owner, "/SNI", &props)
    }

    #[test]
    fn menu_requests_are_sequenced() {
        let mut m = TrayModel::default();
        let it = item(":1.1", 1, "Active");
        let id = it.id.clone();
        m.apply(&TrayEvent::ItemAdded(it));
        assert!(m.apply(&TrayEvent::MenuRequested { item: id.clone(), request: 7 }));
        assert_eq!(m.menu_request(), Some((7, &id)));
        assert!(m.apply(&TrayEvent::MenuRequested { item: id.clone(), request: 8 }));
        assert_eq!(m.menu_request(), Some((8, &id)), "aynı öğe için tekrar istek ayırt edilir");
    }

    /// Kök neden regresyonu: aynı menüyü ikinci kez açmak aynı ağacı getirir; yeni açılışın
    /// yanıtı yine de değişiklik sayılmalı (yoksa UI uyanmaz, menü açılmaz).
    #[test]
    fn same_tree_for_a_new_request_is_a_change() {
        let mut m = TrayModel::default();
        let it = item(":1.1", 1, "Active");
        let id = it.id.clone();
        m.apply(&TrayEvent::ItemAdded(it));
        let menu = MenuTree { revision: 1, root: MenuNode::new(0) };
        assert!(m.apply(&TrayEvent::MenuLayout { item: id.clone(), request: 1, menu: menu.clone() }));
        assert!(!m.apply(&TrayEvent::MenuLayout { item: id.clone(), request: 1, menu: menu.clone() }), "aynı açılış, aynı ağaç");
        assert!(m.apply(&TrayEvent::MenuLayout { item: id.clone(), request: 2, menu: menu.clone() }), "yeni açılış");
        assert_eq!(m.menu_layout(&id), Some((2, &menu)));
    }

    #[test]
    fn keeps_registration_order_and_dedups() {
        let mut m = TrayModel::default();
        assert!(m.apply(&TrayEvent::ItemAdded(item(":1.3", 3, "Active"))));
        assert!(m.apply(&TrayEvent::ItemAdded(item(":1.1", 1, "Passive"))));
        assert!(m.apply(&TrayEvent::ItemAdded(item(":1.2", 2, "Active"))));
        let order: Vec<u64> = m.items().iter().map(|i| i.seq).collect();
        assert_eq!(order, [1, 2, 3]);
        assert_eq!(m.visible().count(), 2);
        assert!(!m.apply(&TrayEvent::ItemUpdated(item(":1.2", 2, "Active"))), "aynı veri değişiklik değil");
        assert!(m.apply(&TrayEvent::ItemUpdated(item(":1.2", 2, "Passive"))));
        assert_eq!(m.items().len(), 3);
        assert_eq!(m.visible().count(), 1);
    }

    #[test]
    fn menus_follow_items() {
        let mut m = TrayModel::default();
        let it = item(":1.1", 1, "Active");
        let id = it.id.clone();
        let menu = MenuTree { revision: 1, root: MenuNode::new(0) };
        assert!(!m.apply(&TrayEvent::MenuLayout { item: id.clone(), request: 1, menu: menu.clone() }), "bilinmeyen öğe");
        m.apply(&TrayEvent::ItemAdded(it));
        assert!(m.apply(&TrayEvent::MenuLayout { item: id.clone(), request: 1, menu: menu.clone() }));
        assert!(!m.apply(&TrayEvent::MenuLayout { item: id.clone(), request: 1, menu }));
        assert!(m.menu(&id).is_some());
        assert!(m.apply(&TrayEvent::ItemRemoved(id.clone())));
        assert!(m.menu(&id).is_none());
        assert!(!m.apply(&TrayEvent::ItemRemoved(id.clone())));
        assert!(!m.apply(&TrayEvent::MenuRequested { item: id.clone(), request: 3 }), "bilinmeyen öğe");
        assert_eq!(m.menu_request(), None);
        assert!(m.apply(&TrayEvent::Role(TrayRole::Host)));
        assert_eq!(m.role(), Some(TrayRole::Host));
    }
}
