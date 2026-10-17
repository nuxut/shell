//! Saat: bar biçiminin en küçük birimine hizalı timer; saat dilimi değişimi (timedate1 ya da
//! `msg clock refresh`) sonrası yeniden hesaplama.

use nuxut::{Context, TimerId};

use super::{Shell, ShellMessage};
use crate::config::ShellConfig;
use crate::services::clock::ClockFormat;
use crate::services::timezone::{self, TimezoneEvent, TimezoneOptions, TimezoneWatch};

impl Shell {
    /// Saat metinlerini günceller ve timer'ı bar biçiminin en küçük birimine hizalı kurar.
    pub(super) fn tick_clock(&mut self, cx: &mut Context<Self>) {
        let now = chrono::Local::now();
        let theme = cx.theme().clone();
        for s in &mut self.screens {
            if s.set_time(&now, &self.model.config, &theme) {
                cx.request_frame(s.main);
            }
        }
        if let Some(t) = self.clock_timer.take() {
            cx.cancel_timer(t);
        }
        self.clock_timer = Some(cx.set_timer(self.clock.until_next_tick(&now)));
        // Gün döndüyse sıradaki etkinliğin yakın aralığı yeniden sorgulanır (değişmediyse iş yok).
        self.refresh_calendar_range(false);
        // Ek uyanma olmadan: saat ayarlandıysa gece ışığı yeniden hesaplar.
        self.night_light_clock_check(cx);
    }

    /// Saat ya da dilim timer'ıysa işler ve `true` döner.
    pub(super) fn clock_timer_fired(&mut self, cx: &mut Context<Self>, id: TimerId) -> bool {
        if self.tz_timer == Some(id) {
            self.tz_timer = None;
            self.timezone_settled(cx);
            return true;
        }
        if self.clock_timer != Some(id) {
            return false;
        }
        self.clock_timer = None;
        self.tick_clock(cx);
        true
    }

    /// timedate1 dinleyicisi (salt dinleme; test örneğinde de güvenli, bkz. `services::timezone`).
    pub(super) fn start_timezone(&mut self, cx: &mut Context<Self>) {
        let sender = cx.sender();
        match TimezoneWatch::spawn(TimezoneOptions::default(), move |e| {
            let _ = sender.send(ShellMessage::Timezone(e));
        }) {
            Ok(w) => self.timezone = Some(w),
            Err(e) => tracing::warn!("saat dilimi: dinleyici başlatılamadı: {e}"),
        }
    }

    pub(super) fn on_timezone(&mut self, cx: &mut Context<Self>, e: TimezoneEvent) {
        match e {
            TimezoneEvent::Changed { .. } => self.refresh_timezone(cx),
        }
    }

    /// Saat dilimi değişti (timedate1 sinyali ya da `msg clock refresh`): `chrono::Local`'ın
    /// `/etc/localtime` önbelleği dolsun diye [`timezone::SETTLE`] sonra yeniden hesaplanır.
    /// Art arda gelen istekler tek yeniden hesaplamada birleşir.
    pub(super) fn refresh_timezone(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.tz_timer.take() {
            cx.cancel_timer(t);
        }
        self.tz_timer = Some(cx.set_timer(timezone::SETTLE));
    }

    /// Yeni dilimle: metin kuran her şey yeniden kurulur (bar saati, takvim "bugün"ü ve etkinlik
    /// saatleri, kartlar — dil değişimiyle aynı yol), takvim sorgusu yeni ofsete, saat timer'ı
    /// yeniden hizalanır; gece ışığının konumu ve planı yeni yerel saatle yeniden hesaplanır.
    fn timezone_settled(&mut self, cx: &mut Context<Self>) {
        tracing::info!(offset = chrono::Local::now().offset().to_string(), "saat dilimi: yeniden hesaplanıyor");
        self.relocalize_screens(cx);
        self.night_light_timezone_changed(cx);
        self.tick_clock(cx);
    }
}

/// Saat timer'ının hizalanacağı biçim: bar'ın yönüne göre etkin saat biçimi (kalıp yazılmadıysa
/// otomatik, dakikalık). Locale değişince (`locale_changed`) yeniden hesaplanır.
pub(super) fn tick_format(config: &ShellConfig) -> ClockFormat {
    let vertical = config.bar.is_vertical();
    ClockFormat::resolve(config.bar.widgets.clock.pattern(vertical), vertical, &crate::services::clock::time_locale())
}
