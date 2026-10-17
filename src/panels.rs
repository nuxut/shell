//! Yaprakların (saat popout'u, kenar panelleri) açık/sabit durumu, sabit panellerin yer
//! paylaşımı ve sabitleme düğmesinin köşesi: saf mantık, birim testli.
//!
//! Kurallar `docs/design/panels.md`: en fazla bir **geçici** yaprak (popout ya da sabitlenmemiş
//! kenar paneli) ve kenar başına en fazla bir **sabit** panel. Örtük kapatma (Escape, dışarı
//! tıklama, odak kaybı) yalnızca geçici yaprağı kapatır; açık kapatma (aynı düğme, komut)
//! hangisi olursa olsun kapatır ve sabitlemeyi düşürür.

use nuxut_core::{Rect, Size};

use crate::layout::Edge;
use crate::widgets::PanelId;

/// Bir durum değişikliğinin sonucu: açılan ve kapanan yapraklar (çağıran yaprak animasyonlarını
/// ve açılış/kapanış kancalarını buna göre uygular).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transition {
    pub opened: Vec<PanelId>,
    pub closed: Vec<PanelId>,
}

impl Transition {
    fn closed(p: PanelId) -> Self {
        Self { opened: Vec::new(), closed: vec![p] }
    }

    pub fn is_empty(&self) -> bool {
        self.opened.is_empty() && self.closed.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PanelState {
    transient: Option<PanelId>,
    /// Sabitleme sırasıyla (yer paylaşımı bu sırayla yapılır).
    pinned: Vec<Edge>,
}

impl PanelState {
    /// Geçici yaprak (en üstte; klavye ve örtük kapatma onun).
    pub fn transient(&self) -> Option<PanelId> {
        self.transient
    }

    /// Sabit paneller, sabitleme sırasıyla.
    pub fn pinned(&self) -> &[Edge] {
        &self.pinned
    }

    pub fn is_pinned(&self, edge: Edge) -> bool {
        self.pinned.contains(&edge)
    }

    pub fn is_open(&self, p: PanelId) -> bool {
        self.transient == Some(p) || matches!(p, PanelId::Edge(e) if self.is_pinned(e))
    }

    pub fn any_open(&self) -> bool {
        self.transient.is_some() || !self.pinned.is_empty()
    }

    /// Açık yapraklar üstten alta: geçici, sonra en son sabitlenenden başlayarak sabitler
    /// (işaretçi ve klavye bu sırayla dağıtılır).
    pub fn open_panels(&self) -> impl Iterator<Item = PanelId> + '_ {
        self.transient.into_iter().chain(self.pinned.iter().rev().map(|&e| PanelId::Edge(e)))
    }

    /// Açar (zaten açıksa değişiklik yok). Önceki geçici yaprak kapanır; sabitler kalır.
    pub fn open(&mut self, p: PanelId) -> Transition {
        if self.is_open(p) {
            return Transition::default();
        }
        let closed = self.transient.replace(p).into_iter().collect();
        Transition { opened: vec![p], closed }
    }

    pub fn toggle(&mut self, p: PanelId) -> Transition {
        if self.is_open(p) { self.close(p) } else { self.open(p) }
    }

    /// Açık kapatma (aynı düğme, komut, kenar devre dışı): sabit de olsa kapanır, sabitleme düşer.
    pub fn close(&mut self, p: PanelId) -> Transition {
        if self.transient == Some(p) {
            self.transient = None;
            return Transition::closed(p);
        }
        if let PanelId::Edge(e) = p
            && let Some(i) = self.pinned.iter().position(|&x| x == e)
        {
            self.pinned.remove(i);
            return Transition::closed(p);
        }
        Transition::default()
    }

    /// Örtük kapatma (Escape, dışarı tıklama, odak kaybı): yalnızca geçici yaprak.
    pub fn dismiss(&mut self) -> Transition {
        self.transient.take().map(Transition::closed).unwrap_or_default()
    }

    /// Hepsi (`panel close-all`, bar gizlendi).
    pub fn close_all(&mut self) -> Transition {
        let closed = self.open_panels().collect();
        self.transient = None;
        self.pinned.clear();
        Transition { opened: Vec::new(), closed }
    }

    /// Sabitleme. `true`: kapalı panel açılır ve sabitlenir (geçici yaprağa dokunulmaz),
    /// geçici panel sabite geçer. `false`: panel açık kalır ve geçici olur; başka bir geçici
    /// yaprak varsa o kapanır.
    pub fn set_pinned(&mut self, edge: Edge, pinned: bool) -> Transition {
        let p = PanelId::Edge(edge);
        if pinned == self.is_pinned(edge) {
            return Transition::default();
        }
        if pinned {
            self.pinned.push(edge);
            if self.transient == Some(p) {
                self.transient = None;
                return Transition::default();
            }
            return Transition { opened: vec![p], closed: Vec::new() };
        }
        self.pinned.retain(|&e| e != edge);
        let closed = self.transient.replace(p).into_iter().collect();
        Transition { opened: Vec::new(), closed }
    }

    /// "Tek geçici yüzey" (karar: 2026-10-13): bar'dan bir menü (tepsi menüsü,
    /// uygulamanın bağlam menüsü) açılırken kapanacak yaprak — geçici yaprak; sabit panel kalır
    /// (menü onun alanını [`free_area`] ile dolu sayar). Popout'lar (`Clock`, `Sysmon`) zaten
    /// yaprak oldukları için [`PanelState::open`] öncekini kapatır. Kapanış çağıranın
    /// `close` yoluyla, menü açılışı beklenmeden yapılır.
    pub fn closed_by_bar_menu(&self) -> Option<PanelId> {
        self.transient
    }

    /// Odak kapma gerekli mi: yalnızca geçici yaprak açıkken. Sabit panel grab tutmaz (dışarı
    /// tıklama onu kapatmamalı); sabit + geçici açıkken grab yalnızca geçiciyi kapsar (tek ana
    /// yüzey: liste aynı yüzeydir, sabit panele tıklama grab'ı bitirmez).
    pub fn wants_focus_grab(&self) -> bool {
        self.transient.is_some()
    }
}

/// Odak kapma (`hyprland_focus_grab_v1`) için bir sonraki adım.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrabStep {
    /// Değişiklik yok.
    Keep,
    /// Grab kur.
    Acquire,
    /// Grab'ı bırak (`cleared` gelmez; kapanış çift tetiklenmez).
    Release,
    /// Bırak ve yeniden kur: klavye `Exclusive`'ten `OnDemand`'e indi (önce mod, sonra grab;
    /// Hyprland'de follow-mouse odağı yarışını önler — noctalia ile aynı sıra).
    Reacquire,
}

/// Grab kararı: `want` = geçici yaprak açık ([`PanelState::wants_focus_grab`]), `held` = elde
/// grab var, `to_on_demand` = bu adımda klavye `Exclusive` → `OnDemand` indi. Protokol yoksa
/// (sway) çağıran grab kuramaz; davranış odak kaybı yoluyla sürer.
pub fn focus_grab_step(want: bool, held: bool, to_on_demand: bool) -> GrabStep {
    match (want, held) {
        (true, false) => GrabStep::Acquire,
        (true, true) if to_on_demand => GrabStep::Reacquire,
        (false, true) => GrabStep::Release,
        _ => GrabStep::Keep,
    }
}

/// `area`'dan `edge` kenarında `depth` kalınlığında şeridi çıkarır (kalan alan; negatif olmaz).
pub fn carve(area: Rect, edge: Edge, depth: f32) -> Rect {
    let d = if depth.is_finite() { depth.max(0.0) } else { 0.0 };
    match edge {
        Edge::Top => {
            let d = d.min(area.height);
            Rect::new(area.x, area.y + d, area.width, area.height - d)
        }
        Edge::Bottom => Rect::new(area.x, area.y, area.width, (area.height - d).max(0.0)),
        Edge::Left => {
            let d = d.min(area.width);
            Rect::new(area.x + d, area.y, area.width - d, area.height)
        }
        Edge::Right => Rect::new(area.x, area.y, (area.width - d).max(0.0), area.height),
    }
}

/// Sabit panellerin yer paylaşımı. `pinned`: sabitleme sırasıyla (kenar, istenen derinlik).
/// Döner: her sabit panelin yerleşeceği alan (kendinden öncekilerin kalanı) ve geçici yaprağın
/// alanı (hepsinin kalanı).
pub fn share_area(inner: Rect, pinned: &[(Edge, f32)]) -> (Vec<Rect>, Rect) {
    let mut rest = inner;
    let mut areas = Vec::with_capacity(pinned.len());
    for &(edge, depth) in pinned {
        areas.push(rest);
        rest = carve(rest, edge, depth);
    }
    (areas, rest)
}

/// Bar'dan açılan her şeyin (popout'lar, tepsi menüsü, ipucu) ortak yerleşim alanı: iç alan
/// eksi görünen sabit paneller. Popout'lar bu alanda yaprak olarak, menü/ipucu bunu sınır
/// (`WidgetCx::bounds`) alıp içinde kaydırılarak/yön çevrilerek yerleşir; sabit panelle
/// asla üst üste binmez (`docs/design/panels.md` §5).
pub fn free_area(inner: Rect, pinned: &[(Edge, f32)]) -> Rect {
    share_area(inner, pinned).1
}

/// Sabitleme düğmesinin köşesi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Corner {
    pub right: bool,
    pub bottom: bool,
}

/// Panelin çıktığı kenara yakın köşe; diğer eksende bar'a yakın taraf (bar o eksende değilse:
/// sol/sağ panelde üst, üst/alt panelde sağ = end).
pub fn pin_corner(panel: Edge, bar: Edge) -> Corner {
    match panel {
        Edge::Left | Edge::Right => Corner { right: panel == Edge::Right, bottom: bar == Edge::Bottom },
        Edge::Top | Edge::Bottom => Corner { right: bar != Edge::Left, bottom: panel == Edge::Bottom },
    }
}

/// Sabitleme düğmesinin dikdörtgeni: panelin içerik dikdörtgeninin (`area`; panel iç boşluğu
/// uygulanmadan) `corner` köşesinde `size` kenarlı kare. İçerik için yer ayrılmaz: düğme
/// içeriğin üstünde yüzer, kartlar onun altından akar. Çakışmasızlık yatay oluktan gelir:
/// panel iç boşluğu + kart sütunu boşluğu + kartın kendi iç boşluğu ≥ `size` olduğundan isabet
/// alanı yalnızca kart yüzeyinin köşe boşluğuna biner, kart içeriğine değmez (`screen.rs`
/// `PIN_SIZE_GRID` ve testi).
pub fn pin_rect(area: Rect, corner: Corner, size: f32) -> Rect {
    let size = size.max(0.0).min(area.width.max(0.0)).min(area.height.max(0.0));
    let x = if corner.right { area.right() - size } else { area.x };
    let y = if corner.bottom { area.bottom() - size } else { area.y };
    Rect::new(x, y, size, size)
}

/// Pin düğmesinin altında kalan widget'ın içeriğe değmemesi için panelin o köşede içeriği ne
/// kadar itmesi gerektiği (düğmenin köşesinin y tarafında; 0 = gerek yok). `clearance`: widget'ın
/// köşesinden içeriğine yatay/dikey boşluk ([`crate::widgets::PanelWidget::corner_clearance`]).
/// Düğme yatayda boşluğa sığıyorsa (kart sütunu oluğu) itme yok; sığmıyorsa düğmenin dikey
/// taşması boşluğa inene kadar itilir.
pub fn pin_clearance_shift(pin: Rect, corner: Corner, widget: Rect, clearance: Size) -> f32 {
    let overlaps = pin.x < widget.right() && widget.x < pin.right() && pin.y < widget.bottom() && widget.y < pin.bottom();
    if !overlaps || pin.is_empty() || widget.is_empty() {
        return 0.0;
    }
    let ox = if corner.right { widget.right() - pin.x } else { pin.right() - widget.x };
    let oy = if corner.bottom { widget.bottom() - pin.y } else { pin.bottom() - widget.y };
    if ox <= clearance.width.max(0.0) + 0.01 {
        return 0.0;
    }
    (oy - clearance.height.max(0.0)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: PanelId = PanelId::Edge(Edge::Right);
    const L: PanelId = PanelId::Edge(Edge::Left);
    const T: PanelId = PanelId::Edge(Edge::Top);
    const C: PanelId = PanelId::Clock;

    fn tr(opened: &[PanelId], closed: &[PanelId]) -> Transition {
        Transition { opened: opened.to_vec(), closed: closed.to_vec() }
    }

    #[test]
    fn single_transient_replaces_previous() {
        let mut s = PanelState::default();
        assert_eq!(s.open(R), tr(&[R], &[]));
        assert_eq!(s.open(R), tr(&[], &[]), "zaten açık");
        assert_eq!(s.open(C), tr(&[C], &[R]));
        assert_eq!(s.toggle(C), tr(&[], &[C]));
        assert!(!s.any_open());
    }

    #[test]
    fn pinned_survives_implicit_closes_and_other_panels() {
        let mut s = PanelState::default();
        s.open(R);
        assert_eq!(s.set_pinned(Edge::Right, true), tr(&[], &[]), "geçici → sabit, animasyon yok");
        assert_eq!(s.transient(), None);
        assert_eq!(s.dismiss(), tr(&[], &[]), "Escape/odak kaybı sabiti kapatmaz");
        assert_eq!(s.open(C), tr(&[C], &[]), "başka yaprak sabiti kapatmaz");
        assert!(s.is_open(R) && s.is_open(C));
        assert_eq!(s.open_panels().collect::<Vec<_>>(), [C, R], "geçici en üstte");
        assert_eq!(s.dismiss(), tr(&[], &[C]));
        assert!(s.is_open(R));
    }

    /// Tek geçici yüzey (1): bar menüsü açılınca sabitsiz panel (ve popout) kapanır, sabit kalır;
    /// bar popout'u açılınca da önceki geçici kapanır.
    #[test]
    fn bar_menu_closes_unpinned_panel_only() {
        let mut s = PanelState::default();
        assert_eq!(s.closed_by_bar_menu(), None, "açık yaprak yok");
        s.open(R);
        assert_eq!(s.closed_by_bar_menu(), Some(R), "sabitsiz sağ panel kapanır");
        assert_eq!(s.close(R), tr(&[], &[R]));
        s.set_pinned(Edge::Right, true);
        assert_eq!(s.closed_by_bar_menu(), None, "sabit panel kalır");
        s.open(L);
        assert_eq!(s.closed_by_bar_menu(), Some(L), "sabit + geçici: yalnızca geçici");
        assert_eq!(s.open(C), tr(&[C], &[L]), "saat popout'u geçici paneli kapatır");
        assert_eq!(s.open(PanelId::Sysmon), tr(&[PanelId::Sysmon], &[C]), "popout popout'u kapatır");
        assert_eq!(s.closed_by_bar_menu(), Some(PanelId::Sysmon));
        assert!(s.is_open(R));
    }

    #[test]
    fn free_area_excludes_every_pinned_panel() {
        let inner = Rect::new(0.0, 30.0, 1920.0, 1050.0);
        assert_eq!(free_area(inner, &[]), inner);
        assert_eq!(free_area(inner, &[(Edge::Right, 400.0)]), Rect::new(0.0, 30.0, 1520.0, 1050.0));
        assert_eq!(
            free_area(inner, &[(Edge::Right, 400.0), (Edge::Left, 300.0)]),
            Rect::new(300.0, 30.0, 1220.0, 1050.0)
        );
        // Dar çıkış: panel iç alandan derin → serbest alan boş, negatif değil.
        let a = free_area(Rect::new(0.0, 0.0, 300.0, 500.0), &[(Edge::Left, 400.0)]);
        assert_eq!(a.width, 0.0);
    }

    #[test]
    fn launcher_is_transient_over_pinned() {
        const LA: PanelId = PanelId::Launcher;
        let mut s = PanelState::default();
        s.set_pinned(Edge::Right, true);
        s.open(C);
        assert_eq!(s.open(LA), tr(&[LA], &[C]), "başlatıcı geçici yaprağın yerini alır");
        assert_eq!(s.open_panels().collect::<Vec<_>>(), [LA, R], "başlatıcı sabitin üstünde");
        assert_eq!(s.dismiss(), tr(&[], &[LA]), "Escape/dışarı tıklama yalnızca başlatıcıyı kapatır");
        assert!(s.is_open(R));
    }

    #[test]
    fn explicit_close_unpins() {
        let mut s = PanelState::default();
        s.set_pinned(Edge::Right, true);
        assert!(s.is_open(R));
        assert_eq!(s.toggle(R), tr(&[], &[R]), "aynı düğme sabiti de kapatır");
        assert!(!s.is_pinned(Edge::Right));
        assert_eq!(s.open(R), tr(&[R], &[]));
        assert!(!s.is_pinned(Edge::Right), "sonraki açılış sabitsiz");
    }

    #[test]
    fn pin_on_closed_opens_without_touching_transient() {
        let mut s = PanelState::default();
        s.open(C);
        assert_eq!(s.set_pinned(Edge::Left, true), tr(&[L], &[]));
        assert!(s.is_open(C) && s.is_open(L));
        assert_eq!(s.set_pinned(Edge::Left, true), tr(&[], &[]), "zaten sabit");
    }

    #[test]
    fn unpin_keeps_panel_and_replaces_transient() {
        let mut s = PanelState::default();
        s.set_pinned(Edge::Right, true);
        s.open(T);
        assert_eq!(s.set_pinned(Edge::Right, false), tr(&[], &[T]));
        assert_eq!(s.transient(), Some(R));
        assert_eq!(s.set_pinned(Edge::Right, false), tr(&[], &[]), "sabit değil");
        assert_eq!(s.dismiss(), tr(&[], &[R]), "artık örtük kapanır");
    }

    #[test]
    fn focus_grab_follows_transient_only() {
        let mut s = PanelState::default();
        assert!(!s.wants_focus_grab());
        s.open(R);
        assert!(s.wants_focus_grab(), "geçici yaprak grab alır");
        s.set_pinned(Edge::Right, true);
        assert!(!s.wants_focus_grab(), "sabitlenince grab bırakılır");
        s.open(C);
        assert!(s.wants_focus_grab(), "sabit + geçici: grab geçiciyi kapsar");
        s.dismiss();
        assert!(!s.wants_focus_grab(), "cleared → geçici kapanır, sabit kalır, grab yok");
        assert!(s.is_open(R));
        s.set_pinned(Edge::Right, false);
        assert!(s.wants_focus_grab(), "sabitleme kalkınca panel geçici olur, grab alır");
        s.close_all();
        assert!(!s.wants_focus_grab());
    }

    #[test]
    fn focus_grab_steps() {
        use GrabStep::*;
        assert_eq!(focus_grab_step(true, false, false), Acquire);
        assert_eq!(focus_grab_step(true, false, true), Acquire, "yoksa kur (mod zaten indi)");
        assert_eq!(focus_grab_step(true, true, false), Keep, "yaprak değişti, grab sürer");
        assert_eq!(focus_grab_step(true, true, true), Reacquire, "önce OnDemand, sonra yeni grab");
        assert_eq!(focus_grab_step(false, true, false), Release, "Escape/düğme/komut: önce bırak");
        assert_eq!(focus_grab_step(false, true, true), Release);
        assert_eq!(focus_grab_step(false, false, false), Keep);
        assert_eq!(focus_grab_step(false, false, true), Keep);
    }

    #[test]
    fn close_all_closes_pinned_too() {
        let mut s = PanelState::default();
        s.set_pinned(Edge::Right, true);
        s.set_pinned(Edge::Left, true);
        s.open(C);
        assert_eq!(s.close_all(), tr(&[], &[C, L, R]));
        assert!(!s.any_open() && s.pinned().is_empty());
        assert_eq!(s.close(R), tr(&[], &[]), "kapalıyı kapatmak boş");
    }

    #[test]
    fn pinned_panels_share_inner_area_in_pin_order() {
        let inner = Rect::new(6.0, 34.0, 1908.0, 1040.0);
        let (areas, rest) = share_area(inner, &[(Edge::Right, 380.0), (Edge::Top, 300.0)]);
        assert_eq!(areas[0], inner);
        // Üst panel sağ panelin bittiği yere kadar.
        assert_eq!(areas[1], Rect::new(6.0, 34.0, 1528.0, 1040.0));
        assert_eq!(rest, Rect::new(6.0, 334.0, 1528.0, 740.0));
        let (_, none) = share_area(inner, &[]);
        assert_eq!(none, inner);
    }

    #[test]
    fn carve_never_goes_negative() {
        let a = Rect::new(0.0, 0.0, 100.0, 50.0);
        for e in Edge::ALL {
            for d in [0.0, 30.0, 500.0, f32::NAN, -5.0] {
                let r = carve(a, e, d);
                assert!(r.width >= 0.0 && r.height >= 0.0, "{e:?} {d}: {r:?}");
                assert_eq!(a.intersect(&r), r);
            }
        }
        assert_eq!(carve(a, Edge::Left, 500.0), Rect::new(100.0, 0.0, 0.0, 50.0));
        assert_eq!(carve(a, Edge::Bottom, 20.0), Rect::new(0.0, 0.0, 100.0, 30.0));
    }

    #[test]
    fn pin_corner_follows_panel_edge_then_bar() {
        let c = |right, bottom| Corner { right, bottom };
        // Sol/sağ panel: x panelin kenarı, y bar'a yakın (bar dikeyse üst).
        assert_eq!(pin_corner(Edge::Right, Edge::Top), c(true, false));
        assert_eq!(pin_corner(Edge::Right, Edge::Bottom), c(true, true));
        assert_eq!(pin_corner(Edge::Left, Edge::Top), c(false, false));
        assert_eq!(pin_corner(Edge::Left, Edge::Bottom), c(false, true));
        assert_eq!(pin_corner(Edge::Left, Edge::Left), c(false, false));
        assert_eq!(pin_corner(Edge::Right, Edge::Left), c(true, false));
        // Üst/alt panel: y panelin kenarı, x bar'a yakın (bar yataysa sağ).
        assert_eq!(pin_corner(Edge::Top, Edge::Top), c(true, false));
        assert_eq!(pin_corner(Edge::Bottom, Edge::Top), c(true, true));
        assert_eq!(pin_corner(Edge::Top, Edge::Left), c(false, false));
        assert_eq!(pin_corner(Edge::Bottom, Edge::Right), c(true, true));
    }

    #[test]
    fn pin_floats_in_corner_without_reserving_space() {
        let area = Rect::new(100.0, 50.0, 300.0, 600.0);
        let c = |right, bottom| Corner { right, bottom };
        assert_eq!(pin_rect(area, c(true, false), 32.0), Rect::new(368.0, 50.0, 32.0, 32.0));
        assert_eq!(pin_rect(area, c(false, false), 32.0), Rect::new(100.0, 50.0, 32.0, 32.0));
        assert_eq!(pin_rect(area, c(true, true), 32.0), Rect::new(368.0, 618.0, 32.0, 32.0));
        assert_eq!(pin_rect(area, c(false, true), 32.0), Rect::new(100.0, 618.0, 32.0, 32.0));
        // Sığmayan / bozuk alan: negatif boyut yok, alanın içinde.
        let b = pin_rect(Rect::new(0.0, 0.0, 10.0, 5.0), c(true, false), 32.0);
        assert_eq!(b, Rect::new(5.0, 0.0, 5.0, 5.0));
        let b = pin_rect(Rect::new(0.0, 0.0, -4.0, 20.0), c(false, true), 32.0);
        assert!(b.width == 0.0 && b.height == 0.0, "{b:?}");
    }

    #[test]
    fn pin_pushes_content_only_when_it_would_touch_it() {
        let c = |right, bottom| Corner { right, bottom };
        // İçerik alanı (0,0)-(300,600); panel iç boşluğu 8 → widget 8'den başlar; pin 32.
        let pin_tr = Rect::new(268.0, 0.0, 32.0, 32.0);
        let first = Rect::new(8.0, 8.0, 284.0, 100.0);
        // Kart sütunu oluğu (yatay boşluk 24 ≥ taşma 24): itme yok.
        assert_eq!(pin_clearance_shift(pin_tr, c(true, false), first, Size::new(24.0, 8.0)), 0.0);
        // Yatay boşluk yetmiyor: dikey taşma (24) boşluğa (20) inene kadar → 4.
        assert_eq!(pin_clearance_shift(pin_tr, c(true, false), first, Size::new(20.0, 20.0)), 4.0);
        // Bilinmeyen içerik (boşluk 0): widget tamamen pinin altına.
        assert_eq!(pin_clearance_shift(pin_tr, c(true, false), first, Size::ZERO), 24.0);
        // Alt köşe: widget'ın altından ölçülür.
        let pin_bl = Rect::new(0.0, 568.0, 32.0, 32.0);
        let last = Rect::new(8.0, 400.0, 284.0, 192.0);
        assert_eq!(pin_clearance_shift(pin_bl, c(false, true), last, Size::new(8.0, 8.0)), 16.0);
        // Pinin altında olmayan widget (alt köşede yığın bitmeden): itme yok.
        let short = Rect::new(8.0, 8.0, 284.0, 300.0);
        assert_eq!(pin_clearance_shift(pin_bl, c(false, true), short, Size::ZERO), 0.0);
        // Boş widget / boş pin.
        assert_eq!(pin_clearance_shift(pin_tr, c(true, false), Rect::ZERO, Size::ZERO), 0.0);
        assert_eq!(pin_clearance_shift(Rect::ZERO, c(true, false), first, Size::ZERO), 0.0);
    }
}
