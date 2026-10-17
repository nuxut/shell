//! Ekran görüntüsü seçim katmanı: çıkış başına bir layer-shell `Overlay` yüzeyi.
//!
//! Dondurulmuş kareyi (yakalanan çıkış görüntüsü) tam ekran çizer; seçimin dışı temanın gölge
//! rengiyle karartılır, seçim `primary` kenarlıkla ve köşe tutamaçlarıyla, fiziksel piksel
//! boyutu bir etiketle gösterilir. Seçim yokken imlecin altındaki pencere aynı biçimde
//! vurgulanır. Durum makinesi `services::screenshot::selection`'da; bu dosya yalnızca girdi
//! çevirisi ve çizim. Renkler, köşe yarıçapları ve yazı tipi tema token'larından.

use nuxut::i18n::t;
use nuxut::render::{ImageData, ImageFit, ImageStyle, Texture, TextureOptions};
use nuxut::ui::{TextRole, text_style};
use nuxut::{
    Border, Button, Color, Corners, CursorShape, Key, KeyEvent, Painter, Point, PointerEvent, PointerKind, Rect, RectStyle, Size,
    Theme,
};

use crate::services::screenshot::Frame;
use crate::services::screenshot::geometry::{self, RgbaImage};
use crate::services::screenshot::selection::{CursorHint, FRect, Handle, Outcome, SelKey, Selection};

/// Seçim dışının karartma opaklığı (temanın gölge rengi üstünde; boşta karartmasıyla aynı yol).
const DIM_ALPHA: f32 = 0.45;
/// Seçim kenarlığı (mantıksal px).
const BORDER_WIDTH: f32 = 2.0;
/// Köşe tutamacı kenarı (mantıksal px).
const HANDLE_SIZE: f32 = 8.0;
/// Etiketin seçimden ve kenarlardan uzaklığı.
const LABEL_GAP: f32 = 8.0;
/// Etiket iç boşluğu (yatay, dikey).
const LABEL_PAD: (f32, f32) = (8.0, 4.0);

/// Tek çıkışın seçim katmanı.
#[derive(Debug)]
pub struct ShotView {
    frame: Frame,
    texture: Option<Texture>,
    selection: Selection,
    /// Görünür pencereler (çıkışa göre, çıkış sınırına kırpılmış; öncelik sırasıyla).
    windows: Vec<FRect>,
    size: Size,
}

impl ShotView {
    /// `windows`: düzen (genel mantıksal) koordinatında görünür pencereler.
    pub fn new(frame: Frame, windows: &[geometry::LogicalRect]) -> Self {
        let l = frame.layout;
        let local: Vec<FRect> = windows
            .iter()
            .filter_map(|w| w.intersect(&l))
            .map(|w| FRect::new((w.x - l.x) as f32, (w.y - l.y) as f32, w.w as f32, w.h as f32))
            .collect();
        let size = Size::new(l.w as f32, l.h as f32);
        Self { selection: Selection::new((size.width, size.height)), frame, texture: None, windows: local, size }
    }

    pub fn name(&self) -> &str {
        &self.frame.name
    }

    /// Configure: yüzeyin mantıksal boyutu (düzenden farklıysa yüzeyinki geçerli).
    pub fn resized(&mut self, size: Size) {
        if size.width > 0.0 && size.height > 0.0 {
            self.size = size;
            self.selection.set_size((size.width, size.height));
        }
    }

    pub fn has_selection(&self) -> bool {
        self.selection.rect().is_some()
    }

    /// Başka çıkışta seçim başladı. Değiştiyse `true`.
    pub fn clear_selection(&mut self) -> bool {
        self.selection.clear()
    }

    /// İşaretçi olayı → (sonuç, imleç).
    pub fn pointer(&mut self, event: PointerEvent) -> (Outcome, CursorShape) {
        let p = (event.position.x, event.position.y);
        let outcome = match event.kind {
            PointerKind::Enter | PointerKind::Motion => self.selection.motion(p),
            PointerKind::Leave => self.selection.leave(),
            PointerKind::Press(Button::Left) => self.selection.press(p),
            PointerKind::Release(Button::Left) => self.selection.release(p, &self.windows),
            PointerKind::Press(Button::Right) => Outcome::Cancel,
            _ => Outcome::None,
        };
        (outcome, cursor_shape(self.selection.cursor(p)))
    }

    /// Tuş olayı (bırakmalar yok sayılır).
    pub fn key(&mut self, event: &KeyEvent) -> Outcome {
        match sel_key(event) {
            Some(k) => self.selection.key(k, event.modifiers.shift, &self.windows),
            None => Outcome::None,
        }
    }

    /// Bu çıkışın seçimini Enter'la onaylar (seçim, vurgulanan pencere ya da tüm çıkış).
    pub fn confirm_key(&mut self) -> Outcome {
        self.selection.key(SelKey::Confirm, false, &self.windows)
    }

    /// Onaylanan dikdörtgenin dondurulmuş kareden kesimi (fiziksel piksel).
    pub fn crop(&self, rect: FRect) -> Option<RgbaImage> {
        let img = &self.frame.image;
        let r = geometry::crop_rect(rect.tuple(), (self.size.width, self.size.height), (img.width, img.height))?;
        img.crop(r)
    }

    /// Seçimin fiziksel boyutu (etiket).
    fn physical(&self, rect: FRect) -> (u32, u32) {
        let img = &self.frame.image;
        geometry::physical_size(rect.tuple(), (self.size.width, self.size.height), (img.width, img.height))
    }

    pub fn draw(&mut self, painter: &mut Painter, theme: &Theme, hint: bool) {
        let full = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        painter.damage_all();
        let texture = self.texture.get_or_insert_with(|| {
            let img = &self.frame.image;
            painter.upload_texture(ImageData { width: img.width, height: img.height, pixels: img.pixels.clone() }, TextureOptions::default())
        });
        painter.image(full, texture, &ImageStyle::new(ImageFit::Fill));

        let c = &theme.colors;
        let dim = c.shadow.with_alpha(DIM_ALPHA);
        let shown = self.selection.rect().or_else(|| self.selection.highlighted(&self.windows));
        match shown {
            Some(r) => {
                let r = to_rect(r);
                for d in outside(full, r) {
                    painter.rect(d, &RectStyle::filled(dim, Corners::ZERO));
                }
                let border = Border { width: BORDER_WIDTH, color: c.primary };
                painter.rect(r, &RectStyle { fill: Color::TRANSPARENT, radii: Corners::ZERO, border: Some(border) });
                if self.selection.rect().is_some() {
                    for h in handle_rects(r) {
                        painter.rect(h, &RectStyle::filled(c.primary, Corners::ZERO));
                    }
                }
                let (w, h) = self.physical(from_rect(r));
                let label = t!("shell-screenshot-size", width = w.to_string(), height = h.to_string());
                draw_label(painter, theme, &label, label_origin(full, r), false);
            }
            None => painter.rect(full, &RectStyle::filled(dim, Corners::ZERO)),
        }
        if hint && self.selection.rect().is_none() && !self.selection.is_dragging() {
            draw_label(painter, theme, &t!("shell-screenshot-hint"), None, true);
        }
    }
}

/// Etiket: `surface_container` zemin, `on_surface` metin, küçük köşe. `at` yoksa altta ortada.
fn draw_label(painter: &mut Painter, theme: &Theme, s: &str, at: Option<Point>, bottom_center: bool) {
    let style = text_style(theme, TextRole::Label).with_tabular_numbers();
    let layout = painter.text_system().layout(s, &style, None);
    let ts = layout.size();
    let bw = ts.width + LABEL_PAD.0 * 2.0;
    let bh = ts.height + LABEL_PAD.1 * 2.0;
    let full = painter.size();
    let origin = match at {
        Some(p) => Point::new(p.x.min(full.width - bw - LABEL_GAP).max(LABEL_GAP), p.y.min(full.height - bh - LABEL_GAP).max(LABEL_GAP)),
        None if bottom_center => Point::new(((full.width - bw) / 2.0).max(0.0), full.height - bh - LABEL_GAP * 4.0),
        None => Point::new(LABEL_GAP, LABEL_GAP),
    };
    let c = &theme.colors;
    painter.rect(Rect::new(origin.x, origin.y, bw, bh), &RectStyle::filled(c.surface_container, theme.shape.small()));
    painter.text(&layout, Point::new(origin.x + LABEL_PAD.0, origin.y + LABEL_PAD.1), c.on_surface);
}

/// Etiket seçimin altında sol hizalı; altta yer yoksa seçimin içinde, üst köşede.
fn label_origin(full: Rect, r: Rect) -> Option<Point> {
    let below = r.y + r.height + LABEL_GAP;
    Some(if below + 32.0 < full.height { Point::new(r.x, below) } else { Point::new(r.x + LABEL_GAP, r.y + LABEL_GAP) })
}

/// `full` içinde `r`'nin dışında kalan en çok dört dikdörtgen (üst, alt, sol, sağ).
pub fn outside(full: Rect, r: Rect) -> Vec<Rect> {
    let (x0, y0) = (r.x.max(full.x), r.y.max(full.y));
    let (x1, y1) = ((r.x + r.width).min(full.x + full.width), (r.y + r.height).min(full.y + full.height));
    if x1 <= x0 || y1 <= y0 {
        return vec![full];
    }
    let fr = full.x + full.width;
    let fb = full.y + full.height;
    [
        Rect::new(full.x, full.y, full.width, y0 - full.y),
        Rect::new(full.x, y1, full.width, fb - y1),
        Rect::new(full.x, y0, x0 - full.x, y1 - y0),
        Rect::new(x1, y0, fr - x1, y1 - y0),
    ]
    .into_iter()
    .filter(|d| d.width > 0.0 && d.height > 0.0)
    .collect()
}

fn handle_rects(r: Rect) -> [Rect; 4] {
    let h = HANDLE_SIZE;
    let at = |x: f32, y: f32| Rect::new(x - h / 2.0, y - h / 2.0, h, h);
    [at(r.x, r.y), at(r.x + r.width, r.y), at(r.x, r.y + r.height), at(r.x + r.width, r.y + r.height)]
}

fn to_rect(r: FRect) -> Rect {
    Rect::new(r.x, r.y, r.w, r.h)
}

fn from_rect(r: Rect) -> FRect {
    FRect::new(r.x, r.y, r.width, r.height)
}

/// Tuş → seçim tuşu (bırakma ve ilgisiz tuşlar `None`).
pub fn sel_key(event: &KeyEvent) -> Option<SelKey> {
    if !event.is_down() {
        return None;
    }
    Some(match event.key {
        Key::Escape => SelKey::Cancel,
        Key::Enter | Key::Space => SelKey::Confirm,
        Key::Left => SelKey::Nudge(-1, 0),
        Key::Right => SelKey::Nudge(1, 0),
        Key::Up => SelKey::Nudge(0, -1),
        Key::Down => SelKey::Nudge(0, 1),
        _ => return None,
    })
}

pub fn cursor_shape(hint: CursorHint) -> CursorShape {
    match hint {
        CursorHint::Crosshair => CursorShape::Crosshair,
        CursorHint::Move => CursorShape::Move,
        CursorHint::Resize(h) => match h {
            Handle::N => CursorShape::ResizeN,
            Handle::S => CursorShape::ResizeS,
            Handle::E => CursorShape::ResizeE,
            Handle::W => CursorShape::ResizeW,
            Handle::Ne => CursorShape::ResizeNe,
            Handle::Nw => CursorShape::ResizeNw,
            Handle::Se => CursorShape::ResizeSe,
            Handle::Sw => CursorShape::ResizeSw,
        },
    }
}

#[cfg(test)]
mod tests {
    use nuxut::{KeyState, Modifiers};

    use super::*;
    use crate::services::screenshot::geometry::LogicalRect;

    fn view() -> ShotView {
        // 100x50 mantıksal çıkış, 2x görüntü; konum (100, 0).
        let mut px = Vec::new();
        for y in 0..100u32 {
            for x in 0..200u32 {
                px.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        let image = RgbaImage::new(200, 100, px).unwrap();
        let frame = Frame { name: "HEADLESS-2".into(), layout: LogicalRect::new(100, 0, 100, 50), image };
        let windows = [LogicalRect::new(90, 10, 30, 20), LogicalRect::new(0, 0, 50, 50)];
        ShotView::new(frame, &windows)
    }

    fn key(k: Key, state: KeyState, shift: bool) -> KeyEvent {
        KeyEvent::new(k, state, None, Modifiers { shift, ..Modifiers::default() })
    }

    #[test]
    fn windows_become_local_and_clipped() {
        let v = view();
        assert_eq!(v.windows, vec![FRect::new(0.0, 10.0, 20.0, 20.0)], "çıkış dışındaki pencere atıldı, taşan kırpıldı");
    }

    #[test]
    fn click_on_window_crops_scaled_frame() {
        let mut v = view();
        let ev = |kind| PointerEvent::new(Point::new(5.0, 15.0), kind);
        v.pointer(ev(PointerKind::Motion));
        v.pointer(ev(PointerKind::Press(Button::Left)));
        let (out, _) = v.pointer(ev(PointerKind::Release(Button::Left)));
        let Outcome::Confirm(r) = out else { panic!("{out:?}") };
        let img = v.crop(r).unwrap();
        assert_eq!((img.width, img.height), (40, 40), "2x");
        assert_eq!(img.pixel(0, 0), [0, 20, 0, 255]);
        assert_eq!(img.pixel(39, 39), [39, 59, 0, 255]);
    }

    #[test]
    fn keys_and_cursor() {
        assert_eq!(sel_key(&key(Key::Escape, KeyState::Pressed, false)), Some(SelKey::Cancel));
        assert_eq!(sel_key(&key(Key::Enter, KeyState::Pressed, false)), Some(SelKey::Confirm));
        assert_eq!(sel_key(&key(Key::Space, KeyState::Pressed, false)), Some(SelKey::Confirm));
        assert_eq!(sel_key(&key(Key::Left, KeyState::Repeated, true)), Some(SelKey::Nudge(-1, 0)));
        assert_eq!(sel_key(&key(Key::Enter, KeyState::Released, false)), None);
        assert_eq!(sel_key(&key(Key::Char('a'), KeyState::Pressed, false)), None);
        let mut v = view();
        assert_eq!(v.key(&key(Key::Escape, KeyState::Pressed, false)), Outcome::Cancel);
        let (_, c) = v.pointer(PointerEvent::new(Point::new(50.0, 40.0), PointerKind::Motion));
        assert_eq!(c, CursorShape::Crosshair);
        let (o, _) = v.pointer(PointerEvent::new(Point::new(1.0, 1.0), PointerKind::Press(Button::Right)));
        assert_eq!(o, Outcome::Cancel);
        assert_eq!(cursor_shape(CursorHint::Resize(Handle::Se)), CursorShape::ResizeSe);
        assert_eq!(cursor_shape(CursorHint::Move), CursorShape::Move);
    }

    #[test]
    fn outside_rects_cover_the_rest() {
        let full = Rect::new(0.0, 0.0, 100.0, 50.0);
        let parts = outside(full, Rect::new(10.0, 10.0, 20.0, 20.0));
        assert_eq!(parts.len(), 4);
        let area: f32 = parts.iter().map(|r| r.width * r.height).sum();
        assert_eq!(area, 100.0 * 50.0 - 400.0);
        assert_eq!(outside(full, Rect::new(0.0, 0.0, 100.0, 50.0)).len(), 0);
        assert_eq!(outside(full, Rect::new(200.0, 0.0, 10.0, 10.0)), vec![full]);
        assert_eq!(outside(full, Rect::new(0.0, 0.0, 100.0, 20.0)).len(), 1);
    }

    #[test]
    fn resize_follows_surface_size() {
        let mut v = view();
        v.resized(Size::new(200.0, 100.0));
        // Yüzey 200x100 mantıksal, görüntü 200x100: ölçek 1.
        let img = v.crop(FRect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
        assert_eq!((img.width, img.height), (10, 10));
        v.resized(Size::ZERO);
        assert_eq!(v.size, Size::new(200.0, 100.0), "boş configure yok sayılır");
    }
}
