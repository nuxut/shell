//! Ekranlar (ADR-0005): `msg displays …` komutları, onay zamanlayıcısı ve hotplug,
//! `nuxut-displays` thread'inde ([`DisplayManager`]). Ana thread yalnızca komutu ve monitör
//! değişimini iletir; Hyprland istekleri (≤ 1 sn zaman aşımı, doğrulama beklemesi) UI'ı
//! bekletmez. Yanıtı thread verir.
//!
//! **Güvenlik:** Hyprland'e yazma yalnızca [`SyncEnv::may_control`] doğruyken (kullanıcının
//! oturumu ya da yalıtılmış iç içe test Hyprland'i); dosyalar yalnızca [`SyncEnv::guard`]
//! geçerse (test örneğinde geçici `XDG_CONFIG_HOME` altında).

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nuxut::Context;

use super::{Shell, ShellMessage};
use crate::ipc::DisplaysCommand;
use crate::services::displays::guard::{PendingRecord, pending_path, read_pending};
use crate::services::displays::manager::{DisplayFiles, DisplayManager, Now, Options};
use crate::services::displays::render::{CONF_HEADER, LUA_HEADER, generated_rel_path};
use crate::services::displays::store::{FILE_NAME, HEADER};
use crate::services::hypr_config::control::HyprControl;
use crate::services::hypr_config::sync::{ConfigKind, SyncEnv};
use crate::services::apply;
use crate::services::hypr_config::sync::MAX_OWNED_BYTES;

#[derive(Debug)]
enum Job {
    Command(DisplaysCommand, nuxut::ipc::Reply),
    /// Hyprland monitör listesi değişti (hotplug, açma/kapama, kip): yöneticinin yeniden
    /// okuması gerekir.
    MonitorsChanged,
    ConfirmSeconds(u32),
}

#[derive(Debug)]
pub(super) struct DisplaysWorker {
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
    /// Son iletilen monitör özeti (aynısı tekrar gönderilmez).
    last_monitors: Vec<(String, String, i32, i32, u32, u32, i64)>,
    confirm_seconds: u32,
}

impl Drop for DisplaysWorker {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Gerçek dosya erişimi. Yollar her yazımda çözülür (ayar dili oturum içinde değişebilir).
/// Yazımlar `nuxut-apply` ile (`apply::write_derived`: atomik, symlink korunur, ortam kapısı;
/// Nuxut başlığı taşımayan dosyanın üzerine yazılmaz). Snapshot yok (ADR-0001 §9.3: `displays`
/// kimlik değil; geri dönüş Ekranlar'ın kendi onay/`displays revert` akışı).
struct FsFiles {
    env: SyncEnv,
    /// `guard` geçti: yazım serbest.
    writable: bool,
    pending: Option<PathBuf>,
}

impl FsFiles {
    fn store_path(&self) -> Option<PathBuf> {
        let base = self.env.apply.config_home.as_ref().filter(|p| p.is_absolute())?;
        Some(base.join("nuxut").join(FILE_NAME))
    }

    fn generated_path(&self, kind: ConfigKind) -> Option<PathBuf> {
        let config = self.env.config_path()?;
        Some(config.parent()?.join(generated_rel_path(kind)))
    }

    fn write(&self, path: &std::path::Path, text: &str) -> Result<(), String> {
        apply::write_derived(&self.env.apply, path, text.as_bytes()).map(|_| ()).map_err(|e| apply::error_text(&e))
    }
}

fn read_small(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut s = String::new();
    std::fs::File::open(path).ok()?.take(MAX_OWNED_BYTES + 1).read_to_string(&mut s).ok()?;
    (s.len() as u64 <= MAX_OWNED_BYTES).then_some(s)
}

impl DisplayFiles for FsFiles {
    fn read_store(&self) -> Option<String> {
        read_small(&self.store_path()?)
    }

    fn write_store(&self, text: &str) -> Result<(), String> {
        if !self.writable {
            return Err("bu örnek dosya yazmaz".into());
        }
        let path = self.store_path().ok_or("displays.toml yolu yok")?;
        if read_small(&path).as_deref() == Some(text) {
            return Ok(());
        }
        debug_assert!(HEADER.starts_with(CONF_HEADER));
        self.write(&path, text)
    }

    fn write_generated(&self, kind: ConfigKind, text: &str, create: bool) -> Result<(), String> {
        if !self.writable {
            return Err("bu örnek dosya yazmaz".into());
        }
        let path = self.generated_path(kind).ok_or("Hyprland ayar dizini yok")?;
        let old = read_small(&path);
        if old.as_deref() == Some(text) || (old.is_none() && !create && !path.exists()) {
            return Ok(());
        }
        debug_assert!(text.starts_with(LUA_HEADER) || text.starts_with(CONF_HEADER));
        self.write(&path, text)
    }

    fn read_pending(&self) -> Option<PendingRecord> {
        read_pending(self.pending.as_ref()?)
    }

    fn write_pending(&self, record: &PendingRecord) -> Result<(), String> {
        let path = self.pending.as_ref().ok_or("XDG_RUNTIME_DIR yok")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, record.to_json()).and_then(|()| std::fs::rename(&tmp, path)).map_err(|e| e.to_string())
    }

    fn remove_pending(&self) {
        if let Some(p) = &self.pending {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Bir komutu işler; yanıt metni.
fn handle(m: &mut DisplayManager, cmd: DisplaysCommand) -> (String, bool) {
    let mono = Instant::now();
    let err = |e: crate::services::displays::protocol::ApiError| (e.line(), false);
    match cmd {
        DisplaysCommand::List => m.list(mono).map_or_else(err, |s| (s, false)),
        DisplaysCommand::Status => (m.status(mono), false),
        DisplaysCommand::Apply(json) => match m.apply(&json, Now::system()) {
            Ok(a) => (a.json(), !a.confirm),
            Err(e) => err(e),
        },
        DisplaysCommand::Confirm(t) => m.confirm(&t).map_or_else(err, |()| ("ok".into(), true)),
        DisplaysCommand::Revert(t) => m.revert(&t).map_or_else(err, |()| ("ok".into(), false)),
    }
}

impl Shell {
    /// `start`'ta, Hyprland servisinden sonra. Hyprland yoksa ekran komutları `unavailable`.
    pub(super) fn start_displays(&mut self, cx: &mut Context<Self>) {
        let Some(reader) = self.client.clone() else { return };
        let env = SyncEnv::from_process(self.apply_env.clone());
        let writable = env.guard().is_ok();
        let writer = if writable && env.may_control() { Some(reader.clone()) } else { None };
        let app = crate::ipc::app_name().unwrap_or_else(|_| crate::ipc::APP_NAME.into());
        let pending = env.runtime_dir.as_deref().and_then(|d| pending_path(d, &app));
        let confirm_seconds = self.model.config.displays.confirm_seconds;
        let opts = Options {
            confirm_seconds,
            app,
            exe: std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
            ..Options::default()
        };
        let (tx, rx) = mpsc::channel::<Job>();
        let sender = cx.sender();
        let spawned = std::thread::Builder::new().name("nuxut-displays".into()).spawn(move || {
            let files = FsFiles { env, writable, pending };
            let mut m = DisplayManager::new(
                opts,
                Box::new(reader),
                writer.map(|w| Box::new(w) as Box<dyn HyprControl>),
                Box::new(files),
            );
            m.startup();
            let _ = sender.send(ShellMessage::DisplaysPersisted);
            loop {
                let job = match m.deadline() {
                    Some(d) => match rx.recv_timeout(d.saturating_duration_since(Instant::now())) {
                        Ok(j) => Some(j),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    },
                    None => match rx.recv() {
                        Ok(j) => Some(j),
                        Err(_) => break,
                    },
                };
                // Süre her işten önce denetlenir (onay, son andan sonra gelirse geçmez).
                m.tick(Instant::now());
                match job {
                    None => {}
                    Some(Job::Command(cmd, reply)) => {
                        let (text, persisted) = handle(&mut m, cmd);
                        reply.send(&text);
                        if persisted {
                            let _ = sender.send(ShellMessage::DisplaysPersisted);
                        }
                    }
                    Some(Job::MonitorsChanged) if m.on_monitors_changed() => {
                        let _ = sender.send(ShellMessage::DisplaysPersisted);
                    }
                    Some(Job::MonitorsChanged) => {}
                    Some(Job::ConfirmSeconds(s)) => m.set_confirm_seconds(s),
                }
            }
            // Kapanış: bekleyen değişiklik onaylanmadı → geri dön.
            if let Some(d) = m.deadline() {
                m.tick(d.max(Instant::now()) + Duration::from_millis(1));
            }
        });
        match spawned {
            Ok(thread) => {
                self.displays = Some(DisplaysWorker {
                    tx: Some(tx),
                    thread: Some(thread),
                    last_monitors: Vec::new(),
                    confirm_seconds,
                });
            }
            Err(e) => tracing::warn!("ekran thread'i açılamadı: {e}"),
        }
    }

    /// `msg displays …`: yanıtı ekran thread'i verir.
    pub(super) fn displays_command(&mut self, cmd: DisplaysCommand, reply: nuxut::ipc::Reply) {
        match self.displays.as_ref().and_then(|w| w.tx.as_ref()) {
            Some(tx) => {
                if let Err(mpsc::SendError(Job::Command(_, reply))) = tx.send(Job::Command(cmd, reply)) {
                    reply.send("error: unavailable: ekran servisi çalışmıyor");
                }
            }
            None => reply.send("error: unavailable: Hyprland bağlantısı yok"),
        }
    }

    /// Hyprland durumu değişti: monitör listesi farklıysa ekran thread'i yeniden okusun.
    pub(super) fn displays_monitors_changed(&mut self) {
        let Some(w) = self.displays.as_mut() else { return };
        let now: Vec<_> = self
            .model
            .hypr
            .monitors
            .iter()
            .map(|m| (m.name.clone(), m.description.clone(), m.x, m.y, m.width, m.height, (m.scale * 1000.0).round() as i64))
            .collect();
        if now != w.last_monitors {
            w.last_monitors = now;
            if let Some(tx) = &w.tx {
                let _ = tx.send(Job::MonitorsChanged);
            }
        }
    }

    /// shell.toml yeniden yüklendi.
    pub(super) fn displays_config_changed(&mut self) {
        let s = self.model.config.displays.confirm_seconds;
        if let Some(w) = self.displays.as_mut()
            && w.confirm_seconds != s
        {
            w.confirm_seconds = s;
            if let Some(tx) = &w.tx {
                let _ = tx.send(Job::ConfirmSeconds(s));
            }
        }
    }
}
