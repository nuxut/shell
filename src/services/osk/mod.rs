//! Ekran klavyesi (t125): UI'dan bağımsız çekirdek.
//!
//! - [`keys`]: fiziksel tuş tablosu (evdev kodları, genişlikler).
//! - [`keymap`]: gömülü xkb keymap'leri (TR-Q, US) ve onlardan çıkarılan tuş etiketleri.
//! - [`Osk`]: değiştirici durum makinesi. Dokunuşları sanal klavye çağrılarına ([`Emit`])
//!   çevirir; Wayland'e kendisi dokunmaz (`app/osk.rs` gönderir), bu yüzden birim test edilir.
//!
//! Girdi `zwp_virtual_keyboard_v1` ile gider: kabuk kendi keymap'ini gönderir, tuşlar o keymap'in
//! evdev kodlarıdır. Ekrandaki etiket aynı keymap'ten çıktığı için istemcinin ürettiği karakter
//! her zaman etiketle aynıdır (kullanıcının fiziksel düzeninden bağımsız). Tasarım:
//! `docs/design/osk.md`.

pub mod keymap;
pub mod keys;
pub mod place;

use nuxut::i18n::t;
use serde::Deserialize;

use crate::schema::SettingSchema;
pub use keymap::{KeymapError, Labels, Layout, Level};
pub use keys::{KeyAction, KeySpec, Special};

// ---------------------------------------------------------------------------------------
// Ayarlar

/// `[osk] layout`: klavyenin açılıştaki düzeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutSetting {
    /// Hyprland'in etkin düzenini izler (Türkçe Q → TR, diğerleri → US).
    #[default]
    Auto,
    Tr,
    Us,
}

impl LayoutSetting {
    pub const ALL: [LayoutSetting; 3] = [LayoutSetting::Auto, LayoutSetting::Tr, LayoutSetting::Us];

    /// `shell.toml`'daki adı (serde adıyla aynı; test doğrular).
    pub fn name(self) -> &'static str {
        match self {
            LayoutSetting::Auto => "auto",
            LayoutSetting::Tr => "tr",
            LayoutSetting::Us => "us",
        }
    }

    pub fn label(self) -> String {
        match self {
            LayoutSetting::Auto => t!("shell-schema-osk-layout-auto"),
            LayoutSetting::Tr => t!("shell-schema-osk-layout-tr"),
            LayoutSetting::Us => t!("shell-schema-osk-layout-us"),
        }
    }

    /// Sabit düzen (`auto` için `None`).
    pub fn fixed(self) -> Option<Layout> {
        match self {
            LayoutSetting::Auto => None,
            LayoutSetting::Tr => Some(Layout::Tr),
            LayoutSetting::Us => Some(Layout::Us),
        }
    }
}

/// Tuş yüksekliği sınırı (mantıksal px).
pub const KEY_HEIGHT: std::ops::RangeInclusive<u32> = 32..=72;

/// `[osk]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct OskConfig {
    pub layout: LayoutSetting,
    /// Tuş yüksekliği ([`KEY_HEIGHT`]); klavyenin boyu ve genişliği bundan türer.
    pub key_height: u32,
    /// Yanaşıkken pencereler için yer ayır (exclusive zone). Varsayılan kapalı: klavye overlay'dir,
    /// pencereleri itmez (kullanıcı isteği, 2026-10-16); açıkken eski davranış.
    pub reserve_space: bool,
}

impl Default for OskConfig {
    fn default() -> Self {
        Self { layout: LayoutSetting::Auto, key_height: 48, reserve_space: false }
    }
}

impl OskConfig {
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(
            SettingSchema::choice(
                "osk.layout",
                t!("shell-config-osk-layout-label"),
                d.layout.name(),
                LayoutSetting::ALL.map(LayoutSetting::name).to_vec(),
            )
            .labels(LayoutSetting::ALL.map(LayoutSetting::label).to_vec())
            .describe(t!("shell-config-osk-layout-describe")),
        );
        out.push(
            SettingSchema::int(
                "osk.key_height",
                t!("shell-config-osk-key-height-label"),
                d.key_height.into(),
                (*KEY_HEIGHT.start()).into(),
                (*KEY_HEIGHT.end()).into(),
            )
            .unit("px"),
        );
        out.push(
            SettingSchema::bool("osk.reserve_space", t!("shell-config-osk-reserve-space-label"), d.reserve_space)
                .describe(t!("shell-config-osk-reserve-space-describe")),
        );
    }

    /// Aralık dışını kırpar; her düzeltme için uyarı. İdempotent.
    pub fn validate(&mut self, w: &mut Vec<String>) {
        if !KEY_HEIGHT.contains(&self.key_height) {
            let fixed = self.key_height.clamp(*KEY_HEIGHT.start(), *KEY_HEIGHT.end());
            w.push(t!(
                "shell-config-osk-out-of-range",
                key = "osk.key_height",
                value = self.key_height.to_string(),
                fixed = fixed.to_string(),
                range = format!("{}..={}", KEY_HEIGHT.start(), KEY_HEIGHT.end())
            ));
            self.key_height = fixed;
        }
    }
}

// ---------------------------------------------------------------------------------------
// Komutlar ve görünüm

/// `msg osk …` ve kutucuk komutları.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Show,
    Hide,
}

/// Kutucuğun ve `msg osk status`un gördüğü durum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OskView {
    /// Compositor sanal klavye protokolünü sunuyor.
    pub available: bool,
    pub visible: bool,
    pub layout: Layout,
}

/// `msg osk status` (tek satır JSON, sözleşme osk-v1; `placement`/`position` v1 içinde eklendi,
/// 2026-10-16: yeni anahtar, eski okuyucu etkilenmez).
pub fn status_json(view: &OskView, output: Option<&str>, placement: place::Placement) -> String {
    let (mode, position) = match placement {
        place::Placement::Docked => ("docked", None),
        place::Placement::Floating(p) => ("floating", Some([p.x.round(), p.y.round()])),
    };
    serde_json::json!({
        "available": view.available,
        "visible": view.visible,
        "layout": view.layout.name(),
        "output": output,
        "placement": mode,
        "position": position,
    })
    .to_string()
}

// ---------------------------------------------------------------------------------------
// Durum makinesi

/// Sanal klavyeye gidecek tek çağrı (sırayla uygulanır).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emit {
    /// Bu düzenin keymap'ini gönder (ilk tuştan ve düzen değişiminden önce).
    Keymap(Layout),
    /// Değiştirici maskeleri (grup her zaman 0: keymap'ler tek gruplu).
    Modifiers { depressed: u32, locked: u32 },
    Key { code: u32, pressed: bool },
}

/// Değiştirici tuşların durumu. Shift/AltGr/Ctrl tek vuruşluktur (bir sonraki tuşa uygulanıp
/// kalkar; ikinci dokunuş iptal eder), Caps kilittir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub shift: bool,
    pub caps: bool,
    pub altgr: bool,
    pub ctrl: bool,
}

/// Ekran klavyesinin durumu: etkin düzen, değiştiriciler, basılı tuş.
#[derive(Debug)]
pub struct Osk {
    layout: Layout,
    /// Derlenmiş etiket tabloları (düzen başına tembel).
    labels: [Option<Labels>; 2],
    mods: Mods,
    /// Basılı tutulan tuşun evdev kodu (bırakılana kadar; tekrar istemcide).
    held: Option<u32>,
    /// Compositor'a son gönderilen keymap ve maskeler (`None`: henüz yok / sıfırlandı).
    sent_keymap: Option<Layout>,
    sent_mods: Option<(u32, u32)>,
}

fn slot(layout: Layout) -> usize {
    match layout {
        Layout::Tr => 0,
        Layout::Us => 1,
    }
}

impl Osk {
    /// Düzenin keymap'i derlenemezse hata (gömülü metin; testler derlendiğini doğrular).
    pub fn new(layout: Layout) -> Result<Self, KeymapError> {
        let mut osk = Self { layout, labels: [None, None], mods: Mods::default(), held: None, sent_keymap: None, sent_mods: None };
        osk.ensure_labels(layout)?;
        Ok(osk)
    }

    fn ensure_labels(&mut self, layout: Layout) -> Result<(), KeymapError> {
        if self.labels[slot(layout)].is_none() {
            self.labels[slot(layout)] = Some(Labels::compile(layout)?);
        }
        Ok(())
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn labels(&self) -> &Labels {
        self.labels[slot(self.layout)].as_ref().expect("etkin düzenin etiketi derli (new/set_layout)")
    }

    pub fn mods(&self) -> Mods {
        self.mods
    }

    pub fn held(&self) -> Option<u32> {
        self.held
    }

    /// Etiketlerin aranacağı seviye.
    pub fn level(&self) -> Level {
        Level { shift: self.mods.shift, caps: self.mods.caps, altgr: self.mods.altgr }
    }

    /// Değiştirici tuş bu anda etkin mi (görünümde vurgulanır).
    pub fn is_active(&self, action: KeyAction) -> bool {
        match action {
            KeyAction::Shift => self.mods.shift,
            KeyAction::Caps => self.mods.caps,
            KeyAction::AltGr => self.mods.altgr,
            KeyAction::Ctrl => self.mods.ctrl,
            _ => false,
        }
    }

    /// Tuş bu düzende bir şey yapar mı (AltGr'siz düzende AltGr; sembolsüz seviyedeki karakter
    /// tuşu). Etkisiz tuş soluk çizilir ve basışı yok sayılır.
    pub fn is_enabled(&self, action: KeyAction) -> bool {
        match action {
            KeyAction::AltGr => self.labels().has_level3(),
            KeyAction::Char(code) => self.labels().label(code, self.level()).is_some(),
            _ => true,
        }
    }

    fn masks(&self) -> (u32, u32) {
        let m = self.labels().masks();
        let mut depressed = 0;
        if self.mods.shift {
            depressed |= m.shift;
        }
        if self.mods.altgr {
            depressed |= m.level3;
        }
        if self.mods.ctrl {
            depressed |= m.ctrl;
        }
        (depressed, if self.mods.caps { m.lock } else { 0 })
    }

    fn push_mods(&mut self, out: &mut Vec<Emit>) {
        let masks = self.masks();
        if self.sent_mods != Some(masks) {
            self.sent_mods = Some(masks);
            out.push(Emit::Modifiers { depressed: masks.0, locked: masks.1 });
        }
    }

    /// Tuşa basıldı. Yazı tuşları basılı kalır ([`Osk::release`]'e kadar); değiştiriciler
    /// yalnızca durumu değiştirir. `SwitchLayout` düzeni değiştirir; `Hide` burada etkisizdir
    /// (yürütücü yüzeyi kapatır ve [`Osk::reset`] çağırır).
    pub fn press(&mut self, action: KeyAction) -> Vec<Emit> {
        let mut out = Vec::new();
        if !self.is_enabled(action) {
            return out;
        }
        let code = match action {
            KeyAction::Char(c) => c,
            KeyAction::Special(s) => s.code(),
            KeyAction::Shift => {
                self.mods.shift = !self.mods.shift;
                return out;
            }
            KeyAction::Caps => {
                self.mods.caps = !self.mods.caps;
                return out;
            }
            KeyAction::AltGr => {
                self.mods.altgr = !self.mods.altgr;
                return out;
            }
            KeyAction::Ctrl => {
                self.mods.ctrl = !self.mods.ctrl;
                return out;
            }
            KeyAction::SwitchLayout => return self.set_layout(self.layout.next()),
            KeyAction::Hide => return out,
        };
        self.release_into(&mut out);
        if self.sent_keymap != Some(self.layout) {
            self.sent_keymap = Some(self.layout);
            // Yeni keymap'te istemci durumu sıfırdan kurar; maskeler yeniden gönderilir.
            self.sent_mods = None;
            out.push(Emit::Keymap(self.layout));
        }
        self.push_mods(&mut out);
        out.push(Emit::Key { code, pressed: true });
        self.held = Some(code);
        out
    }

    /// Basılı tuşu bırakır; tek vuruşluk değiştiriciler kalkar.
    pub fn release(&mut self) -> Vec<Emit> {
        let mut out = Vec::new();
        if self.held.is_some() {
            self.release_into(&mut out);
            self.mods = Mods { caps: self.mods.caps, ..Mods::default() };
            self.push_mods(&mut out);
        }
        out
    }

    fn release_into(&mut self, out: &mut Vec<Emit>) {
        if let Some(code) = self.held.take() {
            out.push(Emit::Key { code, pressed: false });
        }
    }

    /// Düzeni değiştirir: basılı tuş önce bırakılır, yeni keymap bir sonraki tuştan önce gider.
    /// Derlenemeyen düzene geçilmez (hata loglanır).
    pub fn set_layout(&mut self, layout: Layout) -> Vec<Emit> {
        let mut out = Vec::new();
        if layout == self.layout {
            return out;
        }
        if let Err(e) = self.ensure_labels(layout) {
            tracing::warn!("ekran klavyesi: {e}");
            return out;
        }
        self.release_into(&mut out);
        self.layout = layout;
        // AltGr'siz düzende bekleyen AltGr anlamsız.
        if !self.labels().has_level3() {
            self.mods.altgr = false;
        }
        out
    }

    /// Klavye gizlendi: basılı tuş bırakılır, değiştiriciler sıfırlanır (istemcide takılı Shift
    /// kalmaz). Sonraki ilk tuş keymap'i yeniden gönderir (yürütücü sanal klavyeyi bırakır).
    pub fn reset(&mut self) -> Vec<Emit> {
        let mut out = Vec::new();
        self.release_into(&mut out);
        self.mods = Mods::default();
        if self.sent_mods.is_some_and(|m| m != (0, 0)) {
            out.push(Emit::Modifiers { depressed: 0, locked: 0 });
        }
        self.sent_mods = None;
        self.sent_keymap = None;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::keys::code;
    use super::*;

    fn key(code: u32, pressed: bool) -> Emit {
        Emit::Key { code, pressed }
    }

    #[test]
    fn first_key_sends_keymap_then_modifiers_then_key() {
        let mut o = Osk::new(Layout::Tr).unwrap();
        assert_eq!(
            o.press(KeyAction::Char(code::AD01)),
            [Emit::Keymap(Layout::Tr), Emit::Modifiers { depressed: 0, locked: 0 }, key(code::AD01, true)]
        );
        assert_eq!(o.held(), Some(code::AD01));
        assert_eq!(o.release(), [key(code::AD01, false)], "maskeler değişmedi: tekrar gönderilmez");
        assert_eq!(o.press(KeyAction::Char(code::AD01)), [key(code::AD01, true)], "keymap bir kez");
        assert_eq!(o.release(), [key(code::AD01, false)]);
        assert_eq!(o.release(), [], "basılı tuş yokken bırakma boş");
    }

    #[test]
    fn one_shot_shift_applies_to_next_key_only() {
        let mut o = Osk::new(Layout::Tr).unwrap();
        o.press(KeyAction::Char(code::AD01));
        o.release();
        assert!(o.press(KeyAction::Shift).is_empty());
        assert!(o.is_active(KeyAction::Shift));
        assert_eq!(o.labels().label(code::AC11, o.level()), Some("İ"));
        assert_eq!(o.press(KeyAction::Char(code::AC11)), [Emit::Modifiers { depressed: 1, locked: 0 }, key(code::AC11, true)]);
        assert_eq!(o.release(), [key(code::AC11, false), Emit::Modifiers { depressed: 0, locked: 0 }]);
        assert!(!o.is_active(KeyAction::Shift));
        // İkinci dokunuş iptal eder.
        o.press(KeyAction::Shift);
        o.press(KeyAction::Shift);
        assert!(!o.mods().shift);
    }

    #[test]
    fn caps_lock_stays_and_uses_lock_mask() {
        let mut o = Osk::new(Layout::Tr).unwrap();
        o.press(KeyAction::Caps);
        let e = o.press(KeyAction::Char(code::AD08));
        assert_eq!(e[1..], [Emit::Modifiers { depressed: 0, locked: 2 }, key(code::AD08, true)]);
        assert_eq!(o.labels().label(code::AD08, o.level()), Some("I"), "Türkçe ı → I");
        o.release();
        assert!(o.mods().caps, "Caps kilit; tuştan sonra kalır");
        o.press(KeyAction::Caps);
        assert_eq!(o.press(KeyAction::Char(code::AD08)), [Emit::Modifiers { depressed: 0, locked: 0 }, key(code::AD08, true)]);
    }

    #[test]
    fn altgr_and_ctrl_combine() {
        let mut o = Osk::new(Layout::Tr).unwrap();
        o.press(KeyAction::AltGr);
        o.press(KeyAction::Ctrl);
        let e = o.press(KeyAction::Char(code::AD01));
        assert_eq!(e[1], Emit::Modifiers { depressed: 4 | (1 << 7), locked: 0 });
        o.release();
        assert_eq!(o.mods(), Mods::default());
    }

    #[test]
    fn new_key_releases_the_held_one_first() {
        let mut o = Osk::new(Layout::Us).unwrap();
        o.press(KeyAction::Special(Special::Backspace));
        let e = o.press(KeyAction::Char(code::AD01));
        assert_eq!(e, [key(code::BACKSPACE, false), key(code::AD01, true)]);
    }

    #[test]
    fn layout_switch_resends_keymap_and_keeps_pending_altgr() {
        let mut o = Osk::new(Layout::Tr).unwrap();
        o.press(KeyAction::Char(code::AD01));
        o.press(KeyAction::AltGr);
        let e = o.press(KeyAction::SwitchLayout);
        assert_eq!(e, [key(code::AD01, false)], "basılı tuş önce bırakılır");
        assert_eq!(o.layout(), Layout::Us);
        // US (pc105) AltGr'yi yalnızca <> tuşunda kullanır: bekleyen AltGr kalır.
        assert!(o.mods().altgr && o.is_enabled(KeyAction::AltGr));
        o.press(KeyAction::AltGr);
        assert_eq!(o.press(KeyAction::Char(code::AD01)), [Emit::Keymap(Layout::Us), Emit::Modifiers { depressed: 0, locked: 0 }, key(code::AD01, true)]);
        assert!(o.set_layout(Layout::Us).iter().all(|e| !matches!(e, Emit::Keymap(_))), "aynı düzen: işlem yok");
    }

    #[test]
    fn symbolless_level_is_inert() {
        let mut o = Osk::new(Layout::Tr).unwrap();
        o.press(KeyAction::AltGr);
        // TR'de W'nin AltGr seviyesi boş.
        assert!(!o.is_enabled(KeyAction::Char(code::AD01 + 1)));
        assert!(o.press(KeyAction::Char(code::AD01 + 1)).is_empty());
        assert!(o.mods().altgr, "etkisiz basış tek vuruşu tüketmez");
    }

    #[test]
    fn reset_releases_and_clears_client_modifiers() {
        let mut o = Osk::new(Layout::Tr).unwrap();
        o.press(KeyAction::Caps);
        o.press(KeyAction::Char(code::AD01));
        assert_eq!(o.reset(), [key(code::AD01, false), Emit::Modifiers { depressed: 0, locked: 0 }]);
        assert_eq!(o.mods(), Mods::default());
        assert_eq!(o.press(KeyAction::Char(code::AD01))[0], Emit::Keymap(Layout::Tr), "gizlenince keymap yeniden");
        let mut fresh = Osk::new(Layout::Tr).unwrap();
        assert!(fresh.reset().is_empty(), "hiç gönderilmemişse boş");
    }

    #[test]
    fn config_names_and_validation() {
        crate::i18n::use_turkish_for_tests();
        for l in LayoutSetting::ALL {
            let c: OskConfig = toml::from_str(&format!("layout = \"{}\"", l.name())).unwrap();
            assert_eq!(c.layout, l);
        }
        assert!(toml::from_str::<OskConfig>("layout = \"de\"").is_err());
        assert_eq!(LayoutSetting::Auto.fixed(), None);
        assert_eq!(LayoutSetting::Us.fixed(), Some(Layout::Us));
        let mut c = OskConfig { key_height: 200, ..OskConfig::default() };
        let mut w = Vec::new();
        c.validate(&mut w);
        assert_eq!(c.key_height, 72);
        assert_eq!(w.len(), 1);
        c.validate(&mut w);
        assert_eq!(w.len(), 1, "idempotent");
    }

    #[test]
    fn status_contract() {
        let v = OskView { available: true, visible: true, layout: Layout::Us };
        let floating = place::Placement::Floating(nuxut::Point::new(120.4, 80.6));
        assert_eq!(
            status_json(&v, Some("DP-1"), floating),
            r#"{"available":true,"layout":"us","output":"DP-1","placement":"floating","position":[120.0,81.0],"visible":true}"#
        );
        assert_eq!(
            status_json(&OskView::default(), None, place::Placement::Docked),
            r#"{"available":false,"layout":"tr","output":null,"placement":"docked","position":null,"visible":false}"#
        );
    }
}
