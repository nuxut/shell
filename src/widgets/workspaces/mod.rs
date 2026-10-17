//! Workspace göstergesi (tasarım: `docs/design/workspaces.md`): sayfa başına her zaman `count`
//! yuva; aktif workspace'i primary hap gösterir ve hap yuva sırası cinsinden (`Animated<f32>`)
//! yaylanır — sayfa içinde kayar, sayfa değişince yeni yerine atlar. Başka monitörde görünen
//! workspace tonal disk, dolu workspace uygulama glifi/numara/büyük nokta, boş workspace küçük
//! nokta. Önceki/sonraki sayfada kayda değer workspace varsa uçlarda ok; okların yeri her zaman
//! ayrılıdır (noktalar kaymaz). Tekerlek/dokunmatik yüzey kaydırması workspace değiştirir.
//! Yatay ve dikey bar.

pub mod page;

use std::time::Instant;

use nuxut::i18n::t;
use nuxut::ui::{Icon, IconName, IconView, Interaction, Label, TextRole, app_glyph, find_nerd_font, style, text_style};
use nuxut::{
    Animated, Color, Corners, CursorShape, Painter, Point, PointerEvent, PointerKind, Rect, RectStyle, TextStyle,
    TextSystem, Theme,
};

use self::page::{ScrollAccumulator, SlotState, WorkspacePage, WorkspaceScroller, notable_ids, workspace_page};
use super::registry::{WorkspaceScroll, WorkspaceStyle};
use super::{Action, BarTooltip, BarWidget, WidgetCx, WidgetKind, center_in, draw_hover};
use crate::model::ShellModel;

/// Başka monitörde görünen workspace'in tonal diskinin opaklığı (`primary` üzerinden).
const ELSEWHERE_ALPHA: f32 = 0.18;
/// Boş yuvanın noktası ve soluk içerik opaklığı (`on_surface_variant` üzerinden).
const EMPTY_ALPHA: f32 = 0.55;

/// Yuvada çizilen içerik.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content {
    /// Nokta; `big`: dolu workspace (`dots` stili).
    Dot { big: bool },
    /// Numara.
    Number,
    /// Uygulama glifi (Nerd Font).
    Glyph(char),
}

/// Stil, yuva durumu ve glif fontunun varlığından içerik (saf).
fn slot_content(style: WorkspaceStyle, slot: &SlotState, glyphs: bool) -> Content {
    match style {
        WorkspaceStyle::Numbers => Content::Number,
        WorkspaceStyle::Dots => Content::Dot { big: slot.occupied },
        WorkspaceStyle::Icons if !slot.occupied => Content::Dot { big: false },
        WorkspaceStyle::Icons => {
            match glyphs.then(|| app_glyph(&slot.app_class)).flatten() {
                Some(g) => Content::Glyph(g),
                None => Content::Number,
            }
        }
    }
}

#[derive(Debug)]
struct Slot {
    state: SlotState,
    content: Content,
    label: Label,
    hover: Interaction,
}

/// Önceki/sonraki sayfa göstergesi.
#[derive(Debug)]
struct Arrow {
    icon: IconView,
    hover: Interaction,
    target: Option<i64>,
    /// Görünürlük (0..1), `motion.effects`.
    alpha: Animated<f32>,
}

impl Arrow {
    fn new(cx: &WidgetCx, name: IconName) -> Self {
        Self {
            icon: IconView::with_role(cx.theme, Icon::named(name), TextRole::IconSmall),
            hover: Interaction::new(cx.theme),
            target: None,
            alpha: Animated::new(0.0, cx.theme.motion.effects),
        }
    }

    fn set_target(&mut self, target: Option<i64>, placed: bool, now: Instant) -> bool {
        let changed = self.target != target;
        self.target = target;
        let a = if target.is_some() { 1.0 } else { 0.0 };
        if !placed {
            self.alpha.jump_to(a);
        } else if self.alpha.target() != a {
            self.alpha.set_target(a, now);
        }
        changed
    }
}

/// Ayrılmış son yuva: bu monitörde aktif adlı workspace (primary hap + uygulama glifi ya da baş
/// harf) ve açık special workspace'in sakin işareti. Yer her zaman ayrılıdır (noktaları itmez);
/// ad ipucu balonunda.
#[derive(Debug)]
struct Extra {
    label: Label,
    /// Adlı workspace hapının görünürlüğü (0..1, `motion.effects`).
    named: Animated<f32>,
    /// Special işaretinin görünürlüğü.
    special: Animated<f32>,
    tooltip: String,
}

impl Extra {
    fn new(cx: &WidgetCx) -> Self {
        Self {
            label: Label::new("", text_style(cx.theme, TextRole::Body)),
            named: Animated::new(0.0, cx.theme.motion.effects),
            special: Animated::new(0.0, cx.theme.motion.effects),
            tooltip: String::new(),
        }
    }

    fn shown(&self) -> bool {
        !self.tooltip.is_empty()
    }
}

/// `Animated` görünürlük hedefini kurar (ilk yerleşimde atlar). Değiştiyse `true`.
fn set_visibility(a: &mut Animated<f32>, on: bool, placed: bool, now: Instant) -> bool {
    let t = if on { 1.0 } else { 0.0 };
    if !placed {
        a.jump_to(t);
        return true;
    }
    if a.target() == t {
        return false;
    }
    a.set_target(t, now);
    true
}

/// Ayrılmış yuvanın ipucu metni (saf).
fn extra_tooltip(named: Option<&str>, special: Option<&str>) -> String {
    let special = special.map(|s| t!("shell-bar-workspaces-special-tooltip", name = s));
    [named.map(str::to_owned), special].into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

/// Adlı workspace'in yuva metni: uygulama glifi, yoksa adın baş harfi (büyük).
fn named_label(name: &str, app_class: &str, glyphs: bool) -> (String, bool) {
    match glyphs.then(|| app_glyph(app_class)).flatten() {
        Some(g) => (g.to_string(), true),
        None => (name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default(), false),
    }
}

/// Widget dikdörtgeni içindeki parçalar (bar ekseni boyunca: ok, yuvalar, ok, ayrılmış yuva).
#[derive(Debug, Clone, PartialEq)]
struct Layout {
    prev: Rect,
    slots: Vec<Rect>,
    next: Rect,
    extra: Rect,
}

#[derive(Debug)]
pub struct Workspaces {
    page: WorkspacePage,
    slots: Vec<Slot>,
    style: WorkspaceStyle,
    pill: Animated<f32>,
    pill_placed: bool,
    /// Bu monitörün aktif workspace'i sayfada mı (değilse hap çizilmez).
    pill_visible: bool,
    prev: Arrow,
    next: Arrow,
    arrows_vertical: bool,
    arrows_placed: bool,
    extra: Extra,
    /// Kaydırma hedefi hesabı için: kayda değer id'ler ve bu monitörün aktif workspace'i.
    notable: Vec<i64>,
    active_id: Option<i64>,
    /// Kaydırma hedefi ve bekleyen (gönderilmiş, dönmemiş) hedef.
    scroller: WorkspaceScroller,
    scroll: ScrollAccumulator,
    /// Glif fontu: `None` = henüz çözülmedi, `Some(None)` = kurulu Nerd Font yok.
    glyph_family: Option<Option<String>>,
    /// Son çizimdeki dikdörtgen (kaydırma isabeti; olay yolunda bar geometrisi yok).
    last_rect: Option<Rect>,
}

impl Workspaces {
    pub fn new(cx: &WidgetCx) -> Self {
        let (p, n) = arrow_icons(cx.vertical);
        Self {
            page: WorkspacePage::default(),
            slots: Vec::new(),
            style: cx.settings.workspaces.style,
            pill: Animated::new(0.0, cx.theme.motion.spatial),
            pill_placed: false,
            pill_visible: false,
            prev: Arrow::new(cx, p),
            next: Arrow::new(cx, n),
            arrows_vertical: cx.vertical,
            arrows_placed: false,
            extra: Extra::new(cx),
            notable: Vec::new(),
            active_id: None,
            scroller: WorkspaceScroller::default(),
            scroll: ScrollAccumulator::default(),
            glyph_family: None,
            last_rect: None,
        }
    }

    fn gap(cx: &WidgetCx) -> f32 {
        cx.theme.space(1)
    }

    /// Ok yuvasının bar ekseni boyunca uzunluğu.
    fn arrow_len(cx: &WidgetCx) -> f32 {
        (cx.item * 0.6).round()
    }

    fn layout(&self, rect: Rect, cx: &WidgetCx) -> Layout {
        let (item, gap, arrow) = (cx.item, Self::gap(cx), Self::arrow_len(cx));
        // `off` eksen boyunca başlangıç, `len` eksen boyunca uzunluk; çapraz eksende item boyu ortalanır.
        let place = |off: f32, len: f32| {
            if cx.vertical {
                Rect::new(rect.center().x - item * 0.5, rect.y + off, item, len)
            } else {
                Rect::new(rect.x + off, rect.center().y - item * 0.5, len, item)
            }
        };
        let start = arrow + gap;
        let slots: Vec<Rect> = (0..self.slots.len()).map(|i| place(start + i as f32 * (item + gap), item)).collect();
        let end = start + self.slots.len() as f32 * (item + gap);
        Layout { prev: place(0.0, arrow), slots, next: place(end, arrow), extra: place(end + arrow + gap, item) }
    }

    /// Glif fontunu bir kez çözer (`Symbols Nerd Font`, yoksa Nerd Font olan `mono` ailesi).
    fn resolve_glyph_family(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> bool {
        if self.glyph_family.is_some() {
            return false;
        }
        // Symbols Nerd Font → mono ailesi (Nerd Font ise) → kurulu Nerd Font'lardan en uygunu.
        let family = find_nerd_font(text, &cx.theme.typography.mono.family);
        if family.is_none() {
            tracing::info!("marka glifi fontu bulunamadı: workspace ikonları yerine numara gösterilecek");
        }
        self.glyph_family = Some(family);
        self.refresh_contents(cx)
    }

    fn glyph_style(&self, cx: &WidgetCx) -> Option<TextStyle> {
        let family = self.glyph_family.as_ref()?.as_ref()?;
        Some(TextStyle::new(family.clone(), (cx.item * 0.62).round(), 400))
    }

    /// Yuva içeriklerini (stil, durum, font) yeniler. Değiştiyse `true`.
    fn refresh_contents(&mut self, cx: &WidgetCx) -> bool {
        let glyph_style = self.glyph_style(cx);
        let body = text_style(cx.theme, TextRole::Body);
        let mut changed = false;
        for slot in &mut self.slots {
            let content = slot_content(self.style, &slot.state, glyph_style.is_some());
            let (text, style) = match content {
                Content::Glyph(g) => (g.to_string(), glyph_style.clone().unwrap_or_else(|| body.clone())),
                Content::Number => (slot.state.id.to_string(), body.clone()),
                Content::Dot { .. } => (String::new(), body.clone()),
            };
            changed |= slot.content != content;
            slot.content = content;
            changed |= slot.label.set_style(style);
            changed |= slot.label.set_text(&text);
        }
        if let Some(n) = &self.page.named {
            let (text, glyph) = named_label(&n.name, &n.app_class, glyph_style.is_some());
            let style = if glyph { glyph_style.unwrap_or(body) } else { body };
            changed |= self.extra.label.set_style(style);
            changed |= self.extra.label.set_text(&text);
        }
        changed
    }
}

fn arrow_icons(vertical: bool) -> (IconName, IconName) {
    if vertical { (IconName::ChevronUp, IconName::ChevronDown) } else { (IconName::ChevronLeft, IconName::ChevronRight) }
}

impl BarWidget for Workspaces {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Workspaces
    }

    fn sync(&mut self, model: &ShellModel, monitor: &str, cx: &WidgetCx) -> bool {
        let settings = &cx.settings.workspaces;
        let page = workspace_page(&model.hypr, monitor, settings.count, self.page.page);
        self.notable = notable_ids(&model.hypr);
        self.active_id = model.hypr.monitor(monitor).map(|m| m.active_workspace.id);
        self.scroller.observe(self.active_id, cx.now);

        let mut changed = self.style != settings.style;
        self.style = settings.style;
        let page_changed = page.page != self.page.page || page.slots.len() != self.slots.len();
        if page_changed {
            self.slots = page
                .slots
                .iter()
                .map(|s| Slot {
                    state: s.clone(),
                    content: Content::Dot { big: false },
                    label: Label::new("", text_style(cx.theme, TextRole::Body)),
                    hover: Interaction::new(cx.theme),
                })
                .collect();
            changed = true;
        }
        let prev_active = self.page.active_index().map(|i| self.page.slots[i].id);
        for (slot, state) in self.slots.iter_mut().zip(&page.slots) {
            changed |= slot.state != *state;
            slot.state = state.clone();
        }
        changed |= self.prev.set_target(page.prev, self.arrows_placed, cx.now);
        changed |= self.next.set_target(page.next, self.arrows_placed, cx.now);
        let placed = self.arrows_placed;
        changed |= set_visibility(&mut self.extra.named, page.named.is_some(), placed, cx.now);
        changed |= set_visibility(&mut self.extra.special, page.special.is_some(), placed, cx.now);
        let tooltip = extra_tooltip(page.named.as_ref().map(|n| n.name.as_str()), page.special.as_deref());
        changed |= self.extra.tooltip != tooltip;
        self.extra.tooltip = tooltip;
        self.arrows_placed = true;

        // Hap hedefi burada (çizimde değil) kurulur: `Screen::draw` bar hareketini widget'lar
        // çizilmeden önce sorar; hedef çizimde değişseydi geçiş karesinden sonra kare istenmezdi.
        let active = page.active_index();
        changed |= self.pill_visible != active.is_some();
        self.pill_visible = active.is_some();
        if let Some(i) = active {
            let target = i as f32;
            if !self.pill_placed || page_changed {
                // İlk yerleşim ya da sayfa değişti: kaymaz, atlar.
                self.pill.jump_to(target);
                self.pill_placed = true;
            } else if prev_active == Some(page.slots[i].id) {
                // Aktif workspace aynı: hedef aynıysa süren hareket bozulmaz.
                if self.pill.target() != target {
                    self.pill.jump_to(target);
                }
            } else {
                self.pill.set_target(target, cx.now);
            }
        }
        self.page = page;
        changed |= self.refresh_contents(cx);
        changed
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.pill.set_spring(cx.theme.motion.spatial, cx.now);
        for a in [&mut self.prev, &mut self.next] {
            a.hover.set_theme(cx.theme, cx.now);
            a.icon.set_theme(cx.theme, cx.now);
            a.alpha.set_spring(cx.theme.motion.effects, cx.now);
        }
        for s in &mut self.slots {
            s.hover.set_theme(cx.theme, cx.now);
        }
        self.extra.named.set_spring(cx.theme.motion.effects, cx.now);
        self.extra.special.set_spring(cx.theme.motion.effects, cx.now);
        self.style = cx.settings.workspaces.style;
        // `mono` ailesi değişmiş olabilir: font yeniden çözülür (ölçümde).
        self.glyph_family = None;
        self.refresh_contents(cx);
    }

    fn measure(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        self.resolve_glyph_family(text, cx);
        if self.arrows_vertical != cx.vertical {
            let (p, n) = arrow_icons(cx.vertical);
            self.prev.icon.set_icon(Icon::named(p));
            self.next.icon.set_icon(Icon::named(n));
            self.arrows_vertical = cx.vertical;
        }
        let n = self.slots.len() as f32;
        if n == 0.0 {
            return 0.0;
        }
        let gap = Self::gap(cx);
        2.0 * (Self::arrow_len(cx) + gap) + (n + 1.0) * cx.item + n * gap
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, _highlighted: bool) {
        self.resolve_glyph_family(painter.text_system(), cx);
        self.last_rect = Some(rect);
        let colors = &cx.theme.colors;
        let now = cx.now;
        let layout = self.layout(rect, cx);
        let radii = Corners::uniform(cx.item * 0.5);

        let mut pill = Rect::ZERO;
        if self.pill_placed && self.pill_visible {
            pill = slot_at(&layout.slots, self.pill.value(now));
            painter.rect(pill, &RectStyle::filled(colors.primary, radii));
        }

        for (slot, &r) in self.slots.iter_mut().zip(&layout.slots) {
            let text = !matches!(slot.content, Content::Dot { .. });
            let (disk, base) = slot_colors(cx.theme, &slot.state, text);
            if let Some(disk) = disk {
                painter.rect(r, &RectStyle::filled(disk, radii));
            }
            // Hap altındaki içerik on_primary; hap kayarken örtüşme oranında karışır.
            let overlap = overlap_ratio(r, pill);
            let color = mix(base, colors.on_primary, overlap);
            // Katmanın zemini: yuvanın çoğu aktif hapın altındaysa hap (`primary`), değilse disk.
            let under = if overlap >= 0.5 { colors.primary } else { disk.unwrap_or(Color::TRANSPARENT) };
            draw_hover(painter, &slot.hover, r, radii, cx, under, &[color]);
            match slot.content {
                Content::Dot { big } => {
                    let d = (cx.item * if big { 0.30 } else { 0.18 }).round().max(2.0);
                    let dot = Rect::new(r.center().x - d * 0.5, r.center().y - d * 0.5, d, d);
                    painter.rect(dot, &RectStyle::filled(color, Corners::uniform(d * 0.5)));
                }
                Content::Number | Content::Glyph(_) => {
                    let size = slot.label.measure_in(painter);
                    slot.label.draw(painter, center_in(r, size.width, size.height), color);
                }
            }
        }

        for (arrow, r) in [(&mut self.prev, layout.prev), (&mut self.next, layout.next)] {
            let a = arrow.alpha.value(now).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let ar = Corners::uniform(r.width.min(r.height) * 0.5);
            draw_hover(painter, &arrow.hover, r, ar, cx, Color::TRANSPARENT, &[colors.on_surface_variant]);
            arrow.icon.draw_centered(painter, r.center(), colors.on_surface_variant.multiply_alpha(a));
        }

        // Ayrılmış yuva: adlı workspace hapı ve/veya special işareti.
        let r = layout.extra;
        let named = self.extra.named.value(now).clamp(0.0, 1.0);
        let special = self.extra.special.value(now).clamp(0.0, 1.0);
        if named > 0.0 {
            painter.rect(r, &RectStyle::filled(colors.primary.multiply_alpha(named), radii));
            let size = self.extra.label.measure_in(painter);
            self.extra.label.draw(painter, center_in(r, size.width, size.height), colors.on_primary.multiply_alpha(named));
        }
        if special > 0.0 {
            // Tek başına: tonal disk + küçük nokta; adlı hapın üstünde: köşede küçük rozet.
            let solo = 1.0 - named;
            if solo > 0.0 {
                painter.rect(r, &RectStyle::filled(tonal_disk(cx.theme, ELSEWHERE_ALPHA * special * solo), radii));
            }
            let d = (cx.item * 0.22).round().max(3.0);
            let (center, corner) = (r.center(), Point::new(r.right() - d * 0.5, r.y + d * 0.5));
            let c = Point::new(center.x + (corner.x - center.x) * named, center.y + (corner.y - center.y) * named);
            let ring = d + 3.0;
            if named > 0.0 {
                let ring_rect = Rect::new(c.x - ring * 0.5, c.y - ring * 0.5, ring, ring);
                painter.rect(ring_rect, &RectStyle::filled(colors.surface.multiply_alpha(special * named), Corners::uniform(ring * 0.5)));
            }
            let dot = Rect::new(c.x - d * 0.5, c.y - d * 0.5, d, d);
            painter.rect(dot, &RectStyle::filled(colors.primary.multiply_alpha(special), Corners::uniform(d * 0.5)));
        }
    }

    fn tooltip(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        // Ipucu yalnızca imlecin üzerindeki yuvanın (ek yuva) altında çıkar: çapa o yuvanın rect'i.
        let layout = self.layout(rect, cx);
        (self.extra.shown() && layout.extra.contains(p))
            .then_some(BarTooltip { text: self.extra.tooltip.as_str(), anchor: Some(layout.extra) })
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        let layout = self.layout(rect, cx);
        let mut changed = false;
        for (slot, r) in self.slots.iter_mut().zip(layout.slots) {
            changed |= slot.hover.pointer(r, pointer, down, cx.now);
        }
        for (arrow, r) in [(&mut self.prev, layout.prev), (&mut self.next, layout.next)] {
            let p = pointer.filter(|_| arrow.target.is_some());
            changed |= arrow.hover.pointer(r, p, down, cx.now);
        }
        changed
    }

    // Ekran okuyucu (ADR-0011): her yuva bir düğme ("Çalışma alanı 3", etkin olan seçili),
    // görünürse önceki/sonraki sayfa okları. Anahtar çalışma alanı kimliği (indis değil).
    fn access_items(&self, rect: Rect, cx: &WidgetCx) -> Vec<super::AccessItem> {
        let layout = self.layout(rect, cx);
        let mut items: Vec<super::AccessItem> = self
            .slots
            .iter()
            .zip(&layout.slots)
            .map(|(s, r)| {
                let label = t!("shell-a11y-workspace", id = s.state.id);
                super::AccessItem::new(format!("ws-{}", s.state.id), label, *r).with_selected(s.state.active)
            })
            .collect();
        for (key, arrow, r, label) in [
            ("prev", &self.prev, layout.prev, t!("shell-a11y-workspaces-prev")),
            ("next", &self.next, layout.next, t!("shell-a11y-workspaces-next")),
        ] {
            if arrow.target.is_some() && r.width > 0.0 && r.height > 0.0 {
                items.push(super::AccessItem::new(key, label, r));
            }
        }
        items
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        let layout = self.layout(rect, cx);
        if let Some(i) = layout.slots.iter().position(|r| r.contains(at)) {
            return Some(Action::SwitchWorkspace(self.slots[i].state.id));
        }
        [(layout.prev, self.prev.target), (layout.next, self.next.target)]
            .into_iter()
            .find(|(r, _)| r.contains(at))
            .and_then(|(_, t)| t)
            .map(Action::SwitchWorkspace)
    }

    /// Kaydırma: widget dikdörtgenindeki tekerlek/dokunmatik yüzey olayı (diğerleri bar'a düşer).
    fn overlay_pointer(
        &mut self,
        event: &PointerEvent,
        _text: &mut TextSystem,
        cx: &WidgetCx,
    ) -> Option<(bool, Option<Action>)> {
        let PointerKind::Scroll { delta, steps } = event.kind else { return None };
        if !self.last_rect.is_some_and(|r| r.contains(event.position)) {
            return None;
        }
        let mode = cx.settings.workspaces.scroll;
        if mode == WorkspaceScroll::Off {
            return None;
        }
        let n = self.scroll.feed(delta, steps, cx.now);
        let target = self.scroller.step(mode, &self.notable, self.active_id, n, cx.now);
        Some((false, target.map(Action::SwitchWorkspace)))
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        !self.pill.is_settled(now)
            | self.slots.iter_mut().any(|s| s.hover.is_animating(now))
            | [&mut self.prev, &mut self.next].into_iter().any(|a| !a.alpha.is_settled(now) | a.hover.is_animating(now))
            | !self.extra.named.is_settled(now)
            | !self.extra.special.is_settled(now)
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        let layout = self.layout(rect, cx);
        let arrows = [(layout.prev, self.prev.target), (layout.next, self.next.target)];
        let hit = layout.slots.iter().any(|r| r.contains(p)) || arrows.iter().any(|(r, t)| t.is_some() && r.contains(p));
        hit.then_some(CursorShape::Pointer)
    }
}

/// Kesirli sıradaki yuva: komşu iki yuva arasında doğrusal (yay taşmasında uçtaki yuvadan
/// dışarı doğru uzatılır).
fn slot_at(slots: &[Rect], pos: f32) -> Rect {
    match slots {
        [] => Rect::ZERO,
        [only] => *only,
        _ => {
            let last = slots.len() - 1;
            let i = (pos.floor().max(0.0) as usize).min(last - 1);
            slots[i].lerp(&slots[i + 1], pos - i as f32)
        }
    }
}

/// `a`'nın alanının ne kadarı `b` ile örtüşüyor (0..1).
fn overlap_ratio(a: Rect, b: Rect) -> f32 {
    let area = a.width * a.height;
    if area <= 0.0 {
        return 0.0;
    }
    let i = a.intersect(&b);
    (i.width * i.height / area).clamp(0.0, 1.0)
}

/// `primary` tonal disk (başka monitörde görünen workspace, tek başına special): üstündeki
/// `primary` içerik yüksek karşıtlıkta ≥ 4.5 kalsın diye framework state layer yardımcısı
/// (`style::state_layer`; standart kipte `primary × alpha`, t124).
fn tonal_disk(theme: &Theme, alpha: f32) -> Color {
    style::state_layer(theme, theme.colors.primary, &[], alpha)
}

/// Yuvanın (tonal disk, içerik) renkleri. `text`: içerik numara/glif (değilse nokta). Boş yuva
/// soluk: metin `faded_text` (yüksek karşıtlıkta ≥ 4.5), nokta `faded_mark` (≥ 3).
fn slot_colors(theme: &Theme, state: &SlotState, text: bool) -> (Option<Color>, Color) {
    let c = &theme.colors;
    if state.visible_elsewhere && !state.active {
        (Some(tonal_disk(theme, ELSEWHERE_ALPHA)), c.primary)
    } else if state.occupied {
        (None, c.on_surface)
    } else if text {
        (None, style::faded_text(theme, c.on_surface_variant, EMPTY_ALPHA, &[]))
    } else {
        (None, style::faded_mark(theme, c.on_surface_variant, EMPTY_ALPHA, &[]))
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    a.lerp(&b, t)
}

#[cfg(test)]
mod tests;
