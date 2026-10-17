//! Medya kartı: etkin MPRIS oynatıcısının kapağı, başlık/sanatçı, ilerleme ve önceki /
//! oynat-duraklat / sonraki. Kapak yalnızca yerel dosyadan (`file://`, servis güvenli okur,
//! `SystemServices` çözer); ağ adresli kapakta ve kapaksız parçada genel müzik ikonu.
//! Kapağa ya da başlığa tıklamak oynatıcı penceresini öne getirir. Oynatıcı yoksa kart gizli.

use std::sync::Arc;
use std::time::Instant;

use nuxut::i18n::t;
use nuxut::render::{ImageData, ImageFit, ImageStyle, RectStyle, Texture, TextureOptions};
use nuxut::ui::{
    Icon, IconButton, IconButtonVariant, IconName, IconView, Label, Progress, TextRole, Ui, text_style,
};
use nuxut::{Corners, CursorShape, Point, PointerEvent, PointerKind, Rect, TextSystem, Theme};

use super::{draw_error, error_height, format_position, padding, set_flag};
use crate::model::ShellModel;
use crate::services::media::{PlaybackStatus, Player};
use crate::system::{ArtImage, ErrorSource, MediaControl, SystemCommand};
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

#[derive(Debug)]
struct ArtTexture {
    /// Hangi görüntüden yüklendi (aynı `Arc` → yeniden yükleme yok).
    source: Arc<[u8]>,
    texture: Texture,
}

#[derive(Debug)]
pub struct MediaCard {
    /// Etkin oynatıcının bus adı (`None`: kart gizli).
    player: Option<String>,
    /// Son görülen oynatıcı durumu (saat vuruşunda konum tahmini için).
    snapshot: Option<Player>,
    playing: bool,
    title: Label,
    artist: Label,
    time: Label,
    progress: Progress,
    has_progress: bool,
    previous: IconButton,
    play: IconButton,
    next: IconButton,
    placeholder: IconView,
    art: Option<ArtImage>,
    texture: Option<ArtTexture>,
    /// Kapak/başlık alanına basıldı (bırakınca öne getir).
    pressed_header: bool,
    error: Label,
    has_error: bool,
}

/// Kapak kenarı.
fn art_side(theme: &Theme) -> f32 {
    theme.space(16)
}

fn artist_text(p: &Player) -> String {
    let artists = p.metadata.artists.join(", ");
    match (artists.is_empty(), p.metadata.album.is_empty()) {
        (false, _) => artists,
        (true, false) => p.metadata.album.clone(),
        (true, true) => p.identity.clone(),
    }
}

fn title_text(p: &Player) -> String {
    if p.metadata.title.is_empty() { p.identity.clone() } else { p.metadata.title.clone() }
}

/// "1:02 / 3:30"; uzunluk bilinmiyorsa `None`.
fn time_text(p: &Player, now: Instant) -> Option<(String, f32)> {
    let len = p.metadata.length.filter(|l| !l.is_zero())?;
    let pos = p.position_at(now).min(len);
    Some((format!("{} / {}", format_position(pos), format_position(len)), pos.as_secs_f32() / len.as_secs_f32()))
}

impl MediaCard {
    pub fn new(cx: &WidgetCx) -> Self {
        let t = cx.theme;
        Self {
            player: None,
            snapshot: None,
            playing: false,
            title: Label::new("", text_style(t, TextRole::Title).with_max_lines(1)),
            artist: Label::new("", text_style(t, TextRole::Caption).with_max_lines(1)),
            time: Label::new("", text_style(t, TextRole::Caption)),
            progress: Progress::new(t, 0.0),
            has_progress: false,
            previous: IconButton::new(t, Icon::named(IconName::SkipPrevious), IconButtonVariant::Standard),
            play: IconButton::new(t, Icon::named(IconName::Play), IconButtonVariant::Filled),
            next: IconButton::new(t, Icon::named(IconName::SkipNext), IconButtonVariant::Standard),
            placeholder: IconView::with_role(t, Icon::named(IconName::Music), TextRole::IconLarge),
            art: None,
            texture: None,
            pressed_header: false,
            error: Label::new("", text_style(t, TextRole::Caption)),
            has_error: false,
        }
    }

    /// (kapak, metin sütunu, ilerleme, düğme satırı merkezi, alt kenar).
    fn layout(&self, rect: Rect, cx: &WidgetCx) -> (Rect, Rect, Rect, Point, f32) {
        let t = cx.theme;
        let p = padding(t);
        let side = art_side(t);
        let art = Rect::new(rect.x + p, rect.y + p, side, side);
        let tx = art.right() + t.space(3);
        let text = Rect::new(tx, art.y, (rect.right() - p - tx).max(0.0), side);
        let mut y = art.bottom() + t.space(2);
        let progress = if self.has_progress {
            let r = Rect::new(rect.x + p, y, rect.width - 2.0 * p, self.progress.preferred_size(t, 0.0).height);
            y = r.bottom() + t.space(1);
            r
        } else {
            Rect::ZERO
        };
        let b = self.play.preferred_size(t);
        let buttons = Point::new(rect.center().x, y + b.height * 0.5);
        (art, text, progress, buttons, y + b.height)
    }

    fn place_buttons(&mut self, center: Point, theme: &Theme) {
        let b = self.play.preferred_size(theme);
        let gap = theme.space(4);
        let y = center.y - b.height * 0.5;
        self.play.set_rect(Rect::new(center.x - b.width * 0.5, y, b.width, b.height));
        self.previous.set_rect(Rect::new(center.x - b.width * 1.5 - gap, y, b.width, b.height));
        self.next.set_rect(Rect::new(center.x + b.width * 0.5 + gap, y, b.width, b.height));
    }

    fn update_time(&mut self, p: &Player, now: Instant) -> bool {
        match time_text(p, now) {
            Some((text, v)) => {
                let mut changed = set_flag(&mut self.has_progress, true);
                changed |= self.time.set_text(&text);
                if (self.progress.value() - v).abs() > 1e-3 {
                    self.progress.set_value(v, now);
                    changed = true;
                }
                changed
            }
            None => set_flag(&mut self.has_progress, false),
        }
    }

    fn header(&self, rect: Rect, cx: &WidgetCx) -> Rect {
        let (art, text, ..) = self.layout(rect, cx);
        Rect::new(art.x, art.y, text.right() - art.x, art.height)
    }
}

impl PanelWidget for MediaCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Media
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let s = &model.system;
        let Some(p) = s.player() else {
            let changed = self.player.take().is_some();
            self.snapshot = None;
            self.art = None;
            self.texture = None;
            return changed;
        };
        let mut changed = self.player.as_deref() != Some(p.bus_name.as_str());
        self.player = Some(p.bus_name.clone());
        changed |= self.title.set_text(&title_text(p));
        changed |= self.artist.set_text(&artist_text(p));
        let playing = p.status == PlaybackStatus::Playing;
        let icon = Icon::named(if playing { IconName::Pause } else { IconName::Play });
        if set_flag(&mut self.playing, playing) {
            self.play.set_icon(icon);
            changed = true;
        }
        changed |= self.play.set_enabled(p.can_control && (p.can_play || p.can_pause), cx.now);
        changed |= self.previous.set_enabled(p.can_go_previous, cx.now);
        changed |= self.next.set_enabled(p.can_go_next, cx.now);
        changed |= self.update_time(p, cx.now);
        let art = s.art_image().cloned();
        if art != self.art {
            self.art = art;
            changed = true;
        }
        let err = s.error_for(ErrorSource::Media).unwrap_or_default();
        changed |= set_flag(&mut self.has_error, !err.is_empty());
        changed |= self.error.set_text(err);
        self.snapshot = Some(p.clone());
        changed
    }

    fn set_time(&mut self, _now: &chrono::DateTime<chrono::Local>, cx: &WidgetCx) -> bool {
        // Konum oynatıcıdan yalnızca değişince gelir; arada hıza göre tahmin edilir.
        match self.snapshot.take() {
            Some(p) => {
                let changed = self.update_time(&p, cx.now);
                self.snapshot = Some(p);
                changed
            }
            None => false,
        }
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        let t = cx.theme;
        self.title.set_style(text_style(t, TextRole::Title).with_max_lines(1));
        self.artist.set_style(text_style(t, TextRole::Caption).with_max_lines(1));
        self.time.set_style(text_style(t, TextRole::Caption));
        self.progress.set_theme(t, cx.now);
        self.previous.set_theme(t, cx.now);
        self.play.set_theme(t, cx.now);
        self.next.set_theme(t, cx.now);
        self.placeholder.set_theme(t, cx.now);
        self.error.set_style(text_style(t, TextRole::Caption));
    }

    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        if self.player.is_none() {
            return Some(0.0);
        }
        let p = padding(cx.theme);
        let (_, col, _, _, bottom) = self.layout(Rect::new(0.0, 0.0, width, 0.0), cx);
        self.title.set_max_width(Some(col.width));
        self.artist.set_max_width(Some(col.width));
        self.error.set_max_width(Some((width - 2.0 * p).max(0.0)));
        Some(bottom + error_height(&mut self.error, text, cx.theme, self.has_error) + p)
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        let t = cx.theme;
        let (art, col, progress, buttons, bottom) = self.layout(rect, cx);
        let radii = Corners::uniform(t.shape.radius_medium);

        // Kapak: çözülmüş yerel görsel ya da tonal zemin üstünde müzik ikonu.
        match &self.art {
            Some(img) => {
                if self.texture.as_ref().is_none_or(|x| !Arc::ptr_eq(&x.source, &img.pixels)) {
                    let data = ImageData { width: img.width, height: img.height, pixels: Arc::clone(&img.pixels) };
                    let texture = ui.painter.upload_texture(data, TextureOptions::default());
                    self.texture = Some(ArtTexture { source: Arc::clone(&img.pixels), texture });
                }
                if let Some(x) = &self.texture {
                    let style = ImageStyle { radii, opacity: ui.alpha(), ..ImageStyle::new(ImageFit::Cover) };
                    ui.painter.image(art, &x.texture, &style);
                }
            }
            None => {
                self.texture = None;
                ui.painter.rect(art, &RectStyle::filled(ui.fade(t.colors.surface_bright), radii));
                let c = ui.fade(t.colors.on_surface_variant);
                self.placeholder.draw_centered(ui.painter, art.center(), c);
            }
        }

        let tm = self.title.measure_in(ui.painter);
        let am = self.artist.measure_in(ui.painter);
        let y = col.center().y - (tm.height + am.height + t.space(1)) * 0.5;
        let on_surface = ui.fade(t.colors.on_surface);
        let variant = ui.fade(t.colors.on_surface_variant);
        self.title.draw(ui.painter, Point::new(col.x, y), on_surface);
        self.artist.draw(ui.painter, Point::new(col.x, y + tm.height + t.space(1)), variant);

        if self.has_progress {
            self.progress.set_rect(progress);
            self.progress.draw(ui);
            let m = self.time.measure_in(ui.painter);
            self.time.draw(ui.painter, Point::new(progress.right() - m.width, progress.bottom() + t.space(1)), variant);
        }

        self.place_buttons(buttons, t);
        self.previous.draw(ui);
        self.play.draw(ui);
        self.next.draw(ui);

        if self.has_error {
            draw_error(ui, &mut self.error, Point::new(rect.x + padding(t), bottom + t.space(1)), t);
        }
    }

    /// Oynatıcı başlığı (tık: oynatıcıyı öne getir; ad başlık · sanatçı, açıklama süre), konum
    /// (ilerleme çubuğu varsa) ve önceki / oynat-duraklat / sonraki düğmeleri.
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, cx: &WidgetCx) {
        if self.player.is_none() {
            return;
        }
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            let name: Vec<&str> = [self.title.text(), self.artist.text()].into_iter().filter(|s| !s.is_empty()).collect();
            tree.push("header", nuxut::access::Role::Button)
                .label(&name.join(" · "))
                .description(self.time.text())
                .bounds(self.header(rect, cx))
                .clickable();
            self.previous.access(tree, "previous").label(&t!("shell-a11y-media-previous"));
            let play = if self.playing { t!("shell-a11y-media-pause") } else { t!("shell-a11y-media-play") };
            self.play.access(tree, "play").label(&play);
            self.next.access(tree, "next").label(&t!("shell-a11y-media-next"));
            if self.has_error {
                self.error.access(tree, "error", rect);
            }
        });
    }

    fn pointer(&mut self, event: &PointerEvent, rect: Rect, _text: &mut TextSystem, cx: &WidgetCx) -> (bool, Option<Action>) {
        let Some(player) = self.player.clone() else { return (false, None) };
        let (.., buttons, _) = self.layout(rect, cx);
        self.place_buttons(buttons, cx.theme);
        let mut repaint = false;
        let mut control = None;
        for (b, c) in [
            (&mut self.previous, MediaControl::Previous),
            (&mut self.play, MediaControl::PlayPause),
            (&mut self.next, MediaControl::Next),
        ] {
            let r = b.pointer(event, cx.now);
            repaint |= r.repaint;
            if r.event.is_some() {
                control = Some(c);
            }
        }
        let header = self.header(rect, cx);
        match event.kind {
            PointerKind::Press(nuxut::Button::Left) => self.pressed_header = header.contains(event.position),
            PointerKind::Release(nuxut::Button::Left) => {
                let on_header = std::mem::take(&mut self.pressed_header) && header.contains(event.position);
                control = control.or(on_header.then_some(MediaControl::Raise));
            }
            PointerKind::Leave => self.pressed_header = false,
            _ => {}
        }
        let action = control.map(|control| Action::System(SystemCommand::Media { player, control }));
        (repaint || action.is_some(), action)
    }

    fn panel_closed(&mut self, _now: Instant) {
        self.pressed_header = false;
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        self.player.as_ref()?;
        self.previous
            .cursor(p)
            .or_else(|| self.play.cursor(p))
            .or_else(|| self.next.cursor(p))
            .or_else(|| self.header(rect, cx).contains(p).then_some(CursorShape::Pointer))
    }

    fn corner_clearance(&self, cx: &WidgetCx) -> nuxut::Size {
        let p = padding(cx.theme);
        nuxut::Size::new(p, p)
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.placeholder.animate(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::media::Metadata;
    use std::time::Duration;

    fn player(title: &str, artists: &[&str], album: &str) -> Player {
        let mut p = Player::new("org.mpris.MediaPlayer2.test".into(), Instant::now());
        p.identity = "Test Oynatıcı".into();
        p.metadata = Metadata {
            title: title.into(),
            artists: artists.iter().map(|a| (*a).to_owned()).collect(),
            album: album.into(),
            ..Metadata::default()
        };
        p
    }

    #[test]
    fn texts_fall_back_sensibly() {
        assert_eq!(title_text(&player("Şarkı", &[], "")), "Şarkı");
        assert_eq!(title_text(&player("", &[], "")), "Test Oynatıcı");
        assert_eq!(artist_text(&player("x", &["A", "B"], "Albüm")), "A, B");
        assert_eq!(artist_text(&player("x", &[], "Albüm")), "Albüm");
        assert_eq!(artist_text(&player("x", &[], "")), "Test Oynatıcı");
    }

    #[test]
    fn time_needs_length() {
        let mut p = player("x", &[], "");
        let now = Instant::now();
        assert!(time_text(&p, now).is_none());
        p.metadata.length = Some(Duration::from_secs(210));
        p.position = Duration::from_secs(62);
        p.position_at = now;
        p.rate = 0.0;
        let (text, v) = time_text(&p, now).unwrap();
        assert_eq!(text, "1:02 / 3:30");
        assert!((v - 62.0 / 210.0).abs() < 1e-4);
    }
}
