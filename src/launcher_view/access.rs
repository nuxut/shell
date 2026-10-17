//! Başlatıcının ekran okuyucu ağacı (framework ADR-0011).
//!
//! Odak arama alanında kalır; seçili sonuç alanın etkin torunu (`active_descendant`): ok tuşuyla
//! gezinirken Orca seçili satırı okur. Satır anahtarı satırın kimliği (katalog girdisi, pano
//! öğesi), indis değil. Eylemler sayfasında odak seçili eylemde.

use std::time::Instant;

use nuxut::access::{AccessTree, PREVIEW_GRAPHEMES, Role, child_id, preview};
use nuxut::i18n::t;

use super::{LauncherMode, LauncherView, Page, Row};

impl LauncherView {
    pub fn access(&self, tree: &mut AccessTree, now: Instant) {
        let group = tree.push("launcher", Role::Dialog).label(&t!("shell-a11y-launcher")).bounds(self.rect).id();
        tree.within(group, |tree| match self.pages.current() {
            Page::Results => {
                let rows = &self.rows;
                let key = |i: usize| row_key(rows.get(i));
                let emoji = self.mode == LauncherMode::Emoji;
                let list_id = child_id(group, "results");
                let selected = (!emoji && !rows.is_empty()).then(|| self.list.access_selected(list_id, key)).flatten();
                let search = self.search.input().access(tree, "search").label(&t!("shell-a11y-launcher-search"));
                if let Some(sel) = selected {
                    search.active_descendant(sel);
                }
                if emoji {
                    return;
                }
                if rows.is_empty() {
                    self.empty.access(tree, "empty");
                } else {
                    self.list.access(tree, "results", now, key).label(&t!("shell-a11y-launcher-results"));
                }
                if self.mode == LauncherMode::Clipboard {
                    // Tehdit modeli (ADR-0011): pano öğesi ağaca tam metin değil kısa önizleme olarak.
                    for i in 0..rows.len() {
                        let id = child_id(list_id, key(i));
                        if let Some(full) = tree.node(id).and_then(|n| n.label()).map(str::to_owned)
                            && let Some(n) = tree.get(id)
                        {
                            n.label(&preview(&full, PREVIEW_GRAPHEMES));
                        }
                    }
                }
            }
            Page::Actions => {
                let actions = tree.push("actions", Role::Group).label(&t!("shell-a11y-launcher-actions")).id();
                tree.within(actions, |tree| {
                    let list = self.actions.access(tree, "list", now, |i| i).id();
                    if let Some(sel) = self.actions.access_selected(list, |i| i) {
                        tree.set_focus(sel);
                    }
                });
            }
        });
    }
}

/// Satırın kalıcı anahtarı (liste yeniden sıralanınca Orca odağı karışmasın).
fn row_key(row: Option<&Row>) -> String {
    match row {
        Some(Row::App(i)) => format!("app-{i}"),
        Some(Row::Clip(id)) => format!("clip-{id}"),
        Some(Row::Calc(_)) => "calc".to_owned(),
        None => String::new(),
    }
}
