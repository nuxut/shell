//! Ekran görüntüsü bağlantısı (`msg shot …`; tasarım `docs/design/screenshot.md`).
//!
//! Tek oturum: yakalama sürerken yeni `shot` reddedilir. `region`: arka plan işi tüm çıkışları
//! yakalar (+ Hyprland görünür pencereleri) → çıkış başına `Overlay` katmanı açılır (klavye
//! `Exclusive` yalnızca hedef çıkışta) → onayda katmanlar hemen kalkar, kesim dondurulmuş
//! kareden yapılır ve teslim (PNG, kayıt, pano) arka planda. Diğer kipler katmansız tek iştir.
//! `msg` yanıtı iş bitince verilir (`{"saved":…,"copied":…}` / `{"cancelled":true}`); seçim
//! IPC yanıt penceresinden (`ipc::REPLY_TIMEOUT`) uzun sürerse yanıt düşer, iş yine yapılır.

use std::path::PathBuf;
use std::time::Instant;

use nuxut::i18n::t;
use nuxut::{Anchor, Context, KeyEvent, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, OutputInfo, Painter, PointerEvent, Size, SurfaceId};

use super::{Shell, ShellMessage};
use crate::screenshot_view::ShotView;
use crate::services::notifications::{InternalAction, InternalNotice, NotifyIcon};
use crate::services::screenshot::capture::{self, Source};
use crate::services::screenshot::geometry::LogicalRect;
use crate::services::screenshot::selection::Outcome;
use crate::services::screenshot::{
    self as shot, Delivered, Delivery, Frame, OutputLayout, ShotCommand, ShotEvent, ShotMode,
};

pub const NAMESPACE: &str = "nuxut-screenshot";
/// Bildirim düğmeleri için bellekte tutulan son kayıt sayısı.
const MAX_RECENT: usize = 16;

/// Ekran görüntüsü durumu (Shell alanı).
#[derive(Debug, Default)]
pub struct ShotRuntime {
    /// Çıkışların adı ve mantıksal düzeni (`App::output_added/changed`).
    outputs: Vec<(OutputId, OutputLayout)>,
    session: Option<Session>,
    next_session: u64,
    /// (numara, yol): bildirimdeki "Aç"/"Klasörde göster" düğmeleri.
    recent: Vec<(u32, PathBuf)>,
    next_recent: u32,
}

/// Bir çıkışın katman yüzeyi ve görünümü.
#[derive(Debug)]
struct Overlay {
    surface: SurfaceId,
    output: OutputId,
    view: ShotView,
}

impl std::ops::Deref for Overlay {
    type Target = ShotView;
    fn deref(&self) -> &ShotView {
        &self.view
    }
}

impl std::ops::DerefMut for Overlay {
    fn deref_mut(&mut self) -> &mut ShotView {
        &mut self.view
    }
}

#[derive(Debug)]
struct Session {
    id: u64,
    delivery: Delivery,
    reply: Option<nuxut::ipc::Reply>,
    /// `region`: klavyenin verileceği çıkış.
    focus: Option<String>,
    views: Vec<Overlay>,
    /// Onaylandı/katmansız iş: teslim bekleniyor.
    delivering: bool,
}

impl ShotRuntime {
    fn view_mut(&mut self, surface: SurfaceId) -> Option<&mut Overlay> {
        self.session.as_mut()?.views.iter_mut().find(|v| v.surface == surface)
    }

    fn remember(&mut self, path: PathBuf) -> u32 {
        self.next_recent = self.next_recent.wrapping_add(1).max(1);
        if self.recent.len() >= MAX_RECENT {
            self.recent.remove(0);
        }
        self.recent.push((self.next_recent, path));
        self.next_recent
    }

    fn recent(&self, n: u32) -> Option<&PathBuf> {
        self.recent.iter().find(|(k, _)| *k == n).map(|(_, p)| p)
    }
}

/// Çıkış bilgisinden mantıksal düzen.
fn layout_of(info: &OutputInfo) -> LogicalRect {
    LogicalRect::new(
        info.position.x.round() as i32,
        info.position.y.round() as i32,
        info.size.width.round() as i32,
        info.size.height.round() as i32,
    )
}

impl Shell {
    pub(super) fn shot_output_changed(&mut self, info: &OutputInfo) {
        let layout = OutputLayout { name: info.name.clone(), layout: layout_of(info) };
        match self.shot.outputs.iter_mut().find(|(id, _)| *id == info.id) {
            Some((_, l)) => *l = layout,
            None => self.shot.outputs.push((info.id, layout)),
        }
    }

    pub(super) fn shot_output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        self.shot.outputs.retain(|(id, _)| *id != output);
        let Some(s) = &mut self.shot.session else { return };
        let before = s.views.len();
        s.views.retain(|v| {
            if v.output == output {
                cx.destroy_surface(v.surface);
            }
            v.output != output
        });
        if before > 0 && s.views.is_empty() && !s.delivering {
            self.shot_cancel(cx);
        }
    }

    /// `msg shot …` ve bildirim düğmeleri. `reply` varsa yanıt iş bitince (yakalama) ya da hemen.
    pub(super) fn shot_command(&mut self, cx: &mut Context<Self>, command: ShotCommand, reply: Option<nuxut::ipc::Reply>) {
        let result = match command {
            ShotCommand::Open(n) | ShotCommand::Reveal(n) => {
                let open = matches!(command, ShotCommand::Open(_));
                match self.shot.recent(n).cloned() {
                    Some(path) => {
                        let _ = std::thread::Builder::new().name("nuxut-shot-open".into()).spawn(move || {
                            if open { shot::save::open_path(&path) } else { shot::save::reveal_path(&path) }
                        });
                        Ok("ok".to_owned())
                    }
                    None => Err(format!("ekran görüntüsü {n} bilinmiyor")), // i18n: tanı msg yanıtı/bildirim ayrıntısı
                }
            }
            ShotCommand::Capture { mode, output, no_save, no_copy } => match self.shot_start(cx, mode, output, no_save, no_copy) {
                Ok(()) => {
                    if let Some(s) = &mut self.shot.session {
                        s.reply = reply;
                    }
                    return;
                }
                Err(e) => Err(e),
            },
        };
        if let Some(r) = reply {
            match result {
                Ok(text) => r.send(&text),
                Err(e) => r.send(&format!("error: {e}")),
            }
        } else if let Err(e) = result {
            tracing::warn!("shot: {e}");
        }
    }

    fn shot_start(&mut self, cx: &mut Context<Self>, mode: ShotMode, output: Option<String>, no_save: bool, no_copy: bool) -> Result<(), String> {
        if self.shot.session.is_some() {
            return Err("busy: başka bir ekran görüntüsü sürüyor".into()); // i18n: tanı msg yanıtı/bildirim ayrıntısı
        }
        let delivery = Delivery::plan(&self.model.config.screenshot, no_save, no_copy, chrono::Local::now())?;
        let layouts: Vec<OutputLayout> = self.shot.outputs.iter().map(|(_, l)| l.clone()).collect();
        if layouts.is_empty() {
            return Err("ekran yok".into());
        }
        let focused = self.model.hypr.focused_monitor().map(|m| m.name.clone());
        let target = match &output {
            Some(name) if !layouts.iter().any(|l| &l.name == name) => return Err(format!("monitör \"{name}\" yok")), // i18n: tanı msg yanıtı/bildirim ayrıntısı
            Some(name) => name.clone(),
            None => focused.filter(|f| layouts.iter().any(|l| &l.name == f)).unwrap_or_else(|| layouts[0].name.clone()),
        };
        let focus = (mode == ShotMode::Region).then(|| target.clone());
        self.shot.next_session += 1;
        let id = self.shot.next_session;
        let sender = cx.sender();
        let send = move |e: ShotEvent| {
            let _ = sender.send(ShellMessage::Screenshot(e));
        };
        let job_delivery = delivery.clone();
        let spawned = match mode {
            ShotMode::Region => {
                let client = self.client.clone();
                let visible = self.visible_workspaces();
                std::thread::Builder::new().name("nuxut-shot".into()).spawn(move || {
                    let result = shot::capture_outputs(&layouts).map(|frames| {
                        let windows = client
                            .as_ref()
                            .map(|c| shot::hypr::visible_windows(c, &visible).unwrap_or_else(|e| {
                                tracing::debug!("shot: pencereler alınamadı: {e}");
                                Vec::new()
                            }))
                            .unwrap_or_default();
                        (frames, windows)
                    });
                    send(ShotEvent::Frames { session: id, result });
                })
            }
            ShotMode::Output => std::thread::Builder::new().name("nuxut-shot".into()).spawn(move || {
                let result = capture::capture(&Source::Output(target)).map_err(|e| e.to_string()).and_then(|img| shot::deliver(&img, &job_delivery));
                send(ShotEvent::Done { session: id, result });
            }),
            ShotMode::Screen => std::thread::Builder::new().name("nuxut-shot".into()).spawn(move || {
                let result = shot::capture_outputs(&layouts)
                    .and_then(|frames| shot::stitch_frames(&frames))
                    .and_then(|img| shot::deliver(&img, &job_delivery));
                send(ShotEvent::Done { session: id, result });
            }),
            ShotMode::Window => {
                let client = self.client.clone().ok_or("window: Hyprland bağlantısı yok")?; // i18n: tanı msg yanıtı/bildirim ayrıntısı
                std::thread::Builder::new().name("nuxut-shot".into()).spawn(move || {
                    let result = shot::hypr::active_window(&client)
                        .map_err(|e| e.to_string())
                        .and_then(|w| w.ok_or_else(|| "odaklı pencere yok".to_owned())) // i18n: tanı msg yanıtı/bildirim ayrıntısı
                        .and_then(|rect| capture::capture(&Source::Region(rect)).map_err(|e| e.to_string()))
                        .and_then(|img| shot::deliver(&img, &job_delivery));
                    send(ShotEvent::Done { session: id, result });
                })
            }
        };
        spawned.map_err(|e| format!("iş başlatılamadı: {e}"))?; // i18n: tanı msg yanıtı/bildirim ayrıntısı
        tracing::info!(mode = mode.name(), "ekran görüntüsü");
        self.shot.session = Some(Session {
            id,
            delivery,
            reply: None,
            focus,
            views: Vec::new(),
            delivering: mode != ShotMode::Region,
        });
        Ok(())
    }

    /// Görünen workspace'ler (monitörlerin etkin ve açık özel workspace'leri).
    fn visible_workspaces(&self) -> Vec<i64> {
        self.model
            .hypr
            .monitors
            .iter()
            .flat_map(|m| [Some(m.active_workspace.id), m.special().map(|w| w.id)])
            .flatten()
            .collect()
    }

    pub(super) fn on_shot(&mut self, cx: &mut Context<Self>, event: ShotEvent) {
        match event {
            ShotEvent::Frames { session, result } => {
                if self.shot.session.as_ref().is_none_or(|s| s.id != session) {
                    return;
                }
                match result {
                    Ok((frames, windows)) => self.shot_open_overlays(cx, frames, &windows),
                    Err(e) => self.shot_finish(cx, Err(e)),
                }
            }
            ShotEvent::Done { session, result } => {
                if self.shot.session.as_ref().is_some_and(|s| s.id == session) {
                    self.shot_finish(cx, result);
                }
            }
        }
    }

    fn shot_open_overlays(&mut self, cx: &mut Context<Self>, frames: Vec<Frame>, windows: &[LogicalRect]) {
        let outputs = self.shot.outputs.clone();
        let Some(s) = &mut self.shot.session else { return };
        for frame in frames {
            // Yakalama sürerken kaldırılan çıkış atlanır.
            let Some((output, _)) = outputs.iter().find(|(_, l)| l.name == frame.name) else { continue };
            let keyboard = if s.focus.as_deref() == Some(frame.name.as_str()) { KeyboardMode::Exclusive } else { KeyboardMode::None };
            let spec = LayerSurfaceSpec {
                layer: Layer::Overlay,
                anchor: Anchor::ALL,
                exclusive_zone: -1,
                keyboard,
                ..LayerSurfaceSpec::new(NAMESPACE, *output)
            };
            let surface = cx.create_layer_surface(spec);
            cx.a11y_exclude(surface); // ekran görüntüsü seçimi henüz erişilebilir değil (boş pencere olmasın)
            cx.set_cursor(surface, nuxut::CursorShape::Crosshair);
            s.views.push(Overlay { surface, output: *output, view: ShotView::new(frame, windows) });
        }
        if s.views.is_empty() {
            self.shot_finish(cx, Err("çıkış yok".into())); // i18n: tanı msg yanıtı/bildirim ayrıntısı
        }
    }

    fn shot_destroy_views(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = &mut self.shot.session {
            for v in s.views.drain(..) {
                cx.destroy_surface(v.surface);
            }
        }
    }

    fn shot_cancel(&mut self, cx: &mut Context<Self>) {
        self.shot_destroy_views(cx);
        if let Some(s) = self.shot.session.take()
            && let Some(r) = s.reply
        {
            r.send(&shot::result_json(None));
        }
        tracing::info!("ekran görüntüsü iptal edildi");
    }

    /// Onay: katmanlar kalkar, kesim dondurulmuş kareden, teslim arka planda.
    fn shot_confirm(&mut self, cx: &mut Context<Self>, surface: SurfaceId, rect: shot::selection::FRect) {
        let Some(s) = &mut self.shot.session else { return };
        let Some(image) = s.views.iter().find(|v| v.surface == surface).and_then(|v| v.crop(rect)) else {
            return self.shot_finish(cx, Err("seçim boş".into())); // i18n: tanı msg yanıtı/bildirim ayrıntısı
        };
        s.delivering = true;
        let (id, delivery) = (s.id, s.delivery.clone());
        self.shot_destroy_views(cx);
        let sender = cx.sender();
        let spawned = std::thread::Builder::new().name("nuxut-shot".into()).spawn(move || {
            let result = shot::deliver(&image, &delivery);
            let _ = sender.send(ShellMessage::Screenshot(ShotEvent::Done { session: id, result }));
        });
        if let Err(e) = spawned {
            self.shot_finish(cx, Err(format!("iş başlatılamadı: {e}"))); // i18n: tanı msg yanıtı/bildirim ayrıntısı
        }
    }

    /// Oturum biter: yanıt, bildirim, kayıt numarası.
    fn shot_finish(&mut self, cx: &mut Context<Self>, result: Result<Delivered, String>) {
        self.shot_destroy_views(cx);
        let Some(s) = self.shot.session.take() else { return };
        match &result {
            Ok(d) => tracing::info!(saved = d.saved.is_some(), copied = d.copied, "ekran görüntüsü tamam"),
            Err(e) => tracing::warn!("ekran görüntüsü başarısız: {e}"),
        }
        if let Some(r) = s.reply {
            match &result {
                Ok(d) => r.send(&shot::result_json(Some(d))),
                Err(e) => r.send(&format!("error: {e}")),
            }
        }
        let recent = result.as_ref().ok().and_then(|d| d.saved.clone()).map(|p| self.shot.remember(p));
        if self.model.config.screenshot.notify {
            self.notice(cx, shot_notice(&result, recent));
        }
    }

    // --- yüzey olayları (mod.rs yönlendirir; yüzey bizim değilse `false`)

    pub(super) fn shot_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size) -> bool {
        let Some(v) = self.shot.view_mut(surface) else { return false };
        v.resized(size);
        cx.request_frame(surface);
        true
    }

    pub(super) fn shot_pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) -> bool {
        let Some(s) = &mut self.shot.session else { return false };
        let Some(i) = s.views.iter().position(|v| v.surface == surface) else { return false };
        let (outcome, cursor) = s.views[i].pointer(event);
        cx.set_cursor(surface, cursor);
        // Tek seçim: bu çıkışta seçim başladıysa diğerlerininki kalkar.
        if s.views[i].has_selection() {
            for (j, v) in s.views.iter_mut().enumerate() {
                if j != i && v.clear_selection() {
                    cx.request_frame(v.surface);
                }
            }
        }
        self.shot_outcome(cx, surface, outcome);
        true
    }

    pub(super) fn shot_key(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: &KeyEvent) -> bool {
        let Some(s) = &mut self.shot.session else { return false };
        if !s.views.iter().any(|v| v.surface == surface) {
            return false;
        }
        // Enter seçimi olan çıkışta onaylar (klavye hedef çıkışta olsa da seçim başka çıkışta olabilir).
        let target = match crate::screenshot_view::sel_key(event) {
            Some(shot::selection::SelKey::Confirm) => s.views.iter().position(|v| v.has_selection()),
            _ => None,
        };
        let (surface, outcome) = match target {
            Some(i) => (s.views[i].surface, s.views[i].confirm_key()),
            None => {
                let v = s.views.iter_mut().find(|v| v.surface == surface).expect("yukarıda bulundu");
                (surface, v.key(event))
            }
        };
        self.shot_outcome(cx, surface, outcome);
        true
    }

    fn shot_outcome(&mut self, cx: &mut Context<Self>, surface: SurfaceId, outcome: Outcome) {
        match outcome {
            Outcome::None => {}
            Outcome::Redraw => cx.request_frame(surface),
            Outcome::Cancel => self.shot_cancel(cx),
            Outcome::Confirm(rect) => self.shot_confirm(cx, surface, rect),
        }
    }

    /// Klavye odağı olayı bizim yüzeyimizse yutulur.
    pub(super) fn shot_owns(&self, surface: SurfaceId) -> bool {
        self.shot.session.as_ref().is_some_and(|s| s.views.iter().any(|v| v.surface == surface))
    }

    pub(super) fn draw_shot(&mut self, cx: &mut Context<Self>, surface: SurfaceId, _now: Instant, painter: &mut Painter) -> bool {
        let theme = cx.theme().clone();
        let Some(s) = &mut self.shot.session else { return false };
        let focus = s.focus.clone();
        let Some(v) = s.views.iter_mut().find(|v| v.surface == surface) else { return false };
        let hint = focus.as_deref() == Some(v.name());
        v.draw(painter, &theme, hint);
        true
    }
}

/// Bitiş bildirimi. Kaydedildiyse "Aç" ve "Klasörde göster" düğmeleri (`shot open|reveal <n>`).
fn shot_notice(result: &Result<Delivered, String>, recent: Option<u32>) -> InternalNotice {
    match result {
        Ok(d) => {
            let mut body = Vec::new();
            if let Some(p) = &d.saved {
                body.push(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            }
            if d.copied {
                body.push(t!("shell-screenshot-copied"));
            }
            if let Some(w) = &d.warning {
                body.push(w.clone());
            }
            let actions = recent
                .map(|n| {
                    vec![
                        InternalAction::new(t!("shell-screenshot-action-open"), format!("shot open {n}")),
                        InternalAction::new(t!("shell-screenshot-action-reveal"), format!("shot reveal {n}")),
                    ]
                })
                .unwrap_or_default();
            let summary = if d.saved.is_some() { t!("shell-screenshot-saved-title") } else { t!("shell-screenshot-copied-title") };
            let notice = InternalNotice::popup(summary, body.join("\n"), actions);
            // Önizleme teslim thread'inde küçültüldü; burada yalnızca piksel Arc'ı taşınır.
            match &d.preview {
                Some(p) => notice.with_image(NotifyIcon::Pixmap { width: p.width, height: p.height, rgba: p.pixels.clone() }),
                None => notice,
            }
        }
        Err(e) => InternalNotice::popup(t!("shell-screenshot-failed-title"), e.clone(), Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{self, Command};

    #[test]
    fn notice_actions_round_trip_through_ipc() {
        crate::i18n::use_turkish_for_tests();
        let d = Delivered { saved: Some("/p/Ekran görüntüsü 1.png".into()), copied: true, warning: None, preview: None };
        let n = shot_notice(&Ok(d), Some(7));
        assert_eq!(n.summary, "Ekran görüntüsü kaydedildi");
        assert!(n.popup);
        assert_eq!(n.body, "Ekran görüntüsü 1.png\nPanoya kopyalandı");
        assert_eq!(n.actions.len(), 2);
        assert_eq!(ipc::parse(&n.actions[0].command), Ok(Command::Shot(ShotCommand::Open(7))));
        assert_eq!(ipc::parse(&n.actions[1].command), Ok(Command::Shot(ShotCommand::Reveal(7))));
        let n = shot_notice(&Ok(Delivered { saved: None, copied: true, warning: None, preview: None }), None);
        assert_eq!(n.summary, "Ekran görüntüsü panoya kopyalandı");
        assert!(n.actions.is_empty());
        let n = shot_notice(&Err("grim yok".into()), None);
        assert_eq!((n.summary.as_str(), n.body.as_str()), ("Ekran görüntüsü alınamadı", "grim yok"));
    }

    #[test]
    fn notice_carries_preview_only_on_success() {
        crate::i18n::use_turkish_for_tests();
        let img = crate::services::screenshot::geometry::RgbaImage::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
        let d = Delivered { saved: None, copied: true, warning: None, preview: Some(img.clone()) };
        let n = shot_notice(&Ok(d), None);
        assert_eq!(n.image, NotifyIcon::Pixmap { width: 2, height: 1, rgba: img.pixels.clone() });
        assert!(crate::services::notifications::internal::image_fits(&n.image), "model sınırına uyar");
        let n = shot_notice(&Ok(Delivered { saved: None, copied: true, warning: None, preview: None }), None);
        assert!(n.image.is_none());
        let n = shot_notice(&Err("grim yok".into()), None);
        assert!(n.image.is_none(), "başarısızlıkta görsel yok");
    }

    #[test]
    fn recent_list_is_bounded() {
        let mut r = ShotRuntime::default();
        let first = r.remember("/a.png".into());
        for i in 0..MAX_RECENT {
            r.remember(format!("/{i}.png").into());
        }
        assert!(r.recent(first).is_none(), "en eski düştü");
        assert_eq!(r.recent.len(), MAX_RECENT);
        assert_eq!(r.recent(first + MAX_RECENT as u32).map(|p| p.as_path()), Some(std::path::Path::new("/15.png")));
    }
}
