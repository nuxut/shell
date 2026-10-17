//! Klavye düzenleri (bar düzen göstergesi; `docs/design/keyboard-layout.md`): Hyprland
//! `j/devices` yanıtından ana klavyenin düzen listesi ve etkin düzen. IO yok; sorgu Hyprland
//! servisinin thread'inde (`activelayout`, bağlanma ve ayar yeniden yüklenmesinde), geçiş
//! `switchxkblayout current next`.
//!
//! Framework'ün `Context::keyboard_layout` yalnız kabuk odaktayken güncellenir (xkb grubu
//! odaklı istemciye gider); bar her zaman doğruyu göstermeli, kaynak bu yüzden Hyprland.

use serde_json::Value;

/// Listede tutulan en çok düzen (xkb grubu sınırı 4; fazlası dış girdi saçmalığıdır).
pub const MAX_LAYOUTS: usize = 8;
/// Düzen/varyant kodunun en uzun hâli (xkb kodları kısa: "tr", "us", "intl").
const MAX_CODE: usize = 32;
/// Etkin düzenin görünen adı ("Turkish", "English (US, intl., with dead keys)").
const MAX_NAME: usize = 96;

/// Ana klavyenin düzenleri.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyboardLayouts {
    /// (düzen, varyant) — `kb_layout`/`kb_variant` sırasıyla.
    pub layouts: Vec<(String, String)>,
    /// Etkin düzenin sırası (`active_layout_index`; listede).
    pub active: usize,
    /// Etkin düzenin xkb açıklaması (ipucu).
    pub active_name: String,
}

impl KeyboardLayouts {
    /// Bar'da gösterilecek kısa ad: "TR", "US"; aynı düzen iki kez varsa varyantla ayrılır
    /// ("US", "US·INTL").
    pub fn label(&self, i: usize) -> Option<String> {
        let (code, variant) = self.layouts.get(i)?;
        let duplicate = self.layouts.iter().filter(|(c, _)| c == code).count() > 1;
        let mut s = code.to_uppercase();
        if duplicate && !variant.is_empty() {
            s.push('·');
            s.push_str(&variant.to_uppercase());
        }
        Some(s)
    }

    pub fn active_label(&self) -> Option<String> {
        self.label(self.active)
    }

    /// Geçiş anlamlı mı (2+ düzen).
    pub fn switchable(&self) -> bool {
        self.layouts.len() >= 2
    }
}

fn code_ok(s: &str) -> bool {
    s.len() <= MAX_CODE && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// `j/devices` yanıtından ana klavye (`main: true`, yoksa ilk klavye). Sanal klavyeler
/// (ekran klavyesi, `virtual` adlı) ana sayılmaz. Bozuk ya da düzeni olmayan yanıt `None`.
pub fn parse_devices(text: &str) -> Option<KeyboardLayouts> {
    let v: Value = serde_json::from_str(text).ok()?;
    let keyboards = v.get("keyboards")?.as_array()?;
    let real = |k: &&Value| !k.get("name").and_then(Value::as_str).is_some_and(|n| n.contains("virtual"));
    let main = keyboards
        .iter()
        .filter(real)
        .find(|k| k.get("main").and_then(Value::as_bool) == Some(true))
        .or_else(|| keyboards.iter().find(real))?;
    let field = |k: &str| main.get(k).and_then(Value::as_str).unwrap_or("");
    let codes: Vec<&str> = field("layout").split(',').map(str::trim).collect();
    let variants: Vec<&str> = field("variant").split(',').map(str::trim).collect();
    let mut layouts = Vec::new();
    for (i, code) in codes.iter().enumerate().take(MAX_LAYOUTS) {
        let variant = variants.get(i).copied().unwrap_or("");
        if code.is_empty() || !code_ok(code) || !code_ok(variant) {
            return None;
        }
        layouts.push(((*code).to_owned(), variant.to_owned()));
    }
    if layouts.is_empty() {
        return None;
    }
    let active = main.get("active_layout_index").and_then(Value::as_u64).map_or(0, |i| i as usize).min(layouts.len() - 1);
    let active_name = field("active_keymap").chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    Some(KeyboardLayouts { layouts, active, active_name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_keyboard_layouts_and_labels() {
        let text = r#"{"mice":[],"keyboards":[
            {"name":"virtual-keyboard","layout":"us","variant":"","active_layout_index":0,"active_keymap":"English (US)","main":false},
            {"name":"at-translated-set-2-keyboard","layout":"tr,us,us","variant":",,intl","active_layout_index":2,
             "active_keymap":"English (US, intl., with dead keys)","main":true}]}"#;
        let k = parse_devices(text).unwrap();
        assert_eq!(k.layouts.len(), 3);
        assert_eq!(k.active, 2);
        assert_eq!(k.active_name, "English (US, intl., with dead keys)");
        assert_eq!([k.label(0), k.label(1), k.label(2)], [Some("TR".into()), Some("US".into()), Some("US·INTL".into())]);
        assert!(k.switchable());
    }

    #[test]
    fn single_layout_and_bad_input() {
        let one = r#"{"keyboards":[{"name":"kbd","layout":"tr","variant":"","active_layout_index":0,"active_keymap":"Turkish","main":true}]}"#;
        let k = parse_devices(one).unwrap();
        assert!(!k.switchable(), "tek düzende gösterge gizli");
        assert_eq!(k.active_label().as_deref(), Some("TR"));
        // main yoksa ilk gerçek klavye; indeks taşarsa son düzen.
        let k = parse_devices(r#"{"keyboards":[{"name":"kbd","layout":"de,fr","active_layout_index":9}]}"#).unwrap();
        assert_eq!(k.active, 1);
        for bad in ["", "{}", r#"{"keyboards":[]}"#, r#"{"keyboards":[{"name":"k","layout":""}]}"#,
                    r#"{"keyboards":[{"name":"k","layout":"t r;rm"}]}"#, r#"{"keyboards":[{"name":"virtual-1","layout":"us","main":true}]}"#] {
            assert_eq!(parse_devices(bad), None, "{bad}");
        }
        let many = format!(r#"{{"keyboards":[{{"name":"k","layout":"{}"}}]}}"#, vec!["us"; 20].join(","));
        assert_eq!(parse_devices(&many).unwrap().layouts.len(), MAX_LAYOUTS);
    }
}
