//! Boşta kalma (t061): karart → ekranı kapat → (açıksa) kilitle → uyu; uykudan önce kilit;
//! kahve modu. Tasarım: `docs/design/idle.md`.
//!
//! - [`machine`]: saf durum makinesi (girdi → [`Effect`]), zaman çağırandan.
//! - [`config`]: `[idle]` şeması ve doğrulaması.
//! - [`watches`]: compositor izlemesi (ext-idle-notify-v1) ↔ aşama eşlemesi.
//! - [`logind`]: `PrepareForSleep`, `idle` blok inhibitörleri, `LockedHint`, uyku gecikme
//!   inhibitörü, `Session.Lock`/`Suspend`.
//!
//! "Boşta" bilgisi compositor'dan (ext-idle-notify-v1, Wayland idle inhibitor'larına saygılı)
//! gelir; bağlaması framework runtime'ındadır (`Context::idle_watch`). Uygulama tarafı
//! `app/idle.rs`.

pub mod config;
pub mod logind;
pub mod machine;
pub mod watches;

pub use config::IdleConfig;
pub use logind::{IdleLogind, LogindAction, LogindEvent, LogindOptions, LogindState};
pub use machine::{Effect, IdleMachine, IdleSettings, IdleStatus, Inhibitor, Stage};
pub use watches::{IdleWatcher, WatchSource, Watches};
