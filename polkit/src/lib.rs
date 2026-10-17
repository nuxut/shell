//! `nuxut-polkit`: Nuxut polkit kimlik doğrulama ajanı. Tasarım ve tehdit modeli:
//! shell `docs/design/polkit.md`, özet `docs/ARCHITECTURE.md` "polkit ajanı".
//!
//! - [`agent`] — yerleşik ajan (sistem bus'ı, kayıt, kuyruk, diyalog süreci). Parola görmez.
//! - [`dialog`] — istek başına diyalog süreci (Wayland arayüzü, yardımcı bağlantısı, parola).
//! - [`session`] — diyaloğun saf durum makinesi.
//! - [`protocol`] / [`helper`] — `polkit-agent-helper-1` satır protokolü ve bağlantısı.
//! - [`request`] — ajan ↔ diyalog JSON sözleşmesi (parola alanı yok).
//! - [`text`] — gösterilen dış metinlerin temizliği (oltalama sınırı).
//! - [`logind`] — kaydolunacak oturum. [`config`] — `shell.toml [polkit]`.

pub mod agent;
pub mod config;
pub mod dialog;
#[cfg(feature = "fake-polkit")]
pub mod headless;
pub mod helper;
pub mod i18n;
pub mod logind;
pub mod protocol;
pub mod request;
pub mod session;
pub mod text;
