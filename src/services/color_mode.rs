//! Koyu/açık kip anahtarı (hızlı anahtarlar kartı): `framework.toml` `[colors] mode`'u yazar.
//!
//! Ayarlar'ın yazdığı anahtarın aynısıdır; framework runtime dosyayı izler, tema değişimi
//! (`theme_changed`) oradan gelir ve birleşik görünüm (`look.mode`, GTK/Qt) her zamanki yoldan
//! izler. Yazım framework'ün tek atomik yazıcısıyla (`nuxut_apply::write_file`, okunan hâlin ön
//! koşuluyla): dosyanın geri kalanı (yorumlar dahil) bayt bayt korunur, araya giren dış yazım
//! ezilmez (yeniden okunup bir kez daha denenir), test örneği kullanıcının dosyasına yazamaz
//! (`Env::guard`). Ayrıştırılamayan dosyaya dokunulmaz. Kip `auto` iken anahtar sabit kipe
//! döner (GNOME'un koyu stil anahtarıyla aynı; otomatik yeniden Ayarlar'dan seçilir).
//!
//! Bloklayan IO: yalnızca [`ColorModeWriter`]'ın thread'inde.

use std::path::PathBuf;
use std::sync::mpsc;

use nuxut::theme::ThemeMode;
use nuxut_apply::{ApplyError, Env, Expect, KeyPath, edit};

/// Dış yazımla çakışmada en çok deneme.
const ATTEMPTS: usize = 3;

/// `<config_home>/nuxut/framework.toml` (`Env`'in kökü: test örneğinde test dizini).
pub fn framework_toml(env: &Env) -> Option<PathBuf> {
    Some(env.config_home.as_ref()?.join("nuxut").join(format!("{}.toml", nuxut::theme::CONFIG_NAME)))
}

/// `[colors] mode`'u `mode`'a ayarlar. Yazıldıysa `true`, zaten öyleyse `false`.
pub fn set_mode(env: &Env, mode: ThemeMode) -> Result<bool, ApplyError> {
    set_key(env, KeyPath::new(["colors", "mode"])?, &format!("\"{}\"", mode.name()))
}

/// `[desktop] unified = false`: birleşik görünümü kapatır (Nuxut öncesine dönüş framework.toml
/// değişiminden her zamanki yoldan gelir). "Bu aracı kullan" bildirim düğmesi (2026-10-17).
pub fn set_unified_off(env: &Env) -> Result<bool, ApplyError> {
    set_key(env, KeyPath::new(["desktop", "unified"])?, "false")
}

fn set_key(env: &Env, key: KeyPath, literal: &str) -> Result<bool, ApplyError> {
    let path = framework_toml(env).ok_or(ApplyError::Invalid("XDG_CONFIG_HOME yok"))?;
    let mut last = ApplyError::Changed;
    for _ in 0..ATTEMPTS {
        let (text, expect) = nuxut_apply::read_optional(&path)?.unwrap_or_else(|| (String::new(), Expect::Absent));
        let new = edit::toml::set(&text, &key, Some(literal)).map_err(|_| ApplyError::Parse(path.clone()))?;
        if new == text {
            return Ok(false);
        }
        match nuxut_apply::write_file(env, &path, new.as_bytes(), Some(&expect)) {
            Ok(_) => return Ok(true),
            Err(ApplyError::Changed) => last = ApplyError::Changed,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// Kip yazımlarını sırayla yapan arka plan thread'i; ilk istekte açılır. Art arda gelen
/// isteklerden yalnızca sonuncusu yazılır. Düşürülünce kanal kapanır, thread kalanı bitirip çıkar.
#[derive(Debug, Default)]
pub struct ColorModeWriter {
    tx: Option<mpsc::Sender<ThemeMode>>,
}

impl ColorModeWriter {
    pub fn request(&mut self, env: &Env, mode: ThemeMode) {
        if let Some(tx) = &self.tx
            && tx.send(mode).is_ok()
        {
            return;
        }
        let (tx, rx) = mpsc::channel::<ThemeMode>();
        let env = env.clone();
        let spawned = std::thread::Builder::new().name("nuxut-color-mode".into()).spawn(move || {
            while let Ok(mut mode) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    mode = newer;
                }
                match set_mode(&env, mode) {
                    Ok(written) => tracing::info!(mode = mode.name(), written, "koyu/açık kip"),
                    Err(e) => tracing::warn!("koyu/açık kip yazılamadı: {}", crate::services::apply::error_text(&e)),
                }
            }
        });
        match spawned {
            Ok(_) => {
                let _ = tx.send(mode);
                self.tx = Some(tx);
            }
            Err(e) => tracing::warn!("koyu/açık kip thread'i başlatılamadı: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> (tempfile::TempDir, Env, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::for_test(dir.path());
        let path = framework_toml(&env).unwrap();
        (dir, env, path)
    }

    #[test]
    fn creates_file_and_keeps_the_rest() {
        let (_d, env, path) = env();
        assert!(set_mode(&env, ThemeMode::Light).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "[colors]\nmode = \"light\"");

        std::fs::write(&path, "# benim\n[colors]\nmode = \"auto\" # yorum\nprimary = \"#ff0000\"\n").unwrap();
        assert!(set_mode(&env, ThemeMode::Dark).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# benim\n[colors]\nmode = \"dark\""), "{text}");
        assert!(text.contains("# yorum") && text.contains("primary = \"#ff0000\""), "{text}");
        assert!(!set_mode(&env, ThemeMode::Dark).unwrap(), "aynı kip: yazım yok");
    }

    #[test]
    fn broken_file_is_left_alone() {
        let (_d, env, path) = env();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[colors\nmode = 1").unwrap();
        assert!(set_mode(&env, ThemeMode::Light).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[colors\nmode = 1");
    }

    #[test]
    fn disabled_env_never_writes() {
        let (_d, env, path) = env();
        let env = Env { disabled: true, ..env };
        assert!(set_mode(&env, ThemeMode::Light).is_err());
        assert!(!path.exists());
    }
}
