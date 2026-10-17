//! Ekran klavyesi kartı (t125): tek anahtar satırı (alt metinde durum: "Kapalı" / "Açık · Türkçe Q").
//! Compositor sanal klavye protokolünü sunmuyorsa kart gizlidir.

use std::time::Instant;

use nuxut::i18n::t;
use nuxut::ui::{Icon, IconName, List, ListEvent, ListItem, ListTrailing, Ui};
use nuxut::{CursorShape, Point, PointerEvent, Rect, TextSystem, Theme};

use super::padding;
use crate::model::ShellModel;
use crate::services::osk::{Command, Layout, OskView};
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

fn layout_name(layout: Layout) -> String {
    match layout {
        Layout::Tr => t!("shell-quick-osk-layout-tr"),
        Layout::Us => t!("shell-quick-osk-layout-us"),
    }
}

fn status_text(view: &OskView) -> String {
    if view.visible {
        t!("shell-quick-osk-on", layout = layout_name(view.layout))
    } else {
        t!("shell-quick-osk-off")
    }
}

fn build_item(theme: &Theme, view: &OskView) -> ListItem {
    ListItem::new(theme, t!("shell-quick-osk-title"))
        .with_leading(theme, Icon::named(IconName::Keyboard))
        .with_subtitle(theme, status_text(view))
        .with_trailing(theme, ListTrailing::Toggle(view.visible))
}

#[derive(Debug)]
pub struct OskCard {
    view: OskView,
    list: List,
}

impl OskCard {
    pub fn new(cx: &WidgetCx) -> Self {
        let view = OskView::default();
        let mut list = List::new(cx.theme);
        list.items_mut().push(build_item(cx.theme, &view));
        Self { view, list }
    }

    fn list_rect(&self, rect: Rect, theme: &Theme) -> Rect {
        let p = padding(theme);
        let h = self.list.items().first().map_or(0.0, |i| i.height(theme));
        Rect::new(rect.x + p, rect.y + p, rect.width - 2.0 * p, h)
    }
}

impl PanelWidget for OskCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Osk
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let view = model.osk;
        if view == self.view {
            return false;
        }
        self.view = view;
        if let Some(item) = self.list.items_mut().first_mut() {
            item.set_subtitle(cx.theme, Some(&status_text(&view)));
            item.set_toggle(view.visible, cx.now);
        }
        true
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.list.set_theme(cx.theme, cx.now);
    }

    fn height(&mut self, _text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        if !self.view.available {
            return Some(0.0);
        }
        Some(self.list_rect(Rect::new(0.0, 0.0, width, 0.0), cx.theme).bottom() + padding(cx.theme))
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        if !self.view.available {
            return;
        }
        let list = self.list_rect(rect, cx.theme);
        if self.list.rect() != list {
            self.list.set_rect(cx.theme, list, cx.now);
        }
        self.list.draw(ui);
    }

    fn pointer(&mut self, event: &PointerEvent, rect: Rect, _text: &mut TextSystem, cx: &WidgetCx) -> (bool, Option<Action>) {
        if !self.view.available {
            return (false, None);
        }
        let list = self.list_rect(rect, cx.theme);
        if self.list.rect() != list {
            self.list.set_rect(cx.theme, list, cx.now);
        }
        let r = self.list.pointer(cx.theme, event, cx.now);
        let cmd = r.event.map(|(_, e)| match e {
            ListEvent::Toggled(true) => Command::Show,
            ListEvent::Toggled(false) => Command::Hide,
            _ => Command::Toggle,
        });
        (r.repaint || cmd.is_some(), cmd.map(Action::Osk))
    }

    /// Göster/gizle anahtarı.
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        if !self.view.available {
            return;
        }
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            self.list.access(tree, "switch", |i| i).label(&self.kind().title());
        });
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        if !self.view.available {
            return None;
        }
        self.list.cursor(p)
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lines() {
        crate::i18n::use_turkish_for_tests();
        let off = OskView { available: true, visible: false, layout: Layout::Tr };
        let on = OskView { available: true, visible: true, layout: Layout::Us };
        assert_eq!(status_text(&off), "Kapalı");
        assert_eq!(status_text(&on), "Açık · İngilizce (ABD)");
    }
}
