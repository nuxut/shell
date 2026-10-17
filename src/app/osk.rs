//! Ekran klavyesi yürütücüsü (t125): yüzey, işaretçi yönlendirmesi, sürükleme, sanal klavye
//! çağrıları ve `msg osk`. Çekirdek `services::osk::Osk`, yer `services::osk::place`, görünüm
//! `osk_view::KeyboardView`.
//!
//! Yüzey `nuxut-osk` (karar 2026-10-16, kullanıcı isteği): Overlay katmanı, klavyenin
//! kendi boyutunda (yüzerken gölge payıyla). Yanaşıkken alt-sol anchor'lı, exclusive zone -1 ve
//! margin klavye dikdörtgeninden (pencereler itilmez; `[osk] reserve_space = true` eski davranış:
//! yalnızca alt anchor, kendi yüksekliği kadar exclusive zone); yüzerken sol üst anchor + margin.
//! Girdi bölgesi yalnızca klavye dikdörtgeni. Klavye etkileşimi YOK: tıklamak odağı çalmaz,
//! sanal klavyenin tuşları odaklı istemciye gider.
//!
//! Sürükleme (2026-10-16, framework `set_layer_placement`): tutamaçta basılınca örtük
//! kapma konumları bu yüzeye getirir; her harekette yüzeyin anchor/margin/boyutu güncellenir
//! (yeniden kurulmaz, ek yüzey yok). Bırakınca yay ile yerine oturur, yay da aynı yoldan taşır.
//! İşaretçinin yüzey içi konumunun tabanı compositor'a bağlı: `place::GrabFrame`.
//!
//! Kabuk yaprağının (başlatıcı, ağ parolası) yazı alanına da yazılabilsin diye geçici yaprakla
//! birlikte açık kalır: yaprağın odak kapmasına (`hyprland_focus_grab_v1`) klavye yüzeyi de
//! eklenir, klavyeye tıklamak yaprağı kapatmaz (`docs/design/osk.md` "Tek geçici yüzey").

use std::time::Instant;

use nuxut::{
    Anchor, Animated, Context, CursorShape, Insets, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, Painter, Point,
    PointerEvent, Rect, Size, SurfaceId,
};

use super::Shell;
use crate::ipc::OskAction;
use crate::osk_view::{KeyInput, KeyboardView, Shape, shadow_margin};
use crate::services::osk::place::{self, Drag, GrabFrame, Placement};
use crate::services::osk::{Command, Emit, KeyAction, Layout, Osk, OskView, status_json};

/// Yüzey ad alanı (`hyprctl layers`).
pub const NAMESPACE: &str = "nuxut-osk";

/// Klavyenin ekran kenarlarından en az uzaklığı (yanlarda; dar ekranda tuşlar daralır).
const SIDE_MARGIN: f32 = 16.0;

#[derive(Debug, Default)]
pub(super) struct OskRuntime {
    /// Durum makinesi (ilk açılışta kurulur; keymap'ler o zaman derlenir).
    osk: Option<Osk>,
    open: Option<Open>,
    /// Kip ve yüzen konum: oturum boyunca hatırlanır (gizleyip açınca aynı yerde).
    placement: Placement,
    /// Compositor'ın bildirdiği son etkin düzen (`[osk] layout = "auto"` bunu izler).
    compositor_layout: Option<Layout>,
    /// Son uygulanan `[osk]` (yeniden yüklemede yalnızca değişim yeniden kurar).
    config: Option<crate::services::osk::OskConfig>,
    /// İşaretçinin yüzey içi konumunun sürüklerken tabanı (açılışta compositor'a göre).
    grab_frame: GrabFrame,
}

#[derive(Debug)]
struct Open {
    surface: SurfaceId,
    output: OutputId,
    view: KeyboardView,
    /// Klavye dikdörtgeni (çıkış koordinatı).
    rect: Rect,
    /// Yüzeyin çıkıştaki sol üstü: istenen (son `set_layer_placement`) ve son karenin commit'iyle
    /// gönderilen (çizimde kaydedilir; framework yerleşimi `App::draw`'dan önce uygular).
    surface_origin: Point,
    committed_origin: Point,
    exclusive_zone: i32,
    drag: Option<DragSession>,
}

#[derive(Debug)]
struct DragSession {
    drag: Drag,
    /// Basma anında yüzeyin sol üstü (`GrabFrame::Press` tabanı).
    press_origin: Point,
    /// Bırakınca hedefe yaylanan klavye dikdörtgeni (çıkış koordinatı).
    rect: Animated<Rect>,
    /// Bırakıldı: yay oturunca bu yerleşim kalıcı olur.
    landing: Option<Placement>,
}

/// Yüzey yerleşimi: (spec alanları, klavyenin yüzeydeki yeri, yüzeyin çıkıştaki sol üstü).
#[derive(Debug, PartialEq)]
struct SurfacePlan {
    anchor: Anchor,
    size: (u32, u32),
    margin: Insets,
    exclusive_zone: i32,
    frame: Rect,
    origin: Point,
    shape: Shape,
}

/// Yanaşık ve yüzen kip aynı yüzeyi kullanır: sol üst anchor, margin klavye dikdörtgeninden
/// (exclusive zone -1, başka bölgeler yok sayılır), boyut her iki kipte gölge paylı. Sürüklerken
/// kip değişse de yalnızca margin değişir: boyut değişimi (configure gidiş-dönüşü, eski boyutlu
/// tamponun yeni yerde bir kare görünmesi) yok. Yanaşık klavyenin gölgesi çizilmez; girdi bölgesi
/// yalnızca klavye. `reserve`: eski davranış — yalnızca alt anchor (exclusive zone köşe
/// anchor'ında yok sayılır), kendi boyutu, kendi yüksekliği kadar exclusive zone.
fn plan(placement: Placement, rect: Rect, shadow: f32, reserve: bool) -> SurfacePlan {
    let (w, h) = (rect.width, rect.height);
    let shape = match placement {
        Placement::Docked => Shape::Docked,
        Placement::Floating(_) => Shape::Floating,
    };
    if reserve && shape == Shape::Docked {
        return SurfacePlan {
            anchor: Anchor::BOTTOM,
            size: (w as u32, h as u32),
            margin: Insets::ZERO,
            exclusive_zone: h as i32,
            frame: Rect::new(0.0, 0.0, w, h),
            origin: Point::new(rect.x, rect.y),
            shape,
        };
    }
    SurfacePlan {
        anchor: Anchor::TOP | Anchor::LEFT,
        size: ((w + 2.0 * shadow) as u32, (h + 2.0 * shadow) as u32),
        margin: Insets { top: rect.y - shadow, right: 0.0, bottom: 0.0, left: rect.x - shadow },
        exclusive_zone: -1,
        frame: Rect::new(shadow, shadow, w, h),
        origin: Point::new(rect.x - shadow, rect.y - shadow),
        shape,
    }
}

impl Shell {
    /// Açılışta izlenecek düzen: ayardaki sabit düzen ya da compositor'ınki.
    fn osk_wanted_layout(&self) -> Layout {
        self.model.config.osk.layout.fixed().or(self.osk.compositor_layout).unwrap_or_default()
    }

    fn osk_publish(&mut self, cx: &mut Context<Self>) {
        let view = OskView {
            available: cx.virtual_keyboard_supported(),
            visible: self.osk.open.is_some(),
            layout: self.osk.osk.as_ref().map_or_else(|| self.osk_wanted_layout(), Osk::layout),
        };
        if self.model.osk != view {
            self.model.osk = view;
            self.model_changed(cx);
        }
    }

    /// Açılışta (kutucuk desteği görsün).
    pub(super) fn start_osk(&mut self, cx: &mut Context<Self>) {
        self.osk.config = Some(self.model.config.osk.clone());
        let hyprland = crate::services::setup::hypr_session(std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").as_deref());
        self.osk.grab_frame = GrabFrame::for_compositor(hyprland);
        self.osk.compositor_layout = cx.keyboard_layout().map(Layout::from_xkb_name);
        self.osk_publish(cx);
    }

    /// `msg osk …`.
    pub(super) fn osk_ipc(&mut self, cx: &mut Context<Self>, action: OskAction, output: Option<&str>) -> Result<String, String> {
        match action {
            OskAction::Status => {
                self.osk_publish(cx);
                let name = self.osk.open.as_ref().and_then(|o| self.screens.iter().find(|s| s.output == o.output)).map(|s| s.name.as_str());
                Ok(status_json(&self.model.osk, name, self.osk.placement))
            }
            OskAction::Run(c) => self.osk_action(cx, c, output).map(|()| "ok".into()),
        }
    }

    /// Kutucuk, kısayol ve `msg` ortak yolu.
    pub(super) fn osk_action(&mut self, cx: &mut Context<Self>, command: Command, output: Option<&str>) -> Result<(), String> {
        let show = match command {
            Command::Show => true,
            Command::Hide => false,
            Command::Toggle => self.osk.open.is_none(),
        };
        let result = if show {
            self.osk_show(cx, output)
        } else {
            self.osk_hide(cx);
            Ok(())
        };
        self.osk_publish(cx);
        result
    }

    /// Çıkışın klavye alanı (kabuk çerçevesinin iç dikdörtgeni, çıkış koordinatı) ve klavye boyutu.
    fn osk_geometry(&self, theme: &nuxut::Theme, output: OutputId) -> Option<(Rect, Size)> {
        let screen = self.screens.iter().find(|s| s.output == output)?;
        let size = screen.size();
        let area = if size.width > 0.0 { screen.inner_rect(&self.model.config) } else { Rect::new(0.0, 0.0, 1920.0, 1080.0) };
        let key_height = self.model.config.osk.key_height as f32;
        let kb = KeyboardView::preferred_size(theme, key_height, (area.width - 2.0 * SIDE_MARGIN).max(0.0));
        Some((area, kb))
    }

    fn osk_show(&mut self, cx: &mut Context<Self>, output: Option<&str>) -> Result<(), String> {
        if !cx.virtual_keyboard_supported() {
            return Err("compositor sanal klavye protokolünü (zwp_virtual_keyboard_v1) sunmuyor".into());
        }
        let idx = self.target_screen(output)?;
        let target = self.screens[idx].output;
        if let Some(open) = &self.osk.open {
            if open.output == target {
                return Ok(());
            }
            // Başka monitöre taşı: kapatıp orada aç (durum korunur, basılı tuş bırakılır).
            self.osk_hide(cx);
        }
        if self.osk.osk.is_none() {
            let osk = Osk::new(self.osk_wanted_layout()).map_err(|e| e.to_string())?;
            self.osk.osk = Some(osk);
        }
        let theme = cx.theme().clone();
        let mut view = KeyboardView::new(&theme, self.model.config.osk.key_height as f32);
        if let Some(osk) = &self.osk.osk {
            view.sync(osk, Instant::now());
        }
        let Some((surface, rect, plan)) = self.osk_create_surface(cx, &theme, target, &mut view) else {
            return Err("monitör yok".into());
        };
        self.osk.open = Some(Open {
            surface,
            output: target,
            view,
            rect,
            surface_origin: plan.origin,
            committed_origin: plan.origin,
            exclusive_zone: plan.exclusive_zone,
            drag: None,
        });
        self.osk_sync_focus_grabs(cx);
        tracing::info!(output = %self.screens[idx].name, placement = ?self.osk.placement, "ekran klavyesi açıldı");
        Ok(())
    }

    /// Klavye yüzeyini şimdiki yerleşimle kurar; (yüzey, klavye dikdörtgeni, yerleşim).
    fn osk_create_surface(
        &mut self,
        cx: &mut Context<Self>,
        theme: &nuxut::Theme,
        output: OutputId,
        view: &mut KeyboardView,
    ) -> Option<(SurfaceId, Rect, SurfacePlan)> {
        let (area, size) = self.osk_geometry(theme, output)?;
        let rect = place::rect_of(self.osk.placement, area, size);
        let p = plan(self.osk.placement, rect, shadow_margin(theme), self.model.config.osk.reserve_space);
        let spec = LayerSurfaceSpec {
            layer: Layer::Overlay,
            anchor: p.anchor,
            size: p.size,
            margin: p.margin,
            exclusive_zone: p.exclusive_zone,
            keyboard: KeyboardMode::None,
            ..LayerSurfaceSpec::new(NAMESPACE, output)
        };
        let surface = cx.create_layer_surface(spec);
        view.set_frame(theme, p.frame, p.shape);
        cx.set_input_region(surface, &[p.frame]);
        Some((surface, rect, p))
    }

    /// Açık klavye yüzeyini yerinde taşır/boyutlar (yerleşim bir sonraki karenin commit'iyle
    /// gider). Exclusive zone yalnızca `reserve_space` geçişlerinde değişir (ayrı commit).
    fn osk_move_surface(&mut self, cx: &mut Context<Self>, theme: &nuxut::Theme, placement: Placement, rect: Rect) {
        let reserve = self.model.config.osk.reserve_space;
        let Some(open) = &mut self.osk.open else { return };
        let p = plan(placement, rect, shadow_margin(theme), reserve);
        if p.exclusive_zone != open.exclusive_zone {
            cx.set_exclusive_zone(open.surface, p.exclusive_zone);
            open.exclusive_zone = p.exclusive_zone;
        }
        cx.set_layer_placement(open.surface, p.anchor, p.margin, Some(p.size));
        if open.view.frame() != p.frame {
            cx.set_input_region(open.surface, &[p.frame]);
        }
        open.view.set_frame(theme, p.frame, p.shape);
        open.rect = rect;
        open.surface_origin = p.origin;
        cx.request_frame(open.surface);
    }

    fn osk_hide(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.osk.open.take() else { return };
        if let Some(osk) = &mut self.osk.osk {
            let emits = osk.reset();
            apply(cx, &emits);
        }
        cx.virtual_keyboard_release();
        // Sürüklerken gizlenirse yerleşim kalıcı olmaz (önceki kip korunur).
        cx.destroy_surface(open.surface);
        self.osk_sync_focus_grabs(cx);
        tracing::info!("ekran klavyesi kapandı");
    }

    /// Klavye yüzeyi (varsa) yaprakların odak kapmasında: klavyeye dokunmak geçici yaprağı kapatmaz.
    pub(super) fn osk_surface(&self) -> Option<SurfaceId> {
        self.osk.open.as_ref().map(|o| o.surface)
    }

    fn osk_sync_focus_grabs(&mut self, cx: &mut Context<Self>) {
        let extra = self.osk_surface();
        for s in &mut self.screens {
            s.focus_grab_extra = extra;
            if let Some(id) = s.focus_grab {
                let mut surfaces = vec![s.main];
                surfaces.extend(extra);
                cx.focus_grab_set_surfaces(id, &surfaces);
            }
        }
    }

    pub(super) fn osk_owns(&self, surface: SurfaceId) -> bool {
        self.osk_surface() == Some(surface)
    }

    pub(super) fn osk_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, _size: Size) -> bool {
        if !self.osk_owns(surface) {
            return false;
        }
        cx.request_frame(surface);
        true
    }

    pub(super) fn osk_pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) -> bool {
        if !self.osk_owns(surface) {
            return false;
        }
        let now = Instant::now();
        let theme = cx.theme().clone();
        let grab_frame = self.osk.grab_frame;
        let (Some(open), Some(osk)) = (&mut self.osk.open, &mut self.osk.osk) else { return true };
        if open.drag.as_ref().is_some_and(|d| d.landing.is_some()) {
            return true; // yerine oturuyor: yeni basış beklesin
        }
        let (repaint, input) = open.view.pointer(&theme, &event, now);
        let cursor = if open.view.over_handle(&theme, event.position) { CursorShape::Grab } else { CursorShape::Default };
        cx.set_cursor(surface, cursor);
        let mut hide = false;
        let mut drag_event = None;
        if let Some(input) = input {
            let emits = match input {
                KeyInput::Press(KeyAction::Hide) => {
                    hide = true;
                    Vec::new()
                }
                KeyInput::Press(action) => osk.press(action),
                KeyInput::Release => osk.release(),
                KeyInput::DragStart(p) | KeyInput::DragMove(p) | KeyInput::DragEnd(p) => {
                    let press = open.drag.as_ref().map_or(open.committed_origin, |d| d.press_origin);
                    drag_event = Some((input, grab_frame.to_output(p, press, open.committed_origin)));
                    Vec::new()
                }
            };
            apply(cx, &emits);
            open.view.sync(osk, now);
        }
        if repaint {
            cx.request_frame(surface);
        }
        if let Some((input, global)) = drag_event {
            self.osk_drag(cx, input, global);
        }
        if hide {
            self.osk_hide(cx);
        }
        // Düzen tuşu ya da gizleme kutucuğa yansısın.
        self.osk_publish(cx);
        true
    }

    /// Tutamaç sürüklemesi (`global`: çıkış koordinatı). Yüzey her harekette taşınır.
    fn osk_drag(&mut self, cx: &mut Context<Self>, input: KeyInput, global: Point) {
        let theme = cx.theme().clone();
        let now = Instant::now();
        let Some(open) = &self.osk.open else { return };
        let Some((area, _)) = self.osk_geometry(&theme, open.output) else { return };
        if let (KeyInput::DragEnd(_), Some(d)) = (input, &open.drag)
            && let Some((to, landing)) = self.osk_drop_target(&theme, &d.drag, global, open.output)
        {
            // Başka monitöre bırakıldı: orada bir kez yeniden kurulur (yüzey çıkışa bağlı).
            let name = self.screens[to].name.clone();
            self.osk_hide(cx);
            self.osk.placement = landing;
            if let Err(e) = self.osk_show(cx, Some(&name)) {
                tracing::warn!("ekran klavyesi {name} çıkışına taşınamadı: {e}");
            }
            self.osk_publish(cx);
            return;
        }
        let placement = self.osk.placement;
        let Some(open) = &mut self.osk.open else { return };
        let moved = match input {
            KeyInput::DragStart(_) => {
                open.drag = Some(DragSession {
                    drag: Drag::begin(global, open.rect, placement),
                    press_origin: open.committed_origin,
                    rect: Animated::new(open.rect, theme.motion.spatial),
                    landing: None,
                });
                cx.request_frame(open.surface);
                None
            }
            KeyInput::DragMove(_) => open.drag.as_mut().map(|d| {
                let r = d.drag.update(global, area);
                d.rect.jump_to(r);
                (d.drag.is_floating(), r)
            }),
            KeyInput::DragEnd(_) => open.drag.as_mut().map(|d| {
                let r = d.drag.update(global, area);
                d.rect.jump_to(r);
                let landing = d.drag.finish(r, area);
                d.rect.set_target(place::rect_of(landing, area, Size::new(r.width, r.height)), now);
                d.landing = Some(landing);
                (d.drag.is_floating(), r)
            }),
            _ => None,
        };
        if let Some((floating, r)) = moved {
            let shown = if floating { Placement::Floating(Point::new(r.x, r.y)) } else { placement };
            self.osk_move_surface(cx, &theme, shown, r);
        }
    }

    /// Bırakma başka bir monitörün üstündeyse (o ekranın sırası, oradaki yerleşim).
    fn osk_drop_target(&self, theme: &nuxut::Theme, drag: &Drag, pointer: Point, from: OutputId) -> Option<(usize, Placement)> {
        let from = self.screens.iter().position(|s| s.output == from)?;
        let mut outputs = Vec::with_capacity(self.screens.len());
        for s in &self.screens {
            let (area, _) = self.osk_geometry(theme, s.output)?;
            outputs.push((Rect::new(s.position.x, s.position.y, s.size().width, s.size().height), area));
        }
        place::drop_target(drag, pointer, from, &outputs)
    }

    /// Bırakılan klavyenin yayı: her karede bir adım taşınır; oturunca yerleşim kalıcı olur.
    fn osk_land_step(&mut self, cx: &mut Context<Self>, theme: &nuxut::Theme, now: Instant) {
        let Some(d) = self.osk.open.as_mut().and_then(|o| o.drag.as_mut()) else { return };
        let Some(landing) = d.landing else { return };
        let r = d.rect.value(now);
        if d.rect.is_settled(now) {
            if let Some(open) = &mut self.osk.open {
                open.drag = None;
            }
            self.osk.placement = landing;
            self.osk_move_surface(cx, theme, landing, r);
            tracing::debug!(placement = ?landing, "ekran klavyesi yerleşti");
        } else {
            // Yay boyunca yüzen biçim (yanaşmaya inerken de); oturunca kipin biçimi.
            self.osk_move_surface(cx, theme, Placement::Floating(Point::new(r.x, r.y)), r);
        }
    }

    /// Ekran okuyucu ağacı (`KeyboardView::access`; tuşlar eylemsiz). Yüzey klavyeninse `true`.
    pub(super) fn osk_access(&self, surface: SurfaceId, tree: &mut nuxut::access::AccessTree) -> bool {
        if !self.osk_owns(surface) {
            return false;
        }
        tree.root().label(&nuxut::i18n::t!("shell-a11y-osk"));
        if let (Some(open), Some(osk)) = (&self.osk.open, &self.osk.osk) {
            open.view.access(tree, osk);
        }
        true
    }

    pub(super) fn draw_osk(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) -> bool {
        if !self.osk_owns(surface) {
            return false;
        }
        let theme = cx.theme().clone();
        let Some(open) = &mut self.osk.open else { return true };
        // Bekleyen yerleşim bu karenin commit'iyle gidiyor (framework onu `draw`'dan önce uygular).
        open.committed_origin = open.surface_origin;
        let mut ui = nuxut::ui::Ui::new(painter, &theme, now);
        open.view.draw(&mut ui);
        let wants_frame = ui.wants_frame();
        drop(ui);
        // Yayın sonraki adımı bir sonraki karede (bu kare şimdiki yerleşimle çizildi).
        self.osk_land_step(cx, &theme, now);
        if wants_frame {
            cx.request_frame(surface);
        }
        true
    }

    /// Çıkışın boyutu ya da kabuk alanı değişti: açık klavye yeni alana yerleşir.
    pub(super) fn osk_screen_resized(&mut self, cx: &mut Context<Self>, output: OutputId) {
        let theme = cx.theme().clone();
        let Some(open) = &self.osk.open else { return };
        if open.output != output || open.drag.is_some() {
            return;
        }
        let Some((area, size)) = self.osk_geometry(&theme, output) else { return };
        let rect = place::rect_of(self.osk.placement, area, size);
        if rect != open.rect {
            self.osk_move_surface(cx, &theme, self.osk.placement, rect);
        }
    }

    /// Kapanış (sinyal, `msg quit`): basılı tuş ve değiştiriciler istemcide takılı kalmasın.
    pub(super) fn osk_shutdown(&mut self, cx: &mut Context<Self>) {
        self.osk_hide(cx);
    }

    /// Tema değişti: tuş yazı stilleri ve ölçüler yenilenir (yüzey yeniden kurulur).
    pub(super) fn osk_theme_changed(&mut self, cx: &mut Context<Self>) {
        self.osk_reopen(cx);
    }

    /// `[osk]` değişti (yalnızca gerçekten değiştiyse; her config yeniden yüklemesinde değil).
    pub(super) fn osk_config_changed(&mut self, cx: &mut Context<Self>) {
        let config = self.model.config.osk.clone();
        if self.osk.config.as_ref() == Some(&config) {
            return;
        }
        let layout_changed = self.osk.config.as_ref().is_some_and(|c| c.layout != config.layout);
        self.osk.config = Some(config);
        let layout = self.osk_wanted_layout();
        if layout_changed && let Some(osk) = &mut self.osk.osk {
            let emits = osk.set_layout(layout);
            apply(cx, &emits);
        }
        self.osk_reopen(cx);
    }

    /// Açık klavye yeni tema/ölçüyle yeniden kurulur.
    fn osk_reopen(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.osk.open else {
            self.osk_publish(cx);
            return;
        };
        let name = self.screens.iter().find(|s| s.output == open.output).map(|s| s.name.clone());
        self.osk_hide(cx);
        if let Err(e) = self.osk_show(cx, name.as_deref()) {
            tracing::warn!("ekran klavyesi yeniden açılamadı: {e}");
        }
        self.osk_publish(cx);
    }

    pub(super) fn osk_output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        if self.osk.open.as_ref().is_some_and(|o| o.output == output) {
            self.osk_hide(cx);
            self.osk_publish(cx);
        }
    }

    /// Compositor'ın etkin düzeni değişti (wl_keyboard keymap/grup ya da Hyprland
    /// `activelayout`). `auto` iken klavye onu izler; elle seçilen düzen bir sonraki değişime
    /// kadar kalır.
    pub(super) fn osk_compositor_layout(&mut self, cx: &mut Context<Self>, name: &str) {
        let layout = Layout::from_xkb_name(name);
        if self.osk.compositor_layout == Some(layout) {
            return;
        }
        self.osk.compositor_layout = Some(layout);
        if self.model.config.osk.layout.fixed().is_some() {
            return;
        }
        if let Some(osk) = &mut self.osk.osk {
            let emits = osk.set_layout(layout);
            apply(cx, &emits);
            if let Some(open) = &mut self.osk.open {
                open.view.sync(osk, Instant::now());
                cx.request_frame(open.surface);
            }
        }
        self.osk_publish(cx);
    }
}

/// Durum makinesinin çağrılarını sanal klavyeye uygular.
fn apply(cx: &mut Context<Shell>, emits: &[Emit]) {
    for e in emits {
        let result = match *e {
            Emit::Keymap(layout) => cx.virtual_keyboard_set_keymap(layout.keymap()),
            Emit::Modifiers { depressed, locked } => cx.virtual_keyboard_modifiers(depressed, 0, locked, 0),
            Emit::Key { code, pressed } => cx.virtual_keyboard_key(code, pressed),
        };
        if let Err(err) = result {
            tracing::warn!(?e, "ekran klavyesi: sanal klavye çağrısı başarısız: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docked_and_floating_share_the_surface_so_dragging_only_moves_it() {
        let docked = Rect::new(342.0, 600.0, 916.0, 292.0);
        let p = plan(Placement::Docked, docked, 30.0, false);
        assert_eq!((p.anchor, p.exclusive_zone, p.size), (Anchor::TOP | Anchor::LEFT, -1, (976, 352)));
        assert_eq!((p.margin.left, p.margin.top), (312.0, 570.0));
        assert_eq!(p.frame, Rect::new(30.0, 30.0, 916.0, 292.0));
        assert_eq!(p.shape, Shape::Docked);
        // Aynı yerde yüzen: yalnızca biçim farklı (sürüklerken kip geçişinde boyut değişmez).
        let f = plan(Placement::Floating(Point::new(docked.x, docked.y)), docked, 30.0, false);
        assert_eq!(f, SurfacePlan { shape: Shape::Floating, ..p });
    }

    #[test]
    fn reserved_dock_keeps_the_edge_anchor_and_its_own_height() {
        let r = Rect::new(342.0, 600.0, 916.0, 292.0);
        let p = plan(Placement::Docked, r, 30.0, true);
        assert_eq!((p.anchor, p.exclusive_zone, p.size), (Anchor::BOTTOM, 292, (916, 292)));
        assert_eq!(p.frame, Rect::new(0.0, 0.0, 916.0, 292.0), "ayırırken gölge payı yok");
        // Yüzerken ayırma yok.
        assert_eq!(plan(Placement::Floating(Point::new(1.0, 1.0)), r, 30.0, true).exclusive_zone, -1);
    }

    #[test]
    fn floating_plan_positions_by_margin_with_shadow_room() {
        let r = Rect::new(200.0, 150.0, 916.0, 292.0);
        let p = plan(Placement::Floating(Point::new(200.0, 150.0)), r, 30.0, true);
        assert_eq!(p.anchor, Anchor::TOP | Anchor::LEFT);
        assert_eq!(p.exclusive_zone, -1, "yüzen klavye yer ayırmaz, diğer bölgeleri yok sayar");
        assert_eq!((p.margin.left, p.margin.top), (170.0, 120.0));
        assert_eq!(p.size, (976, 352));
        assert_eq!(p.frame, Rect::new(30.0, 30.0, 916.0, 292.0));
        assert_eq!(p.origin, Point::new(170.0, 120.0));
        // Yüzeydeki çerçeve + yüzeyin çıkıştaki yeri = klavyenin çıkıştaki yeri.
        assert_eq!((p.origin.x + p.frame.x, p.origin.y + p.frame.y), (r.x, r.y));
    }
}
