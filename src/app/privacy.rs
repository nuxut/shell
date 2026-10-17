//! Gizlilik göstergesi: kamera izleyicisi yalnızca bar'da `privacy` widget'ı kamerayı
//! gösterirken çalışır (yoksa inotify da yok). Mikrofon ses servisinden, ekran yakalama
//! Hyprland `screencast` olayından gelir; onlar için ek servis yok.

use nuxut::Context;

use super::{Shell, ShellMessage};
use crate::config::BarSection;
use crate::services::camera::{CameraEvent, CameraOptions, CameraService};
use crate::widgets::WidgetKind;

/// Görsel test kancası: yalnız ikincil (test) örnekte (`NUXUT_SHELL_INSTANCE`) `NUXUT_TEST_CAMERA_ROOT`
/// verilirse aygıtlar `<kök>/dev`, procfs `<kök>/proc` altından okunur (kamerasız ortamda sahte
/// aygıt). Ana örnek bu değişkeni yok sayar.
const TEST_CAMERA_ROOT_ENV: &str = "NUXUT_TEST_CAMERA_ROOT";

fn camera_options() -> CameraOptions {
    let test_root = std::env::var_os("NUXUT_SHELL_INSTANCE")
        .filter(|i| !i.is_empty())
        .and_then(|_| std::env::var_os(TEST_CAMERA_ROOT_ENV))
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute());
    match test_root {
        Some(root) => CameraOptions { dev_dir: root.join("dev"), proc_dir: root.join("proc"), ..CameraOptions::default() },
        None => CameraOptions::default(),
    }
}

impl Shell {
    /// İzleyiciyi config'e eşler: gerekiyorsa başlatır, gerekmiyorsa durdurur ve listeyi boşaltır.
    pub(super) fn sync_camera_service(&mut self, cx: &mut Context<Self>) {
        let bar = &self.model.config.bar;
        let wanted = bar.widgets.privacy.camera
            && BarSection::ALL.into_iter().any(|s| bar.widgets_in(s).contains(&WidgetKind::Privacy));
        match (self.camera.is_some(), wanted) {
            (true, false) => {
                self.camera = None;
                if std::mem::take(&mut self.model.camera).in_use() {
                    self.model_changed(cx);
                }
            }
            (false, true) => {
                let sender = cx.sender();
                match CameraService::spawn(camera_options(), move |e| {
                    let _ = sender.send(ShellMessage::Camera(e));
                }) {
                    Ok(svc) => self.camera = Some(svc),
                    Err(e) => tracing::warn!("kamera izleyici başlatılamadı: {e}"),
                }
            }
            _ => {}
        }
    }

    pub(super) fn on_camera(&mut self, cx: &mut Context<Self>, e: CameraEvent) {
        // Durdurulduktan sonra kuyrukta kalmış olay.
        if self.camera.is_none() {
            return;
        }
        let CameraEvent::State(state) = e;
        if self.model.camera != state {
            self.model.camera = state;
            self.model_changed(cx);
        }
    }
}
