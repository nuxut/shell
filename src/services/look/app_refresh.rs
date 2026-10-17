//! Renkleri canlı yenilemeyen GTK4 uygulamaları (2026-10-17; docs/design/look-gtk.md "Canlı
//! yenilenmeyen uygulamalar"). libadwaita `~/.config/gtk-4.0/gtk.css`'i yalnız açılışta okur;
//! Nautilus gibi `--gapplication-service` olarak arka planda kalan uygulama son pencere kapansa da
//! eski renkleri tutar (gerçek Nautilus 51 ile ölçüldü). Kullanıcıya komut (`nautilus -q`)
//! yazdırılmaz; renkler değişince her uygulama için:
//!
//! - Çalışmıyorsa: iş yok.
//! - Çalışıyor ama açık penceresi yoksa: yalnız arka plan süreci GApplication'ın kendi `quit`
//!   eylemiyle (`org.gtk.Actions.Activate`) kapatılır; bir sonraki açılış yeni renklerle gelir.
//!   Kullanıcının penceresine dokunulmaz, süreç öldürülmez.
//! - Açık penceresi varsa (ya da pencereler bilinemiyorsa): bildirim, "Şimdi yeniden başlat"
//!   düğmesi (`msg look restart-app <kimlik>`): `quit`, ad bırakılınca `org.freedesktop.Application
//!   .Activate` ile D-Bus etkinleştirmesi (yeni pencere).
//!
//! Liste [`App::ALL`]: yenisi buraya bir satırla eklenir (bus adı, nesne yolu, pencere sınıfı).
//! D-Bus ve Hyprland IO'su bloklar: yalnız arka plan thread'lerinde çağrılır.

use std::time::{Duration, Instant};

use serde::Deserialize;

const DBUS_TIMEOUT: Duration = Duration::from_secs(3);
/// `quit`'ten sonra adın bırakılması için en çok bekleme.
const RELEASE_WAIT: Duration = Duration::from_secs(20);
/// `j/clients` yanıt sınırı (bayt).
const MAX_CLIENTS_JSON: usize = 4 * 1024 * 1024;

/// Canlı yenilenmeyen uygulama.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    Nautilus,
}

impl App {
    pub const ALL: [App; 1] = [App::Nautilus];

    /// `msg look restart-app <kimlik>`.
    pub fn id(self) -> &'static str {
        match self {
            App::Nautilus => "nautilus",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == s)
    }

    /// GApplication kimliği = D-Bus adı.
    pub fn bus_name(self) -> &'static str {
        match self {
            App::Nautilus => "org.gnome.Nautilus",
        }
    }

    pub fn object_path(self) -> &'static str {
        match self {
            App::Nautilus => "/org/gnome/Nautilus",
        }
    }

    /// Hyprland pencere sınıfı (`class`/`initialClass`).
    pub fn window_class(self) -> &'static str {
        match self {
            App::Nautilus => "org.gnome.Nautilus",
        }
    }

}

/// Renk değişiminden sonra bir uygulama için yapılacak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Çalışmıyor: iş yok.
    Nothing,
    /// Yalnız arka plan süreci: sessizce `quit`.
    QuitBackground,
    /// Açık pencere var ya da bilinmiyor: bildirim.
    Ask,
}

/// Saf karar: `windows` = `None` pencereler bilinemedi (Hyprland yok/yanıt yok).
pub fn decide(running: bool, windows: Option<usize>) -> Step {
    match (running, windows) {
        (false, _) => Step::Nothing,
        (true, Some(0)) => Step::QuitBackground,
        (true, _) => Step::Ask,
    }
}

#[derive(Deserialize)]
struct Client {
    #[serde(default)]
    class: String,
    #[serde(default, rename = "initialClass")]
    initial_class: String,
}

/// `j/clients` yanıtında sınıfı `class` olan pencere sayısı; bozuk/aşırı büyük yanıt → `None`.
pub fn count_windows(clients_json: &str, class: &str) -> Option<usize> {
    if clients_json.len() > MAX_CLIENTS_JSON {
        return None;
    }
    let list: Vec<Client> = serde_json::from_str(clients_json).ok()?;
    Some(list.iter().filter(|c| c.class == class || c.initial_class == class).count())
}

fn bus() -> Result<zbus::blocking::Connection, String> {
    zbus::blocking::connection::Builder::session()
        .and_then(|b| b.method_timeout(DBUS_TIMEOUT).build())
        .map_err(|e| e.to_string())
}

fn has_owner(conn: &zbus::blocking::Connection, name: &str) -> bool {
    conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(name,))
        .ok()
        .and_then(|m| m.body().deserialize::<bool>().ok())
        .unwrap_or(false)
}

/// GApplication'ın `quit` eylemi (`nautilus -q` ile aynı yol).
fn quit(conn: &zbus::blocking::Connection, app: App) -> Result<(), String> {
    let params: Vec<zbus::zvariant::Value<'_>> = Vec::new();
    let platform: std::collections::HashMap<&str, zbus::zvariant::Value<'_>> = std::collections::HashMap::new();
    conn.call_method(Some(app.bus_name()), app.object_path(), Some("org.gtk.Actions"), "Activate", &("quit", params, platform))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Renkler değişti: her uygulama için karar ve sessiz kısım (arka plan süreci kapatma). Bildirim
/// gerekenler döner. `clients`: Hyprland `j/clients` yanıtı (yoksa `None`).
pub fn after_colors_changed(clients: Option<&str>) -> Vec<App> {
    let Ok(conn) = bus() else { return Vec::new() };
    let mut ask = Vec::new();
    for app in App::ALL {
        let windows = clients.and_then(|c| count_windows(c, app.window_class()));
        match decide(has_owner(&conn, app.bus_name()), windows) {
            Step::Nothing => {}
            Step::QuitBackground => match quit(&conn, app) {
                Ok(()) => tracing::info!(app = app.id(), "görünüm: arka plandaki uygulama yeni renkler için kapatıldı"),
                Err(e) => tracing::warn!(app = app.id(), "görünüm: arka plan süreci kapatılamadı: {e}"),
            },
            Step::Ask => ask.push(app),
        }
    }
    ask
}

/// "Şimdi yeniden başlat": `quit`, ad bırakılınca D-Bus etkinleştirmesiyle yeni pencere.
pub fn restart(app: App) -> Result<(), String> {
    let conn = bus()?;
    if has_owner(&conn, app.bus_name()) {
        quit(&conn, app)?;
        let start = Instant::now();
        while has_owner(&conn, app.bus_name()) {
            if start.elapsed() > RELEASE_WAIT {
                return Err("uygulama kapanmadı".into()); // i18n: tanı günlüğe gider
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let platform: std::collections::HashMap<&str, zbus::zvariant::Value<'_>> = std::collections::HashMap::new();
    conn.call_method(Some(app.bus_name()), app.object_path(), Some("org.freedesktop.Application"), "Activate", &(platform,))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_table() {
        assert_eq!(decide(false, Some(0)), Step::Nothing);
        assert_eq!(decide(false, None), Step::Nothing);
        assert_eq!(decide(true, Some(0)), Step::QuitBackground, "yalnız arka plan süreci");
        assert_eq!(decide(true, Some(2)), Step::Ask, "açık pencereye dokunulmaz");
        assert_eq!(decide(true, None), Step::Ask, "bilinmiyorsa sorulur");
    }

    #[test]
    fn windows_by_class() {
        let json = r#"[{"class":"org.gnome.Nautilus","initialClass":"org.gnome.Nautilus"},{"class":"kitty"},{"class":"","initialClass":"org.gnome.Nautilus"},{"x":1}]"#;
        assert_eq!(count_windows(json, "org.gnome.Nautilus"), Some(2));
        assert_eq!(count_windows("[]", "org.gnome.Nautilus"), Some(0));
        assert_eq!(count_windows("çöp", "org.gnome.Nautilus"), None);
    }

    #[test]
    fn ids_round_trip() {
        for a in App::ALL {
            assert_eq!(App::parse(a.id()), Some(a));
            assert!(a.object_path().starts_with('/') && a.bus_name().contains('.'));
        }
        assert_eq!(App::parse("firefox"), None);
    }
}
