//! Nuxut kilit ekranı için minimal PAM istemcisi.
//!
//! libpam'e doğrudan, küçük bir FFI (bindgen yok). Tasarım ve tehdit modeli: shell
//! `docs/ARCHITECTURE.md` "Kilit ekranı". Bu crate projede `unsafe`'in kilit tarafındaki tek
//! yeridir; her blok `// SAFETY:` yorumludur.
//!
//! - [`SecretBuf`] — sabit kapasiteli, `mlock`'lu, sıfırlanan şifre tamponu.
//! - [`authenticate`] — bloklayan doğrulama; ayrı thread'den çağrılır. Yalnızca
//!   [`Outcome::Success`] başarıdır.
//! - [`Service::system`] — `/etc/pam.d/nuxut-lock` yoksa `login`.
//! - [`current_user`] — `getpwuid_r(getuid())`.
//! - [`process`] — `PR_SET_DUMPABLE`, başlatıcının hazır fd'si.

mod auth;
mod conv;
mod ffi;
pub mod process;
mod secret;
mod user;

pub use auth::{AuthResult, InvalidService, Outcome, Service, authenticate};
pub use conv::{MAX_MESSAGE_BYTES, MAX_MESSAGES, MessageKind, PamMessage};
pub use secret::{MAX_SECRET_BYTES, SecretBuf, SecretError};
pub use user::{UserInfo, current_user, lookup_uid};
