//! `nuxut-lock`: Nuxut kilit ekranı. Tasarım ve tehdit modeli: shell `docs/ARCHITECTURE.md`
//! "Kilit ekranı".
//!
//! - [`state`] — saf durum makinesi; güvenlik kurallarının tamamı burada ve testli.
//! - [`auth`] — `Authenticator` (PAM / sahte) ve deneme thread'i.
//! - [`assets`] — avatar ve duvar kağıdı yolları.
//! - [`i18n`] — kilidin Fluent kataloğu (`shell-lock-…`).
//! - [`images`] — duvar kağıdı ve avatarın yalıtılmış çözümü (paylaşılan çözücüler).
//! - [`config`] — `lock.toml`; bozuk config kilidi engellemez.
//! - [`fingerprint`] — fprintd D-Bus istemcisi, şifreyle paralel.
//! - [`instance`] — ikincil örnek kararı: erken çıkış yalnızca birincil kilitliyken.
//! - [`logind`] — `SetLockedHint` (bilgi amaçlı).
//! - [`view`] — çıkış başına görünüm ve durum satırları.

pub mod app;
pub mod assets;
pub mod auth;
pub mod cli;
pub mod config;
pub mod fingerprint;
pub mod i18n;
pub mod images;
pub mod instance;
pub mod logind;
pub mod state;
pub mod view;
