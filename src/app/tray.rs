//! Sistem tepsisi bağlantısı: servis, komutlar, çıkış ölçeğine göre ikon boyutu.

use nuxut::Context;

use super::{Shell, ShellMessage};
use crate::services::image_sandbox;
use crate::services::tray::{MenuEventKind, ScrollOrientation, TrayEvent, TrayOptions, TrayService};
use crate::widgets::TrayCommand;

impl Shell {
    pub(super) fn start_tray(&mut self, cx: &mut Context<Self>) {
        let sender = cx.sender();
        let icon_px = self.wanted_tray_icon_px();
        let sandbox = image_sandbox::decoder(&self.image_sandbox);
        match TrayService::spawn(TrayOptions { icon_px, sandbox, ..TrayOptions::default() }, move |e| {
            let _ = sender.send(ShellMessage::Tray(e));
        }) {
            Ok(service) => {
                let handle = service.handle();
                self.tray = Some((service, handle));
                self.tray_icon_px = Some(icon_px);
            }
            Err(e) => tracing::warn!("tepsi servisi başlatılamadı: {e}"),
        }
    }

    pub(super) fn on_tray(&mut self, cx: &mut Context<Self>, e: TrayEvent) {
        if let TrayEvent::Failed(reason) = &e {
            tracing::warn!("tepsi servisi çalışmıyor: {reason}");
        }
        if self.model.apply_tray(&e) {
            self.model_changed(cx);
        }
    }

    fn wanted_tray_icon_px(&self) -> u32 {
        tray_icon_px(self.model.config.bar.widgets.tray.icon_size, self.main_scales.iter().map(|(_, s)| *s))
    }

    /// Tepsi ikonlarının çözüleceği boyutu (ikon boyu × en büyük çıkış ölçeği) servise bildirir;
    /// yalnızca değişince.
    pub(super) fn sync_tray_icon_size(&mut self) {
        let px = self.wanted_tray_icon_px();
        let Some((_, h)) = &self.tray else { return };
        if self.tray_icon_px != Some(px) && h.set_icon_size(px) {
            tracing::debug!(px, "tepsi ikon boyutu");
            self.tray_icon_px = Some(px);
        }
    }

    pub(super) fn tray_command(&self, cmd: TrayCommand) {
        let Some((_, h)) = &self.tray else { return };
        let ok = match cmd {
            TrayCommand::Activate { item, x, y } => h.activate(&item, x, y),
            TrayCommand::SecondaryActivate { item, x, y } => h.secondary_activate(&item, x, y),
            TrayCommand::Scroll { item, delta, horizontal } => {
                let o = if horizontal { ScrollOrientation::Horizontal } else { ScrollOrientation::Vertical };
                h.scroll(&item, delta, o)
            }
            TrayCommand::OpenMenu { item, request } => h.open_menu(&item, request),
            TrayCommand::CloseMenu(item) => h.close_menu(&item),
            TrayCommand::MenuAboutToShow { item, ids } => h.menu_about_to_show(&item, &ids),
            TrayCommand::MenuClicked { item, id } => h.menu_event(&item, id, MenuEventKind::Clicked),
            TrayCommand::ContextMenu { item, x, y } => h.context_menu(&item, x, y),
        };
        if !ok {
            tracing::debug!("tepsi komutu iletilemedi (kuyruk dolu ya da servis durmuş)");
        }
    }
}

/// Tepsi ikonlarının fiziksel boyutu: ikon boyu × en büyük çıkış ölçeği (çıkış yoksa 1),
/// yukarı yuvarlanır; böylece en keskin çıkışta da ikon küçültülerek çizilir, büyütülmez.
fn tray_icon_px(icon_size: f32, scales: impl Iterator<Item = f32>) -> u32 {
    let scale = scales.filter(|s| s.is_finite() && *s > 0.0).fold(1.0_f32, f32::max);
    (icon_size * scale).ceil().max(1.0) as u32
}

#[cfg(test)]
mod tests {
    use super::tray_icon_px;

    #[test]
    fn tray_icon_px_uses_largest_scale() {
        assert_eq!(tray_icon_px(18.0, std::iter::empty()), 18);
        assert_eq!(tray_icon_px(18.0, [1.0, 2.0, 1.5].into_iter()), 36);
        // Kesirli ölçek yukarı yuvarlanır.
        assert_eq!(tray_icon_px(18.0, [1.25].into_iter()), 23);
        // Bozuk ölçekler yok sayılır.
        assert_eq!(tray_icon_px(18.0, [f32::NAN, -2.0, f32::INFINITY].into_iter()), 18);
    }
}
