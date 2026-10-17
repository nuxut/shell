//! `logo` bar widget'ı: Nuxut marka işareti (Kara Sağan, framework `BrandMark`). Tıklayınca
//! uygulama başlatıcısını açar/kapatır (`launcher` düğmesiyle aynı yaprak); başlatıcı açıkken
//! vurgulanır.
//!
//! Görünüm `launcher`/`session` gibi kare ikon düğmeleriyle aynı dildedir: dairesel state layer
//! (hover/basma), açıkken `surface_bright` dolgu ve `primary` işaret; boşta `on_surface` tek
//! renk (DESIGN "Marka işareti"). Fark yalnızca boyda: işaretin çatal kuyruğu ancak ≥24 px'te
//! seçilir, bu yüzden düğme kenarı [`mark_side`] (öğe boyunun 4/3'ü, en az 24 px, bar
//! kalınlığına sığacak kadar). İşaret statiktir: yalnızca hover, vurgu ya da tema değişince
//! (widget'ın kendi dikdörtgeni) yeniden çizilir; ilk doku yüklemesi kare bütçesine takılırsa
//! hazır olana dek tek bir kare daha istenir.

use std::time::Instant;

use nuxut::theme::ColorRole;
use nuxut::ui::{BrandMark, Interaction, Ui};
use nuxut::{Corners, CursorShape, Painter, Point, Rect, RectStyle, TextSystem};

use super::{Action, BarWidget, PanelId, WidgetCx, WidgetKind, chip_fill, draw_hover};

/// İşaretin en küçük mantıksal kenarı: DESIGN "Marka işareti" (kuyruk 16 px'te incelir).
pub const MIN_MARK_SIDE: f32 = 24.0;

/// Düğmenin (ve işaretin) kenarı: öğe boyunun 4/3'ü (işaretin görünen gövdesi kutusunun ~%70'i;
/// böylece gövde diğer ikonlarla aynı optik boyda kalır), en az [`MIN_MARK_SIDE`]. Bar
/// kalınlığından (öğe + iki yanda `space(2)`) her yanda `space(1)` pay bırakacak kadar sınırlı;
/// bu sınır yalnızca 32 px'ten ince bar'da 24'ün altına iner.
pub fn mark_side(cx: &WidgetCx) -> f32 {
    (cx.item * 4.0 / 3.0).max(MIN_MARK_SIDE).min(cx.item + cx.theme.space(2))
}

#[derive(Debug)]
pub struct Logo {
    mark: BrandMark,
    hover: Interaction,
}

impl Logo {
    pub fn new(cx: &WidgetCx) -> Self {
        Self { mark: BrandMark::new().with_size(mark_side(cx)), hover: Interaction::new(cx.theme) }
    }

    fn button_rect(rect: Rect, cx: &WidgetCx) -> Rect {
        let s = mark_side(cx);
        Rect::new(rect.center().x - s * 0.5, rect.center().y - s * 0.5, s, s)
    }
}

impl BarWidget for Logo {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Logo
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.hover.set_theme(cx.theme, cx.now);
        self.mark.set_size(mark_side(cx));
        self.mark.set_theme(cx.theme, cx.now);
    }

    fn measure(&mut self, _text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        mark_side(cx)
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, highlighted: bool) {
        let b = Self::button_rect(rect, cx);
        let radii = Corners::uniform(b.width * 0.5);
        if highlighted {
            painter.rect(b, &RectStyle::filled(cx.theme.colors.surface_bright, radii));
        }
        let role = if highlighted { ColorRole::Primary } else { ColorRole::OnSurface };
        let mark = if highlighted { cx.theme.colors.primary } else { cx.theme.colors.on_surface };
        draw_hover(painter, &self.hover, b, radii, cx, chip_fill(cx.theme, highlighted), &[mark]);
        // Rol değişimi aynı maskeyi kullanır (yeniden raster yok). Hasar bar'ın widget başına
        // takibinde (`bar_damage`); bileşenin kendi hasar bildirimi burada kapalı.
        self.mark.set_style(nuxut::ui::BrandStyle::Mono(role));
        self.mark.set_size(mark_side(cx));
        self.mark.set_rect(b);
        let mut ui = Ui::new(painter, cx.theme, cx.now);
        self.mark.draw(&mut ui);
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        self.hover.pointer(Self::button_rect(rect, cx), pointer, down, cx.now)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        Self::button_rect(rect, cx).contains(at).then_some(Action::TogglePanel(PanelId::Launcher))
    }

    fn panel(&self, _cx: &WidgetCx) -> Option<PanelId> {
        Some(PanelId::Launcher)
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        // İlk doku yüklemesi kare bütçesine takıldıysa hazır olana dek bir kare daha.
        self.hover.is_animating(now) | self.mark.is_pending()
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        Self::button_rect(rect, cx).contains(p).then_some(CursorShape::Pointer)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use nuxut::{Point, Rect, TextSystem, Theme};

    use super::{Logo, MIN_MARK_SIDE, mark_side};
    use crate::widgets::{Action, BarWidget, PanelId, WidgetCx, WidgetSettings};

    fn cx<'a>(theme: &'a Theme, settings: &'a WidgetSettings, item: f32) -> WidgetCx<'a> {
        WidgetCx { theme, settings, vertical: false, item, now: Instant::now(), bounds: Rect::new(0.0, 0.0, 1920.0, 1080.0) }
    }

    #[test]
    fn side_is_at_least_24_and_fits_the_bar() {
        let theme = Theme::default();
        let settings = WidgetSettings::default();
        // Varsayılan bar (34 px) → öğe 18 → işaret 24, bar'da her yanda 5 px.
        let bar = 34.0;
        let c = cx(&theme, &settings, WidgetCx::item_for(&theme, bar));
        assert_eq!(mark_side(&c), MIN_MARK_SIDE);
        assert!(bar - mark_side(&c) >= 2.0 * theme.space(1));
        for bar in [32.0, 34.0, 40.0, 48.0, 64.0, 128.0] {
            let c = cx(&theme, &settings, WidgetCx::item_for(&theme, bar));
            let side = mark_side(&c);
            assert!(side >= MIN_MARK_SIDE, "{bar}: {side}");
            assert!(side <= bar - 2.0 * theme.space(1) + 1e-3, "{bar}: {side}");
            assert!(side >= c.item, "işaret diğer ikon düğmelerinden küçük değil");
        }
    }

    #[test]
    fn click_toggles_launcher_inside_button_only() {
        let theme = Theme::default();
        let settings = WidgetSettings::default();
        let c = cx(&theme, &settings, 18.0);
        let mut w = Logo::new(&c);
        let mut text = TextSystem::new();
        let len = w.measure(&mut text, &c);
        assert_eq!(len, MIN_MARK_SIDE);
        let rect = Rect::new(10.0, 0.0, len, 34.0);
        assert_eq!(w.click(rect, rect.center(), &c), Some(Action::TogglePanel(PanelId::Launcher)));
        assert_eq!(w.click(rect, Point::new(rect.center().x, 1.0), &c), None, "düğme dairesinin dışı");
        assert_eq!(w.panel(&c), Some(PanelId::Launcher), "başlatıcı açıkken vurgulanır");
        assert!(w.cursor(rect, rect.center(), &c).is_some());
        assert!(!w.is_animating(c.now), "boşta kare yok");
    }

    #[test]
    fn pending_texture_requests_another_frame() {
        let theme = Theme::default();
        let settings = WidgetSettings::default();
        let c = cx(&theme, &settings, 18.0);
        let mut w = Logo::new(&c);
        assert!(!w.is_animating(c.now), "çizimden önce bekleyen doku yok");
        let mut text = TextSystem::new();
        // Kayıtçıda doku GPU'ya hiç inmez: işaret hep bekliyor, widget kare istemeli.
        nuxut::render::Recorder::new().record(&mut text, (64, 64), 1.0, |p| {
            w.draw(p, Rect::new(0.0, 0.0, 32.0, 32.0), &c, false);
        });
        assert!(w.is_animating(c.now));
    }
}
