//! Gece ışığı bağlantısı (`services::night_light`): arka uç, tek uyanma timer'ı, uykudan
//! uyanma ve saat sıçramasında yeniden hesap, hızlı ayar eylemleri, `msg night-light`.

use std::time::Instant;

use chrono::{DateTime, Local, Utc};
use nuxut::{ColorMatrix, ColorTransformError, Context, OutputId, TimerId};

use super::Shell;
use crate::ipc::NightLightCommand;
use crate::services::night_light::{ColorTransform, Command, NightLight, Now, Settings, location};

/// Test kancası: test örneğinde (`NUXUT_SHELL_INSTANCE` dolu) `1` ise arka uç destekli sayılır
/// ve çağrılar yalnızca loglanır (izole compositor'da kartı görmek için). Asıl örnekte yok sayılır.
const FAKE_ENV: &str = "NUXUT_NIGHT_LIGHT_FAKE";
/// Duvar saati ile monoton saat bu kadardan fazla ayrışırsa (saat ayarı) plan yeniden kurulur.
const CLOCK_JUMP: std::time::Duration = std::time::Duration::from_secs(2);

/// CTM arka ucunun türü.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// Compositor `hyprland-ctm-control-v1` sunmuyor: kart gizli, `msg night-light` hata döner.
    #[default]
    Unsupported,
    /// Yalnızca test örneği: çağrılar loglanır.
    Fake,
    /// Framework renk dönüşümü (`Context::set_color_transforms`).
    Ctm,
}

/// Arka uca giden, `Context` gerektiren işlem ([`Shell::night_light_flush`] sırayla uygular).
#[derive(Debug, Clone, PartialEq)]
enum Op {
    Commit(Vec<(OutputId, ColorMatrix)>),
    Release,
}

/// CTM arka ucu. Durum makinesi `Context`'e erişmeden çağırır; çağrılar sıraya girer ve her
/// makine çağrısından hemen sonra [`Shell::night_light_flush`] ile compositor'a gider.
#[derive(Debug, Default)]
pub(super) struct Backend {
    kind: Kind,
    /// `set` ile belirlenip henüz `commit` edilmemiş matrisler (çıkış başına son değer).
    batch: Vec<(OutputId, ColorMatrix)>,
    ops: Vec<Op>,
}

impl ColorTransform for Backend {
    type Output = OutputId;

    fn supported(&self) -> bool {
        self.kind != Kind::Unsupported
    }

    fn set(&mut self, output: OutputId, matrix: [f32; 9]) {
        let m = ColorMatrix(matrix);
        match self.batch.iter_mut().find(|(o, _)| *o == output) {
            Some(slot) => slot.1 = m,
            None => self.batch.push((output, m)),
        }
    }

    fn commit(&mut self) {
        if !self.batch.is_empty() {
            self.ops.push(Op::Commit(std::mem::take(&mut self.batch)));
        }
    }

    fn release(&mut self) {
        self.batch.clear();
        self.ops.push(Op::Release);
    }
}

fn kind_from_env(instance: Option<&str>, fake: Option<&str>, ctm: bool) -> Kind {
    match (instance, fake) {
        (Some(i), Some("1")) if !i.is_empty() => Kind::Fake,
        _ if ctm => Kind::Ctm,
        _ => Kind::Unsupported,
    }
}

#[derive(Debug)]
pub(super) struct NightLightRuntime {
    machine: NightLight<OutputId>,
    backend: Backend,
    timer: Option<TimerId>,
    /// Son güncellemenin (duvar, monoton) anı: saat sıçraması denetimi.
    stamp: Option<(DateTime<Utc>, Instant)>,
}

impl Default for NightLightRuntime {
    fn default() -> Self {
        Self { machine: NightLight::new(Settings::default()), backend: Backend::default(), timer: None, stamp: None }
    }
}

fn now() -> Now<Local> {
    Now { wall: Local::now(), mono: Instant::now() }
}

/// Config → ayarlar; konum gerekirse saat diliminden (küçük, sınırlı okuma).
fn resolve(config: &crate::services::night_light::NightLightConfig) -> Settings {
    let (settings, warning) = config.settings(|| {
        let found = location::from_system();
        if let Some((zone, (lat, lon))) = &found {
            tracing::debug!(zone, lat, lon, "gece ışığı: konum saat diliminden");
        }
        found.map(|(_, c)| c)
    });
    if let Some(w) = warning {
        tracing::warn!("night_light: {w}");
    }
    settings
}

impl Shell {
    pub(super) fn start_night_light(&mut self, cx: &mut Context<Self>) {
        let instance = std::env::var(crate::ipc::INSTANCE_ENV).ok();
        let fake = std::env::var(FAKE_ENV).ok();
        let kind = kind_from_env(instance.as_deref(), fake.as_deref(), cx.color_transform_supported());
        self.night_light.backend.kind = kind;
        tracing::info!(?kind, "gece ışığı arka ucu");
        let settings = resolve(&self.model.config.night_light);
        self.night_light.machine.set_settings(settings);
        self.night_light_update(cx);
    }

    /// Hedefi yeniden hesaplar, uygular, timer'ı bir sonraki değişime kurar, modeli yeniler.
    pub(super) fn night_light_update(&mut self, cx: &mut Context<Self>) {
        let now = now();
        let rt = &mut self.night_light;
        let wake = rt.machine.update(&now, &mut rt.backend);
        self.night_light_flush(cx);
        self.night_light_after(cx, &now, wake);
    }

    /// Sıradaki arka uç işlemlerini compositor'a gönderir. `Blocked` (başka istemci CTM'in
    /// sahibi) → makine durur ve yöneticimiz bırakılır (bir sonraki elle açma yeniden bağlanır).
    fn night_light_flush(&mut self, cx: &mut Context<Self>) {
        let rt = &mut self.night_light;
        let ops = std::mem::take(&mut rt.backend.ops);
        if rt.backend.kind != Kind::Ctm {
            for op in &ops {
                tracing::info!(?op, "gece ışığı (sahte arka uç)");
            }
            return;
        }
        let mut blocked = false;
        for op in ops {
            match op {
                Op::Commit(transforms) => match cx.set_color_transforms(&transforms) {
                    Ok(()) => {}
                    Err(ColorTransformError::Blocked) => {
                        blocked = true;
                        break;
                    }
                    // Çıkış bu arada kalktı ya da matris geçersiz (model sınırlı; olmamalı).
                    Err(e) => tracing::warn!(?e, "gece ışığı: renk dönüşümü uygulanamadı"),
                },
                Op::Release => cx.color_transform_release(),
            }
        }
        if blocked {
            self.night_light_set_blocked(cx);
        }
    }

    fn night_light_set_blocked(&mut self, cx: &mut Context<Self>) {
        tracing::warn!("gece ışığı: başka bir istemci (ör. hyprsunset) renk dönüşümünü yönetiyor");
        let rt = &mut self.night_light;
        rt.machine.set_blocked(&mut rt.backend);
        // Bırakma Blocked durumunu sıfırlar; makine elle açılana dek yeniden denemez.
        for op in std::mem::take(&mut rt.backend.ops) {
            if op == Op::Release {
                cx.color_transform_release();
            }
        }
    }

    fn night_light_after(&mut self, cx: &mut Context<Self>, now: &Now<Local>, wake: Option<Instant>) {
        let rt = &mut self.night_light;
        rt.stamp = Some((now.wall.to_utc(), now.mono));
        if let Some(t) = rt.timer.take() {
            cx.cancel_timer(t);
        }
        if let Some(at) = wake {
            rt.timer = Some(cx.set_timer(at.saturating_duration_since(now.mono)));
        }
        let view = rt.machine.view(now, rt.backend.supported());
        if self.model.night_light != view {
            self.model.night_light = view;
            self.model_changed(cx);
        }
    }

    pub(super) fn night_light_timer_fired(&mut self, cx: &mut Context<Self>, id: TimerId) -> bool {
        if self.night_light.timer != Some(id) {
            return false;
        }
        self.night_light.timer = None;
        self.night_light_update(cx);
        true
    }

    /// Saat timer'ının her atımında (ek uyanma yok): duvar saati sıçradıysa yeniden hesap.
    pub(super) fn night_light_clock_check(&mut self, cx: &mut Context<Self>) {
        let Some((wall, mono)) = self.night_light.stamp else { return };
        let expected = wall + chrono::TimeDelta::from_std(mono.elapsed()).unwrap_or_default();
        if (Utc::now() - expected).abs() > chrono::TimeDelta::from_std(CLOCK_JUMP).unwrap_or_default() {
            tracing::info!("gece ışığı: saat değişti, yeniden hesaplanıyor");
            self.night_light_update(cx);
        }
    }

    /// Hızlı ayar kartı ya da IPC komutu.
    pub(super) fn night_light_command(&mut self, cx: &mut Context<Self>, cmd: Command) -> Result<(), String> {
        let now = now();
        let rt = &mut self.night_light;
        let wake = rt.machine.command(cmd, &now, &mut rt.backend).map_err(|e| e.to_string())?;
        self.night_light_flush(cx);
        self.night_light_after(cx, &now, wake);
        Ok(())
    }

    /// `msg night-light …`.
    pub(super) fn night_light_ipc(&mut self, cx: &mut Context<Self>, c: NightLightCommand) -> Result<String, String> {
        match c {
            NightLightCommand::Status => {
                let rt = &self.night_light;
                Ok(rt.machine.status_json(&now(), rt.backend.supported()))
            }
            NightLightCommand::Run(cmd) => self.night_light_command(cx, cmd).map(|()| "ok".into()),
        }
    }

    /// Config yeniden yüklendi.
    pub(super) fn sync_night_light_config(&mut self, cx: &mut Context<Self>) {
        let settings = resolve(&self.model.config.night_light);
        if settings != *self.night_light.machine.settings() {
            self.night_light.machine.set_settings(settings);
            self.night_light_update(cx);
        }
    }

    /// Saat dilimi değişti (`timezone_settled`): saat diliminden gelen konum ve yerel takvimdeki
    /// plan yeniden kurulur (ayarlar aynı kalsa da yerel saat sınırları kayar).
    pub(super) fn night_light_timezone_changed(&mut self, cx: &mut Context<Self>) {
        let settings = resolve(&self.model.config.night_light);
        if settings != *self.night_light.machine.settings() {
            self.night_light.machine.set_settings(settings);
        }
        self.night_light_update(cx);
    }

    pub(super) fn night_light_output_added(&mut self, cx: &mut Context<Self>, output: OutputId) {
        let rt = &mut self.night_light;
        rt.machine.output_added(output, &mut rt.backend);
        self.night_light_flush(cx);
    }

    pub(super) fn night_light_output_removed(&mut self, output: OutputId) {
        self.night_light.machine.output_removed(output);
    }

    /// Kapanış (SIGTERM, `msg quit`): etkinse renk dönüşümü hemen bırakılır.
    pub(super) fn night_light_shutdown(&mut self, cx: &mut Context<Self>) {
        let rt = &mut self.night_light;
        rt.machine.shutdown(&mut rt.backend);
        if let Some(t) = rt.timer.take() {
            cx.cancel_timer(t);
        }
        self.night_light_flush(cx);
    }

    /// Compositor: başka bir istemci CTM'i yönetiyor (`App::color_transform_blocked`).
    pub(super) fn night_light_blocked(&mut self, cx: &mut Context<Self>) {
        self.night_light_set_blocked(cx);
        self.night_light_update(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_selection() {
        assert_eq!(kind_from_env(Some("nl-w"), Some("1"), false), Kind::Fake);
        assert_eq!(kind_from_env(Some("nl-w"), Some("1"), true), Kind::Fake, "test örneği gerçek CTM'e dokunmaz");
        assert_eq!(kind_from_env(None, Some("1"), false), Kind::Unsupported, "asıl örnek kancayı yok sayar");
        assert_eq!(kind_from_env(None, Some("1"), true), Kind::Ctm);
        assert_eq!(kind_from_env(Some(""), Some("1"), false), Kind::Unsupported);
        assert_eq!(kind_from_env(Some("nl-w"), Some("yes"), true), Kind::Ctm);
        assert!(!Backend::default().supported());
    }

    /// set/commit/release sıraya girer: çıkış başına son değer, tek commit'te; bırakma bekleyen
    /// set'leri atar.
    #[test]
    fn backend_queues_ops() {
        let (a, b) = (OutputId::for_test(1), OutputId::for_test(2));
        let m = |v: f32| [v, 0.0, 0.0, 0.0, v, 0.0, 0.0, 0.0, v];
        let mut be = Backend { kind: Kind::Ctm, ..Backend::default() };
        be.commit();
        assert!(be.ops.is_empty(), "boş commit gönderilmez");
        be.set(a, m(0.5));
        be.set(b, m(0.5));
        be.set(a, m(0.7));
        be.commit();
        be.set(b, m(0.1));
        be.release();
        assert_eq!(
            be.ops,
            [Op::Commit(vec![(a, ColorMatrix(m(0.7))), (b, ColorMatrix(m(0.5)))]), Op::Release]
        );
        assert!(be.batch.is_empty());
    }
}
