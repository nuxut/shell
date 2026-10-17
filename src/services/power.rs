//! Güç: UPower + power-profiles-daemon istemcisi `nuxut-power` crate'indedir (Ayarlar ile
//! paylaşılır; UI'sız). Burada yalnızca yeniden dışa aktarma; hata türünün kullanıcı metni
//! `system.rs`'de (`charge_error_text` / `profile_error_text`).

pub use nuxut_power::*;
