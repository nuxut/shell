//! `nuxut-shell doctor`un canlı bölümü (`msg doctor live`; docs/design/doctor.md): kabuğun
//! kimliği, Hyprland eşitleme ve görünüm durumu (var olan `hypr status` / `look status`
//! JSON'ları) ve ses özeti. Yalnız okur. Yanıtı ses thread'i verir; ses servisi yoksa hemen.

use std::sync::{Arc, Mutex};

use nuxut::Context;
use serde_json::{Value, json};

use super::Shell;

impl Shell {
    pub(super) fn doctor_live(&mut self, cx: &mut Context<Self>, reply: nuxut::ipc::Reply) {
        let look = crate::services::look::LookState::read(cx.theme())
            .ok()
            .map(|s| s.status_json(self.look.writer(), self.look.report.clone(), self.look.themes_rev()));
        let base = json!({
            "v": 1,
            "pid": std::process::id(),
            "version": env!("CARGO_PKG_VERSION"),
            "commit": crate::doctor::build_commit(),
            "instance": crate::ipc::app_name().ok(),
            "hypr": serde_json::from_str::<Value>(&self.hypr_status).ok(),
            "look": look.and_then(|l| serde_json::from_str::<Value>(&l).ok()),
            "audio": Value::Null,
            // Kilit dinleyicisi (docs/design/lock-trigger.md): ayar açık ve logind Lock sinyali bize ulaşabiliyor.
            "lock": {"listening": self.model.config.lock.listen_logind && self.idle.lock_listener_ready()},
            "clipboard": self.clipboard_doctor(),
        });
        let reply = Arc::new(Mutex::new(Some(reply)));
        let send = {
            let reply = Arc::clone(&reply);
            move |mut v: Value, audio: Option<crate::services::audio::AudioSummary>| {
                if let Some(a) = audio {
                    v["audio"] = serde_json::to_value(a).unwrap_or(Value::Null);
                }
                if let Some(r) = reply.lock().ok().and_then(|mut r| r.take()) {
                    r.send(&v.to_string());
                }
            }
        };
        let sent = {
            let (base, send) = (base.clone(), send.clone());
            self.system.as_ref().is_some_and(|s| s.audio_summary(Box::new(move |a| send(base, Some(a)))))
        };
        if !sent {
            send(base, None);
        }
    }
}
