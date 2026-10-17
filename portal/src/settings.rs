//! `org.freedesktop.impl.portal.Settings` (sürüm 2).
//!
//! Durum paylaşılır ([`Shared`]): izleyici döngüsü yazar, D-Bus çağrıları okur. Kilit yalnızca
//! değer kopyası için tutulur (bloklayan iş yok).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::appearance::{Appearance, NAMESPACE};

/// Arayüz sürümü (xdg-desktop-portal 1.22 `Settings` arka uç belirtimi).
pub const VERSION: u32 = 2;

/// Paylaşılan görünüm durumu; `None` = saydam (birleşik görünüm kapalı).
pub type Shared = Arc<Mutex<Option<Appearance>>>;

/// Portal hata alanı.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
pub enum PortalError {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// Bilinmeyen ad alanı ya da anahtar (xdg-desktop-portal sıradaki arka uca geçer).
    NotFound(String),
}

/// `ReadAll` ad alanı süzgeci: boş liste ya da boş desen hepsiyle eşleşir; yalnızca sondaki `*`
/// glob'u desteklenir (`org.freedesktop.*`). Belirtim ve xdg-desktop-portal-gnome ile aynı kural.
pub fn namespace_matches(namespace: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| match p.strip_suffix('*') {
        _ if p.is_empty() => true,
        Some(prefix) => namespace.starts_with(prefix),
        None => namespace == p,
    })
}

/// D-Bus nesnesi.
#[derive(Debug, Clone)]
pub struct Settings {
    state: Shared,
}

impl Settings {
    pub fn new(state: Shared) -> Self {
        Self { state }
    }

    fn current(&self) -> Option<Appearance> {
        // Zehirli kilit: yazıcı panik vermez (değer kopyası), yine de son değer okunur.
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl Settings {
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut out = HashMap::new();
        if let Some(a) = self.current()
            && namespace_matches(NAMESPACE, &namespaces)
        {
            out.insert(NAMESPACE.to_owned(), a.entries().into_iter().map(|(k, v)| (k.to_owned(), v)).collect());
        }
        out
    }

    fn read(&self, namespace: &str, key: &str) -> Result<OwnedValue, PortalError> {
        let found = (namespace == NAMESPACE).then(|| self.current()).flatten().and_then(|a| a.value(key));
        found.ok_or_else(|| PortalError::NotFound("Requested setting not found".to_owned()))
    }

    #[zbus(signal)]
    pub async fn setting_changed(emitter: &SignalEmitter<'_>, namespace: &str, key: &str, value: Value<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn glob_rules() {
        let ns = NAMESPACE;
        assert!(namespace_matches(ns, &[]));
        assert!(namespace_matches(ns, &p(&[""])));
        assert!(namespace_matches(ns, &p(&["*"])));
        assert!(namespace_matches(ns, &p(&["org.freedesktop.*"])));
        assert!(namespace_matches(ns, &p(&["org.freedesktop.appearance"])));
        assert!(namespace_matches(ns, &p(&["org.gnome.*", "org.freedesktop.appearance"])));
        assert!(!namespace_matches(ns, &p(&["org.gnome.*"])));
        assert!(!namespace_matches(ns, &p(&["org.freedesktop"])));
        assert!(!namespace_matches(ns, &p(&["org.freedesktop.appearance.x"])));
        // Yalnızca sondaki glob: ortadaki `*` düz karakterdir.
        assert!(!namespace_matches(ns, &p(&["org.*.appearance"])));
    }
}
