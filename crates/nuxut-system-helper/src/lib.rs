//! Nuxut kök yetkili sistem yardımcısı (ADR-0008, `docs/design/boot-theme.md`).
//!
//! `pkexec /usr/lib/nuxut/nuxut-system-helper <alan> …` ile, alan başına ayrı polkit eylemiyle
//! (`org.nuxut.system-helper.<alan>`, `exec.argv1` eşlemesi) çağrılır. Alanlar:
//! `boot-theme`, `greeter` (giriş ekranı; [`greeter`]) ve `packages` (önerilen resmî depo paketleri;
//! [`packages`]).
//!
//! İlkeler: girdi yalnız sınırlı argümanlar (dosya yolu ya da içerik kabul edilmez); yazma yalnız
//! beyaz listeye ([`policy`]); her yol doğrulanmış tanıtıcılarla ([`safefs`]); her değişiklik
//! yedekli ve günlüklü ([`manifest`]); hata = o hedefin işlem öncesi hâline dönüş ([`ops::Tx`]).
//! Panik yok: sürüm derlemesi `panic = "abort"`, kod `unwrap` kullanmaz.

pub mod args;
pub mod boot_theme;
pub mod cmd;
pub mod events;
pub mod greeter;
pub mod manifest;
pub mod ops;
pub mod packages;
pub mod policy;
pub mod safefs;

/// Kurulu yol (paket ve `install.sh --system-theme`): kökün sahip olduğu dizinde olmalı (polkit
/// eylemi bu yola bağlı; kullanıcının yazabildiği bir yerdeki ikili kök yetki almamalı).
pub const INSTALL_PATH: &str = "/usr/lib/nuxut/nuxut-system-helper";
/// polkit eylem kimlikleri (`dist/polkit/org.nuxut.system-helper.policy`).
pub const ACTION_BOOT_THEME: &str = "org.nuxut.system-helper.boot-theme";
pub const ACTION_GREETER: &str = "org.nuxut.system-helper.greeter";
pub const ACTION_PACKAGES: &str = "org.nuxut.system-helper.packages";
