//! Gece ışığı (t113, `docs/design/night-light.md`): akşam ekranın renk sıcaklığını düşürür,
//! sabah geri alır. UI'dan ve compositor'dan bağımsız durum makinesi: çıktısını bir
//! [`ColorTransform`] arka ucuna yazar (Hyprland'de `hyprland-ctm-control-v1`, framework
//! `nuxut-wayland` üzerinden; testlerde çağrı kaydeden sahte arka uç).
//!
//! - Saf çekirdek: [`model`] (Kelvin → RGB, zamanlama planı, geçiş, uyanma anı).
//! - Konum: [`location`] (saat diliminden yaklaşık koordinat).
//! - Ayarlar: [`config`] (`[night_light]`).
//!
//! Kurallar:
//! - **Boşta uyanma yok:** [`NightLight::update`] bir sonraki değişim anını döndürür; çağıran
//!   tek bir timer'ı yalnızca o ana kurar. Uykudan uyanma ve saat sıçramasında çağıran yeniden
//!   `update` çağırır (plan her seferinde "şimdi"den kurulur).
//! - **Kapalı kullanıcıda CTM'e dokunulmaz:** hedef hiç etkin olmadıysa arka uca tek çağrı
//!   yapılmaz. Etkinken kapanınca yalnızca `release`: Hyprland yönetici yok olunca tüm çıkışları
//!   birime döndürür (0.56.2 CTMControl.cpp, framework doğrulaması); birim set yöneticiyi tutup
//!   kullanıcının hyprsunset'ini engellerdi.
//! - **Elle geçersiz kılma** (hızlı ayar, `msg night-light on|off|toggle`): zamanlamanın aynı
//!   duruma yerleştiği bir sonraki sınıra kadar geçerli ("yarına kadar kapalı"); `always`'te ya
//!   da ana anahtar kapalıyken oturum boyunca. Yalnızca bellekte (yeniden başlatmada zamanlamaya
//!   dönülür).
//! - **Başka istemci** (`blocked`): durum gösterilir, kendiliğinden tekrar denenmez; kullanıcı
//!   elle açınca bir kez denenir.
//! - **Hız sınırı:** arka uca iki uygulama arası en az [`MIN_APPLY_INTERVAL`] (kaydırıcı
//!   sürüklenirken ≤ 30 Hz); arada gelen değer bekletilir, son değer uygulanır.

pub mod config;
pub mod location;
pub mod model;

use std::fmt::Debug;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};

pub use config::{Location, LocationSource, NightLightConfig, ScheduleKind, Settings};
use model::{NEUTRAL_KELVIN, Plan, Schedule};

/// Arka uca iki uygulama arası en az bu kadar (≈ 30 Hz).
pub const MIN_APPLY_INTERVAL: Duration = Duration::from_millis(33);

/// Çıkış başına renk dönüşüm matrisi yazan arka uç. Framework CTM yeteneğinin şekli
/// (`docs/design/night-light.md` "Framework talebi"); gerçek uygulama framework'te.
pub trait ColorTransform {
    /// Çıkış kimliği (gerçekte `nuxut::OutputId`).
    type Output: Copy + PartialEq + Debug;
    /// Compositor CTM yöneticisini sunuyor mu. `false` ise hiçbir çağrı yapılmaz.
    fn supported(&self) -> bool;
    /// Çıkışın bir sonraki matrisini belirler (satır öncelikli 3×3; birim = etkisiz). İlk çağrı
    /// yöneticiyi bağlar.
    fn set(&mut self, output: Self::Output, matrix: [f32; 9]);
    /// Belirlenen matrisleri atomik olarak uygular.
    fn commit(&mut self);
    /// Yöneticiyi bırakır (compositor CTM'i sıfırlar).
    fn release(&mut self);
}

/// Kullanıcı komutları (hızlı ayar, IPC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Elle aç (`true`) / kapat.
    Set(bool),
    Toggle,
    /// Gece sıcaklığı (K; [`config::TEMPERATURE`]'a kırpılır). Oturum belleğinde; config
    /// değeri değişince config geçerli olur.
    Temperature(u32),
}

/// Komut uygulanamadı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("gece ışığı bu oturumda desteklenmiyor (compositor renk dönüşümü sunmuyor)")]
    Unsupported,
}

/// Hızlı ayarın alt satırı: şu anki durum ve bir sonraki değişim.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Status {
    /// Kapalı; zamanlanmış açılış yok.
    #[default]
    Off,
    /// Açık; zamanlanmış kapanış yok.
    On,
    /// Açık; `at`'te kapanır (`sun`: gün doğumunda).
    OnUntil { at: DateTime<Utc>, sun: bool },
    /// Kapalı; `at`'te açılır (`sun`: gün batımında).
    OffUntil { at: DateTime<Utc>, sun: bool },
    /// Başka bir uygulama (kullanıcının kendi hyprsunset'i) CTM'i yönetiyor.
    Blocked,
}

/// Modele (hızlı ayar kartı) giden görünüm.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NightLightView {
    /// Arka uç var mı; yoksa kart gizlidir.
    pub supported: bool,
    /// Etki şu an var mı (geçiş dahil).
    pub active: bool,
    /// Gece sıcaklığı (kaydırıcı), K.
    pub temperature: u32,
    /// Şu an uygulanan sıcaklık (kapalıyken nötr), K.
    pub current: u32,
    pub status: Status,
}

/// Bir güncellemenin "şimdi"si: duvar saati (zamanlama) ve monoton saat (timer'lar).
#[derive(Debug, Clone)]
pub struct Now<Tz: TimeZone> {
    pub wall: DateTime<Tz>,
    pub mono: Instant,
}

impl<Tz: TimeZone> Now<Tz> {
    fn utc(&self) -> DateTime<Utc> {
        self.wall.to_utc()
    }

    /// Duvar saati anı → monoton an (geçmişse şimdi).
    fn mono_at(&self, at: DateTime<Utc>) -> Instant {
        self.mono + (at - self.utc()).to_std().unwrap_or(Duration::ZERO)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Override {
    on: bool,
    /// `None`: oturum boyunca.
    until: Option<DateTime<Utc>>,
}

/// Gece ışığı durum makinesi. Arka uç her çağrıda verilir (gerçek arka uç runtime bağlamını
/// ödünç alır).
#[derive(Debug)]
pub struct NightLight<O> {
    settings: Settings,
    /// Kaydırıcı/IPC sıcaklığı (oturum belleği).
    temperature: Option<u32>,
    manual: Option<Override>,
    blocked: bool,
    outputs: Vec<O>,
    /// Son uygulanan sıcaklık; `None` = yönetici bağlı değil (ya da bırakıldı).
    applied: Option<u32>,
    last_apply: Option<Instant>,
    /// [`Self::shutdown`] çağrıldı: kapanış turu bitene kadar gelen hiçbir olay yeniden uygulamaz.
    stopped: bool,
}

impl<O: Copy + PartialEq + Debug> NightLight<O> {
    pub fn new(settings: Settings) -> Self {
        Self { settings, temperature: None, manual: None, blocked: false, outputs: Vec::new(), applied: None, last_apply: None, stopped: false }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Gece sıcaklığı: kaydırıcı değeri, yoksa config.
    pub fn night_temperature(&self) -> u32 {
        self.temperature.unwrap_or(self.settings.temperature)
    }

    /// Config değişti. Sıcaklık değiştiyse kaydırıcı değeri, ana anahtar ya da zamanlama
    /// değiştiyse elle geçersiz kılma düşer. Sonra [`Self::update`] çağrılmalı.
    pub fn set_settings(&mut self, settings: Settings) {
        let old = std::mem::replace(&mut self.settings, settings);
        if old.temperature != settings.temperature {
            self.temperature = None;
        }
        if old.enabled != settings.enabled || old.schedule != settings.schedule {
            self.manual = None;
        }
    }

    fn plan<Tz: TimeZone>(&self, now: &Now<Tz>) -> Plan {
        Plan::new(&self.settings.schedule, self.settings.transition, &now.wall)
    }

    fn scheduled(&self, plan: &Plan, t: DateTime<Utc>) -> f32 {
        if self.settings.enabled { plan.strength(t) } else { 0.0 }
    }

    fn strength(&self, plan: &Plan, t: DateTime<Utc>) -> f32 {
        match self.manual {
            Some(o) if o.until.is_none_or(|u| t < u) => {
                if o.on {
                    1.0
                } else {
                    0.0
                }
            }
            _ => self.scheduled(plan, t),
        }
    }

    /// Hedefi yeniden hesaplar, gerekirse uygular. Dönüş: bir sonraki uyanma (yoksa `None`).
    pub fn update<Tz: TimeZone, B: ColorTransform<Output = O>>(&mut self, now: &Now<Tz>, backend: &mut B) -> Option<Instant> {
        let t = now.utc();
        if self.manual.is_some_and(|o| o.until.is_some_and(|u| u <= t)) {
            self.manual = None;
        }
        if !backend.supported() || self.blocked || self.stopped {
            return None;
        }
        let plan = self.plan(now);
        let strength = self.strength(&plan, t);
        let target = (strength > 0.0).then(|| model::blend(self.night_temperature(), strength));
        let mut wake = None;
        if target != self.applied {
            match self.last_apply.map(|l| l + MIN_APPLY_INTERVAL) {
                Some(ready) if now.mono < ready => wake = Some(ready),
                _ => self.apply(target, now.mono, backend),
            }
        }
        let next = match self.manual {
            Some(o) => o.until,
            None if self.settings.enabled => plan.next_wake(t, self.night_temperature()),
            None => None,
        };
        [wake, next.map(|at| now.mono_at(at))].into_iter().flatten().min()
    }

    fn apply<B: ColorTransform<Output = O>>(&mut self, target: Option<u32>, mono: Instant, backend: &mut B) {
        match target {
            Some(k) => {
                let m = model::matrix(k);
                for o in &self.outputs {
                    backend.set(*o, m);
                }
                if !self.outputs.is_empty() {
                    backend.commit();
                }
                self.applied = Some(k);
            }
            None => {
                if self.applied.take().is_none() {
                    return;
                }
                backend.release();
            }
        }
        self.last_apply = Some(mono);
    }

    /// Yeni çıkış: etkinse matrisi ona da uygular.
    pub fn output_added<B: ColorTransform<Output = O>>(&mut self, output: O, backend: &mut B) {
        if self.stopped {
            return;
        }
        if !self.outputs.contains(&output) {
            self.outputs.push(output);
        }
        if let Some(k) = self.applied
            && backend.supported()
            && !self.blocked
        {
            backend.set(output, model::matrix(k));
            backend.commit();
        }
    }

    pub fn output_removed(&mut self, output: O) {
        self.outputs.retain(|o| *o != output);
    }

    /// Kabuk kapanıyor (SIGTERM, `msg quit`): etkinse yönetici bırakılır. Framework kapanışta da
    /// bırakır ve Hyprland istemci ölünce sıfırlar; bu, kabuk tarafının açık güvencesi. Mandal:
    /// sonrasında `update`, `command` ve `output_added` hiçbir şey uygulamaz (quit turu bitmeden
    /// gelen saat/çıkış/timer olayı CTM'i yeniden bağlamasın; shell-reviewer, 2026-10-13).
    pub fn shutdown<B: ColorTransform<Output = O>>(&mut self, backend: &mut B) {
        self.stopped = true;
        if self.applied.take().is_some() {
            backend.release();
        }
    }

    /// Compositor başka bir istemcinin CTM'i yönettiğini bildirdi: yöneticimiz bırakılır,
    /// kendiliğinden tekrar denenmez.
    pub fn set_blocked<B: ColorTransform<Output = O>>(&mut self, backend: &mut B) {
        self.blocked = true;
        if self.applied.take().is_some() {
            backend.release();
        }
    }

    /// Kullanıcı komutu; ardından [`Self::update`] ile aynı dönüş.
    pub fn command<Tz: TimeZone, B: ColorTransform<Output = O>>(
        &mut self,
        cmd: Command,
        now: &Now<Tz>,
        backend: &mut B,
    ) -> Result<Option<Instant>, CommandError> {
        if self.stopped {
            return Ok(None);
        }
        if !backend.supported() {
            return Err(CommandError::Unsupported);
        }
        let t = now.utc();
        let plan = self.plan(now);
        match cmd {
            Command::Temperature(k) => {
                self.temperature = Some(k.clamp(*config::TEMPERATURE.start(), *config::TEMPERATURE.end()));
            }
            Command::Set(_) | Command::Toggle => {
                let active = self.strength(&plan, t) > 0.0;
                let want = match cmd {
                    Command::Set(on) => on,
                    _ => !active,
                };
                if want {
                    // Elle açma, başka istemci yüzünden durmuş denemeyi bir kez yeniler.
                    self.blocked = false;
                }
                self.set_manual(want, &plan, t);
            }
        }
        Ok(self.update(now, backend))
    }

    /// Elle geçersiz kılma: zamanlama zaten o durumda yerleşmişse kılma yok; yoksa zamanlamanın
    /// o duruma yerleştiği bir sonraki ana kadar.
    fn set_manual(&mut self, want: bool, plan: &Plan, t: DateTime<Utc>) {
        let scheduled = self.scheduled(plan, t);
        if (want && scheduled >= 1.0) || (!want && scheduled <= 0.0) {
            self.manual = None;
            return;
        }
        let bounded = self.settings.enabled && self.settings.schedule != Schedule::Always;
        let until = bounded.then(|| plan.next_edge(t, want).map(|e| plan.settled(e))).flatten();
        self.manual = Some(Override { on: want, until });
    }

    /// Kart ve `msg night-light status` için görünüm.
    pub fn view<Tz: TimeZone>(&self, now: &Now<Tz>, supported: bool) -> NightLightView {
        let t = now.utc();
        let plan = self.plan(now);
        let active = self.strength(&plan, t) > 0.0;
        let status = if self.blocked {
            Status::Blocked
        } else {
            // Bir sonraki gerçek değişim: elle kılma bitince zamanlama aynı durumdan devam eder.
            let after = match self.manual {
                Some(Override { until: None, .. }) => None,
                Some(Override { until: Some(u), .. }) => Some(u),
                None if self.settings.enabled => Some(t),
                None => None,
            };
            let next = after.and_then(|a| plan.next_edge(a, !active));
            match (active, next) {
                (true, Some(e)) => Status::OnUntil { at: e.at, sun: plan.is_sun() },
                (false, Some(e)) => Status::OffUntil { at: e.at, sun: plan.is_sun() },
                (true, None) => Status::On,
                (false, None) => Status::Off,
            }
        };
        NightLightView {
            supported,
            active: active && !self.blocked,
            temperature: self.night_temperature(),
            current: self.applied.unwrap_or(NEUTRAL_KELVIN),
            status,
        }
    }

    /// `msg night-light status`: tek satır JSON.
    pub fn status_json<Tz: TimeZone>(&self, now: &Now<Tz>, supported: bool) -> String {
        let v = self.view(now, supported);
        let (status, next) = match v.status {
            Status::Off => ("off", None),
            Status::On => ("on", None),
            Status::OnUntil { at, .. } => ("on", Some(at)),
            Status::OffUntil { at, .. } => ("off", Some(at)),
            Status::Blocked => ("blocked", None),
        };
        let schedule = match self.settings.schedule {
            Schedule::Always => "always",
            Schedule::Manual { .. } => "manual",
            Schedule::Sun { .. } => "sun",
        };
        serde_json::json!({
            "supported": supported,
            "enabled": self.settings.enabled,
            "schedule": schedule,
            "status": status,
            "active": v.active,
            "temperature": v.temperature,
            "current": v.current,
            "override": self.manual.map(|o| if o.on { "on" } else { "off" }),
            "next_change": next.map(|t| t.to_rfc3339()),
            "location": self.settings.location.map(|l| location_json(&l)),
            "today": self.settings.location.and_then(|l| l.coords).and_then(|(lat, lon)| {
                model::sun_day(now.wall.date_naive(), lat, lon).map(|d| sun_day_json(&d))
            }),
        })
        .to_string()
    }
}

/// `status` JSON'unda konum: `{source, lat, lon}` (bulunamadıysa `lat`/`lon` null).
fn location_json(l: &Location) -> serde_json::Value {
    let source = match l.source {
        LocationSource::Manual => "manual",
        LocationSource::Timezone => "timezone",
        LocationSource::None => "none",
    };
    serde_json::json!({
        "source": source,
        "lat": l.coords.map(|c| c.0),
        "lon": l.coords.map(|c| c.1),
    })
}

/// `status` JSON'unda bugünün güneş olayları: unix saniye ya da null; kutup günü/gecesinde
/// `polar` = `"day"`/`"night"`.
fn sun_day_json(d: &model::SunDay) -> serde_json::Value {
    let (sunrise, sunset, polar) = match *d {
        model::SunDay::Events { sunrise, sunset } => (sunrise, sunset, None),
        model::SunDay::PolarDay => (None, None, Some("day")),
        model::SunDay::PolarNight => (None, None, Some("night")),
    };
    serde_json::json!({
        "sunrise": sunrise.map(|t| t.timestamp()),
        "sunset": sunset.map(|t| t.timestamp()),
        "polar": polar,
    })
}

#[cfg(test)]
mod tests {
    use super::model::tests::{Cet, cet, manual, utc};
    use super::*;

    /// Çağrıları kaydeden sahte arka uç.
    #[derive(Debug, Default)]
    struct Fake {
        unsupported: bool,
        calls: Vec<Call>,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Set(u8, [f32; 9]),
        Commit,
        Release,
    }

    impl ColorTransform for Fake {
        type Output = u8;
        fn supported(&self) -> bool {
            !self.unsupported
        }
        fn set(&mut self, output: u8, matrix: [f32; 9]) {
            self.calls.push(Call::Set(output, matrix));
        }
        fn commit(&mut self) {
            self.calls.push(Call::Commit);
        }
        fn release(&mut self) {
            self.calls.push(Call::Release);
        }
    }

    impl Fake {
        fn take(&mut self) -> Vec<Call> {
            std::mem::take(&mut self.calls)
        }
    }

    fn settings(enabled: bool) -> Settings {
        Settings { enabled, temperature: 4000, schedule: manual("20:00", "07:00"), transition: Duration::ZERO, location: None }
    }

    /// Saat: 10 Ocak 2026 (CET +1), monoton taban sabit.
    struct Clock {
        base: Instant,
        start: DateTime<Cet>,
    }

    impl Clock {
        fn new(h: u32, m: u32) -> Self {
            Self { base: Instant::now(), start: cet(2026, 1, 10, h, m) }
        }
        fn at(&self, offset: Duration) -> Now<Cet> {
            Now { wall: self.start + chrono::TimeDelta::from_std(offset).unwrap(), mono: self.base + offset }
        }
    }

    fn light(enabled: bool) -> (NightLight<u8>, Fake) {
        let mut nl = NightLight::new(settings(enabled));
        let mut fake = Fake::default();
        nl.output_added(1, &mut fake);
        nl.output_added(2, &mut fake);
        (nl, fake)
    }

    #[test]
    fn disabled_never_touches_the_backend() {
        let (mut nl, mut fake) = light(false);
        let clock = Clock::new(22, 0);
        assert_eq!(nl.update(&clock.at(Duration::ZERO), &mut fake), None, "kapalı → uyanma yok");
        assert!(fake.calls.is_empty());
        let v = nl.view(&clock.at(Duration::ZERO), true);
        assert_eq!((v.active, v.status, v.current), (false, Status::Off, NEUTRAL_KELVIN));
    }

    #[test]
    fn unsupported_backend_rejects_commands() {
        let (mut nl, _) = light(true);
        let mut fake = Fake { unsupported: true, ..Fake::default() };
        let clock = Clock::new(22, 0);
        assert_eq!(nl.update(&clock.at(Duration::ZERO), &mut fake), None);
        assert_eq!(nl.command(Command::Set(true), &clock.at(Duration::ZERO), &mut fake), Err(CommandError::Unsupported));
        assert!(fake.calls.is_empty());
    }

    #[test]
    fn scheduled_night_applies_once_and_wakes_at_boundary() {
        let (mut nl, mut fake) = light(true);
        let clock = Clock::new(22, 0);
        let wake = nl.update(&clock.at(Duration::ZERO), &mut fake).unwrap();
        let m = model::matrix(4000);
        assert_eq!(fake.take(), [Call::Set(1, m), Call::Set(2, m), Call::Commit], "tek commit");
        // 07:00'ye 9 saat.
        assert_eq!(wake - clock.base, Duration::from_secs(9 * 3600));
        // Aynı hedef: arka uca çağrı yok.
        nl.update(&clock.at(Duration::from_secs(60)), &mut fake);
        assert!(fake.calls.is_empty());
        // Sabah: yalnızca bırak.
        let wake = nl.update(&clock.at(Duration::from_secs(9 * 3600)), &mut fake).unwrap();
        assert_eq!(fake.take(), [Call::Release], "kapatma = yöneticiyi bırak (birim set değil)");
        assert_eq!(wake - clock.base, Duration::from_secs(22 * 3600), "akşam 20:00");
        let v = nl.view(&clock.at(Duration::from_secs(9 * 3600)), true);
        assert_eq!(v.status, Status::OffUntil { at: utc(2026, 1, 11, 19, 0), sun: false });
    }

    #[test]
    fn override_off_lasts_until_morning() {
        let (mut nl, mut fake) = light(true);
        let clock = Clock::new(22, 0);
        nl.update(&clock.at(Duration::ZERO), &mut fake);
        fake.take();
        let later = clock.at(Duration::from_secs(1));
        let wake = nl.command(Command::Toggle, &later, &mut fake).unwrap().unwrap();
        assert_eq!(fake.take().last(), Some(&Call::Release));
        // Kılma gün doğumunda (07:00 = 06:00 UTC) biter: tek uyanma orada.
        assert_eq!(wake - clock.base, Duration::from_secs(9 * 3600));
        let v = nl.view(&later, true);
        assert!(!v.active);
        assert_eq!(v.status, Status::OffUntil { at: utc(2026, 1, 11, 19, 0), sun: false }, "bir sonraki açılış ertesi akşam");
        // Sabah kılma düşer, zamanlama (kapalı) devam eder; arka uca çağrı yok.
        nl.update(&clock.at(Duration::from_secs(9 * 3600)), &mut fake);
        assert!(fake.calls.is_empty());
        assert_eq!(nl.manual, None);
        // Ertesi akşam yeniden açılır.
        nl.update(&clock.at(Duration::from_secs(22 * 3600)), &mut fake);
        assert_eq!(fake.take().len(), 3);
    }

    #[test]
    fn override_on_during_the_day_hands_over_at_sunset() {
        let (mut nl, mut fake) = light(true);
        let clock = Clock::new(12, 0);
        assert!(nl.update(&clock.at(Duration::ZERO), &mut fake).is_some());
        assert!(fake.calls.is_empty());
        let wake = nl.command(Command::Set(true), &clock.at(Duration::ZERO), &mut fake).unwrap().unwrap();
        assert_eq!(fake.take().len(), 3);
        assert_eq!(wake - clock.base, Duration::from_secs(8 * 3600), "kılma 20:00'de biter");
        // Durum bir sonraki gerçek değişimi söyler: sabah 07:00.
        let v = nl.view(&clock.at(Duration::ZERO), true);
        assert_eq!(v.status, Status::OnUntil { at: utc(2026, 1, 11, 6, 0), sun: false });
        // 20:00: zamanlama da açık → kesinti yok, çağrı yok.
        nl.update(&clock.at(Duration::from_secs(8 * 3600)), &mut fake);
        assert!(fake.calls.is_empty());
        // Zaten açıkken "aç" kılma yaratmaz.
        nl.command(Command::Set(true), &clock.at(Duration::from_secs(9 * 3600)), &mut fake).unwrap();
        assert_eq!(nl.manual, None);
    }

    #[test]
    fn always_and_master_off_override_for_the_session() {
        let (mut nl, mut fake) = light(false);
        let clock = Clock::new(12, 0);
        let now = clock.at(Duration::ZERO);
        assert_eq!(nl.command(Command::Set(true), &now, &mut fake).unwrap(), None, "oturum boyunca, uyanma yok");
        assert!(nl.view(&now, true).active);
        assert_eq!(nl.view(&now, true).status, Status::On);
        nl.set_settings(Settings { enabled: true, schedule: Schedule::Always, ..settings(true) });
        assert_eq!(nl.manual, None, "ana anahtar değişti → kılma düşer");
        nl.update(&now, &mut fake);
        nl.command(Command::Set(false), &now, &mut fake).unwrap();
        assert_eq!(nl.manual, Some(Override { on: false, until: None }));
        assert_eq!(nl.view(&now, true).status, Status::Off);
    }

    #[test]
    fn slider_is_coalesced_to_30hz() {
        let (mut nl, mut fake) = light(true);
        let clock = Clock::new(22, 0);
        nl.update(&clock.at(Duration::ZERO), &mut fake);
        fake.take();
        let ms = |n| Duration::from_millis(n);
        // 33 ms dolmadan: bekletilir, uyanma hız sınırının sonunda.
        let wake = nl.command(Command::Temperature(3000), &clock.at(ms(10)), &mut fake).unwrap().unwrap();
        assert!(fake.calls.is_empty());
        assert_eq!(wake - clock.base, MIN_APPLY_INTERVAL);
        nl.command(Command::Temperature(2500), &clock.at(ms(20)), &mut fake).unwrap();
        // Timer: son değer uygulanır.
        nl.update(&clock.at(MIN_APPLY_INTERVAL), &mut fake);
        assert_eq!(fake.take()[0], Call::Set(1, model::matrix(2500)));
        assert_eq!(nl.view(&clock.at(MIN_APPLY_INTERVAL), true).temperature, 2500);
        // Sınır dışı değer kırpılır.
        nl.command(Command::Temperature(99), &clock.at(ms(100)), &mut fake).unwrap();
        assert_eq!(nl.night_temperature(), 1500);
        // Config sıcaklığı değişince kaydırıcı değeri düşer.
        nl.set_settings(Settings { temperature: 3500, ..settings(true) });
        assert_eq!(nl.night_temperature(), 3500);
    }

    #[test]
    fn blocked_stops_until_user_turns_on() {
        let (mut nl, mut fake) = light(true);
        let clock = Clock::new(22, 0);
        nl.update(&clock.at(Duration::ZERO), &mut fake);
        fake.take();
        nl.set_blocked(&mut fake);
        assert_eq!(fake.take(), [Call::Release]);
        assert_eq!(nl.update(&clock.at(Duration::from_secs(60)), &mut fake), None, "tekrar deneme yok");
        assert!(fake.calls.is_empty());
        assert_eq!(nl.view(&clock.at(Duration::ZERO), true).status, Status::Blocked);
        // Elle açma bir kez dener.
        nl.command(Command::Set(true), &clock.at(Duration::from_secs(120)), &mut fake).unwrap();
        assert_eq!(fake.take().len(), 3);
    }

    /// Normal kapanış (SIGTERM / msg quit) etkin gece ışığını bırakır; kapalıyken çağrı yok.
    #[test]
    fn shutdown_releases_only_when_active() {
        let (mut nl, mut fake) = light(true);
        let clock = Clock::new(22, 0);
        nl.update(&clock.at(Duration::ZERO), &mut fake);
        fake.take();
        nl.shutdown(&mut fake);
        assert_eq!(fake.take(), [Call::Release]);
        nl.shutdown(&mut fake);
        assert!(fake.calls.is_empty(), "ikinci kapanış çağrı üretmez");
        // Mandal: kapanış turunda gelen olaylar yeniden uygulamaz.
        assert_eq!(nl.update(&clock.at(Duration::from_secs(60)), &mut fake), None);
        nl.output_added(3, &mut fake);
        assert_eq!(nl.command(Command::Set(true), &clock.at(Duration::from_secs(120)), &mut fake), Ok(None));
        assert_eq!(nl.command(Command::Temperature(3000), &clock.at(Duration::from_secs(180)), &mut fake), Ok(None));
        assert!(fake.calls.is_empty(), "kapanıştan sonra CTM'e dokunulmaz");
        let (mut nl, mut fake) = light(false);
        nl.update(&clock.at(Duration::ZERO), &mut fake);
        nl.shutdown(&mut fake);
        assert!(fake.calls.is_empty(), "hiç etkin olmadıysa CTM'e dokunulmaz");
    }

    #[test]
    fn new_output_gets_the_current_matrix() {
        let (mut nl, mut fake) = light(true);
        let clock = Clock::new(22, 0);
        nl.update(&clock.at(Duration::ZERO), &mut fake);
        fake.take();
        nl.output_added(3, &mut fake);
        assert_eq!(fake.take(), [Call::Set(3, model::matrix(4000)), Call::Commit]);
        nl.output_removed(3);
        // Kapalıyken yeni çıkış: çağrı yok.
        let (mut nl, mut fake) = light(false);
        nl.update(&clock.at(Duration::ZERO), &mut fake);
        nl.output_added(3, &mut fake);
        assert!(fake.calls.is_empty());
    }

    #[test]
    fn wake_from_sleep_recomputes() {
        // Uykudan uyanma: çağıran `update`'i yeni "şimdi" ile çağırır; uyuyan timer'a güvenilmez.
        let (mut nl, mut fake) = light(true);
        let clock = Clock::new(19, 0);
        nl.update(&clock.at(Duration::ZERO), &mut fake);
        assert!(fake.calls.is_empty());
        // Monoton saat uykuda durdu (1 sn), duvar saati 5 saat ilerledi.
        let woke = Now { wall: clock.start + chrono::TimeDelta::hours(5), mono: clock.base + Duration::from_secs(1) };
        let wake = nl.update(&woke, &mut fake).unwrap();
        assert_eq!(fake.take().len(), 3, "gece: uygulanır");
        assert_eq!(wake - woke.mono, Duration::from_secs(7 * 3600), "00:00 → 07:00");
    }

    #[test]
    fn status_json_is_one_line() {
        let (mut nl, mut fake) = light(true);
        let clock = Clock::new(22, 0);
        nl.update(&clock.at(Duration::ZERO), &mut fake);
        let j: serde_json::Value = serde_json::from_str(&nl.status_json(&clock.at(Duration::ZERO), true)).unwrap();
        assert_eq!(j["status"], "on");
        assert_eq!(j["current"], 4000);
        assert_eq!(j["schedule"], "manual");
        assert_eq!(j["next_change"], "2026-01-11T06:00:00+00:00");
        assert!(j["override"].is_null());
        assert!(j["location"].is_null() && j["today"].is_null(), "sun dışı zamanlamada konum yok");
    }

    /// Ayarlar'ın konum satırı: kaynak, koordinat ve bugünün güneş olayları (unix sn).
    #[test]
    fn status_json_location_and_today() {
        let clock = Clock::new(12, 0);
        let status = |location: Location, lat: f64| {
            let schedule = Schedule::Sun { lat, lon: 29.0 };
            let mut nl = NightLight::<u8>::new(Settings { schedule, location: Some(location), ..settings(true) });
            nl.outputs = vec![1];
            let j: serde_json::Value = serde_json::from_str(&nl.status_json(&clock.at(Duration::ZERO), true)).unwrap();
            j
        };
        let ist = Location { source: LocationSource::Timezone, coords: Some((41.0, 29.0)) };
        let j = status(ist, 41.0);
        assert_eq!(j["location"], serde_json::json!({"source": "timezone", "lat": 41.0, "lon": 29.0}));
        let (rise, set) = (j["today"]["sunrise"].as_i64().unwrap(), j["today"]["sunset"].as_i64().unwrap());
        assert!(rise < set && set - rise > 8 * 3600, "{j}");
        assert!(j["today"]["polar"].is_null());
        // Ocak, 78° K: kutup gecesi.
        let north = Location { source: LocationSource::Manual, coords: Some((78.0, 29.0)) };
        let j = status(north, 78.0);
        assert_eq!(j["location"]["source"], "manual");
        assert_eq!(j["today"], serde_json::json!({"sunrise": null, "sunset": null, "polar": "night"}));
        // Konum bulunamadı: kaynak none, bugün yok.
        let none = Location { source: LocationSource::None, coords: None };
        let j = status(none, 41.0);
        assert_eq!(j["location"], serde_json::json!({"source": "none", "lat": null, "lon": null}));
        assert!(j["today"].is_null());
    }
}
