//! Şimdi çalıyor: etkin MPRIS oynatıcısının ([`crate::system::SystemModel::player`]) küçük kapağı,
//! "Başlık · Sanatçı" ve oynatma durumu. Veri medya kartıyla aynı modelden gelir; yeni servis,
//! yeni uyanma yok (ilerleme bar'da gösterilmez, saat vuruşu dinlenmez).
//!
//! - Görünürlük: etkin oynatıcı çalıyor ya da duraklatılmışsa; durmuş/oynatıcı yoksa gizli (0).
//! - Yatay bar: sabit `width` (metin "…" ile sığar). Başlık/durum değişince yerleşim kaymaz,
//!   yalnızca bu widget'ın dikdörtgeni yenilenir (`bar_damage`). Dikey bar: yalnızca kapak/ikon.
//! - Kapak: `SystemModel::art_image` (yerel dosya ya da `[media] remote_art` açıkken https;
//!   indirme `nuxut-net`, çözme ADR-0007 sandbox'ı — `SystemServices`). Yoksa müzik ikonu.
//! - Sol tık: `panel` (medya kartının olduğu panel); orta tık: oynat/duraklat; tekerlek:
//!   `scroll` (varsayılan ses — durum göstergesinin ses ikonuyla aynı yol). İpucu: tam başlık,
//!   sanatçı, oynatıcı ve durum.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nuxut::i18n::t;
use nuxut::render::{ImageData, ImageFit, ImageStyle, RectStyle, Texture, TextureOptions};
use nuxut::ui::{Icon, IconName, IconView, Interaction, Label, TextRole, text_style};
use nuxut::{Button, Corners, CursorShape, Painter, Point, PointerEvent, PointerKind, Rect, TextSystem};

use super::registry::{MediaScroll, NowPlayingSettings};
use super::{Action, BarTooltip, BarWidget, PanelId, WidgetCx, WidgetKind, chip_fill, draw_hover};
use crate::bar_edge::{EdgeCommand, ScrollAccumulator, ScrollAction};
use crate::model::ShellModel;
use crate::services::media::{PlaybackStatus, Player};
use crate::system::{ArtImage, MediaControl, SystemCommand};

/// Parça kipinde bir kaydırma hareketi tek atlama yapar; bu kadar sessizlikten sonra yeni hareket.
const TRACK_GESTURE_GAP: Duration = Duration::from_millis(400);

/// Bar'da gösterilen, karşılaştırılabilir durum (değişmediyse yeniden çizim yok).
#[derive(Debug, Clone, PartialEq)]
struct View {
    player: String,
    playing: bool,
    /// "Başlık · Sanatçı".
    line: String,
    tooltip: String,
    can_play_pause: bool,
    can_next: bool,
    can_previous: bool,
}

/// Oynatıcı görünür mü ve ne gösterilir. Durmuş oynatıcı gizlidir.
fn view(p: &Player) -> Option<View> {
    let playing = match p.status {
        PlaybackStatus::Playing => true,
        PlaybackStatus::Paused => false,
        PlaybackStatus::Stopped => return None,
    };
    let title = if p.metadata.title.is_empty() { p.identity.as_str() } else { p.metadata.title.as_str() };
    let artists = p.metadata.artists.join(", ");
    let line = match (title.is_empty(), artists.is_empty()) {
        (true, true) => return None,
        (false, true) => title.to_owned(),
        (true, false) => artists.clone(),
        (false, false) => format!("{title} · {artists}"),
    };
    let state = if playing { t!("shell-now-playing-playing") } else { t!("shell-now-playing-paused") };
    let mut lines: Vec<&str> = vec![title];
    if !artists.is_empty() {
        lines.push(&artists);
    }
    if !p.metadata.album.is_empty() && p.metadata.album != title {
        lines.push(&p.metadata.album);
    }
    let footer = if p.identity.is_empty() { state } else { format!("{} · {state}", p.identity) };
    lines.push(&footer);
    let tooltip = lines.into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
    Some(View {
        player: p.bus_name.clone(),
        playing,
        line,
        tooltip,
        can_play_pause: p.can_control && (p.can_play || p.can_pause),
        can_next: p.can_control && p.can_go_next,
        can_previous: p.can_control && p.can_go_previous,
    })
}

#[derive(Debug)]
struct ArtTexture {
    /// Hangi görüntüden yüklendi (aynı `Arc` → yeniden yükleme yok).
    source: Arc<[u8]>,
    texture: Texture,
}

#[derive(Debug)]
pub struct NowPlaying {
    view: Option<View>,
    settings: NowPlayingSettings,
    label: Label,
    state: IconView,
    placeholder: IconView,
    art: Option<ArtImage>,
    texture: Option<ArtTexture>,
    hover: Interaction,
    scroll: ScrollAccumulator,
    /// Parça kipinde bu hareketin son kaydırma anı ve atlama yapılıp yapılmadığı.
    track_gesture: Option<(Instant, bool)>,
    /// Son çizimdeki widget dikdörtgeni (orta tık ve tekerlek isabeti; olaylar bar sırasından
    /// önce `overlay_pointer`'a gelir).
    last_rect: Option<Rect>,
}

impl NowPlaying {
    pub fn new(cx: &WidgetCx) -> Self {
        let t = cx.theme;
        Self {
            view: None,
            settings: cx.settings.now_playing.clone(),
            label: Label::new("", label_style(cx)),
            state: IconView::with_role(t, Icon::named(IconName::Play), TextRole::IconSmall),
            placeholder: IconView::with_role(t, Icon::named(IconName::Music), TextRole::IconSmall),
            art: None,
            texture: None,
            hover: Interaction::new(t),
            scroll: ScrollAccumulator::default(),
            track_gesture: None,
            last_rect: None,
        }
    }

    fn target(&self) -> PanelId {
        self.settings.panel.into()
    }

    /// (kapak, metin, durum ikonu merkezi) — hap içinde. Dikey bar'da yalnızca kapak.
    fn layout(&self, chip: Rect, cx: &WidgetCx) -> (Rect, Rect, Point) {
        let t = cx.theme;
        let side = art_side(cx);
        if cx.vertical {
            let art = Rect::new(chip.center().x - side * 0.5, chip.center().y - side * 0.5, side, side);
            return (art, Rect::ZERO, Point::ZERO);
        }
        let inset = (chip.height - side) * 0.5;
        let art = Rect::new(chip.x + inset, chip.y + inset, side, side);
        let icon = t.space(4);
        let state = Point::new(chip.right() - t.space(2) - icon * 0.5, chip.center().y);
        let tx = art.right() + t.space(2);
        let text = Rect::new(tx, chip.y, (state.x - icon * 0.5 - t.space(1) - tx).max(0.0), chip.height);
        (art, text, state)
    }

    /// Orta tık / tekerlek. `None`: olay bu widget'ın değil.
    fn input(&mut self, event: &PointerEvent, cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        let rect = self.last_rect?;
        let view = self.view.as_ref()?;
        let inside = chip_rect(rect, cx).contains(event.position);
        let media = |control| Action::System(SystemCommand::Media { player: view.player.clone(), control });
        match event.kind {
            PointerKind::Press(Button::Middle) if inside => Some((false, None)),
            PointerKind::Release(Button::Middle) if inside => {
                Some((false, view.can_play_pause.then(|| media(MediaControl::PlayPause))))
            }
            PointerKind::Scroll { delta, steps } if inside => {
                let n = self.scroll.push(delta, steps, cx.now);
                let action = match self.settings.scroll {
                    MediaScroll::Off => return None,
                    MediaScroll::Volume => {
                        (n != 0).then_some(Action::BarEdge(EdgeCommand::Scroll { action: ScrollAction::Volume, steps: n }))
                    }
                    MediaScroll::Track => {
                        let fresh = self.track_gesture.is_none_or(|(at, _)| cx.now.saturating_duration_since(at) > TRACK_GESTURE_GAP);
                        let done = !fresh && self.track_gesture.is_some_and(|(_, done)| done);
                        let control = match n {
                            0 => None,
                            n if n > 0 && view.can_next => Some(MediaControl::Next),
                            n if n < 0 && view.can_previous => Some(MediaControl::Previous),
                            _ => None,
                        };
                        let skip = control.filter(|_| !done);
                        self.track_gesture = Some((cx.now, done || skip.is_some()));
                        skip.map(media)
                    }
                };
                Some((false, action))
            }
            PointerKind::ScrollEnd => {
                self.scroll.end();
                self.track_gesture = None;
                None
            }
            _ => None,
        }
    }
}

/// Hap kalınlığı: öğe boyundan bir kademe büyük (kapak okunur kalsın; bar kalınlığı öğe boyu +
/// iki yanda `space(2)` olduğundan bar şeridine sığar).
fn thickness(cx: &WidgetCx) -> f32 {
    cx.item + cx.theme.space(2)
}

/// Kapak kenarı: hap kalınlığının iki yanda `space(1)/2` içi.
fn art_side(cx: &WidgetCx) -> f32 {
    (thickness(cx) - cx.theme.space(1)).max(8.0)
}

fn label_style(cx: &WidgetCx) -> nuxut::TextStyle {
    text_style(cx.theme, TextRole::Body).with_max_lines(1)
}

/// Hap: bar'a dik eksende [`thickness`] kalınlığında.
fn chip_rect(rect: Rect, cx: &WidgetCx) -> Rect {
    let t = thickness(cx);
    if cx.vertical {
        Rect::new(rect.center().x - t * 0.5, rect.y, t, rect.height)
    } else {
        Rect::new(rect.x, rect.center().y - t * 0.5, rect.width, t)
    }
}

impl BarWidget for NowPlaying {
    fn kind(&self) -> WidgetKind {
        WidgetKind::NowPlaying
    }

    fn sync(&mut self, model: &ShellModel, _monitor: &str, _cx: &WidgetCx) -> bool {
        let s = &model.system;
        let next = s.player().and_then(view);
        let art = next.as_ref().and_then(|_| s.art_image().cloned());
        let mut changed = false;
        if art != self.art {
            self.art = art;
            changed = true;
        }
        if next == self.view {
            return changed;
        }
        if next.is_none() {
            // Gizli widget kapak dokusunu tutmaz.
            self.texture = None;
        }
        if let Some(v) = &next {
            self.label.set_text(&v.line);
            // İkon orta tıkın yapacağı eylemi gösterir (medya kartı, GNOME/macOS/Windows medya
            // denetimleri gibi): çalarken duraklat, duraklatılmışken oynat.
            self.state.set_icon(Icon::named(if v.playing { IconName::Pause } else { IconName::Play }));
        }
        self.view = next;
        true
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.hover.set_theme(cx.theme, cx.now);
        self.state.set_theme(cx.theme, cx.now);
        self.placeholder.set_theme(cx.theme, cx.now);
        self.label.set_style(label_style(cx));
        self.settings = cx.settings.now_playing.clone();
    }

    fn measure(&mut self, _text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        if self.view.is_none() {
            return 0.0;
        }
        if cx.vertical {
            return thickness(cx);
        }
        let width = self.settings.width;
        let (_, text, _) = self.layout(Rect::new(0.0, 0.0, width, cx.item), cx);
        self.label.set_max_width(Some(text.width));
        width
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, highlighted: bool) {
        self.last_rect = Some(rect);
        let Some(view) = &self.view else { return };
        let colors = &cx.theme.colors;
        let chip = chip_rect(rect, cx);
        let radii = Corners::uniform(if cx.vertical { cx.theme.shape.radius_small } else { chip.height * 0.5 });
        if highlighted {
            painter.rect(chip, &RectStyle::filled(colors.surface_bright, radii));
        }
        let (text_color, state_color) = match (highlighted, view.playing) {
            (true, _) => (colors.primary, colors.primary),
            (false, true) => (colors.on_surface, colors.primary),
            (false, false) => (colors.on_surface_variant, colors.on_surface_variant),
        };
        draw_hover(painter, &self.hover, chip, radii, cx, chip_fill(cx.theme, highlighted), &[text_color, state_color]);

        let (art, text, state) = self.layout(chip, cx);
        // Duraklatılmışken kapak soluk (dikey bar'da tek durum ipucu bu).
        let opacity = if view.playing { 1.0 } else { 0.55 };
        let art_radii = Corners::uniform(cx.theme.shape.radius_small);
        match &self.art {
            Some(img) => {
                if self.texture.as_ref().is_none_or(|x| !Arc::ptr_eq(&x.source, &img.pixels)) {
                    let data = ImageData { width: img.width, height: img.height, pixels: Arc::clone(&img.pixels) };
                    let texture = painter.upload_texture(data, TextureOptions::default());
                    self.texture = Some(ArtTexture { source: Arc::clone(&img.pixels), texture });
                }
                if let Some(x) = &self.texture {
                    let style = ImageStyle { radii: art_radii, opacity, ..ImageStyle::new(ImageFit::Cover) };
                    painter.image(art, &x.texture, &style);
                }
            }
            None => {
                self.texture = None;
                painter.rect(art, &RectStyle::filled(colors.surface_bright, art_radii));
                let c = if view.playing { colors.on_surface_variant } else { colors.on_surface_variant.multiply_alpha(opacity) };
                self.placeholder.draw_centered(painter, art.center(), c);
            }
        }
        if cx.vertical {
            return;
        }
        let s = self.label.measure_in(painter);
        painter.push_clip(text);
        self.label.draw(painter, Point::new(text.x, text.center().y - s.height * 0.5), text_color);
        painter.pop_clip();
        self.state.draw_centered(painter, state, state_color);
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        if self.view.is_none() {
            return false;
        }
        self.hover.pointer(chip_rect(rect, cx), pointer, down, cx.now)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        self.view.as_ref()?;
        chip_rect(rect, cx).contains(at).then(|| Action::TogglePanel(self.target()))
    }

    fn panel(&self, _cx: &WidgetCx) -> Option<PanelId> {
        Some(self.target())
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.hover.is_animating(now)
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        self.view.as_ref()?;
        chip_rect(rect, cx).contains(p).then_some(CursorShape::Pointer)
    }

    fn tooltip(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        let view = self.view.as_ref()?;
        chip_rect(rect, cx).contains(p).then_some(BarTooltip { text: &view.tooltip, anchor: None })
    }

    fn overlay_pointer(&mut self, event: &PointerEvent, _text: &mut TextSystem, cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        self.input(event, cx)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::media::Metadata;
    use crate::widgets::WidgetSettings;
    use nuxut::Theme;

    fn player(status: PlaybackStatus, title: &str, artists: &[&str]) -> Player {
        let mut p = Player::new("org.mpris.MediaPlayer2.test".into(), Instant::now());
        p.identity = "Test Oynatıcı".into();
        p.status = status;
        p.can_control = true;
        p.can_play = true;
        p.can_pause = true;
        p.can_go_next = true;
        p.can_go_previous = true;
        p.metadata = Metadata {
            title: title.into(),
            artists: artists.iter().map(|a| (*a).to_owned()).collect(),
            length: Some(Duration::from_secs(200)),
            ..Metadata::default()
        };
        p
    }

    fn model(players: Vec<Player>) -> ShellModel {
        let mut m = ShellModel::default();
        m.system.players = players;
        m
    }

    struct Harness {
        theme: Theme,
        settings: WidgetSettings,
        now: Instant,
    }

    impl Harness {
        fn new() -> Self {
            Self { theme: Theme::default(), settings: WidgetSettings::default(), now: Instant::now() }
        }
        fn cx(&self, vertical: bool) -> WidgetCx<'_> {
            WidgetCx { theme: &self.theme, settings: &self.settings, vertical, item: 26.0, now: self.now, bounds: Rect::ZERO }
        }
    }

    fn event(kind: PointerKind, at: Point) -> PointerEvent {
        PointerEvent::new(at, kind)
    }

    fn scroll(up: bool) -> PointerKind {
        PointerKind::Scroll { delta: Point::ZERO, steps: Point::new(0.0, if up { -1.0 } else { 1.0 }) }
    }

    #[test]
    fn visibility_follows_playback_state() {
        assert!(view(&player(PlaybackStatus::Stopped, "Şarkı", &["A"])).is_none());
        let playing = view(&player(PlaybackStatus::Playing, "Şarkı", &["A", "B"])).unwrap();
        assert!(playing.playing);
        assert_eq!(playing.line, "Şarkı · A, B");
        let paused = view(&player(PlaybackStatus::Paused, "Şarkı", &[])).unwrap();
        assert!(!paused.playing);
        assert_eq!(paused.line, "Şarkı");
        // Başlıksız parça: oynatıcı adı.
        assert_eq!(view(&player(PlaybackStatus::Playing, "", &[])).unwrap().line, "Test Oynatıcı");
    }

    #[test]
    fn tooltip_has_full_title_artist_and_player() {
        let long = "Çok uzun bir parça adı ".repeat(8);
        let v = view(&player(PlaybackStatus::Playing, long.trim(), &["Sanatçı"])).unwrap();
        let lines: Vec<_> = v.tooltip.lines().collect();
        assert_eq!(lines[0], long.trim());
        assert_eq!(lines[1], "Sanatçı");
        assert!(lines[2].starts_with("Test Oynatıcı · "), "{lines:?}");
    }

    /// Konum ilerlemesi görünümü değiştirmez (bar hasarı yok); başlık değişimi genişliği
    /// değiştirmez (yalnızca widget dikdörtgeni yenilenir, yerleşim kaymaz).
    #[test]
    fn progress_does_not_redraw_and_width_is_fixed() {
        let h = Harness::new();
        let cx = h.cx(false);
        let mut w = NowPlaying::new(&cx);
        let mut text = TextSystem::new();
        assert_eq!(w.measure(&mut text, &cx), 0.0, "oynatıcı yok: gizli");
        let mut p = player(PlaybackStatus::Playing, "Kısa", &["A"]);
        assert!(w.sync(&model(vec![p.clone()]), "", &cx));
        let short = w.measure(&mut text, &cx);
        assert_eq!(short, h.settings.now_playing.width);
        p.position = Duration::from_secs(42);
        assert!(!w.sync(&model(vec![p.clone()]), "", &cx), "ilerleme yeniden çizim istemez");
        p.metadata.title = "Çok çok çok daha uzun bir başlık ".repeat(5);
        assert!(w.sync(&model(vec![p.clone()]), "", &cx));
        assert_eq!(w.measure(&mut text, &cx), short, "genişlik sabit");
        p.status = PlaybackStatus::Stopped;
        assert!(w.sync(&model(vec![p]), "", &cx));
        assert_eq!(w.measure(&mut text, &cx), 0.0);
        // Dikey bar: yalnızca kapak, hap kalınlığında kare.
        let vcx = h.cx(true);
        let mut v = NowPlaying::new(&vcx);
        v.sync(&model(vec![player(PlaybackStatus::Paused, "x", &[])]), "", &vcx);
        assert_eq!(v.measure(&mut text, &vcx), thickness(&vcx));
    }

    #[test]
    fn clicks_and_wheel() {
        let mut h = Harness::new();
        let mut w = NowPlaying::new(&h.cx(false));
        w.sync(&model(vec![player(PlaybackStatus::Playing, "Şarkı", &["A"])]), "", &h.cx(false));
        let rect = Rect::new(100.0, 0.0, 240.0, 34.0);
        w.last_rect = Some(rect);
        let at = rect.center();
        let outside = Point::new(10.0, 10.0);
        let cx = h.cx(false);
        assert_eq!(w.click(rect, at, &cx), Some(Action::TogglePanel(PanelId::Edge(crate::layout::Edge::Right))));
        let media = |control| Some(Action::System(SystemCommand::Media { player: "org.mpris.MediaPlayer2.test".into(), control }));
        assert_eq!(w.input(&event(PointerKind::Press(Button::Middle), at), &cx), Some((false, None)));
        assert_eq!(w.input(&event(PointerKind::Release(Button::Middle), at), &cx), Some((false, media(MediaControl::PlayPause))));
        assert_eq!(w.input(&event(PointerKind::Release(Button::Middle), outside), &cx), None, "dışarıda bar'a düşer");
        // Varsayılan tekerlek: ses (OSD'li uç eylem yolu).
        let vol = Action::BarEdge(EdgeCommand::Scroll { action: ScrollAction::Volume, steps: 1 });
        assert_eq!(w.input(&event(scroll(true), at), &cx), Some((false, Some(vol))));

        // Parça kipi: hareket başına tek atlama; sessizlikten sonra yeni hareket.
        h.settings.now_playing.scroll = MediaScroll::Track;
        let cx = h.cx(false);
        w.set_theme(&cx);
        assert_eq!(w.input(&event(scroll(true), at), &cx), Some((false, media(MediaControl::Next))));
        assert_eq!(w.input(&event(scroll(true), at), &cx), Some((false, None)), "aynı hareket");
        assert_eq!(w.input(&event(PointerKind::ScrollEnd, at), &cx), None);
        assert_eq!(w.input(&event(scroll(false), at), &cx), Some((false, media(MediaControl::Previous))));
        h.now += TRACK_GESTURE_GAP * 2;
        let cx = h.cx(false);
        assert_eq!(w.input(&event(scroll(false), at), &cx), Some((false, media(MediaControl::Previous))));

        h.settings.now_playing.scroll = MediaScroll::Off;
        let cx = h.cx(false);
        w.set_theme(&cx);
        assert_eq!(w.input(&event(scroll(true), at), &cx), None, "kapalı: bar'a düşer");
    }

    #[test]
    fn hidden_widget_ignores_input() {
        let h = Harness::new();
        let cx = h.cx(false);
        let mut w = NowPlaying::new(&cx);
        let rect = Rect::new(0.0, 0.0, 240.0, 34.0);
        w.last_rect = Some(rect);
        assert_eq!(w.click(rect, rect.center(), &cx), None);
        assert_eq!(w.input(&event(PointerKind::Release(Button::Middle), rect.center()), &cx), None);
        assert!(w.tooltip(rect, rect.center(), &cx).is_none());
    }
}
