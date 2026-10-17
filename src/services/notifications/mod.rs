//! Bildirim sunucusu (`org.freedesktop.Notifications`, Desktop Notifications Spec 1.2) ve
//! bildirim merkezi modeli.
//!
//! - [`server`]: zbus sunucusu [`NotificationServer`] ve UI komutları [`NotifyHandle`].
//! - [`model`]: veri türleri ve saf UI modeli [`NotificationCenter`] (popup'lar, geçmiş, DND,
//!   süre dolumu, hover).
//! - [`markup`]: gövde biçimlendirmesinin güvenli alt kümesi → [`BodySpan`] listesi.
//! - [`hints`]: hint çözme (bellek dostu), `image-data` doğrulama, simge yolları.
//! - [`sender`]: `Notify` çağıranının doğrulanmış kimliği (görsel kaynak anahtarı).
//! - [`internal`]: shell'in kendi (D-Bus'sız) bildirimleri [`InternalNotice`]; sessiz olanı
//!   popup'sız merkeze düşer, düğmeleri shell iç komutlarına yönlenir.
//!
//! Akış: sunucu [`NotifyEvent`]'leri callback ile verir → UI `NotificationCenter::on_event`;
//! kullanıcı eylemleri `NotifyHandle`'a (`close`, `invoke_action`, `open_link`); modelin
//! kendiliğinden kapattıkları `NotificationCenter::take_closed` → `NotifyHandle::close`.
//!
//! Geçmiş diske yazılmaz (gizlilik). İleride istenirse: `$XDG_STATE_HOME/nuxut/notifications`
//! altında, kullanıcı ayarıyla açılan, yalnızca metin alanlarını (görsel yok) tutan, boyut ve
//! yaş sınırlı, 0600 izinli bir dosya; yüklenen kayıtlar D-Bus açısından ölü (eylemsiz) olur.

pub mod budget;
pub mod hints;
pub mod images;
pub mod internal;
pub mod markup;
pub mod model;
pub mod sender;
pub mod server;

pub use hints::{NotifyIcon, Urgency};
pub use internal::{InternalAction, InternalNotice, NoticeMood};
pub use markup::{BodyLink, BodySpan, TextStyle, body_text};
pub use model::{
    AppRule, CloseReason, Expiry, HistoryGroup, Notification, NotificationCenter, NotificationsConfig, NotifyAction,
    NotifyEvent,
};
pub use server::{NotificationServer, NotifyError, NotifyHandle, ServerOptions};
