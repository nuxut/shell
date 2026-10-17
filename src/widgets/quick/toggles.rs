//! Hızlı anahtarlar kartı (`toggles`): tek dokunuşluk kutucuk ızgarası — Wi-Fi, Bluetooth,
//! Rahatsız etme, Uyanık tut, Gece ışığı, Koyu tema. Çizim ve girdi framework `TileGrid`'inde;
//! kart yalnızca modelden kutucuk tanımlarını ([`specs`]) çıkarır ve tıklamayı eyleme çevirir.
//! Yeri ve mevcut kartlarla ilişkisi: `docs/design/quick-toggles.md`.
//!
//! - Kullanılamayan anahtar gösterilmez (Wi-Fi cihazı yok, Bluetooth adaptörü yok, bildirim
//!   sunucusu başkasında, gece ışığı desteklenmiyor); hiçbiri yoksa kart da yok.
//! - Kutucuk basınca hemen döner (iyimser); gerçek durum bir sonraki model eşlemesinde gelir
//!   ([`PanelWidget::sync`] kutucuğun gösterdiğini modelle karşılaştırır, reddedilen istek geri döner).
//! - Koyu tema `framework.toml [colors] mode`'u yazar; durum temadan okunur (`cx.theme.mode`).

use std::time::Instant;

use nuxut::i18n::t;
use nuxut::theme::ThemeMode;
use nuxut::ui::{Icon, IconName, TileEvent, TileGrid, TileItem, Ui};
use nuxut::{CursorShape, Point, PointerEvent, Rect, Size, TextSystem};

use super::bluetooth::switch_subtitle;
use super::network::wifi_subtitle;
use super::night_light::{local_time, status_text};
use super::{column, column_inset, wifi_icon};
use crate::model::ShellModel;
use crate::services::night_light::Command as NightLightCommand;
use crate::system::SystemCommand;
use crate::widgets::{Action, NotifyCommand, PanelWidget, WidgetCx, WidgetKind};

/// Kutucuğun anahtarı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Switch {
    Wifi,
    Bluetooth,
    Dnd,
    Caffeine,
    NightLight,
    Dark,
}

impl Switch {
    /// Kutucuk yeni duruma geçti: uygulanacak eylem.
    fn action(self, on: bool) -> Action {
        match self {
            Switch::Wifi => Action::System(SystemCommand::SetWifi(on)),
            Switch::Bluetooth => Action::System(SystemCommand::SetBluetooth(on)),
            Switch::Dnd => Action::Notify(NotifyCommand::SetDnd(on)),
            Switch::Caffeine => Action::IdleInhibit(on),
            Switch::NightLight => Action::NightLight(NightLightCommand::Set(on)),
            Switch::Dark => Action::SetThemeMode(if on { ThemeMode::Dark } else { ThemeMode::Light }),
        }
    }
}

/// Bir kutucuğun görünür tanımı.
#[derive(Debug, Clone, PartialEq)]
struct Spec {
    switch: Switch,
    icon: IconName,
    label: String,
    subtitle: String,
    on: bool,
    enabled: bool,
}

fn on_off(on: bool, on_text: impl FnOnce() -> String) -> String {
    if on { on_text() } else { t!("shell-quick-state-off") }
}

/// Modelden kutucuklar (sıra: bağlantı, dikkat, ekran — ikişerli satırlar).
fn specs(model: &ShellModel, mode: ThemeMode) -> Vec<Spec> {
    let mut out = Vec::with_capacity(6);
    let n = &model.system.network;
    if n.available && !n.wifi_devices.is_empty() {
        let strength = n.active_access_point().map(|ap| ap.strength);
        out.push(Spec {
            switch: Switch::Wifi,
            icon: match strength {
                Some(s) if n.wifi_enabled => wifi_icon(s),
                _ if n.wifi_enabled => IconName::Wifi,
                _ => IconName::WifiOff,
            },
            label: "Wi-Fi".into(),
            subtitle: wifi_subtitle(n),
            on: n.wifi_enabled,
            enabled: n.wifi_hardware_enabled,
        });
    }
    let b = &model.system.bluetooth;
    if b.available && b.adapter().is_some() {
        out.push(Spec {
            switch: Switch::Bluetooth,
            icon: if b.powered() { IconName::Bluetooth } else { IconName::BluetoothOff },
            label: "Bluetooth".into(),
            subtitle: switch_subtitle(b),
            on: b.powered(),
            enabled: true,
        });
    }
    if model.notifications_active {
        let on = model.notifications.dnd();
        out.push(Spec {
            switch: Switch::Dnd,
            icon: if on { IconName::DoNotDisturb } else { IconName::NotificationsOff },
            label: t!("shell-quick-toggle-dnd"),
            subtitle: on_off(on, || t!("shell-quick-toggle-dnd-on")),
            on,
            enabled: true,
        });
    }
    out.push(Spec {
        switch: Switch::Caffeine,
        icon: IconName::Coffee,
        label: t!("shell-quick-toggle-caffeine"),
        subtitle: on_off(model.idle_inhibit, || t!("shell-quick-toggle-caffeine-on")),
        on: model.idle_inhibit,
        enabled: true,
    });
    let nl = &model.night_light;
    if nl.supported {
        out.push(Spec {
            switch: Switch::NightLight,
            icon: IconName::NightLight,
            label: t!("shell-quick-night-light-title"),
            subtitle: status_text(nl.status, local_time),
            on: nl.active,
            enabled: true,
        });
    }
    out.push(dark_spec(mode));
    out
}

fn dark_spec(mode: ThemeMode) -> Spec {
    let dark = mode == ThemeMode::Dark;
    Spec {
        switch: Switch::Dark,
        icon: if dark { IconName::DarkMode } else { IconName::LightMode },
        label: t!("shell-quick-toggle-dark"),
        subtitle: on_off(dark, || t!("shell-quick-toggle-on")),
        on: dark,
        enabled: true,
    }
}

fn item(s: &Spec) -> TileItem {
    TileItem::new(Icon::named(s.icon), s.label.clone()).subtitle(s.subtitle.clone()).on(s.on)
}

#[derive(Debug)]
pub struct TogglesCard {
    specs: Vec<Spec>,
    grid: TileGrid,
}

impl TogglesCard {
    pub fn new(cx: &WidgetCx) -> Self {
        Self { specs: Vec::new(), grid: TileGrid::new(cx.theme, Vec::new()) }
    }

    /// Tanımları ızgaraya uygular; görünür bir şey değiştiyse `true`.
    fn apply(&mut self, specs: Vec<Spec>, cx: &WidgetCx) -> bool {
        let same_tiles = specs.len() == self.specs.len()
            && specs.iter().zip(&self.specs).all(|(a, b)| a.switch == b.switch && a.label == b.label);
        if !same_tiles {
            let mut grid = TileGrid::new(cx.theme, specs.iter().map(item).collect());
            for (tile, s) in grid.tiles_mut().iter_mut().zip(&specs) {
                tile.set_enabled(s.enabled, cx.now);
            }
            self.grid = grid;
            self.specs = specs;
            return true;
        }
        let mut changed = false;
        for ((tile, new), old) in self.grid.tiles_mut().iter_mut().zip(&specs).zip(&self.specs) {
            // Gösterilenle karşılaştır: iyimser dönüş reddedildiyse geri gelir.
            if tile.is_on() != new.on {
                tile.set_on(new.on, cx.now);
                changed = true;
            }
            if new.subtitle != old.subtitle {
                tile.set_subtitle(cx.theme, Some(&new.subtitle));
                changed = true;
            }
            if new.icon != old.icon {
                tile.set_icon(Icon::named(new.icon));
                changed = true;
            }
            if new.enabled != old.enabled {
                tile.set_enabled(new.enabled, cx.now);
                changed = true;
            }
        }
        self.specs = specs;
        changed
    }

    fn place(&mut self, rect: Rect, cx: &WidgetCx) {
        let r = column(rect, cx.theme);
        self.grid.layout(cx.theme, Point::new(r.x, r.y), r.width);
    }
}

impl PanelWidget for TogglesCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Toggles
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        self.apply(specs(model, cx.theme.mode), cx)
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        self.grid.set_theme(cx.theme, cx.now);
        // Koyu tema kutucuğu temadan okunur: kip değiştiyse yalnızca o güncellenir.
        let mut specs = self.specs.clone();
        if let Some(s) = specs.iter_mut().find(|s| s.switch == Switch::Dark) {
            *s = dark_spec(cx.theme.mode);
        }
        self.apply(specs, cx);
    }

    fn height(&mut self, _text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        if self.specs.is_empty() {
            return Some(0.0);
        }
        let w = (width - 2.0 * column_inset(cx.theme)).max(0.0);
        Some(TileGrid::size_for(cx.theme, self.specs.len(), w).height)
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        self.place(rect, cx);
        self.grid.draw(ui);
    }

    fn pointer(&mut self, event: &PointerEvent, rect: Rect, _text: &mut TextSystem, cx: &WidgetCx) -> (bool, Option<Action>) {
        self.place(rect, cx);
        let r = self.grid.pointer(cx.theme, event, cx.now);
        let action = r.event.and_then(|(i, e)| match e {
            TileEvent::Toggled(on) => self.specs.get(i).map(|s| s.switch.action(on)),
            TileEvent::Expand => None,
        });
        (r.repaint || action.is_some(), action)
    }

    fn cursor(&self, _rect: Rect, p: Point, _cx: &WidgetCx) -> Option<CursorShape> {
        self.grid.cursor(p)
    }

    fn corner_clearance(&self, cx: &WidgetCx) -> Size {
        Size::new(column_inset(cx.theme), 0.0)
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        // Kutucuklar kendi geçişleri sürerken `ui.request_frame()` der.
        false
    }

    /// Kart adlı `Group`, kutucuklar `Switch` (basılı = açık; açıklama alt metin). Düğüm
    /// anahtarı anahtarın türü: kutucuk kümesi değişince AT odağı doğru kutucukta kalır.
    fn access(&self, tree: &mut nuxut::access::AccessTree, _rect: Rect, _cx: &WidgetCx) {
        if self.specs.is_empty() {
            return;
        }
        self.grid.access(tree, "toggles", |i| self.specs.get(i).map(|s| s.switch)).label(&self.kind().title());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::night_light::NightLightView;

    fn switches(model: &ShellModel, mode: ThemeMode) -> Vec<Switch> {
        specs(model, mode).into_iter().map(|s| s.switch).collect()
    }

    #[test]
    fn unavailable_switches_are_hidden() {
        crate::i18n::use_turkish_for_tests();
        let m = ShellModel::default();
        // Ağ/BT/bildirim sunucusu/gece ışığı yok: yalnızca her zaman var olanlar.
        assert_eq!(switches(&m, ThemeMode::Dark), [Switch::Caffeine, Switch::Dark]);

        let mut m = ShellModel { notifications_active: true, ..ShellModel::default() };
        m.night_light = NightLightView { supported: true, ..Default::default() };
        assert_eq!(switches(&m, ThemeMode::Dark), [Switch::Dnd, Switch::Caffeine, Switch::NightLight, Switch::Dark]);
    }

    #[test]
    fn states_and_subtitles_follow_model() {
        crate::i18n::use_turkish_for_tests();
        let mut m = ShellModel { idle_inhibit: true, ..ShellModel::default() };
        let s = specs(&m, ThemeMode::Light);
        let caffeine = s.iter().find(|s| s.switch == Switch::Caffeine).unwrap();
        assert!(caffeine.on);
        assert_eq!(caffeine.subtitle, "Ekran açık kalır");
        let dark = s.iter().find(|s| s.switch == Switch::Dark).unwrap();
        assert!(!dark.on);
        assert_eq!((dark.icon, dark.subtitle.as_str()), (IconName::LightMode, "Kapalı"));
        m.idle_inhibit = false;
        let s = specs(&m, ThemeMode::Dark);
        assert_eq!(s.iter().find(|s| s.switch == Switch::Caffeine).unwrap().subtitle, "Kapalı");
        assert!(s.iter().find(|s| s.switch == Switch::Dark).unwrap().on);
    }

    #[test]
    fn switches_map_to_actions() {
        assert_eq!(Switch::Caffeine.action(true), Action::IdleInhibit(true));
        assert_eq!(Switch::Dark.action(false), Action::SetThemeMode(ThemeMode::Light));
        assert_eq!(Switch::Dnd.action(true), Action::Notify(NotifyCommand::SetDnd(true)));
        assert_eq!(Switch::Wifi.action(false), Action::System(SystemCommand::SetWifi(false)));
        assert_eq!(Switch::Bluetooth.action(true), Action::System(SystemCommand::SetBluetooth(true)));
        assert_eq!(Switch::NightLight.action(true), Action::NightLight(NightLightCommand::Set(true)));
    }

    #[test]
    fn card_rebuilds_only_when_switch_set_changes_and_reverts_rejected_toggle() {
        crate::i18n::use_turkish_for_tests();
        let theme = nuxut::Theme::default();
        let settings = crate::widgets::WidgetSettings::default();
        let now = Instant::now();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now, bounds: Rect::ZERO };
        let mut card = TogglesCard::new(&cx);
        let m = ShellModel::default();
        assert!(card.sync(&m, &cx));
        assert_eq!(card.grid.tiles().len(), 2);
        assert!(!card.sync(&m, &cx), "aynı model: değişiklik yok");

        // Kullanıcı uyanık tutu açtı (iyimser), servis reddetti: model aynı → kutucuk geri döner.
        card.grid.tiles_mut()[0].set_on(true, now);
        assert!(card.sync(&m, &cx));
        assert!(!card.grid.tiles()[0].is_on());

        let m2 = ShellModel { notifications_active: true, ..ShellModel::default() };
        assert!(card.sync(&m2, &cx));
        assert_eq!(card.grid.tiles().len(), 3);
    }

    #[test]
    fn access_tree_lists_switches_with_state() {
        use nuxut::access::{AccessTree, Role, Toggled};
        crate::i18n::use_turkish_for_tests();
        let theme = nuxut::Theme::default();
        let settings = crate::widgets::WidgetSettings::default();
        let now = Instant::now();
        let cx = WidgetCx { theme: &theme, settings: &settings, vertical: false, item: 26.0, now, bounds: Rect::ZERO };
        let mut card = TogglesCard::new(&cx);
        let mut tree = AccessTree::new();
        card.access(&mut tree, Rect::ZERO, &cx);
        assert!(tree.is_empty(), "kart boşken ağaçta yok");

        let m = ShellModel { idle_inhibit: true, ..ShellModel::default() };
        card.sync(&m, &cx);
        card.place(Rect::new(0.0, 0.0, 400.0, 200.0), &cx);
        let mut tree = AccessTree::new();
        card.access(&mut tree, Rect::ZERO, &cx);
        let switches: Vec<_> = tree.nodes().filter(|(_, n)| n.role() == Role::Switch).map(|(_, n)| n).collect();
        assert_eq!(switches.len(), 2);
        let caffeine = switches.iter().find(|n| n.label() == Some("Uyanık tut")).expect("uyanık tut kutucuğu");
        assert_eq!(caffeine.toggled(), Some(Toggled::True));
        assert_eq!(caffeine.description(), Some("Ekran açık kalır"));
        assert!(caffeine.bounds().is_some_and(|b| b.width() > 0.0));
        assert!(tree.unlabeled().is_empty());
    }
}
