//! `msg defaults status|set` (sözleşme defaults-v1): istekler `nuxut-defaults` thread'inde
//! sırayla işlenir (`services::default_apps::Worker`; masaüstü indeksi önbelleği thread'de kalır).
//! Thread ilk komutta açılır; boşta iş yok. Düşürülünce kanal kapanır ve thread beklenir.

use std::sync::mpsc;
use std::thread::JoinHandle;

use nuxut::Context;
use nuxut::wayland::MessageSender;

use super::{Shell, ShellMessage};
use crate::ipc::DefaultsCommand;
use crate::services::default_apps::{Dirs, Worker};

type Job = (DefaultsCommand, nuxut::ipc::Reply);

#[derive(Debug, Default)]
pub(super) struct DefaultsRuntime {
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for DefaultsRuntime {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl DefaultsRuntime {
    fn sender(&mut self, env: &nuxut_apply::Env, shell: &MessageSender<ShellMessage>) -> Option<&mpsc::Sender<Job>> {
        if self.tx.is_none() {
            let (tx, rx) = mpsc::channel::<Job>();
            let env = env.clone();
            let shell = shell.clone();
            let spawned = std::thread::Builder::new().name("nuxut-defaults".into()).spawn(move || {
                let dirs = Dirs::from_env(&env);
                let mut worker = Worker::new(env, dirs);
                for (cmd, reply) in rx {
                    let text = match cmd {
                        DefaultsCommand::Status => worker.status_json(),
                        DefaultsCommand::Set { role, id } => worker.set_json(&role, &id),
                        // Başlatma ana thread'deki başlatıcı servisinde (kapsam, ortam, tek yol).
                        DefaultsCommand::Open { role } => {
                            let (json, entry) = worker.open(&role);
                            if let Some(entry) = entry {
                                let _ = shell.send(ShellMessage::LaunchDefault(entry));
                            }
                            json
                        }
                    };
                    reply.send(&text);
                }
            });
            match spawned {
                Ok(t) => {
                    self.tx = Some(tx);
                    self.thread = Some(t);
                }
                Err(e) => tracing::warn!("varsayılan uygulamalar thread'i açılamadı: {e}"),
            }
        }
        self.tx.as_ref()
    }
}

impl Shell {
    /// `msg defaults …`; `open` başlatmayı [`ShellMessage::LaunchDefault`] ile ana thread'e verir.
    pub(super) fn defaults_command(&mut self, cx: &mut Context<Self>, cmd: DefaultsCommand, reply: nuxut::ipc::Reply) {
        let env = self.apply_env.clone();
        match self.defaults.sender(&env, &cx.sender()) {
            Some(tx) => {
                if let Err(mpsc::SendError((_, reply))) = tx.send((cmd, reply)) {
                    reply.send("error: defaults thread'i kapalı");
                }
            }
            None => reply.send("error: defaults thread'i açılamadı"),
        }
    }
}
