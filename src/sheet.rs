//! Kenardan büyüyen blob yaprağı (sheet): saat popout'u ve kenar çekmeceleri ortak mekanik.
//!
//! Yaprak çerçevenin iç kenarına yapışık bir `BlobShape`'tir. Açılınca tohumdan (kenarda,
//! derinliği 0) hedef dikdörtgene `motion.spatial` yayıyla büyür; hız yönünde esner
//! (`Deform`, çapa kenarın ortası). Kapanırken yay sıfırı aşarsa şekil kenar şeridine gömülür
//! (blob çökmesi); kapanıp oturunca alandan çıkar. İçerik [`Sheet::content_alpha`] ile
//! yaprakla birlikte büyüyerek belirir (büyüme %30 → %70); içerik her karede hedef boyutta yerleşir
//! ([`Sheet::content_rect`]) ve `visual_rect`'e kırpılır (yeniden yerleşme titremesi yok).

use std::time::Instant;

use nuxut::{Animated, BlobShape, Deform, Point, Rect, Theme};

use crate::layout::Edge;

/// İçeriğin belirmeye başladığı / tam göründüğü büyüme ilerlemesi.
const CONTENT_FADE_FROM: f32 = 0.30;
const CONTENT_FADE_TO: f32 = 0.70;

#[derive(Debug)]
pub struct Sheet {
    edge: Edge,
    open: bool,
    rect: Animated<Rect>,
    target: Rect,
    seed: Rect,
    deform: Deform,
    reveal: Animated<f32>,
    /// İçerik bu açılışta bir kez tam göründü: büyüme eşiği uygulanmaz (açıkken yeniden
    /// hedefleme içeriği gizlemez; kapanırken içerik `reveal` ile söner, yarıda geri açılışta
    /// sönükleşmeden döner). Tohumdan yeni açılış ve kenar değişimi sıfırlar.
    revealed: bool,
    /// Son karede bildirilen hasar (eski ∪ yeni için).
    pub last_damage: Option<Rect>,
}

impl Sheet {
    pub fn new(theme: &Theme, edge: Edge) -> Self {
        Self {
            edge,
            open: false,
            rect: Animated::new(Rect::ZERO, theme.motion.spatial),
            target: Rect::ZERO,
            seed: Rect::ZERO,
            deform: Deform::new(theme.blob.deform_spring, theme.blob.deform),
            reveal: Animated::new(0.0, theme.motion.effects),
            revealed: false,
            last_damage: None,
        }
    }

    pub fn edge(&self) -> Edge {
        self.edge
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// İçerik bu açılışta tam göründü mü (bkz. [`Sheet::content_alpha`]).
    pub fn is_revealed(&self) -> bool {
        self.revealed
    }

    /// Açık hâldeki hedef dikdörtgen (input bölgesi için).
    pub fn target(&self) -> Rect {
        self.target
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.rect.set_spring(theme.motion.spatial, now);
        self.reveal.set_spring(theme.motion.effects, now);
        self.deform.set_params(theme.blob.deform_spring, theme.blob.deform, now);
    }

    /// Kenar değişirse (config) yaprak anında kapalı duruma döner.
    pub fn set_edge(&mut self, edge: Edge) {
        if edge != self.edge {
            self.edge = edge;
            self.open = false;
            self.revealed = false;
            self.rect.jump_to(Rect::ZERO);
            self.reveal.jump_to(0.0);
        }
    }

    /// Açar: `target` açık dikdörtgen, `seed` kenardaki tohum (derinlik 0, kenar şeridinin
    /// 1 px içinde ki blob filetosuyla bar'a yapışık başlasın).
    pub fn open(&mut self, target: Rect, seed: Rect, now: Instant) {
        // Kapanıp oturmuş yaprak tohumdan başlar; kapanırken yeniden açılan, olduğu yerden döner.
        // Açılışta içeriğin görünürlüğünü yalnız büyüme ilerlemesi belirler
        // ([`Sheet::content_alpha`]); `reveal` kapanış solmasıdır. Tohumdan açılışta yay 0'dan
        // başlasaydı büyüme eşiği aşıldığı halde içerik ~200 ms soluk kalır, yaprak boş kutu gibi
        // görünürdü. Kapanırken geri açılan yaprakta solma yayla geri döner (sıçrama yok).
        if !self.open && self.rect.is_settled(now) {
            self.rect.jump_to(seed);
            self.revealed = false;
            self.reveal.jump_to(1.0);
        } else {
            self.reveal.set_target(1.0, now);
        }
        self.target = target;
        self.seed = seed;
        self.rect.set_target(target, now);
        self.open = true;
    }

    pub fn close(&mut self, now: Instant) {
        if !self.open {
            return;
        }
        self.open = false;
        self.rect.set_target(self.seed, now);
        self.reveal.set_target(0.0, now);
    }

    /// Geometri değişti (boyut, config, tam ekran): açıksa yeni hedefe yayla akar.
    pub fn retarget(&mut self, target: Rect, seed: Rect, now: Instant) {
        self.target = target;
        self.seed = seed;
        let goal = if self.open { target } else { seed };
        if self.rect.target() != goal {
            self.rect.set_target(goal, now);
        }
    }

    /// Bu karedeki blob şekli; kapanıp oturmuşsa `None`.
    pub fn shape(&mut self, now: Instant) -> Option<BlobShape> {
        if !self.open && self.rect.is_settled(now) {
            return None;
        }
        let raw = self.rect.value(now);
        let horizontal = self.edge.is_horizontal();
        // Negatif derinlik (kapanış aşması): sıfır derinlikli, uzak kenarı şeride gömülü şekil.
        let rect = match self.edge {
            Edge::Top if raw.height < 0.0 => Rect::new(raw.x, raw.y + raw.height, raw.width.max(0.0), 0.0),
            Edge::Bottom if raw.height < 0.0 => Rect::new(raw.x, raw.y, raw.width.max(0.0), 0.0),
            Edge::Left if raw.width < 0.0 => Rect::new(raw.x + raw.width, raw.y, 0.0, raw.height.max(0.0)),
            Edge::Right if raw.width < 0.0 => Rect::new(raw.x, raw.y, 0.0, raw.height.max(0.0)),
            _ => Rect::new(raw.x, raw.y, raw.width.max(0.0), raw.height.max(0.0)),
        };
        // Yalnızca büyüme ekseninde esner (merkez hızı).
        let v = self.rect.velocity(now);
        let velocity = if horizontal {
            Point::new(0.0, v.y + v.height * 0.5)
        } else {
            Point::new(v.x + v.width * 0.5, 0.0)
        };
        self.deform.update(velocity, now);
        let anchor = match self.edge {
            Edge::Top => Point::new(0.5, 0.0),
            Edge::Bottom => Point::new(0.5, 1.0),
            Edge::Left => Point::new(0.0, 0.5),
            Edge::Right => Point::new(1.0, 0.5),
        };
        // Köşeleri blob alanı biçimler (ADR-0008): kenara birleşen köşe R filetosu, boşluğa
        // açılan köşe `BlobField::corner_radius`.
        let shape = BlobShape::new_rect(rect).with_deform(self.deform.matrix(now), anchor);
        if self.open && self.growth_alpha(shape.visual_rect()) >= 1.0 {
            self.revealed = true;
        }
        Some(shape)
    }

    /// Büyüme ilerlemesi 0..1: tohumdan hedefe, iki eksenin gerisinde kalanı (derinlikte tohum
    /// 0; kenar boyunca tohum `length_factor` kadar).
    fn growth(&self, visual: Rect) -> f32 {
        let horizontal = self.edge.is_horizontal();
        let (len, depth) = if horizontal { (visual.width, visual.height) } else { (visual.height, visual.width) };
        let (t_len, t_depth) =
            if horizontal { (self.target.width, self.target.height) } else { (self.target.height, self.target.width) };
        let s_len = if horizontal { self.seed.width } else { self.seed.height };
        let p_depth = depth / t_depth.max(1.0);
        let span = t_len - s_len;
        let p_len = if span > 1.0 { (len - s_len) / span } else { 1.0 };
        p_depth.min(p_len)
    }

    /// Büyüme eşiğinin opaklığı: ilerleme %30'da başlar, %70'te tamdır (yumuşak geçiş).
    fn growth_alpha(&self, visual: Rect) -> f32 {
        let t = ((self.growth(visual) - CONTENT_FADE_FROM) / (CONTENT_FADE_TO - CONTENT_FADE_FROM)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// İçeriğin opaklığı. İçerik yaprakla birlikte büyüyerek çıkar (DESIGN.md: şekiller
    /// birbirinden büyüyerek çıkar): görünen dikdörtgene kırpılır, büyüme ilerlemesi %30'u
    /// geçince belirmeye başlar, %70'te tamdır; yaprak hiçbir karede içeriksiz koyu kutu
    /// olarak durmaz (`docs/design/panels.md` §4.1). İçerik bu açılışta bir kez tam göründüyse
    /// tohumdan yeni açılışa dek eşik uygulanmaz: açıkken yeniden hedefleme (yeni bildirim kartı,
    /// sabitleme ile yer paylaşımı) içeriği gizlemez; kapanışta içerik `reveal` ile söner.
    pub fn content_alpha(&self, visual: Rect, now: Instant) -> f32 {
        let gate = if self.revealed { 1.0 } else { self.growth_alpha(visual) };
        self.reveal.value(now).clamp(0.0, 1.0) * gate
    }

    /// İçeriğin yerleşim dikdörtgeni: her karede **hedef boyutta** (içerik yeniden yerleşmez);
    /// derinlikte bar'dan uzak kenara yapışık (yaprakla birlikte "iner"), kenar boyunca görünen
    /// dikdörtgenin ortasında. Görünen dikdörtgen daha darken (açılışın son ~%25'i) taşan kısım
    /// kırpılır; yay taşmasında ve esnemede içerik yerinde durur.
    ///
    /// Eskiden kenar boyunca görünen boyut kullanılıyordu: içerik açılış boyunca her karede
    /// başka genişlikte yerleşiyor, takvim ızgarası (`floor(genişlik / 7)` hücre) piksel
    /// adımlarıyla yeniden diziliyor, altındaki liste 6 px'lik sıçramalarla kayıyordu
    /// (saat popout'u "titreyerek" açılıyordu; `docs/design/panels.md` "İçerik yerleşimi").
    pub fn content_rect(&self, visual: Rect) -> Rect {
        let t = self.target;
        let c = visual.center();
        match self.edge {
            Edge::Top => Rect::new(c.x - t.width * 0.5, visual.bottom() - t.height, t.width, t.height),
            Edge::Bottom => Rect::new(c.x - t.width * 0.5, visual.y, t.width, t.height),
            Edge::Left => Rect::new(visual.right() - t.width, c.y - t.height * 0.5, t.width, t.height),
            Edge::Right => Rect::new(visual.x, c.y - t.height * 0.5, t.width, t.height),
        }
    }

    /// İçerik kayma payının işareti: içerik belirirken kenara doğru bu yönde hafifçe kayar.
    pub fn lift_direction(&self) -> Point {
        match self.edge {
            Edge::Top => Point::new(0.0, -1.0),
            Edge::Bottom => Point::new(0.0, 1.0),
            Edge::Left => Point::new(-1.0, 0.0),
            Edge::Right => Point::new(1.0, 0.0),
        }
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        (self.open || !self.rect.is_settled(now))
            && (!self.rect.is_settled(now) | !self.deform.is_settled(now) | !self.reveal.is_settled(now))
    }
}

/// Kenardaki tohum: `closed` (derinlik 0) dikdörtgeni şeridin 1 px içine kaydırılır;
/// `length_factor` < 1 ise kenar boyunca ortadan daraltılır (popout ortadan büyür).
pub fn seed_for(edge: Edge, closed: Rect, length_factor: f32) -> Rect {
    let f = length_factor.clamp(0.0, 1.0);
    match edge {
        Edge::Top | Edge::Bottom => {
            let w = closed.width * f;
            let y = if edge == Edge::Top { closed.y - 1.0 } else { closed.y + 1.0 };
            Rect::new(closed.center().x - w * 0.5, y, w, 0.0)
        }
        Edge::Left | Edge::Right => {
            let h = closed.height * f;
            let x = if edge == Edge::Left { closed.x - 1.0 } else { closed.x + 1.0 };
            Rect::new(x, closed.center().y - h * 0.5, 0.0, h)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn seed_is_centered_and_tucked_into_strip() {
        let closed = Rect::new(100.0, 34.0, 400.0, 0.0);
        assert_eq!(seed_for(Edge::Top, closed, 0.5), Rect::new(200.0, 33.0, 200.0, 0.0));
        let closed = Rect::new(1880.0, 40.0, 0.0, 1000.0);
        assert_eq!(seed_for(Edge::Right, closed, 1.0), Rect::new(1881.0, 40.0, 0.0, 1000.0));
        assert_eq!(seed_for(Edge::Left, Rect::new(6.0, 40.0, 0.0, 100.0), 1.0).x, 5.0);
    }

    #[test]
    fn opens_settles_and_disappears_after_close() {
        let theme = Theme::default();
        let mut s = Sheet::new(&theme, Edge::Right);
        let t0 = Instant::now();
        assert!(s.shape(t0).is_none(), "kapalı yaprak alanda yok");
        let target = Rect::new(1500.0, 40.0, 380.0, 1000.0);
        s.open(target, seed_for(Edge::Right, Rect::new(1880.0, 40.0, 0.0, 1000.0), 1.0), t0);
        assert!(s.shape(t0).is_some());
        let late = t0 + Duration::from_secs(5);
        assert!(!s.is_animating(late));
        let v = s.shape(late).unwrap().visual_rect();
        assert!((v.x - target.x).abs() < 0.5 && (v.width - target.width).abs() < 0.5, "{v:?}");
        assert!((s.content_alpha(v, late) - 1.0).abs() < 1e-3);
        s.close(late);
        let later = late + Duration::from_secs(5);
        assert!(s.shape(later).is_none(), "kapanıp oturan yaprak alandan çıkar");
        assert!(!s.is_animating(later));
    }

    #[test]
    fn content_hidden_while_small() {
        let theme = Theme::default();
        let mut s = Sheet::new(&theme, Edge::Top);
        let t0 = Instant::now();
        let target = Rect::new(700.0, 34.0, 380.0, 184.0);
        s.open(target, seed_for(Edge::Top, Rect::new(700.0, 34.0, 380.0, 0.0), 0.5), t0);
        let half = Rect::new(795.0, 34.0, 190.0, 92.0);
        assert_eq!(s.content_alpha(half, t0 + Duration::from_secs(5)), 0.0);
    }

    /// Boş yaprak regresyonu (2026-10-17): açılışta (tohumdan ve kapanırken geri açılışta)
    /// yaprak belirgin büyüklüğe ulaştığı hiçbir karede içeriksiz durmaz; açıkken yeniden
    /// hedefleme (yeni bildirim kartı) içeriği gizlemez. Dört kenar, iki hareket ön ayarı.
    #[test]
    fn sheet_never_shows_empty_once_grown() {
        use nuxut::theme::{FrameworkConfig, MotionPreset};
        let frame = |t0: Instant, i: u64| t0 + Duration::from_micros(16_667 * i);
        for preset in [MotionPreset::Calm, MotionPreset::Lively] {
            let mut cfg = FrameworkConfig::default();
            cfg.motion.preset = preset;
            let theme = Theme::from_config(&cfg).0;
            for edge in Edge::ALL {
                for factor in [0.5, 1.0] {
                    let (target, closed) = match edge {
                        Edge::Top => (Rect::new(770.0, 34.0, 380.0, 240.0), Rect::new(770.0, 34.0, 380.0, 0.0)),
                        Edge::Bottom => (Rect::new(770.0, 806.0, 380.0, 240.0), Rect::new(770.0, 1046.0, 380.0, 0.0)),
                        Edge::Left => (Rect::new(40.0, 220.0, 380.0, 640.0), Rect::new(40.0, 220.0, 0.0, 640.0)),
                        Edge::Right => (Rect::new(1500.0, 220.0, 380.0, 640.0), Rect::new(1880.0, 220.0, 0.0, 640.0)),
                    };
                    let seed = seed_for(edge, closed, factor);
                    let mut s = Sheet::new(&theme, edge);
                    // Tohumdan açılış, sonra kapanırken (yarıda) geri açılış.
                    let t0 = Instant::now();
                    {
                        s.open(target, seed, t0);
                        // `full`: büyüme %70'i geçince en az bu opaklık. Kapanırken geri açılışta kapanış solması
                        // yayla geri döner (sıçramaz): içerik boş kalmaz, sönükleşmez.
                        let checks = |s: &mut Sheet, from: Instant, n: u64, what: &str, full: f32| {
                            let mut last = 0.0_f32;
                            for i in 0..n {
                                let now = frame(from, i);
                                let Some(shape) = s.shape(now) else { continue };
                                let v = shape.visual_rect();
                                let a = s.content_alpha(v, now);
                                let ctx = format!("{preset:?} {edge:?} ×{factor} {what} kare {i}: {v:?} opaklık {a}");
                                if s.growth(v) >= CONTENT_FADE_TO {
                                    assert!(a >= full, "{ctx}");
                                }
                                if s.growth(v) > CONTENT_FADE_FROM + 0.05 {
                                    assert!(a > 0.0, "{ctx}");
                                    assert!(a >= last - 1e-3, "{ctx}: sönükleşti ({last} → {a})");
                                    last = a;
                                }
                            }
                        };
                        checks(&mut s, t0, 120, "açılış", 0.999);
                        assert!(s.is_revealed(), "{preset:?} {edge:?}: içerik tam görünmedi");

                        // Açıkken büyüme: içerik her karede tam.
                        let t1 = frame(t0, 120);
                        let bigger = match edge {
                            Edge::Top => Rect::new(770.0, 34.0, 380.0, 480.0),
                            Edge::Bottom => Rect::new(770.0, 566.0, 380.0, 480.0),
                            Edge::Left => Rect::new(40.0, 100.0, 380.0, 900.0),
                            Edge::Right => Rect::new(1500.0, 100.0, 380.0, 900.0),
                        };
                        s.retarget(bigger, seed, t1);
                        for i in 0..90 {
                            let now = frame(t1, i);
                            let v = s.shape(now).expect("açık").visual_rect();
                            assert!(s.content_alpha(v, now) >= 0.999, "{preset:?} {edge:?} büyüme kare {i}: {v:?}");
                        }
                        s.retarget(target, seed, frame(t1, 90));

                        // Kapanırken yarıda geri açılış: yine boş kalmaz.
                        let t2 = frame(t1, 200);
                        s.close(t2);
                        assert!(s.is_revealed(), "kapanış mandalı korur (geri açılış sönükleşmez)");
                        let t3 = frame(t2, 6);
                        let _ = s.shape(t3);
                        s.open(target, seed, t3);
                        checks(&mut s, t3, 120, "geri açılış", 0.25);
                        assert!(s.is_revealed());

                        // Kapanıp oturduktan sonra yeniden açılış: tohumda içerik yok, sonra belirir.
                        let t4 = frame(t3, 120);
                        s.close(t4);
                        let t5 = t4 + Duration::from_secs(5);
                        assert!(s.shape(t5).is_none());
                        s.open(target, seed, t5);
                        let v = s.shape(t5).expect("açılan").visual_rect();
                        assert_eq!(s.content_alpha(v, t5), 0.0, "{preset:?} {edge:?}: tohumda içerik");
                        checks(&mut s, t5, 120, "yeniden açılış", 0.999);
                    }
                }
            }
        }
    }

    #[test]
    fn content_rect_hugs_far_edge() {
        let theme = Theme::default();
        let mut s = Sheet::new(&theme, Edge::Top);
        let t0 = Instant::now();
        s.open(Rect::new(0.0, 34.0, 300.0, 200.0), Rect::new(75.0, 33.0, 150.0, 0.0), t0);
        let c = s.content_rect(Rect::new(0.0, 34.0, 300.0, 150.0));
        assert_eq!(c, Rect::new(0.0, -16.0, 300.0, 200.0));
        let mut r = Sheet::new(&theme, Edge::Right);
        r.open(Rect::new(1500.0, 40.0, 380.0, 1000.0), Rect::new(1881.0, 40.0, 0.0, 1000.0), t0);
        assert_eq!(r.content_rect(Rect::new(1600.0, 40.0, 280.0, 1000.0)).x, 1600.0);
    }

    /// Titreme regresyonu (saat popout'u, 2026-10-13): açılış boyunca içerik görünür olduğu
    /// HER karede (yay taşması ve esneme dahil) içerik dikdörtgeni hedef boyuttadır ve kenar
    /// boyunca yerinde durur; içerik (takvim ızgarası `floor(genişlik / 7)`) yeniden
    /// yerleşmez. Dört kenar, varsayılan (calm) ve taşmalı (lively) hareket.
    #[test]
    fn content_is_laid_out_at_target_size_on_every_frame() {
        use nuxut::theme::{FrameworkConfig, MotionPreset};
        for preset in [MotionPreset::Calm, MotionPreset::Lively] {
            let mut cfg = FrameworkConfig::default();
            cfg.motion.preset = preset;
            let theme = Theme::from_config(&cfg).0;
            for edge in Edge::ALL {
                let mut s = Sheet::new(&theme, edge);
                let (target, closed) = match edge {
                    Edge::Top => (Rect::new(770.0, 34.0, 380.0, 640.0), Rect::new(770.0, 34.0, 380.0, 0.0)),
                    Edge::Bottom => (Rect::new(770.0, 406.0, 380.0, 640.0), Rect::new(770.0, 1046.0, 380.0, 0.0)),
                    Edge::Left => (Rect::new(40.0, 220.0, 380.0, 640.0), Rect::new(40.0, 220.0, 0.0, 640.0)),
                    Edge::Right => (Rect::new(1500.0, 220.0, 380.0, 640.0), Rect::new(1880.0, 220.0, 0.0, 640.0)),
                };
                let t0 = Instant::now();
                s.open(target, seed_for(edge, closed, 0.5), t0);
                let mut visible = 0;
                for i in 0..120 {
                    let now = t0 + Duration::from_micros(16_667 * i);
                    let Some(shape) = s.shape(now) else { continue };
                    let visual = shape.visual_rect();
                    if s.content_alpha(visual, now) <= 0.0 {
                        continue;
                    }
                    visible += 1;
                    let c = s.content_rect(visual);
                    let ctx = format!("{preset:?} {edge:?} kare {i}: görünen {visual:?} içerik {c:?}");
                    assert_eq!((c.width, c.height), (target.width, target.height), "{ctx}");
                    if edge.is_horizontal() {
                        assert!((c.x - target.x).abs() < 0.01, "{ctx}");
                    } else {
                        assert!((c.y - target.y).abs() < 0.01, "{ctx}");
                    }
                }
                assert!(visible > 5, "{preset:?} {edge:?}: içerik görünmedi");
            }
        }
    }
}
