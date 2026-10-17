//! Gece ışığının saf çekirdeği: Kelvin → RGB, zamanlama aralıkları, geçiş karışımı ve bir
//! sonraki uyanma anı. Ortam, saat ve dosya sistemi yok: "şimdi" ve saat dilimi çağırandan
//! gelir (testler DST'li sahte bir saat dilimiyle çalışır).
//!
//! Model: zamanlama, "gece" aralıklarının (UTC) bir listesine çevrilir ([`Plan`]). Etki gücü
//! (0 = gündüz, 1 = tam gece) her aralığın sınırı etrafında `transition` süresince doğrusal
//! artar/azalır (sınır geçişin ortasıdır); sıcaklık bu güçle **mired** uzayında karıştırılır
//! (algısal olarak eşit adımlar). Plan, yerel takvimde bugünün iki gün öncesinden iki gün
//! sonrasına kadar kurulur; her güncellemede yeniden kurulur (ucuz: en çok on güneş olayı).

use std::time::Duration;

use chrono::{DateTime, Datelike, Days, MappedLocalTime, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone, Utc};

/// Etkisiz sıcaklık: bu değerde (ve üstünde) matris birimdir.
pub const NEUTRAL_KELVIN: u32 = 6500;

/// Birim matris (satır öncelikli 3×3): renk değişmez.
pub const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// Geçiş sırasında iki uyanma arası en çok bu kadar.
const MAX_STEP: Duration = Duration::from_secs(30);
/// Geçiş sırasında iki uyanma arası en az bu kadar (çok kısa geçişte kare yağmuru olmasın).
const MIN_STEP: Duration = Duration::from_secs(1);
/// Geçiş sırasında bir adımın hedeflediği sıcaklık farkı (K).
const STEP_KELVIN: f64 = 50.0;
/// Plan penceresi: bugünden bu kadar gün önce ve sonra.
const WINDOW_DAYS: u64 = 2;
/// Güneş zamanlamasında önümüzdeki günlerde sınır yoksa (kutup günü/gecesi) planın yeniden
/// kurulacağı süre. Tek uyanma; boşta yoklama değildir.
const POLAR_RECHECK: Duration = Duration::from_secs(12 * 3600);

/// Tanner Helland yaklaşımı (hyprsunset ile aynı eğri), 0..=255.
fn helland(kelvin: f64) -> [f64; 3] {
    let t = kelvin / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2) };
    let g = if t <= 66.0 {
        99.470_802_586_1 * t.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
    };
    [r, g, b].map(|c| c.clamp(0.0, 255.0))
}

/// Sıcaklık → kanal kazançları (0..=1). Eğri [`NEUTRAL_KELVIN`]'e göre normalize edilir:
/// 6500 K tam birimdir (Helland eğrisinin 6500'deki hafif yeşil/mavi düşüşü ekrana yansımaz).
pub fn kelvin_to_rgb(kelvin: u32) -> [f32; 3] {
    let c = helland(f64::from(kelvin));
    let n = helland(f64::from(NEUTRAL_KELVIN));
    [0, 1, 2].map(|i| (c[i] / n[i]).clamp(0.0, 1.0) as f32)
}

/// Sıcaklığın renk dönüşüm matrisi (köşegen). Nötr ve üstü birim.
pub fn matrix(kelvin: u32) -> [f32; 9] {
    if kelvin >= NEUTRAL_KELVIN {
        return IDENTITY;
    }
    let [r, g, b] = kelvin_to_rgb(kelvin);
    [r, 0.0, 0.0, 0.0, g, 0.0, 0.0, 0.0, b]
}

fn mired(kelvin: u32) -> f64 {
    1e6 / f64::from(kelvin.max(1))
}

/// Gündüz (nötr) ile gece sıcaklığı arasında `strength` (0..=1) oranında, mired uzayında karışım.
pub fn blend(night_kelvin: u32, strength: f32) -> u32 {
    let s = f64::from(strength.clamp(0.0, 1.0));
    let m = mired(NEUTRAL_KELVIN) + (mired(night_kelvin) - mired(NEUTRAL_KELVIN)) * s;
    (1e6 / m).round() as u32
}

/// Ne zaman etkin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Schedule {
    /// Açıkken sürekli.
    Always,
    /// Yerel saat aralığı; `to <= from` ise gece yarısını aşar. `from == to`: tüm gün.
    Manual { from: NaiveTime, to: NaiveTime },
    /// Gün batımından gün doğumuna (derece; kuzey ve doğu pozitif).
    Sun { lat: f64, lon: f64 },
}

/// Zamanlamanın bir sınırı: gece aralığının başı (`turns_on`) ya da sonu. `at` geçişin ortasıdır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub at: DateTime<Utc>,
    pub turns_on: bool,
}

/// Belirli bir an için kurulmuş zamanlama planı.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    always: bool,
    sun: bool,
    /// Birleştirilmiş, sıralı gece aralıkları `[başlangıç, bitiş)`; pencereye kırpılmış.
    intervals: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    window: (DateTime<Utc>, DateTime<Utc>),
    transition: TimeDelta,
}

/// Yerel saat → UTC. Belirsiz anda (geri alınan saat) erken olan; var olmayan anda (ileri
/// alınan saat) boşluğun hemen sonrası.
fn local_to_utc<Tz: TimeZone>(tz: &Tz, local: NaiveDateTime) -> DateTime<Utc> {
    let mut probe = local;
    // Boşluklar en çok birkaç saat; yarım saat adımlarla ilk var olan ana ilerlenir.
    for _ in 0..8 {
        match tz.from_local_datetime(&probe) {
            MappedLocalTime::Single(t) | MappedLocalTime::Ambiguous(t, _) => return t.with_timezone(&Utc),
            MappedLocalTime::None => probe += TimeDelta::minutes(30),
        }
    }
    // Olmaması gereken saat dilimi verisi: yerel saati UTC say (panik yok).
    local.and_utc()
}

fn midnight<Tz: TimeZone>(tz: &Tz, date: NaiveDate) -> DateTime<Utc> {
    local_to_utc(tz, date.and_time(NaiveTime::MIN))
}

/// Güneşin yaklaşık sapma açısı (derece; Cooper). Yalnızca kutup günü/gecesini ayırmak için.
fn declination(date: NaiveDate) -> f64 {
    let n = f64::from(date.ordinal());
    -23.44 * (std::f64::consts::TAU / 365.0 * (n + 10.0)).cos()
}

/// Bir günün güneş olayları (Ayarlar'daki "bugün gün batımı" satırı).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunDay {
    /// Olaylar; biri yoksa (yalnız doğuş ya da batış olan geçiş günü) `None`.
    Events { sunrise: Option<DateTime<Utc>>, sunset: Option<DateTime<Utc>> },
    /// Kutup günü: güneş batmaz.
    PolarDay,
    /// Kutup gecesi: güneş doğmaz.
    PolarNight,
}

/// `date` (yerel takvim günü) için gün doğumu/batımı; koordinat geçersizse `None`.
pub fn sun_day(date: NaiveDate, lat: f64, lon: f64) -> Option<SunDay> {
    let day = sunrise::SolarDay::new(sunrise::Coordinates::new(lat, lon)?, date);
    let rise = day.event_time(sunrise::SolarEvent::Sunrise);
    let set = day.event_time(sunrise::SolarEvent::Sunset);
    Some(match (rise, set) {
        (None, None) if lat * declination(date) < 0.0 => SunDay::PolarNight,
        (None, None) => SunDay::PolarDay,
        (sunrise, sunset) => SunDay::Events { sunrise, sunset },
    })
}

/// Yerel bir günün gece parçaları (gece yarısından gün doğumuna, gün batımından gece yarısına).
fn sun_segments<Tz: TimeZone>(tz: &Tz, date: NaiveDate, lat: f64, lon: f64) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let start = midnight(tz, date);
    let end = midnight(tz, date + Days::new(1));
    let Some(coords) = sunrise::Coordinates::new(lat, lon) else { return Vec::new() };
    let day = sunrise::SolarDay::new(coords, date);
    let rise = day.event_time(sunrise::SolarEvent::Sunrise);
    let set = day.event_time(sunrise::SolarEvent::Sunset);
    match (rise, set) {
        (Some(r), Some(s)) => vec![(start, r.clamp(start, end)), (s.clamp(start, end), end)],
        (Some(r), None) => vec![(start, r.clamp(start, end))],
        (None, Some(s)) => vec![(s.clamp(start, end), end)],
        // Kutup gecesi: güneş ufkun altında (enlem ile sapma ters işaretli) → tüm gün gece.
        (None, None) if lat * declination(date) < 0.0 => vec![(start, end)],
        // Kutup günü: hiç gece yok.
        (None, None) => Vec::new(),
    }
}

impl Plan {
    /// `now`'ın yerel takvimi etrafında planı kurar.
    pub fn new<Tz: TimeZone>(schedule: &Schedule, transition: Duration, now: &DateTime<Tz>) -> Self {
        let tz = now.timezone();
        let today = now.date_naive();
        let first = today - Days::new(WINDOW_DAYS);
        let last = today + Days::new(WINDOW_DAYS);
        let window = (midnight(&tz, first), midnight(&tz, last + Days::new(1)));
        let transition = TimeDelta::from_std(transition).unwrap_or(TimeDelta::zero());
        let days = first.iter_days().take_while(|d| *d <= last);
        let raw: Vec<(DateTime<Utc>, DateTime<Utc>)> = match *schedule {
            Schedule::Always => vec![window],
            Schedule::Manual { from, to } if from == to => vec![window],
            Schedule::Manual { from, to } => days
                .map(|d| {
                    let end_day = if to > from { d } else { d + Days::new(1) };
                    (local_to_utc(&tz, d.and_time(from)), local_to_utc(&tz, end_day.and_time(to)))
                })
                .collect(),
            Schedule::Sun { lat, lon } => days.flat_map(|d| sun_segments(&tz, d, lat, lon)).collect(),
        };
        let mut intervals = merge(raw);
        for iv in &mut intervals {
            iv.0 = iv.0.clamp(window.0, window.1);
            iv.1 = iv.1.clamp(window.0, window.1);
        }
        intervals.retain(|(s, e)| s < e);
        Self {
            always: *schedule == Schedule::Always,
            sun: matches!(schedule, Schedule::Sun { .. }),
            intervals,
            window,
            transition,
        }
    }

    /// Sınırlar güneşe mi bağlı (durum metni: "gün doğumunda" / saat).
    pub fn is_sun(&self) -> bool {
        self.sun
    }

    fn half(&self) -> TimeDelta {
        self.transition / 2
    }

    /// Gerçek sınırlar (pencere kenarına kırpılmış uçlar sınır değildir), zaman sırasıyla.
    pub fn edges(&self) -> impl Iterator<Item = Edge> + '_ {
        self.intervals.iter().flat_map(move |&(s, e)| {
            let on = (s != self.window.0).then_some(Edge { at: s, turns_on: true });
            let off = (e != self.window.1).then_some(Edge { at: e, turns_on: false });
            on.into_iter().chain(off)
        })
    }

    /// Etki gücü: 0 = gündüz, 1 = tam gece.
    pub fn strength(&self, t: DateTime<Utc>) -> f32 {
        if self.always {
            return 1.0;
        }
        let half = self.half();
        let total = self.transition.num_milliseconds() as f64;
        self.intervals
            .iter()
            .map(|&(s, e)| {
                if total <= 0.0 {
                    return if s <= t && t < e { 1.0 } else { 0.0 };
                }
                let up = (t - (s - half)).num_milliseconds() as f64 / total;
                let down = ((e + half) - t).num_milliseconds() as f64 / total;
                up.min(down).clamp(0.0, 1.0)
            })
            .fold(0.0_f64, f64::max) as f32
    }

    /// Bir sonraki değişim anı: geçişteyse bir sonraki adım (≤ 30 sn ya da ~50 K), değilse
    /// bir sonraki geçişin başı. Değişim yoksa `None` (boşta uyanma yok).
    pub fn next_wake(&self, t: DateTime<Utc>, night_kelvin: u32) -> Option<DateTime<Utc>> {
        if self.always {
            return None;
        }
        let half = self.half();
        if self.transition > TimeDelta::zero()
            && let Some(ramp_end) = self.edges().map(|e| e.at + half).find(|end| *end - self.transition <= t && t < *end)
        {
            return Some(self.ramp_step(t, night_kelvin).map_or(ramp_end, |next| next.min(ramp_end)));
        }
        let next = self.edges().map(|e| e.at - half).find(|start| *start > t);
        match next {
            Some(n) => Some(n),
            None if self.sun => Some(t + TimeDelta::from_std(POLAR_RECHECK).unwrap_or(TimeDelta::hours(12))),
            None => None,
        }
    }

    /// Geçiş içinde bir sonraki adım: sıcaklığın ~[`STEP_KELVIN`] değişeceği süre, sınırlı.
    fn ramp_step(&self, t: DateTime<Utc>, night_kelvin: u32) -> Option<DateTime<Utc>> {
        let span = (mired(night_kelvin) - mired(NEUTRAL_KELVIN)).abs();
        let secs = self.transition.num_milliseconds() as f64 / 1000.0;
        if span <= 0.0 || secs <= 0.0 {
            return None;
        }
        let rate = span / secs; // mired/sn
        let m = mired(blend(night_kelvin, self.strength(t)));
        // dK = 1e6/m² dm → 50 K için dm = 50 m² / 1e6.
        let dt = (STEP_KELVIN * m * m / 1e6 / rate).clamp(MIN_STEP.as_secs_f64(), MAX_STEP.as_secs_f64());
        Some(t + TimeDelta::milliseconds((dt * 1000.0) as i64))
    }

    /// `after`'dan sonra yerleşen ilk sınır (geçişi `after`'dan sonra biten), yönüyle.
    pub fn next_edge(&self, after: DateTime<Utc>, turns_on: bool) -> Option<Edge> {
        let half = self.half();
        self.edges().find(|e| e.turns_on == turns_on && e.at + half > after)
    }

    /// Sınırın geçişinin bittiği an (elle geçersiz kılma burada biter).
    pub fn settled(&self, edge: Edge) -> DateTime<Utc> {
        edge.at + self.half()
    }
}

/// Aralıkları sıralar ve çakışan/bitişik olanları birleştirir.
fn merge(mut raw: Vec<(DateTime<Utc>, DateTime<Utc>)>) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    raw.retain(|(s, e)| s < e);
    raw.sort();
    let mut out: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::with_capacity(raw.len());
    for (s, e) in raw {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// `HH:MM` (00:00–23:59) ayrıştırır.
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.trim().split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 || !h.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::{FixedOffset, Offset};

    /// Orta Avrupa kuralıyla DST'li sahte saat dilimi: kışın +1, Mart'ın son Pazarı 01:00 UTC'den
    /// Ekim'in son Pazarı 01:00 UTC'ye kadar +2.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Cet;

    fn last_sunday(year: i32, month: u32) -> NaiveDate {
        let mut d = NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap() - Days::new(1);
        while d.weekday() != chrono::Weekday::Sun {
            d = d - Days::new(1);
        }
        d
    }

    impl Cet {
        fn hours_at_utc(utc: &NaiveDateTime) -> i32 {
            let y = utc.year();
            let start = last_sunday(y, 3).and_hms_opt(1, 0, 0).unwrap();
            let end = last_sunday(y, 10).and_hms_opt(1, 0, 0).unwrap();
            if *utc >= start && *utc < end { 2 } else { 1 }
        }
    }

    impl TimeZone for Cet {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            Cet
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).unwrap())
        }

        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> MappedLocalTime<FixedOffset> {
            let ok: Vec<i32> = [2, 1].into_iter().filter(|h| Cet::hours_at_utc(&(*local - TimeDelta::hours((*h).into()))) == *h).collect();
            let off = |h: i32| FixedOffset::east_opt(h * 3600).unwrap();
            match ok.as_slice() {
                [h] => MappedLocalTime::Single(off(*h)),
                [a, b] => MappedLocalTime::Ambiguous(off(*a), off(*b)),
                _ => MappedLocalTime::None,
            }
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(12, 0, 0).unwrap())
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            FixedOffset::east_opt(Cet::hours_at_utc(utc) * 3600).unwrap()
        }
    }

    pub(crate) fn cet(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Cet> {
        Cet.with_ymd_and_hms(y, mo, d, h, mi, 0).earliest().unwrap()
    }

    pub(crate) fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    pub(crate) fn manual(from: &str, to: &str) -> Schedule {
        Schedule::Manual { from: parse_hhmm(from).unwrap(), to: parse_hhmm(to).unwrap() }
    }

    const MIN30: Duration = Duration::from_secs(30 * 60);

    #[test]
    fn kelvin_curve_is_neutral_at_6500_and_monotonic() {
        assert_eq!(kelvin_to_rgb(NEUTRAL_KELVIN), [1.0, 1.0, 1.0]);
        assert_eq!(matrix(NEUTRAL_KELVIN), IDENTITY);
        assert_eq!(matrix(7000), IDENTITY);
        let mut prev = [0.0f32; 3];
        for k in (1000..=6500).step_by(100) {
            let c = kelvin_to_rgb(k);
            assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "{k}: {c:?}");
            assert!(c[1] >= prev[1] && c[2] >= prev[2], "yeşil ve mavi ısındıkça azalır: {k}");
            assert!(c[0] >= c[1] && c[1] >= c[2], "sıcak renk: {k} {c:?}");
            prev = c;
        }
        let m = matrix(4000);
        assert!(m[0] == 1.0 && m[4] < 1.0 && m[8] < m[4] && m[1] == 0.0);
    }

    #[test]
    fn blend_walks_in_mireds() {
        assert_eq!(blend(4000, 0.0), NEUTRAL_KELVIN);
        assert_eq!(blend(4000, 1.0), 4000);
        // Mired ortası: 1e6 / ((153.8 + 250) / 2) ≈ 4952 (doğrusal Kelvin ortası 5250 olurdu).
        assert_eq!(blend(4000, 0.5), 4952);
        assert_eq!(blend(4000, 7.0), 4000, "güç kırpılır");
    }

    #[test]
    fn hhmm_parsing() {
        assert_eq!(parse_hhmm("07:05"), NaiveTime::from_hms_opt(7, 5, 0));
        assert_eq!(parse_hhmm("7:05"), NaiveTime::from_hms_opt(7, 5, 0));
        for bad in ["", "24:00", "12:60", "12", "12:5", "-1:00", "1a:00", "12:00:00", "123:00"] {
            assert_eq!(parse_hhmm(bad), None, "{bad}");
        }
    }

    #[test]
    fn manual_schedule_crosses_midnight() {
        let now = cet(2026, 1, 10, 12, 0);
        let p = Plan::new(&manual("20:00", "07:00"), Duration::ZERO, &now);
        // 10 Ocak 20:00 (+1) = 19:00 UTC; 11 Ocak 07:00 = 06:00 UTC.
        assert_eq!(p.strength(utc(2026, 1, 10, 18, 59)), 0.0);
        assert_eq!(p.strength(utc(2026, 1, 10, 19, 0)), 1.0);
        assert_eq!(p.strength(utc(2026, 1, 11, 5, 59)), 1.0);
        assert_eq!(p.strength(utc(2026, 1, 11, 6, 0)), 0.0);
        assert_eq!(p.next_wake(now.to_utc(), 4000), Some(utc(2026, 1, 10, 19, 0)));
        assert_eq!(p.next_wake(utc(2026, 1, 10, 22, 0), 4000), Some(utc(2026, 1, 11, 6, 0)));
        // Gündüz aralığı da çalışır (gece yarısı aşmadan).
        let p = Plan::new(&manual("13:00", "14:00"), Duration::ZERO, &now);
        assert_eq!(p.strength(utc(2026, 1, 10, 12, 30)), 1.0);
        assert_eq!(p.strength(utc(2026, 1, 10, 13, 0)), 0.0);
    }

    #[test]
    fn same_from_and_to_means_all_day() {
        let now = cet(2026, 1, 10, 12, 0);
        let p = Plan::new(&manual("08:00", "08:00"), MIN30, &now);
        assert_eq!(p.strength(now.to_utc()), 1.0);
        assert_eq!(p.edges().count(), 0);
        assert_eq!(p.next_wake(now.to_utc(), 4000), None, "değişim yok → uyanma yok");
    }

    #[test]
    fn always_never_wakes() {
        let now = cet(2026, 1, 10, 12, 0);
        let p = Plan::new(&Schedule::Always, MIN30, &now);
        assert_eq!(p.strength(now.to_utc()), 1.0);
        assert_eq!(p.next_wake(now.to_utc(), 4000), None);
        assert_eq!(p.next_edge(now.to_utc(), false), None);
    }

    #[test]
    fn transition_is_centered_on_boundary() {
        let now = cet(2026, 1, 10, 12, 0);
        let p = Plan::new(&manual("20:00", "07:00"), MIN30, &now);
        // Sınır 19:00 UTC; geçiş 18:45–19:15.
        assert_eq!(p.strength(utc(2026, 1, 10, 18, 45)), 0.0);
        assert!((p.strength(utc(2026, 1, 10, 19, 0)) - 0.5).abs() < 1e-6);
        assert_eq!(p.strength(utc(2026, 1, 10, 19, 15)), 1.0);
        // Geçiş dışında bir sonraki uyanma geçişin başı.
        assert_eq!(p.next_wake(now.to_utc(), 4000), Some(utc(2026, 1, 10, 18, 45)));
        // Geçişte adım ≤ 30 sn, ≥ 1 sn; geçiş sonunu aşmaz.
        let t = utc(2026, 1, 10, 18, 50);
        let w = p.next_wake(t, 4000).unwrap();
        assert!(w > t && w - t <= TimeDelta::seconds(30), "{w}");
        let t = utc(2026, 1, 10, 19, 14) + TimeDelta::seconds(59);
        assert_eq!(p.next_wake(t, 4000), Some(utc(2026, 1, 10, 19, 15)));
        // Bir adımda sıcaklık ≲ 50 K değişir (ya da 30 sn sınırı).
        let mut t = utc(2026, 1, 10, 18, 45);
        let end = utc(2026, 1, 10, 19, 15);
        let mut steps = 0;
        while t < end {
            let next = p.next_wake(t, 1500).unwrap();
            let dk = blend(1500, p.strength(t)).abs_diff(blend(1500, p.strength(next)));
            assert!(dk <= 60 || next - t == TimeDelta::seconds(1), "{t}: {dk} K");
            t = next;
            steps += 1;
        }
        assert!(steps < 200, "{steps}");
        // Geçiş bittikten sonra bir sonraki geçişin başına kadar uyanma yok.
        assert_eq!(p.next_wake(end, 4000), Some(utc(2026, 1, 11, 5, 45)));
    }

    #[test]
    fn short_interval_never_reaches_full_strength() {
        let now = cet(2026, 1, 10, 12, 0);
        let p = Plan::new(&manual("20:00", "20:10"), MIN30, &now);
        let peak = p.strength(utc(2026, 1, 10, 19, 5));
        assert!(peak > 0.0 && peak < 1.0, "{peak}");
    }

    #[test]
    fn dst_day_shortens_the_night_and_skips_the_gap() {
        // 29 Mart 2026: 02:00 → 03:00 (+1 → +2).
        let now = cet(2026, 3, 28, 12, 0);
        let p = Plan::new(&manual("20:00", "07:00"), Duration::ZERO, &now);
        let on = p.next_edge(now.to_utc(), true).unwrap();
        let off = p.next_edge(now.to_utc(), false).unwrap();
        assert_eq!(on.at, utc(2026, 3, 28, 19, 0));
        assert_eq!(off.at, utc(2026, 3, 29, 5, 0), "07:00 yaz saati = 05:00 UTC");
        assert_eq!(off.at - on.at, TimeDelta::hours(10));
        // Var olmayan yerel saat (02:30) boşluğun sonrasına kayar: 03:00 (+2) = 01:00 UTC.
        let p = Plan::new(&manual("02:30", "06:00"), Duration::ZERO, &now);
        let on = p.next_edge(now.to_utc(), true).unwrap();
        assert_eq!(on.at, utc(2026, 3, 29, 1, 0));
        // Geri alınan saatte (25 Ekim 02:30 iki kez) erken olan.
        let now = cet(2026, 10, 24, 12, 0);
        let p = Plan::new(&manual("02:30", "06:00"), Duration::ZERO, &now);
        assert_eq!(p.next_edge(now.to_utc(), true).unwrap().at, utc(2026, 10, 25, 0, 30));
    }

    #[test]
    fn sun_schedule_follows_sunset_and_sunrise() {
        // İstanbul, 10 Ocak: gün batımı ~14:50 UTC, gün doğumu ~05:30 UTC.
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        let now = tz.with_ymd_and_hms(2026, 1, 10, 12, 0, 0).unwrap();
        let p = Plan::new(&Schedule::Sun { lat: 41.0, lon: 29.0 }, Duration::ZERO, &now);
        assert!(p.is_sun());
        let set = p.next_edge(now.to_utc(), true).unwrap().at;
        let rise = p.next_edge(now.to_utc(), false).unwrap().at;
        assert!(set > utc(2026, 1, 10, 14, 30) && set < utc(2026, 1, 10, 15, 10), "{set}");
        assert!(rise > utc(2026, 1, 11, 5, 15) && rise < utc(2026, 1, 11, 5, 45), "{rise}");
        assert_eq!(p.strength(now.to_utc()), 0.0);
        assert_eq!(p.strength(utc(2026, 1, 10, 23, 0)), 1.0, "gece yarısı birleşir");
        assert_eq!(p.strength(utc(2026, 1, 11, 0, 0)), 1.0);
    }

    #[test]
    fn polar_day_and_night() {
        let tz = FixedOffset::east_opt(3600).unwrap();
        // Tromsø (69.6 K): Haziran'da kutup günü, Aralık'ta kutup gecesi.
        let june = tz.with_ymd_and_hms(2026, 6, 21, 12, 0, 0).unwrap();
        let p = Plan::new(&Schedule::Sun { lat: 69.65, lon: 18.96 }, MIN30, &june);
        assert_eq!(p.strength(june.to_utc()), 0.0);
        assert_eq!(p.strength(june.to_utc() + TimeDelta::hours(12)), 0.0);
        let w = p.next_wake(june.to_utc(), 4000).unwrap();
        assert_eq!(w - june.to_utc(), TimeDelta::hours(12), "tek yeniden denetim, yoklama yok");
        let dec = tz.with_ymd_and_hms(2026, 12, 21, 12, 0, 0).unwrap();
        let p = Plan::new(&Schedule::Sun { lat: 69.65, lon: 18.96 }, MIN30, &dec);
        assert_eq!(p.strength(dec.to_utc()), 1.0);
        assert_eq!(p.strength(dec.to_utc() + TimeDelta::hours(12)), 1.0);
        // Güney yarımkürede tersi (Haziran'da kutup gecesi).
        let p = Plan::new(&Schedule::Sun { lat: -75.0, lon: 0.0 }, MIN30, &june);
        assert_eq!(p.strength(june.to_utc()), 1.0);
    }

    #[test]
    fn offset_is_consistent_with_utc_offset() {
        // Test saat diliminin kendisi: yaz/kış kayması doğru yerde.
        assert_eq!(cet(2026, 3, 29, 1, 0).offset().fix().local_minus_utc(), 3600);
        assert_eq!(cet(2026, 3, 29, 4, 0).offset().fix().local_minus_utc(), 7200);
    }
}
