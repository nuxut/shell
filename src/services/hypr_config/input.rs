//! `shell.toml` `[input]`: klavye, fare ve dokunmatik yüzey (sözleşme `input-v1`).
//!
//! **İlke: içe aktar, ezme.** Her anahtar isteğe bağlıdır; yazılmamış anahtar için üretilen
//! dosyaya (nuxut.conf / nuxut.lua) hiç satır girmez, kullanıcının Hyprland ayarındaki değer
//! (ör. `kb_layout = tr`) aynen kalır. Serde varsayılanı yoktur; şemadaki `default` yalnızca
//! Ayarlar'ın "varsayılan" göstergesi için Hyprland varsayılanını taşır.
//!
//! Ayrıştırma esnektir: bölümler ham TOML tablosu olarak okunur, [`InputConfig::validate`]
//! anahtar anahtar türe çevirir. Geçersiz değer (tür, ad deseni, aralık dışı sayı, bilinmeyen
//! seçenek) yalnızca o anahtarı düşürür + uyarı; dosyanın geri kalanı uygulanır. Sayılar
//! kırpılmaz.
//!
//! `msg input status` ([`InputStatus`]) etkin değerleri Hyprland'den `getoption` ile okur
//! (tek `[[BATCH]]` isteği, salt okunur).

use nuxut::i18n::t;
use serde::Serialize;

use super::control::HyprControl;
use super::generate::safe_value;
use crate::config::sanitize;
use crate::schema::{SettingSchema, SettingValue};

pub const REPEAT_RATE: std::ops::RangeInclusive<i64> = 1..=100;
pub const REPEAT_DELAY: std::ops::RangeInclusive<i64> = 100..=2000;
pub const SENSITIVITY: std::ops::RangeInclusive<f64> = -1.0..=1.0;
pub const SCROLL_FACTOR: std::ops::RangeInclusive<f64> = 0.1..=3.0;
/// Düzen sayısı (xkb en çok 4 grup).
pub const MAX_LAYOUTS: usize = 4;
pub const MAX_OPTIONS: usize = 16;
pub const ACCEL_PROFILES: &[&str] = &["adaptive", "flat"];
pub const SCROLL_METHODS: &[&str] = &["2fg", "edge", "on_button_down", "no_scroll"];

/// Bir klavye düzeni (`layout` boş olamaz; `variant` boş olabilir).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KbLayout {
    pub layout: String,
    pub variant: String,
}

/// Doğrulanmış `[input]`; `None` = dosyada yazılmamış (ya da geçersiz) → satır yok.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputSettings {
    pub layouts: Option<Vec<KbLayout>>,
    pub options: Option<Vec<String>>,
    pub repeat_rate: Option<i64>,
    pub repeat_delay: Option<i64>,
    pub numlock_by_default: Option<bool>,
    pub sensitivity: Option<f64>,
    pub accel_profile: Option<&'static str>,
    pub mouse_natural_scroll: Option<bool>,
    pub touchpad_natural_scroll: Option<bool>,
    pub tap_to_click: Option<bool>,
    pub disable_while_typing: Option<bool>,
    pub scroll_factor: Option<f64>,
    pub scroll_method: Option<&'static str>,
}

/// `[input]` (ham bölümler + doğrulanmış sonuç; `bar.widget` tablolarındaki kalıp).
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct InputConfig {
    keyboard: Option<toml::Table>,
    mouse: Option<toml::Table>,
    touchpad: Option<toml::Table>,
    /// [`Self::validate`] sonrası geçerli değerler.
    #[serde(skip)]
    pub settings: InputSettings,
}

/// Bir ayarın Hyprland karşılığı: `shell.toml` anahtarı, hyprlang anahtarı, Lua yolu
/// (`hl.config({ input = { … } })` içinde; `touchpad.` alt tablo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyMap {
    pub setting: &'static str,
    pub hypr: &'static str,
    pub lua: &'static str,
}

const fn km(setting: &'static str, hypr: &'static str, lua: &'static str) -> KeyMap {
    KeyMap { setting, hypr, lua }
}

/// Sözleşme tablosu (sıra = üretim sırası). `keyboard.layouts` iki Hyprland anahtarı yazar.
pub const KEYS: &[KeyMap] = &[
    km("input.keyboard.layouts", "input:kb_layout", "kb_layout"),
    km("input.keyboard.layouts", "input:kb_variant", "kb_variant"),
    km("input.keyboard.options", "input:kb_options", "kb_options"),
    km("input.keyboard.repeat_rate", "input:repeat_rate", "repeat_rate"),
    km("input.keyboard.repeat_delay", "input:repeat_delay", "repeat_delay"),
    km("input.keyboard.numlock_by_default", "input:numlock_by_default", "numlock_by_default"),
    km("input.mouse.sensitivity", "input:sensitivity", "sensitivity"),
    km("input.mouse.accel_profile", "input:accel_profile", "accel_profile"),
    km("input.mouse.natural_scroll", "input:natural_scroll", "natural_scroll"),
    km("input.touchpad.natural_scroll", "input:touchpad:natural_scroll", "touchpad.natural_scroll"),
    km("input.touchpad.tap_to_click", "input:touchpad:tap-to-click", "touchpad.tap_to_click"),
    km("input.touchpad.disable_while_typing", "input:touchpad:disable_while_typing", "touchpad.disable_while_typing"),
    km("input.touchpad.scroll_factor", "input:touchpad:scroll_factor", "touchpad.scroll_factor"),
    km("input.touchpad.scroll_method", "input:scroll_method", "scroll_method"),
];

/// Hyprland anahtarının `shell.toml` karşılığı (override eşlemesi için).
pub fn setting_for_hypr(hypr: &str) -> Option<&'static str> {
    KEYS.iter().find(|k| k.hypr == hypr).map(|k| k.setting)
}

/// Değişikliği canlı `keyword` ile değil yeniden okumayla uygulanan anahtarlar: düzen ve
/// varyant birlikte derlenmeli (tek tek `keyword` arada geçersiz bir xkb bileşimi kurabilir),
/// `kb_options` boş olabilir (boş `keyword` değeri yerine dosyadan okunur).
pub const RELOAD_KEYS: &[&str] = &["input:kb_layout", "input:kb_variant", "input:kb_options"];

/// Üretilecek tek değer.
#[derive(Debug, Clone, PartialEq)]
pub enum HyprValue {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl HyprValue {
    /// hyprlang değeri (`safe_value`'dan geçer; dize değerleri doğrulamada desenle sınırlı).
    pub fn conf(&self) -> String {
        match self {
            Self::Str(s) => s.clone(),
            Self::Int(i) => i.to_string(),
            Self::Float(f) => float_text(*f),
            Self::Bool(b) => b.to_string(),
        }
    }
}

/// Sonlu ondalık, en çok 3 basamak (`0`, `0.5`, `-0.25`).
pub fn float_text(v: f64) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let r = (v * 1000.0).round() / 1000.0;
    let r = if r == 0.0 { 0.0 } else { r }; // -0 yazılmaz
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

/// Üretilecek bir satır.
#[derive(Debug, Clone, PartialEq)]
pub struct InputLine {
    pub key: KeyMap,
    pub value: HyprValue,
}

impl InputSettings {
    pub fn is_empty(&self) -> bool {
        self.lines().is_empty()
    }

    /// Yazılı anahtarların satırları, [`KEYS`] sırasıyla. `layouts` → `kb_layout` +
    /// `kb_variant` (virgülle, aynı sayıda öğe); `options` boşsa `kb_options` boş (açıkça temizler).
    pub fn lines(&self) -> Vec<InputLine> {
        let mut out = Vec::new();
        for key in KEYS {
            let value = match key.hypr {
                "input:kb_layout" => self.layouts.as_ref().map(|l| join(l.iter().map(|l| l.layout.as_str()))),
                "input:kb_variant" => self.layouts.as_ref().map(|l| join(l.iter().map(|l| l.variant.as_str()))),
                "input:kb_options" => self.options.as_ref().map(|o| join(o.iter().map(String::as_str))),
                "input:repeat_rate" => self.repeat_rate.map(HyprValue::Int),
                "input:repeat_delay" => self.repeat_delay.map(HyprValue::Int),
                "input:numlock_by_default" => self.numlock_by_default.map(HyprValue::Bool),
                "input:sensitivity" => self.sensitivity.map(HyprValue::Float),
                "input:accel_profile" => self.accel_profile.map(|s| HyprValue::Str(s.into())),
                "input:natural_scroll" => self.mouse_natural_scroll.map(HyprValue::Bool),
                "input:touchpad:natural_scroll" => self.touchpad_natural_scroll.map(HyprValue::Bool),
                "input:touchpad:tap-to-click" => self.tap_to_click.map(HyprValue::Bool),
                "input:touchpad:disable_while_typing" => self.disable_while_typing.map(HyprValue::Bool),
                "input:touchpad:scroll_factor" => self.scroll_factor.map(HyprValue::Float),
                "input:scroll_method" => self.scroll_method.map(|s| HyprValue::Str(s.into())),
                _ => None,
            };
            // Doğrulama zaten güvenli karakterle sınırlar; yine de dosyaya güvensiz değer girmez.
            if let Some(value) = value.filter(|v| safe_value(&v.conf())) {
                out.push(InputLine { key: *key, value });
            }
        }
        out
    }

    /// Yazılı `shell.toml` anahtarları (sözleşmedeki `managed`; tekrarsız, [`KEYS`] sırasıyla).
    pub fn managed(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for l in self.lines() {
            if !out.contains(&l.key.setting) {
                out.push(l.key.setting);
            }
        }
        out
    }
}

fn join<'a>(items: impl Iterator<Item = &'a str>) -> HyprValue {
    HyprValue::Str(items.collect::<Vec<_>>().join(","))
}

// ---------------------------------------------------------------------------------------
// Doğrulama

/// Düzen/varyant adı: `[A-Za-z0-9_()+:-]{1,64}`.
fn valid_name(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_()+:-".contains(&b))
}

/// xkb seçeneği: `^[a-z0-9_]+:[A-Za-z0-9_()+-]{1,64}$`.
fn valid_option(s: &str) -> bool {
    let Some((group, name)) = s.split_once(':') else { return false };
    !group.is_empty()
        && group.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_()+-".contains(&b))
}

/// Uyarıda değer (kısa, denetim karaktersiz).
fn shown(v: &toml::Value) -> String {
    sanitize(&v.to_string())
}

fn range_text<T: std::fmt::Display>(r: &std::ops::RangeInclusive<T>) -> String {
    format!("{}..={}", r.start(), r.end())
}

struct Section<'a> {
    name: &'static str,
    table: &'a toml::Table,
    w: &'a mut Vec<String>,
}

impl Section<'_> {
    fn key(&self, k: &str) -> String {
        format!("input.{}.{k}", self.name)
    }

    fn wrong_type(&mut self, k: &str, v: &toml::Value) {
        let key = self.key(k);
        self.w.push(t!("shell-config-input-wrong-type", key = key, value = shown(v)));
    }

    fn out_of_range(&mut self, k: &str, value: String, range: String) {
        let key = self.key(k);
        self.w.push(t!("shell-config-input-out-of-range", key = key, value = value, range = range));
    }

    fn invalid_item(&mut self, k: &str, v: &toml::Value) {
        let key = self.key(k);
        self.w.push(t!("shell-config-input-invalid-item", key = key, value = shown(v)));
    }

    fn bool(&mut self, k: &str) -> Option<bool> {
        let v = self.table.get(k)?;
        let b = v.as_bool();
        if b.is_none() {
            self.wrong_type(k, v);
        }
        b
    }

    fn int(&mut self, k: &str, range: std::ops::RangeInclusive<i64>) -> Option<i64> {
        let v = self.table.get(k)?;
        match v.as_integer() {
            Some(i) if range.contains(&i) => Some(i),
            Some(i) => {
                self.out_of_range(k, i.to_string(), range_text(&range));
                None
            }
            None => {
                self.wrong_type(k, v);
                None
            }
        }
    }

    fn float(&mut self, k: &str, range: std::ops::RangeInclusive<f64>) -> Option<f64> {
        let v = self.table.get(k)?;
        let f = v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
        match f {
            Some(f) if f.is_finite() && range.contains(&f) => Some(f),
            Some(_) => {
                self.out_of_range(k, shown(v), range_text(&range));
                None
            }
            None => {
                self.wrong_type(k, v);
                None
            }
        }
    }

    fn choice(&mut self, k: &str, options: &[&'static str]) -> Option<&'static str> {
        let v = self.table.get(k)?;
        let found = v.as_str().and_then(|s| options.iter().find(|o| **o == s)).copied();
        if found.is_none() {
            let key = self.key(k);
            self.w.push(t!("shell-config-input-invalid-choice", key = key, value = shown(v), options = options.join(", ")));
        }
        found
    }

    fn unknown(&mut self, known: &[&str]) {
        let mut unknown: Vec<String> =
            self.table.keys().filter(|k| !known.contains(&k.as_str())).take(4).map(|k| format!("\"{}\"", sanitize(k))).collect();
        if !unknown.is_empty() {
            unknown.sort();
            let section = format!("input.{}", self.name);
            self.w.push(t!("shell-config-input-unknown-key", section = section, keys = unknown.join(", ")));
        }
    }

    /// Liste anahtarı: dizi mi, öğe sayısı aralıkta mı?
    fn list(&mut self, k: &str, count: std::ops::RangeInclusive<usize>) -> Option<&toml::value::Array> {
        let v = self.table.get(k)?;
        let Some(items) = v.as_array() else {
            self.wrong_type(k, v);
            return None;
        };
        if !count.contains(&items.len()) {
            let key = self.key(k);
            self.w.push(t!("shell-config-input-count", key = key, count = items.len().to_string(), range = range_text(&count)));
            return None;
        }
        Some(items)
    }

    fn layouts(&mut self) -> Option<Vec<KbLayout>> {
        let items = self.list("layouts", 1..=MAX_LAYOUTS)?.clone();
        let mut out = Vec::with_capacity(items.len());
        for item in &items {
            let parsed = item.as_table().and_then(|t| {
                let layout = t.get("layout")?.as_str()?;
                let variant = match t.get("variant") {
                    None => "",
                    Some(v) => v.as_str()?,
                };
                let extra = t.keys().any(|k| k != "layout" && k != "variant");
                (valid_name(layout) && (variant.is_empty() || valid_name(variant)) && !extra)
                    .then(|| KbLayout { layout: layout.to_owned(), variant: variant.to_owned() })
            });
            match parsed {
                Some(l) => out.push(l),
                None => {
                    self.invalid_item("layouts", item);
                    return None;
                }
            }
        }
        Some(out)
    }

    fn options(&mut self) -> Option<Vec<String>> {
        let items = self.list("options", 0..=MAX_OPTIONS)?.clone();
        let mut out = Vec::with_capacity(items.len());
        for item in &items {
            match item.as_str().filter(|s| valid_option(s)) {
                Some(s) => out.push(s.to_owned()),
                None => {
                    self.invalid_item("options", item);
                    return None;
                }
            }
        }
        Some(out)
    }
}

impl InputConfig {
    /// Ham bölümleri doğrular ([`Self::settings`]); ham tablolar atılır (ikinci çağrı uyarısız,
    /// sonuç aynı — idempotent).
    pub fn validate(&mut self, w: &mut Vec<String>) {
        if self.keyboard.is_none() && self.mouse.is_none() && self.touchpad.is_none() {
            return;
        }
        let empty = toml::Table::new();
        let mut s = InputSettings::default();
        {
            let mut k = Section { name: "keyboard", table: self.keyboard.as_ref().unwrap_or(&empty), w };
            k.unknown(&["layouts", "options", "repeat_rate", "repeat_delay", "numlock_by_default"]);
            s.layouts = k.layouts();
            s.options = k.options();
            s.repeat_rate = k.int("repeat_rate", REPEAT_RATE);
            s.repeat_delay = k.int("repeat_delay", REPEAT_DELAY);
            s.numlock_by_default = k.bool("numlock_by_default");
        }
        {
            let mut m = Section { name: "mouse", table: self.mouse.as_ref().unwrap_or(&empty), w };
            m.unknown(&["sensitivity", "accel_profile", "natural_scroll"]);
            s.sensitivity = m.float("sensitivity", SENSITIVITY);
            s.accel_profile = m.choice("accel_profile", ACCEL_PROFILES);
            s.mouse_natural_scroll = m.bool("natural_scroll");
        }
        {
            let mut t = Section { name: "touchpad", table: self.touchpad.as_ref().unwrap_or(&empty), w };
            t.unknown(&["natural_scroll", "tap_to_click", "disable_while_typing", "scroll_factor", "scroll_method"]);
            s.touchpad_natural_scroll = t.bool("natural_scroll");
            s.tap_to_click = t.bool("tap_to_click");
            s.disable_while_typing = t.bool("disable_while_typing");
            s.scroll_factor = t.float("scroll_factor", SCROLL_FACTOR);
            s.scroll_method = t.choice("scroll_method", SCROLL_METHODS);
        }
        self.keyboard = None;
        self.mouse = None;
        self.touchpad = None;
        self.settings = s;
    }

    /// Şema (`since = 2`, `SETTING_SINCE`'teki `input.` öneki). `default` = Hyprland varsayılanı
    /// (yalnızca gösterge; dosyaya yazılmaz). Liste anahtarları `custom` türü: Ayarlar kendi
    /// bloğuyla yazar.
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let layouts_default = toml::Value::Array(vec![toml::Value::Table(toml::Table::from_iter([
            ("layout".to_owned(), toml::Value::String("us".into())),
            ("variant".to_owned(), toml::Value::String(String::new())),
        ]))]);
        let empty = toml::Value::Array(Vec::new());
        out.push(SettingSchema::custom("input.keyboard.layouts", t!("shell-config-input-layouts-label"), SettingValue::List(layouts_default)));
        out.push(SettingSchema::custom("input.keyboard.options", t!("shell-config-input-options-label"), SettingValue::List(empty)));
        let int = |key, title: String, default, r: std::ops::RangeInclusive<i64>, unit| {
            SettingSchema::int(key, title, default, *r.start(), *r.end()).unit(unit)
        };
        out.push(int("input.keyboard.repeat_rate", t!("shell-config-input-repeat-rate-label"), 25, REPEAT_RATE, "/s"));
        out.push(int("input.keyboard.repeat_delay", t!("shell-config-input-repeat-delay-label"), 600, REPEAT_DELAY, "ms").step(50.0));
        out.push(SettingSchema::bool("input.keyboard.numlock_by_default", t!("shell-config-input-numlock-label"), false));
        let float = |key, title: String, default: f32, r: std::ops::RangeInclusive<f64>, step| {
            SettingSchema::float(key, title, default, *r.start() as f32, *r.end() as f32).step(step)
        };
        out.push(float("input.mouse.sensitivity", t!("shell-config-input-sensitivity-label"), 0.0, SENSITIVITY, 0.05));
        out.push(SettingSchema::choice(
            "input.mouse.accel_profile",
            t!("shell-config-input-accel-profile-label"),
            "adaptive",
            ACCEL_PROFILES.to_vec(),
        )
        .labels(vec![t!("shell-schema-input-accel-profile-adaptive"), t!("shell-schema-input-accel-profile-flat")]));
        out.push(SettingSchema::bool("input.mouse.natural_scroll", t!("shell-config-input-mouse-natural-scroll-label"), false));
        out.push(SettingSchema::bool("input.touchpad.natural_scroll", t!("shell-config-input-touchpad-natural-scroll-label"), false));
        out.push(SettingSchema::bool("input.touchpad.tap_to_click", t!("shell-config-input-tap-to-click-label"), true));
        out.push(SettingSchema::bool("input.touchpad.disable_while_typing", t!("shell-config-input-disable-while-typing-label"), true));
        out.push(float("input.touchpad.scroll_factor", t!("shell-config-input-scroll-factor-label"), 1.0, SCROLL_FACTOR, 0.1));
        out.push(SettingSchema::choice(
            "input.touchpad.scroll_method",
            t!("shell-config-input-scroll-method-label"),
            "2fg",
            SCROLL_METHODS.to_vec(),
        )
        .labels(vec![
            t!("shell-schema-input-scroll-method-2fg"),
            t!("shell-schema-input-scroll-method-edge"),
            t!("shell-schema-input-scroll-method-on-button-down"),
            t!("shell-schema-input-scroll-method-no-scroll"),
        ]));
    }
}

// ---------------------------------------------------------------------------------------
// `msg input status`

/// Sözleşme sürümü.
pub const STATUS_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct KeyboardStatus {
    pub layouts: Option<Vec<KbLayout>>,
    pub options: Option<Vec<String>>,
    pub repeat_rate: Option<i64>,
    pub repeat_delay: Option<i64>,
    pub numlock_by_default: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MouseStatus {
    pub sensitivity: Option<f64>,
    pub accel_profile: Option<String>,
    pub natural_scroll: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TouchpadStatus {
    pub natural_scroll: Option<bool>,
    pub tap_to_click: Option<bool>,
    pub disable_while_typing: Option<bool>,
    pub scroll_factor: Option<f64>,
    pub scroll_method: Option<String>,
}

/// Geçersiz kılınan yönetilen anahtar. İki kaynak: dosya taraması (hyprlang kaynak sırası; yer
/// bilinir) ve değer karşılaştırması (uygulama bitmişken Hyprland'in etkin değeri bizim
/// ürettiğimizden farklı — Lua'da tek yol; yer bilinmez, `file`/`line` JSON'da `null`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputOverride {
    pub key: &'static str,
    pub hypr: String,
    pub file: Option<String>,
    pub line: Option<usize>,
}

/// `msg input status` yanıtı (sözleşme `input-v1`, sürüm 1).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InputStatus {
    pub v: u32,
    pub available: bool,
    pub keyboard: KeyboardStatus,
    pub mouse: MouseStatus,
    pub touchpad: TouchpadStatus,
    pub managed: Vec<&'static str>,
    pub overrides: Vec<InputOverride>,
    /// `[compositor] sync_input` (eklemeli alan): `false` iken `managed` yazılı ama
    /// uygulanmayan anahtarlardır.
    pub sync: bool,
}

impl InputStatus {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"v\":1,\"available\":false}".into())
    }
}

/// Sorgulanan Hyprland seçenekleri (her [`KEYS`] öğesi bir kez).
fn status_options() -> Vec<&'static str> {
    KEYS.iter().map(|k| k.hypr).collect()
}

/// Tek `[[BATCH]]` getoption isteği.
pub fn getoption_batch() -> String {
    let parts: Vec<String> = status_options().iter().map(|o| format!("j/getoption {o}")).collect();
    format!("[[BATCH]]{}", parts.join(";"))
}

/// Hyprland'in "boş dize" işareti.
const EMPTY: &str = "[[EMPTY]]";

/// Art arda gelen getoption JSON nesnelerini `option → değer` eşlemesine çevirir; ayrıştırılamayan
/// kısımdan sonrası atılır (bilinmeyen seçenek düz metin döner).
fn parse_options(text: &str) -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut out = std::collections::BTreeMap::new();
    let stream = serde_json::Deserializer::from_str(text).into_iter::<serde_json::Value>();
    for v in stream {
        let Ok(serde_json::Value::Object(mut o)) = v else { break };
        let Some(serde_json::Value::String(name)) = o.remove("option") else { continue };
        let value = ["str", "int", "float", "bool"].iter().find_map(|k| o.remove(*k));
        if let Some(value) = value {
            out.insert(name, value);
        }
    }
    out
}

fn split_list(s: &str) -> Vec<String> {
    if s == EMPTY || s.trim().is_empty() {
        return Vec::new();
    }
    s.split(',').map(|p| p.trim().to_owned()).collect()
}

/// Liste anahtarları (`kb_layout`/`kb_variant`/`kb_options`): öğeler kırpılır, sondaki boş öğeler
/// atılır (`","` ile `""` aynı varyant listesi); `[[EMPTY]]` boş liste.
fn list_norm(s: &str) -> Vec<&str> {
    if s == EMPTY {
        return Vec::new();
    }
    let mut v: Vec<&str> = s.split(',').map(str::trim).collect();
    while v.last().is_some_and(|p| p.is_empty()) {
        v.pop();
    }
    v
}

/// Ürettiğimiz değer Hyprland'in `getoption` değerine (normalize edilerek) eşit mi: bool ↔ 0/1,
/// ondalık ±1e-3, liste anahtarlarında [`list_norm`], dizelerde `[[EMPTY]]` = boş. Tür
/// uyuşmazsa (beklenmeyen biçim) eşit sayılır: emin olmadan override bildirilmez.
fn same_value(hypr: &str, ours: &HyprValue, live: &serde_json::Value) -> bool {
    use serde_json::Value as J;
    let as_num = |v: &J| v.as_f64().or_else(|| v.as_bool().map(|b| f64::from(u8::from(b))));
    match ours {
        HyprValue::Str(s) => match live.as_str() {
            Some(l) if RELOAD_KEYS.contains(&hypr) => list_norm(s) == list_norm(l),
            Some(l) => s.trim() == if l == EMPTY { "" } else { l.trim() },
            None => true,
        },
        HyprValue::Bool(b) => as_num(live).is_none_or(|n| (n != 0.0) == *b),
        HyprValue::Int(i) => as_num(live).is_none_or(|n| (n - *i as f64).abs() < 0.5),
        HyprValue::Float(f) => as_num(live).is_none_or(|n| (n - f).abs() <= 1e-3),
    }
}

/// Değer karşılaştırmalı override'lar: yazılı her satır için etkin değer farklıysa (yer bilinmez).
fn value_overrides(settings: &InputSettings, live: &std::collections::BTreeMap<String, serde_json::Value>) -> Vec<InputOverride> {
    settings
        .lines()
        .into_iter()
        .filter(|l| live.get(l.key.hypr).is_some_and(|v| !same_value(l.key.hypr, &l.value, v)))
        .map(|l| InputOverride { key: l.key.setting, hypr: l.key.hypr.to_owned(), file: None, line: None })
        .collect()
}

/// Etkin değerler (Hyprland'den) + `managed` + `overrides`. `ctl` yoksa ya da Hyprland cevap
/// vermezse `available: false`, tüm değerler `null`. `overrides`: dosya taramasının bulduğu
/// (hyprlang anahtarı, dosya, satır). `settled`: son eşitleme bitti ve canlı uygulandı — yalnızca
/// o zaman etkin değeri bizimkinden farklı yazılı anahtar da override sayılır (dosya taramasının
/// bulduğu anahtar için ikinci kayıt eklenmez; yeri bilinen tercih edilir).
pub fn query_status(
    ctl: Option<&dyn HyprControl>,
    settings: &InputSettings,
    sync: bool,
    overrides: &[(String, String, usize)],
    settled: bool,
) -> InputStatus {
    let managed = settings.managed();
    let overrides = overrides
        .iter()
        .filter_map(|(hypr, file, line)| {
            let key = setting_for_hypr(hypr)?;
            managed
                .contains(&key)
                .then(|| InputOverride { key, hypr: hypr.clone(), file: Some(file.clone()), line: Some(*line) })
        })
        .collect();
    let mut status = InputStatus {
        v: STATUS_VERSION,
        available: false,
        keyboard: KeyboardStatus::default(),
        mouse: MouseStatus::default(),
        touchpad: TouchpadStatus::default(),
        managed,
        overrides,
        sync,
    };
    let reply = ctl.and_then(|c| c.request(&getoption_batch()).ok());
    let Some(map) = reply.map(|t| parse_options(&t)).filter(|m| !m.is_empty()) else {
        return status;
    };
    status.available = true;
    if sync && settled {
        for o in value_overrides(settings, &map) {
            if !status.overrides.iter().any(|f| f.hypr == o.hypr) {
                status.overrides.push(o);
            }
        }
    }
    let text =|k: &str| map.get(k).and_then(|v| v.as_str()).map(|s| if s == EMPTY { String::new() } else { s.to_owned() });
    let int = |k: &str| map.get(k).and_then(|v| v.as_i64().or_else(|| v.as_bool().map(i64::from)));
    let float = |k: &str| map.get(k).and_then(serde_json::Value::as_f64).map(|f| (f * 1e6).round() / 1e6);
    let boolean = |k: &str| map.get(k).and_then(|v| v.as_bool().or_else(|| v.as_i64().map(|i| i != 0)));

    status.keyboard.layouts = map.get("input:kb_layout").and_then(|v| v.as_str()).map(|layouts| {
        let variants = text("input:kb_variant").map(|v| split_list(&v)).unwrap_or_default();
        split_list(layouts)
            .into_iter()
            .enumerate()
            .map(|(i, layout)| KbLayout { layout, variant: variants.get(i).cloned().unwrap_or_default() })
            .collect()
    });
    status.keyboard.options = text("input:kb_options").map(|s| split_list(&s).into_iter().filter(|o| !o.is_empty()).collect());
    status.keyboard.repeat_rate = int("input:repeat_rate");
    status.keyboard.repeat_delay = int("input:repeat_delay");
    status.keyboard.numlock_by_default = boolean("input:numlock_by_default");
    status.mouse.sensitivity = float("input:sensitivity");
    status.mouse.accel_profile = text("input:accel_profile");
    status.mouse.natural_scroll = boolean("input:natural_scroll");
    status.touchpad.natural_scroll = boolean("input:touchpad:natural_scroll");
    status.touchpad.tap_to_click = boolean("input:touchpad:tap-to-click");
    status.touchpad.disable_while_typing = boolean("input:touchpad:disable_while_typing");
    status.touchpad.scroll_factor = float("input:touchpad:scroll_factor");
    status.touchpad.scroll_method = text("input:scroll_method");
    status
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn parse(text: &str) -> (InputSettings, Vec<String>) {
        crate::i18n::use_turkish_for_tests();
        #[derive(serde::Deserialize)]
        struct Doc {
            input: InputConfig,
        }
        let mut doc: Doc = toml::from_str(text).unwrap();
        let mut w = Vec::new();
        doc.input.validate(&mut w);
        (doc.input.settings, w)
    }

    const FULL: &str = r#"
[input.keyboard]
layouts = [{ layout = "tr", variant = "" }, { layout = "us", variant = "intl" }]
options = ["grp:alt_shift_toggle", "caps:escape"]
repeat_rate = 25
repeat_delay = 600
numlock_by_default = false
[input.mouse]
sensitivity = -0.5
accel_profile = "flat"
natural_scroll = true
[input.touchpad]
natural_scroll = false
tap_to_click = true
disable_while_typing = true
scroll_factor = 1
scroll_method = "2fg"
"#;

    #[test]
    fn full_section_parses() {
        let (s, w) = parse(FULL);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.layouts.as_ref().unwrap()[1], KbLayout { layout: "us".into(), variant: "intl".into() });
        assert_eq!(s.options.as_deref(), Some(&["grp:alt_shift_toggle".to_owned(), "caps:escape".to_owned()][..]));
        assert_eq!((s.repeat_rate, s.repeat_delay, s.numlock_by_default), (Some(25), Some(600), Some(false)));
        assert_eq!((s.sensitivity, s.accel_profile, s.mouse_natural_scroll), (Some(-0.5), Some("flat"), Some(true)));
        assert_eq!(s.scroll_factor, Some(1.0), "tam sayı ondalık kabul edilir");
        assert_eq!(s.managed().len(), 13);
    }

    #[test]
    fn missing_section_writes_nothing() {
        let (s, w) = parse("[input]\n");
        assert!(w.is_empty() && s == InputSettings::default() && s.is_empty() && s.managed().is_empty());
        // Yalnızca bir anahtar: yalnızca onun satırı (kb_layout yok → kullanıcının `tr`'si kalır).
        let (s, _) = parse("[input.keyboard]\nrepeat_rate = 70\n");
        let lines = s.lines();
        assert_eq!(lines.len(), 1);
        assert_eq!((lines[0].key.hypr, lines[0].value.conf()), ("input:repeat_rate", "70".into()));
        assert_eq!(s.managed(), ["input.keyboard.repeat_rate"]);
    }

    #[test]
    fn invalid_values_drop_only_that_key() {
        let (s, w) = parse(
            r#"
[input.keyboard]
layouts = [{ layout = "tr;reload" }]
options = ["caps:escape", "BAD"]
repeat_rate = 0
repeat_delay = 5000
numlock_by_default = "yes"
nope = 1
[input.mouse]
sensitivity = 1.5
accel_profile = "custom"
natural_scroll = true
[input.touchpad]
scroll_factor = 0.05
scroll_method = "x"
tap_to_click = false
"#,
        );
        assert_eq!(w.len(), 10, "{w:#?}");
        assert!(w.iter().any(|m| m.contains("\"nope\"")), "{w:#?}");
        assert!(w.iter().any(|m| m.contains("input.keyboard.repeat_rate") && m.contains("1..=100")), "kırpılmaz: {w:?}");
        assert_eq!(s.layouts, None);
        assert_eq!(s.options, None);
        assert_eq!((s.repeat_rate, s.repeat_delay, s.numlock_by_default), (None, None, None));
        assert_eq!((s.sensitivity, s.accel_profile), (None, None));
        assert_eq!((s.scroll_factor, s.scroll_method), (None, None));
        // Diğerleri uygulanır.
        assert_eq!((s.mouse_natural_scroll, s.tap_to_click), (Some(true), Some(false)));
    }

    #[test]
    fn layout_and_option_rules() {
        let bad = |t: &str| parse(t).0.layouts.is_none();
        assert!(bad("[input.keyboard]\nlayouts = []\n"), "boş liste");
        assert!(bad("[input.keyboard]\nlayouts = [{ layout = \"\" }]\n"), "boş layout");
        assert!(bad("[input.keyboard]\nlayouts = [{layout=\"a\"},{layout=\"b\"},{layout=\"c\"},{layout=\"d\"},{layout=\"e\"}]\n"), "en çok 4");
        assert!(bad("[input.keyboard]\nlayouts = [{ layout = \"us\", variant = \"a b\" }]\n"));
        assert!(bad("[input.keyboard]\nlayouts = [{ layout = \"us\", model = \"x\" }]\n"), "bilinmeyen alan");
        assert!(bad("[input.keyboard]\nlayouts = \"us\"\n"));
        assert!(bad(&format!("[input.keyboard]\nlayouts = [{{ layout = \"{}\" }}]\n", "a".repeat(65))));
        let (s, w) = parse("[input.keyboard]\nlayouts = [{ layout = \"us\" }, { layout = \"de\", variant = \"nodeadkeys\" }]\n");
        assert!(w.is_empty());
        assert_eq!(s.layouts.unwrap()[0].variant, "", "variant yazılmazsa boş");
        // Seçenekler: boş liste geçerli (açıkça temizler), desen.
        let (s, w) = parse("[input.keyboard]\noptions = []\n");
        assert!(w.is_empty());
        let lines = s.lines();
        assert_eq!((lines[0].key.hypr, lines[0].value.conf()), ("input:kb_options", String::new()));
        for bad in ["caps", ":x", "Caps:escape", "caps:", "caps:esc ape", "caps:a;b", "caps:a#b"] {
            assert!(!valid_option(bad), "{bad}");
        }
        for good in ["grp:alt_shift_toggle", "lv3:ralt_switch", "compose:ralt", "grp_led:scroll", "caps:ctrl_modifier"] {
            assert!(valid_option(good), "{good}");
        }
        let many = (0..17).map(|i| format!("\"caps:x{i}\"")).collect::<Vec<_>>().join(",");
        assert!(parse(&format!("[input.keyboard]\noptions = [{many}]\n")).0.options.is_none());
    }

    #[test]
    fn layouts_and_variants_written_together() {
        let (s, _) = parse(FULL);
        let lines = s.lines();
        let get = |k: &str| lines.iter().find(|l| l.key.hypr == k).map(|l| l.value.conf());
        assert_eq!(get("input:kb_layout").as_deref(), Some("tr,us"));
        assert_eq!(get("input:kb_variant").as_deref(), Some(",intl"));
        assert_eq!(get("input:kb_options").as_deref(), Some("grp:alt_shift_toggle,caps:escape"));
        assert_eq!(get("input:sensitivity").as_deref(), Some("-0.5"));
        assert_eq!(get("input:touchpad:tap-to-click").as_deref(), Some("true"));
        assert_eq!(get("input:touchpad:scroll_factor").as_deref(), Some("1"));
        assert_eq!(float_text(0.1 + 0.2), "0.3");
        assert_eq!(float_text(-0.0), "0");
    }

    #[test]
    fn validate_is_idempotent() {
        let mut c: InputConfig = toml::from_str::<toml::Table>(FULL).unwrap()["input"].clone().try_into().unwrap();
        let mut w = Vec::new();
        c.validate(&mut w);
        let before = c.clone();
        c.validate(&mut w);
        assert!(w.is_empty() && c == before);
    }

    /// Sahte Hyprland: getoption batch'ine sabit cevap.
    struct Fake {
        reply: Result<String, String>,
        sent: RefCell<Vec<String>>,
    }

    impl HyprControl for Fake {
        fn request(&self, command: &str) -> Result<String, String> {
            self.sent.borrow_mut().push(command.to_owned());
            self.reply.clone()
        }
    }

    const REPLY: &str = concat!(
        "{\"option\": \"input:kb_layout\", \"str\": \"tr,us\", \"set\": true }\n\n\n",
        "{\"option\": \"input:kb_variant\", \"str\": \",intl\", \"set\": true }\n\n\n",
        "{\"option\": \"input:kb_options\", \"str\": \"[[EMPTY]]\", \"set\": false }\n\n\n",
        "{\"option\": \"input:repeat_rate\", \"int\": 70, \"set\": true }\n\n\n",
        "{\"option\": \"input:repeat_delay\", \"int\": 200, \"set\": true }\n\n\n",
        "{\"option\": \"input:numlock_by_default\", \"int\": 1, \"set\": true }\n\n\n",
        "{\"option\": \"input:sensitivity\", \"float\": 0.000000, \"set\": false }\n\n\n",
        "{\"option\": \"input:accel_profile\", \"str\": \"[[EMPTY]]\", \"set\": false }\n\n\n",
        "{\"option\": \"input:natural_scroll\", \"int\": 0, \"set\": false }\n\n\n",
        "{\"option\": \"input:touchpad:natural_scroll\", \"int\": 1, \"set\": true }\n\n\n",
        "{\"option\": \"input:touchpad:tap-to-click\", \"int\": 1, \"set\": false }\n\n\n",
        "{\"option\": \"input:touchpad:disable_while_typing\", \"int\": 1, \"set\": false }\n\n\n",
        "{\"option\": \"input:touchpad:scroll_factor\", \"float\": 1.000000, \"set\": false }\n\n\n",
        "{\"option\": \"input:scroll_method\", \"str\": \"[[EMPTY]]\", \"set\": false }\n",
    );

    #[test]
    fn status_json_shape() {
        let fake = Fake { reply: Ok(REPLY.into()), sent: RefCell::default() };
        let (s, _) = parse("[input.keyboard]\nlayouts = [{ layout = \"tr\" }]\nrepeat_rate = 70\n");
        let overrides = vec![
            ("input:repeat_rate".to_owned(), "~/.config/hypr/input.conf".to_owned(), 12),
            ("input:repeat_delay".to_owned(), "~/.config/hypr/input.conf".to_owned(), 13),
            ("decoration:rounding".to_owned(), "~/.config/hypr/x.conf".to_owned(), 1),
        ];
        let st = query_status(Some(&fake), &s, true, &overrides, false);
        assert_eq!(fake.sent.borrow().len(), 1, "tek istek");
        assert!(fake.sent.borrow()[0].starts_with("[[BATCH]]j/getoption input:kb_layout;j/getoption input:kb_variant;"));
        let json: serde_json::Value = serde_json::from_str(&st.to_json()).unwrap();
        let expected = serde_json::json!({
            "v": 1,
            "available": true,
            "keyboard": { "layouts": [{"layout":"tr","variant":""},{"layout":"us","variant":"intl"}], "options": [],
                          "repeat_rate": 70, "repeat_delay": 200, "numlock_by_default": true },
            "mouse": { "sensitivity": 0.0, "accel_profile": "", "natural_scroll": false },
            "touchpad": { "natural_scroll": true, "tap_to_click": true, "disable_while_typing": true,
                          "scroll_factor": 1.0, "scroll_method": "" },
            "managed": ["input.keyboard.layouts", "input.keyboard.repeat_rate"],
            "overrides": [{"key": "input.keyboard.repeat_rate", "hypr": "input:repeat_rate", "file": "~/.config/hypr/input.conf", "line": 12}],
            "sync": true,
        });
        assert_eq!(json, expected);
    }

    #[test]
    fn status_without_hyprland_is_null() {
        let (s, _) = parse("[input.mouse]\nnatural_scroll = true\n");
        for st in [
            query_status(None, &s, false, &[], false),
            query_status(Some(&Fake { reply: Err("bağlanılamadı".into()), sent: RefCell::default() }), &s, false, &[], false),
            query_status(Some(&Fake { reply: Ok("unknown request".into()), sent: RefCell::default() }), &s, false, &[], false),
        ] {
            let json: serde_json::Value = serde_json::from_str(&st.to_json()).unwrap();
            assert_eq!(json["available"], false);
            assert_eq!(json["v"], 1);
            for (section, n) in [("keyboard", 5), ("mouse", 3), ("touchpad", 5)] {
                let o = json[section].as_object().unwrap();
                assert_eq!(o.len(), n);
                assert!(o.values().all(serde_json::Value::is_null), "{section}: {o:?}");
            }
            assert_eq!(json["managed"], serde_json::json!(["input.mouse.natural_scroll"]));
            assert_eq!(json["sync"], false);
        }
        // Kısmi cevap: bilinmeyen seçenekten sonrası null, cevap gelen değerler dolu.
        let partial = Fake { reply: Ok("{\"option\":\"input:kb_layout\",\"str\":\"us\",\"set\":false}\nno such option".into()), sent: RefCell::default() };
        let st = query_status(Some(&partial), &s, true, &[], false);
        assert!(st.available);
        assert_eq!(st.keyboard.layouts, Some(vec![KbLayout { layout: "us".into(), variant: String::new() }]));
        assert_eq!(st.keyboard.repeat_rate, None);
    }

    fn fake_reply() -> Fake {
        Fake { reply: Ok(REPLY.into()), sent: RefCell::default() }
    }

    /// Uygulama bitmişken etkin değeri bizimkinden farklı yazılı anahtar: yeri bilinmeyen override.
    #[test]
    fn value_mismatch_is_override_when_settled() {
        let (s, _) = parse(
            "[input.keyboard]\nrepeat_rate = 70\nrepeat_delay = 600\n[input.mouse]\naccel_profile = \"flat\"\n[input.touchpad]\ntap_to_click = false\n",
        );
        let st = query_status(Some(&fake_reply()), &s, true, &[], true);
        let keys: Vec<_> = st.overrides.iter().map(|o| (o.key, o.hypr.as_str())).collect();
        assert_eq!(
            keys,
            [
                ("input.keyboard.repeat_delay", "input:repeat_delay"),
                ("input.mouse.accel_profile", "input:accel_profile"),
                ("input.touchpad.tap_to_click", "input:touchpad:tap-to-click"),
            ],
            "repeat_rate eşit (70) → override değil"
        );
        let json: serde_json::Value = serde_json::from_str(&st.to_json()).unwrap();
        assert_eq!(
            json["overrides"][0],
            serde_json::json!({"key": "input.keyboard.repeat_delay", "hypr": "input:repeat_delay", "file": null, "line": null})
        );
        // sync_input kapalıyken satır üretilmez: settled gelse de uyuşmazlık override değil.
        assert!(query_status(Some(&fake_reply()), &s, false, &[], true).overrides.is_empty());
    }

    /// Uygulama sürüyor (ya da canlı uygulanmadı): uyuşmazlık henüz override sayılmaz.
    #[test]
    fn value_mismatch_ignored_while_applying() {
        let (s, _) = parse("[input.keyboard]\nrepeat_delay = 600\n");
        let st = query_status(Some(&fake_reply()), &s, true, &[], false);
        assert!(st.available && st.overrides.is_empty(), "{:?}", st.overrides);
        assert_eq!(st.keyboard.repeat_delay, Some(200), "etkin değer yine döner");
    }

    /// Normalize eşitlikler: bool ↔ 0/1, ondalık ±1e-3, liste kırpma/sondaki boşlar, `[[EMPTY]]`.
    #[test]
    fn value_compare_normalizes() {
        use serde_json::json;
        let b = HyprValue::Bool;
        assert!(same_value("input:numlock_by_default", &b(true), &json!(1)));
        assert!(same_value("input:numlock_by_default", &b(true), &json!(true)));
        assert!(same_value("input:natural_scroll", &b(false), &json!(0)));
        assert!(!same_value("input:natural_scroll", &b(false), &json!(1)));
        assert!(same_value("input:repeat_rate", &HyprValue::Int(25), &json!(25)));
        assert!(!same_value("input:repeat_rate", &HyprValue::Int(25), &json!(26)));
        let f = HyprValue::Float;
        assert!(same_value("input:sensitivity", &f(-0.5), &json!(-0.500_000)));
        assert!(same_value("input:touchpad:scroll_factor", &f(1.0), &json!(1.000_9)));
        assert!(!same_value("input:touchpad:scroll_factor", &f(1.0), &json!(1.01)));
        let s = |v: &str| HyprValue::Str(v.into());
        assert!(same_value("input:kb_layout", &s("tr,us"), &json!(" tr , us ")));
        assert!(!same_value("input:kb_layout", &s("tr,us"), &json!("us,tr")), "sıra önemli");
        assert!(same_value("input:kb_variant", &s(","), &json!("[[EMPTY]]")), "boş varyantlar");
        assert!(same_value("input:kb_variant", &s(",intl"), &json!(",intl")));
        assert!(same_value("input:kb_options", &s(""), &json!("[[EMPTY]]")));
        assert!(same_value("input:kb_options", &s("grp:alt_shift_toggle,caps:escape"), &json!("grp:alt_shift_toggle, caps:escape")));
        assert!(!same_value("input:kb_options", &s(""), &json!("caps:escape")));
        assert!(same_value("input:accel_profile", &s("flat"), &json!("flat")));
        assert!(!same_value("input:accel_profile", &s("adaptive"), &json!("[[EMPTY]]")), "boş = libinput varsayılanı, aynı değil");
        assert!(same_value("input:repeat_rate", &HyprValue::Int(25), &json!("garip")), "beklenmeyen tür: override bildirilmez");

        // Uçtan uca: REPLY'ın tüm değerlerini yazan ayar → override yok.
        let (all, w) = parse(
            r#"
[input.keyboard]
layouts = [{ layout = "tr" }, { layout = "us", variant = "intl" }]
options = []
repeat_rate = 70
repeat_delay = 200
numlock_by_default = true
[input.mouse]
sensitivity = 0.0005
natural_scroll = false
[input.touchpad]
natural_scroll = true
tap_to_click = true
disable_while_typing = true
scroll_factor = 1.0
"#,
        );
        assert!(w.is_empty(), "{w:?}");
        let st = query_status(Some(&fake_reply()), &all, true, &[], true);
        assert!(st.overrides.is_empty(), "{:?}", st.overrides);
    }

    /// Dosya taramasının bulduğu anahtar değer karşılaştırmasında ikinci kez eklenmez (yeri
    /// bilinen kalır); bulmadığı uyuşmazlık eklenir.
    #[test]
    fn value_override_not_duplicated() {
        let (s, _) = parse("[input.keyboard]\nrepeat_rate = 25\nrepeat_delay = 600\n");
        let file = vec![("input:repeat_rate".to_owned(), "~/.config/hypr/input.conf".to_owned(), 12)];
        let st = query_status(Some(&fake_reply()), &s, true, &file, true);
        assert_eq!(
            st.overrides,
            [
                InputOverride {
                    key: "input.keyboard.repeat_rate",
                    hypr: "input:repeat_rate".into(),
                    file: Some("~/.config/hypr/input.conf".into()),
                    line: Some(12),
                },
                InputOverride { key: "input.keyboard.repeat_delay", hypr: "input:repeat_delay".into(), file: None, line: None },
            ]
        );
    }

    #[test]
    fn hypr_key_mapping() {
        assert_eq!(setting_for_hypr("input:kb_variant"), Some("input.keyboard.layouts"));
        assert_eq!(setting_for_hypr("input:touchpad:tap-to-click"), Some("input.touchpad.tap_to_click"));
        assert_eq!(setting_for_hypr("input:scroll_method"), Some("input.touchpad.scroll_method"));
        assert_eq!(setting_for_hypr("general:gaps_in"), None);
        for k in KEYS {
            assert!(super::super::generate::MANAGED_KEYS.contains(&k.hypr), "{}", k.hypr);
            assert_eq!(k.hypr, k.hypr.to_ascii_lowercase(), "tarama anahtarı küçük harfle karşılaştırır");
        }
    }
}
