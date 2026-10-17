//! Ekranlar (ADR-0005): monitör modeli, doğrulama, Hyprland kural üretimi (Lua + hyprlang),
//! `displays.toml` küme hafızası, IPC şeması ve onay/geri dönüş durum makinesi.
//!
//! Saf katmanlar ([`model`], [`validate`], [`render`], [`store`], [`protocol`]) IO yapmaz.
//! [`manager`] Hyprland'e yalnızca [`HyprControl`](crate::services::hypr_config::control::HyprControl)
//! ve dosyalara yalnızca [`manager::DisplayFiles`] üzerinden erişir (testlerde ikisi de sahte).
//! Çalışma anı thread'i `app/displays.rs`'tedir.

pub mod guard;
pub mod manager;
pub mod model;
pub mod protocol;
pub mod render;
pub mod store;
pub mod validate;

use serde::Deserialize;

use nuxut::i18n::t;
use crate::schema::SettingSchema;
use manager::{DEFAULT_CONFIRM_SECONDS, MAX_CONFIRM_SECONDS, MIN_CONFIRM_SECONDS};

/// `shell.toml [displays]`: yalnızca davranış ayarları (K7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct DisplaysConfig {
    /// Riskli değişiklikten sonra onay penceresinin geri sayımı (sn).
    pub confirm_seconds: u32,
}

impl Default for DisplaysConfig {
    fn default() -> Self {
        Self { confirm_seconds: DEFAULT_CONFIRM_SECONDS }
    }
}

impl DisplaysConfig {
    pub fn schema(out: &mut Vec<SettingSchema>) {
        out.push(
            SettingSchema::int(
                "displays.confirm_seconds",
                t!("shell-displays-confirm-seconds-label"),
                i64::from(DEFAULT_CONFIRM_SECONDS),
                i64::from(MIN_CONFIRM_SECONDS),
                i64::from(MAX_CONFIRM_SECONDS),
            )
            .unit("s"),
        );
    }

    pub fn validate(&mut self, w: &mut Vec<String>) {
        let r = MIN_CONFIRM_SECONDS..=MAX_CONFIRM_SECONDS;
        if !r.contains(&self.confirm_seconds) {
            let fixed = self.confirm_seconds.clamp(MIN_CONFIRM_SECONDS, MAX_CONFIRM_SECONDS);
            w.push(format!(
                "displays.confirm_seconds = {} aralık dışı, {fixed} kullanılıyor ({MIN_CONFIRM_SECONDS}..={MAX_CONFIRM_SECONDS})",
                self.confirm_seconds
            ));
            self.confirm_seconds = fixed;
        }
    }
}
