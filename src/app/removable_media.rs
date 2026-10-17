//! Çıkarılabilir medya (`services::removable_media`): takılınca "Aç / Çıkar" bildirimi,
//! `msg removable …` komutları, sonuç bildirimleri. Kararlar: `docs/design/removable-media.md`.

use std::collections::HashMap;
use std::time::Instant;

use nuxut::Context;
use nuxut::i18n::t;

use super::{Shell, ShellMessage};
use crate::services::notifications::{InternalAction, InternalNotice};
use crate::services::removable_media::{
    MediaAction, MediaCommand, MediaEvent, MediaOptions, MediaService, Volume, list_json,
};

/// Uygulama tarafı durum.
#[derive(Debug, Default)]
pub(super) struct MediaRuntime {
    service: Option<MediaService>,
    available: bool,
    volumes: Vec<Volume>,
    /// Birim kimliği → onun "takıldı" bildirimi (sökülünce kapatılır).
    notices: HashMap<String, u32>,
}

/// Test örneği (`NUXUT_SHELL_INSTANCE`) gerçek sistem bus'ında udisks'i etkinleştirmez ve
/// bağla/ayır/çıkar çağırmaz (kullanıcının aygıtları); `DBUS_SYSTEM_BUS_ADDRESS` açıkça (sahte
/// bir bus'a) yönlendirilmişse tam çalışır (`app/idle.rs` `logind_allowed` ile aynı kural).
fn full_access(instance: Option<&str>, system_bus: Option<&str>) -> bool {
    instance.is_none_or(str::is_empty) || system_bus.is_some_and(|a| !a.is_empty())
}

/// "Takıldı" bildirimi: başlık sabit, gövde birimin adı; düğmeler `removable open|eject <id>`.
pub(super) fn added_notice(v: &Volume) -> InternalNotice {
    InternalNotice::popup(
        t!("shell-media-added-title"),
        v.display_name(),
        vec![
            InternalAction::new(t!("shell-media-action-open"), format!("removable open {}", v.id)),
            InternalAction::new(t!("shell-media-action-eject"), format!("removable eject {}", v.id)),
        ],
    )
}

/// Eylem sonucu bildirimi (`None`: gösterilmez — açma başarısı ve vazgeçilen polkit istemi).
pub(super) fn result_notice(e: &MediaEvent) -> Option<InternalNotice> {
    match e {
        MediaEvent::Ejected { name, .. } => {
            Some(InternalNotice::popup(t!("shell-media-ejected-title"), t!("shell-media-ejected-body", name = name.clone()), Vec::new()))
        }
        MediaEvent::Failed { error, .. } if error.silent() => None,
        MediaEvent::Failed { name, action, error, .. } => {
            let title = match action {
                MediaAction::Open => t!("shell-media-open-failed-title", name = name.clone()),
                MediaAction::Eject => t!("shell-media-eject-failed-title", name = name.clone()),
            };
            Some(InternalNotice::popup(title, error.message(), Vec::new()))
        }
        _ => None,
    }
}

impl Shell {
    pub(super) fn start_removable_media(&mut self, cx: &mut Context<Self>) {
        if !self.model.config.removable_media.enabled || self.removable.service.is_some() {
            return;
        }
        let instance = std::env::var(crate::ipc::INSTANCE_ENV).ok();
        let system_bus = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").ok();
        let full = full_access(instance.as_deref(), system_bus.as_deref());
        if !full {
            tracing::info!("çıkarılabilir medya: test örneği, yalnızca izleme (etkinleştirme ve eylem yok)");
        }
        let options = MediaOptions { activate: full, actions: full, ..MediaOptions::default() };
        let sender = cx.sender();
        match MediaService::spawn(options, move |e| {
            let _ = sender.send(ShellMessage::RemovableMedia(e));
        }) {
            Ok(s) => self.removable.service = Some(s),
            Err(e) => tracing::warn!("çıkarılabilir medya: izleyici başlatılamadı: {e}"),
        }
    }

    /// `[removable_media] enabled` değişti: servisi başlat ya da durdur.
    pub(super) fn removable_media_config_changed(&mut self, cx: &mut Context<Self>) {
        if self.model.config.removable_media.enabled {
            self.start_removable_media(cx);
        } else if self.removable.service.take().is_some() {
            self.removable.available = false;
            self.removable.volumes.clear();
            let ids: Vec<u32> = self.removable.notices.drain().map(|(_, n)| n).collect();
            self.close_media_notices(cx, &ids);
        }
    }

    pub(super) fn on_removable_media(&mut self, cx: &mut Context<Self>, e: MediaEvent) {
        match e {
            MediaEvent::Volumes { available, volumes } => {
                self.removable.available = available;
                self.removable.volumes = volumes;
            }
            MediaEvent::Added(v) => {
                if !self.model.config.removable_media.notify {
                    return;
                }
                if let Some(old) = self.removable.notices.remove(&v.id) {
                    self.model.notifications.remove(old);
                }
                let id = self.model.notifications.add_internal(added_notice(&v), Instant::now());
                self.removable.notices.insert(v.id, id);
                self.model.refresh_unread();
                self.sync_notifications(cx);
                self.model_changed(cx);
            }
            // Sökülen birimin bekleyen bildirimi kapanır (düğmeleri artık anlamsız).
            MediaEvent::Removed { id } => {
                if let Some(n) = self.removable.notices.remove(&id) {
                    self.close_media_notices(cx, &[n]);
                }
            }
            MediaEvent::Opened { mount_point, .. } => {
                let _ = std::thread::Builder::new().name("nuxut-media-open".into()).spawn(move || {
                    crate::services::screenshot::save::open_path(&mount_point);
                });
            }
            e @ (MediaEvent::Ejected { .. } | MediaEvent::Failed { .. }) => {
                // `msg` ile çıkarıldıysa "takıldı" bildirimi hâlâ duruyordur (düğmeyle
                // çıkarıldıysa merkez onu zaten kaldırdı).
                if let MediaEvent::Ejected { id, .. } = &e
                    && let Some(n) = self.removable.notices.remove(id)
                {
                    self.model.notifications.remove(n);
                }
                if let Some(n) = result_notice(&e) {
                    self.notice(cx, n);
                }
            }
        }
    }

    /// Bildirimleri kaldırır (popup + merkez) ve ekranı günceller; hiçbiri kalmamışsa
    /// (kullanıcı kapatmış) bir şey yapmaz.
    fn close_media_notices(&mut self, cx: &mut Context<Self>, ids: &[u32]) {
        let mut any = false;
        for &n in ids {
            any |= self.model.notifications.remove(n).is_some();
        }
        if any {
            self.model.refresh_unread();
            self.sync_notifications(cx);
            self.model_changed(cx);
        }
    }

    /// `msg removable …` ve bildirim düğmeleri.
    pub(super) fn removable_command(&mut self, c: MediaCommand) -> Result<String, String> {
        let Some(service) = &self.removable.service else {
            return Err("çıkarılabilir medya kapalı ([removable_media] enabled = false)".into()); // i18n: tanı msg yanıtı/bildirim ayrıntısı
        };
        let known = |id: &str| self.removable.volumes.iter().any(|v| v.id == id);
        let sent = match &c {
            MediaCommand::List => return Ok(list_json(self.removable.available, &self.removable.volumes)),
            MediaCommand::Open(id) | MediaCommand::Eject(id) if !known(id) => {
                return Err(format!("bilinmeyen birim \"{id}\""));
            }
            MediaCommand::Open(id) => service.open(id),
            MediaCommand::Eject(id) => service.eject(id),
        };
        if sent { Ok("ok".into()) } else { Err("busy: çıkarılabilir medya kuyruğu dolu".into()) } // i18n: tanı msg yanıtı/bildirim ayrıntısı
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{self, Command};
    use crate::services::removable_media::{MediaError, VolumeName};

    fn vol() -> Volume {
        Volume {
            id: "sdb1".into(),
            name: VolumeName::Label("YEDEK".into()),
            mount_points: Vec::new(),
            drive: "/org/freedesktop/UDisks2/drives/X".into(),
            ejectable: true,
            can_power_off: true,
            crypto_backing: None,
            size: 1,
        }
    }

    #[test]
    fn added_notice_actions_round_trip_through_ipc() {
        crate::i18n::use_turkish_for_tests();
        let n = added_notice(&vol());
        assert!(n.popup);
        assert_eq!(n.body, "YEDEK");
        assert_eq!(n.actions.len(), 2);
        assert_eq!(n.actions[0].label, "Aç");
        assert_eq!(n.actions[1].label, "Çıkar");
        assert_eq!(ipc::parse(&n.actions[0].command), Ok(Command::Removable(MediaCommand::Open("sdb1".into()))));
        assert_eq!(ipc::parse(&n.actions[1].command), Ok(Command::Removable(MediaCommand::Eject("sdb1".into()))));
        assert!(n.actions.iter().all(|a| a.command.len() <= crate::services::notifications::internal::MAX_INTERNAL_COMMAND_BYTES));
    }

    #[test]
    fn result_notices() {
        crate::i18n::use_turkish_for_tests();
        let ejected = result_notice(&MediaEvent::Ejected { id: "sdb1".into(), name: "YEDEK".into() }).unwrap();
        assert!(ejected.popup && ejected.body.contains("YEDEK"));
        let busy = MediaEvent::Failed { id: "sdb1".into(), name: "YEDEK".into(), action: MediaAction::Eject, error: MediaError::Busy };
        let n = result_notice(&busy).unwrap();
        assert!(n.summary.contains("YEDEK"), "{}", n.summary);
        assert_eq!(n.body, MediaError::Busy.message());
        let cancelled = MediaEvent::Failed { id: "x".into(), name: "x".into(), action: MediaAction::Open, error: MediaError::Cancelled };
        assert!(result_notice(&cancelled).is_none());
        assert!(result_notice(&MediaEvent::Opened { id: "x".into(), mount_point: "/m".into() }).is_none());
    }

    #[test]
    fn test_instance_on_real_system_bus_is_listen_only() {
        assert!(full_access(None, None));
        assert!(full_access(Some(""), None));
        assert!(!full_access(Some("test-media"), None));
        assert!(!full_access(Some("test-media"), Some("")));
        assert!(full_access(Some("test-media"), Some("unix:path=/tmp/x")));
    }
}
