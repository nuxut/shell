//! `org.kde.StatusNotifierWatcher` sunucusu (oturumda başka watcher yoksa) ve kayıt
//! dizgilerinin ayrıştırılması.
//!
//! Arayüz yalnızca kaydı servis döngüsüne iletir ve listeyi sunar; hiçbir D-Bus çağrısını
//! beklemez. Liste paylaşılan bir `Mutex` içindedir (kısa kritik bölge, `await` yok).

use std::sync::{Arc, Mutex, PoisonError};

use zbus::message::Header;
use zbus::names::{BusName, UniqueName};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::ObjectPath;

pub const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
pub const WATCHER_PATH: &str = "/StatusNotifierWatcher";
pub const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
pub const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";
/// Kayıt dizgisinin bayt sınırı (bus adı ≤ 255 + yol).
const MAX_SERVICE_BYTES: usize = 1024;

/// Bir kaydın hedefi: öğeyi sunan bus adı (benzersiz veya iyi bilinen) ve nesne yolu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Registration {
    pub bus: String,
    pub path: String,
}

/// `RegisterStatusNotifierItem` argümanını çözer. Biçimler:
/// - `org.kde.StatusNotifierItem-123-1` / `:1.42` → varsayılan yol `/StatusNotifierItem`
/// - `/org/ayatana/NotificationItem/nm_applet` (yalnızca yol) → bus = çağıran (`sender`)
/// - `:1.42/StatusNotifierItem` (watcher listesi biçimi) → bus + yol
pub(crate) fn parse_registration(service: &str, sender: Option<&str>) -> Option<Registration> {
    if service.is_empty() || service.len() > MAX_SERVICE_BYTES {
        return None;
    }
    let (bus, path) = if service.starts_with('/') {
        (sender?, service)
    } else {
        match service.find('/') {
            Some(i) => (&service[..i], &service[i..]),
            None => (service, DEFAULT_ITEM_PATH),
        }
    };
    BusName::try_from(bus).ok()?;
    ObjectPath::try_from(path).ok()?;
    Some(Registration { bus: bus.to_owned(), path: path.to_owned() })
}

pub(crate) fn is_unique_name(name: &str) -> bool {
    UniqueName::try_from(name).is_ok()
}

/// Watcher'dan servis döngüsüne giden kayıt.
#[derive(Debug)]
pub(crate) struct ItemRegistration {
    pub service: String,
    pub sender: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RegisteredItems(Arc<Mutex<Vec<String>>>);

impl RegisteredItems {
    pub fn snapshot(&self) -> Vec<String> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn set(&self, items: Vec<String>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = items;
    }
}

#[derive(Debug)]
pub(crate) struct Watcher {
    pub registrations: async_channel::Sender<ItemRegistration>,
    pub items: RegisteredItems,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(&self, service: &str, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        let sender = header.sender().map(|s| s.to_string());
        let registration = ItemRegistration { service: service.to_owned(), sender };
        // Döngü meşgulse beklemek yerine reddedilir: object server kilitlenmez.
        self.registrations
            .try_send(registration)
            .map_err(|_| zbus::fdo::Error::LimitsExceeded("tepsi kayıt kuyruğu dolu".into()))
    }

    async fn register_status_notifier_host(
        &self,
        _service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        // Host bu servisin kendisi; ek host'lar kabul edilir ama ayrıca izlenmez.
        Self::status_notifier_host_registered(&emitter).await?;
        Ok(())
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.snapshot()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    pub async fn status_notifier_item_registered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn status_notifier_item_unregistered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(bus: &str, path: &str) -> Option<Registration> {
        Some(Registration { bus: bus.into(), path: path.into() })
    }

    #[test]
    fn registration_forms() {
        assert_eq!(parse_registration("org.kde.StatusNotifierItem-12-1", None), reg("org.kde.StatusNotifierItem-12-1", DEFAULT_ITEM_PATH));
        assert_eq!(parse_registration(":1.42", Some(":1.9")), reg(":1.42", DEFAULT_ITEM_PATH));
        assert_eq!(parse_registration("/org/ayatana/NotificationItem/nm_applet", Some(":1.7")), reg(":1.7", "/org/ayatana/NotificationItem/nm_applet"));
        assert_eq!(parse_registration(":1.42/StatusNotifierItem", None), reg(":1.42", "/StatusNotifierItem"));
    }

    #[test]
    fn invalid_registrations() {
        assert_eq!(parse_registration("", Some(":1.1")), None);
        assert_eq!(parse_registration("/only/path", None), None, "yalnızca yol: gönderen gerekir");
        assert_eq!(parse_registration("bad name!", None), None);
        assert_eq!(parse_registration(":1.4//double", None), None);
        assert_eq!(parse_registration(&"a.b".repeat(400), None), None);
        assert!(is_unique_name(":1.4"));
        assert!(!is_unique_name("org.a.b"));
    }
}
