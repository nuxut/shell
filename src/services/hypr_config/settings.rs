//! `shell.toml` bölümleri: `[compositor]`, `[windows]`, `[keybinds]` (tasarım notu §6.1); `[input]`
//! kendi modülünde ([`super::input`]).
//! Şema ve doğrulama burada; `config.rs` yalnızca alanları taşır ve çağırır.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::keybinds::{Action, Binding, parse_action_combo};
use crate::config::sanitize;
use nuxut::i18n::t;
use crate::schema::SettingSchema;

/// `[compositor]`: masaüstü ayar dosyasıyla eşitleme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CompositorConfig {
    /// Açılışta Hyprland ayarında `source = …/nuxut.conf` yoksa bir kez ekle.
    pub ensure_source: bool,
    /// Köşe + gölge + bulanıklık.
    pub sync_look: bool,
    pub sync_motion: bool,
    /// Pencere aralığı + kenarlık.
    pub sync_windows: bool,
    pub sync_keybinds: bool,
    /// `[input]` (klavye, fare, dokunmatik yüzey); `false` ise `[input]`'tan hiç satır yok.
    pub sync_input: bool,
    /// Temel katman (ADR-0006): pencere/çalışma alanı kısayolları, pencere ve katman kuralları,
    /// yerleşim, jest ve davranış varsayılanları; kullanıcının ayarından önce okunur. `false`:
    /// temel dosyada yalnızca kabuk eylemlerinin kısayolları kalır.
    pub base_layer: bool,
    /// Kabuk kısayolları (ses, parlaklık, medya, ekran görüntüsü…) kullanıcının ayarında aynı tuş
    /// bağlıysa devralınır: ayar katmanında `unbind` + Nuxut'un bind'i (kullanıcı isteği
    /// 2026-10-16, "Nuxut yapabiliyorsa varsayılan Nuxut"). `false`: kullanıcınınki kalır, Nuxut'unki
    /// üretilmez. Kullanıcının dosyasına hiçbir koşulda yazılmaz.
    pub take_over_keys: bool,
}

impl Default for CompositorConfig {
    fn default() -> Self {
        Self {
            ensure_source: true,
            sync_look: true,
            sync_motion: true,
            sync_windows: true,
            sync_keybinds: true,
            sync_input: true,
            base_layer: true,
            take_over_keys: true,
        }
    }
}

impl CompositorConfig {
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(SettingSchema::bool("compositor.ensure_source", t!("shell-hypr-ensure-source-label"), d.ensure_source));
        out.push(SettingSchema::bool("compositor.sync_look", t!("shell-hypr-sync-look-label"), d.sync_look));
        out.push(SettingSchema::bool("compositor.sync_motion", t!("shell-hypr-sync-motion-label"), d.sync_motion));
        out.push(SettingSchema::bool("compositor.sync_windows", t!("shell-hypr-sync-windows-label"), d.sync_windows));
        out.push(SettingSchema::bool("compositor.sync_keybinds", t!("shell-hypr-sync-keybinds-label"), d.sync_keybinds));
        out.push(SettingSchema::bool("compositor.sync_input", t!("shell-hypr-sync-input-label"), d.sync_input));
        out.push(SettingSchema::bool("compositor.base_layer", t!("shell-hypr-base-layer-label"), d.base_layer));
        out.push(SettingSchema::bool("compositor.take_over_keys", t!("shell-hypr-take-over-keys-label"), d.take_over_keys));
    }

    /// Herhangi bir şey eşitleniyor mu (değilse nuxut.conf hiç yazılmaz). `sync_input` tek başına
    /// sayılmaz: yalnızca `[input]`'ta yazılı anahtar varsa dosya gerekir (bkz. `Syncer::run`).
    pub fn any(&self) -> bool {
        self.ensure_source || self.sync_look || self.sync_motion || self.sync_windows || self.sync_keybinds
    }
}

pub const GAP_INNER: std::ops::RangeInclusive<u32> = 0..=64;
pub const GAP_OUTER: std::ops::RangeInclusive<u32> = 0..=128;
pub const BORDER: std::ops::RangeInclusive<u32> = 0..=16;
pub const BLUR_SIZE: std::ops::RangeInclusive<u32> = 1..=32;

/// `[windows]`: pencere aralığı, kenarlık, bulanıklık.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct WindowsConfig {
    pub gap_inner: u32,
    pub gap_outer: u32,
    pub border: u32,
    pub blur: bool,
    pub blur_size: u32,
}

impl Default for WindowsConfig {
    fn default() -> Self {
        Self { gap_inner: 4, gap_outer: 8, border: 2, blur: true, blur_size: 8 }
    }
}

impl WindowsConfig {
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        let int = |key, title, v: u32, r: &std::ops::RangeInclusive<u32>| {
            SettingSchema::int(key, title, v.into(), (*r.start()).into(), (*r.end()).into()).unit("px")
        };
        out.push(int("windows.gap_inner", t!("shell-hypr-gaps-inner-label"), d.gap_inner, &GAP_INNER));
        out.push(int("windows.gap_outer", t!("shell-hypr-gaps-outer-label"), d.gap_outer, &GAP_OUTER));
        out.push(int("windows.border", t!("shell-hypr-border-size-label"), d.border, &BORDER));
        out.push(SettingSchema::bool("windows.blur", t!("shell-hypr-blur-label"), d.blur));
        out.push(int("windows.blur_size", t!("shell-hypr-blur-size-label"), d.blur_size, &BLUR_SIZE));
    }

    pub fn validate(&mut self, w: &mut Vec<String>) {
        for (v, key, r) in [
            (&mut self.gap_inner, "windows.gap_inner", GAP_INNER),
            (&mut self.gap_outer, "windows.gap_outer", GAP_OUTER),
            (&mut self.border, "windows.border", BORDER),
            (&mut self.blur_size, "windows.blur_size", BLUR_SIZE),
        ] {
            if !r.contains(v) {
                let fixed = (*v).clamp(*r.start(), *r.end());
                w.push(format!("{key} = {v} aralık dışı, {fixed} kullanılıyor ({}..={})", r.start(), r.end()));
                *v = fixed;
            }
        }
    }
}

/// `[keybinds]`: `eylem = "MOD+Tuş"` (`""` = atanmamış). Doğrulamadan sonra her eylem
/// tabloda bulunur (verilmeyen varsayılanını alır); bilinmeyen eylem atılır.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct KeybindsConfig {
    pub map: BTreeMap<String, String>,
}

impl Default for KeybindsConfig {
    fn default() -> Self {
        Self { map: Action::ALL.iter().map(|a| (a.name().to_owned(), a.default_combo().to_owned())).collect() }
    }
}

impl KeybindsConfig {
    pub fn schema(out: &mut Vec<SettingSchema>) {
        for a in Action::ALL {
            out.push(SettingSchema::text(format!("keybinds.{}", a.name()), a.title(), a.default_combo()));
        }
    }

    pub fn validate(&mut self, w: &mut Vec<String>) {
        let mut unknown = Vec::new();
        self.map.retain(|k, _| {
            let ok = Action::from_name(k).is_some();
            if !ok && unknown.len() < 4 {
                unknown.push(format!("\"{}\"", sanitize(k)));
            }
            ok
        });
        if !unknown.is_empty() {
            w.push(format!("keybinds: bilinmeyen eylem {} yok sayıldı", unknown.join(", ")));
        }
        let mut used: Vec<Binding> = Vec::new();
        for &a in Action::ALL {
            let text = self.map.entry(a.name().to_owned()).or_insert_with(|| a.default_combo().to_owned());
            if text.trim().is_empty() {
                text.clear();
                continue;
            }
            match parse_action_combo(a, text) {
                Ok(combo) => {
                    let b = Binding { action: a, combo };
                    if let Some(other) = used.iter().find(|o| o.clashes(&b)) {
                        w.push(format!("keybinds.{}: {} zaten {} için atanmış, atanmamış sayıldı", a.name(), b.combo, other.action.name()));
                        text.clear();
                    } else {
                        used.push(b);
                    }
                }
                Err(e) => {
                    w.push(format!("keybinds.{}: {e}; atanmamış sayıldı", a.name()));
                    text.clear();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keybinds_validate() {
        let mut k = KeybindsConfig { map: BTreeMap::new() };
        k.map.insert("panel_right".into(), "SUPER+N".into());
        k.map.insert("bar_toggle".into(), "SUPER+n".into());
        k.map.insert("volume_up".into(), "SUPER+;".into());
        k.map.insert("nope".into(), "X".into());
        let mut w = Vec::new();
        k.validate(&mut w);
        assert_eq!(w.len(), 3, "{w:?}");
        assert_eq!(k.map.len(), Action::ALL.len());
        assert_eq!(k.map["bar_toggle"], "");
        assert_eq!(k.map["volume_up"], "");
        assert_eq!(k.map["volume_down"], "XF86AudioLowerVolume");
        let mut again = k.clone();
        let mut w2 = Vec::new();
        again.validate(&mut w2);
        assert!(w2.is_empty() && again == k, "idempotent");
        let mut d = KeybindsConfig::default();
        d.validate(&mut w2);
        assert!(w2.is_empty() && d == KeybindsConfig::default());
    }

    #[test]
    fn windows_validate() {
        let mut c = WindowsConfig { gap_inner: 999, blur_size: 0, ..Default::default() };
        let mut w = Vec::new();
        c.validate(&mut w);
        assert_eq!((c.gap_inner, c.blur_size), (64, 1));
        assert_eq!(w.len(), 2);
    }
}
