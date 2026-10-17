//! İlk açılış / Hoş geldin (ADR-0001 §10): `msg setup …` ve açılışta Ayarlar'ın Hoş geldin
//! penceresini başlatma. Dosya kuralları ve kararlar saf `services::setup`'ta.

use std::path::PathBuf;

use super::Shell;
use crate::ipc::SetupCommand;
use crate::services::setup::{self, HyprBrief, MigrationBrief, SetupStatus, WriteOutcome};

impl Shell {
    /// `msg setup …`.
    pub(super) fn setup_command(&mut self, command: SetupCommand) -> Result<String, String> {
        match command {
            SetupCommand::Status => Ok(self.setup_status().to_json()),
            SetupCommand::Record { state, version } => match self.setup.record(state, version, &setup::now_rfc3339()) {
                Ok(WriteOutcome::Written) => {
                    tracing::info!(state = state.name(), version, "ilk açılış durumu yazıldı");
                    Ok("ok".into())
                }
                Ok(WriteOutcome::Unchanged) => Ok("ok".into()),
                Err(e) => {
                    tracing::warn!("setup.toml yazılmadı: {e}");
                    Err(e)
                }
            },
        }
    }

    pub(super) fn setup_status(&self) -> SetupStatus {
        let read = self.setup.read();
        if let setup::SetupRead::Corrupt(reason) = &read {
            // Uyarı açılışta (`offer_welcome`) bir kez; durum yoklamaları log'u doldurmasın.
            tracing::debug!(reason = %reason, "setup.toml bozuk; ilk açılış sayılmıyor");
        }
        let (source, launch) = self.hypr_brief;
        let session = setup::hypr_session(std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").as_deref());
        // Geri alınabilir son göç: `migration.shell` snapshot kimliği (`msg snapshot restore
        // <id>` ya da `config undo-migration`); snapshot alınamadıysa opak, kararlı etiket.
        let pending_notice = self.migration_undo.pending().map(|r| match &r.snapshot {
            Some(id) => id.as_str().to_owned(),
            None => format!("migration.shell:{}-{}", r.from, r.to),
        });
        let main = crate::services::hypr_config::takeover::main_state(&crate::services::hypr_config::sync::SyncEnv::from_process(self.apply_env.clone()));
        SetupStatus::new(&read, HyprBrief { session, source, launch, main }, MigrationBrief { pending_notice })
    }

    /// `settings open <sayfa>` (bildirim düğmesi): Ayarlar'ı o sayfada ayrık açar. Test örneği
    /// açmaz: Ayarlar tek örneklidir, kullanıcının açık Ayarlar'ına komut giderdi.
    pub(super) fn open_settings(&self, page: String) -> Result<String, String> {
        if self.setup.test_instance() {
            return Err("test örneği: Ayarlar açılmadı".into()); // i18n: tanı msg yanıtı (çevrilmez)
        }
        let path: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        let settings = setup::find_settings(crate::self_exe::exe_dir().as_deref(), &path)
            .ok_or_else(|| format!("{} bulunamadı", setup::SETTINGS_BIN))?; // i18n: tanı msg yanıtı (çevrilmez)
        let setsid = crate::services::launcher::catalog::find_executable("setsid", &path);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        setup::spawn_settings(settings, page, setsid, home).map_err(|e| e.to_string())?;
        Ok("ok".into())
    }

    /// `start`'ta bir kez: setup.toml yoksa (ve ayar açık, varsayılan örnek, Hyprland oturumu)
    /// `nuxut-settings --welcome` ayrık başlatılır. Shell dosya yazmaz; Ayarlar `setup done/skip`
    /// çağırır. Bulunamazsa/başlatılamazsa uyarı, panik yok.
    ///
    /// Dönüş: Hoş geldin ekranda mı (bu çağrıda başlatıldı ya da bu oturumda zaten başlatılmıştı
    /// ve kurulum hâlâ bitmedi)? `true` iken "güncelleme bitti" bildirimi bastırılır
    /// (2026-10-13: ikisi asla birlikte çıkmaz; `app/update_notice.rs`).
    pub(super) fn offer_welcome(&mut self) -> bool {
        let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        let session = setup::hypr_session(signature.as_deref());
        let read = self.setup.read();
        if let setup::SetupRead::Corrupt(reason) = &read {
            tracing::warn!(reason = %reason, "setup.toml bozuk; Hoş geldin yeniden açılmıyor (Ayarlar'dan açılabilir)");
        }
        if !setup::should_offer_welcome(&read, self.model.config.setup.welcome, &self.setup, session) {
            return false;
        }
        let signature = signature.and_then(|s| s.into_string().ok()).unwrap_or_default();
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        if !setup::claim_session_marker(runtime.as_deref(), &signature) {
            tracing::info!("Hoş geldin bu oturumda zaten başlatıldı");
            return true;
        }
        let path: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        let Some(settings) = setup::find_settings(crate::self_exe::exe_dir().as_deref(), &path) else {
            tracing::warn!("ilk açılış: {} bulunamadı (shell'in dizininde ve PATH'te); Hoş geldin açılmadı", setup::SETTINGS_BIN);
            return false;
        };
        let setsid = crate::services::launcher::catalog::find_executable("setsid", &path);
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match setup::spawn_welcome(settings, setsid, home) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("Hoş geldin başlatılamadı: {e}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ShellConfig;
    use crate::ipc::{self, Command};

    fn run(shell: &mut Shell, line: &str) -> Result<String, String> {
        match ipc::parse(line)? {
            Command::Setup(c) => shell.setup_command(c),
            other => panic!("setup komutu beklenirdi: {other:?}"),
        }
    }

    /// Yazım izni yoksa (kapalı ortam) `skipped(…)` ve dosya yok; izinliyse
    /// `ok`, durum JSON'u dosyayı yansıtır, küçük sürüm düşürmez.
    #[test]
    fn status_and_record_through_ipc() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let mut shell = Shell::new(ShellConfig::validated_default());
        shell.set_apply_env(nuxut_apply::Env { disabled: true, ..nuxut_apply::Env::for_test(dir.path()) });
        assert_eq!(run(&mut shell, "setup done 1"), Err("skipped(disabled)".into()));
        assert!(!state.exists(), "reddedilen yazım dosya bırakmaz");

        shell.set_apply_env(nuxut_apply::Env::for_test(dir.path()));
        let status = |shell: &mut Shell| serde_json::from_str::<serde_json::Value>(&run(shell, "setup status").unwrap()).unwrap();
        let s = status(&mut shell);
        assert_eq!(s["first_run"], true);
        assert!(s["setup_state"].is_null());
        assert_eq!(s["hypr"]["source"], "pending");
        assert!(s["hypr"]["launch"].is_null());
        assert!(s["migration"]["pending_notice"].is_null());

        assert_eq!(run(&mut shell, "setup skip 3"), Ok("ok".into()));
        assert_eq!(run(&mut shell, "setup done 2"), Ok("ok".into()), "düşürme no-op ama ok");
        let s = status(&mut shell);
        assert_eq!(s["first_run"], false);
        assert_eq!(s["setup_version"], 3);
        assert_eq!(s["setup_state"], "skipped");
        assert!(state.join("nuxut/setup.toml").is_file());
    }
}
