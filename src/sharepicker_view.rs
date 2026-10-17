//! Ekran paylaşım seçicisi (`nuxut-sharepicker`): xdph'in `screencopy:custom_picker_binary`
//! olarak başlattığı kısa ömürlü süreç. Tasarım ve sözleşme: `docs/design/screenshare-picker.md`;
//! saf sözleşme katmanı `services::sharepicker`.
//!
//! - **Seçici kartı:** odaklı çıkışta layer-shell `Overlay` yüzeyi (`KeyboardMode::Exclusive`),
//!   framework [`Dialog`] kartı. Sekmeler (Ekran / Pencere / Bölge) [`SegmentedButton`]; ekran ve
//!   pencere listeleri [`List`]; "İzni hatırla" anahtarlı [`ListItem`].
//! - **Bölge:** "Paylaş" kart yüzeyini kapatır, her çıkışa bir seçim katmanı açar. Durum makinesi
//!   ekran görüntüsünün bölge seçimiyle ortaktır (`services::screenshot::selection`); kare
//!   dondurulmaz, canlı ekran karartılarak çizilir. Esc ya da sağ tık karta geri döner.
//! - **Klavye:** Esc iptal, Enter paylaş, Sol/Sağ sekme, Yukarı/Aşağı liste (odak = seçim), Tab
//!   liste ile "İzni hatırla" arasında, Space anahtarı çevirir. Kart ilk sunulduktan sonra kısa
//!   süre tuşlar yok sayılır (yazı-önü: başka pencerede basılan Enter ekranı paylaşmasın).
//! - Sonuç [`PickerSetup::result`]'a yazılır; `run` dönünce ikili stdout'a tek satır basar.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nuxut::i18n::t;
use nuxut::ui::{
    Dialog, DialogAction, DialogActionKind, DialogEvent, Icon, IconName, IconView, Label, List, ListEvent, ListItem,
    ListTrailing, Segment, SegmentedButton, TextRole, Ui, app_glyph, text_style,
};
use nuxut::render::{ImageData, ImageFit, ImageStyle, TextAlign, Texture, TextureOptions};
use nuxut_image::{AppIconOptions, AppIconTextures, AppIcons};
use nuxut::wayland::{Anchor, KeyboardMode, Layer, LayerSurfaceSpec};
use nuxut::{
    App, Border, Button, CaptureError, CaptureId, CaptureOptions, CaptureSource, CapturedImage, Color, Context, Corners,
    CursorShape, Key, KeyEvent, KeyState, OutputId, OutputInfo, Painter, Point, PointerEvent, PointerKind, Rect, RectStyle,
    Size, SurfaceId, Theme, TimerId, ToplevelEvent,
};

use crate::screenshot_view::{cursor_shape, outside, sel_key};
use crate::services::screenshot::selection::{Outcome, SelKey, Selection};
use crate::services::idle::{IdleLogind, LogindEvent, LogindOptions};
use crate::services::sharepicker::{Choice, LOCK_DEADLINE, LockGate, Region, WindowList, display_text, valid_output_name};

const NAMESPACE: &str = "nuxut-sharepicker";
/// İlk sunumdan sonra tuşların yok sayıldığı süre (polkit diyaloğuyla aynı).
const INPUT_DELAY: Duration = Duration::from_millis(400);
/// Etkileşimsiz bu süre sonunda iptal: xdph seçiciyi beklerken ekran paylaşımı isteği asılı
/// kalmasın (her girdi süreyi yeniler).
const IDLE_TIMEOUT: Duration = Duration::from_secs(180);
/// Bölge katmanında seçim dışının karartması (ekran görüntüsü katmanıyla aynı).
const DIM_ALPHA: f32 = 0.45;
const BORDER_WIDTH: f32 = 2.0;
const HANDLE_SIZE: f32 = 8.0;
const LABEL_GAP: f32 = 8.0;
const LABEL_PAD: (f32, f32) = (8.0, 4.0);
/// Gösterilen çıkış açıklamasının en uzun hâli (karakter).
const MAX_DESCRIPTION_CHARS: usize = 64;
/// Önizlemesi istenen en çok pencere (her biri bir yakalama; runtime aynı anda 2 iş yürütür).
const MAX_PREVIEWS: usize = 32;
/// Pencere sekmesinde önizleme bölmesinin alan genişliğindeki payı.
const PREVIEW_SHARE: f32 = 0.42;

/// Bir pencerenin önizleme durumu (Hyprland adresiyle anahtarlı). Pikseller yalnızca bellekte,
/// süreç kısa ömürlü; loglanmaz (framework capture.md, değişmez 9).
enum Preview {
    Pending(CaptureId),
    Ready(ImageData),
    Uploaded(Texture),
    /// Yakalanamadı (gizli çalışma alanı, `noscreenshare`, zaman aşımı…): genel ikon.
    Failed,
}

/// Pencere önizlemeleri: pencere listesi (ext-foreign-toplevel-list) tembel başlar, xdph'in
/// verdiği tam adres `toplevel_by_hyprland_address` ile tutamaca çevrilir, her pencere bir kez
/// yakalanır.
#[derive(Default)]
struct Previews {
    /// Başlatma denendi (bir kez).
    started: bool,
    /// Pencere listesi çalışıyor: önizleme bölmesi gösterilir.
    active: bool,
    map: HashMap<u64, Preview>,
}

impl Previews {
    /// Önizlemesi henüz istenmemiş adresler (sıra korunur, en çok [`MAX_PREVIEWS`] istek).
    fn wanted(&self, windows: &WindowList) -> Vec<u64> {
        let room = MAX_PREVIEWS.saturating_sub(self.map.len());
        let mut out: Vec<u64> = Vec::new();
        for w in &windows.windows {
            if w.address != 0 && !self.map.contains_key(&w.address) && !out.contains(&w.address) {
                out.push(w.address);
            }
        }
        out.truncate(room);
        out
    }

    /// Yakalama sonucu; bilinmeyen (iptal edilmiş) kimlik `false`.
    fn finish(&mut self, id: CaptureId, image: Option<ImageData>) -> bool {
        let Some(slot) = self.map.values_mut().find(|s| matches!(s, Preview::Pending(c) if *c == id)) else {
            return false;
        };
        *slot = image.map_or(Preview::Failed, Preview::Ready);
        true
    }
}

/// Uygulama iletileri (arka plan thread'lerinden).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMessage {
    /// Uygulama ikonu çözüldü.
    Icons,
    /// logind kilit durumu.
    Lock(LockGate),
}

/// Başlatma parametreleri.
#[derive(Debug)]
pub struct PickerSetup {
    pub windows: WindowList,
    /// Ortamda pencere listesi vardı (yoksa xdph pencere paylaşımını sunmuyor).
    pub windows_available: bool,
    /// "İzni hatırla" başlangıç değeri (`--allow-token`).
    pub allow_token: bool,
    /// Sonuç: seçim ve "izni hatırla". `None` = iptal.
    pub result: Arc<Mutex<Option<(Choice, bool)>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Screen,
    Window,
    Region,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::Screen, Tab::Window, Tab::Region];

    fn index(self) -> usize {
        match self {
            Tab::Screen => 0,
            Tab::Window => 1,
            Tab::Region => 2,
        }
    }

    fn from_index(i: usize) -> Self {
        Self::ALL.get(i).copied().unwrap_or(Tab::Screen)
    }
}

/// Klavye odağı (Tab ile değişir).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Remember,
}

/// Kartın gövdesi (tema/dil değişince yeniden kurulur).
struct Body {
    tabs: SegmentedButton,
    screens: List,
    /// `screens` satırlarının çıkış adları (aynı sıra).
    screen_names: Vec<String>,
    windows: List,
    window_ids: Vec<u32>,
    /// `windows` satırlarının sınıfı ve Hyprland adresi (aynı sıra).
    window_classes: Vec<String>,
    window_addresses: Vec<u64>,
    /// Satırın ön ikonu gerçek uygulama ikonuna çevrildi mi (yoksa glif/genel ikon).
    window_icon_ready: Vec<bool>,
    /// Önizleme bölmesinin yedek ikonu (önizleme yokken).
    preview_icon: IconView,
    remember: ListItem,
    region_icon: IconView,
    region_text: Label,
    empty: Label,
    status: Label,
    area_h: f32,
}

/// Bir çıkışın bölge seçim katmanı.
#[derive(Debug)]
struct RegionView {
    surface: SurfaceId,
    output: OutputId,
    name: String,
    selection: Selection,
    size: Size,
    scale: f32,
}

pub struct SharePicker {
    setup: PickerSetup,
    outputs: Vec<OutputInfo>,
    started: bool,
    surface: Option<SurfaceId>,
    size: Size,
    body: Option<Body>,
    dialog: Option<Dialog>,
    tab: Tab,
    focus: Focus,
    selected_screen: Option<String>,
    selected_window: Option<u32>,
    remember: bool,
    /// Durum satırında "önce seç" uyarısı.
    show_hint: bool,
    regions: Vec<RegionView>,
    shown_at: Option<Instant>,
    idle_timer: Option<TimerId>,
    done: bool,
    /// Kartın yüzey ölçeği (ikon ve önizleme piksel boyutu).
    scale: f32,
    /// Uygulama ikonu çözücüsü (ilk kurulumda; bitince `message` yeniden çizim ister).
    icons: Option<AppIcons>,
    icon_textures: AppIconTextures,
    previews: Previews,
    /// Oturum kilidi kapısı (logind `LockedHint`): kilitliyken istek reddedilir, önizleme yok.
    lock: LockGate,
    /// logind izleyicisi (yalnız okur; ilk çıkışta başlar, seçici kapanınca durur).
    logind: Option<IdleLogind>,
    /// İlk logind yanıtı için süre (dolarsa bilinmiyor = kilitli, istek reddedilir).
    lock_deadline: Option<TimerId>,
}

impl std::fmt::Debug for SharePicker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharePicker")
            .field("tab", &self.tab)
            .field("outputs", &self.outputs.len())
            .field("regions", &self.regions.len())
            .finish_non_exhaustive()
    }
}

impl SharePicker {
    pub fn new(setup: PickerSetup) -> Self {
        let remember = setup.allow_token;
        Self {
            setup,
            outputs: Vec::new(),
            started: false,
            surface: None,
            size: Size::ZERO,
            body: None,
            dialog: None,
            tab: Tab::Screen,
            focus: Focus::List,
            selected_screen: None,
            selected_window: None,
            remember,
            show_hint: false,
            regions: Vec::new(),
            shown_at: None,
            idle_timer: None,
            done: false,
            scale: 1.0,
            icons: None,
            icon_textures: AppIconTextures::new(),
            previews: Previews::default(),
            lock: LockGate::Unknown,
            logind: None,
            lock_deadline: None,
        }
    }

    fn finish(&mut self, cx: &mut Context<Self>, mut choice: Option<Choice>) {
        if self.done {
            return;
        }
        if choice.is_some() && !self.lock.share_allowed() {
            if !self.lock.must_reject() {
                // İlk logind yanıtı bekleniyor (en çok çağrı zaman aşımı kadar): seçim sürer.
                tracing::info!("kilit durumu henüz bilinmiyor; seçim bekletildi");
                return;
            }
            choice = None;
        }
        self.done = true;
        self.logind = None;
        if let Some(choice) = choice
            && let Ok(mut r) = self.setup.result.lock()
        {
            *r = Some((choice, self.remember));
        }
        if self.previews.active {
            cx.toplevels_stop();
        }
        cx.quit();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        tracing::info!("seçici iptal edildi");
        self.finish(cx, None);
    }

    fn reset_idle(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.idle_timer.take() {
            cx.cancel_timer(t);
        }
        self.idle_timer = Some(cx.set_timer(IDLE_TIMEOUT));
    }

    fn redraw(&self, cx: &mut Context<Self>) {
        if let Some(s) = self.surface {
            cx.request_frame(s);
        }
    }

    fn input_allowed(&self) -> bool {
        self.shown_at.is_some_and(|t| t.elapsed() >= INPUT_DELAY)
    }

    fn open_picker_surface(&mut self, cx: &mut Context<Self>) {
        let mut spec = LayerSurfaceSpec::on_focused_output(NAMESPACE);
        spec.layer = Layer::Overlay;
        spec.anchor = Anchor::ALL;
        spec.exclusive_zone = -1;
        spec.keyboard = KeyboardMode::Exclusive;
        self.surface = Some(cx.create_layer_surface(spec));
        self.size = Size::ZERO;
        self.body = None;
        self.dialog = None;
    }

    // --- Kart ----------------------------------------------------------------------------

    fn build(&mut self, cx: &mut Context<Self>) {
        if self.body.is_some() || self.size.width <= 0.0 {
            return;
        }
        let theme = cx.theme().clone();
        let now = Instant::now();
        let width = card_width(&theme, self.size);
        if self.icons.is_none() {
            let tx = cx.sender();
            let sandbox = crate::services::image_sandbox::decoder(&crate::services::image_sandbox::new_shared());
            let options = AppIconOptions { sandbox, theme: theme.desktop.icon_theme.clone(), dirs: None };
            self.icons = Some(AppIcons::new(options, move || {
                let _ = tx.send(PickerMessage::Icons);
            }));
        }
        let segments = vec![
            Segment::new(t!("shell-sharepicker-tab-screen")).with_icon(Icon::named(IconName::Monitor)),
            Segment::new(t!("shell-sharepicker-tab-window")).with_icon(Icon::named(IconName::Window)),
            Segment::new(t!("shell-sharepicker-tab-region")).with_icon(Icon::named(IconName::Fullscreen)),
        ];
        let mut tabs = SegmentedButton::new(&theme, segments, self.tab.index());
        let tabs_h = tabs.preferred_size(cx.text(), &theme).height;
        let mut remember = ListItem::new(&theme, t!("shell-sharepicker-remember"))
            .with_subtitle(&theme, t!("shell-sharepicker-remember-detail"))
            .with_trailing(&theme, ListTrailing::Toggle(self.remember));
        remember.set_focus_visible(self.focus == Focus::Remember);
        let remember_h = remember.height(&theme);
        let centered = text_style(&theme, TextRole::Body).with_align(TextAlign::Center);
        let mut region_text = Label::new(t!("shell-sharepicker-region-body"), centered.clone());
        region_text.set_max_width(Some(width - theme.space(8)));
        let mut empty = Label::new(self.empty_text(), centered);
        empty.set_max_width(Some(width - theme.space(8)));
        let mut status = Label::new("", text_style(&theme, TextRole::Caption));
        status.set_max_width(Some(width));
        let line_h = status_line_height(cx, &theme);
        let mut region_icon = IconView::with_role(&theme, Icon::named(IconName::Fullscreen), TextRole::IconLarge);
        region_icon.measure(cx.text());
        let mut preview_icon = IconView::with_role(&theme, Icon::named(IconName::Window), TextRole::IconLarge);
        preview_icon.measure(cx.text());
        let area_h = theme.space(64).min((self.size.height - theme.space(80)).max(theme.space(24)));
        let mut body = Body {
            tabs,
            screens: List::new(&theme),
            screen_names: Vec::new(),
            windows: List::new(&theme),
            window_ids: Vec::new(),
            window_classes: Vec::new(),
            window_addresses: Vec::new(),
            window_icon_ready: Vec::new(),
            preview_icon,
            remember,
            region_icon,
            region_text,
            empty,
            status,
            area_h,
        };
        self.fill_screens(&theme, &mut body);
        self.fill_windows(&theme, &mut body);
        self.sync_status(&mut body);
        let height = tabs_h + theme.space(4) + area_h + theme.space(3) + remember_h + theme.space(2) + line_h;
        let mut dialog = Dialog::new(&theme);
        dialog.set_dismissible(false);
        let bounds = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        dialog.open(cx.text(), &theme, &t!("shell-sharepicker-title"), actions(), Size::new(width, height), bounds, now);
        self.dialog = Some(dialog);
        self.body = Some(body);
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.body = None;
        self.dialog = None;
        self.build(cx);
        self.redraw(cx);
    }

    fn empty_text(&self) -> String {
        if self.setup.windows_available {
            t!("shell-sharepicker-no-windows")
        } else {
            t!("shell-sharepicker-windows-unavailable")
        }
    }

    fn fill_screens(&self, theme: &Theme, body: &mut Body) {
        body.screen_names.clear();
        let mut items = Vec::new();
        for o in &self.outputs {
            let mut item = ListItem::new(theme, o.name.clone())
                .with_subtitle(theme, screen_detail(o))
                .with_leading(theme, Icon::named(IconName::Monitor));
            item.set_selected(self.selected_screen.as_deref() == Some(o.name.as_str()));
            items.push(item);
            body.screen_names.push(o.name.clone());
        }
        *body.screens.items_mut() = items;
    }

    fn fill_windows(&self, theme: &Theme, body: &mut Body) {
        body.window_ids.clear();
        body.window_classes.clear();
        body.window_addresses.clear();
        body.window_icon_ready.clear();
        let mut items = Vec::new();
        for w in &self.setup.windows.windows {
            let title = if w.title.is_empty() { t!("shell-sharepicker-window-untitled") } else { w.title.clone() };
            let icon = match app_glyph(&w.class) {
                Some(g) => Icon::glyph(&g.to_string()),
                None => Icon::named(IconName::Window),
            };
            let mut item = ListItem::new(theme, title).with_leading(theme, icon);
            if !w.class.is_empty() {
                item = item.with_subtitle(theme, w.class.clone());
            }
            item.set_selected(self.selected_window == Some(w.id));
            items.push(item);
            body.window_ids.push(w.id);
            body.window_classes.push(w.class.clone());
            body.window_addresses.push(w.address);
            body.window_icon_ready.push(false);
        }
        *body.windows.items_mut() = items;
    }

    fn sync_status(&self, body: &mut Body) {
        let text = if self.show_hint { t!("shell-sharepicker-pick-first") } else { String::new() };
        body.status.set_text(&text);
    }

    fn active_list<'b>(&self, body: &'b mut Body) -> Option<&'b mut List> {
        match self.tab {
            Tab::Screen => Some(&mut body.screens),
            Tab::Window if !body.window_ids.is_empty() => Some(&mut body.windows),
            _ => None,
        }
    }

    fn set_tab(&mut self, cx: &mut Context<Self>, tab: Tab) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        self.show_hint = false;
        if tab == Tab::Window {
            self.start_previews(cx);
        }
        let now = Instant::now();
        let mut body = self.body.take();
        if let Some(b) = body.as_mut() {
            b.tabs.set_selected(tab.index(), now);
            b.screens.blur(now);
            b.windows.blur(now);
            self.sync_status(b);
        }
        self.body = body;
        self.redraw(cx);
    }

    /// `index`. satırı seçer (etkin sekmenin listesinde).
    fn select_row(&mut self, index: usize) {
        let Some(mut b) = self.body.take() else { return };
        match self.tab {
            Tab::Screen => {
                if let Some(name) = b.screen_names.get(index) {
                    self.selected_screen = Some(name.clone());
                }
                for (i, item) in b.screens.items_mut().iter_mut().enumerate() {
                    item.set_selected(i == index);
                }
            }
            Tab::Window => {
                if let Some(id) = b.window_ids.get(index) {
                    self.selected_window = Some(*id);
                }
                for (i, item) in b.windows.items_mut().iter_mut().enumerate() {
                    item.set_selected(i == index);
                }
            }
            Tab::Region => {}
        }
        self.show_hint = false;
        self.sync_status(&mut b);
        self.body = Some(b);
    }

    fn set_remember(&mut self, on: bool) {
        self.remember = on;
        if let Some(b) = self.body.as_mut() {
            b.remember.set_toggle(on, Instant::now());
        }
    }

    /// "Paylaş": seçimi bitirir ya da bölge kipine geçer. Seçim yoksa kart açık kalır.
    fn share(&mut self, cx: &mut Context<Self>) {
        let choice = match self.tab {
            Tab::Screen => self.selected_screen.clone().map(Choice::Output),
            Tab::Window => self.selected_window.map(Choice::Window),
            Tab::Region => {
                self.enter_region(cx);
                return;
            }
        };
        match choice {
            Some(c) => {
                tracing::info!(kind = choice_kind(&c), "seçim yapıldı");
                self.finish(cx, Some(c));
            }
            None => {
                self.show_hint = true;
                if let Some(mut b) = self.body.take() {
                    self.sync_status(&mut b);
                    self.body = Some(b);
                }
                self.reopen_dialog(cx);
            }
        }
    }

    /// `Dialog` eylemde kendini kapatır; seçim yoksa kart aynı karede yeniden açılır.
    fn reopen_dialog(&mut self, cx: &mut Context<Self>) {
        let theme = cx.theme().clone();
        let bounds = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        if let Some(d) = self.dialog.as_mut() {
            let content = d.content_rect(&theme).size();
            d.open(cx.text(), &theme, &t!("shell-sharepicker-title"), actions(), content, bounds, Instant::now());
        }
        self.redraw(cx);
    }

    fn picker_key(&mut self, cx: &mut Context<Self>, event: &KeyEvent) {
        if event.state == KeyState::Released {
            return;
        }
        if !self.input_allowed() {
            tracing::debug!("yazı-önü kapısı: tuş yok sayıldı");
            return;
        }
        self.reset_idle(cx);
        let now = Instant::now();
        let theme = cx.theme().clone();
        match event.key {
            Key::Escape => return self.cancel(cx),
            Key::Enter => return self.share(cx),
            Key::Left | Key::Right if !event.modifiers.ctrl && !event.modifiers.alt => {
                let i = self.tab.index();
                let next = if event.key == Key::Left { i.checked_sub(1) } else { (i + 1 < Tab::ALL.len()).then_some(i + 1) };
                if let Some(n) = next {
                    self.set_tab(cx, Tab::from_index(n));
                }
                return;
            }
            Key::Tab => {
                self.focus = match self.focus {
                    Focus::List => Focus::Remember,
                    Focus::Remember => Focus::List,
                };
                let focus = self.focus;
                if let Some(b) = self.body.as_mut() {
                    b.remember.set_focus_visible(focus == Focus::Remember);
                    if focus == Focus::Remember {
                        b.screens.blur(now);
                        b.windows.blur(now);
                    }
                }
                self.redraw(cx);
                return;
            }
            _ => {}
        }
        let Some(mut b) = self.body.take() else { return };
        if self.focus == Focus::Remember {
            let r = b.remember.key(event, now);
            self.body = Some(b);
            if let Some(ListEvent::Toggled(on)) = r.event {
                self.set_remember(on);
            }
            if r.repaint || r.event.is_some() {
                self.redraw(cx);
            }
            return;
        }
        let mut select = None;
        let mut repaint = false;
        if let Some(list) = self.active_list(&mut b) {
            let before = list.focused();
            let r = list.key(&theme, event, now);
            repaint = r.repaint || r.event.is_some();
            select = match r.event {
                Some((i, ListEvent::Clicked)) => Some(i),
                _ => list.focused().filter(|f| Some(*f) != before),
            };
        }
        self.body = Some(b);
        if let Some(i) = select {
            self.select_row(i);
            repaint = true;
        }
        if repaint {
            self.redraw(cx);
        }
    }

    fn picker_pointer(&mut self, cx: &mut Context<Self>, event: &PointerEvent) {
        let now = Instant::now();
        if matches!(event.kind, PointerKind::Press(_)) {
            self.reset_idle(cx);
        }
        let theme = cx.theme().clone();
        let Some(mut b) = self.body.take() else { return };
        let mut repaint = false;
        let mut tab = None;
        let mut select = None;
        let mut toggled = None;
        let r = b.tabs.pointer(event, now);
        repaint |= r.repaint;
        if let Some(i) = r.event {
            tab = Some(Tab::from_index(i));
        }
        if let Some(list) = self.active_list(&mut b) {
            let r = list.pointer(&theme, event, now);
            repaint |= r.repaint;
            if let Some((i, ListEvent::Clicked)) = r.event {
                select = Some(i);
            }
        }
        let r = b.remember.pointer(event, now);
        repaint |= r.repaint;
        if let Some(ListEvent::Toggled(on)) = r.event {
            toggled = Some(on);
        }
        let cursor = b
            .tabs
            .cursor(event.position)
            .or_else(|| self.active_list(&mut b).and_then(|l| l.cursor(event.position)))
            .or_else(|| b.remember.cursor(event.position));
        self.body = Some(b);
        if let Some(on) = toggled {
            self.set_remember(on);
        }
        if let Some(t) = tab {
            self.set_tab(cx, t);
        }
        if let Some(i) = select {
            self.select_row(i);
            repaint = true;
        }
        let mut action = None;
        if let Some(d) = self.dialog.as_mut() {
            let r = d.pointer(event, now);
            repaint |= r.repaint;
            if let Some(DialogEvent::Action(i)) = r.event {
                action = Some(i);
            }
            if let Some(s) = self.surface {
                cx.set_cursor(s, cursor.or_else(|| d.cursor(event.position)).unwrap_or(CursorShape::Default));
            }
        }
        match action {
            Some(1) => self.share(cx),
            Some(_) => self.cancel(cx),
            None if repaint => self.redraw(cx),
            None => {}
        }
    }

    fn draw_picker(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        let theme = cx.theme();
        let tab = self.tab;
        let selected = self.selected_window;
        let scale = self.scale.max(1.0).ceil() as u32;
        let (icons, textures, previews) = (self.icons.as_ref(), &mut self.icon_textures, &mut self.previews);
        let (Some(dialog), Some(b)) = (self.dialog.as_mut(), self.body.as_mut()) else {
            return;
        };
        // Doku yüklemesi süren ikon/önizleme: bir kare daha.
        let mut pending = false;
        let mut ui = Ui::new(painter, theme, now);
        let c = &theme.colors;
        dialog.draw(&mut ui, |ui, rect| {
            let mut y = rect.y;
            let tabs_h = b.tabs.preferred_size(ui.painter.text_system(), theme).height;
            b.tabs.set_rect(theme, Rect::new(rect.x, y, rect.width, tabs_h));
            b.tabs.draw(ui);
            y += tabs_h + theme.space(4);
            let area = Rect::new(rect.x, y, rect.width, b.area_h);
            match tab {
                Tab::Screen => {
                    b.screens.set_rect(theme, area, now);
                    b.screens.draw(ui);
                }
                Tab::Window if !b.window_ids.is_empty() => {
                    if let Some(icons) = icons {
                        pending |= upgrade_window_icons(ui.painter, theme, icons, textures, b, scale);
                    }
                    // Önizleme bölmesi sağda (pencere listesi çalışıyorsa); seçili pencerenin karesi.
                    let pane_w = if previews.active { (area.width * PREVIEW_SHARE).floor() } else { 0.0 };
                    let gap = if pane_w > 0.0 { theme.space(3) } else { 0.0 };
                    let list = Rect::new(area.x, area.y, area.width - pane_w - gap, area.height);
                    b.windows.set_rect(theme, list, now);
                    b.windows.draw(ui);
                    if pane_w > 0.0 {
                        let pane = Rect::new(list.x + list.width + gap, area.y, pane_w, area.height);
                        let address = selected
                            .and_then(|id| b.window_ids.iter().position(|w| *w == id))
                            .and_then(|i| b.window_addresses.get(i).copied());
                        let preview = address.and_then(|a| previews.map.get_mut(&a));
                        pending |= draw_preview(ui, theme, pane, preview, &mut b.preview_icon);
                    }
                }
                Tab::Window => {
                    let s = b.empty.measure_in(ui.painter);
                    let p = Point::new(area.x + (area.width - s.width) * 0.5, area.y + (area.height - s.height) * 0.5);
                    b.empty.draw(ui.painter, p, ui.fade(c.on_surface_variant));
                }
                Tab::Region => {
                    let icon = b.region_icon.size();
                    let s = b.region_text.measure_in(ui.painter);
                    let total = icon.height + theme.space(3) + s.height;
                    let top = area.y + (area.height - total) * 0.5;
                    b.region_icon.draw_centered(
                        ui.painter,
                        Point::new(area.x + area.width * 0.5, top + icon.height * 0.5),
                        ui.fade(c.primary),
                    );
                    let p = Point::new(area.x + (area.width - s.width) * 0.5, top + icon.height + theme.space(3));
                    b.region_text.draw(ui.painter, p, ui.fade(c.on_surface_variant));
                }
            }
            y += b.area_h + theme.space(3);
            let h = b.remember.height(theme);
            b.remember.set_rect(theme, Rect::new(rect.x, y, rect.width, h));
            b.remember.draw(ui);
            y += h + theme.space(2);
            b.status.draw(ui.painter, Point::new(rect.x, y), ui.fade(c.error));
        });
        if ui.wants_frame() || pending {
            cx.request_frame(surface);
        }
    }

    // --- Bölge ---------------------------------------------------------------------------

    fn enter_region(&mut self, cx: &mut Context<Self>) {
        let focused = self.surface.and_then(|s| cx.surface_output(s));
        let outputs: Vec<OutputInfo> = self.outputs.clone();
        if outputs.is_empty() {
            self.reopen_dialog(cx);
            return;
        }
        if let Some(s) = self.surface.take() {
            cx.destroy_surface(s);
        }
        self.body = None;
        self.dialog = None;
        let keyboard_output = focused.filter(|f| outputs.iter().any(|o| o.id == *f)).unwrap_or(outputs[0].id);
        for o in outputs {
            let keyboard = if o.id == keyboard_output { KeyboardMode::Exclusive } else { KeyboardMode::None };
            let spec = LayerSurfaceSpec {
                layer: Layer::Overlay,
                anchor: Anchor::ALL,
                exclusive_zone: -1,
                keyboard,
                ..LayerSurfaceSpec::new(NAMESPACE, o.id)
            };
            let surface = cx.create_layer_surface(spec);
            cx.set_cursor(surface, CursorShape::Crosshair);
            self.regions.push(RegionView {
                surface,
                output: o.id,
                name: o.name.clone(),
                selection: Selection::new((o.size.width, o.size.height)),
                size: o.size,
                scale: o.scale,
            });
        }
        tracing::info!(outputs = self.regions.len(), "bölge seçimi açıldı");
    }

    fn leave_region(&mut self, cx: &mut Context<Self>) {
        for v in self.regions.drain(..) {
            cx.destroy_surface(v.surface);
        }
        self.open_picker_surface(cx);
    }

    fn region_outcome(&mut self, cx: &mut Context<Self>, index: usize, outcome: Outcome) {
        match outcome {
            Outcome::None => {}
            Outcome::Redraw => {
                if let Some(v) = self.regions.get(index) {
                    cx.request_frame(v.surface);
                }
            }
            Outcome::Cancel => self.leave_region(cx),
            Outcome::Confirm(rect) => {
                let Some(v) = self.regions.get(index) else { return };
                let bounds = (v.size.width, v.size.height);
                match Region::from_logical(&v.name, rect.tuple(), bounds) {
                    Some(r) => {
                        tracing::info!(kind = "region", "seçim yapıldı");
                        self.finish(cx, Some(Choice::Region(r)));
                    }
                    None => tracing::warn!("bölge sözleşmeye yazılamadı; seçim sürüyor"),
                }
            }
        }
    }

    fn region_pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: &PointerEvent) {
        let Some(i) = self.regions.iter().position(|v| v.surface == surface) else { return };
        if matches!(event.kind, PointerKind::Press(_)) {
            self.reset_idle(cx);
        }
        let p = (event.position.x, event.position.y);
        if matches!(event.kind, PointerKind::Press(Button::Left)) {
            // Başka çıkışta seçim başladı: diğerlerininki kalkar.
            for (j, v) in self.regions.iter_mut().enumerate() {
                if j != i && v.selection.clear() {
                    cx.request_frame(v.surface);
                }
            }
        }
        let v = &mut self.regions[i];
        let outcome = match event.kind {
            PointerKind::Enter | PointerKind::Motion => v.selection.motion(p),
            PointerKind::Leave => v.selection.leave(),
            PointerKind::Press(Button::Left) => v.selection.press(p),
            PointerKind::Release(Button::Left) => v.selection.release(p, &[]),
            PointerKind::Press(Button::Right) => Outcome::Cancel,
            _ => Outcome::None,
        };
        let shape = cursor_shape(v.selection.cursor(p));
        cx.set_cursor(surface, shape);
        self.region_outcome(cx, i, outcome);
    }

    fn region_key(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: &KeyEvent) {
        let Some(own) = self.regions.iter().position(|v| v.surface == surface) else { return };
        let Some(key) = sel_key(event) else { return };
        self.reset_idle(cx);
        // Enter seçimi olan çıkışta onaylar (klavye başka çıkışta olabilir); yoksa klavyenin
        // olduğu çıkışın tamamı.
        let target = match key {
            SelKey::Confirm => self.regions.iter().position(|v| v.selection.rect().is_some()).unwrap_or(own),
            _ => own,
        };
        let shift = event.modifiers.shift;
        let outcome = self.regions[target].selection.key(key, shift, &[]);
        self.region_outcome(cx, target, outcome);
    }

    fn draw_region(&mut self, cx: &mut Context<Self>, surface: SurfaceId, painter: &mut Painter) {
        let Some(v) = self.regions.iter().find(|v| v.surface == surface) else { return };
        let theme = cx.theme();
        let full = Rect::new(0.0, 0.0, v.size.width, v.size.height);
        painter.damage_all();
        let c = &theme.colors;
        let dim = c.shadow.with_alpha(DIM_ALPHA);
        match v.selection.rect() {
            Some(r) => {
                let r = Rect::new(r.x, r.y, r.w, r.h);
                for d in outside(full, r) {
                    painter.rect(d, &RectStyle::filled(dim, Corners::ZERO));
                }
                let border = Border { width: BORDER_WIDTH, color: c.primary };
                painter.rect(r, &RectStyle { fill: Color::TRANSPARENT, radii: Corners::ZERO, border: Some(border) });
                for h in handle_rects(r) {
                    painter.rect(h, &RectStyle::filled(c.primary, Corners::ZERO));
                }
                // Etikette yaklaşık fiziksel boyut (mantıksal × ölçek).
                let s = v.scale.max(1.0);
                let label = t!(
                    "shell-screenshot-size",
                    width = ((r.width * s).round() as u32).to_string(),
                    height = ((r.height * s).round() as u32).to_string()
                );
                let below = r.y + r.height + LABEL_GAP;
                let at = if below + 32.0 < full.height { Point::new(r.x, below) } else { Point::new(r.x + LABEL_GAP, r.y + LABEL_GAP) };
                draw_label(painter, theme, &label, Some(at));
            }
            None => painter.rect(full, &RectStyle::filled(dim, Corners::ZERO)),
        }
        if v.selection.rect().is_none() && !v.selection.is_dragging() {
            draw_label(painter, theme, &t!("shell-sharepicker-region-hint"), None);
        }
    }
}

fn actions() -> Vec<DialogAction> {
    vec![
        DialogAction::new(t!("shell-sharepicker-cancel"), DialogActionKind::Default),
        DialogAction::new(t!("shell-sharepicker-share"), DialogActionKind::Primary),
    ]
}

fn choice_kind(c: &Choice) -> &'static str {
    match c {
        Choice::Output(_) => "screen",
        Choice::Window(_) => "window",
        Choice::Region(_) => "region",
    }
}

/// Kart genişliği (mantıksal px, tema boşluk birimiyle).
fn card_width(theme: &Theme, bounds: Size) -> f32 {
    theme.space(130).min(bounds.width - theme.space(12)).max(theme.space(60))
}

fn status_line_height(cx: &mut Context<SharePicker>, theme: &Theme) -> f32 {
    // Durum satırına ayrılan yer: gösterilebilecek tek metnin (uyarı) yüksekliği.
    Label::new(t!("shell-sharepicker-pick-first"), text_style(theme, TextRole::Caption)).measure(cx.text()).height
}

/// Pencere satırlarının ön ikonunu, çözülen gerçek uygulama ikonuyla değiştirir (yedek: glif ya
/// da genel ikon kalır). Döner: yüklenmesi süren doku var (bir kare daha).
fn upgrade_window_icons(
    painter: &mut Painter,
    theme: &Theme,
    icons: &AppIcons,
    textures: &mut AppIconTextures,
    b: &mut Body,
    scale: u32,
) -> bool {
    // Satır ikonunun kenarı (ListItem ön ikonu: varsayılan rolün kutusu).
    let side = IconView::new(theme, Icon::named(IconName::Window)).size().height.round().max(1.0) as u32;
    let mut pending = false;
    for (i, item) in b.windows.items_mut().iter_mut().enumerate() {
        if b.window_icon_ready.get(i).copied().unwrap_or(true) {
            continue;
        }
        let Some(class) = b.window_classes.get(i).filter(|c| !c.is_empty()) else { continue };
        if let Some(texture) = textures.get(painter, icons, class, side, scale) {
            pending |= !texture.is_ready();
            item.set_leading(theme, Some(Icon::image(texture)));
            b.window_icon_ready[i] = true;
        }
    }
    pending
}

/// Önizleme bölmesi: seçili pencerenin karesi (en-boy korunur) ya da genel pencere ikonu.
/// Döner: doku yüklemesi sürüyor.
fn draw_preview(ui: &mut Ui, theme: &Theme, pane: Rect, preview: Option<&mut Preview>, fallback: &mut IconView) -> bool {
    let c = &theme.colors;
    let radii = theme.shape.small();
    ui.painter.rect(pane, &RectStyle::filled(ui.fade(c.surface_container), radii));
    if let Some(p) = preview
        && let Preview::Ready(_) | Preview::Uploaded(_) = p
    {
        if let Preview::Ready(image) = p {
            let texture = ui.painter.upload_texture(image.clone(), TextureOptions { mipmaps: false });
            *p = Preview::Uploaded(texture);
        }
        if let Preview::Uploaded(texture) = p {
            let inner = pane.inset(nuxut::Insets::uniform(theme.space(2)));
            let style = ImageStyle { fit: ImageFit::Contain, radii, opacity: ui.alpha(), tint: None };
            ui.painter.image(inner, texture, &style);
            return !texture.is_ready();
        }
    }
    fallback.draw_centered(ui.painter, pane.center(), ui.fade(c.on_surface_variant));
    false
}

/// Ekran satırının alt metni: açıklama (varsa) · boyut · yenileme hızı.
fn screen_detail(o: &OutputInfo) -> String {
    // Gerçek çözünürlük: fiziksel boyut (mantıksalla aynı yönde); bilinmiyorsa mantıksal.
    let (w, h) = match o.physical_size {
        (0, _) | (_, 0) => (o.size.width.max(0.0).round() as u32, o.size.height.max(0.0).round() as u32),
        p => p,
    };
    let size = if o.refresh > 0.0 {
        t!(
            "shell-sharepicker-screen-mode",
            width = w.to_string(),
            height = h.to_string(),
            refresh = format!("{:.0}", o.refresh)
        )
    } else {
        t!("shell-screenshot-size", width = w.to_string(), height = h.to_string())
    };
    let description = display_text(&o.description, MAX_DESCRIPTION_CHARS);
    if description.is_empty() { size } else { format!("{description} · {size}") }
}

/// Etiket: `surface_container` zemin, `on_surface` metin. `at` yoksa altta ortada.
fn draw_label(painter: &mut Painter, theme: &Theme, s: &str, at: Option<Point>) {
    let style = text_style(theme, TextRole::Label).with_tabular_numbers();
    let layout = painter.text_system().layout(s, &style, None);
    let ts = layout.size();
    let bw = ts.width + LABEL_PAD.0 * 2.0;
    let bh = ts.height + LABEL_PAD.1 * 2.0;
    let full = painter.size();
    let origin = match at {
        Some(p) => Point::new(
            p.x.min(full.width - bw - LABEL_GAP).max(LABEL_GAP),
            p.y.min(full.height - bh - LABEL_GAP).max(LABEL_GAP),
        ),
        None => Point::new(((full.width - bw) / 2.0).max(0.0), full.height - bh - LABEL_GAP * 4.0),
    };
    let c = &theme.colors;
    painter.rect(Rect::new(origin.x, origin.y, bw, bh), &RectStyle::filled(c.surface_container, theme.shape.small()));
    painter.text(&layout, Point::new(origin.x + LABEL_PAD.0, origin.y + LABEL_PAD.1), c.on_surface);
}

fn handle_rects(r: Rect) -> [Rect; 4] {
    let h = HANDLE_SIZE;
    let at = |x: f32, y: f32| Rect::new(x - h / 2.0, y - h / 2.0, h, h);
    [at(r.x, r.y), at(r.x + r.width, r.y), at(r.x, r.y + r.height), at(r.x + r.width, r.y + r.height)]
}

impl App for SharePicker {
    type Message = PickerMessage;

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.logind.is_none() && !self.done {
            self.watch_lock(cx);
        }
        if !valid_output_name(&output.name) {
            tracing::warn!("adı sözleşmeye uymayan çıkış atlandı");
        } else if !self.outputs.iter().any(|o| o.id == output.id) {
            self.outputs.push(output.clone());
            self.refresh_screens(cx);
        }
        if !self.started {
            self.started = true;
            self.open_picker_surface(cx);
            self.reset_idle(cx);
        }
    }

    fn output_changed(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if let Some(o) = self.outputs.iter_mut().find(|o| o.id == output.id) {
            if valid_output_name(&output.name) {
                *o = output.clone();
            }
            self.refresh_screens(cx);
        }
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        self.outputs.retain(|o| o.id != output);
        let before = self.regions.len();
        self.regions.retain(|v| v.output != output);
        if before > 0 && self.regions.is_empty() {
            // Bölge katmanlarının hepsi gitti: karta dön.
            self.open_picker_surface(cx);
            return;
        }
        if let Some(s) = self.surface
            && cx.surface_size(s).is_none()
        {
            // Kartın çıkışı gitti: görünmeyen seçici iptal (fail-closed).
            self.cancel(cx);
            return;
        }
        self.refresh_screens(cx);
    }

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, scale: f32) {
        if let Some(v) = self.regions.iter_mut().find(|v| v.surface == surface) {
            if size.width > 0.0 && size.height > 0.0 {
                v.size = size;
                v.scale = scale;
                v.selection.set_size((size.width, size.height));
            }
            cx.request_frame(surface);
            return;
        }
        if Some(surface) != self.surface {
            return;
        }
        let changed = self.size != size || self.scale != scale;
        self.size = size;
        self.scale = scale;
        if changed {
            self.rebuild(cx);
        }
        cx.request_frame(surface);
    }

    fn surface_presented(&mut self, _cx: &mut Context<Self>, surface: SurfaceId) {
        if Some(surface) == self.surface && self.shown_at.is_none() {
            tracing::info!("seçici gösterildi");
            self.shown_at = Some(Instant::now());
        }
    }

    fn close_requested(&mut self, cx: &mut Context<Self>, _surface: SurfaceId) {
        self.cancel(cx);
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) {
        if Some(surface) == self.surface {
            self.picker_pointer(cx, &event);
        } else {
            self.region_pointer(cx, surface, &event);
        }
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        if Some(surface) == self.surface {
            self.picker_key(cx, &event);
        } else {
            self.region_key(cx, surface, &event);
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.lock_deadline == Some(id) {
            self.lock_deadline = None;
            if self.lock == LockGate::Unknown {
                tracing::warn!("logind yanıt vermedi; kilit durumu bilinmiyor, istek reddediliyor");
            }
            return self.apply_lock(cx, self.lock.after_deadline());
        }
        if self.idle_timer == Some(id) {
            self.idle_timer = None;
            tracing::info!("etkileşimsiz süre doldu; iptal");
            self.cancel(cx);
        }
    }

    fn terminate(&mut self, cx: &mut Context<Self>) {
        self.cancel(cx);
    }

    fn theme_changed(&mut self, cx: &mut Context<Self>) {
        self.rebuild(cx);
        for v in &self.regions {
            cx.request_frame(v.surface);
        }
    }

    fn locale_changed(&mut self, cx: &mut Context<Self>) {
        self.rebuild(cx);
    }

    fn message(&mut self, cx: &mut Context<Self>, message: PickerMessage) {
        match message {
            // Uygulama ikonu çözüldü: satır ikonları bir sonraki karede.
            PickerMessage::Icons => self.redraw(cx),
            PickerMessage::Lock(gate) => self.apply_lock(cx, gate),
        }
    }

    fn toplevel(&mut self, cx: &mut Context<Self>, event: ToplevelEvent) {
        // Adres yanıtı `Added`'dan önce gelir: yeni pencerenin tutamacı artık aranabilir.
        if let ToplevelEvent::Added(_) = event {
            self.request_previews(cx);
        }
    }

    fn captured(&mut self, cx: &mut Context<Self>, id: CaptureId, result: Result<CapturedImage, CaptureError>) {
        let image = match result {
            Ok(c) => Some(c.image),
            Err(e) => {
                // Yalnızca hata türü (pencere kimliği/başlığı yok).
                tracing::info!(error = ?e, "pencere önizlemesi alınamadı");
                None
            }
        };
        if self.previews.finish(id, image) && self.tab == Tab::Window {
            self.redraw(cx);
        }
    }

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        if Some(surface) == self.surface {
            self.draw_picker(cx, surface, now, painter);
        } else {
            self.draw_region(cx, surface, painter);
        }
    }
}

impl SharePicker {
    /// Pencere sekmesi ilk açıldığında pencere listesini başlatır (compositor pencere yakalamayı
    /// sunuyorsa). Tembel: Ekran/Bölge seçen kullanıcı için hiçbir pencere yakalanmaz.
    fn start_previews(&mut self, cx: &mut Context<Self>) {
        if !self.lock.previews_allowed()
            || self.previews.started || self.setup.windows.windows.is_empty() || !cx.capture_support().toplevel {
            return;
        }
        self.previews.started = true;
        match cx.toplevels_start() {
            Ok(()) => {
                self.previews.active = true;
                self.request_previews(cx);
            }
            Err(e) => tracing::info!(error = ?e, "pencere listesi yok; önizleme gösterilmez"),
        }
    }

    /// Tutamacı bilinen, henüz istenmemiş pencereler için önizleme yakalaması ister.
    fn request_previews(&mut self, cx: &mut Context<Self>) {
        if !self.previews.active || !self.lock.previews_allowed() {
            return;
        }
        let (w, h) = self.preview_pixels(cx.theme());
        for address in self.previews.wanted(&self.setup.windows) {
            let Some(id) = cx.toplevel_by_hyprland_address(address) else { continue };
            let capture = cx.capture(CaptureSource::Toplevel(id), CaptureOptions::preview(w, h));
            self.previews.map.insert(address, Preview::Pending(capture));
        }
    }

    /// logind'i yalnız okumak için izler (`LockedHint`); durum `PickerMessage::Lock` ile gelir.
    /// Kapalı-güvenli: durum bilinene dek framework yakalama kapısı kapalı; başlatılamazsa ya da
    /// [`LOCK_DEADLINE`] içinde yanıt gelmezse istek reddedilir (`LockGate`).
    fn watch_lock(&mut self, cx: &mut Context<Self>) {
        cx.set_session_locked_hint(true);
        self.lock_deadline = Some(cx.set_timer(LOCK_DEADLINE));
        let tx = cx.sender();
        let spawned = IdleLogind::spawn(LogindOptions::default(), move |e| {
            if let LogindEvent::State(s) = e {
                let _ = tx.send(PickerMessage::Lock(LockGate::from_logind(s.available, s.has_session, s.locked_hint)));
            }
        });
        match spawned {
            Ok(l) => self.logind = Some(l),
            Err(e) => {
                tracing::warn!("logind izlenemedi; kilit durumu bilinmiyor, istek reddediliyor: {e}");
                self.apply_lock(cx, LockGate::from_logind(false, false, false));
            }
        }
    }

    /// Kilit durumu değişti: kilitliyse istek reddedilir (yürüyen önizlemeler framework kapısında
    /// `Locked` ile biter); açıksa bekleyen önizlemeler istenir.
    fn apply_lock(&mut self, cx: &mut Context<Self>, gate: LockGate) {
        if self.done || self.lock == gate {
            return;
        }
        self.lock = gate;
        cx.set_session_locked_hint(!gate.previews_allowed());
        if gate.must_reject() {
            tracing::info!("oturum kilitli ya da kilit durumu okunamadı: ekran paylaşım isteği reddedildi");
            self.finish(cx, None);
        } else if self.tab == Tab::Window {
            self.start_previews(cx);
            self.request_previews(cx);
        }
    }

    /// Önizleme bölmesinin fiziksel piksel boyutu (yakalama bu kutuya sığdırılır).
    fn preview_pixels(&self, theme: &Theme) -> (u32, u32) {
        let width = card_width(theme, self.size) * PREVIEW_SHARE;
        let height = self.body.as_ref().map_or(theme.space(64), |b| b.area_h);
        let px = |v: f32| ((v * self.scale.max(1.0)).ceil() as u32).clamp(16, 2048);
        (px(width), px(height))
    }

    fn refresh_screens(&mut self, cx: &mut Context<Self>) {
        let theme = cx.theme().clone();
        if let Some(name) = &self.selected_screen
            && !self.outputs.iter().any(|o| &o.name == name)
        {
            self.selected_screen = None;
        }
        if let Some(mut b) = self.body.take() {
            self.fill_screens(&theme, &mut b);
            self.body = Some(b);
            self.redraw(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::screenshot::selection::FRect;

    #[test]
    fn tabs_round_trip() {
        for t in Tab::ALL {
            assert_eq!(Tab::from_index(t.index()), t);
        }
        assert_eq!(Tab::from_index(99), Tab::Screen);
    }

    #[test]
    fn handle_rects_centered_on_corners() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        let hs = handle_rects(r);
        assert_eq!(hs[0].center(), Point::new(10.0, 20.0));
        assert_eq!(hs[3].center(), Point::new(110.0, 70.0));
    }

    #[test]
    fn previews_want_each_known_address_once_and_are_bounded() {
        use crate::services::sharepicker::SharedWindow;
        let w = |id: u32, address: u64| SharedWindow { id, class: "kitty".into(), title: String::new(), address };
        let list = WindowList { windows: vec![w(1, 10), w(2, 0), w(3, 30), w(4, 10)], ..WindowList::default() };
        let mut p = Previews::default();
        // Adressiz (0) pencere istenmez; aynı adres bir kez.
        assert_eq!(p.wanted(&list), [10, 30]);
        p.map.insert(10, Preview::Failed);
        assert_eq!(p.wanted(&list), [30], "istenmiş adres yeniden istenmez");
        let many = WindowList { windows: (1..=100).map(|i| w(i, u64::from(i))).collect(), ..WindowList::default() };
        let mut p = Previews::default();
        assert_eq!(p.wanted(&many).len(), MAX_PREVIEWS);
        for a in 1..=5 {
            p.map.insert(a, Preview::Failed);
        }
        assert_eq!(p.wanted(&many).len(), MAX_PREVIEWS - 5, "toplam istek sınırlı");
    }

    #[test]
    fn region_selection_maps_to_contract() {
        // Seçim makinesi çıkışa göre mantıksal px verir; sözleşme aynı birimi bekler.
        let mut s = Selection::new((1280.0, 720.0));
        s.press((100.0, 100.0));
        s.motion((400.0, 300.0));
        s.release((400.0, 300.0), &[]);
        let Outcome::Confirm(r) = s.key(SelKey::Confirm, false, &[]) else { panic!("onay bekleniyordu") };
        let region = Region::from_logical("HEADLESS-1", r.tuple(), (1280.0, 720.0)).unwrap();
        assert_eq!((region.x, region.y, region.width, region.height), (100, 100, 300, 200));
        // Tıklama (sürüklemesiz): tüm çıkış.
        let mut s = Selection::new((1280.0, 720.0));
        s.press((5.0, 5.0));
        let Outcome::Confirm(r) = s.release((5.0, 5.0), &[]) else { panic!("onay bekleniyordu") };
        assert_eq!(r, FRect::new(0.0, 0.0, 1280.0, 720.0));
    }
}
