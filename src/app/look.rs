//! Birleşik görünümün çalışma anı (ADR-0004 §4-4, §7): tema ya da `framework.toml` (`[desktop]`,
//! `[colors]`, palet) değişince `nuxut-look` thread'i planı kurar ve `nuxut-apply` ile uygular
//! (`services::look::sync`); ana thread yalnızca isteği gönderir ve son raporu tutar.
//!
//! - Ana thread aynı görünüm girdisini (kip, renkler, tipografi, masaüstü seçimleri) tekrar
//!   göndermez; thread art arda gelenleri [`DEBOUNCE`] boyunca birleştirir (yalnızca sonuncusu).
//!   Tur sürerken yeni istek gelirse tur ayarlar arasında yarıda bırakılır, canlı eylemler (KDE
//!   sinyalleri) gönderilmez; en yeni istekle yeniden başlanır ve sinyaller yalnızca tamamlanan
//!   son turda, yarıda kalanların yazdıkları dahil tek sefer gider ("son seçim kazanır").
//!   Değişiklik yoksa `nuxut-apply` `NoOp` döner: snapshot, sinyal ve `setcursor` yok. Boşta iş yok.
//! - Plan önceki uygulanan planla aynıysa tur hiç çalışmaz; sürekli yeniden yazılan ayar durdurulur,
//!   uyarılar durum değişince bir kez (`services::look::guard`, docs/design/look-external.md).
//! - Kapı: ikincil örnekte (`Env::disabled`) thread hiç başlamaz; test örneğinde dosyalar yalnızca
//!   test köklerine yazılır, dconf ve canlı eylemler atlanır (`services::look::writer`).
//! - İmleç ortamı `hypr_config` üreticisine girdi olur (`SyncRequest::cursor_env`); ilk görünüm turu
//!   bitene kadar Hyprland eşitlemesi bekletilir (dosya bir kez yazılsın).

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nuxut::Context;
use nuxut::i18n::t;

use super::{Shell, ShellMessage};
use crate::services::look::backend::ApplyReport;
use crate::services::look::writer::{self, SystemLive, WRITER_NAME};
use crate::services::look::{self, LookState, SyncResult};
use crate::services::notifications::{InternalAction, InternalNotice};

/// Art arda değişimlerin birleştirildiği süre (palet + `framework.toml` yazımı aynı seçimden).
/// Kısa: tur sürerken gelen yeni istek turu zaten yarıda keser ("son seçim kazanır").
pub const DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Debug)]
enum Job {
    Sync(Box<nuxut::Theme>),
    /// GTK dosyaları dışarıdan değişti: aynı plan da yeniden uygulanır (`guard` unutulur); yazım
    /// olursa (başka bir araç Nuxut dosyasını değiştirmişti) sonuç `repaired`.
    Verify(Box<nuxut::Theme>),
    /// `msg snapshot restore` bir `look.*` ayarını geri yükledi (geri yazılan değerlerle).
    Restored(String, Box<nuxut_apply::RestoredValues>),
}

/// Görünüm thread'i; düşürülünce kanal kapanır ve thread beklenir.
#[derive(Debug)]
pub(super) struct LookWorker {
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for LookWorker {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Ana thread'in görünüm durumu.
#[derive(Debug, Default)]
pub(super) struct LookRuntime {
    worker: Option<LookWorker>,
    /// Son gönderilen girdi (aynısı tekrar gönderilmez).
    last: Option<Box<nuxut::Theme>>,
    /// İlk tur bitti mi (Hyprland eşitlemesi bunu bekler).
    pub(super) settled: bool,
    pub(super) report: Option<ApplyReport>,
    pub(super) cursor_env: Vec<(String, String)>,
    /// Kurulu tema dizinleri izleyicisi (`msg look status` `themes_rev`); açılamazsa `None` (0).
    themes: Option<look::watch::ThemeWatcher>,
    /// GTK yapılandırma dosyaları izleyicisi (kendini onarma; `look::gtk_watch`).
    gtk: Option<look::gtk_watch::GtkWatcher>,
    /// Bu oturumda onarma bildirimi gösterildi mi (bir kez).
    repair_notified: bool,
}

impl LookRuntime {
    /// `msg look status` `writer`.
    pub(super) fn writer(&self) -> &'static str {
        if self.worker.is_some() { WRITER_NAME } else { "disabled" }
    }

    /// `msg look status` `themes_rev`.
    pub(super) fn themes_rev(&self) -> u64 {
        self.themes.as_ref().map_or(0, look::watch::ThemeWatcher::rev)
    }
}

/// Görünüm girdisi aynı mı (yalnızca planı etkileyen alanlar)?
fn same_input(a: &nuxut::Theme, b: &nuxut::Theme) -> bool {
    a.mode == b.mode
        && a.colors == b.colors
        && a.contrast == b.contrast
        && a.reduced_motion() == b.reduced_motion()
        && a.typography == b.typography
        && a.desktop == b.desktop
}

impl Shell {
    /// `start`'ta, Hyprland eşitlemesinden önce. İkincil örnekte (yazım kapalı) başlamaz.
    pub(super) fn start_look(&mut self, cx: &mut Context<Self>) {
        // Tema izleyicisi yazım kapısından bağımsız: `themes_rev` her örnekte verilir.
        let sender = cx.sender();
        match look::watch::ThemeWatcher::spawn(look::watch::WatchOptions::from_env(), move |rev| {
            let _ = sender.send(ShellMessage::ThemesChanged(rev));
        }) {
            Ok(w) => self.look.themes = Some(w),
            Err(e) => tracing::warn!("tema izleyicisi açılamadı: {e}"),
        }
        let env = self.apply_env.clone();
        if env.disabled {
            self.look.settled = true;
            return;
        }
        // Test örneği Hyprland'e komut göndermez.
        let hypr = if env.may_control_hyprland() { self.client.clone() } else { None };
        let (tx, rx) = mpsc::channel::<Job>();
        let sender = cx.sender();
        let spawned = std::thread::Builder::new().name("nuxut-look".into()).spawn(move || {
            let mut sink = SystemLive::new(hypr, &env);
            // Geri yüklenen imlecin okunduğu parça (LookState ile aynı XDG kuralı).
            let settings_ini = look::plan::Dirs::from_env().map(|d| d.config.join("gtk-3.0/settings.ini"));
            let mut first = true;
            let mut guard = look::guard::ApplyGuard::default();
            // Yarıda bırakılan turun ardından işlenecek en yeni istek.
            let mut next: Option<Box<nuxut::Theme>> = None;
            // Yarıda kalan turların yazdığı, canlı eylemi henüz gönderilmemiş ayarlar.
            let mut carried: Vec<&'static str> = Vec::new();
            // Bu tur bir doğrulama isteğinden mi (GTK dosyaları dışarıdan değişti).
            let mut verify = false;
            loop {
                let job = match next.take() {
                    Some(theme) => Job::Sync(theme),
                    None => match rx.recv() {
                        Ok(job) => job,
                        Err(_) => break,
                    },
                };
                let theme = match job {
                    Job::Restored(setting, values) => {
                        let live = writer::after_restore(&env, &mut sink, &setting, &values, settings_ini.as_deref());
                        tracing::info!(setting, live = live.len(), "görünüm: geri yükleme sonrası canlı eylemler");
                        continue;
                    }
                    Job::Sync(theme) => theme,
                    Job::Verify(theme) => {
                        verify = true;
                        theme
                    }
                };
                // Birleştirme (ilk tur hemen: Hyprland eşitlemesi bekliyor).
                let mut theme = theme;
                let mut restored = Vec::new();
                if !std::mem::replace(&mut first, false) {
                    loop {
                        match rx.recv_timeout(DEBOUNCE) {
                            Ok(Job::Sync(t)) => theme = t,
                            Ok(Job::Verify(t)) => {
                                verify = true;
                                theme = t;
                            }
                            Ok(Job::Restored(s, v)) => restored.push((s, v)),
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                }
                let started = Instant::now();
                let round_verify = std::mem::take(&mut verify);
                if round_verify {
                    guard.forget();
                }
                let result = match LookState::read(&theme) {
                    // Aynı plan (tetik kurulu tema izleyicisinden ya da ilgisiz bir yeniden
                    // yüklemeden geldi): yazım, sinyal, log yok (`look::guard`).
                    Ok(state) if !guard.should_apply(state.framework.desktop.unified, &state.plan) => None,
                    Ok(mut state) => {
                        let now = Instant::now();
                        let unified = state.framework.desktop.unified;
                        let full = state.plan.clone();
                        let (filtered, paused) = guard.filter(&full, now);
                        state.plan = filtered;
                        // Son seçim kazanır: yeni istek gelmişse kalan ayarlar ve canlı eylemler
                        // atlanır, en yenisiyle hemen yeniden başlanır.
                        let mut cancel = || newer_request(&rx, &mut next, &mut restored, &mut verify);
                        let mut result = look::sync_with(&state, &env, &mut sink, &mut cancel, &carried);
                        if let Some(r) = result.report.as_ref().filter(|r| r.cancelled) {
                            for s in &r.written {
                                if !carried.contains(s) {
                                    carried.push(s);
                                }
                            }
                            guard.forget();
                            tracing::debug!(written = ?r.written, ms = started.elapsed().as_millis() as u64, "birleşik görünüm: tur daha yeni istek için yarıda bırakıldı");
                            for (s, v) in restored {
                                writer::after_restore(&env, &mut sink, &s, &v, settings_ini.as_deref());
                            }
                            continue;
                        }
                        carried.clear();
                        if let Some(r) = &mut result.report {
                            r.paused = paused;
                            r.ok &= r.paused.is_empty();
                        }
                        for s in guard.record(unified, &full, result.report.as_ref(), now) {
                            tracing::warn!(
                                setting = s,
                                window_s = look::guard::LOOP_WINDOW.as_secs(),
                                pause_s = look::guard::LOOP_PAUSE.as_secs(),
                                "birleşik görünüm: ayar sürekli yeniden yazılıyor (başka bir araç geri alıyor ya da girdi salınıyor); bir süre durduruldu"
                            );
                        }
                        if let Some(r) = &result.report {
                            log_report(r, guard.problems_changed(r), started.elapsed());
                        }
                        Some(result)
                    }
                    Err(e) => {
                        tracing::warn!("birleşik görünüm okunamadı: {e}");
                        Some(SyncResult::default())
                    }
                };
                for (s, v) in restored {
                    writer::after_restore(&env, &mut sink, &s, &v, settings_ini.as_deref());
                }
                let result = result.map(|mut r| {
                    r.repaired = round_verify && r.report.as_ref().is_some_and(|x| !x.cancelled && !x.written.is_empty());
                    r
                });
                if let Some(result) = result
                    && sender.send(ShellMessage::Look(result)).is_err()
                {
                    break;
                }
            }
        });
        match spawned {
            Ok(thread) => {
                self.look.worker = Some(LookWorker { tx: Some(tx), thread: Some(thread) });
                self.request_look_sync(cx);
                // Kendini onarma: GTK dosyaları dışarıdan değişince doğrulama turu (`look::gtk_watch`).
                if let Some(config) = look::plan::Dirs::from_env().map(|d| d.config) {
                    let sender = cx.sender();
                    match look::gtk_watch::GtkWatcher::spawn(config, look::gtk_watch::SETTLE, move || {
                        let _ = sender.send(ShellMessage::GtkFilesChanged);
                    }) {
                        Ok(w) => self.look.gtk = Some(w),
                        Err(e) => tracing::warn!("GTK dosya izleyicisi açılamadı: {e}"),
                    }
                }
            }
            Err(e) => {
                tracing::warn!("görünüm thread'i açılamadı: {e}");
                self.look.settled = true;
            }
        }
    }

    /// Tema değişti (ya da açılış): girdi değiştiyse tur iste.
    pub(super) fn request_look_sync(&mut self, cx: &mut Context<Self>) {
        let Some(tx) = self.look.worker.as_ref().and_then(|w| w.tx.as_ref()) else { return };
        let theme = cx.theme();
        if self.look.last.as_deref().is_some_and(|l| same_input(l, theme)) {
            return;
        }
        self.look.last = Some(Box::new(theme.clone()));
        let _ = tx.send(Job::Sync(Box::new(theme.clone())));
    }

    /// GTK dosyaları dışarıdan değişti: aynı girdiyle de doğrulama turu (`Job::Verify`).
    pub(super) fn gtk_files_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(tx) = self.look.worker.as_ref().and_then(|w| w.tx.as_ref()) {
            let _ = tx.send(Job::Verify(Box::new(cx.theme().clone())));
        }
    }

    /// Kurulu temalar değişti: aynı girdiyle de tur iste (yeni kurulan istenen tema yedeğin
    /// yerini alsın; değişiklik yoksa `nuxut-apply` `NoOp`).
    pub(super) fn themes_changed(&mut self, cx: &mut Context<Self>) {
        if self.look.worker.is_some() {
            self.look.last = None;
            self.request_look_sync(cx);
        }
    }

    /// `look.*` snapshot'ı geri yüklendi (`msg snapshot restore`).
    pub(super) fn look_restored(&mut self, setting: &str, values: Box<nuxut_apply::RestoredValues>) {
        if let Some(tx) = self.look.worker.as_ref().and_then(|w| w.tx.as_ref()) {
            let _ = tx.send(Job::Restored(setting.to_owned(), values));
        }
    }

    pub(super) fn on_look(&mut self, cx: &mut Context<Self>, result: SyncResult) {
        let colors = result.report.as_ref().is_some_and(|r| !r.cancelled && r.written.contains(&look::plan::setting::MODE));
        if colors {
            self.refresh_static_apps(cx);
        }
        // Başka bir araç Nuxut'un GTK dosyalarını değiştirmişti, Nuxut yeniden uyguladı: oturumda
        // bir kez söylenir (kullanıcı isteği 2026-10-17: "neler oluyor" denmesin).
        if result.repaired && !std::mem::replace(&mut self.look.repair_notified, true) {
            let notice = InternalNotice::popup(
                t!("shell-look-repaired-title"),
                t!("shell-look-repaired-body"),
                vec![InternalAction::new(t!("shell-look-repaired-use-other"), "look use-other-tool")],
            );
            self.notice(cx, notice);
        }
        if result.report.is_some() {
            self.look.report = result.report;
        }
        let first = !std::mem::replace(&mut self.look.settled, true);
        let env_changed = self.look.cursor_env != result.cursor_env;
        self.look.cursor_env = result.cursor_env;
        if first || env_changed {
            self.request_hypr_sync(cx);
        }
    }
}

impl Shell {
    /// GTK4 renkleri yazıldı: canlı yenilenmeyen uygulamalar (`look::app_refresh`) — yalnız arka
    /// plan süreci sessizce kapatılır, açık penceresi olan için "Şimdi yeniden başlat" bildirimi.
    /// Test/ikincil örnekte (canlı eylem kapısı kapalı) hiçbir şey yapılmaz.
    fn refresh_static_apps(&mut self, cx: &mut Context<Self>) {
        if self.apply_env.guard(nuxut_apply::TargetKind::Dconf).is_err() {
            return;
        }
        let client = self.client.clone();
        let sender = cx.sender();
        let spawned = std::thread::Builder::new().name("nuxut-app-refresh".into()).spawn(move || {
            let clients = client.and_then(|c| c.request("j/clients").ok());
            for app in look::app_refresh::after_colors_changed(clients.as_deref()) {
                let name = app_name(app);
                let notice = InternalNotice::popup(
                    t!("shell-look-app-restart-title", app = name.as_str()),
                    t!("shell-look-app-restart-body", app = name.as_str()),
                    vec![InternalAction::new(t!("shell-look-app-restart-now"), format!("look restart-app {}", app.id()))],
                );
                if sender.send(ShellMessage::Notice(notice)).is_err() {
                    break;
                }
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("uygulama yenileme thread'i açılamadı: {e}");
        }
    }
}

/// Bildirimde görünen uygulama adı.
fn app_name(app: look::app_refresh::App) -> String {
    match app {
        look::app_refresh::App::Nautilus => t!("shell-look-app-nautilus"),
    }
}

/// Tur sürerken kuyruğa bakar (bloklamaz): yeni `Sync` geldiyse en yenisi `next`'e konur ve tur
/// yarıda bırakılmalıdır (`true`). Geri yükleme bildirimleri turu kesmez, sonra işlenir.
fn newer_request(rx: &mpsc::Receiver<Job>, next: &mut Option<Box<nuxut::Theme>>, restored: &mut Vec<(String, Box<nuxut_apply::RestoredValues>)>, verify: &mut bool) -> bool {
    while let Ok(job) = rx.try_recv() {
        match job {
            Job::Sync(t) => *next = Some(t),
            Job::Verify(t) => {
                *verify = true;
                *next = Some(t);
            }
            Job::Restored(s, v) => restored.push((s, v)),
        }
    }
    next.is_some()
}

/// Yalnızca ayar kimlikleri, sonuç adları ve dokunulmayan dosyaların `~/…` yolları (değer yok).
/// Uyarılar yalnızca sorun kümesi değişince (`problems_changed`): her turda aynısı loglanmaz.
fn log_report(r: &ApplyReport, problems_changed: bool, took: Duration) {
    if !r.written.is_empty() {
        tracing::info!(written = ?r.written, live = ?r.live, ms = took.as_millis() as u64, "birleşik görünüm uygulandı");
    }
    if !problems_changed {
        return;
    }
    for e in &r.errors {
        tracing::warn!("birleşik görünüm: {e}");
    }
    for e in &r.external {
        tracing::warn!(
            setting = e.setting,
            path = %e.path,
            reason = e.reason,
            "birleşik görünüm: dosya başka bir araç tarafından yönetiliyor, dokunulmadı (ayarın geri kalanı uygulandı)"
        );
    }
    if r.errors.is_empty() && r.external.is_empty() {
        tracing::info!("birleşik görünüm: önceki sorunlar giderildi");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_request_keeps_latest_and_collects_restores() {
        let (tx, rx) = mpsc::channel();
        let mut next = None;
        let mut restored = Vec::new();
        assert!(!newer_request(&rx, &mut next, &mut restored, &mut false), "boş kuyruk turu kesmez");
        tx.send(Job::Restored("look.icons".into(), Box::default())).unwrap();
        assert!(!newer_request(&rx, &mut next, &mut restored, &mut false), "geri yükleme turu kesmez");
        assert_eq!(restored.len(), 1);
        let a = nuxut::Theme { contrast: nuxut::theme::ContrastMode::High, ..Default::default() };
        tx.send(Job::Sync(Box::default())).unwrap();
        tx.send(Job::Sync(Box::new(a.clone()))).unwrap();
        assert!(newer_request(&rx, &mut next, &mut restored, &mut false));
        assert!(same_input(next.as_deref().unwrap(), &a), "en yeni istek kazanır");
    }

    #[test]
    fn input_equality_ignores_non_look_fields() {
        let a = nuxut::Theme::default();
        let mut b = a.clone();
        b.shape.radius_small += 1.0;
        assert!(same_input(&a, &b), "köşe yarıçapı görünüm planını etkilemez");
        b.desktop.unified = !a.desktop.unified;
        assert!(!same_input(&a, &b));
    }

    /// Erişilebilirlik girdileri (t124) de turu tetikler: karşıtlık ve azaltılmış hareket.
    #[test]
    fn input_equality_sees_contrast_and_motion() {
        let a = nuxut::Theme::default();
        let mut b = a.clone();
        b.contrast = nuxut::theme::ContrastMode::High;
        assert!(!same_input(&a, &b));
        let mut c = a.clone();
        c.motion.enabled = !a.motion.enabled;
        assert!(!same_input(&a, &c));
    }
}
