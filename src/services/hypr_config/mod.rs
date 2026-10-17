//! Ayarlar ↔ Hyprland ortaklığı (tasarım: `docs/design/hyprland-sync.md`).
//!
//! - [`parse`]: Hyprland ayar zincirini (source, joker, `$değişken`, yorum) sınırlı tarar.
//! - [`ensure`]: gerçek ayar yolu, `source` satırı kararı ve satır metinleri.
//! - [`generate`]: tema/shell.toml → `nuxut.conf` metni + canlı `keyword` listesi.
//! - [`base`]: temel katman (ADR-0006): kısayollar, pencere/katman kuralları, varsayılanlar; kullanıcı dosyasının başında.
//! - [`keybinds`]: kısayol eylemleri, tuş dizgesi, çakışma.
//! - [`control`]: canlı uygulama planı ve soyut Hyprland erişimi.
//! - [`settings`]: `[compositor]`, `[windows]`, `[keybinds]` şema + doğrulama.
//! - [`input`]: `[input]` (klavye, fare, dokunmatik; yalnızca yazılı anahtarlar) ve `msg input status`.
//! - [`lua`]: Lua arka ucu (Hyprland ≥ 0.55): üretim, kaçış, `hyprland.lua` taraması.
//! - [`takeover`]: ana ayarın devralınması (benzersiz yedek, doğrulama, geri alma; ADR-0007).
//! - [`sync`]: tek eşitleme turu (yazımlar `nuxut-apply`, `hypr.source` snapshot'ı) ve ortam güvenlik kapısı (`app/hypr_sync.rs` thread'inde).

pub mod control;
pub mod base;
pub mod ensure;
pub mod generate;
pub mod input;
pub mod lua;
pub mod keybinds;
pub mod parse;
pub mod settings;
pub mod sync;
pub mod takeover;
