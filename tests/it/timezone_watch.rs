//! Saat dilimi dinleyicisi: sahte `org.freedesktop.timedate1` ile özel bus'ta uçtan uca.
//! Kullanıcının sistem bus'ına ve saat dilimine dokunulmaz (gerçek timedated'e `SetTimezone`
//! çağrılmaz; sahte servis yalnızca `PropertiesChanged` yayınlar).
//!
//! Kapsam: dinleyici timedated'den önce başlar (gerçekte bus etkinleştirmesiyle gelir),
//! `Timezone` değişimi olay üretir, ilgisiz özellik (`NTP`) üretmez, timedated çıkıp yeniden
//! gelince (yeni benzersiz ad) yine kabul edilir, sahibi olmayan göndericinin (unicast) sahte
//! sinyali reddedilir, temiz kapanış.

use std::collections::HashMap;
use std::time::Duration;

use nuxut_shell::services::timezone::{TIMEDATE1, TIMEDATE1_PATH, TimezoneEvent, TimezoneOptions, TimezoneWatch};
use zbus::zvariant::Value;

use crate::support::{Events, PrivateBus, channel};

const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

fn start(bus: &PrivateBus) -> (TimezoneWatch, Events<TimezoneEvent>) {
    let (sink, events) = channel();
    let options = TimezoneOptions {
        bus_address: Some(bus.address.clone()),
        call_timeout: Duration::from_millis(500),
        max_backoff: Duration::from_millis(200),
    };
    (TimezoneWatch::spawn(options, sink).unwrap(), events)
}

fn emit(conn: &zbus::blocking::Connection, dest: Option<&str>, prop: &str, value: Value<'_>) {
    let changed: HashMap<&str, Value<'_>> = HashMap::from([(prop, value)]);
    let invalidated: Vec<&str> = Vec::new();
    conn.emit_signal(dest, TIMEDATE1_PATH, PROPERTIES, "PropertiesChanged", &(TIMEDATE1, changed, invalidated)).unwrap();
}

/// Dinleyicinin aboneliği kurulana kadar (hazır olayı yok) aralıklı yayınlar; olayı bekler.
fn emit_until_seen(conn: &zbus::blocking::Connection, events: &Events<TimezoneEvent>, zone: &str) {
    let want = TimezoneEvent::Changed { zone: Some(zone.to_owned()) };
    for _ in 0..50 {
        emit(conn, None, "Timezone", Value::from(zone));
        if events.collect(Duration::from_millis(100)).contains(&want) {
            return;
        }
    }
    panic!("{zone}: dilim değişimi bildirilmedi");
}

#[test]
fn timezone_changes_are_reported_across_restarts() {
    let Some(bus) = PrivateBus::start() else { return };
    let (service, events) = start(&bus);
    // timedated sonradan gelir (bus etkinleştirmesi gibi).
    let fake = bus.serve(TIMEDATE1);
    emit_until_seen(&fake, &events, "Europe/Istanbul");
    emit(&fake, None, "Timezone", Value::from("America/New_York"));
    events.wait_for("ikinci değişim", |e| *e == TimezoneEvent::Changed { zone: Some("America/New_York".into()) });
    // İlgisiz özellik: olay yok.
    emit(&fake, None, "NTP", Value::from(true));
    assert!(events.collect(Duration::from_millis(300)).is_empty(), "NTP değişimi dilim değişimi sayıldı");

    // timedated boşta çıkar, yeniden etkinleşir: yeni benzersiz ad yine kabul edilir.
    drop(fake);
    let fake = bus.serve(TIMEDATE1);
    emit(&fake, None, "Timezone", Value::from("Asia/Tokyo"));
    events.wait_for("yeniden gelen timedated", |e| *e == TimezoneEvent::Changed { zone: Some("Asia/Tokyo".into()) });

    let t = std::time::Instant::now();
    drop(service);
    assert!(t.elapsed() < Duration::from_secs(1), "temiz ve hızlı kapanış");
}

#[test]
fn spoofed_timezone_signal_is_ignored() {
    let Some(bus) = PrivateBus::start() else { return };
    let (_service, events) = start(&bus);
    let fake = bus.serve(TIMEDATE1);
    emit_until_seen(&fake, &events, "Europe/Istanbul");
    let attacker = bus.connect();
    let names: Vec<String> = attacker
        .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    for name in names.iter().filter(|n| n.starts_with(':')) {
        emit(&attacker, Some(name.as_str()), "Timezone", Value::from("Etc/Spoof"));
    }
    let spoofed = events.collect(Duration::from_millis(400)).into_iter().any(|e| matches!(e, TimezoneEvent::Changed { .. }));
    assert!(!spoofed, "sahte sinyal kabul edildi");
}
