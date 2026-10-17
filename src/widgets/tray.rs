//! Sistem tepsisi widget'ı: görünür (Passive olmayan) öğelerin ikonları ve dbusmenu menüsü.
//!
//! - Durum yalnızca modelden okunur (`ShellModel::tray`); eylemler `Action::Tray` olarak döner,
//!   widget servis tutmaz.
//! - İkonlar servisin çözdüğü `TrayItem::image`'dır (düz RGBA); widget her görüntüyü bir kez
//!   dokuya yükler ve görüntü değişene kadar yeniden kullanır (her karede yükleme yok).
//! - Fare: sol tık `Activate` (öğe yalnızca menüyse menü), sağ tık menü, orta tık
//!   `SecondaryActivate`, tekerlek `Scroll`. Sol tık bar isabet testinden (`click`), diğerleri
//!   `overlay_pointer`'dan gelir (Screen her işaretçi olayını önce overlay'e sorar).
//! - Menü: `nuxut::ui::Menu`, bar'dan blob olarak büyür. Her açılış (sağ tık, yalnızca-menü
//!   öğeye sol tık, servisin `MenuRequested`'i) yeni bir istek kimliği alır (`OpenMenu
//!   { request }`); menü yalnızca o kimlikle gelen ağaçla açılır, açıkken aynı kimlikle gelen
//!   güncellemeler `set_model` ile uygulanır. Eski açılışın geç yanıtı yok sayılır. Açık menünün
//!   ikonuna tekrar tıklamak yalnızca kapatır (bırakma yutulur); bekleyen açılış yeni bir
//!   basmayla iptal olur. Menü çerçevenin iç alanına (`WidgetCx::bounds`) sığar. Model çizimde
//!   (`draw`, `draw_overlay`) kurulur: menü öğesi ikonları (servisin çözdüğü
//!   `MenuNode::image`) o an dokuya yüklenir ve `Icon::image`/`Icon::symbolic` olur.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use nuxut::render::{ImageData, ImageFit, ImageStyle, Texture, TextureOptions};
use nuxut::ui::{Icon, Interaction, Menu, MenuAnchor, MenuEvent, MenuModel, MenuSide, Ui};
use nuxut::{
    BlobShape, Button, Corners, CursorShape, KeyEvent, Painter, Point, PointerEvent, PointerKind, Rect, RectStyle,
    TextSystem, Theme,
};

use super::{Action, BarWidget, TrayCommand, WidgetCx, WidgetKind, chip_fill, draw_hover};
use crate::model::ShellModel;
use crate::services::tray::{
    ItemId, MenuRequest, MenuTree, Pixmap, Status, TrayItem, dbus_menu_id, is_symbolic, menu_model_with, next_menu_request,
};

/// Tekerlek çentiği başına SNI `Scroll` miktarı (KDE/Qt açı birimi).
const SCROLL_UNITS_PER_STEP: f32 = 120.0;

/// Bir tepsi yuvası (görünür öğe).
#[derive(Debug, Clone, PartialEq)]
struct Slot {
    id: ItemId,
    image: Option<Pixmap>,
    /// Dikkat istiyor ve ayrı dikkat ikonu yok: vurgu rozeti çizilir.
    attention_badge: bool,
    item_is_menu: bool,
    has_menu: bool,
}

impl Slot {
    fn from_item(item: &TrayItem) -> Self {
        Self {
            id: item.id.clone(),
            image: item.image.clone(),
            attention_badge: item.status == Status::NeedsAttention && item.attention_icon.is_empty(),
            item_is_menu: item.item_is_menu,
            has_menu: item.menu.is_some(),
        }
    }
}

/// Yuvaların bar ekseni boyunca toplam uzunluğu.
fn total_length(count: usize, side: f32, gap: f32) -> f32 {
    if count == 0 { 0.0 } else { count as f32 * side + (count - 1) as f32 * gap }
}

/// Widget dikdörtgeninde yuvaların kare dikdörtgenleri (bar ekseni boyunca, çapraz eksende
/// ortalı).
fn slot_rects(rect: Rect, count: usize, side: f32, gap: f32, vertical: bool) -> Vec<Rect> {
    let start = if vertical {
        rect.y + (rect.height - total_length(count, side, gap)).max(0.0) * 0.5
    } else {
        rect.x + (rect.width - total_length(count, side, gap)).max(0.0) * 0.5
    };
    (0..count)
        .map(|i| {
            let along = start + i as f32 * (side + gap);
            if vertical {
                Rect::new(rect.center().x - side * 0.5, along, side, side)
            } else {
                Rect::new(along, rect.center().y - side * 0.5, side, side)
            }
        })
        .collect()
}

fn slot_at(rects: &[Rect], p: Point) -> Option<usize> {
    rects.iter().position(|r| r.contains(p))
}

/// Yuva tıklamasının sonucu: menü açılışı (istek kimliğini widget verir) ya da doğrudan komut.
#[derive(Debug, Clone, PartialEq)]
enum SlotAction {
    OpenMenu,
    Command(TrayCommand),
}

/// Birincil tıklamanın eylemi: yalnızca menü sunan öğe menüyü açar, diğerleri `Activate`.
fn primary_action(slot: &Slot, at: Point) -> SlotAction {
    if slot.item_is_menu && slot.has_menu {
        SlotAction::OpenMenu
    } else {
        SlotAction::Command(TrayCommand::Activate { item: slot.id.clone(), x: at.x.round() as i32, y: at.y.round() as i32 })
    }
}

/// Sağ/orta tık eylemi: sağ tık dbusmenu varsa menüyü (shell çizer), yoksa uygulamanın kendi
/// bağlam menüsünü (`ContextMenu`) açar; orta tık `SecondaryActivate`.
fn button_action(slot: &Slot, button: Button, at: Point) -> Option<SlotAction> {
    let (item, x, y) = (slot.id.clone(), at.x.round() as i32, at.y.round() as i32);
    match button {
        Button::Right if slot.has_menu => Some(SlotAction::OpenMenu),
        Button::Right => Some(SlotAction::Command(TrayCommand::ContextMenu { item, x, y })),
        Button::Middle => Some(SlotAction::Command(TrayCommand::SecondaryActivate { item, x, y })),
        _ => None,
    }
}

/// Tekerlek/dokunmatik yüzey → SNI `Scroll`. Pozitif SNI miktarı yukarı/sağa kaydırmadır
/// (Qt `angleDelta` ile aynı); `PointerKind::Scroll`'da pozitif y aşağıdır.
fn scroll_command(id: &ItemId, delta: Point, steps: Point) -> Option<TrayCommand> {
    let (amount, horizontal) = {
        let (dx, dy) = if steps.x != 0.0 || steps.y != 0.0 {
            (steps.x * SCROLL_UNITS_PER_STEP, steps.y * SCROLL_UNITS_PER_STEP)
        } else {
            (delta.x, delta.y)
        };
        if dy.abs() >= dx.abs() { (-dy, false) } else { (-dx, true) }
    };
    let amount = amount.round();
    (amount != 0.0 && amount.is_finite()).then(|| TrayCommand::Scroll {
        item: id.clone(),
        delta: amount.clamp(i32::MIN as f32, i32::MAX as f32) as i32,
        horizontal,
    })
}

/// Menü olayı → servis komutu. Tıklama ve kapanışta servis izlemeyi kendisi bırakır.
fn menu_command(item: &ItemId, event: &MenuEvent) -> Option<TrayCommand> {
    match event {
        MenuEvent::Activated(id) => Some(TrayCommand::MenuClicked { item: item.clone(), id: dbus_menu_id(*id)? }),
        MenuEvent::SubmenuOpened(id) => {
            Some(TrayCommand::MenuAboutToShow { item: item.clone(), ids: vec![dbus_menu_id(*id)?] })
        }
        MenuEvent::Dismissed => Some(TrayCommand::CloseMenu(item.clone())),
    }
}

/// Menünün bar kenarına göre açılacağı taraf.
fn menu_anchor(slot: Rect, vertical: bool, bounds: Rect) -> MenuAnchor {
    let mut anchor = MenuAnchor::below(slot);
    anchor.side = if vertical {
        if slot.center().x < bounds.center().x { MenuSide::Right } else { MenuSide::Left }
    } else if slot.center().y < bounds.center().y {
        MenuSide::Below
    } else {
        MenuSide::Above
    };
    anchor
}

/// Düz RGBA → premultiplied doku verisi.
fn image_data(p: &Pixmap) -> Option<ImageData> {
    let image = nuxut_image::Image::from_straight(p.width, p.height, p.rgba.to_vec()).ok()?;
    Some(ImageData { width: p.width, height: p.height, pixels: image.into_pixels().into() })
}

#[derive(Debug)]
struct CachedTexture {
    /// Hangi görüntüden yüklendi (aynı `Arc` → yeniden yükleme yok).
    source: Arc<[u8]>,
    texture: Texture,
}

/// Menü öğesi ikon dokuları: servisin çözdüğü `MenuNode::image` başına bir doku; yalnızca
/// son kurulan modelde kullanılanlar tutulur (menü kapanınca/değişince serbest kalır).
#[derive(Debug, Default)]
struct MenuIcons {
    /// Anahtar: piksel verisinin adresi (aynı `Arc` → aynı doku).
    textures: HashMap<usize, CachedTexture>,
}

impl MenuIcons {
    fn model(&mut self, tree: &MenuTree, painter: &mut Painter) -> MenuModel {
        let mut used = HashMap::new();
        let old = &mut self.textures;
        let model = menu_model_with(tree, &mut |node| {
            let image = node.image.as_ref()?;
            let key = Arc::as_ptr(&image.rgba) as *const u8 as usize;
            let texture = match old.remove(&key).or_else(|| used.remove(&key)) {
                Some(c) => c,
                None => CachedTexture {
                    source: Arc::clone(&image.rgba),
                    texture: painter.upload_texture(image_data(image)?, TextureOptions::default()),
                },
            };
            let icon = if is_symbolic(node) {
                Icon::symbolic(texture.texture.clone())
            } else {
                Icon::image(texture.texture.clone())
            };
            used.insert(key, texture);
            Some(icon)
        });
        self.textures = used;
        model
    }

    fn clear(&mut self) {
        self.textures.clear();
    }
}

/// Menü durumu.
///
/// Açılış akışı: tıklama → [`Tray::request_menu`] yeni bir istek kimliği alır, `OpenMenu`
/// gönderir ve **yalnızca o kimlikle gelen** ağacı bekler (`TrayModel::menu_layout`). Aynı
/// ağaç ikinci kez gelse de yeni kimlik modeli değiştirir, `sync` çalışır, menü açılır; önceki
/// açılışın geç gelen yanıtı ise kimlik tutmadığı için yok sayılır.
#[derive(Debug, Default)]
struct MenuState {
    /// Menüsü gösterilen (ya da açılması beklenen) öğe ve bağlantı yuvası.
    item: Option<(ItemId, Rect)>,
    /// Beklenen/gösterilen açılışın istek kimliği.
    request: Option<MenuRequest>,
    /// Son gösterilen ağaç (modeldeki değişikliği fark etmek için).
    shown: Option<MenuTree>,
    /// Çizimde (Painter + TextSystem varken) uygulanacak iş: açılacak/güncellenecek ağaç.
    /// İkon dokuları yüklenebilsin diye model çizim sırasında kurulur.
    pending: Option<MenuTree>,
    /// Ekrandaki menünün ait olduğu açılış: aynıysa model güncellenir, değilse yeni bağlantıya
    /// açılır.
    displayed: Option<MenuRequest>,
    /// Son işlenen servis kaynaklı istek (`TrayModel::menu_request`).
    last_service_request: Option<MenuRequest>,
    /// Açık menüyü kendi yuvasına basarak kapatan tıklamanın bırakılması yutulur (aç/kapa):
    /// aynı tıklama menüyü yeniden açmaz, bar'a da sol tık olarak düşmez.
    swallow_release: Option<ItemId>,
}

impl MenuState {
    /// Açılışı unutur (ekrandaki menünün kapanışı ayrıca yapılır).
    fn forget(&mut self) {
        self.item = None;
        self.request = None;
        self.shown = None;
        self.pending = None;
        self.displayed = None;
    }
}

#[derive(Debug)]
pub struct Tray {
    slots: Vec<Slot>,
    hovers: Vec<Interaction>,
    /// Son çizimdeki yuva dikdörtgenleri (overlay işaretçisi için).
    rects: Vec<Rect>,
    textures: HashMap<ItemId, CachedTexture>,
    menu: Menu,
    menu_state: MenuState,
    menu_icons: MenuIcons,
    /// Menünün sığacağı alan (yüzey koordinatı).
    bounds: Rect,
    /// Widget kurulmadan önceki servis istekleri (ör. config yeniden yüklenince modelde kalan
    /// eski `MenuRequested`) menü açmaz: kimlikler süreç genelinde artar.
    created: MenuRequest,
}

impl Tray {
    pub fn new(cx: &WidgetCx) -> Self {
        Self {
            slots: Vec::new(),
            hovers: Vec::new(),
            rects: Vec::new(),
            textures: HashMap::new(),
            menu: Menu::new(cx.theme),
            menu_state: MenuState::default(),
            menu_icons: MenuIcons::default(),
            bounds: Rect::new(0.0, 0.0, 0.0, 0.0),
            created: next_menu_request(),
        }
    }

    fn gap(cx: &WidgetCx) -> f32 {
        cx.theme.space(1)
    }

    /// Kullanıcı bu öğenin menüsünü istedi: yeni açılış başlatır, servise gidecek komutu döndürür.
    fn request_menu(&mut self, id: &ItemId, slot: Rect) -> TrayCommand {
        let request = next_menu_request();
        self.await_menu(id, slot, request);
        TrayCommand::OpenMenu { item: id.clone(), request }
    }

    /// `request` açılışının ağacını bekle (gelince `sync` → `pending` → çizimde açılır).
    fn await_menu(&mut self, id: &ItemId, slot: Rect, request: MenuRequest) {
        let s = &mut self.menu_state;
        s.item = Some((id.clone(), slot));
        s.request = Some(request);
        s.shown = None;
        s.pending = None;
    }

    /// Açılacak/güncellenecek ağaç ve bağlantı yuvası (bekleyen iş yoksa `None`).
    fn take_pending(&mut self) -> Option<(MenuTree, Rect)> {
        let tree = self.menu_state.pending.take()?;
        let (_, slot) = self.menu_state.item.clone()?;
        Some((tree, slot))
    }

    /// Ağacı gösterir: ekrandaki menü bu açılışınsa model güncellenir, değilse menü bu yuvaya açılır.
    fn show_menu(&mut self, text: &mut TextSystem, theme: &Theme, model: MenuModel, slot: Rect, vertical: bool, now: Instant) {
        let request = self.menu_state.request;
        if self.menu.is_open() && self.menu_state.displayed == request {
            self.menu.set_model(text, theme, model, now);
        } else {
            let bounds = self.bounds;
            self.menu.open(text, theme, model, menu_anchor(slot, vertical, bounds), bounds, now);
        }
        self.menu_state.displayed = request;
    }

    /// Bekleyen menü işini uygular (çizim sırasında: ikon dokuları yüklenir). Görünüm
    /// değiştiyse `true`.
    fn apply_pending(&mut self, painter: &mut Painter, cx: &WidgetCx) -> bool {
        let (theme, now) = (cx.theme, cx.now);
        // Çıkış boyutu / çerçeve değişti: açık menü yeni alana sığdırılır.
        let mut changed = false;
        if self.bounds != cx.bounds {
            self.bounds = cx.bounds;
            if self.menu.is_visible() {
                self.menu.set_bounds(painter.text_system(), theme, self.bounds, now);
                changed = true;
            }
        }
        let Some((tree, slot)) = self.take_pending() else { return changed };
        let model = self.menu_icons.model(&tree, painter);
        self.show_menu(painter.text_system(), theme, model, slot, cx.vertical, now);
        true
    }

    fn close_menu(&mut self, now: Instant) -> bool {
        let was = self.menu.is_open();
        if was {
            self.menu.close(now);
        }
        self.menu_state.forget();
        self.menu_icons.clear();
        was
    }

    fn menu_item(&self) -> Option<&ItemId> {
        self.menu_state.item.as_ref().map(|(id, _)| id)
    }

    fn slot_id_at(&self, p: Point) -> Option<&ItemId> {
        slot_at(&self.rects, p).and_then(|i| self.slots.get(i)).map(|s| &s.id)
    }

    /// Yuva eylemini komuta çevirir (menü açılışı istek kimliği alır).
    fn slot_command(&mut self, action: SlotAction, id: &ItemId, slot: Rect) -> TrayCommand {
        match action {
            SlotAction::OpenMenu => self.request_menu(id, slot),
            SlotAction::Command(command) => command,
        }
    }

    /// Menü olayını işler ve eylemi döndürür.
    fn on_menu_event(&mut self, event: Option<MenuEvent>) -> Option<Action> {
        let event = event?;
        let item = self.menu_item()?.clone();
        if matches!(event, MenuEvent::Activated(_) | MenuEvent::Dismissed) {
            // Menü kendi kapanışını başlattı; durum temizlenir.
            self.menu_state.forget();
        }
        menu_command(&item, &event).map(Action::Tray)
    }

    /// İkon dokusu (gerekirse yükler).
    fn texture(&mut self, painter: &mut Painter, slot: &Slot) -> Option<Texture> {
        let image = slot.image.as_ref()?;
        if let Some(c) = self.textures.get(&slot.id)
            && Arc::ptr_eq(&c.source, &image.rgba)
        {
            return Some(c.texture.clone());
        }
        let data = image_data(image)?;
        let texture = painter.upload_texture(data, TextureOptions::default());
        self.textures.insert(slot.id.clone(), CachedTexture { source: Arc::clone(&image.rgba), texture: texture.clone() });
        Some(texture)
    }
}

impl BarWidget for Tray {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Tray
    }

    fn sync(&mut self, model: &ShellModel, _monitor: &str, cx: &WidgetCx) -> bool {
        let tray = &model.tray;
        let slots: Vec<Slot> = tray.visible().map(Slot::from_item).collect();
        let mut changed = slots != self.slots;
        if changed {
            self.textures.retain(|id, _| slots.iter().any(|s| &s.id == id));
            self.hovers.resize_with(slots.len(), || Interaction::new(cx.theme));
            self.slots = slots;
        }

        // Activate desteklemeyen öğe: servis menüyü açmamızı istedi (ve okumaya başladı). Bu
        // arada kullanıcı başka bir menü açtıysa ya da istek widget'tan eskiyse yok sayılır.
        if let Some((request, id)) = tray.menu_request()
            && self.menu_state.last_service_request != Some(request)
        {
            self.menu_state.last_service_request = Some(request);
            let idle = self.menu_state.item.is_none() && !self.menu.is_open();
            if idle
                && request > self.created
                && let Some(i) = self.slots.iter().position(|s| &s.id == id)
            {
                let rect = self.rects.get(i).copied().unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
                let id = id.clone();
                self.await_menu(&id, rect, request);
                changed = true;
            }
        }

        // Menüsü gösterilen öğe kayboldu → kapat; bu açılışın ağacı geldi/değişti → göster.
        if let Some(id) = self.menu_item().cloned() {
            if tray.get(&id).is_none() {
                changed |= self.close_menu(cx.now);
            } else if let Some((request, tree)) = tray.menu_layout(&id)
                && self.menu_state.request == Some(request)
                && self.menu_state.shown.as_ref() != Some(tree)
            {
                self.menu_state.shown = Some(tree.clone());
                self.menu_state.pending = Some(tree.clone());
                changed = true;
            }
        }
        changed
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        for h in &mut self.hovers {
            h.set_theme(cx.theme, cx.now);
        }
    }

    fn measure(&mut self, _text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        total_length(self.slots.len(), cx.item, Self::gap(cx))
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, _highlighted: bool) {
        self.apply_pending(painter, cx);
        self.rects = slot_rects(rect, self.slots.len(), cx.item, Self::gap(cx), cx.vertical);
        let colors = &cx.theme.colors;
        let icon = cx.settings.tray.icon_size.min(cx.item);
        let slots = self.slots.clone();
        for (i, slot) in slots.iter().enumerate() {
            let r = self.rects[i];
            let radii = Corners::uniform(r.width * 0.5);
            let open = self.menu.is_open() && self.menu_item() == Some(&slot.id);
            if open {
                painter.rect(r, &RectStyle::filled(colors.surface_bright, radii));
            }
            if let Some(h) = self.hovers.get(i) {
                // İçerik uygulama ikonu (rengi bilinmez); ölçüt bar metni (`on_surface`).
                draw_hover(painter, h, r, radii, cx, chip_fill(cx.theme, open), &[colors.on_surface]);
            }
            let icon_rect = Rect::new(r.center().x - icon * 0.5, r.center().y - icon * 0.5, icon, icon);
            match self.texture(painter, slot) {
                Some(texture) => painter.image(icon_rect, &texture, &ImageStyle::new(ImageFit::Contain)),
                None => {
                    // İkonsuz öğe: tema renginde küçük yer tutucu.
                    let s = icon * 0.45;
                    let dot = Rect::new(r.center().x - s * 0.5, r.center().y - s * 0.5, s, s);
                    painter.rect(dot, &RectStyle::filled(colors.on_surface_variant, Corners::uniform(s * 0.5)));
                }
            }
            if slot.attention_badge {
                let s = (icon * 0.3).max(4.0);
                let badge = Rect::new(icon_rect.x + icon_rect.width - s * 0.75, icon_rect.y - s * 0.25, s, s);
                painter.rect(badge, &RectStyle::filled(colors.primary, Corners::uniform(s * 0.5)));
            }
        }
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        let rects = slot_rects(rect, self.slots.len(), cx.item, Self::gap(cx), cx.vertical);
        let mut changed = false;
        for (h, r) in self.hovers.iter_mut().zip(&rects) {
            changed |= h.pointer(*r, pointer, down, cx.now);
        }
        changed
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        let rects = slot_rects(rect, self.slots.len(), cx.item, Self::gap(cx), cx.vertical);
        let i = slot_at(&rects, at)?;
        let slot = self.slots.get(i)?.clone();
        let command = self.slot_command(primary_action(&slot, at), &slot.id, rects[i]);
        Some(Action::Tray(command))
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        let hover = self.hovers.iter_mut().fold(false, |acc, h| acc | h.is_animating(now));
        hover | self.menu.is_animating(now) | self.menu_state.pending.is_some()
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        let rects = slot_rects(rect, self.slots.len(), cx.item, Self::gap(cx), cx.vertical);
        slot_at(&rects, p).map(|_| CursorShape::Pointer)
    }

    fn overlay_cursor(&self, p: Point) -> Option<CursorShape> {
        self.menu.is_open().then(|| self.menu.cursor(p)).flatten()
    }

    fn overlay_shapes(&mut self, cx: &WidgetCx) -> Vec<BlobShape> {
        self.menu.shape(cx.now, cx.theme).into_iter().collect()
    }

    fn draw_overlay(&mut self, ui: &mut Ui, cx: &WidgetCx) {
        self.apply_pending(&mut *ui.painter, cx);
        if self.menu.is_visible() {
            self.menu.draw_content(ui);
        }
    }

    fn overlay_input(&self) -> Vec<Rect> {
        self.menu.input_rect().into_iter().collect()
    }

    fn overlay_pointer(
        &mut self,
        event: &PointerEvent,
        text: &mut TextSystem,
        cx: &WidgetCx,
    ) -> Option<(bool, Option<Action>)> {
        let now = cx.now;
        let press = matches!(event.kind, PointerKind::Press(_));
        if press {
            // Yeni tıklama: önceki kapatan tıklamanın yutma işareti artık geçersiz.
            self.menu_state.swallow_release = None;
        }

        // Açık menü: içerideki her olay ve dışarı basma (kapatır) menünün.
        if self.menu.is_open() {
            let inside = self.menu.input_rect().is_some_and(|r| r.contains(event.position));
            if inside || press {
                let shown_for = self.menu_item().cloned();
                let response = self.menu.pointer(text, cx.theme, event, now);
                let dismissed = matches!(response.event, Some(MenuEvent::Dismissed));
                let action = self.on_menu_event(response.event);
                // Menüyü kendi ikonuna basarak kapatan tıklama yalnızca kapatır: bırakılması
                // menüyü yeniden açmamalı (aç/kapa).
                if dismissed && shown_for.is_some() && self.slot_id_at(event.position) == shown_for.as_ref() {
                    self.menu_state.swallow_release = shown_for;
                }
                return Some((response.repaint || action.is_some(), action));
            }
        }

        let hit = slot_at(&self.rects, event.position);
        // Henüz gösterilmemiş (ağacı yolda) açılış: yeni bir basma onu iptal eder; geç gelen
        // ağaç kullanıcı başka işe geçmişken menü açmaz. Basma yuvadaysa bırakmada yeni açılış
        // başlar.
        if press && self.menu_state.item.is_some() && !self.menu.is_open() {
            self.menu_state.forget();
        }
        if let PointerKind::Release(_) = event.kind
            && let Some(id) = self.menu_state.swallow_release.take()
            && hit.and_then(|i| self.slots.get(i)).is_some_and(|s| s.id == id)
        {
            return Some((false, None));
        }

        // Kapalı menü: sağ/orta tık ve tekerlek ikonlarda (sol tık bar isabet testinde).
        let i = hit?;
        let slot = self.slots.get(i)?.clone();
        let rect = self.rects[i];
        let command = match event.kind {
            PointerKind::Press(Button::Right | Button::Middle) => return Some((false, None)),
            PointerKind::Release(button @ (Button::Right | Button::Middle)) => {
                button_action(&slot, button, event.position).map(|a| self.slot_command(a, &slot.id, rect))
            }
            PointerKind::Scroll { delta, steps } => scroll_command(&slot.id, delta, steps),
            _ => return None,
        };
        Some((command.is_some(), command.map(Action::Tray)))
    }

    fn wants_keyboard(&self) -> bool {
        self.menu.is_open()
    }

    fn key(&mut self, event: &KeyEvent, text: &mut TextSystem, cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        if !self.menu.is_open() {
            return None;
        }
        let redraw = false;
        let response = self.menu.key(text, cx.theme, event, cx.now);
        let action = self.on_menu_event(response.event);
        Some((redraw || response.repaint || action.is_some(), action))
    }

    fn close_overlay(&mut self, now: Instant) -> bool {
        self.close_menu(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(id: &str, item_is_menu: bool, has_menu: bool) -> Slot {
        Slot {
            id: crate::services::tray::ItemId::new(id, "/SNI"),
            image: None,
            attention_badge: false,
            item_is_menu,
            has_menu,
        }
    }

    #[test]
    fn layout_horizontal_and_vertical() {
        assert_eq!(total_length(0, 24.0, 4.0), 0.0);
        assert_eq!(total_length(3, 24.0, 4.0), 80.0);
        let r = slot_rects(Rect::new(100.0, 0.0, 80.0, 34.0), 3, 24.0, 4.0, false);
        assert_eq!(r[0], Rect::new(100.0, 5.0, 24.0, 24.0));
        assert_eq!(r[2], Rect::new(156.0, 5.0, 24.0, 24.0));
        let v = slot_rects(Rect::new(0.0, 200.0, 34.0, 100.0), 2, 24.0, 4.0, true);
        assert_eq!(v[0], Rect::new(5.0, 224.0, 24.0, 24.0), "dikeyde ortalı");
        assert_eq!(v[1].y, 252.0);
        assert_eq!(slot_at(&r, Point::new(130.0, 10.0)), Some(1));
        assert_eq!(slot_at(&r, Point::new(126.0, 10.0)), None, "boşlukta isabet yok");
    }

    #[test]
    fn primary_click_respects_item_is_menu() {
        let at = Point::new(10.4, 3.6);
        let s = slot(":1.1", false, true);
        assert_eq!(primary_action(&s, at), SlotAction::Command(TrayCommand::Activate { item: s.id.clone(), x: 10, y: 4 }));
        let m = slot(":1.2", true, true);
        assert_eq!(primary_action(&m, at), SlotAction::OpenMenu);
        let broken = slot(":1.3", true, false);
        assert!(
            matches!(primary_action(&broken, at), SlotAction::Command(TrayCommand::Activate { .. })),
            "menüsüz yalnızca-menü öğe"
        );
    }

    #[test]
    fn secondary_buttons() {
        let at = Point::new(3.0, 4.0);
        let with_menu = slot(":1.1", false, true);
        assert_eq!(button_action(&with_menu, Button::Right, at), Some(SlotAction::OpenMenu));
        let plain = slot(":1.2", false, false);
        assert_eq!(
            button_action(&plain, Button::Right, at),
            Some(SlotAction::Command(TrayCommand::ContextMenu { item: plain.id.clone(), x: 3, y: 4 }))
        );
        assert_eq!(
            button_action(&plain, Button::Middle, at),
            Some(SlotAction::Command(TrayCommand::SecondaryActivate { item: plain.id.clone(), x: 3, y: 4 }))
        );
        assert_eq!(button_action(&plain, Button::Left, at), None);
    }

    /// Tek geçici yüzey (1): menü açan komutlar (sağ tık dbusmenu, bağlam menüsü) geçici yaprağı
    /// kapattırır; diğerleri dokunmaz.
    #[test]
    fn menu_opening_commands() {
        let item = crate::services::tray::ItemId::new(":1.1", "/SNI");
        let open = TrayCommand::OpenMenu { item: item.clone(), request: next_menu_request() };
        assert!(open.opens_menu());
        assert!(TrayCommand::ContextMenu { item: item.clone(), x: 1, y: 2 }.opens_menu());
        assert!(!TrayCommand::Activate { item: item.clone(), x: 1, y: 2 }.opens_menu());
        assert!(!TrayCommand::CloseMenu(item.clone()).opens_menu());
        assert!(!TrayCommand::MenuClicked { item, id: 1 }.opens_menu());
    }

    #[test]
    fn scroll_mapping() {
        let id = crate::services::tray::ItemId::new(":1.1", "/SNI");
        let down = scroll_command(&id, Point::new(0.0, 15.0), Point::new(0.0, 1.0)).unwrap();
        assert_eq!(down, TrayCommand::Scroll { item: id.clone(), delta: -120, horizontal: false });
        let touch = scroll_command(&id, Point::new(-7.6, 1.0), Point::new(0.0, 0.0)).unwrap();
        assert_eq!(touch, TrayCommand::Scroll { item: id.clone(), delta: 8, horizontal: true });
        assert_eq!(scroll_command(&id, Point::new(0.0, 0.0), Point::new(0.0, 0.0)), None);
        assert_eq!(scroll_command(&id, Point::new(f32::NAN, 0.0), Point::new(0.0, 0.0)), None);
    }

    #[test]
    fn menu_events_map_to_commands() {
        let id = crate::services::tray::ItemId::new(":1.1", "/SNI");
        assert_eq!(menu_command(&id, &MenuEvent::Activated(5)), Some(TrayCommand::MenuClicked { item: id.clone(), id: 5 }));
        assert_eq!(
            menu_command(&id, &MenuEvent::SubmenuOpened(3)),
            Some(TrayCommand::MenuAboutToShow { item: id.clone(), ids: vec![3] })
        );
        assert_eq!(menu_command(&id, &MenuEvent::Dismissed), Some(TrayCommand::CloseMenu(id.clone())));
        assert_eq!(menu_command(&id, &MenuEvent::Activated(u64::MAX)), None);
    }

    #[test]
    fn premultiplies_textures() {
        let p = Pixmap { width: 1, height: 1, rgba: Arc::from(vec![200u8, 100, 50, 128]) };
        let d = image_data(&p).unwrap();
        assert_eq!((d.width, d.height), (1, 1));
        assert!(d.pixels[0] < 200 && d.pixels[3] == 128, "premultiplied: {:?}", &d.pixels[..]);
    }

    /// "Tek geçici yüzey" (2): sabit panel varken tepsi menüsü panelle asla kesişmez. Menü,
    /// shell'in yaptığı gibi açılır: sınır = `panels::free_area` (`WidgetCx::bounds`), bağlantı
    /// = `menu_anchor` (yön serbest alana göre; framework sığmazsa çevirir, içinde kaydırır).
    mod pinned_exclusion {
        use super::*;
        use crate::layout::Edge;
        use crate::panels::free_area;
        use crate::services::tray::{MenuNode, menu_model};

        fn tree() -> MenuTree {
            let label = |id: i32, s: &str| MenuNode { label: s.into(), ..MenuNode::new(id) };
            let children = vec![
                label(1, "Aç"),
                label(2, "Seçenekler ve uzun bir menü öğesi etiketi"),
                label(3, "Hakkında"),
                label(4, "Çıkış"),
            ];
            MenuTree { revision: 1, root: MenuNode { children, ..MenuNode::new(0) } }
        }

        /// İç alandan `edge` kenarında `depth` kalınlığında sabit panel dikdörtgeni.
        fn panel_rect(inner: Rect, edge: Edge, depth: f32) -> Rect {
            match edge {
                Edge::Top => Rect::new(inner.x, inner.y, inner.width, depth),
                Edge::Bottom => Rect::new(inner.x, inner.bottom() - depth, inner.width, depth),
                Edge::Left => Rect::new(inner.x, inner.y, depth, inner.height),
                Edge::Right => Rect::new(inner.right() - depth, inner.y, depth, inner.height),
            }
        }

        fn intersects(a: Rect, b: Rect) -> bool {
            a.x < b.right() - 0.01 && b.x < a.right() - 0.01 && a.y < b.bottom() - 0.01 && b.y < a.bottom() - 0.01
        }

        /// Menüyü açar, hedef dikdörtgenini döndürür; panelle kesişmediğini ve serbest alanda
        /// kaldığını doğrular.
        fn open(inner: Rect, pinned: Edge, depth: f32, slot: Rect, vertical: bool) -> Rect {
            let theme = Theme::default();
            let mut text = TextSystem::new();
            let bounds = free_area(inner, &[(pinned, depth)]);
            let panel = panel_rect(inner, pinned, depth);
            let mut menu = Menu::new(&theme);
            menu.open(&mut text, &theme, menu_model(&tree()), menu_anchor(slot, vertical, bounds), bounds, Instant::now());
            let r = menu.input_rect().expect("menü açık");
            assert!(r.width > 0.0 && r.height > 0.0, "boş menü: {r:?}");
            assert!(!intersects(r, panel), "menü {r:?} sabit panelle {panel:?} kesişiyor");
            assert!(
                r.x >= bounds.x - 0.01
                    && r.right() <= bounds.right() + 0.01
                    && r.y >= bounds.y - 0.01
                    && r.bottom() <= bounds.bottom() + 0.01,
                "menü {r:?} serbest alanın {bounds:?} dışında"
            );
            r
        }

        const INNER: Rect = Rect::new(8.0, 34.0, 1904.0, 1038.0);

        #[test]
        fn right_panel_pushes_menu_left_of_its_inner_edge() {
            // Kullanıcı hatası: sağ panel açıkken sağdaki tepsi öğesinin menüsü panelin üstüne biniyordu.
            let slot = Rect::new(1714.0, 5.0, 24.0, 24.0);
            let r = open(INNER, Edge::Right, 380.0, slot, false);
            assert!(r.right() <= INNER.right() - 380.0 + 0.01);
            assert!((r.y - INNER.y).abs() < 0.01, "bar'a bağlı kalır (altında): {r:?}");
        }

        #[test]
        fn left_panel_pushes_menu_right() {
            let slot = Rect::new(40.0, 5.0, 24.0, 24.0);
            let r = open(INNER, Edge::Left, 380.0, slot, false);
            assert!(r.x >= INNER.x + 380.0 - 0.01);
        }

        #[test]
        fn vertical_bar_on_panel_edge_opens_away_from_panel() {
            // Bar sağda (dikey), sağ panel sabit: menü panelin soluna açılır.
            let inner = Rect::new(42.0, 8.0, 1870.0, 1064.0);
            let slot = Rect::new(1879.0, 900.0, 24.0, 24.0);
            let r = open(inner, Edge::Right, 380.0, slot, true);
            assert!(r.right() <= inner.right() - 380.0 + 0.01);
            // Bar solda (dikey), sol panel sabit: panelin sağına.
            let inner = Rect::new(42.0, 8.0, 1870.0, 1064.0);
            let slot = Rect::new(9.0, 900.0, 24.0, 24.0);
            let r = open(inner, Edge::Left, 380.0, slot, true);
            assert!(r.x >= inner.x + 380.0 - 0.01);
        }

        #[test]
        fn bottom_bar_flips_above_pinned_bottom_panel() {
            // Bar altta, alt panel sabit: menü yukarı açılır ve panelin üstünde kalır.
            let inner = Rect::new(8.0, 8.0, 1904.0, 1038.0);
            let slot = Rect::new(1714.0, 1051.0, 24.0, 24.0);
            let r = open(inner, Edge::Bottom, 300.0, slot, false);
            assert!(r.bottom() <= inner.bottom() - 300.0 + 0.01, "{r:?}");
        }

        #[test]
        fn narrow_output_shrinks_menu_into_free_area() {
            // Dar çıkış: serbest alan menüden dar; menü daralır, panele taşmaz.
            let inner = Rect::new(8.0, 34.0, 584.0, 400.0);
            let slot = Rect::new(560.0, 5.0, 24.0, 24.0);
            let r = open(inner, Edge::Right, 450.0, slot, false);
            assert!(r.width <= 584.0 - 450.0 + 0.01, "{r:?}");
        }
    }

    /// Kullanıcı yorumu 4: "sağ tık bazen menü açıyor bazen açmıyor; hızlı tıklamalarda
    /// karışıyor". Uygulama gibi davranan düzenek: servis olayı modele uygulanır ve **yalnızca
    /// model değişince** `sync` çalışır (`App::on_tray` → `model_changed`); çizim
    /// `take_pending` + `show_menu`; işaretçi önce overlay'e, tüketilmeyen sol bırakma bar'a
    /// (`click`) gider (`Screen::pointer`).
    mod scenarios {
        use std::collections::HashMap;
        use std::time::Duration;

        use super::*;
        use crate::services::tray::{MenuNode, TrayEvent, menu_model};
        use crate::widgets::WidgetSettings;

        fn bar() -> Rect {
            Rect::new(0.0, 0.0, 200.0, 30.0)
        }

        const OUTSIDE: Point = Point::new(1500.0, 900.0);

        fn tree() -> MenuTree {
            let open = MenuNode { label: "Aç".into(), ..MenuNode::new(1) };
            MenuTree { revision: 1, root: MenuNode { children: vec![open], ..MenuNode::new(0) } }
        }

        struct Harness {
            theme: &'static Theme,
            settings: &'static WidgetSettings,
            model: ShellModel,
            w: Tray,
            text: TextSystem,
            now: Instant,
            sent: Vec<TrayCommand>,
        }

        impl Harness {
            /// `(sahip, yalnızca-menü)` öğeleri, hepsi dbusmenu'lü.
            fn new(items: &[(&str, bool)]) -> Self {
                let theme: &'static Theme = Box::leak(Box::default());
                let settings: &'static WidgetSettings = Box::leak(Box::default());
                let now = Instant::now();
                let cx = Self::make_cx(theme, settings, now);
                let mut h = Self {
                    theme,
                    settings,
                    model: ShellModel::default(),
                    w: Tray::new(&cx),
                    text: TextSystem::new(),
                    now,
                    sent: Vec::new(),
                };
                for (seq, (owner, is_menu)) in items.iter().enumerate() {
                    let mut item =
                        TrayItem::from_properties(ItemId::new(owner, "/SNI"), seq as u64, owner, "/SNI", &HashMap::new());
                    item.menu = Some("/MenuBar".into());
                    item.item_is_menu = *is_menu;
                    item.status = Status::Active;
                    h.deliver(TrayEvent::ItemAdded(item));
                }
                let gap = Tray::gap(&h.cx());
                h.w.rects = slot_rects(bar(), h.w.slots.len(), 26.0, gap, false);
                h
            }

            fn make_cx(theme: &'static Theme, settings: &'static WidgetSettings, now: Instant) -> WidgetCx<'static> {
                WidgetCx { theme, settings, vertical: false, item: 26.0, now, bounds: Rect::new(0.0, 0.0, 1920.0, 1080.0) }
            }

            fn cx(&self) -> WidgetCx<'static> {
                Self::make_cx(self.theme, self.settings, self.now)
            }

            fn id(&self, i: usize) -> ItemId {
                self.w.slots[i].id.clone()
            }

            fn at(&self, i: usize) -> Point {
                self.w.rects[i].center()
            }

            /// Servis olayı → model; değiştiyse `sync`; sonra bir kare.
            fn deliver(&mut self, e: TrayEvent) {
                if self.model.tray.apply(&e) {
                    let cx = self.cx();
                    self.w.sync(&self.model, "M", &cx);
                }
                self.frame();
            }

            fn frame(&mut self) {
                self.now += Duration::from_millis(16);
                self.w.bounds = self.cx().bounds;
                if let Some((tree, slot)) = self.w.take_pending() {
                    self.w.show_menu(&mut self.text, self.theme, menu_model(&tree), slot, false, self.now);
                }
            }

            fn pointer(&mut self, kind: PointerKind, at: Point) {
                let cx = self.cx();
                let event = PointerEvent::new(at, kind);
                let action = match self.w.overlay_pointer(&event, &mut self.text, &cx) {
                    Some((_, action)) => action,
                    None if kind == PointerKind::Release(Button::Left) => self.w.click(bar(), at, &cx),
                    None => None,
                };
                if let Some(Action::Tray(command)) = action {
                    self.sent.push(command);
                }
                self.frame();
            }

            fn click(&mut self, button: Button, at: Point) {
                self.pointer(PointerKind::Press(button), at);
                self.pointer(PointerKind::Release(button), at);
            }

            /// Gönderilen `OpenMenu` istekleri, sırayla.
            fn requests(&self) -> Vec<MenuRequest> {
                self.sent
                    .iter()
                    .filter_map(|c| if let TrayCommand::OpenMenu { request, .. } = c { Some(*request) } else { None })
                    .collect()
            }

            fn last_request(&self) -> MenuRequest {
                *self.requests().last().expect("OpenMenu gönderilmedi")
            }

            /// Sahte servis yanıtı: her açılışta aynı ağaç (statik menülü uygulama).
            fn answer(&mut self, item: &ItemId, request: MenuRequest) {
                self.deliver(TrayEvent::MenuLayout { item: item.clone(), request, menu: tree() });
            }

            /// Ekranda açık menünün öğesi.
            fn shown(&self) -> Option<ItemId> {
                if self.w.menu.is_open() { self.w.menu_item().cloned() } else { None }
            }
        }

        /// Kök neden: ikinci açılışta servis aynı ağacı getirir; model "değişmedi" derse `sync`
        /// çalışmaz ve menü hiç açılmaz (ya da sonra ilgisiz bir model değişikliğinde açılır).
        #[test]
        fn reopening_with_identical_tree_shows_menu() {
            let mut h = Harness::new(&[(":1.1", false)]);
            let a = h.id(0);
            for round in 0..4 {
                h.click(Button::Right, h.at(0));
                let r = h.last_request();
                assert_eq!(h.shown(), None, "{round}: ağaç gelmeden menü yok");
                h.answer(&a, r);
                assert_eq!(h.shown(), Some(a.clone()), "{round}: menü açılır");
                h.click(Button::Left, OUTSIDE);
                assert_eq!(h.shown(), None, "{round}: dışarı tık kapatır");
                assert_eq!(h.sent.last(), Some(&TrayCommand::CloseMenu(a.clone())));
            }
            let requests = h.requests();
            assert_eq!(requests.len(), 4);
            assert!(requests.windows(2).all(|w| w[0] < w[1]), "her açılış yeni kimlik: {requests:?}");
        }

        /// Açık menünün ikonuna tekrar sağ tık yalnızca kapatır; sonradan gelen model
        /// değişiklikleri ya da eski açılışın güncellemesi menüyü geri getirmez.
        #[test]
        fn right_click_on_open_icon_toggles_closed() {
            let mut h = Harness::new(&[(":1.1", false)]);
            let a = h.id(0);
            h.click(Button::Right, h.at(0));
            let r = h.last_request();
            h.answer(&a, r);
            assert_eq!(h.shown(), Some(a.clone()));

            h.click(Button::Right, h.at(0));
            assert_eq!(h.shown(), None, "ikinci tık kapatır");
            assert_eq!(h.requests(), [r], "kapatan tık yeni açılış başlatmaz");
            assert_eq!(h.sent.last(), Some(&TrayCommand::CloseMenu(a.clone())));

            // Eski açılışın güncellemesi ve ilgisiz model değişikliği.
            let mut changed = tree();
            changed.revision = 2;
            h.deliver(TrayEvent::MenuLayout { item: a.clone(), request: r, menu: changed });
            let mut item = h.model.tray.get(&a).cloned().unwrap();
            item.title = "yeni başlık".into();
            h.deliver(TrayEvent::ItemUpdated(item));
            assert_eq!(h.shown(), None, "kapanan menü kendiliğinden açılmaz");

            // Üçüncü tık yine açar.
            h.click(Button::Right, h.at(0));
            let r3 = h.last_request();
            assert_ne!(r3, r);
            h.answer(&a, r3);
            assert_eq!(h.shown(), Some(a));
        }

        /// Ağaç gelmeden art arda sağ tıklar: yalnızca son açılışın yanıtı menüyü açar, eski
        /// yanıtlar (sırasız gelse de) yok sayılır.
        #[test]
        fn rapid_right_clicks_use_latest_request() {
            let mut h = Harness::new(&[(":1.1", false)]);
            let a = h.id(0);
            for _ in 0..3 {
                h.click(Button::Right, h.at(0));
            }
            let [r1, r2, r3] = h.requests()[..] else { panic!("{:?}", h.sent) };
            h.answer(&a, r2);
            assert_eq!(h.shown(), None, "eski açılışın yanıtı açmaz");
            h.answer(&a, r1);
            assert_eq!(h.shown(), None);
            h.answer(&a, r3);
            assert_eq!(h.shown(), Some(a.clone()));
            assert_eq!(h.w.menu_state.displayed, Some(r3));
            // Gösterildikten sonra gelen eski yanıt menüyü değiştirmez.
            let mut stale = tree();
            stale.revision = 9;
            h.deliver(TrayEvent::MenuLayout { item: a.clone(), request: r1, menu: stale });
            assert_eq!(h.w.menu_state.shown.as_ref().map(|t| t.revision), Some(1));
            assert_eq!(h.shown(), Some(a));
        }

        /// Ağacı yolda olan açılış başka bir tıklamayla iptal olur: geç gelen ağaç kullanıcı
        /// başka işe geçmişken menü açmaz.
        #[test]
        fn pending_open_is_cancelled_by_another_press() {
            let mut h = Harness::new(&[(":1.1", false)]);
            let a = h.id(0);
            h.click(Button::Right, h.at(0));
            let r = h.last_request();
            h.click(Button::Left, OUTSIDE);
            h.answer(&a, r);
            assert_eq!(h.shown(), None);
        }

        /// Menü açıkken başka öğeye sağ tık: ilk menü kapanır, ikinci öğenin menüsü kendi
        /// yuvasına açılır; ilk öğenin geç yanıtı karışmaz.
        #[test]
        fn right_click_on_other_item_while_open() {
            let mut h = Harness::new(&[(":1.1", false), (":1.2", false)]);
            let (a, b) = (h.id(0), h.id(1));
            h.click(Button::Right, h.at(0));
            let ra = h.last_request();
            h.answer(&a, ra);
            assert_eq!(h.shown(), Some(a.clone()));

            h.click(Button::Right, h.at(1));
            let rb = h.last_request();
            assert_ne!(ra, rb);
            let n = h.sent.len();
            assert_eq!(h.sent[n - 2], TrayCommand::CloseMenu(a.clone()), "{:?}", h.sent);
            assert_eq!(h.sent[n - 1], TrayCommand::OpenMenu { item: b.clone(), request: rb });
            h.answer(&a, ra);
            assert_eq!(h.shown(), None, "kapanan öğenin yanıtı açmaz");
            h.answer(&b, rb);
            assert_eq!(h.shown(), Some(b));
            assert_eq!(h.w.menu_state.item.as_ref().map(|(_, r)| *r), Some(h.w.rects[1]), "ikinci yuvaya bağlı");
        }

        /// Sol/sağ karışık: yalnızca-menü öğede sol tık da menüyü açar/kapatır; menüyü kapatan
        /// tık başka bir eylem (Activate, yeniden açılış) üretmez.
        #[test]
        fn mixed_left_and_right_clicks() {
            let mut h = Harness::new(&[(":1.1", true), (":1.2", false)]);
            let (m, n) = (h.id(0), h.id(1));

            h.click(Button::Left, h.at(0));
            let r1 = h.last_request();
            h.answer(&m, r1);
            assert_eq!(h.shown(), Some(m.clone()), "yalnızca-menü öğe sol tıkla açılır");
            h.click(Button::Left, h.at(0));
            assert_eq!(h.shown(), None);
            h.click(Button::Right, h.at(0));
            let r2 = h.last_request();
            h.answer(&m, r2);
            assert_eq!(h.shown(), Some(m.clone()));
            h.click(Button::Left, h.at(0));
            assert_eq!(h.shown(), None, "sağla açılan menü sol tıkla kapanır");
            assert_eq!(h.requests(), [r1, r2]);

            // Normal öğe: menüsü açıkken ikonuna sol tık yalnızca kapatır (Activate yok);
            // kapalıyken sol tık Activate.
            h.click(Button::Right, h.at(1));
            let r3 = h.last_request();
            h.answer(&n, r3);
            assert_eq!(h.shown(), Some(n.clone()));
            h.click(Button::Left, h.at(1));
            assert_eq!(h.shown(), None);
            assert!(!h.sent.iter().any(|c| matches!(c, TrayCommand::Activate { .. })), "{:?}", h.sent);
            h.click(Button::Left, h.at(1));
            assert!(matches!(h.sent.last(), Some(TrayCommand::Activate { item, .. }) if *item == n));
        }

        /// Servisin `MenuRequested`'i (Activate desteklenmiyor): boştayken menüyü o kimlikle
        /// açar; kullanıcı başka menüdeyken ya da istek widget'tan eskiyse yok sayılır.
        #[test]
        fn service_menu_requests() {
            let mut h = Harness::new(&[(":1.1", false), (":1.2", false)]);
            let (a, b) = (h.id(0), h.id(1));
            let r = next_menu_request();
            h.deliver(TrayEvent::MenuRequested { item: a.clone(), request: r });
            h.answer(&a, r);
            assert_eq!(h.shown(), Some(a.clone()));

            let late = next_menu_request();
            h.deliver(TrayEvent::MenuRequested { item: b.clone(), request: late });
            h.answer(&b, late);
            assert_eq!(h.shown(), Some(a.clone()), "açık menü geç istekle ezilmez");

            // Yeniden kurulan widget (config yeniden yükleme) modeldeki eski isteği açmaz.
            let cx = h.cx();
            let mut fresh = Tray::new(&cx);
            fresh.sync(&h.model, "M", &cx);
            assert!(fresh.menu_state.item.is_none() && fresh.menu_state.pending.is_none());
        }
    }
}
