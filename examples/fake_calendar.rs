//! Sahte `org.nuxut.Calendar1` (yalnızca izole test ortamında görsel test için): takvim
//! popout'unu ve "Takvim" kartını gerçek `nuxut-calendard` olmadan etkinlikle çizdirir. Her
//! `GetOccurrences` sorgusunu stderr'e yazar (birleşik aralık sorgusu canlı izlenir).
//!
//! ```sh
//! dbus-run-session -- sh betik.sh             # özel oturum bus'ı
//! cargo run --example fake_calendar &         # o bus'ta adı alır
//! nuxut-shell
//! ```
//!
//! Etkinlikler tarihten belirlenimli: bugün beş etkinlik (biri tüm gün, biri uzun başlıklı),
//! üç günde bir toplantı, yedi günde bir iki günlük etkinlik.
//!
//! Güvenlik: yalnızca `DBUS_SESSION_BUS_ADDRESS` bus'ına bağlanır; bu `dbus-run-session`'ın
//! özel bus'ı olmalıdır (kullanıcının oturumunda gerçek daemon adı tutuyor olabilir).

use std::time::Duration;

use chrono::{Local, NaiveDate, Offset};
use nuxut_calendar_ipc::{BUS_NAME, OBJECT_PATH, OccurrenceEntry};

const DAY: i64 = 86_400;

struct FakeCalendar {
    offset: i64,
}

fn timed(day_start: i64, h: i64, m: i64, mins: i64, summary: &str, color: &str) -> OccurrenceEntry {
    let start = day_start + h * 3600 + m * 60;
    OccurrenceEntry {
        start,
        end: start + mins * 60,
        summary: summary.into(),
        color: color.into(),
        ..Default::default()
    }
}

fn all_day(
    day: NaiveDate,
    days: u64,
    day_start: i64,
    summary: &str,
    color: &str,
) -> OccurrenceEntry {
    let end = day + chrono::Days::new(days);
    OccurrenceEntry {
        start: day_start,
        end: day_start + days as i64 * DAY,
        all_day: true,
        start_date: day.format("%Y-%m-%d").to_string(),
        end_date: end.format("%Y-%m-%d").to_string(),
        summary: summary.into(),
        color: color.into(),
        ..Default::default()
    }
}

#[zbus::interface(name = "org.nuxut.Calendar1")]
impl FakeCalendar {
    fn get_occurrences(&self, from: i64, to: i64) -> Vec<OccurrenceEntry> {
        let today = Local::now().date_naive();
        let days = ((to - from) / DAY).clamp(0, 800);
        eprintln!(
            "sorgu: {} .. {} ({days} gün)",
            chrono::DateTime::from_timestamp(from + self.offset, 0)
                .map(|d| d.date_naive().to_string())
                .unwrap_or_default(),
            chrono::DateTime::from_timestamp(to + self.offset, 0)
                .map(|d| d.date_naive().to_string())
                .unwrap_or_default(),
        );
        let mut out = Vec::new();
        for i in 0..days {
            let start = from + i * DAY;
            let Some(date) =
                chrono::DateTime::from_timestamp(start + self.offset, 0).map(|d| d.date_naive())
            else {
                continue;
            };
            let n = (start + self.offset).div_euclid(DAY);
            if date == today {
                out.push(all_day(date, 1, start, "Proje teslimi", "#e11d48")); // renk-verisi: sahte daemon'un takvim rengi
                out.push(timed(start, 9, 30, 30, "Sabah toplantısı", "#2563eb")); // renk-verisi: sahte daemon'un takvim rengi
                let long = "Öğle yemeği — Kadıköy'deki uzun adlı lokantada ekip buluşması";
                out.push(timed(start, 12, 0, 60, long, "#16a34a")); // renk-verisi: sahte daemon'un takvim rengi
                out.push(timed(start, 15, 0, 45, "Kod incelemesi", "#9333ea")); // renk-verisi: sahte daemon'un takvim rengi
                out.push(timed(start, 18, 30, 60, "Spor", "#ea580c")); // renk-verisi: sahte daemon'un takvim rengi
                continue;
            }
            if n % 3 == 0 {
                out.push(timed(start, 10, 0, 60, "Ekip toplantısı", "#2563eb")); // renk-verisi: sahte daemon'un takvim rengi
            }
            if n % 7 == 0 {
                out.push(all_day(date, 2, start, "Konferans", "#0891b2")); // renk-verisi: sahte daemon'un takvim rengi
            }
        }
        // `FAKE_CALENDAR_SOON=<dk>`: şimdiden <dk> dakika sonra başlayan 45 dakikalık etkinlik
        // (bar'daki sıradaki etkinlik göstergesi için).
        if let Some(mins) = std::env::var("FAKE_CALENDAR_SOON").ok().and_then(|v| v.parse::<i64>().ok()) {
            let start = Local::now().timestamp() + mins * 60;
            if start < to && start + 45 * 60 > from {
                out.push(OccurrenceEntry {
                    start,
                    end: start + 45 * 60,
                    summary: "Ekip toplantısı".into(),
                    color: "#2563eb".into(), // renk-verisi: sahte daemon'un takvim rengi
                    ..Default::default()
                });
            }
        }
        out.sort_by_key(|e| e.start);
        out
    }

    #[zbus(signal)]
    async fn occurrences_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        from: i64,
        to: i64,
    ) -> zbus::Result<()>;
}

fn main() -> zbus::Result<()> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .map_err(|_| zbus::Error::Address("DBUS_SESSION_BUS_ADDRESS yok".into()))?;
    let offset = i64::from(Local::now().offset().fix().local_minus_utc());
    let _conn = zbus::blocking::connection::Builder::address(address.as_str())?
        .serve_at(OBJECT_PATH, FakeCalendar { offset })?
        .name(BUS_NAME)?
        .build()?;
    eprintln!("sahte takvim hazır");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
