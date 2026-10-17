//! logind `SetLockedHint`, özel bus'ta sahte logind ile.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{LIVENESS, PrivateBus};
use nuxut_lock::logind::{LOGIN1, SESSION_AUTO, set_locked_hint};

struct FakeSession(Arc<Mutex<Vec<bool>>>);

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl FakeSession {
    fn set_locked_hint(&self, locked: bool) {
        self.0.lock().unwrap().push(locked);
    }
}

#[test]
fn locked_hint_reaches_session() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    let hints = Arc::new(Mutex::new(Vec::new()));
    let _server = zbus::blocking::connection::Builder::address(bus.address.as_str())
        .unwrap()
        .name(LOGIN1)
        .unwrap()
        .serve_at(SESSION_AUTO, FakeSession(hints.clone()))
        .unwrap()
        .build()
        .unwrap();
    // Sonuç beklenir; yük altında 2 sn yetmiyordu (yalnızca asılı kalmaya karşı sınır).
    set_locked_hint(Some(&bus.address), true, LIVENESS).unwrap();
    set_locked_hint(Some(&bus.address), false, LIVENESS).unwrap();
    assert_eq!(*hints.lock().unwrap(), [true, false]);
}

#[test]
fn missing_logind_is_an_error_not_a_hang() {
    let Some(bus) = PrivateBus::start() else {
        return;
    };
    assert!(set_locked_hint(Some(&bus.address), true, Duration::from_secs(1)).is_err());
}
