//! Bar şeridinin hasar takibi.
//!
//! Saat tıkı, sistem izleyici değeri, ses/pil ya da workspace değişimi gibi tek widget
//! güncellemeleri yalnızca o widget'ın dikdörtgenine hasar verir; bar yerleşimi (şerit,
//! widget dikdörtgenleri, uç bölgeleri) değiştiyse tüm şerit yenilenir.
//!
//! Renderer hasar bölgesine kırparak **her şeyi** (blob zemini dahil) yeniden çizer; bu yüzden
//! widget dikdörtgeni yarı saydam bar zeminini de kapsar, artık piksel kalmaz. Widget'lar kendi
//! dikdörtgenleri içinde çizer; öğe dikdörtgeni şeridin dik ekseninin tamamını kaplar. Güvenlik
//! payı olarak dikdörtgen bar ekseni boyunca boşluğun yarısı kadar genişletilir (komşuya değmez)
//! ve şeride kırpılır.

use nuxut::Rect;

/// Bar şeridinin bir karedeki yerleşimi: hasar kararının karşılaştırıldığı durum.
#[derive(Debug, Clone, PartialEq)]
pub struct BarFrame {
    /// Bar şeridi (`ShellLayout::bar`).
    pub strip: Rect,
    /// `Screen::bar` sırasıyla widget dikdörtgenleri (gizli widget: boş).
    pub items: Vec<Rect>,
    /// `EdgeSide::ALL` sırasıyla uç bölgeleri (uç widget ∪ ipucu yuvası).
    pub zones: [Option<Rect>; 2],
    /// Widget'lar arası boşluk (`bar.spacing`).
    pub gap: f32,
    /// Dikey bar mı (bar ekseni y).
    pub vertical: bool,
}

/// Sonraki karede yenilenecek bar bölgeleri.
#[derive(Debug)]
pub struct BarDamage {
    /// Tüm şerit (vurgu değişimi, yaprak geçişi, ilk kare).
    all: bool,
    /// `Screen::bar` sırasıyla kirli widget'lar (uzunluk yerleşimle eşleşmezse tüm şerit).
    widgets: Vec<bool>,
    /// `EdgeSide::ALL` sırasıyla kirli uç ipucu ikonları.
    hints: [bool; 2],
    /// Son çizilen yerleşim; değişirse tüm şerit (eski ve yeni konum aynı şeritte).
    last: Option<BarFrame>,
}

impl Default for BarDamage {
    fn default() -> Self {
        Self { all: true, widgets: Vec::new(), hints: [false; 2], last: None }
    }
}

impl BarDamage {
    /// Tüm şerit yenilensin.
    pub fn mark_all(&mut self) {
        self.all = true;
    }

    /// `index`teki widget değişti.
    pub fn mark(&mut self, index: usize) {
        if self.widgets.len() <= index {
            self.widgets.resize(index + 1, false);
        }
        self.widgets[index] = true;
    }

    /// `side` ucunun ipucu ikonu değişti (`EdgeSide::ALL` sırası).
    pub fn mark_hint(&mut self, side: usize) {
        if let Some(h) = self.hints.get_mut(side) {
            *h = true;
        }
    }

    /// Bekleyen hasar var mı (animasyon hariç).
    #[cfg(test)]
    pub fn is_pending(&self) -> bool {
        self.all || self.hints.iter().any(|h| *h) || self.widgets.iter().any(|w| *w)
    }

    /// Kare hasarını üretir ve bayrakları sıfırlar. `animating` widget'ları ve `hints_animating`
    /// uçları bu karede de yenilenir. Dönen dikdörtgenler mantıksal koordinattadır.
    ///
    /// Bir şey işaretlendiği hâlde görünür bölge kalmadıysa (ör. gizli widget) tek boş
    /// dikdörtgen döner: `Painter::damage(Rect::ZERO)` "değişiklik yok" demektir; hiç
    /// çağrılmazsa framework tüm yüzeyi hasarlı sayar.
    pub fn take(&mut self, frame: BarFrame, animating: &[bool], hints_animating: [bool; 2]) -> Vec<Rect> {
        let layout_changed = self.last.as_ref() != Some(&frame);
        let all = std::mem::take(&mut self.all) || layout_changed;
        let widgets = std::mem::take(&mut self.widgets);
        let hints = std::mem::take(&mut self.hints);
        let requested = all || widgets.iter().chain(&hints).chain(animating).chain(&hints_animating).any(|f| *f);
        let strip = frame.strip;
        let mut out = if all {
            // Eski şerit de (bar daraldı/kaydı): eski konumdaki pikseller silinsin.
            let old = self.last.as_ref().map(|l| l.strip).filter(|s| !s.is_empty() && *s != strip);
            [Some(strip).filter(|s| !s.is_empty()), old].into_iter().flatten().collect()
        } else if strip.is_empty() {
            Vec::new()
        } else {
            let dirty = |flags: &[bool], i: usize| flags.get(i).copied().unwrap_or(false);
            let mut out = Vec::new();
            for (i, r) in frame.items.iter().enumerate() {
                if (dirty(&widgets, i) || dirty(animating, i)) && !r.is_empty() {
                    out.extend(clip_to(pad_along(*r, &frame), strip));
                }
            }
            for (side, zone) in frame.zones.iter().enumerate() {
                if (hints[side] || hints_animating[side])
                    && let Some(z) = zone
                {
                    out.extend(clip_to(pad_along(*z, &frame), strip));
                }
            }
            out
        };
        if requested && out.is_empty() {
            out.push(Rect::ZERO);
        }
        self.last = Some(frame);
        out
    }
}

/// Dikdörtgeni bar ekseni boyunca boşluğun yarısı kadar genişletir.
fn pad_along(r: Rect, frame: &BarFrame) -> Rect {
    let p = (frame.gap * 0.5).max(0.0);
    if frame.vertical { Rect::new(r.x, r.y - p, r.width, r.height + 2.0 * p) } else { Rect::new(r.x - p, r.y, r.width + 2.0 * p, r.height) }
}

/// `r ∩ clip`; kesişim boşsa `None`.
fn clip_to(r: Rect, clip: Rect) -> Option<Rect> {
    let x0 = r.x.max(clip.x);
    let y0 = r.y.max(clip.y);
    let x1 = r.right().min(clip.right());
    let y1 = r.bottom().min(clip.bottom());
    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(clock_width: f32) -> BarFrame {
        // Yatay bar: başta workspace, ortada saat, sonda sistem izleyici.
        BarFrame {
            strip: Rect::new(0.0, 0.0, 1000.0, 40.0),
            items: vec![
                Rect::new(10.0, 4.0, 120.0, 32.0),
                Rect::new(500.0 - clock_width * 0.5, 4.0, clock_width, 32.0),
                Rect::new(850.0, 4.0, 140.0, 32.0),
            ],
            zones: [Some(Rect::new(10.0, 4.0, 120.0, 32.0)), None],
            gap: 8.0,
            vertical: false,
        }
    }

    fn settled(d: &mut BarDamage) {
        d.take(frame(80.0), &[], [false; 2]);
    }

    #[test]
    fn first_frame_damages_whole_strip() {
        let mut d = BarDamage::default();
        assert!(d.is_pending());
        assert_eq!(d.take(frame(80.0), &[], [false; 2]), vec![Rect::new(0.0, 0.0, 1000.0, 40.0)]);
    }

    #[test]
    fn clock_tick_damages_only_clock_rect() {
        let mut d = BarDamage::default();
        settled(&mut d);
        d.mark(1);
        let r = d.take(frame(80.0), &[], [false; 2]);
        // Saat 460..540; eksen boyunca yarım boşluk (4) payı, şeride kırpılmış.
        assert_eq!(r, vec![Rect::new(456.0, 4.0, 88.0, 32.0)]);
        assert!(!d.is_pending());
        // Değişiklik yoksa hasar yok.
        assert!(d.take(frame(80.0), &[], [false; 2]).is_empty());
    }

    #[test]
    fn layout_change_damages_whole_strip() {
        let mut d = BarDamage::default();
        settled(&mut d);
        d.mark(1);
        // Saat metni genişledi: yerleşim değişti → tüm şerit (eski konum da silinir).
        assert_eq!(d.take(frame(96.0), &[], [false; 2]), vec![Rect::new(0.0, 0.0, 1000.0, 40.0)]);
        // Bar boyu değişti (şerit) → tüm şerit.
        let mut f = frame(96.0);
        f.strip.height = 36.0;
        assert_eq!(d.take(f, &[], [false; 2]), vec![Rect::new(0.0, 0.0, 1000.0, 36.0), Rect::new(0.0, 0.0, 1000.0, 40.0)]);
    }

    #[test]
    fn mark_all_and_animation_and_hints() {
        let mut d = BarDamage::default();
        settled(&mut d);
        d.mark_all();
        assert_eq!(d.take(frame(80.0), &[], [false; 2]).len(), 1);
        // Animasyondaki widget (hover) yalnızca kendi dikdörtgeni.
        assert_eq!(d.take(frame(80.0), &[false, false, true], [false; 2]), vec![Rect::new(846.0, 4.0, 148.0, 32.0)]);
        // Uç ipucu: bölge, sol kenarda şeride kırpılır.
        d.mark_hint(0);
        assert_eq!(d.take(frame(80.0), &[], [false; 2]), vec![Rect::new(6.0, 4.0, 128.0, 32.0)]);
        // Uç bölgesi yokken ipucu hasarı açık boş dikdörtgen (tam yüzeye düşmesin).
        d.mark_hint(1);
        assert_eq!(d.take(frame(80.0), &[], [false; 2]), vec![Rect::ZERO]);
    }

    #[test]
    fn vertical_pads_along_y_and_hidden_widget_ignored() {
        let mut d = BarDamage::default();
        let f = BarFrame {
            strip: Rect::new(0.0, 0.0, 40.0, 800.0),
            items: vec![Rect::new(4.0, 0.0, 32.0, 50.0), Rect::ZERO],
            zones: [None, None],
            gap: 6.0,
            vertical: true,
        };
        d.take(f.clone(), &[], [false; 2]);
        d.mark(0);
        d.mark(1);
        d.mark(7);
        assert_eq!(d.take(f, &[], [false; 2]), vec![Rect::new(4.0, 0.0, 32.0, 53.0)]);
    }

    #[test]
    fn empty_strip_explicit_empty_damage() {
        let mut d = BarDamage::default();
        let mut f = frame(80.0);
        f.strip = Rect::ZERO;
        assert_eq!(d.take(f, &[], [false; 2]), vec![Rect::ZERO]);
    }
}
