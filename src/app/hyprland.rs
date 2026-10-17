//! Hyprland bağlantısı: olay servisi, workspace geçişi, tam ekranın exclusion/input'a etkisi.

use std::time::Instant;

use nuxut::Context;

use super::{Shell, ShellMessage};
use crate::services::hyprland::{HyprlandEvent, HyprlandService, WorkspaceTarget};

impl Shell {
    pub(super) fn start_hyprland(&mut self, cx: &mut Context<Self>) {
        let sender = cx.sender();
        match HyprlandService::spawn_from_env(move |e| {
            // Runtime kapandıysa olay düşer; servis Shell ile birlikte kapanır.
            let _ = sender.send(ShellMessage::Hyprland(e));
        }) {
            Ok(service) => {
                self.client = Some(service.client().clone());
                self.hypr = Some(service);
            }
            Err(e) => tracing::error!("Hyprland servisi başlatılamadı, workspace göstergesi boş kalacak: {e}"),
        }
    }

    pub(super) fn on_hyprland(&mut self, cx: &mut Context<Self>, e: HyprlandEvent) {
        // Ekran yakalama sayacı: yalnız gizlilik göstergesini ilgilendirir (exclusion/monitör işi yok).
        if let HyprlandEvent::Screencast(on) = e {
            if self.model.apply_hyprland(HyprlandEvent::Screencast(on)) {
                self.model_changed(cx);
            }
            return;
        }
        if matches!(e, HyprlandEvent::ConfigReloaded) {
            // Ayar yeniden yüklemesi büyütmeyi ayar değerine döndürür: kabuğun büyütmesi geri gelir.
            self.magnifier_config_reloaded();
            return;
        }
        if let HyprlandEvent::ActiveLayout { layout, .. } = &e {
            self.osk_compositor_layout(cx, layout);
        }
        if self.model.apply_hyprland(e) {
            self.hyprland_changed(cx);
            self.displays_monitors_changed();
        }
    }

    /// Hyprland durumu değişti: tam ekran değişen ekranlarda exclusion ve input da eşlenir.
    fn hyprland_changed(&mut self, cx: &mut Context<Self>) {
        let theme = cx.theme().clone();
        let config = self.model.config.clone();
        let now = Instant::now();
        for i in 0..self.screens.len() {
            let was_fullscreen = self.screens[i].is_fullscreen();
            if self.screens[i].sync_model(&self.model, &theme, now) {
                if self.screens[i].is_fullscreen() != was_fullscreen {
                    Self::sync_exclusions(cx, &mut self.screens[i], &config);
                    Self::apply_input(cx, &mut self.screens[i], &config);
                }
                cx.request_frame(self.screens[i].main);
            }
        }
    }

    pub(super) fn switch_workspace(&self, id: i64) {
        let Ok(id) = u32::try_from(id) else { return };
        self.dispatch_workspace(WorkspaceTarget::Id(id));
    }

    /// Bar uç eylemi `workspace` kaydırması: workspace widget'ıyla aynı yol ve ölçüt
    /// (`[bar.widget.workspaces] scroll = all|occupied`; `off` yalnızca widget'ı kapatır, açıkça
    /// atanmış uç eylemi `all` ile çalışır). `steps` > 0 sonraki workspace.
    pub(super) fn scroll_workspace(&mut self, monitor: &str, steps: i32) {
        use crate::widgets::registry::WorkspaceScroll;
        use crate::widgets::workspaces::page::notable_ids;
        let mode = match self.model.config.bar.widgets.workspaces.scroll {
            WorkspaceScroll::Off => WorkspaceScroll::All,
            m => m,
        };
        let active = self.model.hypr.monitor(monitor).map(|m| m.active_workspace.id);
        let notable = notable_ids(&self.model.hypr);
        if let Some(id) = self.workspace_scroller.step(mode, &notable, active, steps, Instant::now()) {
            self.switch_workspace(id);
        }
    }

    fn dispatch_workspace(&self, target: WorkspaceTarget) {
        let Some(client) = self.client.clone() else { return };
        // socket1 isteği kısa ama zaman aşımına kadar bloklayabilir: ana döngü dışında.
        let spawned = std::thread::Builder::new().name("nuxut-dispatch".into()).spawn(move || {
            if let Err(e) = client.dispatch_workspace(target) {
                tracing::warn!("workspace {target:?} açılamadı: {e}");
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("dispatch thread'i açılamadı: {e}");
        }
    }
}

impl Shell {
    /// Bar düzen göstergesine tık: ana klavyede sıradaki düzen. Hyprland'e yazma yalnız komut
    /// izni varken (test örneği kullanıcının oturumuna asla; `SyncEnv::may_control`). Yeni düzen
    /// `activelayout` olayıyla geri gelir (iyimser güncelleme yok).
    pub(super) fn keyboard_layout_next(&self) {
        use crate::services::hypr_config::sync::SyncEnv;
        if !SyncEnv::from_process(self.apply_env.clone()).may_control() {
            tracing::info!("klavye düzeni değiştirilmedi: bu örnek Hyprland'e komut göndermez");
            return;
        }
        let Some(client) = &self.client else { return };
        if let Err(e) = client.switch_layout_next() {
            tracing::warn!("klavye düzeni değiştirilemedi: {e}");
        }
    }
}
