//! Bölge seçimi durum makinesi (tek çıkış; koordinatlar çıkışa göre mantıksal piksel).
//!
//! - Sol tuşla sürükle: yeni seçim. Seçimin içinden sürükle: taşı. Kenar/köşeden sürükle:
//!   yeniden boyutlandır (ters dönerse normalleşir). Hepsi çıkış sınırına kırpılır.
//! - Seçim yokken tıklama (sürüklemeden bırakma): imlecin altındaki pencere, yoksa tüm çıkış
//!   onaylanır. Seçim varken dışarıya tıklama seçimi kaldırır.
//! - Enter/Space: seçim, yoksa vurgulanan pencere, o da yoksa tüm çıkış onaylanır.
//! - Esc ya da sağ tık: iptal. Ok tuşları seçimi 1 px (Shift ile 10 px) kaydırır.
//!
//! Pencereler çağıranın öncelik sırasıyla verilir (üstteki önce); vurgu ilk eşleşendir.

/// Kayan noktalı dikdörtgen (mantıksal px).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl FRect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// İki köşeden (sıra önemsiz).
    pub fn from_corners(a: (f32, f32), b: (f32, f32)) -> Self {
        let (x0, x1) = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
        let (y0, y1) = if a.1 <= b.1 { (a.1, b.1) } else { (b.1, a.1) };
        Self::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.right() && p.1 >= self.y && p.1 < self.bottom()
    }

    /// `bounds` ile kesişim (boşsa `None`).
    pub fn clip(&self, bounds: &FRect) -> Option<FRect> {
        let x = self.x.max(bounds.x);
        let y = self.y.max(bounds.y);
        let r = self.right().min(bounds.right());
        let b = self.bottom().min(bounds.bottom());
        (r > x && b > y).then(|| FRect::new(x, y, r - x, b - y))
    }

    /// Boyutu korunarak `bounds` içine itilir (boyut büyükse kırpılır).
    fn pushed_inside(&self, bounds: &FRect) -> FRect {
        let w = self.w.min(bounds.w);
        let h = self.h.min(bounds.h);
        let x = self.x.clamp(bounds.x, bounds.right() - w);
        let y = self.y.clamp(bounds.y, bounds.bottom() - h);
        FRect::new(x, y, w, h)
    }

    pub fn tuple(&self) -> (f32, f32, f32, f32) {
        (self.x, self.y, self.w, self.h)
    }
}

/// Yeniden boyutlandırma tutamacı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    N,
    S,
    E,
    W,
    Ne,
    Nw,
    Se,
    Sw,
}

impl Handle {
    fn edges(self) -> (bool, bool, bool, bool) {
        // (sol, üst, sağ, alt)
        match self {
            Handle::N => (false, true, false, false),
            Handle::S => (false, false, false, true),
            Handle::E => (false, false, true, false),
            Handle::W => (true, false, false, false),
            Handle::Ne => (false, true, true, false),
            Handle::Nw => (true, true, false, false),
            Handle::Se => (false, false, true, true),
            Handle::Sw => (true, false, false, true),
        }
    }
}

/// İmleç ipucu (görünüm `CursorShape`'e çevirir).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorHint {
    Crosshair,
    Move,
    Resize(Handle),
}

/// Tuş (görünüm `KeyEvent`'ten çevirir).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelKey {
    Cancel,
    Confirm,
    /// Ok tuşu yönü (-1/0/1).
    Nudge(i8, i8),
}

/// Bir olayın sonucu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    /// Değişiklik yok.
    None,
    /// Yeniden çizilmeli.
    Redraw,
    /// Bu dikdörtgen onaylandı (çıkış sınırı içinde, boş değil).
    Confirm(FRect),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    /// Basıldı, henüz eşik aşılmadı (tıklama mı sürükleme mi belli değil).
    Pending { start: (f32, f32) },
    New { anchor: (f32, f32) },
    Move { start: (f32, f32), orig: FRect },
    Resize { handle: Handle, orig: FRect },
}

/// Tıklamayı sürüklemeden ayıran eşik (mantıksal px).
pub const DRAG_THRESHOLD: f32 = 4.0;
/// Kenar/köşe tutamacının yakalama payı (mantıksal px, kenarın iki yanında).
pub const HANDLE_SLOP: f32 = 8.0;
/// Shift'li ok tuşu adımı.
pub const NUDGE_FAST: f32 = 10.0;

/// Bir çıkışın seçim durumu.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    bounds: FRect,
    rect: Option<FRect>,
    drag: Option<Drag>,
    pointer: Option<(f32, f32)>,
}

impl Selection {
    /// `size`: çıkışın mantıksal boyutu.
    pub fn new(size: (f32, f32)) -> Self {
        Self { bounds: FRect::new(0.0, 0.0, size.0.max(0.0), size.1.max(0.0)), rect: None, drag: None, pointer: None }
    }

    pub fn bounds(&self) -> FRect {
        self.bounds
    }

    /// Yüzey boyutu değişti (configure): seçim yeni sınıra kırpılır.
    pub fn set_size(&mut self, size: (f32, f32)) {
        self.bounds = FRect::new(0.0, 0.0, size.0.max(0.0), size.1.max(0.0));
        self.rect = self.rect.and_then(|r| r.clip(&self.bounds));
    }

    /// Geçerli seçim (sürükleme sürerken de).
    pub fn rect(&self) -> Option<FRect> {
        self.rect
    }

    pub fn is_dragging(&self) -> bool {
        matches!(self.drag, Some(Drag::New { .. } | Drag::Move { .. } | Drag::Resize { .. }))
    }

    /// Başka çıkışta seçim başladı: bu çıkışınki kalkar.
    pub fn clear(&mut self) -> bool {
        let had = self.rect.is_some() || self.drag.is_some();
        self.rect = None;
        self.drag = None;
        had
    }

    /// İşaretçi çıktı: vurgu kalkar (sürükleme sürmüyorsa).
    pub fn leave(&mut self) -> Outcome {
        if self.drag.is_some() {
            return Outcome::None;
        }
        if self.pointer.take().is_some() { Outcome::Redraw } else { Outcome::None }
    }

    /// Vurgulanacak pencere: seçim ve sürükleme yokken imlecin altındaki ilk pencere.
    pub fn highlighted(&self, windows: &[FRect]) -> Option<FRect> {
        if self.rect.is_some() || self.drag.is_some_and(|d| !matches!(d, Drag::Pending { .. })) {
            return None;
        }
        let p = self.pointer?;
        windows.iter().find(|w| w.contains(p)).and_then(|w| w.clip(&self.bounds))
    }

    pub fn cursor(&self, p: (f32, f32)) -> CursorHint {
        match self.drag {
            Some(Drag::Move { .. }) => return CursorHint::Move,
            Some(Drag::Resize { handle, .. }) => return CursorHint::Resize(handle),
            Some(Drag::New { .. } | Drag::Pending { .. }) => return CursorHint::Crosshair,
            None => {}
        }
        match self.rect {
            Some(r) => match handle_at(&r, p) {
                Some(h) => CursorHint::Resize(h),
                None if r.contains(p) => CursorHint::Move,
                None => CursorHint::Crosshair,
            },
            None => CursorHint::Crosshair,
        }
    }

    pub fn press(&mut self, p: (f32, f32)) -> Outcome {
        self.pointer = Some(p);
        self.drag = Some(match self.rect {
            Some(r) => match handle_at(&r, p) {
                Some(handle) => Drag::Resize { handle, orig: r },
                None if r.contains(p) => Drag::Move { start: p, orig: r },
                None => Drag::Pending { start: p },
            },
            None => Drag::Pending { start: p },
        });
        Outcome::Redraw
    }

    pub fn motion(&mut self, p: (f32, f32)) -> Outcome {
        let moved = self.pointer != Some(p);
        self.pointer = Some(p);
        let b = self.bounds;
        let clamp = |q: (f32, f32)| (q.0.clamp(b.x, b.right()), q.1.clamp(b.y, b.bottom()));
        match self.drag {
            None => return if moved { Outcome::Redraw } else { Outcome::None },
            Some(Drag::Pending { start }) => {
                if (p.0 - start.0).hypot(p.1 - start.1) < DRAG_THRESHOLD {
                    return Outcome::None;
                }
                self.drag = Some(Drag::New { anchor: clamp(start) });
                self.rect = Some(FRect::from_corners(clamp(start), clamp(p)));
            }
            Some(Drag::New { anchor }) => self.rect = Some(FRect::from_corners(anchor, clamp(p))),
            Some(Drag::Move { start, orig }) => {
                let moved = FRect { x: orig.x + p.0 - start.0, y: orig.y + p.1 - start.1, ..orig };
                self.rect = Some(moved.pushed_inside(&b));
            }
            Some(Drag::Resize { handle, orig }) => {
                let (l, t, r, bo) = handle.edges();
                let q = clamp(p);
                let x0 = if l { q.0 } else { orig.x };
                let y0 = if t { q.1 } else { orig.y };
                let x1 = if r { q.0 } else { orig.right() };
                let y1 = if bo { q.1 } else { orig.bottom() };
                self.rect = Some(FRect::from_corners((x0, y0), (x1, y1)));
            }
        }
        Outcome::Redraw
    }

    /// Sol tuş bırakıldı. `windows`: tıklamada seçilecek pencereler (öncelik sırasıyla).
    pub fn release(&mut self, p: (f32, f32), windows: &[FRect]) -> Outcome {
        self.pointer = Some(p);
        match self.drag.take() {
            None => Outcome::None,
            Some(Drag::Pending { .. }) => {
                if self.rect.take().is_some() {
                    // Seçim varken dışarıya tıklama: seçim kalkar.
                    return Outcome::Redraw;
                }
                let target = windows.iter().find(|w| w.contains(p)).and_then(|w| w.clip(&self.bounds));
                self.confirm(target)
            }
            Some(_) => {
                if self.rect.is_some_and(|r| r.w < 1.0 || r.h < 1.0) {
                    self.rect = None;
                }
                Outcome::Redraw
            }
        }
    }

    pub fn key(&mut self, key: SelKey, fast: bool, windows: &[FRect]) -> Outcome {
        match key {
            SelKey::Cancel => Outcome::Cancel,
            SelKey::Confirm => {
                if self.is_dragging() {
                    return Outcome::None;
                }
                let target = self.rect.or_else(|| self.highlighted(windows));
                self.confirm(target)
            }
            SelKey::Nudge(dx, dy) => {
                let Some(r) = self.rect else { return Outcome::None };
                if self.drag.is_some() {
                    return Outcome::None;
                }
                let step = if fast { NUDGE_FAST } else { 1.0 };
                let next = FRect { x: r.x + f32::from(dx) * step, y: r.y + f32::from(dy) * step, ..r }.pushed_inside(&self.bounds);
                if next == r {
                    return Outcome::None;
                }
                self.rect = Some(next);
                Outcome::Redraw
            }
        }
    }

    /// Hedef ya da tüm çıkış; boş sınırda (henüz configure yok) onay yok.
    fn confirm(&self, target: Option<FRect>) -> Outcome {
        match target.or_else(|| (self.bounds.w > 0.0 && self.bounds.h > 0.0).then_some(self.bounds)) {
            Some(r) => Outcome::Confirm(r),
            None => Outcome::None,
        }
    }
}

/// `r`'nin kenar/köşe tutamacı `p`'de mi (köşe önce).
pub fn handle_at(r: &FRect, p: (f32, f32)) -> Option<Handle> {
    let near = |a: f32, b: f32| (a - b).abs() <= HANDLE_SLOP;
    let in_x = p.0 >= r.x - HANDLE_SLOP && p.0 <= r.right() + HANDLE_SLOP;
    let in_y = p.1 >= r.y - HANDLE_SLOP && p.1 <= r.bottom() + HANDLE_SLOP;
    if !in_x || !in_y {
        return None;
    }
    let (l, rt, t, b) = (near(p.0, r.x), near(p.0, r.right()), near(p.1, r.y), near(p.1, r.bottom()));
    Some(match (l, rt, t, b) {
        (true, _, true, _) => Handle::Nw,
        (_, true, true, _) => Handle::Ne,
        (true, _, _, true) => Handle::Sw,
        (_, true, _, true) => Handle::Se,
        (true, ..) => Handle::W,
        (_, true, ..) => Handle::E,
        (_, _, true, _) => Handle::N,
        (.., true) => Handle::S,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel() -> Selection {
        Selection::new((1000.0, 500.0))
    }

    fn drag(s: &mut Selection, a: (f32, f32), b: (f32, f32)) {
        s.press(a);
        s.motion(b);
        s.release(b, &[]);
    }

    #[test]
    fn drag_creates_normalized_clamped_selection() {
        let mut s = sel();
        drag(&mut s, (300.0, 200.0), (100.0, 50.0));
        assert_eq!(s.rect(), Some(FRect::new(100.0, 50.0, 200.0, 150.0)));
        let mut s = sel();
        drag(&mut s, (900.0, 400.0), (1200.0, 700.0));
        assert_eq!(s.rect(), Some(FRect::new(900.0, 400.0, 100.0, 100.0)), "çıkış sınırına kırpılır");
    }

    #[test]
    fn small_motion_is_a_click_selecting_window_or_output() {
        let windows = [FRect::new(50.0, 50.0, 200.0, 100.0), FRect::new(0.0, 0.0, 1000.0, 500.0)];
        let mut s = sel();
        s.press((60.0, 60.0));
        assert_eq!(s.motion((62.0, 61.0)), Outcome::None, "eşik altında");
        assert_eq!(s.release((62.0, 61.0), &windows), Outcome::Confirm(windows[0]), "üstteki pencere");
        let mut s = sel();
        s.press((600.0, 400.0));
        assert_eq!(s.release((600.0, 400.0), &windows[..1]), Outcome::Confirm(FRect::new(0.0, 0.0, 1000.0, 500.0)), "pencere yok: çıkış");
        // Çıkış dışına taşan pencere kırpılır.
        let mut s = sel();
        s.press((990.0, 10.0));
        assert_eq!(s.release((990.0, 10.0), &[FRect::new(900.0, -20.0, 300.0, 100.0)]), Outcome::Confirm(FRect::new(900.0, 0.0, 100.0, 80.0)));
    }

    #[test]
    fn click_outside_existing_selection_clears_it() {
        let mut s = sel();
        drag(&mut s, (100.0, 100.0), (200.0, 200.0));
        s.press((500.0, 400.0));
        assert_eq!(s.release((500.0, 400.0), &[]), Outcome::Redraw);
        assert_eq!(s.rect(), None);
    }

    #[test]
    fn move_inside_keeps_size_and_stays_in_bounds() {
        let mut s = sel();
        drag(&mut s, (100.0, 100.0), (300.0, 200.0));
        assert_eq!(s.cursor((150.0, 150.0)), CursorHint::Move);
        s.press((150.0, 150.0));
        s.motion((250.0, 170.0));
        assert_eq!(s.rect(), Some(FRect::new(200.0, 120.0, 200.0, 100.0)));
        s.motion((5000.0, 5000.0));
        assert_eq!(s.rect(), Some(FRect::new(800.0, 400.0, 200.0, 100.0)), "sınıra itilir");
        s.release((5000.0, 5000.0), &[]);
        assert!(!s.is_dragging());
    }

    #[test]
    fn resize_from_edges_and_corners_with_flip() {
        let mut s = sel();
        drag(&mut s, (100.0, 100.0), (300.0, 200.0));
        assert_eq!(s.cursor((300.0, 150.0)), CursorHint::Resize(Handle::E));
        assert_eq!(s.cursor((100.0, 100.0)), CursorHint::Resize(Handle::Nw));
        assert_eq!(s.cursor((200.0, 205.0)), CursorHint::Resize(Handle::S));
        s.press((300.0, 150.0));
        s.motion((400.0, 999.0));
        assert_eq!(s.rect(), Some(FRect::new(100.0, 100.0, 300.0, 100.0)), "E yalnızca sağ kenar");
        s.release((400.0, 999.0), &[]);
        // Sol üst köşeyi sağ alt köşenin ötesine: normalleşir.
        s.press((100.0, 100.0));
        s.motion((500.0, 300.0));
        assert_eq!(s.rect(), Some(FRect::new(400.0, 200.0, 100.0, 100.0)));
        s.release((500.0, 300.0), &[]);
        // Sıfıra indirilen seçim kalkar.
        s.press((500.0, 250.0)); // E kenarı
        s.motion((400.0, 250.0));
        s.release((400.0, 250.0), &[]);
        assert_eq!(s.rect(), None);
    }

    #[test]
    fn keys_confirm_cancel_and_nudge() {
        let windows = [FRect::new(10.0, 10.0, 50.0, 50.0)];
        let mut s = sel();
        assert_eq!(s.key(SelKey::Cancel, false, &windows), Outcome::Cancel);
        assert_eq!(s.key(SelKey::Confirm, false, &windows), Outcome::Confirm(FRect::new(0.0, 0.0, 1000.0, 500.0)), "hiçbir şey: çıkış");
        s.motion((20.0, 20.0));
        assert_eq!(s.highlighted(&windows), Some(windows[0]));
        assert_eq!(s.key(SelKey::Confirm, false, &windows), Outcome::Confirm(windows[0]), "vurgulanan pencere");
        assert_eq!(s.key(SelKey::Nudge(1, 0), false, &windows), Outcome::None, "seçim yok");
        drag(&mut s, (100.0, 100.0), (200.0, 150.0));
        assert_eq!(s.highlighted(&windows), None, "seçim varken vurgu yok");
        assert_eq!(s.key(SelKey::Nudge(1, 0), false, &windows), Outcome::Redraw);
        assert_eq!(s.key(SelKey::Nudge(0, -1), true, &windows), Outcome::Redraw);
        assert_eq!(s.rect(), Some(FRect::new(101.0, 90.0, 100.0, 50.0)));
        for _ in 0..20 {
            s.key(SelKey::Nudge(0, -1), true, &windows);
        }
        assert_eq!(s.rect().unwrap().y, 0.0, "sınırda durur");
        assert_eq!(s.key(SelKey::Nudge(0, -1), false, &windows), Outcome::None);
        assert_eq!(s.key(SelKey::Confirm, false, &windows), Outcome::Confirm(FRect::new(101.0, 0.0, 100.0, 50.0)));
    }

    #[test]
    fn confirm_ignored_while_dragging_and_resize_clips() {
        let mut s = sel();
        s.press((10.0, 10.0));
        s.motion((100.0, 100.0));
        assert_eq!(s.key(SelKey::Confirm, false, &[]), Outcome::None);
        s.release((100.0, 100.0), &[]);
        s.set_size((50.0, 50.0));
        assert_eq!(s.rect(), Some(FRect::new(10.0, 10.0, 40.0, 40.0)));
        assert!(s.clear());
        assert_eq!(s.rect(), None);
        let empty = Selection::new((0.0, 0.0));
        assert_eq!(empty.confirm(None), Outcome::None);
    }

    #[test]
    fn leave_drops_highlight_but_not_during_drag() {
        let windows = [FRect::new(0.0, 0.0, 100.0, 100.0)];
        let mut s = sel();
        s.motion((10.0, 10.0));
        assert!(s.highlighted(&windows).is_some());
        assert_eq!(s.leave(), Outcome::Redraw);
        assert!(s.highlighted(&windows).is_none());
        s.press((10.0, 10.0));
        s.motion((50.0, 50.0));
        assert_eq!(s.leave(), Outcome::None);
        assert!(s.is_dragging());
    }

    #[test]
    fn handle_hit_test() {
        let r = FRect::new(100.0, 100.0, 100.0, 100.0);
        assert_eq!(handle_at(&r, (205.0, 205.0)), Some(Handle::Se));
        assert_eq!(handle_at(&r, (95.0, 205.0)), Some(Handle::Sw));
        assert_eq!(handle_at(&r, (203.0, 95.0)), Some(Handle::Ne));
        assert_eq!(handle_at(&r, (150.0, 97.0)), Some(Handle::N));
        assert_eq!(handle_at(&r, (150.0, 150.0)), None);
        assert_eq!(handle_at(&r, (150.0, 250.0)), None);
        assert_eq!(handle_at(&r, (300.0, 100.0)), None);
    }
}
