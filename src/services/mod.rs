//! Shell servisleri: UI'dan bağımsız, bloklayan IO kendi thread'lerinde, sonuçları bir
//! callback ile dışarı verir (`App::Message`'a `MessageSender` ile bağlanır).
//!
//! Sözleşme (tüm servisler): `spawn(..., sink: impl Fn(Event) + Send + 'static)`; düşürülünce
//! temiz kapanır (thread'ler join edilir); dış girdi boyut sınırlı; panik yok; servis
//! thread'leri `nuxut::run` başladıktan sonra (olay işleyicilerinden) başlatılır ki sinyal
//! maskesini devralsın.

pub mod apply;
pub mod audio;
pub mod bluetooth;
pub mod brightness;
pub mod calendar;
pub mod camera;
pub mod clipboard;
pub mod default_apps;
pub mod clock;
pub mod color_mode;
pub mod displays;
pub mod file_chooser;
pub mod hypr_config;
pub mod hyprland;
pub mod idle;
pub mod image_sandbox;
pub mod launcher;
pub mod lock_launcher;
pub mod login_screen;
pub mod logout;
pub mod keyboard_layouts;
pub mod magnifier;
pub mod osk;
pub mod look;
pub mod media;
pub mod network;
pub mod night_light;
pub mod notifications;
pub mod portal_env;
pub mod power;
pub mod removable_media;
pub mod safe_file;
pub mod screenshot;
pub mod session;
pub mod setup;
pub mod sharepicker;
pub mod shell_state;
pub mod sysmon;
pub mod system_bundle;
pub mod system_theme;
pub mod timezone;
pub mod tray;
