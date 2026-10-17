//! Bildirim sunucusu bağlantısı: sunucu, model eşleme, popup süre dolumu timer'ı, ölçek.

use std::time::Instant;

use nuxut::{Context, TimerId};

use super::{Shell, ShellMessage};
use crate::ipc;
use crate::services::image_sandbox;
use crate::services::notifications::{CloseReason, InternalNotice, NotificationServer, NotifyError, NotifyEvent, ServerOptions};
use crate::widgets::NotifyCommand;

impl Shell {
    pub(super) fn start_notifications(&mut self, cx: &mut Context<Self>) {
        let sender = cx.sender();
        let options = ServerOptions { sandbox: image_sandbox::decoder(&self.image_sandbox), ..ServerOptions::default() };
        match NotificationServer::spawn_with(options, move |e| {
            let _ = sender.send(ShellMessage::Notify(e));
        }) {
            Ok(server) => {
                let handle = server.handle();
                self.notify = Some((server, handle));
                self.model.notifications_active = true;
            }
            Err(NotifyError::NameTaken { owner }) => {
                tracing::warn!("bildirim sunucusu adı başka bir süreçte ({owner}); shell bildirimsiz çalışacak");
            }
            Err(e) => tracing::warn!("bildirim sunucusu başlatılamadı: {e}"),
        }
    }

    pub(super) fn on_notify(&mut self, cx: &mut Context<Self>, e: NotifyEvent) {
        let changed = self.model.apply_notify(e, Instant::now());
        self.sync_notifications(cx);
        if changed {
            self.model_changed(cx);
        }
    }

    pub(super) fn on_notify_command(&mut self, cx: &mut Context<Self>, cmd: NotifyCommand) {
        let internal = self.notify_command(cmd, Instant::now());
        self.sync_notifications(cx);
        self.model_changed(cx);
        if let Some(line) = internal {
            self.run_internal_command(cx, &line);
        }
    }

    /// Shell'in kendi bildirimini merkeze ekler (D-Bus'sız; sessizse popup ve ses yok) ve
    /// ekranı günceller. Bildirim sunucusu çalışmasa da (ad başkasında) merkezde görünür.
    pub(super) fn notice(&mut self, cx: &mut Context<Self>, notice: InternalNotice) {
        self.post_notice(notice);
        self.sync_notifications(cx);
        self.model_changed(cx);
    }

    /// [`notice`](Self::notice)'ın bağlamsız hali (döngü başlamadan, ör. açılıştaki config
    /// göçü): yalnızca model; ilk kare onu çizer. Popup'lı bildirimin süresi sonraki
    /// `sync_notifications`'ta kurulur.
    pub fn post_notice(&mut self, notice: InternalNotice) {
        self.model.notifications.add_internal(notice, Instant::now());
        self.model.refresh_unread();
    }

    /// Dahili bildirim düğmesinin komutu: `msg` ile aynı ayrıştırıcı ve yürütücü.
    fn run_internal_command(&mut self, cx: &mut Context<Self>, line: &str) {
        match ipc::parse(line).and_then(|c| self.execute(cx, c)) {
            Ok(_) => tracing::info!(command = line, "bildirim eylemi yürütüldü"),
            Err(e) => tracing::warn!(command = line, "bildirim eylemi başarısız: {e}"),
        }
    }

    /// Popup süre dolumu timer'ıysa işler ve `true` döner.
    pub(super) fn notify_timer_fired(&mut self, cx: &mut Context<Self>, id: TimerId) -> bool {
        if self.notify_timer != Some(id) {
            return false;
        }
        self.notify_timer = None;
        let expired = self.model.notifications.expire(Instant::now());
        self.sync_notifications(cx);
        if !expired.is_empty() {
            self.model_changed(cx);
        }
        true
    }

    /// Bildirim modelinin kendi kapattıklarını istemcilere bildirir ve popup süre dolumu
    /// timer'ını bir sonraki son tarihe kurar.
    pub(super) fn sync_notifications(&mut self, cx: &mut Context<Self>) {
        let closed = self.model.notifications.take_closed();
        if let Some((_, handle)) = &self.notify {
            for (id, reason) in closed {
                handle.close(id, reason);
            }
        }
        if let Some(t) = self.notify_timer.take() {
            cx.cancel_timer(t);
        }
        if let Some(deadline) = self.model.notifications.next_deadline() {
            self.notify_timer = Some(cx.set_timer(deadline.saturating_duration_since(Instant::now())));
        }
    }

    /// Çıkış ölçeklerinin en büyüğü değiştiyse bildirim görsellerinin çözünürlüğü güncellenir
    /// (çok monitörde en keskin çıkışa göre); yalnızca değişince.
    pub(super) fn sync_notify_scale(&mut self) {
        let scale = self.main_scales.iter().map(|(_, s)| *s).filter(|s| s.is_finite() && *s > 0.0).fold(1.0_f32, f32::max);
        let Some((_, h)) = &self.notify else { return };
        if self.notify_scale != Some(scale.to_bits()) {
            h.set_output_scale(scale);
            self.notify_scale = Some(scale.to_bits());
        }
    }

    /// Dahili bildirim eylemi seçildiyse yürütülecek komut satırını döner.
    fn notify_command(&mut self, cmd: NotifyCommand, now: Instant) -> Option<String> {
        let center = &mut self.model.notifications;
        let handle = self.notify.as_ref().map(|(_, h)| h);
        let mut internal = None;
        match cmd {
            NotifyCommand::Dismiss(id) => {
                if let (Some(id), Some(h)) = (center.remove(id), handle) {
                    h.close(id, CloseReason::Dismissed);
                }
            }
            NotifyCommand::HidePopup(id) => {
                center.dismiss_popup(id);
            }
            NotifyCommand::Invoke { id, key } => {
                if let Some(line) = center.internal_command(id, &key) {
                    // D-Bus'taki gibi: eylem seçilince (resident değil) bildirim kapanır.
                    internal = Some(line.to_owned());
                    center.remove(id);
                } else if let Some(h) = handle {
                    h.invoke_action(id, &key);
                }
            }
            NotifyCommand::ClearAll => {
                let ids = center.clear_all();
                if let Some(h) = handle {
                    for id in ids {
                        h.close(id, CloseReason::Dismissed);
                    }
                }
            }
            NotifyCommand::MarkAllRead => center.mark_all_read(),
            NotifyCommand::SetDnd(on) => center.set_dnd(on),
            NotifyCommand::Hover { id, hovered } => {
                center.set_hovered(id, hovered, now);
            }
            NotifyCommand::OpenLink(url) => {
                // Bağlantı sunucuda doğrulanmıştır; open_link yeniden doğrular ve portal ile açar.
                if let Some(h) = handle
                    && !h.open_link(&url, None)
                {
                    tracing::warn!("bağlantı açılamadı");
                }
            }
        }
        self.model.refresh_unread();
        internal
    }
}
