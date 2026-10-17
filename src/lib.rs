//! nuxut-shell: Hyprland masaüstü kabuğu.
//!
//! Mimari: `docs/ARCHITECTURE.md`. Bu crate'in kütüphane kısmı runtime'dan bağımsız,
//! test edilebilir çekirdeği içerir (config, geometri, model, servisler); `main.rs` bunları
//! runtime'a bağlar.

/// Testlerin tek canlılık sınırı (entegrasyon testleriyle aynı dosya).
#[cfg(test)]
#[path = "../tests/support/liveness.rs"]
pub(crate) mod test_liveness;

pub mod app;
pub mod bar_damage;
pub mod bar_edge;
pub mod calendar_view;
pub mod config;
pub mod config_heal;
pub mod config_migrate;
pub mod doctor;
pub mod i18n;
pub mod ipc;
pub mod emoji_view;
pub mod filechooser_view;
pub mod launcher_view;
pub mod layout;
pub mod model;
pub mod panels;
pub mod percent;
pub mod schema;
pub mod screen;
pub mod osk_view;
pub mod screenshot_view;
pub mod self_exe;
pub mod services;
pub mod sharepicker_view;
pub mod sheet;
pub mod sysmon_view;
pub mod system;
pub mod wallpaper;
pub mod widgets;
