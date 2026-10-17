//! Ekran okuyucu ağacı (framework ADR-0011): ana yüzeyin bar'ı ve açık başlatıcısı.
//!
//! Bar `Toolbar`; her widget [`BarWidget::access_items`] bölgeleri kadar düğme (çok bölgeli
//! widget bir grup altında). AT tıklaması, düğümün ortasına gerçek işaretçi tıklamasıyla aynı
//! yoldan gider ([`Screen::access_point`]): widget'lar için ayrı eylem kodu yok. Ağaç içeriği
//! loglanmaz (değişmez 9).

use std::time::Instant;

use nuxut::access::{AccessRequest, AccessTree, Action as AxAction, NodeId, Role};
use nuxut::i18n::t;
use nuxut::{Point, Rect, TextSystem, Theme};

use super::Screen;
use crate::config::ShellConfig;
use crate::widgets::{AccessItem, Action, PanelId};

impl Screen {
    /// Yüzeyin ağacını kurar (çizimden sonra; yerleşim güncel) ve tıklama noktalarını saklar.
    pub fn access(&mut self, tree: &mut AccessTree, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) {
        tree.root().label(&t!("shell-a11y-main"));
        let g = self.geometry(text, config, theme, now);
        let cx = self.cx(config, theme, now);
        let visible: Vec<(usize, Rect)> =
            g.items.iter().copied().enumerate().filter(|(_, r)| r.width > 0.0 && r.height > 0.0).collect();
        if !self.bar_hidden && !visible.is_empty() {
            let bounds = visible.iter().map(|(_, r)| *r).reduce(|a, b| a.union(&b)).unwrap_or_default();
            let bar = tree.push("bar", Role::Toolbar).label(&t!("shell-a11y-bar")).bounds(bounds).id();
            tree.within(bar, |tree| {
                for (i, r) in &visible {
                    let w = &self.bar[*i].1;
                    let items = w.access_items(*r, &cx);
                    let key = (*i, format!("{:?}", w.kind()));
                    match items.as_slice() {
                        [] => {}
                        [item] => push_item(tree, key, item),
                        _ => {
                            let group = tree.push(key, Role::Group).label(&w.kind().title()).bounds(*r).id();
                            tree.within(group, |tree| {
                                for item in &items {
                                    push_item(tree, item.key.as_str(), item);
                                }
                            });
                        }
                    }
                }
            });
        }
        for (i, edge) in crate::layout::Edge::ALL.into_iter().enumerate() {
            if !(self.drawers[i].is_open() && self.state.is_open(PanelId::Edge(edge))) {
                continue;
            }
            let panel = tree.push(("panel", i), Role::Pane).label(&t!("shell-a11y-panel")).bounds(self.drawers[i].target()).id();
            tree.within(panel, |tree| {
                for (w, r) in self.panels[i].iter().zip(&self.panel_rects[i]) {
                    w.access(tree, *r, &cx);
                }
            });
        }
        if self.state.is_open(PanelId::Clock) && self.calendar_area.width > 0.0 {
            let area = self.calendar_area;
            let clock = tree.push("clock", Role::Pane).label(&t!("shell-a11y-clock")).bounds(area).id();
            tree.within(clock, |tree| self.calendar.access(tree, area));
        }
        if self.state.is_open(PanelId::Launcher) {
            self.launcher.access(tree, now);
        }
        self.popups.access(tree, &t!("shell-a11y-notifications"));
        // OSD odak almaz: değeri canlı bölge olarak duyurulur ("Ses %64", "Büyüteç 2,0×").
        if self.osd.is_visible() && !self.osd_access.is_empty() {
            tree.push("osd", Role::Label).value(&self.osd_access).bounds(self.osd.input_rect().unwrap_or(Rect::ZERO)).live(nuxut::access::Live::Polite);
        }
        self.access_points = tree
            .nodes()
            .filter(|(_, n)| n.supports_action(AxAction::Click))
            .filter_map(|(id, n)| n.bounds().map(|b| (id, Point::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))))
            .collect();
        // Klavye gezinmesinin hedefleri: etkin, görünen, eylemli düğümler ağaç sırasıyla (bar
        // önce, sonra açık yapraklar).
        let bar = tree.node(nuxut::access::root_child("bar")).and_then(|n| n.bounds()).map(rect_of);
        self.nav_targets = tree
            .nodes()
            .filter(|(_, n)| (n.supports_action(AxAction::Click) || n.supports_action(AxAction::Increment)) && !n.is_disabled())
            .filter_map(|(id, n)| {
                let rect = rect_of(n.bounds()?);
                (rect.width > 0.0 && rect.height > 0.0).then(|| NavTarget {
                    id,
                    rect,
                    slider: n.supports_action(AxAction::Increment),
                    in_bar: bar.is_some_and(|b| b.contains(rect.center())),
                })
            })
            .collect();
        // Gezinme odağı AT'ye bildirilir (Orca odaktaki düğümü okur). Başlatıcı kendi odağını verir.
        if !self.state.is_open(PanelId::Launcher)
            && let Some(f) = self.nav_focus.filter(|f| self.nav_targets.iter().any(|t| t.id == *f))
        {
            tree.set_focus(f);
        }
    }

    /// Bileşenin kendi karşıladığı AT eylemi: açılır bildirim kartları ve kenar paneli kartları
    /// (kaydırıcı adımı/değeri, bildirim düğmeleri). Karşılanmadıysa `None`; tıklama o zaman
    /// işaretçi yolundan gider ([`Self::access_point`]).
    pub fn access_action(
        &mut self,
        request: &AccessRequest,
        config: &ShellConfig,
        theme: &Theme,
        now: Instant,
    ) -> Option<(bool, Option<Action>)> {
        if let Some(a) = self.popups.access_action(request, now) {
            return Some((true, Some(a)));
        }
        let cx = self.cx(config, theme, now);
        for (i, edge) in crate::layout::Edge::ALL.into_iter().enumerate() {
            if !(self.drawers[i].is_open() && self.state.is_open(PanelId::Edge(edge))) {
                continue;
            }
            let panel = nuxut::access::root_child(("panel", i));
            for (w, r) in self.panels[i].iter_mut().zip(&self.panel_rects[i]) {
                if let Some(out) = w.access_action(request, panel, *r, &cx) {
                    return Some(out);
                }
            }
        }
        None
    }

    /// AT tıklamasının işaretçi noktası (son kurulan ağaçtan).
    pub fn access_point(&self, node: NodeId) -> Option<Point> {
        self.access_points.get(&node).copied()
    }

    /// Son işaretçi konumu (AT tıklamasından sonra hover durumunu geri koymak için).
    pub fn pointer_position(&self) -> Option<Point> {
        self.pointer
    }
}

fn push_item(tree: &mut AccessTree, key: impl std::hash::Hash, item: &AccessItem) {
    let node = tree.push(key, Role::Button).label(&item.label).description(&item.value).bounds(item.rect).clickable();
    if let Some(on) = item.selected {
        node.toggled(on);
    }
}

/// accesskit sınırı → mantıksal dikdörtgen.
fn rect_of(b: nuxut::access::accesskit::Rect) -> Rect {
    Rect::new(b.x0 as f32, b.y0 as f32, (b.x1 - b.x0) as f32, (b.y1 - b.y0) as f32)
}

/// Klavye gezinmesinin hedefi (son kurulan ağaçtan).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavTarget {
    pub id: NodeId,
    pub rect: Rect,
    /// Kaydırıcı: sol/sağ ok değeri adımlar.
    pub slider: bool,
    /// Bar'da mı (açılan yaprağa geçişte bar dışı ilk hedef seçilir).
    pub in_bar: bool,
}

/// Gezinme tuşunun sonucu: odak taşındı ya da odaktaki düğüme AT eylemi (işaretçi/kart yoluyla
/// aynı: [`crate::app`] `accessibility_action`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NavOutcome {
    Moved,
    Action(AccessRequest),
    /// Bar gezinmesinden çıkıldı (Escape).
    Exit,
}

impl Screen {
    /// Bar gezinmesi (`msg bar focus`): klavye kipine geçer, odak bar'ın ilk öğesine. Hedefler
    /// için ağaç burada kurulur (ekran okuyucu çalışmasa da klavye gezinmesi çalışır).
    pub fn start_bar_nav(&mut self, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) -> bool {
        self.access(&mut AccessTree::new(), text, config, theme, now);
        let Some(first) = self.nav_targets.iter().find(|t| t.in_bar) else { return false };
        self.bar_nav = true;
        self.nav_focus = Some(first.id);
        self.damage_all = true;
        true
    }

    /// Gezinme bitti (Escape, odak kaybı): odak halkası kalkar. Bir şey değiştiyse `true`.
    pub fn end_nav(&mut self) -> bool {
        let changed = self.bar_nav || self.nav_focus.is_some();
        self.bar_nav = false;
        self.nav_focus = None;
        if changed {
            self.damage_all = true;
        }
        changed
    }

    /// AT `Focus`: gezinme odağı bu düğüme (yalnız eylemli, görünen bir hedefse). Etkinleştirmez.
    /// Değiştiyse `true`.
    pub fn nav_focus_to(&mut self, node: NodeId) -> bool {
        if self.nav_focus == Some(node) || !self.nav_targets.iter().any(|t| t.id == node) {
            return false;
        }
        self.nav_focus = Some(node);
        self.damage_all = true;
        true
    }

    pub fn bar_nav(&self) -> bool {
        self.bar_nav
    }

    /// Klavye gezinmesi (yüzey klavye odağındayken, bileşenler tuşu tüketmediyse): Tab / ok
    /// tuşları odağı ağaç sırasıyla taşır (sona gelince başa), Enter / boşluk odaktakini
    /// etkinleştirir, kaydırıcıda sol/sağ değeri adımlar, bar gezinmesinde Escape çıkar.
    /// Gezinilecek bir şey yoksa `None` (tuş normal yoluna devam eder).
    pub fn nav_key(
        &mut self,
        text: &mut TextSystem,
        config: &ShellConfig,
        theme: &Theme,
        event: &nuxut::KeyEvent,
        now: Instant,
    ) -> Option<NavOutcome> {
        use nuxut::{Key, KeyState};
        if event.state == KeyState::Released {
            return None;
        }
        if event.key == Key::Escape {
            return (self.bar_nav && self.end_nav()).then_some(NavOutcome::Exit);
        }
        if !matches!(event.key, Key::Tab | Key::Up | Key::Down | Key::Left | Key::Right | Key::Enter | Key::Space) {
            return None;
        }
        self.access(&mut AccessTree::new(), text, config, theme, now);
        let targets = &self.nav_targets;
        if targets.is_empty() {
            return None;
        }
        let current = self.nav_focus.and_then(|f| targets.iter().position(|t| t.id == f));
        let back = event.key == Key::Up || event.key == Key::Left || (event.key == Key::Tab && event.modifiers.shift);
        match (event.key, current) {
            (Key::Enter | Key::Space, Some(i)) => {
                return Some(NavOutcome::Action(AccessRequest { node: targets[i].id, action: nuxut::access::AccessAction::Click }));
            }
            (Key::Enter | Key::Space, None) => return None,
            (Key::Left | Key::Right, Some(i)) if targets[i].slider => {
                let action = if event.key == Key::Right { nuxut::access::AccessAction::Increment } else { nuxut::access::AccessAction::Decrement };
                return Some(NavOutcome::Action(AccessRequest { node: targets[i].id, action }));
            }
            _ => {}
        }
        let n = targets.len();
        let next = match current {
            // İlk tuş: açık yaprak varsa onun ilk öğesi, yoksa bar'ın ilki.
            None => targets.iter().position(|t| !t.in_bar).unwrap_or(0),
            Some(i) if back => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
        };
        self.nav_focus = Some(targets[next].id);
        self.damage_all = true;
        Some(NavOutcome::Moved)
    }

    /// Etkinleştirmeden sonra (yaprak açılmış olabilir): odak bar'daysa ve artık bar dışında hedef
    /// varsa açılan yaprağın ilk öğesine geçer. Taşındıysa `true`.
    pub fn nav_follow_opened(&mut self, text: &mut TextSystem, config: &ShellConfig, theme: &Theme, now: Instant) -> bool {
        let Some(f) = self.nav_focus else { return false };
        self.access(&mut AccessTree::new(), text, config, theme, now);
        let in_bar = self.nav_targets.iter().find(|t| t.id == f).is_none_or(|t| t.in_bar);
        match self.nav_targets.iter().find(|t| !t.in_bar) {
            Some(first) if in_bar => {
                self.nav_focus = Some(first.id);
                self.damage_all = true;
                true
            }
            _ => false,
        }
    }

    /// Odak halkası (tema `primary`, framework odak kalınlığı).
    pub(super) fn draw_nav_focus(&self, ui: &mut nuxut::ui::Ui) {
        let Some(t) = self.nav_focus.and_then(|f| self.nav_targets.iter().find(|t| t.id == f)) else { return };
        let ring = nuxut::ui::style::FOCUS_RING;
        let r = t.rect.expand(ring + 1.0);
        let radius = (r.height.min(r.width) * 0.5).min(ui.theme.shape.radius_medium + ring);
        let color = ui.fade(ui.theme.colors.primary);
        ui.painter.rect(
            r,
            &nuxut::RectStyle { fill: nuxut::Color::TRANSPARENT, radii: nuxut::Corners::uniform(radius), border: Some(nuxut::Border { width: ring, color }) },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut::access::{AccessAction, Role};
    use nuxut::{Key, KeyEvent, KeyState, Modifiers, OutputId, Size, SurfaceId};

    fn screen() -> (Screen, ShellConfig, Theme, TextSystem) {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let config = ShellConfig::validated_default();
        let mut s = Screen::new(&theme, &config, OutputId::for_test(1), "T-1".into(), SurfaceId::for_test(1));
        s.set_size(Size::new(1920.0, 1080.0));
        (s, config, theme, TextSystem::new())
    }

    fn key(k: Key, shift: bool) -> KeyEvent {
        KeyEvent::new(k, KeyState::Pressed, None, Modifiers { shift, ..Modifiers::default() })
    }

    /// Bar `Toolbar`, saat düğmesi adı ve değeriyle; adsız etkileşimli düğüm yok.
    #[test]
    fn bar_tree_has_named_items() {
        let (mut s, config, theme, mut text) = screen();
        let mut tree = AccessTree::new();
        s.access(&mut tree, &mut text, &config, &theme, Instant::now());
        let bar = tree.node(nuxut::access::root_child("bar")).expect("bar");
        assert_eq!(bar.role(), Role::Toolbar);
        let clock = tree.nodes().find(|(_, n)| n.label() == Some("Saat")).map(|(_, n)| n).expect("saat düğmesi");
        assert!(clock.description().is_some_and(|d| !d.is_empty()), "saat değeri okunur");
        assert!(tree.unlabeled().is_empty(), "adsız: {:?}", tree.unlabeled().len());
        assert!(!s.nav_targets.is_empty() && s.nav_targets.iter().all(|t| t.in_bar));
    }

    /// `msg bar focus`: odak bar'ın ilk öğesine, klavye özel kipe; Tab ileri, Shift+Tab geri
    /// (başa sarar), Enter odaktakine tıklama, Escape çıkar. Odak ağaçta bildirilir.
    #[test]
    fn bar_navigation_moves_activates_and_exits() {
        let (mut s, config, theme, mut text) = screen();
        let now = Instant::now();
        assert!(s.start_bar_nav(&mut text, &config, &theme, now));
        assert_eq!(s.keyboard_mode(), nuxut::KeyboardMode::Exclusive);
        let first = s.nav_focus.expect("odak");
        let n = s.nav_targets.len();
        assert!(n >= 2, "bar'da en az iki öğe");
        assert_eq!(s.nav_key(&mut text, &config, &theme, &key(Key::Tab, false), now), Some(NavOutcome::Moved));
        let second = s.nav_focus.unwrap();
        assert_ne!(first, second);
        assert_eq!(s.nav_key(&mut text, &config, &theme, &key(Key::Tab, true), now), Some(NavOutcome::Moved));
        assert_eq!(s.nav_focus, Some(first));
        s.nav_key(&mut text, &config, &theme, &key(Key::Left, false), now);
        assert_eq!(s.nav_focus, Some(s.nav_targets[n - 1].id), "baştan geri: sona sarar");
        let mut tree = AccessTree::new();
        s.access(&mut tree, &mut text, &config, &theme, now);
        assert_eq!(tree.focus(), s.nav_targets[n - 1].id, "odak AT'ye bildirilir");
        assert_eq!(
            s.nav_key(&mut text, &config, &theme, &key(Key::Enter, false), now),
            Some(NavOutcome::Action(AccessRequest { node: s.nav_targets[n - 1].id, action: AccessAction::Click }))
        );
        assert_eq!(s.nav_key(&mut text, &config, &theme, &key(Key::Escape, false), now), Some(NavOutcome::Exit));
        assert!(!s.bar_nav() && s.nav_focus.is_none());
        assert_eq!(s.keyboard_mode(), nuxut::KeyboardMode::None);
        assert_eq!(s.nav_key(&mut text, &config, &theme, &key(Key::Escape, false), now), None, "gezinme yokken Escape normal yoluna");
        assert_eq!(s.nav_key(&mut text, &config, &theme, &key(Key::Char('a'), false), now), None);
    }
}
