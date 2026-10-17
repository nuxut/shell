//! `[idle]`: boşta kalma ayarları, şeması ve doğrulaması.
//!
//! Kullanıcı kararı: hiçbir boşta eylemi varsayılan açık değil. İstisna uykudan önce kilit
//! (`lock_before_sleep`, boşta eylemi değil; karar 2026-10-16: GNOME/KDE/macOS/
//! Windows'ta varsayılan, kapalı olması güvenlik sürprizi). Her aşamanın kendi anahtarı (`dim`,
//! `screen_off`, `lock`, `suspend`) ve süresi (`*_after`) vardır; anahtar kapalıyken süre
//! saklanır, açınca makul değerle başlar.

use std::time::Duration;

use nuxut::i18n::t;
use serde::Deserialize;

use super::machine::IdleSettings;

use crate::schema::SettingSchema;

/// Aşama süresi sınırı (sn).
pub const STAGE_SECS: std::ops::RangeInclusive<u32> = 30..=14_400;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct IdleConfig {
    /// Ana anahtar: kapalıyken hiçbir boşta eylemi ve uyku öncesi kilit çalışmaz (hypridle
    /// kullanan kapatır). Aşamalar ayrıca kendi anahtarlarıyla açılır.
    pub enabled: bool,
    /// Ekranı karart.
    pub dim: bool,
    /// Saniye ([`STAGE_SECS`]).
    pub dim_after: u32,
    /// Ekranı kapat (Hyprland DPMS).
    pub screen_off: bool,
    pub screen_off_after: u32,
    /// Boşta kalınca kilitle (`lock_after` sonra).
    pub lock: bool,
    /// `0` = boşta kilitleme.
    pub lock_after: u32,
    /// Uykudan önce kilitle (varsayılan açık; karar 2026-10-16, `docs/design/lock-trigger.md`).
    pub lock_before_sleep: bool,
    /// Uyu (logind `Suspend`).
    pub suspend: bool,
    pub suspend_after: u32,
    /// Prizdeyken de uyu (varsayılan: yalnızca pildeyken).
    pub suspend_on_ac: bool,
    /// Bir MPRIS oynatıcısı çalarken bekle.
    pub media_inhibits: bool,
}

impl Default for IdleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dim: false,
            dim_after: 300,
            screen_off: false,
            screen_off_after: 600,
            lock: false,
            lock_after: 600,
            lock_before_sleep: true,
            suspend: false,
            suspend_after: 1800,
            suspend_on_ac: false,
            media_inhibits: true,
        }
    }
}

impl IdleConfig {
    /// Şema; başlık ve açıklamalar katalogdan (`shell-config-idle-*`, ADR-0010).
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        let (min, max) = (i64::from(*STAGE_SECS.start()), i64::from(*STAGE_SECS.end()));
        let secs = |key: &'static str, title: String, value: u32| {
            SettingSchema::int(key, title, value.into(), min, max).step(30.0).unit("s")
        };
        out.push(
            SettingSchema::bool("idle.enabled", t!("shell-config-idle-enabled-label"), d.enabled)
                .describe(t!("shell-config-idle-enabled-description")),
        );
        out.push(SettingSchema::bool("idle.dim", t!("shell-config-idle-dim-label"), d.dim));
        out.push(secs("idle.dim_after", t!("shell-config-idle-dim-after-label"), d.dim_after));
        out.push(SettingSchema::bool("idle.screen_off", t!("shell-config-idle-screen-off-label"), d.screen_off));
        out.push(secs("idle.screen_off_after", t!("shell-config-idle-screen-off-after-label"), d.screen_off_after));
        out.push(
            SettingSchema::bool("idle.lock", t!("shell-config-idle-lock-label"), d.lock)
                .describe(t!("shell-config-idle-lock-description")),
        );
        out.push(
            SettingSchema::int("idle.lock_after", t!("shell-config-idle-lock-after-label"), d.lock_after.into(), 0, max)
                .step(30.0)
                .unit("s")
                .describe(t!("shell-config-idle-lock-after-description")),
        );
        out.push(
            SettingSchema::bool("idle.lock_before_sleep", t!("shell-config-idle-lock-before-sleep-label"), d.lock_before_sleep)
                .describe(t!("shell-config-idle-lock-before-sleep-description")),
        );
        out.push(
            SettingSchema::bool("idle.suspend", t!("shell-config-idle-suspend-label"), d.suspend)
                .describe(t!("shell-config-idle-suspend-description")),
        );
        out.push(secs("idle.suspend_after", t!("shell-config-idle-suspend-after-label"), d.suspend_after));
        out.push(SettingSchema::bool("idle.suspend_on_ac", t!("shell-config-idle-suspend-on-ac-label"), d.suspend_on_ac));
        out.push(
            SettingSchema::bool("idle.media_inhibits", t!("shell-config-idle-media-inhibits-label"), d.media_inhibits)
                .describe(t!("shell-config-idle-media-inhibits-description")),
        );
    }

    pub fn validate(&mut self, w: &mut Vec<String>) {
        for (value, key, zero_ok) in [
            (&mut self.dim_after, "idle.dim_after", false),
            (&mut self.screen_off_after, "idle.screen_off_after", false),
            (&mut self.lock_after, "idle.lock_after", true),
            (&mut self.suspend_after, "idle.suspend_after", false),
        ] {
            if (zero_ok && *value == 0) || STAGE_SECS.contains(value) {
                continue;
            }
            let fixed = (*value).clamp(*STAGE_SECS.start(), *STAGE_SECS.end());
            let range = if zero_ok { "0 | 30..=14400" } else { "30..=14400" };
            w.push(t!("shell-config-idle-out-of-range", key = key, value = *value, fixed = fixed, range = range));
            *value = fixed;
        }
    }

    /// Durum makinesinin ayarları (kapalı aşama = `None`).
    pub fn settings(&self) -> IdleSettings {
        let stage = |on: bool, s: u32| (on && s > 0).then(|| Duration::from_secs(s.into()));
        IdleSettings {
            enabled: self.enabled,
            dim: stage(self.dim, self.dim_after),
            screen_off: stage(self.screen_off, self.screen_off_after),
            lock: self.enabled && self.lock,
            lock_before_sleep: self.enabled && self.lock_before_sleep,
            lock_after: stage(self.lock, self.lock_after),
            suspend: stage(self.suspend, self.suspend_after),
            suspend_on_ac: self.suspend_on_ac,
            media_inhibits: self.media_inhibits,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_turn_every_action_off() {
        let d = IdleConfig::default();
        assert!(!d.dim && !d.screen_off && !d.lock && !d.suspend, "kullanıcı kararı: hiçbir eylem varsayılan açık değil");
        let s = d.settings();
        assert!(s.stages().is_empty() && !s.lock && !s.suspend_on_ac);
        assert!(d.lock_before_sleep && s.lock_before_sleep, "karar: uykudan önce kilit varsayılan açık");
        // Açılınca süreler makul değerle başlar.
        let on = IdleConfig { dim: true, suspend: true, ..d };
        let s = on.settings();
        assert_eq!((s.dim, s.suspend), (Some(Duration::from_secs(300)), Some(Duration::from_secs(1800))));
    }

    #[test]
    fn master_switch_disables_sleep_lock_too() {
        let c = IdleConfig { enabled: false, lock: true, ..IdleConfig::default() };
        let s = c.settings();
        assert!(!s.lock && !s.lock_before_sleep && s.stages().is_empty());
        let c: IdleConfig = toml::from_str("lock_before_sleep = false").unwrap();
        assert!(c.enabled && !c.settings().lock_before_sleep);
    }

    #[test]
    fn validation_clamps_and_lock_after_keeps_zero() {
        let mut c = IdleConfig { dim_after: 0, screen_off_after: 5, lock_after: 0, suspend_after: 99_999, ..IdleConfig::default() };
        let mut w = Vec::new();
        c.validate(&mut w);
        assert_eq!((c.dim_after, c.screen_off_after, c.lock_after, c.suspend_after), (30, 30, 0, 14_400));
        assert_eq!(w.len(), 3, "{w:?}");
        let mut again = Vec::new();
        c.validate(&mut again);
        assert!(again.is_empty(), "idempotent");
        let c = IdleConfig { lock: true, ..c };
        let s = c.settings();
        assert!(s.lock && s.lock_after.is_none(), "0 = boşta kilitlenmez");
    }

    #[test]
    fn parses_partial_table() {
        let c: IdleConfig = toml::from_str("lock = true\ndim = true\ndim_after = 120").unwrap();
        assert!(c.lock && c.dim && !c.suspend && c.dim_after == 120 && c.screen_off_after == 600);
    }
}
