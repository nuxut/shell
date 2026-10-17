//! `msg snapshot list|show|restore` (ADR-0001 §9.3). Gövdeler saf `services::apply`'da; burada
//! yalnızca thread ve canlı uygulama: snapshot deposu kilidi 5 sn'ye kadar bekleyebildiği için
//! her komut kısa ömürlü bir thread'de çalışır ve yanıtı o verir (UI beklemez). Geri yükleme
//! bir şey yazdıysa ana thread'e bildirilir: `hypr.source` → Hyprland yeniden yüklenir ve durum
//! yenilenir, `migration.shell` → shell.toml yeniden yüklenir ve göç kaydı unutulur.

use nuxut::Context;
use nuxut_apply::{RestoreMode, SettingId};

use super::{Shell, ShellMessage};
use crate::ipc::SnapshotCommand;
use crate::services::apply;

impl Shell {
    /// `msg snapshot …`: yanıtı komut thread'i verir.
    pub(super) fn snapshot_command(&mut self, cx: &mut Context<Self>, cmd: SnapshotCommand, reply: nuxut::ipc::Reply) {
        let env = self.apply_env.clone();
        let sender = cx.sender();
        let spawned = std::thread::Builder::new().name("nuxut-snapshot".into()).spawn(move || {
            let text = match cmd {
                SnapshotCommand::List { setting, limit } => apply::snapshot_list(&env, setting, limit),
                SnapshotCommand::Show(id) => apply::snapshot_show(&env, &id),
                SnapshotCommand::Restore { id, force, setting } => {
                    let mode = if force { RestoreMode::Force } else { RestoreMode::Safe };
                    let apply::Restored { json, setting: restored, values } =
                        apply::snapshot_restore(&env, &id, mode, setting.as_ref());
                    if let Some(setting) = restored {
                        let _ = sender.send(ShellMessage::SnapshotRestored(setting, id, Box::new(values)));
                    }
                    Ok(json)
                }
            };
            match text {
                Ok(t) => reply.send(&t),
                Err(e) => reply.send(&format!("error: {e}")),
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("snapshot thread'i açılamadı: {e}");
        }
    }

    /// Geri yükleme yazdı (`msg snapshot restore`): ayarın canlı karşılığı.
    pub(super) fn on_snapshot_restored(
        &mut self,
        cx: &mut Context<Self>,
        setting: SettingId,
        id: nuxut_apply::SnapshotId,
        values: Box<nuxut_apply::RestoredValues>,
    ) {
        tracing::info!(setting = %setting, "snapshot geri yüklendi");
        match setting.as_str() {
            apply::HYPR_SOURCE => self.hypr_reload_after_restore(),
            apply::MIGRATION_SHELL => {
                self.migration_undo.forget_snapshot(&id);
                self.reload_config(cx);
            }
            // Birleşik görünüm: canlı eylemler (KDE sinyalleri; geri yüklenen imleçle setcursor).
            s if apply::LOOK_SETTINGS.contains(&s) => self.look_restored(s, values),
            // Varsayılan uygulamalar: canlı karşılık yok (Ayarlar odakta yeniden sorar).
            s if apply::DEFAULTS_SETTINGS.contains(&s) => {}
            // shell.toml anahtarları: dosya izleyicisi de yakalar; hemen uygula.
            _ => self.reload_config(cx),
        }
    }
}
