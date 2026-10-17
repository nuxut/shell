//! Büyüteç (t124): `services::magnifier` işleri `nuxut-magnifier` thread'inde yürür (Hyprland
//! isteği UI'ı bekletmez). Ana thread yalnızca işi gönderir ve gelen görünümü modele koyar.
//! Tasarım: `docs/design/magnifier.md`.
//!
//! **Güvenlik:** Hyprland'e yazma yalnızca [`SyncEnv::may_control`] doğruyken (kullanıcının
//! oturumundaki asıl örnek ya da yalıtılmış test Hyprland'i); test örneği kullanıcının
//! oturumuna asla komut göndermez — kart gizli, `msg magnifier` hata.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::thread::JoinHandle;

use nuxut::Context;

use super::{Shell, ShellMessage};
use crate::ipc::MagnifierCommand;
use crate::services::hypr_config::control::HyprControl;
use crate::services::hypr_config::sync::SyncEnv;
use crate::services::hyprland::HyprlandClient;
use crate::services::magnifier::{
    Command, Magnifier, MagnifierConfig, MagnifierView, Marker, read_live, reapply, recover, release, run,
};

/// Büyüteç thread'ine giden iş.
enum Job {
    /// Eylem; `reply` varsa (`msg`) sonuç yanıtlanır.
    Run(Command, Option<nuxut::ipc::Reply>),
    /// `msg magnifier status`.
    Status(nuxut::ipc::Reply),
    /// `[magnifier]` değişti.
    Config(MagnifierConfig),
    /// Hyprland ayarı yeniden yüklendi (`configreloaded`): kabuğun büyütmesi geri yazılır.
    Reloaded,
    /// Canlı değeri yeniden oku (panel açıldı).
    Refresh,
}

impl Job {
    /// Kaydırıcının art arda gönderdiği yanıtsız `set`'ler birleştirilebilir.
    fn is_silent_set(&self) -> bool {
        matches!(self, Job::Run(Command::Set(_), None))
    }
}

/// Büyüteç thread'i; düşürülünce (kapanış) kanal kapanır, thread kendi yazdığı büyütmeyi geri
/// alır ve beklenir.
#[derive(Debug, Default)]
pub(super) struct MagnifierRuntime {
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
    /// Kapalıysa nedeni (`msg` yanıtı; tanı).
    disabled: Option<String>,
}

impl MagnifierRuntime {
    /// Kanalı kapatır ve thread'i bekler: kuyruktaki işler ve geri alma bitmeden dönmez (her
    /// Hyprland isteği en çok [`HyprlandClient::DEFAULT_TIMEOUT`]).
    fn stop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::warn!("büyüteç thread'i panikle bitti");
        }
    }
}

impl Drop for MagnifierRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Thread gövdesi: çökme işaretini çözer, işleri sırayla yürütür, kanal kapanınca kendi
/// büyütmesini geri alır.
fn worker(client: HyprlandClient, marker: Option<Marker>, config: MagnifierConfig, rx: mpsc::Receiver<Job>, publish: impl Fn(MagnifierView)) {
    let ctl: &dyn HyprControl = &client;
    let mut m = Magnifier::new(&config);
    let sync_marker = |m: &Magnifier| {
        if let Some(mk) = &marker {
            mk.sync(m);
        }
    };
    if let Some(f) = marker.as_ref().and_then(Marker::read) {
        match recover(&mut m, ctl, f) {
            Ok(true) => tracing::info!(factor = f, "önceki örneğin bıraktığı büyütme geri alındı"),
            Ok(false) => tracing::info!(factor = f, "önceki örneğin büyütmesi değişmiş, dokunulmadı"),
            Err(e) => tracing::warn!("önceki örneğin büyütmesi geri alınamadı: {e}"),
        }
        sync_marker(&m);
    }
    let current = |m: &Magnifier| match read_live(ctl) {
        Ok(f) => m.view(f),
        Err(_) => MagnifierView { available: false, factor: 1.0, max: m.max() },
    };
    publish(current(&m));
    let mut queue: VecDeque<Job> = VecDeque::new();
    while let Some(mut job) = queue.pop_front().or_else(|| rx.recv().ok()) {
        // Kaydırıcı sürüklenirken biriken yanıtsız `set`'lerden yalnızca sonuncusu uygulanır.
        if job.is_silent_set() {
            while let Ok(next) = rx.try_recv() {
                if next.is_silent_set() {
                    job = next;
                } else {
                    queue.push_back(next);
                    break;
                }
            }
        }
        match job {
            Job::Run(cmd, reply) => {
                let result = run(&mut m, ctl, cmd);
                sync_marker(&m);
                match &result {
                    Ok(view) => publish(*view),
                    Err(e) => {
                        tracing::warn!("büyüteç {cmd:?}: {e}");
                        publish(current(&m));
                    }
                }
                if let Some(r) = reply {
                    match result {
                        Ok(_) => r.send("ok"),
                        Err(e) => r.send(&format!("error: magnifier: {e}")),
                    }
                }
            }
            Job::Status(reply) => {
                let live = read_live(ctl);
                if let Ok(f) = live {
                    m.observe(f);
                    sync_marker(&m);
                }
                reply.send(&m.status(live).to_json());
            }
            Job::Config(c) => {
                m.set_config(&c);
                publish(current(&m));
            }
            Job::Reloaded => {
                match reapply(&mut m, ctl) {
                    Ok(Some(f)) => tracing::info!(factor = f, "ayar yeniden yüklendi: büyütme yeniden uygulandı"),
                    Ok(None) => {}
                    Err(e) => tracing::warn!("ayar yeniden yüklendi, büyütme yeniden uygulanamadı: {e}"),
                }
                sync_marker(&m);
                publish(current(&m));
            }
            Job::Refresh => {
                let view = current(&m);
                if view.available {
                    m.observe(view.factor);
                    sync_marker(&m);
                }
                publish(view);
            }
        }
    }
    // Kanal kapandı (kapanış): kendi büyütmemiz hâlâ canlıysa 1'e.
    match release(&mut m, ctl) {
        Ok(true) => tracing::info!("kapanış: büyütme geri alındı"),
        Ok(false) => {}
        Err(e) => tracing::warn!("kapanış: büyütme geri alınamadı: {e}"),
    }
    sync_marker(&m);
}

impl Shell {
    /// `start`'ta, Hyprland servisinden sonra. Komut izni yoksa kart gizli kalır.
    pub(super) fn start_magnifier(&mut self, cx: &mut Context<Self>) {
        let env = SyncEnv::from_process(self.apply_env.clone());
        if !env.may_control() {
            let reason = env.guard().err().unwrap_or("bu örnek Hyprland'e komut göndermez (test örneği)"); // i18n: tanı msg yanıtı (çevrilmez)
            tracing::info!("büyüteç kapalı: {reason}");
            self.magnifier.disabled = Some(reason.to_owned());
            return;
        }
        let Some(client) = self.client.clone() else {
            self.magnifier.disabled = Some("Hyprland bağlantısı yok".into()); // i18n: tanı msg yanıtı (çevrilmez)
            return;
        };
        let marker = Marker::from_env();
        let config = self.model.config.magnifier.clone();
        let (tx, rx) = mpsc::channel();
        let sender = cx.sender();
        let spawned = std::thread::Builder::new().name("nuxut-magnifier".into()).spawn(move || {
            worker(client, marker, config, rx, move |view| {
                let _ = sender.send(ShellMessage::Magnifier(view));
            });
        });
        match spawned {
            Ok(t) => {
                self.magnifier.tx = Some(tx);
                self.magnifier.thread = Some(t);
            }
            Err(e) => {
                tracing::warn!("büyüteç thread'i açılamadı: {e}");
                self.magnifier.disabled = Some(format!("thread açılamadı: {e}")); // i18n: tanı msg yanıtı (çevrilmez)
            }
        }
    }

    fn send_magnifier(&self, job: Job) -> Result<(), String> {
        self.send_magnifier_or_return(job).map_err(|(e, _)| e)
    }

    /// `msg magnifier …`: yanıtı büyüteç thread'i verir.
    pub(super) fn magnifier_command(&mut self, c: MagnifierCommand, reply: nuxut::ipc::Reply) {
        let job = match c {
            MagnifierCommand::Status => Job::Status(reply),
            MagnifierCommand::Run(cmd) => Job::Run(cmd, Some(reply)),
        };
        if let Err((e, job)) = self.send_magnifier_or_return(job) {
            match job {
                Job::Status(r) => {
                    let m = Magnifier::new(&self.model.config.magnifier);
                    r.send(&m.status(Err(e)).to_json());
                }
                Job::Run(_, Some(r)) => r.send(&format!("error: {e}")),
                _ => {}
            }
        }
    }

    /// Gönderilemeyen işi (yanıtı verilsin diye) geri verir.
    fn send_magnifier_or_return(&self, job: Job) -> Result<(), (String, Job)> {
        match &self.magnifier.tx {
            Some(tx) => tx.send(job).map_err(|mpsc::SendError(job)| ("büyüteç thread'i çalışmıyor".to_owned(), job)), // i18n: tanı msg yanıtı (çevrilmez)
            None => Err((
                format!(
                    "büyüteç kullanılamıyor: {}", // i18n: tanı msg yanıtı (çevrilmez)
                    self.magnifier.disabled.as_deref().unwrap_or("başlatılmadı") // i18n: tanı msg yanıtı (çevrilmez)
                ),
                job,
            )),
        }
    }

    /// Kart ya da bildirim düğmesi: yanıtsız eylem.
    pub(super) fn magnifier_action(&mut self, cmd: Command) -> Result<(), String> {
        self.send_magnifier(Job::Run(cmd, None))
    }

    /// Thread'den yeni görünüm.
    pub(super) fn on_magnifier(&mut self, cx: &mut Context<Self>, view: MagnifierView) {
        // Çarpan değiştiyse OSD (docs/design/magnifier.md "Bar hapı ve OSD").
        self.osd_magnifier(cx, &view);
        if self.model.magnifier != view {
            self.model.magnifier = view;
            self.model_changed(cx);
        }
    }

    /// Panel açıldı: kart görünür olabilir, canlı değer yeniden okunur (başka bir yol —
    /// kullanıcının kendi bind'i, Hyprland ayarının yeniden yüklenmesi — değiştirmiş olabilir).
    pub(super) fn magnifier_refresh(&mut self) {
        if self.model.magnifier.available || self.magnifier.tx.is_some() {
            let _ = self.send_magnifier(Job::Refresh);
        }
    }

    /// Hyprland `configreloaded` (`app/hyprland.rs`).
    pub(super) fn magnifier_config_reloaded(&mut self) {
        if self.magnifier.tx.is_some() {
            let _ = self.send_magnifier(Job::Reloaded);
        }
    }

    /// Config yeniden yüklendi.
    pub(super) fn magnifier_config_changed(&mut self) {
        let _ = self.send_magnifier(Job::Config(self.model.config.magnifier.clone()));
    }

    /// Kapanış (SIGTERM, `msg quit`): kabuğun yazdığı büyütme 1'e döner; thread beklenir.
    pub(super) fn magnifier_shutdown(&mut self) {
        self.magnifier.stop();
    }
}
