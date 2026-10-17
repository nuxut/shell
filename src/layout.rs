//! Shell geometrisi: saf fonksiyonlar, mantıksal piksel.
//!
//! Tek kaynak `ShellConfig` + output boyutu. Buradan çerçeve (dış dikdörtgen eksi içte
//! yuvarlatılmış dikdörtgen), bar şeridi ve içerik bölgeleri, dört exclusion yüzeyinin
//! exclusive zone değerleri, ana yüzeyin input bölgesi ve panellerin kapalı/açık hedef
//! dikdörtgenleri türetilir. Animasyon bu hedefler arasında yayla yapılır (runtime tarafı).

use nuxut::{BlobField, BlobShape, Color, FrameShape};
use nuxut_core::{Insets, Rect, Size};
use serde::Deserialize;

use crate::config::ShellConfig;

/// Ekran kenarı. `shell.toml`'da küçük harfle (`"top"`, `"left"`…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    #[default]
    Top,
    Right,
    Bottom,
    Left,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];

    /// Kenar yatay mı (üst/alt)? Yatay kenarda "uzunluk" x, "derinlik" y eksenidir.
    pub fn is_horizontal(self) -> bool {
        matches!(self, Edge::Top | Edge::Bottom)
    }

    pub fn of(self, insets: &Insets) -> f32 {
        match self {
            Edge::Top => insets.top,
            Edge::Right => insets.right,
            Edge::Bottom => insets.bottom,
            Edge::Left => insets.left,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Edge::Top => "top",
            Edge::Right => "right",
            Edge::Bottom => "bottom",
            Edge::Left => "left",
        }
    }
}

/// Çerçeve şekli: `outer` eksi `inner`. İç köşe kavsi temanın `blob.smoothing` değeridir
/// (ADR-0008); blob alanı uygular.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameGeometry {
    pub outer: Rect,
    pub inner: Rect,
}

impl FrameGeometry {
    /// Blob alanının çerçevesi. İç köşe kavsini alan `smoothing`den (R) alır; `inner_radius`
    /// yok sayılır (ADR-0008).
    pub fn shape(&self) -> FrameShape {
        FrameShape { outer: self.outer, inner: self.inner, inner_radius: 0.0 }
    }
}

/// Bu kadarın altındaki kenar/kavis "yok" sayılır (çizimdeki çerçeve kuralıyla aynı).
const FRAME_EPSILON: f32 = 0.01;

/// Dört exclusion yüzeyinin exclusive zone değerleri (tam sayı, yukarı yuvarlanmış: pencere
/// çerçevenin altına asla girmez).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExclusiveZones {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl ExclusiveZones {
    pub fn get(&self, edge: Edge) -> i32 {
        match edge {
            Edge::Top => self.top,
            Edge::Right => self.right,
            Edge::Bottom => self.bottom,
            Edge::Left => self.left,
        }
    }
}

/// Bar'ın üç bölümünün yerleşimi. Yatay bar'da start = sol, dikeyde start = üst.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarSlots {
    pub start: Rect,
    pub center: Rect,
    pub end: Rect,
}

/// Panelin kenar boyunca hizası.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelAlign {
    Start,
    Center,
    End,
    /// Panel merkezi bu eksen koordinatında (ör. tetikleyen widget'ın merkezi). İç alana
    /// sığacak şekilde kaydırılır.
    At(f32),
}

/// Bir kenardan büyüyen panelin tanımı.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelSpec {
    pub edge: Edge,
    pub align: PanelAlign,
    /// Kenar boyunca uzunluk.
    pub length: f32,
    /// Kenardan içeri doğru derinlik (açıkken).
    pub depth: f32,
}

/// Panelin animasyon uçları. `closed` kenara yapışık ve derinliği 0; `open` aynı konumda
/// tam derinlikte. İkisi de iç alanın içindedir.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelGeometry {
    pub closed: Rect,
    pub open: Rect,
}

/// Bir output için hesaplanmış shell geometrisi.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShellLayout {
    pub output: Size,
    pub insets: Insets,
    pub frame: FrameGeometry,
    pub bar_edge: Edge,
    /// Bar şeridinin tamamı (çerçevenin kalın kenarı).
    pub bar: Rect,
    /// Bar içeriği: bar boyunca iç alanla hizalı, uçlarda `bar.padding` kadar içeride.
    pub bar_content: Rect,
}

impl ShellLayout {
    pub fn compute(config: &ShellConfig, output: Size) -> Self {
        Self::from_parts(output, config.frame_insets(), config.bar.edge, config.bar.padding)
    }

    /// Animasyonun ara değerlerinden (yaydaki insets) geometri kurar.
    pub fn from_parts(output: Size, insets: Insets, bar_edge: Edge, bar_padding: f32) -> Self {
        let output = Size::new(sane(output.width), sane(output.height));
        let insets = Insets::new(sane(insets.top), sane(insets.right), sane(insets.bottom), sane(insets.left));
        let insets = fit_insets(insets, output);
        let outer = Rect::new(0.0, 0.0, output.width, output.height);
        let inner = outer.inset(insets);
        let frame = FrameGeometry { outer, inner };

        // Dikey bar'ın şeridi tam yükseklikte olsun (üst/alt şeritleri köşelerden çıkar):
        // içerik bar boyunca iç alanla hizalanır.
        let bar = edge_strip(outer, &insets, bar_edge);
        let bar_content = if bar_edge.is_horizontal() {
            let p = sane(bar_padding).min(inner.width * 0.5);
            Rect::new(inner.x + p, bar.y, (inner.width - 2.0 * p).max(0.0), bar.height)
        } else {
            let p = sane(bar_padding).min(inner.height * 0.5);
            Rect::new(bar.x, inner.y + p, bar.width, (inner.height - 2.0 * p).max(0.0))
        };

        Self { output, insets, frame, bar_edge, bar, bar_content }
    }

    pub fn exclusive_zones(&self) -> ExclusiveZones {
        let z = |v: f32| v.ceil() as i32;
        ExclusiveZones {
            top: z(self.insets.top),
            right: z(self.insets.right),
            bottom: z(self.insets.bottom),
            left: z(self.insets.left),
        }
    }

    /// Çerçevenin dört şeridi (boş olanlar hariç). Üst/alt şeritler köşeleri kapsar,
    /// sol/sağ şeritler aralarında kalır; örtüşme yoktur.
    pub fn frame_strips(&self) -> Vec<Rect> {
        Edge::ALL
            .into_iter()
            .map(|e| edge_strip(self.frame.outer, &self.insets, e))
            .filter(|r| !r.is_empty())
            .collect()
    }

    /// Girdi bölgesinin blob alanı: çizilen alanın oturmuş (hedef) hâli — bu geometrinin
    /// çerçevesi + açık yaprakların hedef dikdörtgenleri, deformsuz; `smoothing` (R) ve
    /// `corner_radius` (r) çizimdekiyle aynı. Renk/gölge girdiyi etkilemez, sabittir; böylece
    /// alan eşitliği bölge önbelleğinin anahtarı olur. Çerçeve kuralı çizimdekiyle aynı:
    /// kenarlar ve R sıfırsa (tam ekran) çerçeve yok.
    pub fn input_field(&self, sheets: &[Rect], smoothing: f32, corner_radius: f32) -> BlobField {
        let smoothing = sane(smoothing);
        let i = self.insets;
        let has_frame = i.top.max(i.right).max(i.bottom).max(i.left) > FRAME_EPSILON || smoothing > FRAME_EPSILON;
        BlobField {
            frame: has_frame.then(|| self.frame.shape()),
            shapes: sheets.iter().filter(|r| !r.is_empty()).map(|&r| BlobShape::new_rect(r)).collect(),
            smoothing,
            corner_radius: sane(corner_radius),
            fill: Color::TRANSPARENT,
            shadow: None,
        }
    }

    /// Ana yüzeyin input bölgesi: blob alanının dolu bölgesi (`BlobField::input_region`; çerçeve,
    /// yapraklar ve birleşim filetoları — boşluğa açılan `r` köşelerinin dışı değil) + blob dışı
    /// etkileşimli dikdörtgenler (`extra`: overlay menüleri, bildirim popup'ları). Hepsi tam
    /// piksele dışa yuvarlanır ve output'a kırpılır; boşlar atılır.
    pub fn input_region(&self, blob: &[Rect], extra: &[Rect]) -> Vec<Rect> {
        let bounds = self.frame.outer;
        blob.iter()
            .chain(extra)
            .map(|r| r.round_out().intersect(&bounds))
            .filter(|r| !r.is_empty())
            .collect()
    }

    /// Bar içerik alanına üç bölümü bar ekseni boyunca yerleştirir (yatay: x, dikey: y).
    /// Start ve end uçlara yaslanır (önce start, sonra end yer alır); center ortalanır ama
    /// yanlara `gap`'ten fazla yaklaşmaz, yer yetmezse aradaki boşluğa sıkıştırılır.
    /// Uzunluklar bar ekseni boyuncadır.
    pub fn bar_slots(&self, start: f32, center: f32, end: f32, gap: f32) -> BarSlots {
        let c = self.bar_content;
        let horizontal = self.bar_edge.is_horizontal();
        let (c0, len) = if horizontal { (c.x, c.width) } else { (c.y, c.height) };
        let (start, center, end, gap) = (sane(start), sane(center), sane(end), sane(gap));
        let sw = start.min(len);
        let ew = end.min(len - sw);
        let (s0, e0) = (c0, c0 + len - ew);

        let mut lo = if sw > 0.0 { s0 + sw + gap } else { c0 };
        let mut hi = if ew > 0.0 { e0 - gap } else { c0 + len };
        if hi < lo {
            // Boşluklar bile sığmıyor: orta bölüm ikisinin arasında sıfır uzunlukta.
            lo = (s0 + sw + e0) * 0.5;
            hi = lo;
        }
        let cw = center.min((hi - lo).max(0.0));
        let m0 = (c0 + len * 0.5 - cw * 0.5).clamp(lo, (hi - cw).max(lo));

        let span = |p: f32, l: f32| if horizontal { Rect::new(p, c.y, l, c.height) } else { Rect::new(c.x, p, c.width, l) };
        BarSlots { start: span(s0, sw), center: span(m0, cw), end: span(e0, ew) }
    }

    /// Panelin kapalı ve açık dikdörtgeni. Panel çerçevenin iç kenarına yapışıktır; boyutu ve
    /// konumu iç alana sığacak şekilde sınırlanır.
    pub fn panel(&self, spec: &PanelSpec) -> PanelGeometry {
        let inner = self.frame.inner;
        let horizontal = spec.edge.is_horizontal();
        let (axis_start, axis_len, depth_max) =
            if horizontal { (inner.x, inner.width, inner.height) } else { (inner.y, inner.height, inner.width) };

        let length = sane(spec.length).min(axis_len);
        let depth = sane(spec.depth).min(depth_max);
        let free = axis_len - length;
        let offset = match spec.align {
            PanelAlign::Start => 0.0,
            PanelAlign::Center => free * 0.5,
            PanelAlign::End => free,
            PanelAlign::At(center) => {
                let c = if center.is_finite() { center } else { axis_start + axis_len * 0.5 };
                (c - length * 0.5 - axis_start).clamp(0.0, free)
            }
        };
        let pos = axis_start + offset;

        let rect = |d: f32| match spec.edge {
            Edge::Top => Rect::new(pos, inner.y, length, d),
            Edge::Bottom => Rect::new(pos, inner.bottom() - d, length, d),
            Edge::Left => Rect::new(inner.x, pos, d, length),
            Edge::Right => Rect::new(inner.right() - d, pos, d, length),
        };
        PanelGeometry { closed: rect(0.0), open: rect(depth) }
    }
}

/// Bir kenarın şeridi. Üst/alt tam genişlik, sol/sağ üst ve alt şeritlerin arasında.
fn edge_strip(outer: Rect, insets: &Insets, edge: Edge) -> Rect {
    let middle_h = (outer.height - insets.top - insets.bottom).max(0.0);
    match edge {
        Edge::Top => Rect::new(outer.x, outer.y, outer.width, insets.top),
        Edge::Bottom => Rect::new(outer.x, outer.bottom() - insets.bottom, outer.width, insets.bottom),
        Edge::Left => Rect::new(outer.x, outer.y + insets.top, insets.left, middle_h),
        Edge::Right => Rect::new(outer.right() - insets.right, outer.y + insets.top, insets.right, middle_h),
    }
}

/// Kenarlar output'a sığmıyorsa (küçük/garip output) karşılıklı kenarları orantılı küçültür.
fn fit_insets(mut insets: Insets, output: Size) -> Insets {
    let fit = |a: &mut f32, b: &mut f32, total: f32| {
        let sum = *a + *b;
        if sum > total && sum > 0.0 {
            let k = total / sum;
            *a *= k;
            *b *= k;
        }
    };
    fit(&mut insets.left, &mut insets.right, output.width);
    fit(&mut insets.top, &mut insets.bottom, output.height);
    insets
}

/// Negatif ve sayı olmayan boyutları 0'a çeker.
fn sane(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FHD: Size = Size::new(1920.0, 1080.0);

    fn layout(text: &str) -> ShellLayout {
        let (c, _) = ShellConfig::parse(text).unwrap();
        ShellLayout::compute(&c, FHD)
    }

    #[test]
    fn default_frame_geometry() {
        let l = layout("");
        assert_eq!(l.insets, Insets::new(34.0, 6.0, 6.0, 6.0));
        assert_eq!(l.frame.outer, Rect::new(0.0, 0.0, 1920.0, 1080.0));
        assert_eq!(l.frame.inner, Rect::new(6.0, 34.0, 1908.0, 1040.0));
        assert_eq!(l.bar_edge, Edge::Top);
        assert_eq!(l.bar, Rect::new(0.0, 0.0, 1920.0, 34.0));
        assert_eq!(l.bar_content, Rect::new(16.0, 0.0, 1888.0, 34.0));
    }

    #[test]
    fn bottom_bar_geometry() {
        let l = layout("[bar]\nedge = \"bottom\"\nsize = 40\n");
        assert_eq!(l.bar, Rect::new(0.0, 1040.0, 1920.0, 40.0));
        assert_eq!(l.frame.inner, Rect::new(6.0, 6.0, 1908.0, 1034.0));
        assert_eq!(l.bar_content.y, 1040.0);
    }

    #[test]
    fn exclusive_zones_round_up() {
        let l = layout("[frame]\nthickness = 4.2\nright = 0\n");
        assert_eq!(l.exclusive_zones(), ExclusiveZones { top: 34, right: 0, bottom: 5, left: 5 });
        assert_eq!(l.exclusive_zones().get(Edge::Left), 5);
    }

    #[test]
    fn frame_strips_tile_the_frame_without_overlap() {
        let l = layout("");
        let strips = l.frame_strips();
        assert_eq!(strips.len(), 4);
        let area: f32 = strips.iter().map(|r| r.width * r.height).sum();
        let expected = 1920.0 * 1080.0 - l.frame.inner.width * l.frame.inner.height;
        assert_eq!(area, expected);
        for (i, a) in strips.iter().enumerate() {
            for b in &strips[i + 1..] {
                assert!(a.intersect(b).is_empty(), "{a:?} ∩ {b:?}");
            }
        }
    }

    #[test]
    fn zero_edges_have_no_strip() {
        let l = layout("[frame]\nenabled = false\n");
        assert_eq!(l.frame_strips(), [Rect::new(0.0, 0.0, 1920.0, 34.0)]);
        assert_eq!(l.exclusive_zones(), ExclusiveZones { top: 34, ..Default::default() });
    }

    #[test]
    fn tiny_or_broken_output_never_produces_negative_geometry() {
        let c = ShellConfig::default();
        for size in [Size::new(10.0, 20.0), Size::ZERO, Size::new(f32::NAN, -5.0)] {
            let l = ShellLayout::compute(&c, size);
            assert!(l.insets.horizontal() <= l.output.width + 1e-3, "{l:?}");
            assert!(l.insets.vertical() <= l.output.height + 1e-3, "{l:?}");
            assert!(l.frame.inner.width >= 0.0 && l.frame.inner.height >= 0.0);
            assert!(l.bar_content.width >= 0.0);
            let blob = l.input_field(&[], 20.0, 12.0).input_region();
            for r in l.input_region(&blob, &[]) {
                assert!(!r.is_empty());
            }
        }
    }

    #[test]
    fn input_region_is_pixel_aligned_and_clipped() {
        // Bölge blob'un gerçek şeklinden: çerçeve + yaprak + birleşim filetoları; boşluğa açılan
        // r köşesinin dışı ve iç alan değil. R = 20, r = 12 (tema varsayılanına yakın).
        let l = layout("[frame]\nthickness = 3.5\n");
        let panel = Rect::new(100.0, 34.0, 400.0, 250.0); // üst bar'dan inen yaprak
        let offscreen = Rect::new(5000.0, 0.0, 10.0, 10.0);
        let blob = l.input_field(&[panel, offscreen, Rect::ZERO], 20.0, 12.0).input_region();
        let overlay = Rect::new(600.3, 40.0, 50.0, 20.2);
        let region = l.input_region(&blob, &[overlay, offscreen]);
        assert!(!region.is_empty());
        for r in &region {
            assert_eq!(*r, r.round_out(), "{r:?}");
            assert_eq!(r.intersect(&l.frame.outer), *r, "output'a kırpılı: {r:?}");
            assert!(!r.is_empty());
        }
        // Piksel merkezi bölgede mi.
        let hit = |x: f32, y: f32| region.iter().any(|r| x >= r.x && x < r.right() && y >= r.y && y < r.bottom());
        assert!(hit(300.5, 150.5), "yaprağın içi");
        assert!(hit(1.5, 500.5) && hit(960.5, 10.5), "çerçeve şeritleri (sol 3.5 px, bar)");
        assert!(hit(95.5, 36.5), "yaprak ile çerçeve arasındaki fileto bölgede");
        assert!(hit(4.5, 35.5), "çerçevenin iç köşe kavsi bölgede");
        assert!(!hit(100.5, 283.5), "boşluğa açılan r köşesinin dışı tıklamayı geçirir");
        assert!(!hit(498.5, 283.5), "sağ alt r köşesi de");
        assert!(hit(300.5, 283.5), "alt kenar boyunca dolu");
        assert!(!hit(900.5, 600.5), "iç alan geçirgen");
        assert!(region.contains(&Rect::new(600.0, 40.0, 51.0, 21.0)), "blob dışı dikdörtgen dışa yuvarlanır");
    }

    #[test]
    fn bar_slots_center_and_edges() {
        let l = layout("");
        let s = l.bar_slots(200.0, 100.0, 150.0, 8.0);
        assert_eq!(s.start, Rect::new(16.0, 0.0, 200.0, 34.0));
        assert_eq!(s.end, Rect::new(1904.0 - 150.0, 0.0, 150.0, 34.0));
        assert_eq!(s.center, Rect::new(960.0 - 50.0, 0.0, 100.0, 34.0));
    }

    #[test]
    fn bar_center_is_pushed_away_from_wide_start() {
        let l = layout("");
        let s = l.bar_slots(1000.0, 200.0, 0.0, 8.0);
        assert_eq!(s.center.x, 16.0 + 1000.0 + 8.0);
        assert_eq!(s.end.width, 0.0);
    }

    #[test]
    fn bar_slots_squeeze_when_too_wide() {
        let l = layout("");
        let s = l.bar_slots(1500.0, 800.0, 1500.0, 8.0);
        assert_eq!(s.start.width, 1500.0);
        assert_eq!(s.end.width, 388.0);
        assert_eq!(s.center.width, 0.0);
        assert!(s.center.x >= s.start.right() && s.center.right() <= s.end.x);
    }

    #[test]
    fn vertical_bar_geometry_and_slots() {
        for (edge, bar_x) in [("left", 0.0), ("right", 1920.0 - 40.0)] {
            let l = layout(&format!("[bar]\nedge = \"{edge}\"\nsize = 40\npadding = 10\n"));
            assert_eq!(l.bar.x, bar_x, "{edge}");
            assert_eq!(l.bar.width, 40.0);
            // içerik iç alanın yüksekliği boyunca, uçlarda padding kadar içeride
            assert_eq!(l.bar_content, Rect::new(bar_x, 6.0 + 10.0, 40.0, 1080.0 - 12.0 - 20.0));
            let s = l.bar_slots(200.0, 60.0, 100.0, 8.0);
            assert_eq!(s.start, Rect::new(bar_x, 16.0, 40.0, 200.0));
            assert_eq!(s.end, Rect::new(bar_x, 1064.0 - 100.0, 40.0, 100.0));
            assert_eq!(s.center, Rect::new(bar_x, 540.0 - 30.0, 40.0, 60.0));
        }
    }

    #[test]
    fn panel_on_vertical_bar_grows_sideways() {
        let l = layout("[bar]\nedge = \"left\"\n");
        let p = l.panel(&PanelSpec { edge: Edge::Left, align: PanelAlign::Start, length: 1000.0, depth: 300.0 });
        assert_eq!(p.open.x, 34.0);
        assert_eq!(p.open.width, 300.0);
        assert_eq!(p.closed.width, 0.0);
    }

    #[test]
    fn top_panel_grows_from_bar() {
        let l = layout("");
        let p = l.panel(&PanelSpec { edge: Edge::Top, align: PanelAlign::Center, length: 400.0, depth: 300.0 });
        assert_eq!(p.closed, Rect::new(6.0 + 754.0, 34.0, 400.0, 0.0));
        assert_eq!(p.open, Rect::new(760.0, 34.0, 400.0, 300.0));
    }

    #[test]
    fn panels_on_every_edge_stick_to_inner_rect() {
        let l = layout("");
        let inner = l.frame.inner;
        for edge in Edge::ALL {
            let p = l.panel(&PanelSpec { edge, align: PanelAlign::Start, length: 200.0, depth: 100.0 });
            let (o, c) = (p.open, p.closed);
            assert_eq!(inner.intersect(&o), o, "{edge:?}");
            match edge {
                Edge::Top => assert!(o.y == inner.y && c.height == 0.0 && c.y == inner.y),
                Edge::Bottom => assert!(o.bottom() == inner.bottom() && c.height == 0.0 && c.y == inner.bottom()),
                Edge::Left => assert!(o.x == inner.x && c.width == 0.0 && c.x == inner.x),
                Edge::Right => assert!(o.right() == inner.right() && c.width == 0.0 && c.x == inner.right()),
            }
            assert_eq!(o.width.max(o.height), 200.0);
        }
    }

    #[test]
    fn panel_at_position_is_clamped_inside() {
        let l = layout("");
        let spec = |c| PanelSpec { edge: Edge::Top, align: PanelAlign::At(c), length: 300.0, depth: 200.0 };
        assert_eq!(l.panel(&spec(500.0)).open.x, 350.0);
        assert_eq!(l.panel(&spec(0.0)).open.x, 6.0);
        assert_eq!(l.panel(&spec(1919.0)).open.right(), 1914.0);
        assert_eq!(l.panel(&spec(f32::NAN)).open.center().x, 960.0);
    }

    #[test]
    fn oversized_panel_is_limited_to_inner_rect() {
        let l = layout("");
        let p = l.panel(&PanelSpec { edge: Edge::Right, align: PanelAlign::End, length: 5000.0, depth: 5000.0 });
        assert_eq!(p.open, l.frame.inner);
    }
}
