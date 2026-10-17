//! Nuxut xdg-desktop-portal arka ucu (`xdg-desktop-portal-nuxut`, docs/design/portal.md).
//!
//! Arayüzler: `org.freedesktop.impl.portal.Settings` (sürüm 2) ve `FileChooser` ([`chooser`]). Settings yalnızca
//! `org.freedesktop.appearance` ad alanı: `color-scheme`, `accent-color`, `contrast`,
//! `reduced-motion`. Diğer ad alanları bilinmez; xdg-desktop-portal onları `nuxut-portals.conf`
//! sırasındaki bir sonraki arka uçtan (gtk) alır.
//!
//! Katmanlar:
//! - [`appearance`]: saf eşleme (çözülmüş tema → portal değerleri) ve fark,
//! - [`settings`]: D-Bus arayüzü (ReadAll ad alanı süzgeci, Read, SettingChanged),
//! - [`service`]: bus bağlantısı, paylaşılan durum ve config izleyici döngüsü,
//! - [`portals_conf`]: `nuxut-portals.conf`'un kullanıcının Hyprland seçiminden türetilmesi,
//! - [`chooser`]: `org.freedesktop.impl.portal.FileChooser` (görünüm ayrı `nuxut-filechooser` süreci).

pub mod appearance;
pub mod chooser;
pub mod portals_conf;
pub mod service;
pub mod settings;

/// Arka ucun bus adı (`nuxut.portal` `DBusName`).
pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.nuxut";
/// xdg-desktop-portal'ın arka uçlarda aradığı nesne yolu.
pub const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
/// Sunulan arayüz (görünüm).
pub const SETTINGS_IFACE: &str = "org.freedesktop.impl.portal.Settings";
/// Dosya seçici arayüzü.
pub const FILE_CHOOSER_IFACE: &str = "org.freedesktop.impl.portal.FileChooser";
