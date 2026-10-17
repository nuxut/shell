//! Test ön koşulu (dbus-daemon, pipewire, wireplumber) başlatılamadığında ne olacağı.
//!
//! Varsayılan: panik — ön koşulu eksik bir ortamda testin sessizce yeşil geçmesi, testin hiç
//! koşmadığını gizler. Yalnızca ilgili ortam değişkeni açıkça `1` ise test uyarıyla atlanır.
//! `tests/it` ve `crates/{nuxut-nm,nuxut-bluez}/tests` (support/mod.rs üzerinden) ile
//! `tests/tray_service.rs` (`#[path]` ile) paylaşır.

#![allow(dead_code)]

use std::fmt::Display;

/// D-Bus gerektiren testleri atlatan değişken (`dbus-daemon`).
pub const SKIP_DBUS: &str = "NUXUT_SKIP_DBUS_TESTS";
/// PipeWire gerektiren testleri atlatan değişken (`pipewire`, `wireplumber`).
pub const SKIP_PIPEWIRE: &str = "NUXUT_SKIP_PIPEWIRE_TESTS";

/// `what` başlatılamadı (`reason`): `skip_env=1` ise uyarıp `None` döner (test atlanır),
/// değilse nedeniyle panikler.
pub fn skip_or_panic<T>(what: &str, skip_env: &str, reason: impl Display) -> Option<T> {
    if std::env::var(skip_env).is_ok_and(|v| v == "1") {
        eprintln!("UYARI: {what} başlatılamadı ({reason}); {skip_env}=1 olduğu için test atlandı");
        return None;
    }
    panic!("{what} başlatılamadı: {reason} (atlamak için {skip_env}=1)");
}
