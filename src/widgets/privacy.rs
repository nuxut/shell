//! Gizlilik göstergesi: mikrofon, kamera ya da ekran yakalama (paylaşım/kayıt) kullanımdayken
//! bar'da dolgulu bir hap; hiçbiri yokken gizli (uzunluk 0). GNOME/macOS/Windows'taki
//! "kullanımda" işaretlerinin karşılığı (`docs/design/privacy-indicator.md`).
//!
//! - Mikrofon: PipeWire kayıt akışları (`AudioState::recording`, ses servisi).
//! - Kamera: `/dev/video*`'u açık tutan süreçler (`ShellModel::camera`; sahibi görülemeyen açma da "bilinmeyen" olarak yanar, `services::camera`).
//! - Ekran: Hyprland `screencast` sayacı (`ShellModel::screencast`) — portal paylaşımı ve
//!   wf-recorder/OBS gibi kayıtlar; bu yüzden ayrı bir "kayıt göstergesi" gerekmez.
//!
//! Renk: ekran yakalanıyorsa `error` (kayıt kırmızısı), yalnız mikrofon/kamera ise `primary`
//! dolgu; ikonlar karşıt (`on_*`). İpucu: hangi uygulamalar ve yakalamanın süresi. Tık: `panel`
//! (ses kartının olduğu panel). Görünür içerik değişmedikçe yeniden çizim yok; saat vuruşu yalnız
//! ipucundaki süreyi günceller.

use std::time::Instant;

use chrono::{DateTime, Local};
use nuxut::i18n::t;
use nuxut::render::RectStyle;
use nuxut::ui::{Icon, IconName, IconView, Interaction, TextRole};
use nuxut::{Corners, CursorShape, Painter, Point, Rect, TextSystem};

use super::registry::PrivacySettings;
use super::{Action, BarTooltip, BarWidget, PanelId, WidgetCx, WidgetKind, draw_hover};
use crate::model::ShellModel;

/// Uygulama listesinde en fazla bu kadar ad (fazlası "+n").
const MAX_APPS: usize = 4;

/// Görünen kaynaklar ve ipucu girdileri.
#[derive(Debug, Clone, Default, PartialEq)]
struct View {
    mic: Vec<String>,
    camera: Vec<String>,
    screen: Option<DateTime<Local>>,
    /// Ekran sayacı bir kopuşu atlattı (fail-closed; ipucunda belirtilir).
    screen_uncertain: bool,
}

impl View {
    fn from_model(model: &ShellModel, s: &PrivacySettings) -> Self {
        let mic = if s.microphone { app_names(model.system.audio.recording.iter().map(|r| r.app.as_str())) } else { Vec::new() };
        let camera = if s.camera {
            let mut names = model.camera.users.clone();
            if model.camera.unattributed {
                names.push(t!("shell-privacy-unknown-app"));
            }
            names
        } else {
            Vec::new()
        };
        let screen = if s.screen && model.screencast.active() { model.screencast.since } else { None };
        // Etkin ama başlangıcı bilinmeyen yakalama olmaz (`since` sayaç 0'dan çıkınca yazılır).
        let screen_uncertain = screen.is_some() && model.screencast.uncertain;
        Self { mic, camera, screen, screen_uncertain }
    }

    fn is_empty(&self) -> bool {
        self.mic.is_empty() && self.camera.is_empty() && self.screen.is_none()
    }

    /// Çizim sırası: ekran, kamera, mikrofon (en kritik önce).
    fn icons(&self) -> Vec<IconName> {
        let mut out = Vec::new();
        if self.screen.is_some() {
            out.push(IconName::Cast);
        }
        if !self.camera.is_empty() {
            out.push(IconName::Videocam);
        }
        if !self.mic.is_empty() {
            out.push(IconName::Mic);
        }
        out
    }

    fn tooltip(&self, now: DateTime<Local>) -> String {
        let mut lines = Vec::new();
        if let Some(since) = self.screen {
            let minutes = (now - since).num_minutes().max(0);
            lines.push(t!("shell-privacy-screen", minutes = minutes, time = since.format("%H:%M").to_string()));
            if self.screen_uncertain {
                lines.push(t!("shell-privacy-screen-uncertain"));
            }
        }
        if !self.camera.is_empty() {
            lines.push(t!("shell-privacy-camera", apps = join_apps(&self.camera)));
        }
        if !self.mic.is_empty() {
            lines.push(t!("shell-privacy-microphone", apps = join_apps(&self.mic)));
        }
        lines.join("\n")
    }
}

/// Tekil, boş olmayan adlar (sırası korunur).
fn app_names<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        let n = n.trim();
        let n = if n.is_empty() { t!("shell-privacy-unknown-app") } else { n.to_owned() };
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

fn join_apps(apps: &[String]) -> String {
    let mut shown = apps.iter().take(MAX_APPS).cloned().collect::<Vec<_>>().join(", ");
    if apps.len() > MAX_APPS {
        shown.push_str(&format!(" +{}", apps.len() - MAX_APPS));
    }
    shown
}

#[derive(Debug)]
pub struct Privacy {
    settings: PrivacySettings,
    view: View,
    icons: Vec<IconView>,
    tooltip: String,
    hover: Interaction,
}

impl Privacy {
    pub fn new(cx: &WidgetCx) -> Self {
        Self {
            settings: cx.settings.privacy.clone(),
            view: View::default(),
            icons: Vec::new(),
            tooltip: String::new(),
            hover: Interaction::new(cx.theme),
        }
    }

    fn padding(cx: &WidgetCx) -> f32 {
        cx.theme.space(2)
    }

    fn gap(cx: &WidgetCx) -> f32 {
        cx.theme.space(1)
    }

    fn apply(&mut self, view: View, cx: &WidgetCx) -> bool {
        if view == self.view {
            return false;
        }
        let names = view.icons();
        if names != self.view.icons() {
            self.icons = names.into_iter().map(|n| IconView::with_role(cx.theme, Icon::named(n), TextRole::IconSmall)).collect();
        }
        self.tooltip = view.tooltip(Local::now());
        self.view = view;
        true
    }

    fn target(&self) -> PanelId {
        self.settings.panel.into()
    }
}

/// Hap: bar'a dik eksende öğe boyunda (durum göstergeleriyle aynı).
fn chip_rect(rect: Rect, cx: &WidgetCx) -> Rect {
    if cx.vertical {
        Rect::new(rect.center().x - cx.item * 0.5, rect.y, cx.item, rect.height)
    } else {
        Rect::new(rect.x, rect.center().y - cx.item * 0.5, rect.width, cx.item)
    }
}

impl BarWidget for Privacy {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Privacy
    }

    fn sync(&mut self, model: &ShellModel, _monitor: &str, cx: &WidgetCx) -> bool {
        self.apply(View::from_model(model, &self.settings), cx)
    }

    /// Yalnızca ipucundaki süre (dakika) değişir; görünüm değişmez, yeniden çizim yok.
    fn set_time(&mut self, now: &DateTime<Local>, _cx: &WidgetCx) -> bool {
        if self.view.screen.is_some() {
            self.tooltip = self.view.tooltip(*now);
        }
        false
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.hover.set_theme(cx.theme, cx.now);
        for i in &mut self.icons {
            i.set_theme(cx.theme, cx.now);
        }
        // Ayar değişince görünüm bir sonraki `sync`'te yeniden hesaplanır.
        self.settings = cx.settings.privacy.clone();
    }

    fn measure(&mut self, text: &mut TextSystem, cx: &WidgetCx) -> f32 {
        if self.view.is_empty() {
            return 0.0;
        }
        let n = self.icons.len() as f32;
        // Hepsi aynı rol: ilk ikonun bar ekseni boyunca boyu.
        let icon = self.icons.first_mut().map_or(0.0, |i| {
            let s = i.measure(text);
            if cx.vertical { s.height } else { s.width }
        });
        Self::padding(cx) * 2.0 + icon * n + Self::gap(cx) * (n - 1.0).max(0.0)
    }

    fn draw(&mut self, painter: &mut Painter, rect: Rect, cx: &WidgetCx, _highlighted: bool) {
        if self.view.is_empty() {
            return;
        }
        let colors = &cx.theme.colors;
        let chip = chip_rect(rect, cx);
        let radii = Corners::uniform(if cx.vertical { cx.theme.shape.radius_small } else { chip.height * 0.5 });
        let (fill, on) = if self.view.screen.is_some() { (colors.error, colors.on_error) } else { (colors.primary, colors.on_primary) };
        painter.rect(chip, &RectStyle::filled(fill, radii));
        draw_hover(painter, &self.hover, chip, radii, cx, fill, &[on]);
        let pad = Self::padding(cx);
        let gap = Self::gap(cx);
        let mut pos = if cx.vertical { chip.y + pad } else { chip.x + pad };
        for icon in &mut self.icons {
            let s = icon.measure(painter.text_system());
            let len = if cx.vertical { s.height } else { s.width };
            let center = if cx.vertical { Point::new(chip.center().x, pos + len * 0.5) } else { Point::new(pos + len * 0.5, chip.center().y) };
            icon.draw_centered(painter, center, on);
            pos += len + gap;
        }
    }

    fn hover(&mut self, rect: Rect, pointer: Option<Point>, down: bool, cx: &WidgetCx) -> bool {
        if self.view.is_empty() {
            return false;
        }
        self.hover.pointer(chip_rect(rect, cx), pointer, down, cx.now)
    }

    fn click(&mut self, rect: Rect, at: Point, cx: &WidgetCx) -> Option<Action> {
        (!self.view.is_empty() && chip_rect(rect, cx).contains(at)).then(|| Action::TogglePanel(self.target()))
    }

    fn panel(&self, _cx: &WidgetCx) -> Option<PanelId> {
        Some(self.target())
    }

    fn is_animating(&mut self, now: Instant) -> bool {
        self.hover.is_animating(now)
    }

    fn cursor(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        (!self.view.is_empty() && chip_rect(rect, cx).contains(p)).then_some(CursorShape::Pointer)
    }

    fn tooltip(&self, rect: Rect, p: Point, cx: &WidgetCx) -> Option<BarTooltip<'_>> {
        (!self.view.is_empty() && chip_rect(rect, cx).contains(p)).then_some(BarTooltip { text: &self.tooltip, anchor: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::audio::AudioStream;
    use crate::widgets::WidgetSettings;
    use nuxut::Theme;

    fn stream(app: &str) -> AudioStream {
        AudioStream { id: 1, app: app.into(), icon: None, media: None, volume: 1.0, muted: false, running: true }
    }

    fn cx<'a>(theme: &'a Theme, settings: &'a WidgetSettings) -> WidgetCx<'a> {
        WidgetCx { theme, settings, vertical: false, item: 26.0, now: Instant::now(), bounds: Rect::ZERO }
    }

    #[test]
    fn hidden_until_something_is_in_use() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let settings = WidgetSettings::default();
        let cx = cx(&theme, &settings);
        let mut w = Privacy::new(&cx);
        let mut text = TextSystem::new();
        let mut m = ShellModel::default();
        assert!(!w.sync(&m, "", &cx));
        assert_eq!(w.measure(&mut text, &cx), 0.0);
        assert_eq!(w.click(Rect::new(0.0, 0.0, 40.0, 34.0), Point::new(5.0, 17.0), &cx), None);

        m.system.audio.recording = vec![stream("Firefox"), stream("Firefox"), stream("")];
        assert!(w.sync(&m, "", &cx));
        let one = w.measure(&mut text, &cx);
        assert!(one > 0.0);
        assert_eq!(w.view.icons(), [IconName::Mic]);
        assert!(w.tooltip.contains("Firefox") && w.tooltip.matches("Firefox").count() == 1, "{}", w.tooltip);
        assert!(!w.sync(&m, "", &cx), "aynı durum yeniden çizim istemez");

        m.camera.users = vec!["zoom".into()];
        m.screencast.apply(true, Local::now());
        assert!(w.sync(&m, "", &cx));
        assert_eq!(w.view.icons(), [IconName::Cast, IconName::Videocam, IconName::Mic], "kritik önce");
        assert!(w.measure(&mut text, &cx) > one);
        assert!(w.tooltip.lines().count() == 3 && w.tooltip.contains("zoom"), "{}", w.tooltip);
        let rect = Rect::new(0.0, 0.0, 80.0, 34.0);
        assert_eq!(w.click(rect, rect.center(), &cx), Some(Action::TogglePanel(PanelId::Edge(crate::layout::Edge::Right))));
    }

    #[test]
    fn disabled_sources_are_ignored() {
        let theme = Theme::default();
        let mut settings = WidgetSettings::default();
        settings.privacy.microphone = false;
        settings.privacy.screen = false;
        let cx = cx(&theme, &settings);
        let mut w = Privacy::new(&cx);
        let mut m = ShellModel::default();
        m.system.audio.recording = vec![stream("Discord")];
        m.screencast.apply(true, Local::now());
        assert!(!w.sync(&m, "", &cx));
        assert!(w.view.is_empty());
        m.camera.users = vec!["cheese".into()];
        assert!(w.sync(&m, "", &cx));
        assert_eq!(w.view.icons(), [IconName::Videocam]);
    }

    /// Fail-closed: sahibi görülemeyen kamera kullanımı ve kopuş sonrası doğrulanamayan ekran
    /// yakalaması gizlenmez.
    #[test]
    fn unattributed_camera_and_uncertain_screen_stay_visible() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let settings = WidgetSettings::default();
        let cx = cx(&theme, &settings);
        let mut w = Privacy::new(&cx);
        let mut m = ShellModel::default();
        m.camera.unattributed = true;
        assert!(w.sync(&m, "", &cx));
        assert_eq!(w.view.icons(), [IconName::Videocam]);
        assert!(w.tooltip.contains("Bilinmeyen"), "{}", w.tooltip);
        m.apply_hyprland(crate::services::hyprland::HyprlandEvent::Screencast(true));
        m.apply_hyprland(crate::services::hyprland::HyprlandEvent::Disconnected);
        assert!(w.sync(&m, "", &cx));
        assert_eq!(w.view.icons(), [IconName::Cast, IconName::Videocam]);
        assert_eq!(w.tooltip.lines().count(), 3, "{}", w.tooltip);
    }

    #[test]
    fn long_app_lists_are_bounded() {
        let apps: Vec<String> = (0..7).map(|i| format!("app{i}")).collect();
        assert_eq!(join_apps(&apps), "app0, app1, app2, app3 +3");
        assert_eq!(join_apps(&apps[..2]), "app0, app1");
    }
}
