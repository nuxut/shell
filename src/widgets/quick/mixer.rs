//! Ses karıştırıcı kartı (`mixer`): **ses çalan** her uygulama akışı için ad + seviye + sessiz.
//! Çıkış/giriş cihazı seçimi ses kartında (`device_picker`), burada tekrarlanmaz.
//!
//! - Yalnızca akan akışlar (`AudioStream::running`; duraklatılmış, corked akış yok). Akan akış
//!   yoksa kart yer kaplamaz (boş durum değil: gizli).
//! - İkon: uygulamanın logosu glif olarak (`app_glyph`, workspace ve paylaşım seçicisiyle aynı
//!   tablo; önce `application.icon-name`, sonra ad), bulunamazsa genel uygulama ikonu. Satırın
//!   solundaki hoparlör düğmesi sessiz aç/kapa, kaydırıcı ses kartıyla aynı ölçek (0..`max_volume`).
//! - Yazımlar ses kartıyla aynı yoldan: `SystemCommand::SetVolume`/`ToggleMute` + `AudioTarget::Node`
//!   (wpctl ve yerli arka uç ikisi de düğüm kimliğiyle yazar).
//! - Tasarım: `docs/design/mixer.md`.

use std::time::Instant;

use nuxut::ui::{Icon, IconName, IconView, Label, TextRole, Ui, app_glyph, text_style};
use nuxut::{CursorShape, Point, PointerEvent, Rect, TextSystem, Theme};

use super::padding;
use super::slider_row::{RowEvent, SliderRow};
use super::volume::{permille, slider_value};
use super::volume_icon;
use crate::model::ShellModel;
use crate::services::audio::{AudioStream, AudioTarget};
use crate::system::SystemCommand;
use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetKind};

/// Gösterilen en çok akış (fazlası gizli; sıra uygulama adına göre).
const MAX_STREAMS: usize = 8;

/// Akışın satır başlığı: "Uygulama · içerik" (içerik yoksa yalnız uygulama).
fn stream_title(s: &AudioStream) -> String {
    match s.media.as_deref().filter(|m| !m.is_empty() && *m != s.app) {
        Some(m) => format!("{} · {m}", s.app),
        None => s.app.clone(),
    }
}

/// Uygulama logosu: ikon adı, sonra uygulama adı; yoksa genel ikon.
fn stream_icon(s: &AudioStream) -> Icon {
    s.icon
        .as_deref()
        .and_then(app_glyph)
        .or_else(|| app_glyph(&s.app))
        .map_or(Icon::named(IconName::Apps), |g| Icon::glyph(&g.to_string()))
}

/// Karttaki akışlar: yalnızca akanlar, uygulama adına göre (aynı adda kimliğe göre).
fn visible_streams(playback: &[AudioStream]) -> Vec<&AudioStream> {
    let mut v: Vec<&AudioStream> = playback.iter().filter(|s| s.running).collect();
    v.sort_by(|a, b| a.app.to_lowercase().cmp(&b.app.to_lowercase()).then(a.id.cmp(&b.id)));
    v.truncate(MAX_STREAMS);
    v
}

#[derive(Debug)]
struct StreamRow {
    id: u32,
    icon: IconView,
    title: Label,
    slider: SliderRow,
}

impl StreamRow {
    fn new(theme: &Theme, s: &AudioStream) -> Self {
        Self {
            id: s.id,
            icon: IconView::with_role(theme, stream_icon(s), TextRole::IconSmall),
            title: Label::new(stream_title(s), text_style(theme, TextRole::Label).with_max_lines(1)),
            slider: SliderRow::new(theme, volume_icon(s.volume, s.muted)),
        }
    }

    fn sync(&mut self, s: &AudioStream, max: f32, now: Instant) -> bool {
        let mut changed = self.icon.set_icon(stream_icon(s));
        changed |= self.title.set_text(&stream_title(s));
        let (value, pct) = slider_value(s.volume, max);
        changed |= self.slider.set(volume_icon(s.volume, s.muted), value, pct, true, now);
        changed
    }
}

/// Bir akış satırının yerleşimi: başlık satırı ve kaydırıcı satırı.
#[derive(Debug, Clone, Copy)]
struct RowRects {
    head: Rect,
    slider: Rect,
}

#[derive(Debug)]
pub struct MixerCard {
    rows: Vec<StreamRow>,
    max_volume: f32,
}

impl MixerCard {
    pub fn new(_cx: &WidgetCx) -> Self {
        Self { rows: Vec::new(), max_volume: 1.0 }
    }

    /// Satır yerleşimleri. Ölçüm için `text` gerekir.
    fn layout(&mut self, text: &mut TextSystem, rect: Rect, cx: &WidgetCx) -> Vec<RowRects> {
        let t = cx.theme;
        let p = padding(t);
        let w = (rect.width - 2.0 * p).max(0.0);
        let mut y = rect.y + p;
        let mut out = Vec::with_capacity(self.rows.len());
        for (i, r) in self.rows.iter_mut().enumerate() {
            if i > 0 {
                y += t.space(2);
            }
            r.title.set_max_width(Some((w - t.space(8)).max(0.0)));
            let lh = r.title.measure(text).height.max(r.icon.measure(text).height);
            let head = Rect::new(rect.x + p, y, w, lh);
            let slider = Rect::new(rect.x + p, head.bottom() + t.space(1), w, r.slider.height(t));
            y = slider.bottom();
            out.push(RowRects { head, slider });
        }
        out
    }
}

impl PanelWidget for MixerCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Mixer
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        self.max_volume = model.system.max_volume;
        let streams = visible_streams(&model.system.audio.playback);
        let mut changed = false;
        if self.rows.len() != streams.len() || self.rows.iter().zip(&streams).any(|(r, s)| r.id != s.id) {
            self.rows = streams.iter().map(|s| StreamRow::new(cx.theme, s)).collect();
            changed = true;
        }
        for (r, s) in self.rows.iter_mut().zip(&streams) {
            changed |= r.sync(s, self.max_volume, cx.now);
        }
        changed
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        for r in &mut self.rows {
            r.icon.set_theme(cx.theme, cx.now);
            r.title.set_style(text_style(cx.theme, TextRole::Label).with_max_lines(1));
            r.slider.set_theme(cx.theme, cx.now);
        }
    }

    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        let rows = self.layout(text, Rect::new(0.0, 0.0, width, 0.0), cx);
        Some(rows.last().map_or(0.0, |r| r.slider.bottom() + padding(cx.theme)))
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        let rows = self.layout(ui.painter.text_system(), rect, cx);
        let t = cx.theme;
        let muted = ui.fade(t.colors.on_surface_variant);
        let fg = ui.fade(t.colors.on_surface);
        for (r, rr) in self.rows.iter_mut().zip(&rows) {
            // Logo, alttaki hoparlör düğmesiyle aynı sütunda ortalanır.
            let slot = r.slider.icon_slot(t);
            r.icon.draw_centered(ui.painter, Point::new(rr.head.x + slot * 0.5, rr.head.center().y), muted);
            let m = r.title.measure_in(ui.painter);
            r.title.draw(ui.painter, Point::new(rr.head.x + slot + t.space(2), rr.head.center().y - m.height * 0.5), fg);
            r.slider.draw(ui, rr.slider);
        }
    }

    fn pointer(&mut self, event: &PointerEvent, rect: Rect, text: &mut TextSystem, cx: &WidgetCx) -> (bool, Option<Action>) {
        let rows = self.layout(text, rect, cx);
        let mut repaint = false;
        let mut action = None;
        for (r, rr) in self.rows.iter_mut().zip(&rows) {
            let (rp, ev) = r.slider.pointer(event, rr.slider, cx.theme, cx.now);
            repaint |= rp;
            let target = AudioTarget::Node(r.id);
            action = action.or(ev.map(|e| match e {
                RowEvent::Icon => SystemCommand::ToggleMute(target),
                RowEvent::Value(v) => SystemCommand::SetVolume { target, permille: permille(v, self.max_volume) },
            }));
        }
        (repaint || action.is_some(), action.map(Action::System))
    }

    /// Uygulama başına kaydırıcı (+ sessiz düğmesi); ad uygulamanın başlığı, anahtar akış kimliği.
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, _cx: &WidgetCx) {
        if self.rows.is_empty() {
            return;
        }
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            for r in &self.rows {
                let key = r.id.to_string();
                let mute = nuxut::i18n::t!("shell-a11y-mixer-mute", app = r.title.text().to_owned());
                r.slider.access(tree, &key, r.title.text(), &mute);
            }
        });
    }

    fn access_action(
        &mut self,
        request: &nuxut::access::AccessRequest,
        parent: nuxut::access::NodeId,
        _rect: Rect,
        cx: &WidgetCx,
    ) -> Option<(bool, Option<Action>)> {
        let card = crate::widgets::access_card_id(parent, self.kind());
        let max = self.max_volume;
        self.rows.iter_mut().find_map(|r| match r.slider.access_action(card, &r.id.to_string(), request, cx.now)? {
            RowEvent::Value(v) => {
                let cmd = SystemCommand::SetVolume { target: AudioTarget::Node(r.id), permille: permille(v, max) };
                Some((true, Some(Action::System(cmd))))
            }
            RowEvent::Icon => None,
        })
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        self.rows.iter().find_map(|r| r.slider.cursor(p))
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        // Kaydırıcı/düğme kendi animasyonları sürerken `ui.request_frame()` der.
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut::{Button, PointerKind};

    fn stream(id: u32, app: &str, running: bool) -> AudioStream {
        AudioStream { id, app: app.into(), icon: None, media: None, volume: 0.5, muted: false, running }
    }

    #[test]
    fn only_running_streams_sorted_by_app() {
        let s = [stream(3, "mpv", true), stream(1, "Firefox", true), stream(2, "Spotify", false), stream(4, "firefox", true)];
        let v: Vec<u32> = visible_streams(&s).iter().map(|s| s.id).collect();
        assert_eq!(v, [1, 4, 3]);
    }

    #[test]
    fn titles_and_icons() {
        let mut s = stream(1, "Firefox", true);
        assert_eq!(stream_title(&s), "Firefox");
        s.media = Some("Firefox".into());
        assert_eq!(stream_title(&s), "Firefox", "içerik adı uygulama adıyla aynıysa yinelenmez");
        s.media = Some("Video".into());
        assert_eq!(stream_title(&s), "Firefox · Video");
        assert!(matches!(stream_icon(&s), Icon::Glyph(_)), "bilinen uygulama: logo glifi");
        assert_eq!(stream_icon(&stream(1, "zzz-bilinmeyen", true)), Icon::named(IconName::Apps));
    }

    #[test]
    fn card_hides_without_running_streams_and_maps_actions() {
        let theme = nuxut::Theme::default();
        let settings = crate::widgets::WidgetSettings::default();
        let now = Instant::now();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now, bounds: Rect::ZERO };
        let mut card = MixerCard::new(&cx);
        let mut text = TextSystem::new();
        let mut m = ShellModel::default();
        m.system.audio.playback = vec![stream(5, "mpv", false)];
        card.sync(&m, &cx);
        assert_eq!(card.height(&mut text, 340.0, &cx), Some(0.0), "akan akış yok: gizli");

        m.system.audio.playback[0].running = true;
        assert!(card.sync(&m, &cx));
        assert!(!card.sync(&m, &cx), "aynı model: değişiklik yok");

        // Satırın hoparlör düğmesine tık: o akışın sessizi.
        let rect = Rect::new(0.0, 0.0, 340.0, card.height(&mut text, 340.0, &cx).unwrap());
        assert!(rect.height > 0.0);
        let rows = card.layout(&mut text, rect, &cx);
        let at = Point::new(rows[0].slider.x + 4.0, rows[0].slider.center().y);
        let mut got = None;
        for kind in [PointerKind::Enter, PointerKind::Motion, PointerKind::Press(Button::Left), PointerKind::Release(Button::Left)] {
            got = got.or(card.pointer(&PointerEvent::new(at, kind), rect, &mut text, &cx).1);
        }
        assert_eq!(got, Some(Action::System(SystemCommand::ToggleMute(AudioTarget::Node(5)))));
    }
}
