//! `nuxut-shell schema`: ayarlar uygulamasının okuduğu makine-okunur şema (TOML).
//!
//! Çıktı: `version`, `config_version` (shell.toml'un güncel sürümü, ADR-0002), `[[widget]]` (her
//! [`WidgetKind`], `ALL` sırasıyla — ayarların "ekle" menüsünün sırası) ve `[[setting]]`
//! (`shell.toml`'un genel anahtarları, noktalı yol). Widget ve ayarlarda `since`: hangi
//! `config_version` ile geldiği (yalnızca 2 ve sonrası yazılır; yoksa 1) — Ayarlar "Yeni" rozeti.
//! Kaynak tektir: varsayılanlar config yapılarının `Default`'undan, sınırlar doğrulamanın
//! kullandığı sabitlerden gelir; şemaya elle sayı yazılmaz. Yeni config anahtarı eklenince
//! o bölümün `schema` fonksiyonuna da eklenir (testler config'te olmayan anahtarı ve
//! doğrulamayla çelişen varsayılan/aralığı yakalar).

use std::borrow::Cow;

use serde::Serialize;

use crate::config::ShellConfig;
use crate::config_migrate::CURRENT_CONFIG_VERSION;
use crate::widgets::WidgetKind;

/// Şema biçiminin sürümü; uyumsuz değişiklikte artar. (`config_version` ve `since` eklemeli
/// alanlardır; Ayarlar bilinmeyen alanları yok sayar.)
pub const SCHEMA_VERSION: u32 = 1;

/// `unit` alanının kapalı kümesi: dile bağımsız semboller (piksel, yüzde, milisaniye, saniye, dakika,
/// saniyede — ör. tuş tekrarı, Kelvin — gece ışığı sıcaklığı; SI sembolü, çevrilmez).
/// Şema çıktısı locale'den bağımsız kalır; Ayarlar sembolü kendi kataloğundan gösterir.
pub const UNITS: [&str; 7] = ["px", "%", "ms", "s", "min", "/s", "K"];

/// `config_version` 1'den sonra eklenen widget'lar: (widget, geldiği sürüm). Yeni widget
/// eklenince buraya [`CURRENT_CONFIG_VERSION`] ile yazılır (sürüm o değişiklikte artmıyorsa
/// bir sonraki göç sürümü kullanılır).
/// Sürüm 1 = `config_version`'dan önceki, kullanıcının kurulu shell'inin (stable c567983)
/// anahtarları; aşağıdakiler v2 geliştirmesinde geldi.
const WIDGET_SINCE: &[(WidgetKind, u32)] = &[
    (WidgetKind::Launcher, 2),
    (WidgetKind::Sysmon, 2),
    (WidgetKind::Status, 2),
    (WidgetKind::Logo, 2),
    (WidgetKind::Calendar, 2),
    (WidgetKind::NightLight, 2),
    (WidgetKind::Magnifier, 2),
    (WidgetKind::NowPlaying, 2),
    (WidgetKind::Toggles, 2),
    (WidgetKind::Osk, 2),
    (WidgetKind::Privacy, 2),
    (WidgetKind::Caffeine, 2),
    (WidgetKind::Mixer, 2),
    (WidgetKind::KeyboardLayout, 2),
    (WidgetKind::NextEvent, 2),
];

/// v1'den sonra gelen genel ayarlar: noktalı yol (`.` ile bitiyorsa tablo öneki) → sürüm.
/// Ayar tanımı kendi bölümünde kalır; "Yeni" rozeti tek yerden, [`SettingSchema::since`] ile.
const SETTING_SINCE: &[(&str, u32)] = &[
    ("bar.edge_actions.", 2),
    ("audio.backend", 2),
    ("launcher.", 2),
    ("compositor.", 2),
    ("windows.", 2),
    ("keybinds.", 2),
    ("setup.", 2),
    ("sysmon.", 2),
    ("wallpaper.backdrop", 2),
    ("wallpaper.backdrop_blur", 2),
    ("wallpaper.backdrop_color", 2),
    ("idle.", 2),
    ("screenshot.", 2),
    ("input.", 2),
    ("clipboard.", 2),
    ("night_light.", 2),
    ("magnifier.", 2),
    ("osk.", 2),
];

/// v1'den sonra gelen widget ayarları: (widget, tablo içi anahtar, sürüm).
const WIDGET_SETTING_SINCE: &[(WidgetKind, &str, u32)] = &[
    (WidgetKind::Workspaces, "style", 2),
    (WidgetKind::Workspaces, "scroll", 2),
    (WidgetKind::ActiveWindow, "max_chars", 2),
];

fn widget_since(kind: WidgetKind) -> Option<u32> {
    WIDGET_SINCE.iter().find(|(k, _)| *k == kind).map(|&(_, v)| v)
}

fn setting_matches(pattern: &str, key: &str) -> bool {
    if pattern.ends_with('.') { key.starts_with(pattern) } else { key == pattern }
}

/// Genel ayarın sürümü ([`SETTING_SINCE`]); tanımda verilmişse o kalır.
fn with_setting_since(s: SettingSchema) -> SettingSchema {
    match SETTING_SINCE.iter().find(|(p, _)| setting_matches(p, &s.key)) {
        Some(&(_, v)) if s.since.is_none() => s.since(v),
        _ => s,
    }
}

fn with_widget_setting_since(kind: WidgetKind, s: SettingSchema) -> SettingSchema {
    match WIDGET_SETTING_SINCE.iter().find(|&&(k, key, _)| k == kind && key == s.key) {
        Some(&(_, _, v)) if s.since.is_none() => s.since(v),
        _ => s,
    }
}

/// Ayarın değer türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingKind {
    Bool,
    Int,
    Float,
    Text,
    /// `options` içinden bir dize.
    Choice,
    /// Yapılı değer (liste/tablo listesi; ör. `input.keyboard.layouts`): Ayarlar kendi
    /// bloğuyla yazar, `default` aynı biçimde bir TOML değeridir.
    Custom,
}

/// Varsayılan değer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    /// [`SettingKind::Custom`] varsayılanı (satır içi TOML dizisi).
    List(toml::Value),
}

/// Sınır ya da adım. Türü ayarın türünü izler: `int` ayarda TOML tam sayısı (`1`), `float`
/// ayarda ondalık (`1.0`) yazılır.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SettingNumber {
    Int(i64),
    Float(f64),
}

impl SettingNumber {
    pub fn as_f64(self) -> f64 {
        match self {
            Self::Int(i) => i as f64,
            Self::Float(f) => f,
        }
    }
}

/// Bir ayarın şeması. Kurucular türle değeri birlikte alır; tür/değer uyuşmazlığı yazılamaz.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingSchema {
    /// Widget ayarında tablo içi anahtar (`count`), genel ayarda noktalı yol (`bar.size`).
    pub key: String,
    pub title: Cow<'static, str>,
    pub kind: SettingKind,
    pub default: SettingValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<SettingNumber>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<SettingNumber>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<SettingNumber>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<&'static str>,
    /// `options`'ın görünen adları, aynı sırayla (etkin dilde; eklemeli alan). Değerler
    /// config'e yazılır, adlar yalnızca gösterilir.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub option_labels: Vec<String>,
    /// Geldiği `config_version` (2+; yoksa 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<u32>,
    /// Ayarlarda başlığın altında gösterilen kısa açıklama (isteğe bağlı; eklemeli alan).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Cow<'static, str>>,
}

impl SettingSchema {
    fn new(key: impl Into<String>, title: impl Into<Cow<'static, str>>, kind: SettingKind, default: SettingValue) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            kind,
            default,
            min: None,
            max: None,
            step: None,
            unit: None,
            options: Vec::new(),
            option_labels: Vec::new(),
            since: None,
            description: None,
        }
    }

    /// Ayarın geldiği `config_version` (Ayarlar "Yeni" rozeti). 1 yazılmaz.
    pub fn since(mut self, version: u32) -> Self {
        self.since = (version > 1).then_some(version);
        self
    }

    pub fn bool(key: impl Into<String>, title: impl Into<Cow<'static, str>>, default: bool) -> Self {
        Self::new(key, title, SettingKind::Bool, SettingValue::Bool(default))
    }

    /// Tamsayı, `min..=max`, adım 1.
    pub fn int(key: impl Into<String>, title: impl Into<Cow<'static, str>>, default: i64, min: i64, max: i64) -> Self {
        let mut s = Self::new(key, title, SettingKind::Int, SettingValue::Int(default));
        let int = SettingNumber::Int;
        (s.min, s.max, s.step) = (Some(int(min)), Some(int(max)), Some(int(1)));
        s
    }

    /// Ondalık, `min..=max`, adım 1.
    pub fn float(key: impl Into<String>, title: impl Into<Cow<'static, str>>, default: f32, min: f32, max: f32) -> Self {
        let mut s = Self::new(key, title, SettingKind::Float, SettingValue::Float(default.into()));
        let float = SettingNumber::Float;
        (s.min, s.max, s.step) = (Some(float(min.into())), Some(float(max.into())), Some(float(1.0)));
        s
    }

    pub fn text(key: impl Into<String>, title: impl Into<Cow<'static, str>>, default: impl Into<String>) -> Self {
        Self::new(key, title, SettingKind::Text, SettingValue::Text(default.into()))
    }

    pub fn choice(key: impl Into<String>, title: impl Into<Cow<'static, str>>, default: &'static str, options: Vec<&'static str>) -> Self {
        let mut s = Self::new(key, title, SettingKind::Choice, SettingValue::Text(default.into()));
        s.options = options;
        s
    }

    /// Yapılı değer ([`SettingKind::Custom`]); varsayılan bir TOML dizisi.
    pub fn custom(key: impl Into<String>, title: impl Into<Cow<'static, str>>, default: SettingValue) -> Self {
        debug_assert!(matches!(default, SettingValue::List(_)));
        Self::new(key, title, SettingKind::Custom, default)
    }

    /// Adım; tamsayı ayarda tam sayıya yuvarlanır (en az 1).
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(match self.kind {
            SettingKind::Int => SettingNumber::Int((step.round() as i64).max(1)),
            _ => SettingNumber::Float(step),
        });
        self
    }

    pub fn describe(mut self, description: impl Into<Cow<'static, str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Birim [`UNITS`] kümesinden bir semboldür; çevirisini gösteren taraf (Ayarlar) yapar.
    pub fn unit(mut self, unit: &'static str) -> Self {
        self.unit = Some(unit);
        self
    }

    /// Seçeneklerin görünen adları (`options` ile aynı sıra ve uzunluk).
    pub fn labels(mut self, labels: Vec<String>) -> Self {
        debug_assert_eq!(labels.len(), self.options.len(), "{}: her seçeneğin bir adı olur", self.key);
        self.option_labels = labels;
        self
    }
}

#[derive(Serialize)]
struct WidgetSchema {
    name: &'static str,
    title: String,
    /// Framework `IconName` dizgesi.
    icon: &'static str,
    bar: bool,
    panel: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<u32>,
    setting: Vec<SettingSchema>,
}

#[derive(Serialize)]
struct Document {
    version: u32,
    config_version: u32,
    widget: Vec<WidgetSchema>,
    setting: Vec<SettingSchema>,
}

/// Şemanın TOML metni.
pub fn generate() -> Result<String, toml::ser::Error> {
    let widget = WidgetKind::ALL
        .into_iter()
        .map(|kind| {
            let placement = kind.placement();
            let mut setting = Vec::new();
            kind.settings_schema(&mut setting);
            let setting = setting.into_iter().map(|s| with_widget_setting_since(kind, s)).collect();
            WidgetSchema {
                name: kind.name(),
                title: kind.title(),
                icon: kind.icon().as_str(),
                bar: placement.bar,
                panel: placement.panel,
                since: widget_since(kind),
                setting,
            }
        })
        .collect();
    let mut setting = Vec::new();
    ShellConfig::schema(&mut setting);
    let setting = setting.into_iter().map(with_setting_since).collect();
    let text = toml::to_string(&Document { version: SCHEMA_VERSION, config_version: CURRENT_CONFIG_VERSION, widget, setting })?;
    Ok(inline_custom_defaults(text))
}

/// `custom` ayarların tablo listesi varsayılanını (`[[setting.default]]`) satır içi diziye çevirir:
/// Ayarlar `default`'u değer olarak okur (`default = [{ layout = "us", variant = "" }]`).
fn inline_custom_defaults(text: String) -> String {
    let Ok(mut doc) = text.parse::<toml_edit::DocumentMut>() else { return text };
    let Some(settings) = doc.get_mut("setting").and_then(toml_edit::Item::as_array_of_tables_mut) else { return text };
    let mut changed = false;
    for t in settings.iter_mut() {
        if let Some(aot) = t.get("default").and_then(toml_edit::Item::as_array_of_tables).cloned() {
            let mut array = aot.into_array();
            array.fmt();
            t.insert("default", toml_edit::Item::Value(toml_edit::Value::Array(array)));
            changed = true;
        }
    }
    if changed { doc.to_string() } else { text }
}

#[cfg(test)]
mod tests {
    use nuxut::ui::IconName;

    use super::*;

    fn doc() -> toml::Table {
        generate().expect("şema üretilir").parse().expect("şema geçerli TOML")
    }

    fn literal(v: &SettingValue) -> String {
        match v {
            SettingValue::Bool(b) => toml::Value::Boolean(*b).to_string(),
            SettingValue::Int(i) => toml::Value::Integer(*i).to_string(),
            SettingValue::Float(f) => toml::Value::Float(*f).to_string(),
            SettingValue::Text(t) => toml::Value::String(t.clone()).to_string(),
            SettingValue::List(v) => v.to_string(),
        }
    }

    /// `[input]` anahtarları isteğe bağlıdır (sözleşme input-v1): yazılınca config'te `Some`
    /// olur, şema varsayılanı yalnızca Hyprland varsayılanının göstergesidir.
    fn optional_key(key: &str) -> bool {
        key.starts_with("input.")
    }

    fn general() -> Vec<SettingSchema> {
        let mut out = Vec::new();
        ShellConfig::schema(&mut out);
        out
    }

    fn widget(kind: WidgetKind) -> Vec<SettingSchema> {
        let mut out = Vec::new();
        kind.settings_schema(&mut out);
        out
    }

    /// `key = value` satırını içeren shell.toml (widget ayarı ise kendi tablosunda).
    /// Şema güncel sürümü anlatır: dosya da güncel sürümlüdür (göç devreye girmez).
    fn config_with(kind: Option<WidgetKind>, key: &str, value: &str) -> String {
        let v = CURRENT_CONFIG_VERSION;
        match kind {
            Some(k) => format!("config_version = {v}\n[bar.widget.{}]\n{key} = {value}\n", k.name()),
            None => format!("config_version = {v}\n{key} = {value}\n"),
        }
    }

    fn all_settings() -> Vec<(Option<WidgetKind>, SettingSchema)> {
        let mut all: Vec<_> = general().into_iter().map(|s| (None, s)).collect();
        for k in WidgetKind::ALL {
            all.extend(widget(k).into_iter().map(|s| (Some(k), s)));
        }
        all
    }

    #[test]
    fn config_version_and_since() {
        let d = doc();
        assert_eq!(d["config_version"].as_integer(), Some(i64::from(CURRENT_CONFIG_VERSION)));
        let current = i64::from(CURRENT_CONFIG_VERSION);
        let mut all: Vec<&toml::Value> = d["setting"].as_array().unwrap().iter().collect();
        for w in d["widget"].as_array().unwrap() {
            all.push(w);
            all.extend(w["setting"].as_array().unwrap());
        }
        for s in all {
            if let Some(v) = s.get("since") {
                let v = v.as_integer().expect("since tam sayı");
                assert!((2..=current).contains(&v), "since = {v}: 2..={current}");
            }
        }
        assert_eq!(SettingSchema::bool("k", "t", true).since(1).since, None, "1 yazılmaz");
        assert_eq!(SettingSchema::bool("k", "t", true).since(2).since, Some(2));
        assert!(WIDGET_SINCE.iter().all(|&(_, v)| (2..=CURRENT_CONFIG_VERSION).contains(&v)));
    }

    /// v2'de gelen ayar/widget'lar `since = 2` taşır, v1'dekiler taşımaz; her tablo girdisi
    /// gerçek bir ayara karşılık gelir (eskimiş girdi kalmaz).
    #[test]
    fn v2_additions_carry_since() {
        let d = doc();
        let since = |s: &toml::Value| s.get("since").and_then(toml::Value::as_integer);
        let settings = d["setting"].as_array().unwrap();
        let general = |key: &str| settings.iter().find(|s| s["key"].as_str() == Some(key)).unwrap_or_else(|| panic!("{key}"));
        for key in ["launcher.enabled", "compositor.sync_look", "windows.gap_inner", "keybinds.volume_up", "bar.edge_actions.scroll_step", "audio.backend"] {
            assert_eq!(since(general(key)), Some(2), "{key}");
        }
        for key in ["bar.size", "frame.thickness", "audio.max_volume", "osd.timeout_ms", "notifications.dnd"] {
            assert_eq!(since(general(key)), None, "{key}");
        }
        let widget_doc = |name: &str| d["widget"].as_array().unwrap().iter().find(|w| w["name"].as_str() == Some(name)).unwrap().clone();
        assert_eq!(since(&widget_doc("launcher")), Some(2));
        assert_eq!(since(&widget_doc("clock")), None);
        let ws = widget_doc("workspaces");
        let ws_setting = |key: &str| ws["setting"].as_array().unwrap().iter().find(|s| s["key"].as_str() == Some(key)).unwrap().clone();
        assert_eq!(since(&ws_setting("style")), Some(2));
        assert_eq!(since(&ws_setting("count")), None);
        for (pattern, _) in SETTING_SINCE {
            assert!(general_keys().iter().any(|k| setting_matches(pattern, k)), "eskimiş SETTING_SINCE: {pattern}");
        }
        for &(kind, key, _) in WIDGET_SETTING_SINCE {
            assert!(widget(kind).iter().any(|s| s.key == key), "eskimiş WIDGET_SETTING_SINCE: {}.{key}", kind.name());
        }
    }

    fn general_keys() -> Vec<String> {
        general().into_iter().map(|s| s.key).collect()
    }

    #[test]
    fn document_shape() {
        let d = doc();
        assert_eq!(d["version"].as_integer(), Some(1));
        assert!(generate().unwrap().starts_with("version = 1\n"), "sürüm en üstte");
        let widgets = d["widget"].as_array().unwrap();
        let names: Vec<_> = widgets.iter().map(|w| w["name"].as_str().unwrap()).collect();
        let expected: Vec<_> = WidgetKind::ALL.iter().map(|k| k.name()).collect();
        assert_eq!(names, expected, "her WidgetKind, ALL sırasıyla");
        for w in widgets {
            assert!(w["bar"].is_bool() && w["panel"].is_bool(), "bar/panel açıkça yazılır");
            assert!(!w["title"].as_str().unwrap().is_empty());
            for s in w["setting"].as_array().unwrap() {
                assert!(s.get("key").and_then(|k| k.as_str()).is_some_and(|k| !k.contains('.')));
            }
        }
        assert!(!d["setting"].as_array().unwrap().is_empty());
    }

    /// Ayarlar "Paneller → Ekle" listesi `panel = true` widget'lardan gelir: takvim kartı
    /// orada, bar listesinde değil; ayarı yok, v2'de geldi ("Yeni" rozeti).
    #[test]
    fn calendar_card_is_panel_only() {
        let d = doc();
        let w = d["widget"].as_array().unwrap().iter().find(|w| w["name"].as_str() == Some("calendar")).expect("calendar");
        assert_eq!(w["panel"].as_bool(), Some(true));
        assert_eq!(w["bar"].as_bool(), Some(false));
        assert_eq!(w["icon"].as_str(), Some("calendar_month"));
        assert_eq!(w["since"].as_integer(), Some(2));
        assert!(w["setting"].as_array().unwrap().is_empty());
        // Config de yalnızca panele kabul eder.
        let (c, warn) = ShellConfig::parse("[panels.left]\nwidgets = [\"calendar\"]\n[bar]\nright = [\"calendar\"]\n").unwrap();
        assert_eq!(c.panels.left.widget_kinds(), [WidgetKind::Calendar]);
        assert!(warn.iter().any(|w| w.contains("calendar")), "{warn:?}");
    }

    #[test]
    fn icons_are_framework_icon_names() {
        for k in WidgetKind::ALL {
            let s = k.icon().as_str();
            assert_eq!(s.parse::<IconName>().map(IconName::as_str), Ok(s), "{}", k.name());
        }
    }

    #[test]
    fn keys_are_unique() {
        let mut keys: Vec<_> = general().into_iter().map(|s| s.key).collect();
        let n = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), n);
        for k in WidgetKind::ALL {
            let mut keys: Vec<_> = widget(k).into_iter().map(|s| s.key).collect();
            let n = keys.len();
            keys.sort();
            keys.dedup();
            assert_eq!(keys.len(), n, "{}", k.name());
        }
    }

    /// Her anahtar config'te var ve varsayılanı gerçek varsayılanla aynı: yalnız o anahtarı
    /// varsayılanıyla yazan shell.toml uyarısız ayrışır ve doğrulanmış varsayılana eşittir
    /// (widget ayarında: çözülmüş widget ayarları; ham tablo yazılan anahtarı tutar).
    #[test]
    fn keys_exist_and_defaults_match_config() {
        let default = ShellConfig::validated_default();
        for (kind, s) in all_settings() {
            let text = config_with(kind, &s.key, &literal(&s.default));
            let (c, w) = ShellConfig::parse(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert!(w.is_empty(), "{text}: {w:?}");
            match kind {
                Some(_) => assert_eq!(c.bar.widgets, default.bar.widgets, "{text}"),
                None if optional_key(&s.key) => {
                    assert_eq!(c.input.settings.managed(), [s.key.as_str()], "{text}: yazılan anahtar yönetilir");
                    let mut c = c;
                    c.input = default.input.clone();
                    assert_eq!(c, default, "{text}");
                }
                None => assert_eq!(c, default, "{text}"),
            }
        }
    }

    /// Aralık doğrulamayla aynı: uçlar uyarısız kabul edilir, dışı reddedilir ya da düzeltilir.
    #[test]
    fn ranges_match_validation() {
        for (kind, s) in all_settings() {
            let (Some(min), Some(max)) = (s.min.map(SettingNumber::as_f64), s.max.map(SettingNumber::as_f64)) else {
                assert!(!matches!(s.kind, SettingKind::Int | SettingKind::Float), "{} sınırsız", s.key);
                continue;
            };
            assert!(min <= max && s.step.is_some_and(|st| st.as_f64() > 0.0), "{}", s.key);
            let lit = |v: f64| match s.kind {
                SettingKind::Int => literal(&SettingValue::Int(v as i64)),
                _ => literal(&SettingValue::Float(v)),
            };
            for edge in [min, max] {
                let text = config_with(kind, &s.key, &lit(edge));
                let (_, w) = ShellConfig::parse(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
                assert!(w.is_empty(), "{text}: {w:?}");
            }
            for outside in [min - 1.0, max + 1.0] {
                let text = config_with(kind, &s.key, &lit(outside));
                if let Ok((_, w)) = ShellConfig::parse(&text) {
                    assert!(!w.is_empty(), "{text}: aralık dışı kabul edildi");
                }
            }
        }
    }

    /// Sınır ve adımın TOML türü ayarın türüyle aynı: `int` ayarda tam sayı (`min = 1`, `1.0`
    /// değil), `float` ayarda ondalık. Hem genel hem widget ayarları.
    #[test]
    fn bounds_follow_setting_kind() {
        let d = doc();
        let mut tables: Vec<&toml::Value> = d["setting"].as_array().unwrap().iter().collect();
        for w in d["widget"].as_array().unwrap() {
            tables.extend(w["setting"].as_array().unwrap());
        }
        let mut seen = (false, false);
        for s in tables {
            let kind = s["kind"].as_str().unwrap();
            for field in ["min", "max", "step"] {
                let Some(v) = s.get(field) else { continue };
                match kind {
                    "int" => {
                        seen.0 = true;
                        assert!(v.is_integer(), "{}.{field} = {v}: tam sayı olmalı", s["key"]);
                    }
                    "float" => {
                        seen.1 = true;
                        assert!(v.is_float(), "{}.{field} = {v}: ondalık olmalı", s["key"]);
                    }
                    _ => panic!("{}: {kind} türünde {field} olmamalı", s["key"]),
                }
            }
        }
        assert!(seen.0 && seen.1, "her iki tür de sınanır");
        let count = SettingSchema::int("k", "t", 10, 1, 20).step(500.0);
        assert_eq!(
            (count.min, count.max, count.step),
            (Some(SettingNumber::Int(1)), Some(SettingNumber::Int(20)), Some(SettingNumber::Int(500)))
        );
        assert_eq!(SettingSchema::int("k", "t", 1, 1, 2).step(0.2).step, Some(SettingNumber::Int(1)), "tamsayı adımı en az 1");
    }

    #[test]
    fn units_are_symbols_from_closed_set() {
        for (_, s) in all_settings() {
            if let Some(unit) = s.unit {
                assert!(UNITS.contains(&unit), "{}: bilinmeyen birim {unit:?}", s.key);
            }
        }
    }

    #[test]
    fn choices_are_complete_and_valid() {
        for (kind, s) in all_settings() {
            if s.kind != SettingKind::Choice {
                assert!(s.options.is_empty() && s.option_labels.is_empty(), "{}", s.key);
                continue;
            }
            let SettingValue::Text(default) = &s.default else { panic!("{}: choice varsayılanı dize", s.key) };
            assert!(s.options.contains(&default.as_str()), "{}: varsayılan options içinde", s.key);
            assert_eq!(s.option_labels.len(), s.options.len(), "{}: her seçeneğin görünen adı var", s.key);
            assert!(s.option_labels.iter().all(|l| !l.is_empty()), "{}: boş seçenek adı", s.key);
            for o in &s.options {
                let text = config_with(kind, &s.key, &literal(&SettingValue::Text((*o).into())));
                let (_, w) = ShellConfig::parse(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
                assert!(w.is_empty(), "{text}: {w:?}");
            }
            // Genel anahtarda ayrıştırma hatası, widget ayarında uyarıyla düzeltme.
            let text = config_with(kind, &s.key, "\"nuxut-no-such-option\"");
            if let Ok((_, w)) = ShellConfig::parse(&text) {
                assert!(!w.is_empty(), "{text}: bilinmeyen seçenek uyarısız kabul edildi");
            }
        }
    }

    /// `[input]`: sözleşmedeki anahtarlar `since = 2` ile; liste anahtarları `custom` türünde,
    /// varsayılanları satır içi TOML değeri (Ayarlar `default`'u değer olarak okur).
    #[test]
    fn input_settings() {
        let d = doc();
        let settings = d["setting"].as_array().unwrap();
        let get = |key: &str| settings.iter().find(|s| s["key"].as_str() == Some(key)).unwrap_or_else(|| panic!("{key}"));
        for key in [
            "compositor.sync_input",
            "input.keyboard.layouts",
            "input.keyboard.options",
            "input.keyboard.repeat_rate",
            "input.keyboard.repeat_delay",
            "input.keyboard.numlock_by_default",
            "input.mouse.sensitivity",
            "input.mouse.accel_profile",
            "input.mouse.natural_scroll",
            "input.touchpad.natural_scroll",
            "input.touchpad.tap_to_click",
            "input.touchpad.disable_while_typing",
            "input.touchpad.scroll_factor",
            "input.touchpad.scroll_method",
        ] {
            assert_eq!(get(key).get("since").and_then(toml::Value::as_integer), Some(2), "{key}");
        }
        let layouts = get("input.keyboard.layouts");
        assert_eq!(layouts["kind"].as_str(), Some("custom"));
        let default = layouts["default"].as_array().expect("satır içi dizi");
        assert_eq!(default[0]["layout"].as_str(), Some("us"));
        assert_eq!(get("input.keyboard.options")["default"].as_array().map(Vec::len), Some(0));
        let text = generate().unwrap();
        let at = text.find("key = \"input.keyboard.layouts\"").unwrap();
        assert!(text.contains("default = [{ layout = \"us\", variant = \"\" }]"), "satır içi tablo: {}", &text[at..at + 300]);
        assert_eq!(get("input.keyboard.repeat_rate")["unit"].as_str(), Some("/s"));
        assert_eq!(get("input.mouse.sensitivity")["min"].as_float(), Some(-1.0));
        assert_eq!(get("input.touchpad.scroll_method")["default"].as_str(), Some("2fg"));
    }
}
