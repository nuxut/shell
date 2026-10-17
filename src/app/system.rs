//! Sistem servisleri bağlantısı (ses, ağ, Bluetooth, güç, parlaklık, medya): başlatma,
//! olaylar, hızlı ayar kartlarının ve OSD/`msg` komutlarının uygulanması. Tüm sistem
//! servisleri yalnızca burada başlar.

use std::time::Instant;

use nuxut::{Context, SurfaceId};

use super::osd::OsdKind;
use super::{Shell, ShellMessage};
use crate::bar_edge::{EdgeCommand, ScrollAction};
use crate::services::audio::{AudioEvent, AudioTarget};
use crate::services::brightness::BrightnessEvent;
use crate::system::{Level, Mute, SystemCommand, SystemEvent, SystemServices, system_options};

impl Shell {
    pub(super) fn start_system(&mut self, cx: &mut Context<Self>) {
        let sender = cx.sender();
        let options = system_options(&self.model.config);
        let art_sandbox = crate::services::image_sandbox::decoder(&self.image_sandbox);
        let services = SystemServices::spawn(options, move |e| {
            let _ = sender.send(ShellMessage::System(e));
        });
        self.system = Some(match art_sandbox {
            Some(sandbox) => services.with_art_sandbox(sandbox),
            None => services,
        });
    }

    pub(super) fn on_system(&mut self, cx: &mut Context<Self>, e: SystemEvent) {
        // OSD önceki değeri modelden okuyabilsin diye uygulamadan önce.
        self.system_osd(cx, &e);
        if let SystemEvent::Power(crate::services::power::PowerEvent::ChargeLimitDone { id, result }) = &e {
            self.charge_limit_done(*id, result);
        }
        let Some(s) = &mut self.system else { return };
        if s.on_event(e, &mut self.model.system) {
            self.model_changed(cx);
            // Açık ipucu (durum göstergesi) yeni değeri göstersin.
            let theme = cx.theme().clone();
            let now = Instant::now();
            for screen in &mut self.screens {
                if screen.refresh_tooltip(&self.model.config, cx.text(), &theme, now) {
                    cx.request_frame(screen.main);
                }
            }
        }
        self.sync_idle_system_and_apply(cx);
    }

    /// OSD kancaları (app/osd.rs).
    fn system_osd(&mut self, cx: &mut Context<Self>, e: &SystemEvent) {
        match e {
            SystemEvent::Audio(AudioEvent::State(s)) => self.osd_audio(cx, s),
            SystemEvent::Brightness(BrightnessEvent::Displays(d)) => self.osd_displays(d),
            SystemEvent::Brightness(BrightnessEvent::Changed(d)) => self.osd_brightness(cx, d),
            _ => {}
        }
    }

    /// Kart komutu. Komut modeli de değiştirebilir (cevaplanan şifre isteği, kapatılan hata
    /// satırı).
    pub(super) fn system_command(&mut self, cx: &mut Context<Self>, cmd: SystemCommand) -> bool {
        let Some(s) = &self.system else { return false };
        let applied = s.command(cmd, &mut self.model.system);
        self.model_changed(cx);
        applied
    }

}

// `msg volume|mic|brightness` ve OSD tuşları (çağıran: app/commands.rs).
impl Shell {
    /// Çıkış sesi (`msg volume`, ses tuşları). Göreli adım serviste uygulanır (tuş
    /// tekrarında bayat modele bakılmaz); üst sınır `[audio] max_volume`.
    pub(super) fn apply_volume(&mut self, cx: &mut Context<Self>, level: Level) -> Result<(), String> {
        self.audio_command(cx, SystemCommand::Volume { target: AudioTarget::DefaultSink, level })
    }

    pub(super) fn apply_volume_mute(&mut self, cx: &mut Context<Self>, mute: Mute) -> Result<(), String> {
        self.audio_command(cx, SystemCommand::SetMute { target: AudioTarget::DefaultSink, mute })
    }

    pub(super) fn apply_mic_mute(&mut self, cx: &mut Context<Self>, mute: Mute) -> Result<(), String> {
        self.audio_command(cx, SystemCommand::SetMute { target: AudioTarget::DefaultSource, mute })
    }

    /// Parlaklık (`msg brightness`, parlaklık tuşları). Hedef: `output` çıktısındaki ekran,
    /// yoksa birincil arka ışık ([`crate::system::SystemModel::brightness_display`]).
    pub(super) fn apply_brightness(
        &mut self,
        cx: &mut Context<Self>,
        output: Option<&str>,
        level: Level,
    ) -> Result<(), String> {
        let Some(display) = self.model.system.brightness_display(output) else {
            return Err("parlaklığı ayarlanabilen ekran yok".into());
        };
        let id = display.id.clone();
        if self.system_command(cx, SystemCommand::Brightness { id, level }) {
            Ok(())
        } else {
            Err("parlaklık servisi çalışmıyor".into())
        }
    }

    fn audio_command(&mut self, cx: &mut Context<Self>, cmd: SystemCommand) -> Result<(), String> {
        if self.system_command(cx, cmd) { Ok(()) } else { Err("ses servisi çalışmıyor".into()) }
    }

    /// Bar uç eylemi (`[bar.edge_actions]`): `msg` komutlarıyla aynı `apply_*` yolları. OSD
    /// servis olayından gelir; monitörü bar'ın monitörü olarak istenir (odaklı monitör değil).
    pub(super) fn bar_edge_command(&mut self, cx: &mut Context<Self>, surface: SurfaceId, cmd: EdgeCommand) {
        let Some(monitor) = self.screens.iter().find(|s| s.main == surface).map(|s| s.name.clone()) else { return };
        let step = i32::try_from(self.model.config.bar.edge_actions.scroll_step).unwrap_or(1);
        let level = |steps: i32| Level::Step(i16::try_from(steps.saturating_mul(step)).unwrap_or(0));
        let request = |kind: OsdKind, this: &mut Self| this.osd_request = Some((kind, monitor.clone(), Instant::now()));
        let result = match cmd {
            EdgeCommand::Scroll { action, steps } => match action {
                ScrollAction::Volume => {
                    request(OsdKind::Volume, self);
                    self.apply_volume(cx, level(steps))
                }
                ScrollAction::Mic => {
                    request(OsdKind::Mic, self);
                    self.audio_command(cx, SystemCommand::Volume { target: AudioTarget::DefaultSource, level: level(steps) })
                }
                ScrollAction::Brightness => {
                    request(OsdKind::Brightness, self);
                    self.apply_brightness(cx, Some(&monitor), level(steps))
                }
                // Yukarı kaydırma önceki workspace (tekerlek yukarı = sola); widget kaydırmasıyla tek yol.
                ScrollAction::Workspace => {
                    self.scroll_workspace(&monitor, -steps);
                    Ok(())
                }
                ScrollAction::Widget | ScrollAction::None => Ok(()),
            },
            EdgeCommand::ToggleMute => {
                request(OsdKind::Volume, self);
                self.apply_volume_mute(cx, Mute::Toggle)
            }
            EdgeCommand::ToggleMicMute => {
                request(OsdKind::Mic, self);
                self.apply_mic_mute(cx, Mute::Toggle)
            }
        };
        if let Err(e) = result {
            tracing::debug!("bar uç eylemi uygulanamadı: {e}");
        }
    }
}

impl Shell {
    /// Config yeniden yüklendi: `[audio] max_volume` ya da `backend` değiştiyse ses servisi yeniden başlar;
    /// `[media] remote_art` canlı uygulanır.
    pub(super) fn sync_system_config(&mut self) {
        let options = system_options(&self.model.config);
        let Some(s) = &mut self.system else { return };
        if options.audio.max_volume != self.model.system.max_volume || options.audio.backend != s.audio_backend() {
            s.set_audio_options(options.audio, &mut self.model.system);
        }
        if options.remote_art != self.model.system.remote_art {
            s.set_remote_art(options.remote_art, &mut self.model.system);
        }
    }
}
