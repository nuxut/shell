//! Boşta kalma durum makinesi (saf; Wayland, D-Bus, saat yok — zaman çağırandan gelir).
//!
//! Girdiler: compositor'ın aşama başına `idled`/`resumed` bildirimi ([`IdleMachine::idle`]),
//! shell düzeyindeki engeller ([`Inhibitor`]), oturum kilidi (`LockedHint`), AC durumu, logind
//! `PrepareForSleep` ve ayarlar. Çıktı: uygulanacak [`Effect`] listesi. Her girdiden sonra
//! istenen durum (karartma, ekranlar) uygulanmış durumla karşılaştırılır; yalnızca fark üretilir,
//! aynı girdi iki kez gelirse ikinci kez etki çıkmaz. Kilit ve uyku boşta dönemi başına en çok
//! bir kez istenir. Tasarım: `docs/design/idle.md`.

use std::time::{Duration, Instant};

use serde::Serialize;

/// Boşta aşaması. Her aşamanın kendi sayacı vardır (son girdiden itibaren).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Dim,
    ScreenOff,
    Lock,
    Suspend,
}

impl Stage {
    pub const ALL: [Stage; 4] = [Stage::Dim, Stage::ScreenOff, Stage::Lock, Stage::Suspend];

    fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Stage::Dim => "dim",
            Stage::ScreenOff => "screen_off",
            Stage::Lock => "lock",
            Stage::Suspend => "suspend",
        }
    }
}

/// Shell düzeyinde boşta eylemlerini durduran kaynak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inhibitor {
    /// Kahve modu (`msg idle inhibit`).
    Manual,
    /// Bir MPRIS oynatıcısı çalıyor (`media_inhibits` açıkken etkili).
    Media,
    /// logind'de `idle` blok inhibitörü var.
    Logind,
}

impl Inhibitor {
    pub const ALL: [Inhibitor; 3] = [Inhibitor::Manual, Inhibitor::Media, Inhibitor::Logind];

    fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Inhibitor::Manual => "manual",
            Inhibitor::Media => "media",
            Inhibitor::Logind => "logind",
        }
    }
}

/// Makinenin ayarları (`[idle]`'dan, [`super::IdleConfig::settings`]). `None` = aşama kapalı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleSettings {
    /// Ana anahtar: kapalıyken hiçbir aşama izlenmez, uyku öncesi kilit de yok.
    pub enabled: bool,
    pub dim: Option<Duration>,
    pub screen_off: Option<Duration>,
    /// Boşta kilit (`lock_after`).
    pub lock: bool,
    pub lock_after: Option<Duration>,
    /// Uykudan önce kilitle (gecikme inhibitörüyle kilit kurulana dek uykuyu beklet).
    pub lock_before_sleep: bool,
    pub suspend: Option<Duration>,
    pub suspend_on_ac: bool,
    pub media_inhibits: bool,
}

impl Default for IdleSettings {
    fn default() -> Self {
        super::IdleConfig::default().settings()
    }
}

impl IdleSettings {
    /// Compositor'da izlenecek aşamalar ve süreleri.
    pub fn stages(&self) -> Vec<(Stage, Duration)> {
        if !self.enabled {
            return Vec::new();
        }
        let lock = if self.lock { self.lock_after } else { None };
        [(Stage::Dim, self.dim), (Stage::ScreenOff, self.screen_off), (Stage::Lock, lock), (Stage::Suspend, self.suspend)]
            .into_iter()
            .filter_map(|(s, d)| d.map(|d| (s, d)))
            .collect()
    }

    /// Süre aşamada (`None` = kapalı).
    pub fn stage(&self, stage: Stage) -> Option<Duration> {
        self.stages().into_iter().find(|(s, _)| *s == stage).map(|(_, d)| d)
    }

    /// Uyku gecikme inhibitörü gerekli mi: yalnızca uykudan önce kilitlenecekse.
    fn wants_sleep_delay(&self) -> bool {
        self.enabled && self.lock_before_sleep
    }
}

/// Uygulanacak yan etki.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Compositor bildirimlerini bu aşamalarla (yeniden) kur: öncekiler bırakılır, sayaçlar
    /// şimdiden başlar. Boş liste = hiçbir şey izlenmez.
    Watch(Vec<(Stage, Duration)>),
    /// Karartma örtüsü aç/kapat.
    Dim(bool),
    /// Ekranlar açık (`true`) / kapalı (DPMS).
    Screens(bool),
    /// Oturumu kilitle (logind `Session.Lock`).
    Lock,
    /// Makineyi uyut (logind `Suspend`).
    Suspend,
    /// logind uyku gecikme inhibitörünü tut (`true`) / bırak.
    SleepDelay(bool),
    /// Oturumun logind `IdleHint`'i (`SetIdleHint`): etkin bir aşama boştayken `true`.
    IdleHint(bool),
}

/// Uyku süreci.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sleep {
    Awake,
    /// `PrepareForSleep(true)` geldi, kilidin kurulması bekleniyor; inhibitör en geç `deadline`'da bırakılır.
    Preparing { deadline: Instant },
    /// İnhibitör bırakıldı, `PrepareForSleep(false)` bekleniyor.
    Asleep,
}

/// `InhibitDelayMaxUSec` okunamazsa (logind varsayılanı).
pub const DEFAULT_DELAY_MAX: Duration = Duration::from_secs(5);
/// Kilidi bekleme üst sınırı (logind daha uzun izin verse de).
pub const MAX_SLEEP_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct IdleMachine {
    settings: IdleSettings,
    /// Compositor'a göre boşta olan aşamalar.
    idle: [bool; 4],
    inhibitors: [bool; 3],
    dimmed: bool,
    screens_off: bool,
    lock_sent: bool,
    suspend_sent: bool,
    locked: bool,
    on_ac: bool,
    /// Gönderilen `IdleHint`.
    hinted: bool,
    sleep: Sleep,
    /// Bu uyku için kilit istendi: uyanınca hâlâ kilitli değilse yeniden istenir.
    sleep_lock: bool,
    /// İstenen gecikme inhibitörü durumu (servis bunu gerçekleştirir).
    delay_held: bool,
    delay_max: Duration,
}

/// `msg idle status` için anlık görüntü.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IdleStatus {
    pub enabled: bool,
    pub inhibited: bool,
    pub inhibitors: Vec<&'static str>,
    pub dimmed: bool,
    pub screens_off: bool,
    pub locked: bool,
    pub lock: bool,
    pub lock_before_sleep: bool,
    pub sleep_delay: bool,
    /// logind `IdleHint` olarak bildirilen.
    pub idle_hint: bool,
    pub on_ac: bool,
    /// Aşama → saniye (`null` = kapalı).
    pub stages: std::collections::BTreeMap<&'static str, Option<u64>>,
}

impl IdleMachine {
    pub fn new(settings: IdleSettings) -> Self {
        Self {
            settings,
            idle: [false; 4],
            inhibitors: [false; 3],
            dimmed: false,
            screens_off: false,
            lock_sent: false,
            suspend_sent: false,
            locked: false,
            on_ac: true,
            sleep: Sleep::Awake,
            sleep_lock: false,
            delay_held: false,
            delay_max: DEFAULT_DELAY_MAX,
            hinted: false,
        }
    }

    /// Başlangıç etkileri: izleyiciler ve (kilit açıksa) gecikme inhibitörü.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut out = vec![Effect::Watch(self.settings.stages())];
        self.sync_delay(&mut out);
        out
    }

    pub fn settings(&self) -> &IdleSettings {
        &self.settings
    }

    /// Engellerden biri etkin mi.
    pub fn inhibited(&self) -> bool {
        Inhibitor::ALL.into_iter().any(|i| self.inhibitor_active(i))
    }

    fn inhibitor_active(&self, i: Inhibitor) -> bool {
        self.inhibitors[i.index()] && (i != Inhibitor::Media || self.settings.media_inhibits)
    }

    pub fn on_ac(&self) -> bool {
        self.on_ac
    }

    pub fn manual_inhibit(&self) -> bool {
        self.inhibitors[Inhibitor::Manual.index()]
    }

    /// Ayarlar değişti. İzlenen aşamalar ya da engel durumu değiştiyse sayaçlar yeniden kurulur.
    pub fn set_settings(&mut self, settings: IdleSettings) -> Vec<Effect> {
        let mut out = Vec::new();
        if settings == self.settings {
            return out;
        }
        let was_inhibited = self.inhibited();
        let rearm = settings.stages() != self.settings.stages();
        self.settings = settings;
        if rearm || (was_inhibited && !self.inhibited()) {
            self.rearm(&mut out);
        }
        self.reconcile(&mut out);
        self.sync_delay(&mut out);
        out
    }

    /// Compositor bildirimi: `stage` boşta (`true`) ya da girdi geldi (`false`).
    pub fn idle(&mut self, stage: Stage, idle: bool) -> Vec<Effect> {
        let mut out = Vec::new();
        self.idle[stage.index()] = idle;
        if !idle {
            match stage {
                Stage::Lock => self.lock_sent = false,
                Stage::Suspend => self.suspend_sent = false,
                Stage::Dim | Stage::ScreenOff => {}
            }
        }
        self.reconcile(&mut out);
        out
    }

    /// Engel başladı/bitti. Bitince sayaçlar o andan yeniden başlar.
    pub fn set_inhibitor(&mut self, inhibitor: Inhibitor, on: bool) -> Vec<Effect> {
        let mut out = Vec::new();
        let was = self.inhibited();
        self.inhibitors[inhibitor.index()] = on;
        if was && !self.inhibited() {
            self.rearm(&mut out);
        }
        self.reconcile(&mut out);
        out
    }

    /// Oturum kilit durumu (`LockedHint`).
    pub fn set_locked(&mut self, locked: bool) -> Vec<Effect> {
        let mut out = Vec::new();
        self.locked = locked;
        if locked {
            // Uyku kilidi kuruldu: sonra açılması kimlik doğrulamadır, uyanışta yeniden istenmez.
            self.sleep_lock = false;
        }
        if locked && matches!(self.sleep, Sleep::Preparing { .. }) {
            self.release_for_sleep(&mut out);
        }
        self.reconcile(&mut out);
        out
    }

    pub fn set_on_ac(&mut self, on_ac: bool) -> Vec<Effect> {
        let mut out = Vec::new();
        self.on_ac = on_ac;
        self.reconcile(&mut out);
        out
    }

    /// logind `InhibitDelayMaxUSec`; 0 → varsayılan, üstü [`MAX_SLEEP_DELAY`]'e kırpılır.
    pub fn set_delay_max(&mut self, max: Duration) {
        self.delay_max = if max.is_zero() { DEFAULT_DELAY_MAX } else { max.min(MAX_SLEEP_DELAY) };
    }

    /// logind `PrepareForSleep`.
    pub fn prepare_for_sleep(&mut self, start: bool, now: Instant) -> Vec<Effect> {
        let mut out = Vec::new();
        if start {
            if self.sleep != Sleep::Awake {
                return out;
            }
            let lock = self.settings.wants_sleep_delay() && !self.locked;
            if lock {
                self.lock_sent = true;
                self.sleep_lock = true;
                out.push(Effect::Lock);
            }
            if self.delay_held && lock {
                self.sleep = Sleep::Preparing { deadline: now + self.delay_max };
            } else {
                if lock {
                    // logind inhibitörü vermedi (yok ya da düştü): uyku beklemez, kilit yarışır.
                    tracing::warn!("uykudan önce kilit: gecikme inhibitörü yok, uyku kilidi beklemiyor");
                }
                self.release_for_sleep(&mut out);
            }
        } else {
            if self.sleep == Sleep::Awake {
                return out;
            }
            self.sleep = Sleep::Awake;
            // Uyanış: kilit uykudan önce kurulamadıysa (zaman aşımı, inhibitör yok, kilitleyici
            // çöktü) masaüstü açık kalmasın. Kilitleyici hâlâ açılıyorsa ikinci istek çift kilit
            // açmaz (`LockLauncher` / nuxut-lock tek örnek).
            if std::mem::take(&mut self.sleep_lock) && !self.locked && self.settings.wants_sleep_delay() {
                tracing::warn!("uyanış: uykudan önce istenen kilit kurulmamış, yeniden isteniyor");
                out.push(Effect::Lock);
            }
            self.rearm(&mut out);
            self.reconcile(&mut out);
            self.sync_delay(&mut out);
        }
        out
    }

    /// Kilidi bekleme süresinin sonu (uygulama bu ana timer kurar).
    pub fn deadline(&self) -> Option<Instant> {
        match self.sleep {
            Sleep::Preparing { deadline } => Some(deadline),
            _ => None,
        }
    }

    /// Süre doldu mu: dolduysa inhibitör kilit beklenmeden bırakılır (uyku asla süresiz engellenmez).
    pub fn tick(&mut self, now: Instant) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Sleep::Preparing { deadline } = self.sleep
            && now >= deadline
        {
            tracing::warn!("uykudan önce kilit zamanında kurulmadı; uyku gecikmesi bırakıldı");
            self.release_for_sleep(&mut out);
        }
        out
    }

    pub fn status(&self) -> IdleStatus {
        IdleStatus {
            enabled: self.settings.enabled,
            inhibited: self.inhibited(),
            inhibitors: Inhibitor::ALL.into_iter().filter(|i| self.inhibitor_active(*i)).map(Inhibitor::name).collect(),
            dimmed: self.dimmed,
            screens_off: self.screens_off,
            locked: self.locked,
            lock: self.settings.lock,
            lock_before_sleep: self.settings.lock_before_sleep,
            sleep_delay: self.delay_held,
            idle_hint: self.hinted,
            on_ac: self.on_ac,
            stages: Stage::ALL.into_iter().map(|s| (s.name(), self.settings.stage(s).map(|d| d.as_secs()))).collect(),
        }
    }

    fn release_for_sleep(&mut self, out: &mut Vec<Effect>) {
        self.sleep = Sleep::Asleep;
        if self.delay_held {
            self.delay_held = false;
            out.push(Effect::SleepDelay(false));
        }
    }

    /// Sayaçları sıfırla: bildirimler yeniden kurulur, boşta bayrakları ve tek seferlik eylemler temizlenir.
    fn rearm(&mut self, out: &mut Vec<Effect>) {
        self.idle = [false; 4];
        self.lock_sent = false;
        self.suspend_sent = false;
        out.push(Effect::Watch(self.settings.stages()));
    }

    fn sync_delay(&mut self, out: &mut Vec<Effect>) {
        if self.sleep != Sleep::Awake {
            return;
        }
        let want = self.settings.wants_sleep_delay();
        if want != self.delay_held {
            self.delay_held = want;
            out.push(Effect::SleepDelay(want));
        }
    }

    /// Aşama şu an etkin mi: compositor boşta diyor, aşama açık, engel yok, uyanık.
    fn active(&self, stage: Stage) -> bool {
        self.idle[stage.index()] && self.settings.stage(stage).is_some() && !self.inhibited()
    }

    /// İstenen durumu uygulanmışla eşler. Uyku sürecinde dokunulmaz (uyanınca düzelir).
    fn reconcile(&mut self, out: &mut Vec<Effect>) {
        if self.sleep != Sleep::Awake {
            return;
        }
        let dim = self.active(Stage::Dim);
        if dim != self.dimmed {
            self.dimmed = dim;
            out.push(Effect::Dim(dim));
        }
        let off = self.active(Stage::ScreenOff);
        if off != self.screens_off {
            self.screens_off = off;
            out.push(Effect::Screens(!off));
        }
        if self.active(Stage::Lock) && !self.locked && !self.lock_sent {
            self.lock_sent = true;
            out.push(Effect::Lock);
        }
        if self.active(Stage::Suspend) && (!self.on_ac || self.settings.suspend_on_ac) && !self.suspend_sent {
            self.suspend_sent = true;
            out.push(Effect::Suspend);
        }
        // logind IdleHint: başka bileşenler (logind IdleAction, oturum izleyicileri) oturumun
        // boşta olduğunu buradan bilir; engel varken boşta sayılmaz.
        let hint = Stage::ALL.into_iter().any(|s| self.active(s));
        if hint != self.hinted {
            self.hinted = hint;
            out.push(Effect::IdleHint(hint));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn settings() -> IdleSettings {
        IdleSettings {
            enabled: true,
            dim: Some(5 * MIN),
            screen_off: Some(10 * MIN),
            lock: false,
            lock_after: Some(10 * MIN),
            lock_before_sleep: false,
            suspend: Some(30 * MIN),
            suspend_on_ac: false,
            media_inhibits: true,
        }
    }

    fn locking() -> IdleSettings {
        IdleSettings { lock: true, lock_before_sleep: true, ..settings() }
    }

    fn all_stages() -> Vec<(Stage, Duration)> {
        vec![(Stage::Dim, 5 * MIN), (Stage::ScreenOff, 10 * MIN), (Stage::Lock, 10 * MIN), (Stage::Suspend, 30 * MIN)]
    }

    #[test]
    fn start_watches_enabled_stages_and_lock_is_off_by_default() {
        let mut m = IdleMachine::new(settings());
        let fx = m.start();
        assert_eq!(fx, [Effect::Watch(vec![(Stage::Dim, 5 * MIN), (Stage::ScreenOff, 10 * MIN), (Stage::Suspend, 30 * MIN)])]);
        assert!(!m.status().sleep_delay, "kilit kapalıyken uyku gecikmesi tutulmaz");

        let mut m = IdleMachine::new(locking());
        assert_eq!(m.start(), [Effect::Watch(all_stages()), Effect::SleepDelay(true)]);

        let mut m = IdleMachine::new(IdleSettings { enabled: false, ..locking() });
        assert_eq!(m.start(), [Effect::Watch(vec![])], "ana anahtar kapalı: uyku öncesi kilit de yok");
        assert!(m.prepare_for_sleep(true, Instant::now()).is_empty());
    }

    #[test]
    fn stages_progress_and_input_reverts_everything() {
        let mut m = IdleMachine::new(locking());
        m.set_on_ac(false);
        m.start();
        assert_eq!(m.idle(Stage::Dim, true), [Effect::Dim(true), Effect::IdleHint(true)]);
        assert_eq!(m.idle(Stage::ScreenOff, true), [Effect::Screens(false)]);
        assert_eq!(m.idle(Stage::Lock, true), [Effect::Lock]);
        assert_eq!(m.idle(Stage::Suspend, true), [Effect::Suspend]);
        // Tekrarlanan bildirim yeni etki üretmez.
        assert!(m.idle(Stage::Lock, true).is_empty());
        assert!(m.idle(Stage::Suspend, true).is_empty());
        // Girdi: compositor her bildirime resumed gönderir; IdleHint son aşamayla düşer.
        assert_eq!(m.idle(Stage::Dim, false), [Effect::Dim(false)]);
        assert_eq!(m.idle(Stage::ScreenOff, false), [Effect::Screens(true)]);
        assert!(m.idle(Stage::Lock, false).is_empty());
        assert_eq!(m.idle(Stage::Suspend, false), [Effect::IdleHint(false)]);
        assert!(!m.status().idle_hint);
        // Yeni boşta dönemi: kilit ve uyku yeniden istenir.
        assert_eq!(m.idle(Stage::Lock, true), [Effect::Lock, Effect::IdleHint(true)]);
        assert_eq!(m.idle(Stage::Suspend, true), [Effect::Suspend]);
    }

    #[test]
    fn lock_stage_is_inert_when_lock_disabled_or_already_locked() {
        let mut m = IdleMachine::new(settings());
        m.start();
        assert!(m.idle(Stage::Lock, true).is_empty(), "otomatik kilit kapalı");

        let mut m = IdleMachine::new(locking());
        m.start();
        assert!(m.set_locked(true).is_empty());
        assert_eq!(m.idle(Stage::Lock, true), [Effect::IdleHint(true)], "zaten kilitli: Lock yok");
    }

    #[test]
    fn suspend_only_on_battery_unless_allowed() {
        let mut m = IdleMachine::new(settings());
        m.start();
        assert_eq!(m.idle(Stage::Suspend, true), [Effect::IdleHint(true)], "prizde uyunmaz");
        // Boştayken fiş çekildi: hemen uyu.
        assert_eq!(m.set_on_ac(false), [Effect::Suspend]);
        assert!(m.set_on_ac(false).is_empty(), "bir kez");

        let mut m = IdleMachine::new(IdleSettings { suspend_on_ac: true, ..settings() });
        m.start();
        assert_eq!(m.idle(Stage::Suspend, true), [Effect::Suspend, Effect::IdleHint(true)]);

        let mut m = IdleMachine::new(IdleSettings { suspend: None, ..settings() });
        m.start();
        m.set_on_ac(false);
        assert!(m.idle(Stage::Suspend, true).is_empty(), "uyku kapalı");
    }

    #[test]
    fn inhibitor_reverts_and_rearms_when_released() {
        let mut m = IdleMachine::new(settings());
        m.start();
        m.idle(Stage::Dim, true);
        m.idle(Stage::ScreenOff, true);
        assert_eq!(m.set_inhibitor(Inhibitor::Manual, true), [Effect::Dim(false), Effect::Screens(true), Effect::IdleHint(false)]);
        assert!(m.status().inhibited);
        // Engel sürerken compositor bildirimleri etkisiz.
        assert!(m.idle(Stage::Dim, true).is_empty());
        // İkinci engel, birincisi kalkınca hâlâ engelli: yeniden kurulmaz.
        assert!(m.set_inhibitor(Inhibitor::Logind, true).is_empty());
        assert!(m.set_inhibitor(Inhibitor::Manual, false).is_empty());
        // Son engel kalkınca sayaçlar şimdiden başlar; eski boşta bayrağı karartmaz.
        let fx = m.set_inhibitor(Inhibitor::Logind, false);
        assert_eq!(fx, [Effect::Watch(settings().stages())]);
        assert!(!m.status().dimmed);
    }

    #[test]
    fn media_inhibits_only_when_enabled() {
        let mut m = IdleMachine::new(settings());
        m.start();
        m.set_inhibitor(Inhibitor::Media, true);
        assert!(m.idle(Stage::Dim, true).is_empty());
        assert_eq!(m.status().inhibitors, ["media"]);
        // Ayar kapanınca medya engel değil; sayaçlar yeniden kurulur.
        let fx = m.set_settings(IdleSettings { media_inhibits: false, ..settings() });
        assert_eq!(fx, [Effect::Watch(settings().stages())]);
        assert!(!m.status().inhibited);
        assert_eq!(m.idle(Stage::Dim, true), [Effect::Dim(true), Effect::IdleHint(true)]);
    }

    #[test]
    fn settings_change_rearms_and_reverts() {
        let mut m = IdleMachine::new(settings());
        m.start();
        m.idle(Stage::Dim, true);
        let fx = m.set_settings(IdleSettings { enabled: false, ..settings() });
        assert_eq!(fx, [Effect::Watch(vec![]), Effect::Dim(false), Effect::IdleHint(false)]);
        assert!(m.set_settings(IdleSettings { enabled: false, ..settings() }).is_empty(), "aynı ayar");
        // Kilit açılınca gecikme inhibitörü alınır, aşamalar değişir.
        let fx = m.set_settings(locking());
        assert_eq!(fx, [Effect::Watch(all_stages()), Effect::SleepDelay(true)]);
        let fx = m.set_settings(settings());
        assert_eq!(fx, [Effect::Watch(settings().stages()), Effect::SleepDelay(false)]);
    }

    #[test]
    fn sleep_waits_for_lock_then_releases() {
        let t0 = Instant::now();
        let mut m = IdleMachine::new(locking());
        m.start();
        m.set_delay_max(Duration::from_secs(5));
        assert_eq!(m.prepare_for_sleep(true, t0), [Effect::Lock]);
        assert_eq!(m.deadline(), Some(t0 + Duration::from_secs(5)));
        assert!(m.prepare_for_sleep(true, t0).is_empty(), "çift sinyal");
        assert!(m.tick(t0 + Duration::from_secs(1)).is_empty());
        assert_eq!(m.set_locked(true), [Effect::SleepDelay(false)]);
        assert_eq!(m.deadline(), None);
        // Uyku sürecinde compositor bildirimleri yan etki üretmez.
        assert!(m.idle(Stage::Dim, true).is_empty());
        // Uyanış: sayaçlar yeniden, gecikme yeniden alınır.
        let fx = m.prepare_for_sleep(false, t0 + Duration::from_secs(60));
        assert_eq!(fx, [Effect::Watch(all_stages()), Effect::SleepDelay(true)]);
    }

    #[test]
    fn sleep_delay_released_on_timeout() {
        let t0 = Instant::now();
        let mut m = IdleMachine::new(locking());
        m.start();
        m.set_delay_max(Duration::ZERO);
        assert_eq!(m.prepare_for_sleep(true, t0), [Effect::Lock]);
        assert_eq!(m.deadline(), Some(t0 + DEFAULT_DELAY_MAX));
        assert_eq!(m.tick(t0 + DEFAULT_DELAY_MAX), [Effect::SleepDelay(false)]);
        assert!(m.tick(t0 + 2 * DEFAULT_DELAY_MAX).is_empty());
        m.set_delay_max(Duration::from_secs(3600));
        // Uyanınca uzun süre bile en çok MAX_SLEEP_DELAY.
        m.prepare_for_sleep(false, t0);
        m.prepare_for_sleep(true, t0);
        assert_eq!(m.deadline(), Some(t0 + MAX_SLEEP_DELAY));
    }

    #[test]
    fn lock_before_sleep_is_independent_of_idle_lock() {
        let t0 = Instant::now();
        let mut m = IdleMachine::new(IdleSettings { lock_before_sleep: true, ..settings() });
        assert_eq!(m.start(), [Effect::Watch(settings().stages()), Effect::SleepDelay(true)], "boşta kilit kapalı");
        assert!(m.idle(Stage::Lock, true).iter().all(|e| *e != Effect::Lock), "boşta kilitlemez");
        assert_eq!(m.prepare_for_sleep(true, t0), [Effect::Lock], "uykudan önce kilitler");
        // Kapatılınca inhibitör bırakılır.
        let mut m = IdleMachine::new(locking());
        m.start();
        let fx = m.set_settings(IdleSettings { lock_before_sleep: false, ..locking() });
        assert_eq!(fx, [Effect::SleepDelay(false)]);
        assert!(m.prepare_for_sleep(true, t0).is_empty());
    }

    #[test]
    fn wake_relocks_when_sleep_lock_did_not_come_up() {
        let t0 = Instant::now();
        let mut m = IdleMachine::new(locking());
        m.start();
        assert_eq!(m.prepare_for_sleep(true, t0), [Effect::Lock]);
        assert_eq!(m.tick(t0 + DEFAULT_DELAY_MAX), [Effect::SleepDelay(false)], "zaman aşımı: uyku bekletilmez");
        let fx = m.prepare_for_sleep(false, t0 + 60 * MIN);
        assert_eq!(fx[0], Effect::Lock, "uyanınca masaüstü açık kalmaz: {fx:?}");
        assert!(fx.contains(&Effect::SleepDelay(true)));
        // Kilit uykudan önce kurulduysa uyanışta yeniden istenmez.
        m.prepare_for_sleep(true, t0);
        m.set_locked(true);
        assert!(!m.prepare_for_sleep(false, t0).contains(&Effect::Lock));
        // Kurulmuştu ama uykuda biri açtı (kimlik doğrulama): yeniden kilitlenmez.
        m.set_locked(false);
        m.prepare_for_sleep(true, t0);
        m.set_locked(true);
        m.set_locked(false);
        assert!(!m.prepare_for_sleep(false, t0).contains(&Effect::Lock));
    }

    #[test]
    fn sleep_when_already_locked_or_lock_disabled() {
        let t0 = Instant::now();
        let mut m = IdleMachine::new(locking());
        m.start();
        m.set_locked(true);
        assert_eq!(m.prepare_for_sleep(true, t0), [Effect::SleepDelay(false)], "kilitli: hemen bırak");

        let mut m = IdleMachine::new(settings());
        m.start();
        assert!(m.prepare_for_sleep(true, t0).is_empty(), "kilit kapalı: inhibitör yok, kilit yok");
        assert!(m.prepare_for_sleep(false, t0).iter().all(|e| matches!(e, Effect::Watch(_))));
    }

    #[test]
    fn wake_from_sleep_turns_screens_back_on() {
        let t0 = Instant::now();
        let mut m = IdleMachine::new(settings());
        m.set_on_ac(false);
        m.start();
        m.idle(Stage::Dim, true);
        m.idle(Stage::ScreenOff, true);
        assert_eq!(m.idle(Stage::Suspend, true), [Effect::Suspend]);
        assert!(m.prepare_for_sleep(true, t0).is_empty());
        let fx = m.prepare_for_sleep(false, t0);
        assert_eq!(fx, [Effect::Watch(settings().stages()), Effect::Dim(false), Effect::Screens(true), Effect::IdleHint(false)]);
        let s = m.status();
        assert!(!s.dimmed && !s.screens_off && !s.idle_hint);
    }

    #[test]
    fn status_reports_stages_in_seconds() {
        let m = IdleMachine::new(settings());
        let s = m.status();
        assert_eq!(s.stages["dim"], Some(300));
        assert_eq!(s.stages["lock"], None, "kilit kapalı");
        assert_eq!(s.stages["suspend"], Some(1800));
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"screen_off\":600"), "{json}");
    }

    /// Rastgele girdi dizileri: etkiler tutarlı kalır (karart/ekran çiftleri dengeli, uyku
    /// sürecinde etki yok, gecikme inhibitörü çift alınıp bırakılmaz).
    #[test]
    fn random_sequences_keep_invariants() {
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..200 {
            let mut m = IdleMachine::new(if next(2) == 0 { settings() } else { locking() });
            let mut dim = false;
            let mut on = true;
            let mut delay = false;
            let mut hint = false;
            let t0 = Instant::now();
            let mut fx = m.start();
            for step in 0..60u64 {
                for e in fx.drain(..) {
                    match e {
                        Effect::Dim(d) => {
                            assert_ne!(d, dim, "karartma çift");
                            dim = d;
                        }
                        Effect::Screens(s) => {
                            assert_ne!(s, on, "ekran çift");
                            on = s;
                        }
                        Effect::SleepDelay(d) => {
                            assert_ne!(d, delay, "gecikme çift");
                            delay = d;
                        }
                        Effect::IdleHint(h) => {
                            assert_ne!(h, hint, "IdleHint çift");
                            hint = h;
                        }
                        Effect::Watch(_) | Effect::Lock | Effect::Suspend => {}
                    }
                }
                assert_eq!(m.status().idle_hint, hint, "IdleHint durumu etkilerle eşleşir");
                assert_eq!((m.status().dimmed, m.status().screens_off), (dim, !on));
                let now = t0 + Duration::from_secs(step);
                let stage = Stage::ALL[next(4) as usize];
                fx = match next(9) {
                    0..=2 => m.idle(stage, next(2) == 0),
                    3 => m.set_inhibitor(Inhibitor::ALL[next(3) as usize], next(2) == 0),
                    4 => m.set_locked(next(2) == 0),
                    5 => m.set_on_ac(next(2) == 0),
                    6 => m.prepare_for_sleep(next(2) == 0, now),
                    7 => m.tick(now + Duration::from_secs(next(40))),
                    _ => m.set_settings(if next(2) == 0 { settings() } else { locking() }),
                };
                let s = m.status();
                assert!(!(s.inhibited && (s.dimmed || s.screens_off || s.idle_hint)) || m.sleep != Sleep::Awake);
            }
        }
    }
}
