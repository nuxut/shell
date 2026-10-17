//! Entegrasyon testleri tek ikilide (`cargo test --test it`). Her `tests/*.rs` ayrı bir ikili
//! olduğu için her biri ayrı linklenirdi; burada hepsi tek bağlantı adımıyla derlenir.
//!
//! Ayrı ikili kalanlar (süreç-global durumu paylaşırlar, birlikte koşmaları ölçümü bozar):
//! - `tests/notifications_memory.rs`: süreç RSS'i ölçümü (`/proc/self/statm`).
//! - `tests/notifications_dbus.rs`: zamanlama testleri + süreç çocuğu olarak işçi sayımı.
//! - `tests/tray_service.rs`: süreç çocuğu olarak işçi sayımı (`support/sandbox.rs`).
//!
//! Ortak yardımcılar `tests/support/` altında; ikili içinde bir kez bağlanır.

#[path = "../support/mod.rs"]
mod support;

mod at_policy;
mod audio_service;
mod calendar_service;
mod displays_ipc;
mod hyprland_service;
mod i18n;
mod idle_logind;
mod install_bins;
mod install_legacy_source;
mod install_path;
mod install_pkgs;
mod install_reexec;
mod install_source_trust;
mod locale_changed;
mod lock_notice_labels;
mod logout;
mod magnifier;
mod mascot_placement;
mod media_service;
mod no_hardcoded_colors;
mod no_hardcoded_fonts;
mod notifications_markup;
mod notifications_model;
mod power_service;
mod removable_media;
mod service_takeover;
mod session_service;
mod timezone_watch;
mod xdph_picker_script;
