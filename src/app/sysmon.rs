//! Sistem izleyicisi: servis yalnızca bar'da `sysmon` widget'ı varken çalışır (yoksa hiç
//! örnekleme yok); ayrıntı kipi (süreçler, tüm sensörler) yalnızca bir ekranda popout açıkken.

use std::time::Instant;

use nuxut::Context;

use super::{Shell, ShellMessage};
use crate::config::BarSection;
use crate::services::sysmon::{SysmonEvent, SysmonService};
use crate::widgets::{PanelId, WidgetKind};

impl Shell {
    /// Servisi config'e eşler: bar'da widget varsa başlatır/ayarlarını verir, yoksa durdurur.
    pub(super) fn sync_sysmon_service(&mut self, cx: &mut Context<Self>) {
        let bar = &self.model.config.bar;
        let wanted = BarSection::ALL.into_iter().any(|s| bar.widgets_in(s).contains(&WidgetKind::Sysmon));
        match (&mut self.sysmon, wanted) {
            (Some(svc), true) => svc.set_config(&self.model.config.sysmon),
            (Some(_), false) => {
                self.sysmon = None;
                if self.model.sysmon.take().is_some() {
                    self.model_changed(cx);
                }
            }
            (None, true) => {
                let sender = cx.sender();
                match SysmonService::spawn(self.model.config.sysmon.clone(), move |e| {
                    let _ = sender.send(ShellMessage::Sysmon(e));
                }) {
                    Ok(svc) => self.sysmon = Some(svc),
                    Err(e) => tracing::warn!("sistem izleyici başlatılamadı: {e}"),
                }
            }
            (None, false) => {}
        }
        self.sync_sysmon_detail();
    }

    pub(super) fn on_sysmon(&mut self, cx: &mut Context<Self>, e: SysmonEvent) {
        let SysmonEvent::Sample(s) = e;
        // Servis durdurulduktan sonra kuyrukta kalmış ölçüm.
        if self.sysmon.is_none() {
            return;
        }
        self.model.sysmon = Some(s);
        let theme = cx.theme().clone();
        let now = Instant::now();
        for screen in &mut self.screens {
            if screen.sync_sysmon(&self.model, cx.text(), &theme, now) {
                cx.request_frame(screen.main);
            }
        }
        // Popout örtük yollarla da kapanır (Escape, dışarı tıklama, odak kaybı): ayrıntı kipi
        // en geç bir sonraki ölçümde düşer.
        self.sync_sysmon_detail();
        self.rearm_wake(cx);
    }

    /// Ayrıntı kipi = herhangi bir ekranda popout açık.
    pub(super) fn sync_sysmon_detail(&mut self) {
        let open = self.screens.iter().any(|s| s.is_panel_open(PanelId::Sysmon));
        if let Some(svc) = &mut self.sysmon {
            svc.set_detail(open);
        }
    }
}
