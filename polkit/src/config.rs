//! `shell.toml [polkit]` (salt okunur). Şema satırı shell'in yapılandırma katmanına ait;
//! ajan yalnızca bu tabloyu okur. Bozuk dosya/değer ajanı engellemez: varsayılan + uyarı.
//!
//! ```toml
//! [polkit]
//! enabled = true     # false: ajan kaydolmadan çıkar (başka ajan kullanılır)
//! timeout_s = 300    # etkileşimsiz diyalog bu süre sonra iptal (30..=3600)
//! ```

use std::io::Read;
use std::time::Duration;

/// `shell.toml` okuma sınırı (shell'in kendi sınırıyla aynı mertebe).
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
pub const MIN_TIMEOUT_S: u64 = 30;
pub const MAX_TIMEOUT_S: u64 = 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolkitConfig {
    pub enabled: bool,
    pub timeout: Duration,
}

impl Default for PolkitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout: Duration::from_secs(300),
        }
    }
}

impl PolkitConfig {
    /// TOML metnini çözer; hiçbir girdi hata döndürmez.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut cfg = Self::default();
        let mut warnings = Vec::new();
        let table = match text.parse::<toml::Table>() {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("shell.toml okunamadı ({}); [polkit] varsayılan", e.message())); // i18n: tanı yapılandırma uyarısı (log)
                return (cfg, warnings);
            }
        };
        let Some(section) = table.get("polkit") else {
            return (cfg, warnings);
        };
        let Some(section) = section.as_table() else {
            warnings.push("[polkit] tablo değil; varsayılanlar".into()); // i18n: tanı yapılandırma uyarısı (log)
            return (cfg, warnings);
        };
        for (key, value) in section {
            match key.as_str() {
                "enabled" => match value.as_bool() {
                    Some(b) => cfg.enabled = b,
                    None => warnings.push("[polkit] enabled true/false olmalı".into()), // i18n: tanı yapılandırma uyarısı (log)
                },
                "timeout_s" => match value.as_integer().and_then(|v| u64::try_from(v).ok()) {
                    Some(v) if (MIN_TIMEOUT_S..=MAX_TIMEOUT_S).contains(&v) => {
                        cfg.timeout = Duration::from_secs(v);
                    }
                    _ => warnings.push(format!(
                        "[polkit] timeout_s {MIN_TIMEOUT_S}..={MAX_TIMEOUT_S} olmalı; 300" // i18n: tanı yapılandırma uyarısı (log)
                    )),
                },
                other => warnings.push(format!("[polkit] bilinmeyen anahtar: {other}")),
            }
        }
        (cfg, warnings)
    }

    /// `$XDG_CONFIG_HOME/nuxut/shell.toml`; yoksa varsayılanlar.
    pub fn load() -> Self {
        let Some(path) = nuxut::config::config_dir().map(|d| d.join("shell.toml")) else {
            return Self::default();
        };
        let mut text = String::new();
        let read = std::fs::File::open(&path)
            .and_then(|f| f.take(MAX_CONFIG_BYTES).read_to_string(&mut text));
        match read {
            Ok(_) => {
                let (cfg, warnings) = Self::parse(&text);
                for w in warnings {
                    tracing::warn!("{w}");
                }
                cfg
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                tracing::warn!("shell.toml okunamadı ({:?}); [polkit] varsayılan", e.kind());
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_values() {
        assert_eq!(PolkitConfig::parse("").0, PolkitConfig::default());
        let (c, w) = PolkitConfig::parse("[bar]\nx = 1\n[polkit]\nenabled = false\ntimeout_s = 60\n");
        assert!(w.is_empty());
        assert!(!c.enabled);
        assert_eq!(c.timeout, Duration::from_secs(60));
    }

    #[test]
    fn bad_values_fall_back() {
        let (c, w) = PolkitConfig::parse("[polkit]\nenabled = 1\ntimeout_s = 5\nfoo = 2\n");
        assert_eq!(c, PolkitConfig::default());
        assert_eq!(w.len(), 3);
        let (c, w) = PolkitConfig::parse("polkit = 3");
        assert_eq!(c, PolkitConfig::default());
        assert_eq!(w.len(), 1);
        let (c, w) = PolkitConfig::parse("[polkit\n");
        assert_eq!(c, PolkitConfig::default());
        assert_eq!(w.len(), 1);
        let (c, _) = PolkitConfig::parse("[polkit]\ntimeout_s = -1\n");
        assert_eq!(c.timeout, Duration::from_secs(300));
    }
}
