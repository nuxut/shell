//! Tepsi verisinden UI'ya köprü (saf fonksiyonlar): dbusmenu ağacı → `nuxut::ui::MenuModel`,
//! menü olayları → servis komutları, tepsi öğesi → çizilecek görüntü.
//!
//! Görsel yükleme burada yapılmaz ve uygulamanın verdiği yollar açılmaz: ikonlar serviste
//! doğrulanıp çözülür (`TrayItem::image`), UI yalnızca pikselleri dokuya yükler.
//! [`Pixmap`] düz RGBA8'dir → `nuxut_image::Image::from_straight`.

use nuxut::ui::{Icon, MenuEvent, MenuItem, MenuItemId, MenuItemKind, MenuModel};

use super::item::{Pixmap, TrayItem};
use super::menu::{MenuNode, MenuTree, ToggleKind};
use super::{ItemId, MenuEventKind, TrayHandle};

/// `nuxut-ui` etiket sınırı (bayt).
const MAX_LABEL_BYTES: usize = nuxut::ui::MAX_MENU_LABEL;
/// `nuxut-ui` kısayol metni sınırı (bayt).
const MAX_SHORTCUT_BYTES: usize = nuxut::ui::MAX_MENU_SHORTCUT;

/// dbusmenu kimliği → menü öğe kimliği (kayıpsız; negatif kimlikler de korunur).
pub fn menu_item_id(dbus_id: i32) -> MenuItemId {
    MenuItemId::from(dbus_id as u32)
}

/// Menü öğe kimliği → dbusmenu kimliği. Bu modülün üretmediği (32 biti aşan) kimlik `None`.
pub fn dbus_menu_id(id: MenuItemId) -> Option<i32> {
    u32::try_from(id).ok().map(|v| v as i32)
}

/// Menü ağacını UI modeline çevirir. Gizli öğeler atlanır; baştaki, sondaki ve art arda
/// gelen ayraçlar teke indirilir. Alt menü çocukları henüz okunmadıysa boş `Submenu` olur
/// (UI açınca [`MenuEvent::SubmenuOpened`] → [`apply_menu_event`] ile istenir).
pub fn menu_model(tree: &MenuTree) -> MenuModel {
    menu_model_with(tree, &mut |_| None)
}

/// [`menu_model`], öğe ikonlarıyla: `icon` her görünür düğüm için çağrılır. UI, servisin
/// güvenle çözdüğü [`MenuNode::image`]'ı dokuya yükleyip [`Icon::image`] ya da
/// ([`is_symbolic`] ise) [`Icon::symbolic`] döndürür.
pub fn menu_model_with(tree: &MenuTree, icon: &mut dyn FnMut(&MenuNode) -> Option<Icon>) -> MenuModel {
    MenuModel::new(convert_children(&tree.root, icon))
}

/// `icon-name` `-symbolic` ile bitiyorsa ikon tek renklidir (temanın içerik rengiyle boyanır).
pub fn is_symbolic(node: &MenuNode) -> bool {
    node.icon_name.ends_with("-symbolic")
}

fn convert_children(node: &MenuNode, icon: &mut dyn FnMut(&MenuNode) -> Option<Icon>) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = Vec::with_capacity(node.children.len());
    for child in node.children.iter().filter(|c| c.visible) {
        let item = convert(child, icon);
        let is_sep = item.kind == MenuItemKind::Separator;
        let prev_sep = items.last().is_none_or(|p| p.kind == MenuItemKind::Separator);
        if is_sep && prev_sep {
            continue;
        }
        items.push(item);
    }
    if items.last().is_some_and(|p| p.kind == MenuItemKind::Separator) {
        items.pop();
    }
    items
}

fn convert(node: &MenuNode, icon: &mut dyn FnMut(&MenuNode) -> Option<Icon>) -> MenuItem {
    let id = menu_item_id(node.id);
    if node.is_separator() {
        return MenuItem { id, ..MenuItem::separator() };
    }
    let label = clip_bytes(&node.label, MAX_LABEL_BYTES);
    let kind = if node.has_submenu() {
        MenuItemKind::Submenu(convert_children(node, icon))
    } else {
        match node.toggle_kind {
            Some(ToggleKind::Checkmark) => MenuItemKind::Check(node.is_checked()),
            Some(ToggleKind::Radio) => MenuItemKind::Radio(node.is_checked()),
            None => MenuItemKind::Action,
        }
    };
    MenuItem { id, label, icon: icon(node), shortcut: shortcut_text(node), enabled: node.enabled, kind }
}

/// İlk kısayol dizisini okunur metne çevirir: `["Control", "q"]` → `Ctrl+Q`.
fn shortcut_text(node: &MenuNode) -> Option<String> {
    let keys = node.shortcuts.first()?;
    let text = keys
        .iter()
        .map(|k| match k.as_str() {
            "Control" => "Ctrl".to_owned(),
            "Super" => "Super".to_owned(),
            k if k.chars().count() == 1 => k.to_uppercase(),
            k => k.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("+");
    (!text.is_empty()).then(|| clip_bytes(&text, MAX_SHORTCUT_BYTES))
}

fn clip_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// UI menü olayını servise iletir. Gönderilen komut sayısını döndürür (test/log için).
/// - `Activated(id)` → dbusmenu `Event(id, "clicked")`; menü kapanır, izleme bırakılır.
/// - `SubmenuOpened(id)` → `Event(id, "opened")` + `AboutToShow(id)` (gerekirse alt ağaç
///   yeniden okunur ve yeni `MenuLayout` gelir → [`menu_model`] ile `Menu::set_model`).
/// - `Dismissed` → `Event(0, "closed")` + izleme bırakılır.
pub fn apply_menu_event(handle: &TrayHandle, item: &ItemId, event: &MenuEvent) -> usize {
    let sent = |ok: bool| usize::from(ok);
    match event {
        MenuEvent::Activated(id) => {
            let Some(id) = dbus_menu_id(*id) else { return 0 };
            sent(handle.menu_event(item, id, MenuEventKind::Clicked)) + sent(handle.close_menu(item))
        }
        MenuEvent::SubmenuOpened(id) => {
            let Some(id) = dbus_menu_id(*id) else { return 0 };
            sent(handle.menu_event(item, id, MenuEventKind::Opened)) + sent(handle.menu_about_to_show(item, &[id]))
        }
        MenuEvent::Dismissed => sent(handle.menu_event(item, 0, MenuEventKind::Closed)) + sent(handle.close_menu(item)),
    }
}

/// `size_px` (fiziksel piksel) için çizilecek görüntü: servisin güvenle çözdüğü
/// [`TrayItem::image`], yoksa uygulamanın pixmap'lerinden en uygunu; hiçbiri yoksa `None`
/// (UI yer tutucu çizer). UI hiçbir zaman uygulamanın verdiği ad ya da yolu kendisi açmaz:
/// `IconName`/`IconThemePath` güvenilmezdir ve yalnızca servisteki doğrulamalı çözücüden
/// geçer (`services::tray::icons`).
pub fn tray_icon_source(item: &TrayItem, size_px: u32) -> Option<&Pixmap> {
    item.image.as_ref().or_else(|| item.current_icon().pixmaps.best(size_px))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use zbus::zvariant::{OwnedValue, Str};

    use super::super::Command;
    use super::super::menu::{MenuItemKind as NodeKind, ToggleState};
    use super::*;

    fn n(id: i32, label: &str) -> MenuNode {
        MenuNode { label: label.into(), ..MenuNode::new(id) }
    }

    fn sep(id: i32) -> MenuNode {
        MenuNode { kind: NodeKind::Separator, ..MenuNode::new(id) }
    }

    fn tree(children: Vec<MenuNode>) -> MenuTree {
        MenuTree { revision: 1, root: MenuNode { children, ..MenuNode::new(0) } }
    }

    #[test]
    fn converts_kinds_flags_and_shortcuts() {
        let mut check = n(2, "Wi-Fi");
        check.toggle_kind = Some(ToggleKind::Checkmark);
        check.toggle_state = ToggleState::On;
        let mut radio = n(3, "Ev");
        radio.toggle_kind = Some(ToggleKind::Radio);
        radio.toggle_state = ToggleState::Indeterminate;
        let mut disabled = n(4, "Kapalı");
        disabled.enabled = false;
        let mut quit = n(5, "Çık");
        quit.shortcuts = vec![vec!["Control".into(), "q".into()]];
        let mut lazy = n(6, "Ağlar");
        lazy.submenu = true;
        let mut sub = n(7, "VPN");
        sub.children = vec![n(8, "wg0")];
        let model = menu_model(&tree(vec![n(1, "Aç"), check, radio, disabled, quit, lazy, sub]));

        let kinds: Vec<_> = model.items.iter().map(|i| (i.id, i.kind.clone())).collect();
        assert_eq!(kinds[0], (1, MenuItemKind::Action));
        assert_eq!(kinds[1], (2, MenuItemKind::Check(true)));
        assert_eq!(kinds[2], (3, MenuItemKind::Radio(false)));
        assert!(!model.items[3].enabled);
        assert_eq!(model.items[4].shortcut.as_deref(), Some("Ctrl+Q"));
        assert_eq!(model.items[5].kind, MenuItemKind::Submenu(Vec::new()), "tembel alt menü");
        assert_eq!(model.items[6].kind, MenuItemKind::Submenu(vec![MenuItem::action(8, "wg0")]));
        assert_eq!(model.find(8).unwrap().label, "wg0");
    }

    #[test]
    fn icons_come_from_callback_for_visible_nodes() {
        let mut a = n(1, "a");
        a.icon_name = "wifi-symbolic".into();
        let mut hidden = n(2, "gizli");
        hidden.visible = false;
        let mut seen = Vec::new();
        let model = menu_model_with(&tree(vec![a, hidden, n(3, "b")]), &mut |node| {
            seen.push(node.id);
            is_symbolic(node).then(|| Icon::named(nuxut::ui::IconName::Wifi))
        });
        assert_eq!(seen, [1, 3], "gizli düğüm için çağrılmaz");
        assert_eq!(model.items[0].icon, Some(Icon::named(nuxut::ui::IconName::Wifi)));
        assert_eq!(model.items[1].icon, None);
    }

    #[test]
    fn hidden_items_and_redundant_separators_are_dropped() {
        let mut hidden = n(3, "gizli");
        hidden.visible = false;
        let model = menu_model(&tree(vec![sep(1), n(2, "a"), sep(4), hidden, sep(5), n(6, "b"), sep(7)]));
        let labels: Vec<_> = model.items.iter().map(|i| (i.label.as_str(), i.kind == MenuItemKind::Separator)).collect();
        assert_eq!(labels, [("a", false), ("", true), ("b", false)]);
        assert!(menu_model(&tree(vec![sep(1), sep(2)])).items.is_empty());
    }

    #[test]
    fn ids_round_trip_and_labels_fit_ui_limits() {
        for id in [0, 1, 42, i32::MAX, -1, i32::MIN] {
            assert_eq!(dbus_menu_id(menu_item_id(id)), Some(id));
        }
        assert_eq!(dbus_menu_id(u64::from(u32::MAX) + 1), None);
        let model = menu_model(&tree(vec![n(1, &"ğ".repeat(200))]));
        assert!(model.items[0].label.len() <= MAX_LABEL_BYTES);
        assert!(model.items[0].label.chars().all(|c| c == 'ğ'));
    }

    #[test]
    fn menu_events_become_commands() {
        let (tx, rx) = async_channel::bounded(16);
        let handle = TrayHandle { tx };
        let item = ItemId::new(":1.2", "/SNI");
        assert_eq!(apply_menu_event(&handle, &item, &MenuEvent::Activated(menu_item_id(7))), 2);
        assert!(matches!(rx.try_recv(), Ok(Command::MenuEvent { id: 7, kind: MenuEventKind::Clicked, .. })));
        assert!(matches!(rx.try_recv(), Ok(Command::CloseMenu(_))));
        assert_eq!(apply_menu_event(&handle, &item, &MenuEvent::SubmenuOpened(3)), 2);
        assert!(matches!(rx.try_recv(), Ok(Command::MenuEvent { id: 3, kind: MenuEventKind::Opened, .. })));
        assert!(matches!(rx.try_recv(), Ok(Command::MenuAboutToShow { ref ids, .. }) if ids == &[3]));
        assert_eq!(apply_menu_event(&handle, &item, &MenuEvent::Dismissed), 2);
        assert!(matches!(rx.try_recv(), Ok(Command::MenuEvent { id: 0, kind: MenuEventKind::Closed, .. })));
        assert!(matches!(rx.try_recv(), Ok(Command::CloseMenu(_))));
        assert_eq!(apply_menu_event(&handle, &item, &MenuEvent::Activated(u64::MAX)), 0, "yabancı kimlik");
    }

    fn item(props: Vec<(&str, OwnedValue)>) -> TrayItem {
        let props: HashMap<String, OwnedValue> = props.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        TrayItem::from_properties(ItemId::new(":1.2", "/SNI"), 0, ":1.2", "/SNI", &props)
    }

    fn s(v: &str) -> OwnedValue {
        OwnedValue::from(Str::from(v.to_owned()))
    }

    fn pixmaps(sizes: &[i32]) -> OwnedValue {
        let list: Vec<(i32, i32, Vec<u8>)> = sizes.iter().map(|&n| (n, n, vec![0xff; (n * n * 4) as usize])).collect();
        OwnedValue::try_from(zbus::zvariant::Value::from(list)).unwrap()
    }

    #[test]
    fn icon_source_is_pixels_only() {
        // Ad ve yol UI'ya verilmez; servis çözmediyse pixmap kullanılır.
        let it = item(vec![("IconName", s("/etc/passwd")), ("IconThemePath", s("/proc/self")), ("IconPixmap", pixmaps(&[16, 32]))]);
        assert_eq!(tray_icon_source(&it, 24).unwrap().width, 32);
        let it = item(vec![("IconPixmap", pixmaps(&[16, 22, 64]))]);
        assert_eq!(tray_icon_source(&it, 20).unwrap().width, 22);
        assert_eq!(tray_icon_source(&it, 128).unwrap().width, 64);
        assert!(tray_icon_source(&item(vec![("IconName", s("x"))]), 24).is_none());

        let mut it = item(vec![("IconName", s("x")), ("IconPixmap", pixmaps(&[64]))]);
        it.image = item(vec![("IconPixmap", pixmaps(&[8]))]).icon.pixmaps.best(8).cloned();
        assert_eq!(tray_icon_source(&it, 24).unwrap().width, 8, "servisin çözdüğü öncelikli");
    }
}
