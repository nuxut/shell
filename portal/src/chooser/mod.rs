//! Dosya seçici: `org.freedesktop.impl.portal.FileChooser` (docs/design/file-chooser.md).
//!
//! Arayüz bu süreçte, görünüm istek başına ayrı `nuxut-filechooser` sürecinde (portal UI'sız
//! kalır, boşta iş yok). Katmanlar:
//! - [`protocol`]: portal ↔ seçici JSON sözleşmesi, yol kodlaması, sınırlar, metin temizliği,
//! - [`options`]: D-Bus seçenekleri → istek; seçici sonucu → doğrulanmış yanıt,
//! - [`child`]: seçici sürecinin ömrü (stdin = yaşam hattı, iptal, öldürme),
//! - [`iface`]: D-Bus arayüzü ve `Request.Close`.

pub mod child;
pub mod iface;
pub mod options;
pub mod protocol;

pub use iface::FileChooser;
