//! Saf güç modeli: UPower `DisplayDevice` + yönetici özellikleri, sistem pillerinin şarj
//! sınırı (UPower ≥ 1.90 `ChargeThreshold*`), power-profiles-daemon durumu; düşük pil geçişi.

use std::collections::BTreeMap;
use std::time::Duration;

use zbus::zvariant::Value;

use nuxut_dbus::{PropMap, prop_bool, prop_f64, prop_i64, prop_str, prop_u32, value_str};

/// İkon adı sınırı.
const MAX_ICON: usize = 128;
/// Kalan süre sınırı (UPower bazen saçma büyük değer verir): 7 gün.
const MAX_TIME: i64 = 7 * 24 * 3600;
/// En fazla profil.
const MAX_PROFILES: usize = 8;
/// İzlenen en fazla sistem pili (gerçek makinelerde 1–2).
pub(crate) const MAX_BATTERIES: usize = 8;
/// UPower `Device.Type`: pil.
const TYPE_BATTERY: u32 = 2;
/// `ChargeStartThreshold` / `ChargeEndThreshold` "ayarlanmadı" değeri (UPower `G_MAXUINT`).
const THRESHOLD_UNSET: u32 = u32::MAX;
/// `ChargeThresholdSettingsSupported` bitleri: başlangıç eşiği, bitiş eşiği.
const SETTINGS_START: u32 = 1;
const SETTINGS_END: u32 = 2;

/// UPower `Device.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChargeState {
    #[default]
    Unknown,
    Charging,
    Discharging,
    Empty,
    FullyCharged,
    PendingCharge,
    PendingDischarge,
}

impl ChargeState {
    fn from_upower(n: u32) -> Self {
        match n {
            1 => Self::Charging,
            2 => Self::Discharging,
            3 => Self::Empty,
            4 => Self::FullyCharged,
            5 => Self::PendingCharge,
            6 => Self::PendingDischarge,
            _ => Self::Unknown,
        }
    }

    pub fn is_charging(self) -> bool {
        matches!(self, Self::Charging | Self::PendingCharge)
    }

    /// Kararlı ad (`msg power status`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Charging => "charging",
            Self::Discharging => "discharging",
            Self::Empty => "empty",
            Self::FullyCharged => "fully-charged",
            Self::PendingCharge => "pending-charge",
            Self::PendingDischarge => "pending-discharge",
        }
    }
}

/// UPower `Device.WarningLevel` (eşikler sistem genelinde `UPower.conf`'ta).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum WarningLevel {
    #[default]
    None,
    /// Pil yüzdesi `PercentageLow` altında.
    Low,
    /// `PercentageCritical` altında.
    Critical,
    /// `PercentageAction` altında: sistem birazdan eylem (askı/kapatma) yapacak.
    Action,
}

impl WarningLevel {
    fn from_upower(n: u32) -> Self {
        match n {
            3 => Self::Low,
            4 => Self::Critical,
            5 => Self::Action,
            // 0 bilinmiyor, 1 yok, 2 UPS deşarj (dizüstü için anlamsız)
            _ => Self::None,
        }
    }

    /// Kararlı ad (`msg power status`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Critical => "critical",
            Self::Action => "action",
        }
    }
}

/// Dizüstü pili (UPower birleşik `DisplayDevice`).
#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    /// 0..=100.
    pub percentage: f64,
    pub state: ChargeState,
    /// Boşalmaya kalan (yalnızca deşarjda, biliniyorsa).
    pub time_to_empty: Option<Duration>,
    /// Dolmaya kalan (yalnızca şarjda, biliniyorsa).
    pub time_to_full: Option<Duration>,
    /// UPower'ın önerdiği freedesktop ikon adı (`battery-level-40-symbolic` ...).
    pub icon_name: String,
    pub warning: WarningLevel,
}

/// power-profiles-daemon profili.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerProfile {
    PowerSaver,
    Balanced,
    Performance,
}

impl PowerProfile {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "power-saver" => Some(Self::PowerSaver),
            "balanced" => Some(Self::Balanced),
            "performance" => Some(Self::Performance),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PowerSaver => "power-saver",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
        }
    }
}

/// power-profiles-daemon durumu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Profiles {
    pub active: Option<PowerProfile>,
    pub available: Vec<PowerProfile>,
    /// Performans profili kısıtlı ise nedeni (`lap-detected`, `high-operating-temperature`).
    pub degraded: Option<String>,
}

/// Ham UPower `DisplayDevice` alanları (kısmi güncellemeler birleştirilir).
#[derive(Debug, Clone, Default, PartialEq)]
struct DeviceProps {
    kind: u32,
    present: bool,
    percentage: f64,
    state: u32,
    time_to_empty: i64,
    time_to_full: i64,
    icon_name: String,
    warning: u32,
}

/// Pil şarj sınırı: tüm sistem pillerinin (UPower `Device`, `Type` = pil, `PowerSupply`)
/// birleşimi. Eşik değerleri UPower'da udev hwdb'den gelir (`CHARGE_LIMIT`), D-Bus'tan salt
/// okunurdur; yalnızca açılıp kapatılabilir (`EnableChargeThreshold`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChargeLimit {
    /// UPower yok, pil yok ya da hiçbir pil desteklemiyor.
    #[default]
    Unsupported,
    Supported {
        /// Destekleyen pillerin hepsinde açık mı?
        enabled: bool,
        /// Bazılarında açık, bazılarında kapalı (başka bir araçla değiştirilmiş). Anahtar
        /// kapalı görünür; açmak hepsini açar.
        mixed: bool,
        /// Şarjın yeniden başladığı yüzde. `None`: ayarlanmadı / aygıt yazılımı seçer / piller
        /// farklı.
        start: Option<u8>,
        /// Şarjın durduğu yüzde (`None` aynı koşullarda).
        end: Option<u8>,
    },
}

impl ChargeLimit {
    pub fn is_supported(self) -> bool {
        matches!(self, Self::Supported { .. })
    }

    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Supported { enabled: true, .. })
    }
}

/// Bir sistem pilinin ham şarj eşiği alanları (kısmi güncellemeler birleştirilir).
#[derive(Debug, Clone, Default)]
struct ChargeDevice {
    supported: bool,
    enabled: bool,
    start: Option<u32>,
    end: Option<u32>,
    /// `ChargeThresholdSettingsSupported` (yalnız UPower ≥ 1.91; yoksa eşikler olduğu gibi).
    settings: Option<u32>,
    /// Son tam okumanın cevap seri numarası: daha eski `PropertiesChanged` atılır. Durum
    /// karşılaştırmasına girmez.
    serial: u32,
}

impl PartialEq for ChargeDevice {
    fn eq(&self, o: &Self) -> bool {
        (self.supported, self.enabled, self.start, self.end, self.settings)
            == (o.supported, o.enabled, o.start, o.end, o.settings)
    }
}

impl ChargeDevice {
    fn apply(&mut self, props: &PropMap) {
        if let Some(b) = prop_bool(props, "ChargeThresholdSupported") {
            self.supported = b;
        }
        if let Some(b) = prop_bool(props, "ChargeThresholdEnabled") {
            self.enabled = b;
        }
        if let Some(n) = prop_u32(props, "ChargeStartThreshold") {
            self.start = Some(n);
        }
        if let Some(n) = prop_u32(props, "ChargeEndThreshold") {
            self.end = Some(n);
        }
        if let Some(n) = prop_u32(props, "ChargeThresholdSettingsSupported") {
            self.settings = Some(n);
        }
    }

    /// Geçerli eşik: değer 0..=100, "ayarlanmadı" değil ve (biliniyorsa) desteklenen ayar.
    fn threshold(&self, raw: Option<u32>, bit: u32) -> Option<u8> {
        if self.settings.is_some_and(|s| s & bit == 0) {
            return None;
        }
        raw.filter(|&n| n != THRESHOLD_UNSET).and_then(|n| u8::try_from(n).ok()).filter(|&n| n <= 100)
    }
}

/// UPower cihazı sistem pili mi (`Type` = pil ve `PowerSupply`)? Fare/kulaklık pilleri
/// (`PowerSupply` = false) ve AC (`Type` = 1) değildir.
pub(crate) fn is_system_battery(props: &PropMap) -> bool {
    prop_u32(props, "Type") == Some(TYPE_BATTERY) && prop_bool(props, "PowerSupply") == Some(true)
}

/// Güç durumunun tamamı. [`PowerState::battery`] ve [`PowerState::charge_limit`] türetilir.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PowerState {
    /// UPower çalışıyor mu?
    pub upower: bool,
    /// Pilden mi çalışıyor (`false`: AC ya da pil yok).
    pub on_battery: bool,
    /// Kapağı olan bir dizüstü mü?
    pub lid_present: bool,
    /// `None`: power-profiles-daemon kullanılamıyor.
    pub profiles: Option<Profiles>,
    device: DeviceProps,
    /// Sistem pilleri (UPower nesne yolu → şarj eşiği alanları).
    batteries: BTreeMap<String, ChargeDevice>,
}

impl PowerState {
    /// Pil (yoksa `None`: masaüstü, VM).
    pub fn battery(&self) -> Option<Battery> {
        // UPower Device.Type: 2 = Battery.
        if !self.upower || !self.device.present || self.device.kind != 2 {
            return None;
        }
        let state = ChargeState::from_upower(self.device.state);
        let secs = |s: i64| (s > 0 && s <= MAX_TIME).then(|| Duration::from_secs(s as u64));
        Some(Battery {
            percentage: self.device.percentage.clamp(0.0, 100.0),
            state,
            time_to_empty: if state == ChargeState::Discharging { secs(self.device.time_to_empty) } else { None },
            time_to_full: if state.is_charging() { secs(self.device.time_to_full) } else { None },
            icon_name: self.device.icon_name.clone(),
            warning: WarningLevel::from_upower(self.device.warning),
        })
    }

    /// AC'ye bağlı mı (pil yoksa da `true`).
    pub fn ac_online(&self) -> bool {
        !self.on_battery
    }

    /// UPower yönetici özellikleri (`OnBattery`, `LidIsPresent`). Sahte veri için de.
    pub fn apply_manager(&mut self, props: &PropMap) {
        if let Some(b) = prop_bool(props, "OnBattery") {
            self.on_battery = b;
        }
        if let Some(b) = prop_bool(props, "LidIsPresent") {
            self.lid_present = b;
        }
    }

    /// Birleşik `DisplayDevice` özellikleri (sınırlı, doğrulanmış). Sahte veri için de.
    pub fn apply_device(&mut self, props: &PropMap) {
        let d = &mut self.device;
        if let Some(n) = prop_u32(props, "Type") {
            d.kind = n;
        }
        if let Some(b) = prop_bool(props, "IsPresent") {
            d.present = b;
        }
        if let Some(p) = prop_f64(props, "Percentage") {
            d.percentage = p;
        }
        if let Some(n) = prop_u32(props, "State") {
            d.state = n;
        }
        if let Some(n) = prop_i64(props, "TimeToEmpty") {
            d.time_to_empty = n;
        }
        if let Some(n) = prop_i64(props, "TimeToFull") {
            d.time_to_full = n;
        }
        if let Some(s) = prop_str(props, "IconName", MAX_ICON) {
            d.icon_name = s;
        }
        if let Some(n) = prop_u32(props, "WarningLevel") {
            d.warning = n;
        }
    }

    pub(crate) fn clear_upower(&mut self) {
        self.upower = false;
        self.on_battery = false;
        self.lid_present = false;
        self.device = DeviceProps::default();
        self.batteries.clear();
    }

    /// Şarj sınırı (birden fazla pilde birleşik; bkz. `docs/design/power.md`).
    pub fn charge_limit(&self) -> ChargeLimit {
        let supported: Vec<&ChargeDevice> = self.batteries.values().filter(|d| d.supported).collect();
        if !self.upower || supported.is_empty() {
            return ChargeLimit::Unsupported;
        }
        let on = supported.iter().filter(|d| d.enabled).count();
        // Piller farklı eşik bildiriyorsa tek değer gösterilmez.
        let common = |f: &dyn Fn(&ChargeDevice) -> Option<u8>| {
            let first = f(supported[0]);
            supported.iter().all(|d| f(d) == first).then_some(first).flatten()
        };
        ChargeLimit::Supported {
            enabled: on == supported.len(),
            mixed: on > 0 && on < supported.len(),
            start: common(&|d| d.threshold(d.start, SETTINGS_START)),
            end: common(&|d| d.threshold(d.end, SETTINGS_END)),
        }
    }

    /// `msg power status` (sözleşme power-v1): tek nesne; alanlar her zaman var, bilinmeyen `null`.
    pub fn status_json(&self) -> serde_json::Value {
        let battery = self.battery().map(|b| {
            serde_json::json!({
                "percentage": b.percentage,
                "state": b.state.as_str(),
                "time_to_empty": b.time_to_empty.map(|d| d.as_secs()),
                "time_to_full": b.time_to_full.map(|d| d.as_secs()),
                "warning": b.warning.as_str(),
            })
        });
        let profiles = self.profiles.as_ref().map(|p| {
            serde_json::json!({
                "active": p.active.map(PowerProfile::as_str),
                "available": p.available.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
                "degraded": p.degraded,
            })
        });
        serde_json::json!({
            "version": 1,
            "upower": self.upower,
            "on_battery": self.on_battery,
            "battery": battery,
            "profiles": profiles,
            "charge_limit": self.charge_limit_json(),
        })
    }

    /// `msg power charge-limit status`: `{"supported":false}` ya da
    /// `{"supported":true,"enabled":…,"mixed":…,"start":n|null,"end":n|null}`.
    pub fn charge_limit_json(&self) -> serde_json::Value {
        match self.charge_limit() {
            ChargeLimit::Unsupported => serde_json::json!({ "supported": false }),
            ChargeLimit::Supported { enabled, mixed, start, end } => serde_json::json!({
                "supported": true,
                "enabled": enabled,
                "mixed": mixed,
                "start": start,
                "end": end,
            }),
        }
    }

    /// İzlenen sistem pili mi?
    pub(crate) fn tracks_battery(&self, path: &str) -> bool {
        self.batteries.contains_key(path)
    }

    /// Tam okunmuş cihazı ekler (sistem pili değilse ya da sınır doluysa eklemez). `serial`:
    /// okuma cevabının seri numarası. Görünür durum değişmiş olabilirse `true`.
    pub fn track_battery(&mut self, path: &str, props: &PropMap, serial: u32) -> bool {
        if !is_system_battery(props) {
            return self.batteries.remove(path).is_some();
        }
        if !self.batteries.contains_key(path) && self.batteries.len() >= MAX_BATTERIES {
            return false;
        }
        let mut d = ChargeDevice { serial, ..ChargeDevice::default() };
        d.apply(props);
        self.batteries.insert(path.to_owned(), d);
        true
    }

    pub(crate) fn untrack_battery(&mut self, path: &str) -> bool {
        self.batteries.remove(path).is_some()
    }

    /// İzlenen pilin `PropertiesChanged`'i; son tam okumadan eski sinyal atılır.
    pub(crate) fn apply_battery(&mut self, path: &str, props: &PropMap, serial: u32) -> bool {
        match self.batteries.get_mut(path) {
            Some(d) if serial > d.serial => {
                d.apply(props);
                true
            }
            _ => false,
        }
    }

    /// Şarj sınırını `on` yapmak için çağrılacak piller (destekleyen ve henüz o durumda
    /// olmayan). `None`: destekleyen pil yok.
    pub(crate) fn charge_targets(&self, on: bool) -> Option<Vec<String>> {
        let supported: Vec<(&String, &ChargeDevice)> = self.batteries.iter().filter(|(_, d)| d.supported).collect();
        if !self.upower || supported.is_empty() {
            return None;
        }
        Some(supported.into_iter().filter(|(_, d)| d.enabled != on).map(|(p, _)| p.clone()).collect())
    }

    /// power-profiles-daemon özellikleri (`GetAll` ya da `PropertiesChanged`). Servis dışında sahte
    /// veri kurmak için de (Ayarlar'ın deneme kaynağı, testler).
    pub fn apply_profiles(&mut self, props: &PropMap) {
        let p = self.profiles.get_or_insert_with(Profiles::default);
        if props.contains_key("ActiveProfile") {
            p.active = prop_str(props, "ActiveProfile", 32).as_deref().and_then(PowerProfile::parse);
        }
        if let Some(v) = props.get("Profiles") {
            p.available = parse_profiles(v);
        }
        if props.contains_key("PerformanceDegraded") {
            p.degraded = prop_str(props, "PerformanceDegraded", 64).filter(|s| !s.is_empty());
        }
    }
}

/// `Profiles` (`aa{sv}`, her biri `Profile` anahtarlı) → bilinen profiller, sıralı, tekrarsız.
fn parse_profiles(v: &Value<'_>) -> Vec<PowerProfile> {
    let Value::Array(a) = v else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in a.inner().iter().take(MAX_PROFILES) {
        let Value::Dict(d) = entry else { continue };
        let name = d.iter().find_map(|(k, v)| match k {
            Value::Str(k) if k.as_str() == "Profile" => value_str(v, 32),
            _ => None,
        });
        if let Some(p) = name.as_deref().and_then(PowerProfile::parse)
            && !out.contains(&p)
        {
            out.push(p);
        }
    }
    out.sort_by_key(|p| match p {
        PowerProfile::PowerSaver => 0,
        PowerProfile::Balanced => 1,
        PowerProfile::Performance => 2,
    });
    out
}

/// Düşük pil olayı gerekir mi? Pilden çalışırken uyarı seviyesi `Low` ve üstüne **yükseldiyse**
/// yeni seviye. Aynı seviyede tekrar etmez; şarja takılınca sıfırlanır (sonraki düşüşte yine
/// uyarır).
pub fn warning_transition(previous: WarningLevel, state: &PowerState) -> Option<WarningLevel> {
    let level = state.battery().map_or(WarningLevel::None, |b| b.warning);
    (state.on_battery && level >= WarningLevel::Low && level > previous).then_some(level)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{Array, Dict, OwnedValue, Signature};

    fn props(entries: &[(&str, Value<'static>)]) -> PropMap {
        entries.iter().map(|(k, v)| (k.to_string(), OwnedValue::try_from(v.try_clone().unwrap()).unwrap())).collect()
    }

    fn laptop(percentage: f64, state: u32, warning: u32) -> PowerState {
        let mut s = PowerState { upower: true, ..Default::default() };
        s.apply_manager(&props(&[("OnBattery", Value::Bool(state == 2))]));
        s.apply_device(&props(&[
            ("Type", Value::U32(2)),
            ("IsPresent", Value::Bool(true)),
            ("Percentage", Value::F64(percentage)),
            ("State", Value::U32(state)),
            ("TimeToEmpty", Value::I64(5400)),
            ("TimeToFull", Value::I64(0)),
            ("IconName", Value::from("battery-level-40-symbolic")),
            ("WarningLevel", Value::U32(warning)),
        ]));
        s
    }

    #[test]
    fn battery_is_derived() {
        let s = laptop(41.5, 2, 1);
        let b = s.battery().unwrap();
        assert_eq!(b.percentage, 41.5);
        assert_eq!(b.state, ChargeState::Discharging);
        assert_eq!(b.time_to_empty, Some(Duration::from_secs(5400)));
        assert_eq!(b.time_to_full, None);
        assert!(!s.ac_online());
        // Kısmi güncelleme: yalnızca yüzde.
        let mut s = s;
        s.apply_device(&props(&[("Percentage", Value::F64(150.0))]));
        assert_eq!(s.battery().unwrap().percentage, 100.0);
    }

    #[test]
    fn no_battery_on_desktop() {
        let mut s = PowerState { upower: true, ..Default::default() };
        s.apply_device(&props(&[("Type", Value::U32(2)), ("IsPresent", Value::Bool(false))]));
        assert!(s.battery().is_none());
        assert!(s.ac_online());
    }

    #[test]
    fn absurd_times_are_dropped() {
        let mut s = laptop(50.0, 2, 1);
        s.apply_device(&props(&[("TimeToEmpty", Value::I64(i64::MAX))]));
        assert_eq!(s.battery().unwrap().time_to_empty, None);
    }

    #[test]
    fn warning_rises_once_per_level() {
        assert_eq!(warning_transition(WarningLevel::None, &laptop(19.0, 2, 3)), Some(WarningLevel::Low));
        assert_eq!(warning_transition(WarningLevel::Low, &laptop(18.0, 2, 3)), None);
        assert_eq!(warning_transition(WarningLevel::Low, &laptop(4.0, 2, 4)), Some(WarningLevel::Critical));
        // Şarjdayken uyarı yok.
        assert_eq!(warning_transition(WarningLevel::None, &laptop(4.0, 1, 4)), None);
    }

    #[test]
    fn profiles_parse() {
        let mut a = Array::new(&Signature::Dict { key: Signature::Str.into(), value: Signature::Variant.into() });
        for name in ["performance", "balanced", "bogus", "power-saver", "balanced"] {
            let mut d = Dict::new(&Signature::Str, &Signature::Variant);
            d.append(Value::from("Profile"), Value::Value(Box::new(Value::from(name)))).unwrap();
            d.append(Value::from("Driver"), Value::Value(Box::new(Value::from("platform_profile")))).unwrap();
            a.append(Value::Dict(d)).unwrap();
        }
        let mut s = PowerState::default();
        s.apply_profiles(&props(&[
            ("ActiveProfile", Value::from("balanced")),
            ("Profiles", Value::Array(a)),
            ("PerformanceDegraded", Value::from("")),
        ]));
        let p = s.profiles.unwrap();
        assert_eq!(p.active, Some(PowerProfile::Balanced));
        assert_eq!(p.available, [PowerProfile::PowerSaver, PowerProfile::Balanced, PowerProfile::Performance]);
        assert_eq!(p.degraded, None);
    }

    fn bat(supported: bool, enabled: bool, start: u32, end: u32) -> PropMap {
        props(&[
            ("Type", Value::U32(2)),
            ("PowerSupply", Value::Bool(true)),
            ("ChargeThresholdSupported", Value::Bool(supported)),
            ("ChargeThresholdEnabled", Value::Bool(enabled)),
            ("ChargeStartThreshold", Value::U32(start)),
            ("ChargeEndThreshold", Value::U32(end)),
        ])
    }

    #[test]
    fn charge_limit_single_battery() {
        let mut s = PowerState { upower: true, ..Default::default() };
        assert_eq!(s.charge_limit(), ChargeLimit::Unsupported, "pil yok");
        assert!(s.track_battery("/b0", &bat(true, false, 75, 80), 5));
        assert_eq!(s.charge_limit(), ChargeLimit::Supported { enabled: false, mixed: false, start: Some(75), end: Some(80) });
        assert_eq!(s.charge_targets(true), Some(vec!["/b0".to_owned()]));
        assert_eq!(s.charge_targets(false), Some(vec![]), "zaten kapalı");
        // Okumadan eski sinyal atılır, yenisi uygulanır.
        let on = props(&[("ChargeThresholdEnabled", Value::Bool(true))]);
        assert!(!s.apply_battery("/b0", &on, 5));
        assert!(s.apply_battery("/b0", &on, 6));
        assert!(s.charge_limit().is_enabled());
        // UPower yok → desteklenmiyor.
        s.clear_upower();
        assert_eq!(s.charge_limit(), ChargeLimit::Unsupported);
        assert_eq!(s.charge_targets(true), None);
    }

    #[test]
    fn charge_limit_thresholds_are_validated() {
        let mut s = PowerState { upower: true, ..Default::default() };
        // G_MAXUINT = ayarlanmadı (aygıt yazılımı seçer); 100'ü aşan saçma değer.
        s.track_battery("/b0", &bat(true, true, u32::MAX, 250), 1);
        assert_eq!(s.charge_limit(), ChargeLimit::Supported { enabled: true, mixed: false, start: None, end: None });
        // UPower 1.91: yalnız bitiş eşiği destekleniyorsa başlangıç gösterilmez.
        s.track_battery("/b0", &bat(true, true, 60, 80), 2);
        s.apply_battery("/b0", &props(&[("ChargeThresholdSettingsSupported", Value::U32(2))]), 3);
        assert_eq!(s.charge_limit(), ChargeLimit::Supported { enabled: true, mixed: false, start: None, end: Some(80) });
    }

    #[test]
    fn charge_limit_multiple_batteries() {
        let mut s = PowerState { upower: true, ..Default::default() };
        s.track_battery("/b0", &bat(true, true, 75, 80), 1);
        s.track_battery("/b1", &bat(true, false, 75, 80), 1);
        // Desteklemeyen pil yok sayılır.
        s.track_battery("/b2", &bat(false, false, 0, 0), 1);
        assert_eq!(s.charge_limit(), ChargeLimit::Supported { enabled: false, mixed: true, start: Some(75), end: Some(80) });
        assert_eq!(s.charge_targets(true), Some(vec!["/b1".to_owned()]));
        assert_eq!(s.charge_targets(false), Some(vec!["/b0".to_owned()]));
        // Farklı eşikler: tek değer gösterilmez.
        s.apply_battery("/b1", &props(&[("ChargeEndThreshold", Value::U32(90))]), 2);
        assert!(matches!(s.charge_limit(), ChargeLimit::Supported { start: Some(75), end: None, .. }));
        // Fare pili (PowerSupply=false) izlenmez; çıkan pil düşer.
        let mouse = props(&[("Type", Value::U32(2)), ("PowerSupply", Value::Bool(false))]);
        assert!(!s.track_battery("/mouse", &mouse, 1));
        assert!(!s.tracks_battery("/mouse"));
        assert!(s.untrack_battery("/b0"));
        assert_eq!(s.charge_limit(), ChargeLimit::Supported { enabled: false, mixed: false, start: Some(75), end: Some(90) });
    }

    #[test]
    fn status_json_shape() {
        let mut s = laptop(41.5, 2, 1);
        assert_eq!(s.charge_limit_json(), serde_json::json!({ "supported": false }));
        s.track_battery("/b0", &bat(true, true, u32::MAX, 80), 1);
        let v = s.status_json();
        assert_eq!(v["version"], 1);
        assert_eq!(v["battery"]["state"], "discharging");
        assert_eq!(v["battery"]["time_to_empty"], 5400);
        assert_eq!(v["profiles"], serde_json::Value::Null);
        assert_eq!(
            v["charge_limit"],
            serde_json::json!({ "supported": true, "enabled": true, "mixed": false, "start": null, "end": 80 })
        );
        // Masaüstü: pil yok.
        let v = PowerState::default().status_json();
        assert_eq!(v["battery"], serde_json::Value::Null);
        assert_eq!(v["upower"], false);
    }
}
