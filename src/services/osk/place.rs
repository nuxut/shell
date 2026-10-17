//! Ekran klavyesinin yeri (t125 v2, karar 2026-10-16): alta yanaşık ya da yüzen;
//! tutamaçtan sürükleme. Saf geometri ve sürükleme durum makinesi (Wayland yok, birim test).
//!
//! Koordinatlar çıkışın mantıksal koordinatlarıdır (sol üst = 0,0). `area`, kabuğun çerçevesinin
//! iç dikdörtgenidir: yanaşık klavye onun altına oturur, yüzen klavye onun içinde kalır.

use nuxut::{Point, Rect, Size};

/// Alttan bu kadar yukarı çekilince yanaşık klavye yüzen kipe geçer (mantıksal px).
pub const UNDOCK_THRESHOLD: f32 = 24.0;
/// Yüzen klavye bırakıldığında alt kenarı yanaşma çizgisine bu kadar yakınsa yeniden yanaşır.
pub const SNAP_DISTANCE: f32 = 48.0;

/// Klavyenin kipi.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Placement {
    /// Alt kenara yanaşık, yatayda ortalı.
    #[default]
    Docked,
    /// Yüzen; klavye dikdörtgeninin sol üst köşesi.
    Floating(Point),
}

/// Yanaşık klavyenin dikdörtgeni: `area`nın alt kenarında, yatayda ortalı.
pub fn docked_rect(area: Rect, size: Size) -> Rect {
    Rect::new(area.x + (area.width - size.width) * 0.5, area.bottom() - size.height, size.width, size.height)
}

/// Dikdörtgeni `area` içine çeker (sığmıyorsa sol üst köşeye hizalanır).
pub fn clamp_into(area: Rect, rect: Rect) -> Rect {
    let x = rect.x.min(area.right() - rect.width).max(area.x);
    let y = rect.y.min(area.bottom() - rect.height).max(area.y);
    Rect::new(x, y, rect.width, rect.height)
}

/// Yerleşimin bu alandaki dikdörtgeni (yüzen konum alana çekilir: monitör küçülmüş olabilir).
pub fn rect_of(placement: Placement, area: Rect, size: Size) -> Rect {
    match placement {
        Placement::Docked => docked_rect(area, size),
        Placement::Floating(origin) => clamp_into(area, Rect::new(origin.x, origin.y, size.width, size.height)),
    }
}

/// Tutamaçtan bir sürükleme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    /// Basıldığı andaki işaretçi (çıkış koordinatı).
    start: Point,
    /// Basıldığı andaki klavye dikdörtgeni.
    start_rect: Rect,
    /// Yüzen kipte mi (yanaşıktan başladıysa eşik aşılınca `true`).
    floating: bool,
}

impl Drag {
    pub fn begin(pointer: Point, rect: Rect, placement: Placement) -> Self {
        Self { start: pointer, start_rect: rect, floating: matches!(placement, Placement::Floating(_)) }
    }

    /// Klavye yüzen kipe geçti mi (yanaşık başlangıçta eşik aşılmadıysa hâlâ yanaşık).
    pub fn is_floating(&self) -> bool {
        self.floating
    }

    /// İşaretçi hareketi → klavyenin şimdiki dikdörtgeni (alana çekili). Yanaşık başlangıçta
    /// yalnızca yukarı çekiş eşiği aşınca yüzer; o ana dek yerinde kalır.
    pub fn update(&mut self, pointer: Point, area: Rect) -> Rect {
        let d = Point::new(pointer.x - self.start.x, pointer.y - self.start.y);
        if !self.floating {
            if -d.y < UNDOCK_THRESHOLD {
                return self.start_rect;
            }
            self.floating = true;
        }
        clamp_into(area, Rect::new(self.start_rect.x + d.x, self.start_rect.y + d.y, self.start_rect.width, self.start_rect.height))
    }

    /// Bırakıldı: son dikdörtgene göre yeni yerleşim. Alt kenar yanaşma çizgisine yakınsa (ya
    /// da hiç yüzmediyse) yanaşık.
    pub fn finish(&self, rect: Rect, area: Rect) -> Placement {
        if !self.floating || area.bottom() - rect.bottom() <= SNAP_DISTANCE {
            Placement::Docked
        } else {
            Placement::Floating(Point::new(rect.x, rect.y))
        }
    }
}

/// Monitörler arası bırakma. Katman yüzeyi bir çıkışa bağlıdır (wlr-layer-shell çıkış
/// değiştiremez) ve sürükleme örtük kapmayla kaynak yüzeyde sürer; bu yüzden klavye sürüklerken
/// kaynak çıkışın kenarında kalır, bırakınca hedef çıkışta bir kez yeniden kurulur.
/// `pointer`: bırakma anındaki işaretçi (kaynak çıkış koordinatı, çıkış dışına taşabilir);
/// `outputs`: her çıkışın düzen dikdörtgeni (mantıksal) ve klavye alanı (kendi koordinatında).
/// İşaretçi başka bir çıkışın üstündeyse (hedefin sırası, oradaki yerleşim).
pub fn drop_target(drag: &Drag, pointer: Point, from: usize, outputs: &[(Rect, Rect)]) -> Option<(usize, Placement)> {
    let (origin, _) = *outputs.get(from)?; // kaynağın düzen dikdörtgeni
    let global = Point::new(origin.x + pointer.x, origin.y + pointer.y);
    let to = outputs.iter().position(|(r, _)| r.contains(global))?;
    if to == from {
        return None;
    }
    let (layout, area) = outputs[to];
    // İşaretçiye göre aynı yerde (kaynak alana çekilmeden), hedef alana çekili.
    let (dx, dy) = (pointer.x - drag.start.x, pointer.y - drag.start.y);
    let s = drag.start_rect;
    let rect = Rect::new(s.x + dx + origin.x - layout.x, s.y + dy + origin.y - layout.y, s.width, s.height);
    Some((to, drag.finish(clamp_into(area, rect), area)))
}

/// Sürüklerken (örtük kapma) yüzey içi işaretçi konumunun tabanı. Yüzey sürükleme sırasında
/// taşındığı için compositor'a bağlıdır (framework ARCHITECTURE "Sürükleme sınırı"): sway 1.12
/// basma anındaki yüzey konumunu tutar (ölçüldü); Hyprland yereli yüzeyin şimdiki
/// geometrisinden hesaplar (kaynaktan: `surfaceCoords = mouse - surfaceBox`, canlı doğrulanmadı).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GrabFrame {
    /// Yerel konum basma anındaki yüzey konumuna göre.
    #[default]
    Press,
    /// Yerel konum yüzeyin son gönderilen (commit edilmiş) konumuna göre.
    Live,
}

impl GrabFrame {
    /// Çalışılan compositor'a göre (`hyprland`: geçerli `HYPRLAND_INSTANCE_SIGNATURE`).
    pub fn for_compositor(hyprland: bool) -> Self {
        if hyprland { Self::Live } else { Self::Press }
    }

    /// Yüzey içi konum → çıkış koordinatı. `press`: basma anında yüzeyin sol üstü,
    /// `committed`: son commit'teki sol üstü.
    pub fn to_output(self, local: Point, press: Point, committed: Point) -> Point {
        let base = match self {
            Self::Press => press,
            Self::Live => committed,
        };
        Point::new(base.x + local.x, base.y + local.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grab_frame_picks_the_base_of_the_local_position() {
        let (press, committed) = (Point::new(100.0, 500.0), Point::new(160.0, 420.0));
        let local = Point::new(10.0, 5.0);
        assert_eq!(GrabFrame::Press.to_output(local, press, committed), Point::new(110.0, 505.0));
        assert_eq!(GrabFrame::Live.to_output(local, press, committed), Point::new(170.0, 425.0));
        assert_eq!(GrabFrame::for_compositor(true), GrabFrame::Live);
        assert_eq!(GrabFrame::for_compositor(false), GrabFrame::Press);
    }

    const AREA: Rect = Rect::new(6.0, 34.0, 1588.0, 860.0);
    const SIZE: Size = Size::new(916.0, 292.0);

    #[test]
    fn docked_sits_on_the_bottom_centered() {
        let r = docked_rect(AREA, SIZE);
        assert_eq!(r.bottom(), AREA.bottom());
        assert!((r.center().x - AREA.center().x).abs() < 1e-3);
        assert_eq!(rect_of(Placement::Docked, AREA, SIZE), r);
    }

    #[test]
    fn floating_is_clamped_into_area() {
        let r = rect_of(Placement::Floating(Point::new(-500.0, 5000.0)), AREA, SIZE);
        assert_eq!((r.x, r.bottom()), (AREA.x, AREA.bottom()));
        let r = rect_of(Placement::Floating(Point::new(100.0, 100.0)), AREA, SIZE);
        assert_eq!((r.x, r.y), (100.0, 100.0));
        // Alandan büyük klavye sol üste hizalanır.
        let tiny = Rect::new(0.0, 0.0, 300.0, 100.0);
        assert_eq!(clamp_into(tiny, Rect::new(50.0, 50.0, 916.0, 292.0)).x, 0.0);
    }

    #[test]
    fn undock_needs_an_upward_pull_past_the_threshold() {
        let start = docked_rect(AREA, SIZE);
        let p0 = Point::new(800.0, start.y + 8.0);
        let mut d = Drag::begin(p0, start, Placement::Docked);
        assert_eq!(d.update(Point::new(900.0, p0.y - 10.0), AREA), start, "eşik altında yerinde");
        assert_eq!(d.update(Point::new(800.0, p0.y + 40.0), AREA), start, "aşağı çekmek yüzdürmez");
        assert!(!d.is_floating());
        let r = d.update(Point::new(850.0, p0.y - 100.0), AREA);
        assert!(d.is_floating());
        assert_eq!((r.x, r.y), (start.x + 50.0, start.y - 100.0));
        // Bırakınca yüzer.
        assert_eq!(d.finish(r, AREA), Placement::Floating(Point::new(r.x, r.y)));
    }

    #[test]
    fn dropping_near_the_bottom_docks_again() {
        let start = Rect::new(200.0, 200.0, SIZE.width, SIZE.height);
        let mut d = Drag::begin(Point::new(300.0, 210.0), start, Placement::Floating(Point::new(200.0, 200.0)));
        let near = d.update(Point::new(300.0, 210.0 + (AREA.bottom() - start.bottom()) - 20.0), AREA);
        assert_eq!(d.finish(near, AREA), Placement::Docked);
        let far = d.update(Point::new(300.0, 260.0), AREA);
        assert!(matches!(d.finish(far, AREA), Placement::Floating(_)));
        // Hiç yüzmeyen yanaşık sürükleme yanaşık kalır.
        let d = Drag::begin(Point::ZERO, docked_rect(AREA, SIZE), Placement::Docked);
        assert_eq!(d.finish(docked_rect(AREA, SIZE), AREA), Placement::Docked);
    }

    #[test]
    fn dropping_on_another_output_moves_there() {
        // Kullanıcının düzeni: 1920×1080 solda, 3440×1440 sağda.
        let left = (Rect::new(0.0, 0.0, 1920.0, 1080.0), Rect::new(6.0, 34.0, 1908.0, 1040.0));
        let right = (Rect::new(1920.0, 0.0, 3440.0, 1440.0), Rect::new(6.0, 34.0, 3428.0, 1400.0));
        let outputs = [left, right];
        let start = Rect::new(200.0, 200.0, SIZE.width, SIZE.height);
        let p0 = Point::new(300.0, 210.0);
        let mut d = Drag::begin(p0, start, Placement::Floating(Point::new(200.0, 200.0)));
        // Kaynak çıkışta bırakma: hedef yok (olağan yol).
        let inside = Point::new(1500.0, 400.0);
        d.update(inside, left.1);
        assert_eq!(drop_target(&d, inside, 0, &outputs), None);
        // Sağ monitöre 2500 px: klavye işaretçiyle aynı göreli yerde, sağın koordinatında.
        let p = Point::new(p0.x + 2500.0, p0.y + 100.0);
        d.update(p, left.1);
        assert_eq!(drop_target(&d, p, 0, &outputs), Some((1, Placement::Floating(Point::new(780.0, 300.0)))));
        // Yanaşık klavye yana sürüklenip öbür monitöre bırakılınca orada yanaşık.
        let docked = docked_rect(left.1, SIZE);
        let d = Drag::begin(Point::new(960.0, 1000.0), docked, Placement::Docked);
        assert_eq!(drop_target(&d, Point::new(2400.0, 1000.0), 0, &outputs), Some((1, Placement::Docked)));
        // Sağdan sola (negatif yerel koordinat); 1020 sol alanın sağ kenarına (1914 - 916) çekilir.
        let d = Drag::begin(Point::new(100.0, 210.0), start, Placement::Floating(Point::new(200.0, 200.0)));
        let (to, at) = drop_target(&d, Point::new(-1000.0, 210.0), 1, &outputs).unwrap();
        assert_eq!((to, at), (0, Placement::Floating(Point::new(998.0, 200.0))));
        // Çıkışların dışında (boşluk) bırakma: hedef yok.
        assert_eq!(drop_target(&d, Point::new(100.0, 5000.0), 1, &outputs), None);
    }

    #[test]
    fn drag_stays_inside_area() {
        let start = Rect::new(200.0, 200.0, SIZE.width, SIZE.height);
        let mut d = Drag::begin(Point::new(300.0, 210.0), start, Placement::Floating(Point::new(200.0, 200.0)));
        let r = d.update(Point::new(-5000.0, -5000.0), AREA);
        assert_eq!((r.x, r.y), (AREA.x, AREA.y));
    }
}
