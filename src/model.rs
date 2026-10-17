//! Shell durumu ve ondan türetilen, çizime hazır görünümler.
//!
//! Model yalnızca veri tutar; olaylar [`ShellModel::apply_hyprland`] ile işlenir. Hata
//! yalıtımı: bağlantı koparsa son geçerli durum korunur, yalnızca bayat olarak işaretlenir.

use std::time::Instant;

use nuxut_calendar_ipc::OccurrenceEntry;

use crate::config::ShellConfig;
use crate::services::calendar::CalendarEvent;
use crate::services::hyprland::{HyprSnapshot, HyprlandEvent, Window};
use crate::services::notifications::{NotificationCenter, NotifyEvent};
use crate::services::session::{Capability, FailureReason, SessionAction, SessionEvent, UserInfo};
use crate::services::tray::{TrayEvent, TrayModel};
use crate::system::{SystemModel, system_options};

#[derive(Debug, Clone)]
pub struct ShellModel {
    pub config: ShellConfig,
    pub hypr: HyprSnapshot,
    /// socket2 bağlı mı; değilse `hypr` son bilinen (bayat olabilecek) durumdur.
    pub hypr_connected: bool,
    /// Popup'lar ve geçmiş. Sunucu yoksa (ad başkasında) boş kalır.
    pub notifications: NotificationCenter,
    /// Bildirim sunucusu çalışıyor mu.
    pub notifications_active: bool,
    /// Okunmamış bildirim sayısı (bar sayacı); `notifications`'tan türetilir.
    pub notifications_unread: u32,
    pub tray: TrayModel,
    pub user: UserInfo,
    /// logind yetenekleri; gelene kadar boş (düğmeler etkin, logind reddederse hata gösterilir).
    pub session_caps: Vec<(SessionAction, Capability)>,
    /// Son başarısız oturum eylemi ve nedeni (oturum kartında gösterilir).
    pub session_error: Option<(SessionAction, FailureReason)>,
    /// Takvimde gösterilen aralığın oluşumları (`nuxut-calendard`).
    pub calendar: Vec<OccurrenceEntry>,
    /// Takvim daemon'u cevap veriyor mu.
    pub calendar_available: bool,
    /// Ses, ağ, Bluetooth, güç, parlaklık, medya (hızlı ayar kartları).
    pub system: SystemModel,
    /// Başlatıcı kataloğu ve kullanım kayıtları (servis thread'inden `Arc`).
    pub launcher_catalog: std::sync::Arc<crate::services::launcher::Catalog>,
    pub launcher_usage: std::sync::Arc<crate::services::launcher::Usage>,
    /// Son sistem izleyici ölçümü (`services::sysmon`; servis yoksa `None`).
    pub sysmon: Option<std::sync::Arc<crate::services::sysmon::Snapshot>>,
    /// Pano geçmişi (`services::clipboard`; servis thread'inden `Arc`, öğeler paylaşılır).
    pub clipboard: std::sync::Arc<crate::services::clipboard::ClipboardSnapshot>,
    /// Gece ışığı görünümü (`services::night_light`; destek yoksa `supported = false`).
    pub night_light: crate::services::night_light::NightLightView,
    /// Büyüteç görünümü (`services::magnifier`; Hyprland'e komut gönderilemiyorsa `available = false`).
    pub magnifier: crate::services::magnifier::MagnifierView,
    /// Uyanık tut: elle tutulan boşta kalma engeli (`msg idle inhibit`, hızlı anahtar, bar ikonu).
    pub idle_inhibit: bool,
    /// Ekran klavyesi görünümü (`services::osk`; sanal klavye protokolü yoksa `available = false`).
    pub osk: crate::services::osk::OskView,
    /// Kamerayı açık tutan süreçlerin adları (`services::camera`; yalnız gizlilik widget'ı
    /// kamerayı gösterirken izlenir, yoksa boş).
    pub camera: crate::services::camera::CameraState,
    /// Ekranı sürekli yakalayan istemciler (Hyprland `screencast`; paylaşım ya da kayıt).
    pub screencast: Screencast,
    /// Ana klavyenin düzenleri (Hyprland `j/devices`; bar düzen göstergesi). Bilinmiyorsa `None`.
    pub keyboard: Option<crate::services::keyboard_layouts::KeyboardLayouts>,
}

impl Default for ShellModel {
    fn default() -> Self {
        Self::new(ShellConfig::default())
    }
}

impl ShellModel {
    pub fn new(config: ShellConfig) -> Self {
        let notifications = NotificationCenter::new(config.notifications.clone());
        let system = SystemModel::new(&system_options(&config));
        Self {
            config,
            hypr: HyprSnapshot::default(),
            hypr_connected: false,
            notifications,
            notifications_active: false,
            notifications_unread: 0,
            tray: TrayModel::default(),
            user: UserInfo { login: String::new(), real_name: None, avatar: None },
            session_caps: Vec::new(),
            session_error: None,
            calendar: Vec::new(),
            calendar_available: false,
            system,
            launcher_catalog: Default::default(),
            launcher_usage: Default::default(),
            sysmon: None,
            clipboard: Default::default(),
            night_light: Default::default(),
            magnifier: Default::default(),
            idle_inhibit: false,
            osk: Default::default(),
            camera: Default::default(),
            screencast: Screencast::default(),
            keyboard: None,
        }
    }

    /// Takvim olayı. Değiştiyse `true`.
    pub fn apply_calendar(&mut self, event: CalendarEvent) -> bool {
        match event {
            CalendarEvent::Occurrences { entries, .. } => {
                let changed = !self.calendar_available || self.calendar != entries;
                self.calendar = entries;
                self.calendar_available = true;
                changed
            }
            CalendarEvent::Unavailable => {
                let changed = self.calendar_available || !self.calendar.is_empty();
                self.calendar.clear();
                self.calendar_available = false;
                changed
            }
        }
    }

    /// Bildirim olayını merkeze uygular. Görünür durum değiştiyse `true`.
    pub fn apply_notify(&mut self, event: NotifyEvent, now: Instant) -> bool {
        let changed = self.notifications.on_event(event, now);
        self.refresh_unread();
        changed
    }

    pub fn refresh_unread(&mut self) {
        self.notifications_unread = u32::try_from(self.notifications.unread_count()).unwrap_or(u32::MAX);
    }

    pub fn apply_tray(&mut self, event: &TrayEvent) -> bool {
        self.tray.apply(event)
    }

    /// Oturum olayı. Değiştiyse `true`.
    pub fn apply_session(&mut self, event: SessionEvent) -> bool {
        match event {
            SessionEvent::Capabilities(caps) => {
                self.session_caps = caps;
                true
            }
            SessionEvent::Done(_) => self.session_error.take().is_some(),
            SessionEvent::Failed { action, reason } => {
                self.session_error = Some((action, reason));
                true
            }
        }
    }

    /// Eylemin düğmesi etkin mi (yetenek bilinmiyorsa evet).
    pub fn session_allowed(&self, action: SessionAction) -> bool {
        self.session_caps.iter().find(|(a, _)| *a == action).is_none_or(|(_, c)| c.allowed())
    }

    /// Hyprland olayını uygular. Görünür durum değiştiyse `true`.
    pub fn apply_hyprland(&mut self, event: HyprlandEvent) -> bool {
        match event {
            HyprlandEvent::Snapshot(snapshot) => {
                let changed = self.hypr != snapshot || !self.hypr_connected;
                self.hypr = snapshot;
                self.hypr_connected = true;
                changed
            }
            HyprlandEvent::Disconnected => {
                // Yeniden bağlanan Hyprland (belki yeni örnek) yakalamaları yeniden bildirir.
                // Kopukken gelen bitiş olayları kaçmış olabilir: sayaç SIFIRLANMAZ (fail-closed),
                // gösterge "doğrulanamadı" olarak açık kalır (`docs/design/privacy-indicator.md`).
                let uncertain = self.screencast.disconnected();
                std::mem::replace(&mut self.hypr_connected, false) || uncertain
            }
            HyprlandEvent::Screencast(on) => self.screencast.apply(on, chrono::Local::now()),
            // Görünür durumu değiştirmez (durum yenilemesi ayrıca gelir); büyüteç dinler.
            HyprlandEvent::ConfigReloaded => false,
            // Ekran klavyesi dinler (`app/osk.rs`).
            HyprlandEvent::ActiveLayout { .. } => false,
            HyprlandEvent::Keyboard(k) => std::mem::replace(&mut self.keyboard, k.clone()) != k,
        }
    }

    /// Monitörün aktif workspace'inde gerçek tam ekran pencere var mı (çerçeve gizlenir).
    /// Maximize (kip 1) sayılmaz: pencere rezerve alana saygılıdır, bar görünür kalmalı.
    pub fn is_fullscreen(&self, monitor: &str) -> bool {
        let Some(m) = self.hypr.monitor(monitor) else { return false };
        self.hypr.workspace(m.active_workspace.id).is_some_and(|w| w.is_fullscreen())
    }

    /// Odaklı pencere; yalnızca bu monitörün aktif ya da special workspace'indeyse.
    pub fn focused_window(&self, monitor: &str) -> Option<&Window> {
        let m = self.hypr.monitor(monitor)?;
        let w = self.hypr.active_window.as_ref()?;
        let on_monitor = w.workspace.id == m.active_workspace.id || m.special().is_some_and(|s| s.id == w.workspace.id);
        on_monitor.then_some(w)
    }
}

/// Ekran yakalama durumu: Hyprland `screencast` olayları istemci başına başla/bitir çifti
/// olduğundan sayılır. Kabuk başlamadan süren yakalama bilinmez (Hyprland sorgu sunmuyor).
/// **Fail-closed:** socket2 koparsa (Hyprland kuyruk taşmasında istemciyi düşürür) aradaki olaylar
/// kaybolur; sayaç korunur ve `uncertain` işaretlenir — gösterge sönmez, ancak sayaç olaylarla 0'a
/// inince söner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Screencast {
    /// Yakalayan istemci sayısı.
    pub clients: u32,
    /// İlk istemcinin başladığı an (sayaç 0'dan çıkınca).
    pub since: Option<chrono::DateTime<chrono::Local>>,
    /// Sayaç bir kopuşu atlattı: bitiş olayları kaçmış olabilir (ipucunda belirtilir).
    pub uncertain: bool,
}

impl Screencast {
    pub fn active(&self) -> bool {
        self.clients > 0
    }

    /// Olayı uygular; görünür durum (etkin mi) değiştiyse `true`.
    pub fn apply(&mut self, on: bool, now: chrono::DateTime<chrono::Local>) -> bool {
        let was = self.active();
        // Bozuk/çift olaya karşı üst sınır; fazladan `0` sayacı eksiye düşürmez.
        self.clients = if on { self.clients.saturating_add(1).min(64) } else { self.clients.saturating_sub(1) };
        match (was, self.active()) {
            (false, true) => self.since = Some(now),
            (true, false) => {
                self.since = None;
                self.uncertain = false;
            }
            _ => {}
        }
        was != self.active()
    }

    /// socket2 koptu. Görünür durum değiştiyse (etkin yakalama artık doğrulanamıyor) `true`.
    pub fn disconnected(&mut self) -> bool {
        self.active() && !std::mem::replace(&mut self.uncertain, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screencast_counts_clients_and_fails_closed_on_disconnect() {
        let mut m = ShellModel::default();
        assert!(m.apply_hyprland(HyprlandEvent::Screencast(true)));
        assert!(m.screencast.active() && m.screencast.since.is_some());
        let since = m.screencast.since;
        assert!(!m.apply_hyprland(HyprlandEvent::Screencast(true)), "ikinci istemci: görünür değişim yok");
        assert!(!m.apply_hyprland(HyprlandEvent::Screencast(false)));
        assert_eq!(m.screencast.since, since, "ilk başlangıç korunur");
        assert!(m.apply_hyprland(HyprlandEvent::Screencast(false)));
        assert!(!m.apply_hyprland(HyprlandEvent::Screencast(false)), "fazladan bitiş eksiye düşmez");
        assert_eq!(m.screencast, Screencast::default());
        // İki eşzamanlı paylaşım: biri bitince gösterge yanık kalır.
        m.apply_hyprland(HyprlandEvent::Screencast(true));
        m.apply_hyprland(HyprlandEvent::Screencast(true));
        assert!(!m.apply_hyprland(HyprlandEvent::Screencast(false)));
        assert!(m.screencast.active());
        // Kopuş: sayaç sıfırlanmaz, doğrulanamadı işaretlenir (fail-closed).
        assert!(m.apply_hyprland(HyprlandEvent::Disconnected));
        assert!(m.screencast.active() && m.screencast.uncertain);
        assert!(!m.apply_hyprland(HyprlandEvent::Disconnected), "ikinci kopuş değişim değil");
        // Yeniden bağlanınca gelen bitiş sayacı 0'a indirir: söner, belirsizlik kalkar.
        assert!(m.apply_hyprland(HyprlandEvent::Screencast(false)));
        assert_eq!(m.screencast, Screencast::default());
        // Yakalama yokken kopuş belirsizlik üretmez.
        m.apply_hyprland(HyprlandEvent::Disconnected);
        assert!(!m.screencast.uncertain);
    }
    use crate::services::hyprland::{Monitor, Workspace, WorkspaceRef};

    fn snapshot() -> HyprSnapshot {
        let monitors: Vec<Monitor> =
            serde_json::from_str(include_str!("../tests/fixtures/hyprland/monitors.json")).unwrap();
        let mut workspaces: Vec<Workspace> =
            serde_json::from_str(include_str!("../tests/fixtures/hyprland/workspaces.json")).unwrap();
        workspaces.sort_by_key(|w| w.id);
        let active_window = serde_json::from_str(include_str!("../tests/fixtures/hyprland/activewindow.json")).unwrap();
        HyprSnapshot { monitors, workspaces, active_window: Some(active_window) }
    }

    #[test]
    fn apply_tracks_connection_and_changes() {
        let mut m = ShellModel::default();
        assert!(m.apply_hyprland(HyprlandEvent::Snapshot(snapshot())));
        assert!(!m.apply_hyprland(HyprlandEvent::Snapshot(snapshot())));
        assert!(m.apply_hyprland(HyprlandEvent::Disconnected));
        assert!(!m.apply_hyprland(HyprlandEvent::Disconnected));
        // Kopukken son durum korunur.
        assert_eq!(m.hypr.monitors.len(), 1);
        assert!(m.apply_hyprland(HyprlandEvent::Snapshot(snapshot())));
    }

    #[test]
    fn fullscreen_and_focused_window() {
        let mut m = ShellModel::default();
        m.apply_hyprland(HyprlandEvent::Snapshot(snapshot()));
        assert!(!m.is_fullscreen("Virtual-1"));
        assert_eq!(m.focused_window("Virtual-1").unwrap().class, "code-oss");
        assert!(m.focused_window("HDMI-A-9").is_none());

        let mut s = snapshot();
        s.monitors[0].active_workspace = WorkspaceRef { id: 3, name: "3".into() };
        // Fixture'daki workspace 3 penceresi maximize (kip 1): çerçeve kalır.
        s.workspaces.iter_mut().find(|w| w.id == 3).unwrap().fullscreen_mode = 1;
        m.apply_hyprland(HyprlandEvent::Snapshot(s.clone()));
        assert!(!m.is_fullscreen("Virtual-1"));
        s.workspaces.iter_mut().find(|w| w.id == 3).unwrap().fullscreen_mode = 2;
        m.apply_hyprland(HyprlandEvent::Snapshot(s));
        assert!(m.is_fullscreen("Virtual-1"));
        // Pencere başka workspace'te → bu monitörde odaklı pencere gösterilmez.
        assert!(m.focused_window("Virtual-1").is_none());
    }
}
