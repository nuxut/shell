//! `nuxut-shell msg …` komutlarının yürütülmesi ve config yeniden yükleme.

use std::time::Instant;

use nuxut::Context;

use super::Shell;
use super::clock::tick_format;
use crate::config::{CONFIG_NAME, ShellConfig};
use crate::config::LoadState;
use crate::config_heal::{self, HealReport};
use crate::config_migrate::{MigrationRecord, WritePolicy};
use crate::ipc::{self, Command, HealCommand, Toggle};
use crate::services::notifications::{InternalAction, InternalNotice};
use crate::widgets::Action;

impl Shell {
    /// Komut satırı modelini uygular (`nuxut-shell msg …`). Yanıt metni ya da hata.
    pub fn execute(&mut self, cx: &mut Context<Self>, command: Command) -> Result<String, String> {
        match command {
            Command::Ping => Ok("pong".into()),
            Command::Version => Ok(format!("nuxut-shell {}", env!("CARGO_PKG_VERSION"))),
            Command::Help => Ok(ipc::HELP.into()),
            Command::Quit => {
                // Yanıt `command` içinde eşzamanlı yazılır; döngü bu olay turundan sonra biter.
                tracing::info!("msg quit: kapanılıyor");
                self.night_light_shutdown(cx);
                self.magnifier_shutdown();
                self.osk_shutdown(cx);
                cx.quit();
                Ok("ok".into())
            }
            Command::ReloadConfig => {
                self.reload_config(cx);
                Ok("ok".into())
            }
            Command::UndoMigration => {
                if self.config_load.policy != WritePolicy::Disk {
                    return Err("bu örnek shell.toml'a yazmaz (ikincil/test örneği)".into());
                }
                let path = nuxut_config::config_path(CONFIG_NAME).ok_or("config dizini bulunamadı")?;
                self.migration_undo.undo(&path, &self.apply_env).map_err(|e| e.to_string())?;
                tracing::info!("shell.toml göçü geri alındı");
                self.reload_config(cx);
                Ok("ok".into())
            }
            Command::ConfigHeal(c) => self.config_heal(cx, c),
            Command::ConfigStatus => self.config_status(),
            Command::ConfigUndo => {
                if self.config_heal.has_undo() || !config_heal::load_undo(&self.apply_env).is_empty() {
                    return self.config_heal(cx, HealCommand::Undo);
                }
                if self.migration_undo.pending().is_some() {
                    return self.execute(cx, Command::UndoMigration);
                }
                Err("geri alınacak bir ayar değişikliği yok".into())
            }
            Command::Panel { panel, action, output } => {
                let idx = self.target_screen(output.as_deref())?;
                let surface = self.screens[idx].main;
                let is_open = self.screens[idx].is_panel_open(panel);
                let wanted = match action {
                    Toggle::Toggle => !is_open,
                    Toggle::On => true,
                    Toggle::Off => false,
                };
                if wanted != is_open {
                    let a = if wanted { Action::TogglePanel(panel) } else { Action::ClosePanel(panel) };
                    self.act(cx, surface, a);
                    if wanted {
                        // Bind'le açıldı: işaretçi bir pencerede; klavye kapanana kadar bizde kalsın.
                        let config = self.model.config.clone();
                        self.screens[idx].hold_keyboard();
                        Self::apply_input(cx, &mut self.screens[idx], &config);
                    }
                }
                Ok("ok".into())
            }
            Command::LauncherMode { mode, output } => self.launcher_mode(cx, mode, output.as_deref()),
            Command::SessionMenu { output } => {
                let panel = crate::widgets::PanelId::Edge(self.model.config.panels.session_edge());
                self.execute(cx, Command::Panel { panel, action: Toggle::On, output })
            }
            Command::Clipboard(c) => self.clipboard_command(c),
            Command::ClosePanels { output } => {
                // Sabitler dahil hepsi (açık kapatma).
                for idx in self.target_screens(output.as_deref())? {
                    let surface = self.screens[idx].main;
                    for open in self.screens[idx].open_panels() {
                        self.act(cx, surface, Action::ClosePanel(open));
                    }
                }
                Ok("ok".into())
            }
            Command::PinPanel { edge, action, output } => {
                let idx = self.target_screen(output.as_deref())?;
                let surface = self.screens[idx].main;
                let pinned = self.screens[idx].is_pinned(edge);
                let wanted = match action {
                    Toggle::Toggle => !pinned,
                    Toggle::On => true,
                    Toggle::Off => false,
                };
                if wanted != pinned {
                    self.act(cx, surface, Action::PinPanel(edge, wanted));
                }
                Ok("ok".into())
            }
            Command::Bar { action, output } => {
                let theme = cx.theme().clone();
                let config = self.model.config.clone();
                let now = Instant::now();
                for idx in self.target_screens(output.as_deref())? {
                    let s = &mut self.screens[idx];
                    let visible = match action {
                        Toggle::Toggle => !s.is_bar_visible(),
                        Toggle::On => true,
                        Toggle::Off => false,
                    };
                    if s.set_bar_visible(visible, &config, &theme, now) {
                        Self::sync_exclusions(cx, &mut self.screens[idx], &config);
                        Self::apply_input(cx, &mut self.screens[idx], &config);
                        cx.request_frame(self.screens[idx].main);
                    }
                }
                Ok("ok".into())
            }
            Command::BarFocus { output } => {
                let idx = self.target_screen(output.as_deref())?;
                self.bar_focus(cx, idx)
            }
            Command::Hypr(c) => self.hypr_command(cx, c),
            Command::Setup(c) => self.setup_command(c),
            // Eşzamansız yanıtlanır (`displays_command`; IPC giriş noktası ayırır).
            Command::Displays(_) => Err("displays yalnızca nuxut-shell msg ile".into()),
            // Eşzamansız yanıtlanır (`input_status_command`; IPC giriş noktası ayırır).
            Command::InputStatus => Err("input status yalnızca nuxut-shell msg ile".into()),
            Command::ClockRefresh => {
                self.refresh_timezone(cx);
                Ok("ok".into())
            }
            Command::Defaults(_) => Err("defaults yalnızca nuxut-shell msg ile".into()),
            // Bildirim düğmesi ya da `msg`; eylem sonucu bildirimle gelir (`app/removable_media.rs`).
            Command::Removable(c) => self.removable_command(c),
            // Durum eşzamanlı; aç/kapa eşzamansız (`charge_limit_command`; IPC giriş noktası ayırır).
            Command::Power(c) => self.power_status_command(c),
            // Model durumundan (servis yoksa varsayılan durum: ikisi de false).
            Command::BluetoothAgentStatus => Ok(crate::services::bluetooth::agent_status_json(&self.model.system.bluetooth)),
            // Salt okunur ve küçük IO (framework.toml, environment.d, tema dizinlerinde var/yok).
            Command::Idle(c) => self.idle_command(cx, c),
            Command::Lock => {
                self.session_action(cx, crate::services::session::SessionAction::Lock);
                Ok("ok".into())
            }
            Command::OpenSettings(page) => self.open_settings(page),
            Command::NightLight(c) => self.night_light_ipc(cx, c),
            // `msg` yanıtı büyüteç thread'inden (`magnifier_command`); burada bildirim düğmesi gibi yanıtsız yol.
            Command::Magnifier(ipc::MagnifierCommand::Run(c)) => self.magnifier_action(c).map(|()| "ok".into()),
            Command::Magnifier(ipc::MagnifierCommand::Status) => Err("magnifier status yalnızca nuxut-shell msg ile".into()),
            Command::Osk { action, output } => self.osk_ipc(cx, action, output.as_deref()),
            Command::WallpaperBuiltins => Ok(crate::wallpaper::builtins_json()),
            Command::ImagesStatus => Ok(crate::services::image_sandbox::status_json(
                crate::services::image_sandbox::active(&self.image_sandbox),
                &self.wallpaper_decoder.stats(),
            )),
            Command::Look(ipc::LookCommand::RestartApp(app)) => restart_app(app),
            Command::Look(c) => {
                let state = crate::services::look::LookState::read(cx.theme())?;
                Ok(match c {
                    ipc::LookCommand::Status => state.status_json(self.look.writer(), self.look.report.clone(), self.look.themes_rev()),
                    ipc::LookCommand::Plan => state.plan_json(self.look.writer()),
                    ipc::LookCommand::RestartApp(app) => return restart_app(app),
                })
            }
            c @ (Command::Volume { .. } | Command::VolumeMute { .. } | Command::MicMute { .. } | Command::Brightness { .. }) => {
                self.osd_command(cx, c)
            }
            // Eşzamansız yanıtlanır: `audio_diagnose` (IPC giriş noktası ayırır).
            Command::AudioDiagnose => Err("audio-diagnose yalnızca nuxut-shell msg ile".into()),
            Command::DoctorLive => Err("doctor live yalnızca nuxut-shell msg ile".into()),
            Command::Snapshot(_) => Err("snapshot yalnızca nuxut-shell msg ile".into()),
            // Bildirim düğmesi (`shot open|reveal`) ya da yanıtsız yakalama; `msg` yanıtı `command`'da.
            Command::Shot(c) => {
                self.shot_command(cx, c, None);
                Ok("ok".into())
            }
        }
    }

    /// `msg audio-diagnose`: rapor ses servisinden ayrı thread'de gelir ve doğrudan yanıtlanır.
    pub(super) fn audio_diagnose(&mut self, reply: nuxut::ipc::Reply) {
        let reply = std::sync::Arc::new(std::sync::Mutex::new(Some(reply)));
        let sent = {
            let reply = std::sync::Arc::clone(&reply);
            self.system.as_ref().is_some_and(|s| {
                s.audio_diagnose(Box::new(move |text| {
                    if let Some(r) = reply.lock().ok().and_then(|mut r| r.take()) {
                        r.send(&text);
                    }
                }))
            })
        };
        if !sent && let Some(r) = reply.lock().ok().and_then(|mut r| r.take()) {
            r.send("error: ses servisi çalışmıyor");
        }
    }

    /// `--output` verilmişse o monitör, yoksa odaklı monitör (o da yoksa ilk ekran).
    pub(super) fn target_screen(&self, output: Option<&str>) -> Result<usize, String> {
        if self.screens.is_empty() {
            return Err("ekran yok".into());
        }
        let name = match output {
            Some(n) => n.to_owned(),
            None => match self.model.hypr.focused_monitor() {
                Some(m) => m.name.clone(),
                None => return Ok(0),
            },
        };
        self.screens.iter().position(|s| s.name == name).ok_or_else(|| format!("monitör \"{name}\" yok"))
    }

    /// `--output` verilmişse o monitör, yoksa tüm ekranlar.
    fn target_screens(&self, output: Option<&str>) -> Result<Vec<usize>, String> {
        match output {
            Some(_) => Ok(vec![self.target_screen(output)?]),
            None => Ok((0..self.screens.len()).collect()),
        }
    }

    pub(super) fn reload_config(&mut self, cx: &mut Context<Self>) {
        self.reload_config_with(cx, self.config_load, Vec::new(), Vec::new());
    }

    /// Yeniden yükler. `state.heal_write`: onarım diske yazılır (`config heal`); `renames`: yazım
    /// hatası önerileri; `extra`: aynı bildirimde gösterilecek başka dosyaların raporları
    /// (framework.toml). shell.toml'un onarım raporu döner.
    pub(super) fn reload_config_with(
        &mut self,
        cx: &mut Context<Self>,
        state: LoadState,
        renames: Vec<(String, String)>,
        extra: Vec<HealReport>,
    ) -> Option<HealReport> {
        let loaded = match ShellConfig::load_healing(state, &self.apply_env, renames) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("shell.toml yüklenemedi, son geçerli config korunuyor: {e}");
                return None;
            }
        };
        self.config_load.unversioned_is_current = loaded.unversioned_is_current;
        for w in &loaded.warnings {
            tracing::warn!("shell.toml: {w}");
        }
        let mut noticed = loaded.migration.as_ref().is_some_and(|record| self.on_config_migrated(record));
        let report = loaded.heal.clone();
        let mut reports = extra;
        reports.extend(loaded.heal);
        noticed |= self.on_config_healed(&reports);
        if loaded.config == self.model.config {
            // Config aynı kalsa da göç bildirimi eklendiyse bildirim merkezi ve okunmamış sayacı
            // ekranlara yansısın (aşağıdaki tam yol bunu zaten yapıyor).
            if noticed {
                self.sync_notifications(cx);
                self.model_changed(cx);
            }
            return report;
        }
        tracing::info!("shell.toml yeniden yüklendi");
        self.model.config = loaded.config;
        self.sync_system_config();
        self.sync_idle_config(cx);
        self.sync_night_light_config(cx);
        self.magnifier_config_changed();
        self.osk_config_changed(cx);
        self.clock = tick_format(&self.model.config);
        let now = Instant::now();
        let theme = cx.theme().clone();
        for s in &mut self.screens {
            s.apply_config(&self.model.config, &theme, now);
            s.sync_model(&self.model, &theme, now);
            Self::sync_exclusions(cx, s, &self.model.config);
            Self::apply_input(cx, s, &self.model.config);
            cx.request_frame(s.main);
        }
        for i in 0..self.screens.len() {
            self.sync_wallpaper(cx, i);
        }
        self.launcher_config_changed();
        self.clipboard_config_changed(cx);
        self.removable_media_config_changed(cx);
        self.sync_sysmon_service(cx);
        self.sync_camera_service(cx);
        // Bildirim ayarları canlı (süreler, DND, sınırlar).
        self.model.notifications.set_config(self.model.config.notifications.clone());
        self.sync_notifications(cx);
        self.tick_clock(cx);
        self.sync_tray_icon_size();
        self.request_hypr_sync(cx);
        self.displays_config_changed();
        // Panele takvim kartı eklenmiş/kaldırılmış olabilir.
        self.refresh_calendar_range(false);
        report
    }
}

impl Shell {
    /// İlk yüklemenin yazım politikası ve sürümsüz dosya durumu; yeniden yüklemeler bunu izler.
    pub fn set_config_load_state(&mut self, state: LoadState) {
        self.config_load = state;
    }

    /// Açılışta (main) bir kez: dosya yazım ortamı (`nuxut_apply::Env::from_process`) ve ondan
    /// türeyen setup.toml erişimi.
    pub fn set_apply_env(&mut self, env: nuxut_apply::Env) {
        self.setup = crate::services::setup::SetupAccess::new(&env);
        self.apply_env = env;
    }

    /// Bir yükleme shell.toml'u göç ettirdi (ADR-0002 §3.5): geri al kaydı saklanır, davranış
    /// değiştiyse tek sessiz dahili bildirim (popup/ses yok, merkezde okunmamış; "Geri al"
    /// düğmesi `config undo-migration` komutunu yürütür). Bağlamsızdır (açılışta döngüden önce
    /// de çağrılır); bildirim eklendiyse `true` — döngü içindeki çağıran ekranları eşler.
    pub fn on_config_migrated(&mut self, record: &MigrationRecord) -> bool {
        self.migration_undo.remember(record);
        let Some(notice) = record.notice() else { return false };
        tracing::info!("{}: {}", notice.summary, notice.body);
        self.post_notice(InternalNotice::silent(
            notice.summary,
            notice.body,
            vec![InternalAction::new(notice.action_label, notice.action_command)],
        ));
        true
    }

    /// Onarım raporları → en çok iki sessiz bildirim (onarım; yazım hatası önerisi). Aynı sorunlar
    /// yeniden yüklemede tekrar bildirilmez. Bağlamsızdır (açılışta da çağrılır); bildirim
    /// eklendiyse `true`.
    pub fn on_config_healed(&mut self, reports: &[HealReport]) -> bool {
        let notices = self.config_heal.observe(reports);
        // Yazılmış onarımın geri al kaydı diske de: `msg config undo` yeniden başlatmadan sonra da çalışır.
        if reports.iter().any(|r| r.written.is_some())
            && let Err(e) = config_heal::save_undo(&self.apply_env, self.config_heal.undo_entries())
        {
            tracing::warn!("ayar onarımının geri al kaydı yazılamadı ({e}); yalnızca bu oturumda geri alınabilir");
        }
        let any = !notices.is_empty();
        for n in notices {
            tracing::info!("{}: {}", n.summary, n.body);
            let actions = n.actions.into_iter().map(|(label, command)| InternalAction::new(label, command)).collect();
            self.post_notice(InternalNotice::silent(n.summary, n.body, actions));
        }
        any
    }

    /// `msg config status`: JSON, salt okunur (her örnekte).
    fn config_status(&self) -> Result<String, String> {
        let input = config_heal::StatusInput {
            disk: self.config_load.policy == WritePolicy::Disk,
            unversioned_is_current: self.config_load.unversioned_is_current,
            migration_undo: self.migration_undo.pending().is_some(),
            undo: self.config_heal.undo_entries(),
        };
        serde_json::to_string_pretty(&config_heal::status(&self.apply_env, input)).map_err(|e| e.to_string())
    }

    /// `msg config heal …`. Yanıtlar tanı çıktısıdır (çevrilmez).
    fn config_heal(&mut self, cx: &mut Context<Self>, command: HealCommand) -> Result<String, String> {
        let disk = self.config_load.policy == WritePolicy::Disk;
        if !disk && command != HealCommand::DryRun {
            return Err("bu örnek ayar dosyalarına yazmaz (ikincil/test örneği)".into());
        }
        match command {
            HealCommand::DryRun => {
                let path = nuxut_config::config_path(CONFIG_NAME).ok_or("config dizini bulunamadı")?;
                let heal = crate::config::HealOptions {
                    keep: config_heal::keep_set(&self.apply_env, config_heal::SHELL_FILE),
                    ..Default::default()
                };
                let loaded = ShellConfig::load_full(&path, None, self.config_load.unversioned_is_current, &heal)
                    .map_err(|e| e.to_string())?;
                let mut out = vec![config_heal::describe(config_heal::SHELL_FILE, &loaded.heal.map(|h| h.actions).unwrap_or_default())];
                let fw = config_heal::heal_framework(&self.apply_env, false);
                out.push(config_heal::describe(config_heal::FRAMEWORK_FILE, &fw.map(|h| h.actions).unwrap_or_default()));
                Ok(out.join("\n"))
            }
            HealCommand::Apply | HealCommand::FixTypos => {
                let renames = match command {
                    HealCommand::FixTypos => self.config_heal.suggestions().iter().map(|(k, c)| (k.clone(), fix_target(k, c))).collect(),
                    _ => Vec::new(),
                };
                if command == HealCommand::FixTypos && renames.is_empty() {
                    return Ok("uygulanacak öneri yok".into());
                }
                let fw: Vec<HealReport> = config_heal::heal_framework(&self.apply_env, true).into_iter().collect();
                let state = LoadState { heal_write: true, ..self.config_load };
                let shell = self.reload_config_with(cx, state, renames, fw.clone());
                let mut lines = Vec::new();
                for r in shell.iter().chain(fw.iter()) {
                    lines.push(config_heal::summary_line(r));
                    lines.push(config_heal::describe(r.file, &r.actions));
                }
                Ok(if lines.is_empty() { "onarılacak bir şey yok".into() } else { lines.join("\n") })
            }
            HealCommand::Undo => {
                let mut entries = self.config_heal.take_undo();
                if entries.is_empty() {
                    // Kabuk onarımdan sonra yeniden başladıysa kayıt diskte. Yalnızca bilinen ayar
                    // dosyalarına geri yazılır (kayıt dosyası güvenilir girdi sayılmaz).
                    entries = config_heal::load_undo(&self.apply_env)
                        .into_iter()
                        .filter(|e| config_heal::file_path(e.file).as_deref() == Some(e.written.path.as_path()))
                        .collect();
                }
                if entries.is_empty() {
                    return Err("geri alınacak bir onarım yok".into());
                }
                let mut errors = Vec::new();
                let mut failed = Vec::new();
                for e in entries {
                    match config_heal::undo(&self.apply_env, &e.written) {
                        Ok(()) => {
                            if let Err(err) = config_heal::add_keep(&self.apply_env, e.file, &e.keep) {
                                tracing::warn!("{}: koru listesi yazılamadı ({err}); sonraki açılış yeniden onarabilir", e.file);
                            }
                            tracing::info!("{} onarımı geri alındı", e.file);
                        }
                        Err(err) => {
                            errors.push(err);
                            failed.push(e);
                        }
                    }
                }
                if let Err(e) = config_heal::save_undo(&self.apply_env, &failed) {
                    tracing::warn!("ayar onarımının geri al kaydı güncellenemedi ({e})");
                }
                if !failed.is_empty() {
                    self.config_heal.restore_undo(failed);
                }
                self.reload_config(cx);
                if errors.is_empty() { Ok("ok".into()) } else { Err(errors.join("; ")) }
            }
            HealCommand::IgnoreTypos => {
                let keys: Vec<String> = self.config_heal.suggestions().iter().map(|(k, _)| k.clone()).collect();
                if keys.is_empty() {
                    return Ok("yok sayılacak öneri yok".into());
                }
                config_heal::add_keep(&self.apply_env, config_heal::SHELL_FILE, &keys)?;
                self.reload_config(cx);
                Ok("ok".into())
            }
        }
    }
}

/// Yazım hatası önerisinin hedef yolu: `bar.siez` + `size` → `bar.size`.
fn fix_target(key: &str, candidate: &str) -> String {
    match key.rsplit_once('.') {
        Some((parent, _)) => format!("{parent}.{candidate}"),
        None => candidate.to_owned(),
    }
}

/// `msg look restart-app <kimlik>` (bildirimdeki "Şimdi yeniden başlat"): bloklayan D-Bus işi arka
/// planda, sonuç günlüğe (`services::look::app_refresh`).
fn restart_app(app: crate::services::look::app_refresh::App) -> Result<String, String> {
    std::thread::Builder::new()
        .name("nuxut-app-restart".into())
        .spawn(move || {
            if let Err(e) = crate::services::look::app_refresh::restart(app) {
                tracing::warn!(app = app.id(), "uygulama yeniden başlatılamadı: {e}");
            }
        })
        .map(|_| "ok".to_owned())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_migrate::{Edit, EditKind, KeyPath, Restore};

    fn record(written: bool) -> MigrationRecord {
        let key = KeyPath::new(&["bar", "widget", "workspaces", "count"]);
        MigrationRecord {
            from: 1,
            to: 2,
            written,
            snapshot: None,
            edits: vec![Edit {
                key: key.clone(),
                kind: EditKind::Transform,
                note: "Sayfa başına workspace".into(),
                restore: Some(Restore { key, value: "7".into() }),
            }],
        }
    }

    /// Göç kaydı → merkezde sessiz (popup'sız), okunmamış bildirim; düğme komutu `msg` ile
    /// aynı ayrıştırıcıdan `config undo-migration` olarak geçer.
    #[test]
    fn migration_posts_silent_unread_notice_with_undo_action() {
        crate::i18n::use_turkish_for_tests();
        let mut shell = Shell::new(ShellConfig::validated_default());
        assert!(!shell.on_config_migrated(&record(false)), "yazılmamış göç bildirilmez");
        assert_eq!(shell.model.notifications_unread, 0);
        assert!(shell.on_config_migrated(&record(true)));
        let center = &shell.model.notifications;
        assert_eq!(center.popups().count(), 0, "sessiz: popup yok");
        assert_eq!(shell.model.notifications_unread, 1);
        let n = center.history().next().expect("merkezde");
        assert_eq!(n.summary, "Ayarların yeni sürüme taşındı");
        assert_eq!(n.actions.len(), 1);
        assert_eq!(n.actions[0].label, "Geri al");
        let command = center.internal_command(n.id, &n.actions[0].key).expect("düğme komutu");
        assert_eq!(ipc::parse(command), Ok(Command::UndoMigration));
        assert!(shell.migration_undo.pending().is_some());
    }
}
