//! Dosya seçici servisleri (`nuxut-filechooser`; docs/design/file-chooser.md). UI'dan bağımsız:
//! - [`listing`]: dizin okuma (sınırlı, sıralı, bağ politikası) ve arka plan okuyucu,
//! - [`mime`]: addan MIME türü (shared-mime-info) ve filtre eşleme,
//! - [`places`]: kullanıcı dizinleri ve GTK yer imleri,
//! - [`names`]: ad doğrulama, benzersiz ad, konum çözümü, arama,
//! - [`thumbs`]: ızgara küçük resimleri (yalnız yalıtılmış çözüm, yalnız görünen hücreler).
//!
//! Portal ile sözleşme `nuxut_portal_backend::chooser::protocol`'dedir (portal crate'i).

pub mod listing;
pub mod mime;
pub mod names;
pub mod places;
pub mod thumbs;
