//! Masaüstü (Hyprland) ortaklığı: tema/shell.toml değişince `nuxut.lua`/`nuxut.conf` yeniden
//! üretilir ve uygulanır, açılışta dahil etme satırı denetlenir (ADR-0003,
//! `docs/design/hyprland-sync.md`). İş `nuxut-hypr-sync` thread'inde
//! (`services::hypr_config::sync::Syncer`); ana thread yalnızca isteği kurar ve son durumu tutar.

use std::sync::mpsc;
use std::thread::JoinHandle;

use nuxut::Context;
use nuxut::i18n::t;

use super::{Shell, ShellMessage};
use crate::ipc::HyprCommand;
use crate::services::hypr_config::control::HyprControl;
use crate::services::hypr_config::generate::{BlurLook, LookGroup, MotionGroup, WindowsGroup};
use crate::services::hypr_config::input::query_status;
use crate::services::hypr_config::sync::{SyncEnv, SyncRequest, SyncStatus, Syncer};
use crate::services::hypr_config::takeover;
use crate::services::notifications::{InternalAction, InternalNotice};

/// Eşitleme thread'ine giden iş.
#[derive(Debug)]
enum Job {
    Sync(Box<SyncRequest>),
    /// "Geri al" (bildirim düğmesi ya da `msg hypr undo`).
    Undo,
    /// `hypr.source` snapshot'ı `msg snapshot restore` ile geri yüklendi: Hyprland yeniden
    /// yüklenir, durum yenilenir.
    Restored,
    /// "Şimdi uygula" (bildirim düğmesi ya da `msg hypr apply-switch`): ayar dili geçişi.
    ApplySwitch,
    /// `msg hypr restore-backup`: en yeni ana ayar yedeğini geri koy (ADR-0007).
    RestoreBackup,
}

/// Arka plan eşitleyicisi; düşürülünce kanal kapanır ve thread beklenir.
#[derive(Debug)]
pub(super) struct HyprSync {
    tx: Option<mpsc::Sender<(u64, Job)>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for HyprSync {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Bulanıklık geçiş sayısı (Hyprland `decoration:blur:passes`); kullanıcıya açılmaz.
const BLUR_PASSES: u32 = 2;
/// Bildirimdeki "Geri al" düğmesinin komutu (`msg hypr undo`).
pub const UNDO_COMMAND: &str = "hypr undo";
/// Ayar dili geçiş bildirimindeki "Şimdi uygula" düğmesinin komutu (`msg hypr apply-switch`).
pub const APPLY_SWITCH_COMMAND: &str = "hypr apply-switch";
/// Devralma bildirimindeki "Geri al" düğmesinin komutu (en yeni ana ayar yedeği).
pub const RESTORE_COMMAND: &str = "hypr restore-backup";

impl Shell {
    /// `start`'ta, Hyprland servisinden sonra. Ortam kapısı geçmezse eşitleme kapalı kalır.
    pub(super) fn start_hypr_sync(&mut self, cx: &mut Context<Self>) {
        let env = SyncEnv::from_process(self.apply_env.clone());
        if let Err(reason) = env.guard() {
            tracing::info!("masaüstü ayar eşitlemesi kapalı: {reason}");
            let status = SyncStatus::inactive(reason);
            self.hypr_brief = (status.source, status.launch);
            self.hypr_status = status.to_json();
            return;
        }
        // Test örneği Hyprland'e asla komut göndermez (yalnızca geçici dizindeki dosyalar).
        let control = if env.may_control() { self.client.clone() } else { None };
        let (tx, rx) = mpsc::channel::<(u64, Job)>();
        let sender = cx.sender();
        let spawned = std::thread::Builder::new().name("nuxut-hypr-sync".into()).spawn(move || {
            let syncer = Syncer::new(env);
            // Son eşitleme isteği ("Şimdi uygula"dan sonra durumu yenilemek için) ve oturum
            // başına bir kez gösterilen geçiş bildirimi.
            let mut last: Option<Box<SyncRequest>> = None;
            let mut switch_notified = false;
            let mut queue = std::collections::VecDeque::new();
            while let Some((mut seq, job)) = queue.pop_front().or_else(|| rx.recv().ok()) {
                let ctl = control.as_ref().map(|c| c as &dyn HyprControl);
                let mut status = match job {
                    Job::Undo => {
                        let s = syncer.undo(ctl);
                        tracing::info!(errors = s.errors.len(), "masaüstü ayar bağlantısı geri alındı");
                        s
                    }
                    Job::RestoreBackup => {
                        let r = syncer.restore_backup(ctl);
                        tracing::info!(result = r.result, restored = r.restored.as_deref().unwrap_or(""), "ana Hyprland ayarı yedekten geri kondu");
                        for e in &r.errors {
                            tracing::warn!("yedekten geri koyma: {e}");
                        }
                        let Some(req) = last.clone() else { continue };
                        queue.push_back((seq, Job::Sync(Box::new(SyncRequest { force_add: false, ..*req }))));
                        continue;
                    }
                    Job::Restored => {
                        for e in syncer.reload_after_restore(ctl) {
                            tracing::warn!("masaüstü ayar eşitlemesi: {e}");
                        }
                        // Durum yenilensin (satır geri geldi ya da kalktı).
                        let Some(req) = last.clone() else { continue };
                        queue.push_back((seq, Job::Sync(Box::new(SyncRequest { force_add: false, ..*req }))));
                        continue;
                    }
                    Job::ApplySwitch => {
                        match syncer.apply_switch(ctl) {
                            Ok(errors) => {
                                tracing::info!(errors = errors.len(), "Hyprland yeni ayar diline geçirildi (reload full-reset)");
                                for e in errors {
                                    tracing::warn!("masaüstü ayar eşitlemesi: {e}");
                                }
                            }
                            Err(e) => tracing::warn!("ayar dili geçişi uygulanmadı: {e}"),
                        }
                        // Durum yenilensin (çalışan dil değişti mi?); ekleme zorlaması tekrarlanmaz.
                        let Some(req) = last.clone() else { continue };
                        queue.push_back((seq, Job::Sync(Box::new(SyncRequest { force_add: false, ..*req }))));
                        continue;
                    }
                    Job::Sync(mut req) => {
                        // Biriken eşitleme istekleri: yalnızca sonuncusu (ekleme isteği korunur);
                        // araya giren geri alma/geçiş sırayla, eşitlemeden sonra işlenir.
                        while let Ok(next) = rx.try_recv() {
                            match next {
                                (next_seq, Job::Sync(n)) => {
                                    seq = next_seq;
                                    let force = req.force_add || n.force_add;
                                    req = n;
                                    req.force_add = force;
                                }
                                other => queue.push_back(other),
                            }
                        }
                        let s = syncer.run(&req, ctl);
                        last = Some(req);
                        report(&s, &sender);
                        if s.pending_switch && !switch_notified {
                            switch_notified = true;
                            notify_switch(&s, &sender);
                        }
                        s
                    }
                };
                for e in &status.errors {
                    tracing::warn!("masaüstü ayar eşitlemesi: {e}");
                }
                // Birleştirmede öne geçen (daha büyük numaralı) eşitleme, sırada bekleyen daha
                // küçük numaralı işi (geri al/geçiş) "bitti" saydırmasın.
                status.seq = queue.iter().map(|(s, _)| s.saturating_sub(1)).fold(seq, u64::min);
                if sender.send(ShellMessage::HyprSync(status)).is_err() {
                    break;
                }
            }
        });
        match spawned {
            Ok(thread) => {
                self.hypr_sync = Some(HyprSync { tx: Some(tx), thread: Some(thread) });
                self.request_hypr_sync(cx);
            }
            Err(e) => tracing::warn!("eşitleme thread'i açılamadı: {e}"),
        }
    }

    /// Güncel tema ve shell.toml'dan istek kurar; aynı istek tekrar gönderilmez (zorlanmadıkça).
    pub(super) fn request_hypr_sync(&mut self, cx: &mut Context<Self>) {
        self.send_hypr_sync(cx, false, false);
    }

    /// Aynı istek olsa da yeniden eşitle (Ekranlar dosyası oluştu/değişti: Nuxut modülü onu
    /// yüklesin — ADR-0005 §6.2).
    pub(super) fn force_hypr_sync(&mut self, cx: &mut Context<Self>) {
        self.send_hypr_sync(cx, true, false);
    }

    /// `force`: aynı istek olsa da gönder; `add_source`: dahil etme satırını bir kez zorla ekle.
    fn send_hypr_sync(&mut self, cx: &mut Context<Self>, force: bool, add_source: bool) {
        if self.hypr_sync.is_none() {
            return;
        }
        // İlk görünüm turu (imleç ortamı) bitmeden üretilmez: dosya bir kez yazılsın.
        if !self.look.settled && !force && !add_source {
            return;
        }
        let theme = cx.theme();
        let c = &self.model.config;
        let w = c.windows;
        let req = SyncRequest {
            compositor: c.compositor,
            look: Some(LookGroup::from_theme(theme, w.blur.then_some(BlurLook { size: w.blur_size, passes: BLUR_PASSES }))),
            motion: Some(MotionGroup::from_theme(theme)),
            windows: Some(WindowsGroup::from_theme(theme, w.gap_inner, w.gap_outer, w.border)),
            keybinds: c.compositor.sync_keybinds.then(|| c.keybinds.map.clone()),
            hide_defaults: c.wallpaper.draws_any(),
            shell_exe: std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
            session_env: crate::services::portal_env::from_process(),
            cursor_env: self.look.cursor_env.clone(),
            input: c.compositor.sync_input.then(|| c.input.settings.clone()),
            force_add: false,
        };
        if !force && self.hypr_last.as_ref() == Some(&req) {
            return;
        }
        self.hypr_last = Some(req.clone());
        self.send_hypr_job(Job::Sync(Box::new(SyncRequest { force_add: add_source, ..req })));
    }

    pub(super) fn on_hypr_sync(&mut self, status: SyncStatus) {
        self.hypr_brief = (status.source, status.launch);
        self.hypr_status = status.to_json();
        self.hypr_overrides = status.overrides.iter().map(|o| (o.key.clone(), o.file.clone(), o.line)).collect();
        self.hypr_seq_done = self.hypr_seq_done.max(status.seq);
        self.hypr_applied = status.applied;
    }

    /// Son gönderilen iş tamamlandı ve son eşitleme Hyprland'e canlı uygulandı: etkin değerler
    /// Nuxut'unkileri yansıtıyor olmalı (getoption tabanlı override tespiti yalnızca o zaman).
    fn hypr_settled(&self) -> bool {
        self.hypr_seq_done >= self.hypr_seq_sent && self.hypr_applied
    }

    /// `msg input status`: Hyprland `getoption` istekleri ayrı thread'de (UI beklemez; istemci
    /// zaman aşımı 1 sn, tek batch isteği). Hyprland yoksa `available: false`. Salt okunur.
    pub(super) fn input_status_command(&mut self, reply: nuxut::ipc::Reply) {
        let c = &self.model.config;
        let settings = c.input.settings.clone();
        let sync = c.compositor.sync_input;
        // sync_input kapalıyken hiçbir girdi anahtarı üretilmez: override da yok.
        let overrides = if sync { self.hypr_overrides.clone() } else { Vec::new() };
        // Değer karşılaştırmalı (Lua'da tek) tespit: yalnızca uygulama bitmiş, canlı yapılmış ve
        // uygulanan istek bugünkü `[input]`'u taşıyorsa (shell.toml değişip eşitleme henüz
        // gönderilmediyse eski değerle karşılaştırılmasın).
        let settled = sync
            && self.hypr_settled()
            && self.hypr_last.as_ref().is_some_and(|r| r.input.as_ref() == Some(&settings));
        let client = self.client.clone();
        let spawned = std::thread::Builder::new().name("nuxut-input-status".into()).spawn(move || {
            let ctl = client.as_ref().map(|c| c as &dyn HyprControl);
            reply.send(&query_status(ctl, &settings, sync, &overrides, settled).to_json());
        });
        if let Err(e) = spawned {
            tracing::warn!("girdi durumu thread'i açılamadı: {e}");
        }
    }

    /// Bildirimdeki "Geri al" düğmesi.
    pub(super) fn hypr_undo(&mut self) {
        self.send_hypr_job(Job::Undo);
    }

    /// Bildirimdeki "Şimdi uygula" düğmesi (ayar dili geçişi).
    pub(super) fn hypr_apply_switch(&mut self) {
        self.send_hypr_job(Job::ApplySwitch);
    }

    /// `hypr.source` snapshot'ı geri yüklendi (`msg snapshot restore`).
    pub(super) fn hypr_reload_after_restore(&mut self) {
        self.send_hypr_job(Job::Restored);
    }

    /// İşi sıra numarasıyla gönderir; thread durumu işlediği son işin numarasıyla döndürür
    /// (birleştirilen eşitlemelerde en büyüğü) → "uygulama sürüyor mu?" ([`Self::hypr_settled`]).
    fn send_hypr_job(&mut self, job: Job) {
        if let Some(tx) = self.hypr_sync.as_ref().and_then(|s| s.tx.as_ref()) {
            self.hypr_seq_sent += 1;
            if tx.send((self.hypr_seq_sent, job)).is_err() {
                self.hypr_seq_sent -= 1;
            }
        }
    }

    pub(super) fn hypr_command(&mut self, cx: &mut Context<Self>, command: HyprCommand) -> Result<String, String> {
        if command == HyprCommand::Status {
            return Ok(self.hypr_status.clone());
        }
        // Ana ayar devralması (CLI) bitti: özet bildirimi rapordan; eşitleme kapalıyken de.
        if command == HyprCommand::TakeoverDone {
            let env = SyncEnv::from_process(self.apply_env.clone());
            let Some(r) = takeover::load_report(&env).filter(|r| r.result == "taken_over") else {
                return Err("devralma raporu yok".into());
            };
            let (title, body) = takeover_notice(&r);
            tracing::info!(backup = r.backup.as_deref().unwrap_or(""), "Hyprland ana ayarı devralındı (bildirim)");
            post(&title, body, (&t!("shell-hypr-sync-undo"), RESTORE_COMMAND), &cx.sender());
            return Ok("ok".into());
        }
        // Pano köprüsü betiği devralması (services::clipboard::takeover) eşitleme kapalıyken de geri alınır.
        if command == HyprCommand::Undo {
            self.clipboard_bridge_takeover_undo();
        }
        if self.hypr_sync.is_none() {
            return Err(format!("eşitleme kapalı: {}", self.hypr_status)); // i18n: tanı msg yanıtı (çevrilmez)
        }
        match command {
            HyprCommand::Undo => self.hypr_undo(),
            HyprCommand::ApplySwitch => self.hypr_apply_switch(),
            HyprCommand::RestoreBackup => self.send_hypr_job(Job::RestoreBackup),
            _ => self.send_hypr_sync(cx, true, command == HyprCommand::AddSource),
        }
        Ok("ok (sonuç: nuxut-shell msg hypr status)".into()) // i18n: tanı msg yanıtı (çevrilmez)
    }
}

/// Log + ilk bağlantı bildirimi (eşitleme thread'inde).
fn report(status: &SyncStatus, sender: &nuxut::wayland::MessageSender<ShellMessage>) {
    if status.wrote {
        tracing::info!(file = status.generated.as_deref().unwrap_or(""), "masaüstü ayar dosyası güncellendi");
    }
    if !status.conflicts.is_empty() {
        tracing::info!(count = status.conflicts.len(), "kısayol çakışması: Nuxut kısayolu üretilmedi (msg hypr status)");
    }
    if !status.taken_over.is_empty() {
        tracing::info!(count = status.taken_over.len(), "kısayol devralındı: kullanıcının bind'i nuxut.conf'ta unbind edildi (msg hypr status; kapatmak: [compositor] take_over_keys = false)");
    }
    if status.added {
        let config = status.config.as_deref().unwrap_or("hyprland.lua");
        let snapshot = status.snapshot.as_deref().unwrap_or("-");
        tracing::info!(config, snapshot, "Hyprland ayarına Nuxut satırı eklendi");
        let body = t!("shell-hypr-sync-connected-body", config = config);
        post(&t!("shell-hypr-sync-connected-title"), body, (&t!("shell-hypr-sync-undo"), UNDO_COMMAND), sender);
    }
}

/// Ayar dili geçişi bildirimi (oturum başına bir kez; ADR-0003 Karar 4): yeni dil sonraki
/// oturumda geçerli, isteğe bağlı "Şimdi uygula".
fn notify_switch(status: &SyncStatus, sender: &nuxut::wayland::MessageSender<ShellMessage>) {
    let config = status.config.as_deref().unwrap_or("hyprland.lua");
    tracing::info!(config, "Hyprland ayar dili değişti; yeni ayar sonraki oturumda geçerli (msg hypr apply-switch)");
    let body = t!("shell-hypr-sync-switch-body", config = config);
    post(&t!("shell-hypr-sync-switch-title"), body, (&t!("shell-hypr-sync-apply-switch"), APPLY_SWITCH_COMMAND), sender);
}

/// Devralma bildirimi metni (başlık, gövde); saf.
fn takeover_notice(r: &takeover::ReportSummary) -> (String, String) {
    let mut body = t!(
        "shell-hypr-main-config-body",
        env = r.migrated.env,
        settings = r.migrated.settings,
        exec = r.not_migrated.exec.len(),
        binds = r.not_migrated.binds,
        rules = r.not_migrated.rules,
        backup = r.backup.as_deref().unwrap_or("-")
    );
    if r.clipboard_bridge.as_deref() == Some("external_not_migrated") {
        body.push(' ');
        body.push_str(&t!("shell-hypr-main-config-clipboard"));
    }
    (t!("shell-hypr-main-config-title"), body)
}

/// Shell'in kendi bildirimi (popup'lı; ana döngüde merkeze eklenir, D-Bus'a gitmez). Düğme
/// `msg` komutuna yönlenir.
fn post(summary: &str, body: String, (label, command): (&str, &str), sender: &nuxut::wayland::MessageSender<ShellMessage>) {
    let notice = InternalNotice::popup(summary, body, vec![InternalAction::new(label, command)]);
    let _ = sender.send(ShellMessage::Notice(notice));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{self, Command};

    #[test]
    fn notice_actions_are_valid_commands() {
        assert_eq!(ipc::parse(UNDO_COMMAND), Ok(Command::Hypr(HyprCommand::Undo)));
        assert_eq!(ipc::parse(APPLY_SWITCH_COMMAND), Ok(Command::Hypr(HyprCommand::ApplySwitch)));
        assert_eq!(ipc::parse(RESTORE_COMMAND), Ok(Command::Hypr(HyprCommand::RestoreBackup)));
    }

    /// Devralma bildirimi: rapor (bilinmeyen alan yok sayılır) okunur; harici pano köprüsü ayrıca
    /// söylenir. Sayıların katalogdaki yeri i18n katalog testinde (değişkenler).
    #[test]
    fn takeover_notice_summarizes_report() {
        let json = r#"{"result":"taken_over","backup":"~/.config/hypr/hyprland.conf.nuxut-yedek-2026-10-17-0900","migrated":{"env":4,"settings":6},"not_migrated":{"exec":["a","b"],"binds":120,"rules":30,"other":9},"clipboard_bridge":"external_not_migrated","yeni_alan":1}"#;
        let r: takeover::ReportSummary = serde_json::from_str(json).unwrap();
        assert_eq!((r.migrated.env, r.migrated.settings, r.not_migrated.exec.len(), r.not_migrated.binds), (4, 6, 2, 120));
        let (title, body) = takeover_notice(&r);
        assert!(!title.is_empty());
        assert!(body.contains(&t!("shell-hypr-main-config-clipboard")));
        let plain = takeover::ReportSummary { clipboard_bridge: None, ..r };
        assert!(!takeover_notice(&plain).1.contains(&t!("shell-hypr-main-config-clipboard")));
    }
}
