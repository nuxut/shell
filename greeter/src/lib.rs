//! `nuxut-greeter`: Nuxut giriş ekranı, greetd istemcisi. Karar: `docs/adr/0010-login-screen.md`;
//! tasarım ve tehdit modeli: `docs/design/greeter.md`.
//!
//! - [`protocol`] — greetd IPC çerçeveleri ve iletileri (boyut sınırlı, şifre sıfırlanan tamponda).
//! - [`client`] — soket thread'i; kopan bağlantı yeniden kurulur.
//! - [`flow`] — saf durum makinesi (istek-yanıt sırası, kullanıcı değişimi, gecikme).
//! - [`sessions`] — `wayland-sessions/*.desktop`.
//! - [`users`] — `/etc/passwd` + `login.defs`.
//! - [`remember`] — son kullanıcı ve oturum.
//! - [`power`] — logind kapat/yeniden başlat.
//! - [`keyboard`] — klavye düzeni göstergesi ve geçişi.
//! - [`view`], [`app`] — kilit ekranının görünümü üstünde form.

pub mod app;
pub mod cli;
pub mod client;
pub mod flow;
pub mod i18n;
pub mod keyboard;
pub mod power;
pub mod protocol;
pub mod remember;
pub mod sessions;
pub mod users;
pub mod view;
