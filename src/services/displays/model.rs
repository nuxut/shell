//! Monitör modeli (ADR-0005 §3): kimlik, kip, ölçek (×120), döndürme, VRR, yerleşim ve
//! Hyprland'in `j/monitors all` cevabının ayrıştırılması. Saf; IO yok.

use serde::Deserialize;

/// Bağlı monitör üst sınırı (ADR-0005 §3.2).
pub const MAX_DISPLAYS: usize = 16;
/// Kimlik alanı ve bağlayıcı adı üst sınırı (karakter).
pub const MAX_FIELD: usize = 64;
/// Monitör başına okunan kip üst sınırı (Hyprland listesi daha uzunsa kırpılır).
pub const MAX_MODES: usize = 256;
/// Ölçek sınırları (×120): 0.5 ..= 4.
pub const MIN_SCALE_120: u32 = 60;
pub const MAX_SCALE_120: u32 = 480;
/// Konum sınırı (mantıksal piksel, mutlak değer).
pub const MAX_COORD: i32 = 1 << 16;
/// Kip boyut sınırı (piksel).
pub const MAX_PIXELS: u32 = 16_384;
/// Yenileme hızı sınırları (mHz).
pub const MIN_MHZ: u32 = 1_000;
pub const MAX_MHZ: u32 = 1_000_000;

/// Kontrol karakteri ve ayraçlardan (`|`) arındırılmış, kırpılmış, sınırlı alan.
fn clean_field(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '|' { '/' } else { c })
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_FIELD)
        .collect()
}

/// Monitör kimliği: üretici + model + seri numarası (ADR-0005 §3.1).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DisplayId {
    pub make: String,
    pub model: String,
    pub serial: String,
}

impl DisplayId {
    pub fn new(make: &str, model: &str, serial: &str) -> Self {
        Self { make: clean_field(make), model: clean_field(model), serial: clean_field(serial) }
    }

    /// `make|model|serial`.
    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.make, self.model, self.serial)
    }
}

/// Ekran kipi: piksel boyu ve yenileme hızı (mHz).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mode {
    pub w: u32,
    pub h: u32,
    pub mhz: u32,
}

impl Mode {
    pub fn is_sane(&self) -> bool {
        (1..=MAX_PIXELS).contains(&self.w) && (1..=MAX_PIXELS).contains(&self.h) && (MIN_MHZ..=MAX_MHZ).contains(&self.mhz)
    }

    /// Hyprland'in `availableModes` dizgisi: `1920x1080@60.00Hz`.
    pub fn parse_hypr(s: &str) -> Option<Self> {
        let s = s.trim().strip_suffix("Hz").unwrap_or(s.trim());
        let (size, rate) = s.split_once('@')?;
        let (w, h) = size.split_once('x')?;
        let m = Self { w: w.parse().ok()?, h: h.parse().ok()?, mhz: hz_to_mhz(rate.parse().ok()?)? };
        m.is_sane().then_some(m)
    }

    /// `displays.toml` biçimi: `3840x2160@59.997`.
    pub fn parse_toml(s: &str) -> Option<Self> {
        Self::parse_hypr(s)
    }

    /// Hyprland kural biçimi (`3840x2160@59.997`; Hyprland en yakın hızı seçer).
    pub fn rule(&self) -> String {
        format!("{}x{}@{}.{:03}", self.w, self.h, self.mhz / 1000, self.mhz % 1000)
    }

    /// Aynı boy, yenileme ±50 mHz (Hyprland listesindeki iki ondalık yuvarlamayı kapsar).
    pub fn matches(&self, other: &Mode) -> bool {
        self.w == other.w && self.h == other.h && self.mhz.abs_diff(other.mhz) <= 50
    }
}

pub fn hz_to_mhz(hz: f64) -> Option<u32> {
    if !hz.is_finite() || hz <= 0.0 {
        return None;
    }
    let m = (hz * 1000.0).round();
    (m >= f64::from(MIN_MHZ) && m <= f64::from(MAX_MHZ)).then_some(m as u32)
}

/// `wl_output` döndürmesi (Hyprland `transform` 0..7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transform(u8);

impl Transform {
    pub const NORMAL: Self = Self(0);

    pub fn new(v: u8) -> Option<Self> {
        (v <= 7).then_some(Self(v))
    }

    pub fn get(self) -> u8 {
        self.0
    }

    /// 90/270 (çevrilmiş dahil): en-boy yer değiştirir.
    pub fn swaps_axes(self) -> bool {
        self.0 % 2 == 1
    }
}

/// Değişken yenileme hızı kipi (Hyprland `vrr` 0..3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Vrr {
    #[default]
    Off,
    On,
    Fullscreen,
    FullscreenMedia,
}

impl Vrr {
    pub const ALL: [Vrr; 4] = [Vrr::Off, Vrr::On, Vrr::Fullscreen, Vrr::FullscreenMedia];

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Fullscreen => "fullscreen",
            Self::FullscreenMedia => "fullscreen-media",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == s)
    }

    pub fn hypr(self) -> u8 {
        self as u8
    }
}

/// Ölçek ×120 → kullanıcıya/dosyaya ondalık (en çok 6 basamak, sondaki sıfırlar atılır).
pub fn scale_str(scale_120: u32) -> String {
    let v = f64::from(scale_120) / 120.0;
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_owned()
}

/// Ondalık ölçek → ×120 (en yakın adım).
pub fn scale_from_f64(v: f64) -> Option<u32> {
    if !v.is_finite() || v <= 0.0 {
        return None;
    }
    let s = (v * 120.0).round();
    (s >= 1.0 && s <= f64::from(MAX_SCALE_120) * 4.0).then_some(s as u32)
}

/// Hyprland'in "temiz bölen" kuralının tam sayı karşılığı: `w/s` ve `h/s` tam sayı
/// (s = scale_120/120 → `w·120 % scale_120 == 0`).
pub fn is_clean_scale(w: u32, h: u32, scale_120: u32) -> bool {
    scale_120 > 0 && (u64::from(w) * 120) % u64::from(scale_120) == 0 && (u64::from(h) * 120) % u64::from(scale_120) == 0
}

/// Sınırlar içindeki en yakın temiz ölçek (eşitlikte küçüğü).
pub fn nearest_clean_scale(w: u32, h: u32, scale_120: u32) -> Option<u32> {
    (0..=MAX_SCALE_120)
        .flat_map(|d| [scale_120.checked_sub(d), scale_120.checked_add(d)])
        .flatten()
        .find(|&s| (MIN_SCALE_120..=MAX_SCALE_120).contains(&s) && is_clean_scale(w, h, s))
}

/// Kullanıcıya önerilen ölçekler (×120): anlamlı alt küme içinden temiz olanlar (ADR-0005 §7.2).
pub const SUGGESTED_SCALES_120: [u32; 11] = [120, 150, 160, 180, 192, 210, 240, 270, 300, 360, 480];

pub fn suggested_scales(w: u32, h: u32) -> Vec<u32> {
    SUGGESTED_SCALES_120.into_iter().filter(|&s| is_clean_scale(w, h, s)).collect()
}

/// Mantıksal boy (döndürme dahil); ölçek temiz değilse `None`.
pub fn logical_size(mode: Mode, scale_120: u32, transform: Transform) -> Option<(i32, i32)> {
    if !is_clean_scale(mode.w, mode.h, scale_120) {
        return None;
    }
    let w = i32::try_from(u64::from(mode.w) * 120 / u64::from(scale_120)).ok()?;
    let h = i32::try_from(u64::from(mode.h) * 120 / u64::from(scale_120)).ok()?;
    Some(if transform.swaps_axes() { (h, w) } else { (w, h) })
}

/// Hyprland'in gördüğü bir monitör (bağlı; kapalı olabilir).
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// Hyprland iç kimliği (`mirrorOf` bunu verir).
    pub hypr_id: i64,
    /// Bağlayıcı adı (`DP-1`).
    pub connector: String,
    pub id: DisplayId,
    /// Hyprland'in kısa açıklaması (`desc:` eşlemesi bununla yapılır).
    pub description: String,
    /// Açıklama Hyprland'inkiyle birebir mi (kırpılmadı/arındırılmadı)? Değilse `desc:` eşlemesi
    /// tutmaz; kural bağlayıcı adıyla yazılır.
    pub description_exact: bool,
    pub physical_mm: (u32, u32),
    pub enabled: bool,
    pub mode: Mode,
    pub pos: (i32, i32),
    pub scale_120: u32,
    pub transform: Transform,
    /// Yansıttığı monitörün bağlayıcı adı.
    pub mirror_of: Option<String>,
    /// Etkin adaptive sync (yapılandırılan kip değil).
    pub vrr_active: bool,
    pub modes: Vec<Mode>,
    /// Kimlik kalıcı mı (seri numarası var ve bağlı monitörler arasında tek)? Değilse eşleme
    /// bağlayıcı adıyla tamamlanır (ADR-0005 §3.1).
    pub id_stable: bool,
}

impl Monitor {
    /// Eşleme anahtarı: kalıcı kimlikte `make|model|serial`, değilse `make|model|serial|bağlayıcı`.
    pub fn key(&self) -> String {
        if self.id_stable { self.id.key() } else { format!("{}|{}", self.id.key(), self.connector) }
    }

    /// Kip bu monitörde seçilebilir mi (liste ya da şu anki kip; VM'de şu anki kip listede yok).
    pub fn supports(&self, mode: &Mode) -> bool {
        self.mode.matches(mode) || self.modes.iter().any(|m| m.matches(mode))
    }

    /// Tercih edilen kip: en büyük alan, sonra en yüksek hız (Hyprland `preferred` bilgisi JSON'da yok).
    pub fn preferred_mode(&self) -> Mode {
        self.modes.iter().copied().max_by_key(|m| (u64::from(m.w) * u64::from(m.h), m.mhz)).unwrap_or(self.mode)
    }
}

/// Bağlı monitörler (Hyprland'den okunan anlık durum).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hardware {
    pub monitors: Vec<Monitor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MonitorJson {
    id: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    make: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    serial: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    physical_width: u32,
    #[serde(default)]
    physical_height: u32,
    #[serde(default)]
    refresh_rate: f64,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default)]
    scale: f64,
    #[serde(default)]
    transform: u8,
    #[serde(default)]
    vrr: serde_json::Value,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    mirror_of: serde_json::Value,
    #[serde(default)]
    available_modes: Vec<String>,
}

/// Bağlayıcı adı: `[A-Za-z0-9._-]`, ≤ 64.
pub fn valid_connector(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_FIELD && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

impl Hardware {
    /// `hyprctl -j monitors all` cevabı. Geçersiz monitör (ad/kip) atlanır; ≤ 16.
    pub fn parse(json: &str) -> Result<Self, String> {
        let list: Vec<MonitorJson> = serde_json::from_str(json).map_err(|e| format!("j/monitors all ayrıştırılamadı: {e}"))?;
        let mut monitors = Vec::new();
        for m in &list {
            if !valid_connector(&m.name) {
                continue;
            }
            let mode = Mode { w: m.width, h: m.height, mhz: hz_to_mhz(m.refresh_rate).unwrap_or(60_000) };
            let modes: Vec<Mode> = m.available_modes.iter().take(MAX_MODES).filter_map(|s| Mode::parse_hypr(s)).collect();
            // Kapalı monitörde boy 0 gelebilir: tercih edilen kipe düş.
            let mode = if mode.is_sane() { mode } else { modes.iter().copied().max_by_key(|m| u64::from(m.w) * u64::from(m.h)).unwrap_or(Mode { w: 1920, h: 1080, mhz: 60_000 }) };
            let mirror_id = match &m.mirror_of {
                serde_json::Value::String(s) => s.parse::<i64>().ok(),
                serde_json::Value::Number(n) => n.as_i64(),
                _ => None,
            };
            let vrr_active = match &m.vrr {
                serde_json::Value::Bool(b) => *b,
                serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
                _ => false,
            };
            monitors.push((
                mirror_id,
                Monitor {
                    hypr_id: m.id,
                    connector: m.name.clone(),
                    id: DisplayId::new(&m.make, &m.model, &m.serial),
                    description: clean_field(&m.description),
                    description_exact: clean_field(&m.description) == m.description,
                    physical_mm: (m.physical_width, m.physical_height),
                    enabled: !m.disabled,
                    mode,
                    pos: (m.x.clamp(-MAX_COORD, MAX_COORD), m.y.clamp(-MAX_COORD, MAX_COORD)),
                    scale_120: scale_from_f64(m.scale).unwrap_or(120).clamp(MIN_SCALE_120, MAX_SCALE_120),
                    transform: Transform::new(m.transform).unwrap_or_default(),
                    mirror_of: None,
                    vrr_active,
                    modes,
                    id_stable: false,
                },
            ));
            if monitors.len() == MAX_DISPLAYS {
                break;
            }
        }
        let names: Vec<(i64, String)> = monitors.iter().map(|(_, m)| (m.hypr_id, m.connector.clone())).collect();
        let mut out: Vec<Monitor> = monitors
            .into_iter()
            .map(|(mirror, mut m)| {
                m.mirror_of = mirror.and_then(|id| names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()));
                m
            })
            .collect();
        let ids: Vec<DisplayId> = out.iter().map(|m| m.id.clone()).collect();
        for m in &mut out {
            m.id_stable = !m.id.serial.is_empty() && ids.iter().filter(|i| **i == m.id).count() == 1;
        }
        Ok(Self { monitors: out })
    }

    pub fn by_key(&self, key: &str) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.key() == key)
    }

    pub fn by_connector(&self, name: &str) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.connector == name)
    }

    /// Bağlı küme anahtarı: sıralı eşleme anahtarları (ADR-0005 §8).
    pub fn set_key(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.monitors.iter().map(Monitor::key).collect();
        keys.sort();
        keys
    }

    /// Şu anki çalışma anı durumu bir yerleşim olarak (K8: ilk açılışta içe aktarma).
    pub fn current_layout(&self) -> DisplayLayout {
        DisplayLayout {
            displays: self
                .monitors
                .iter()
                .map(|m| DisplayConfig {
                    key: m.key(),
                    enabled: m.enabled,
                    mode: m.mode,
                    pos: m.pos,
                    scale_120: m.scale_120,
                    transform: m.transform,
                    vrr: if m.vrr_active { Vrr::On } else { Vrr::Off },
                    mirror_of: m.mirror_of.as_deref().and_then(|n| self.by_connector(n)).map(Monitor::key),
                })
                .collect(),
            primary: None,
        }
    }
}

/// Bir monitörün istenen ayarı. `key`: [`Monitor::key`].
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayConfig {
    pub key: String,
    pub enabled: bool,
    pub mode: Mode,
    /// Mantıksal piksel, sol üst.
    pub pos: (i32, i32),
    pub scale_120: u32,
    pub transform: Transform,
    pub vrr: Vrr,
    /// Yansıtılan monitörün anahtarı.
    pub mirror_of: Option<String>,
}

impl DisplayConfig {
    /// Yerleşime katılır mı (etkin ve yansıtma değil)?
    pub fn is_placed(&self) -> bool {
        self.enabled && self.mirror_of.is_none()
    }

    pub fn logical(&self) -> Option<(i32, i32)> {
        logical_size(self.mode, self.scale_120, self.transform)
    }
}

/// Tüm bağlı monitörlerin istenen durumu.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayLayout {
    pub displays: Vec<DisplayConfig>,
    /// Birincil monitörün anahtarı (`cursor:default_monitor` + shell varsayılanı; K6).
    pub primary: Option<String>,
}

impl DisplayLayout {
    pub fn get(&self, key: &str) -> Option<&DisplayConfig> {
        self.displays.iter().find(|d| d.key == key)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    /// Bu VM'in `hyprctl -j monitors all` çıktısı (kısaltılmış; serial boş, şu anki kip listede yok).
    pub const VM: &str = r#"[{"id":0,"name":"Virtual-1","description":"Red Hat Inc. QEMU Monitor","make":"Red Hat, Inc.","model":"QEMU Monitor","serial":"","width":1920,"height":1080,"physicalWidth":320,"physicalHeight":200,"refreshRate":30.00000,"x":0,"y":0,"scale":1,"transform":0,"focused":true,"vrr":false,"disabled":false,"mirrorOf":"none","availableModes":["1280x800@74.99Hz","3840x2160@60.00Hz","2560x1440@59.95Hz","1920x1080@60.00Hz","1920x1080@50.00Hz"]}]"#;

    /// Sentetik iki monitör: 4K dizüstü (eDP-1, 1.5) + 1440p harici (DP-1) sağda.
    pub const DUAL: &str = r#"[
      {"id":0,"name":"eDP-1","description":"BOE 0x0BCA","make":"BOE","model":"0x0BCA","serial":"","width":3840,"height":2400,"physicalWidth":300,"physicalHeight":190,"refreshRate":60.0,"x":0,"y":0,"scale":2,"transform":0,"vrr":false,"disabled":false,"mirrorOf":"none","availableModes":["3840x2400@60.00Hz","1920x1200@60.00Hz"]},
      {"id":1,"name":"DP-1","description":"Dell Inc. DELL U2720Q ABC123","make":"Dell Inc.","model":"DELL U2720Q","serial":"ABC123","width":2560,"height":1440,"physicalWidth":600,"physicalHeight":340,"refreshRate":59.951,"x":1920,"y":0,"scale":1,"transform":0,"vrr":true,"disabled":false,"mirrorOf":"none","availableModes":["3840x2160@60.00Hz","3840x2160@30.00Hz","2560x1440@59.95Hz","1920x1080@60.00Hz"]}
    ]"#;
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn modes_parse_and_print() {
        let m = Mode::parse_hypr("1920x1080@59.94Hz").unwrap();
        assert_eq!(m, Mode { w: 1920, h: 1080, mhz: 59_940 });
        assert_eq!(m.rule(), "1920x1080@59.940");
        assert_eq!(Mode::parse_toml("3840x2160@59.997").unwrap().mhz, 59_997);
        for bad in ["", "x@60", "0x1080@60Hz", "1920x1080", "1920x1080@0Hz", "99999x1@60Hz", "1920x1080@nanHz"] {
            assert!(Mode::parse_hypr(bad).is_none(), "{bad}");
        }
        assert!(Mode { w: 1920, h: 1080, mhz: 60_000 }.matches(&Mode { w: 1920, h: 1080, mhz: 59_960 }));
        assert!(!Mode { w: 1920, h: 1080, mhz: 60_000 }.matches(&Mode { w: 1920, h: 1080, mhz: 59_940 }));
    }

    #[test]
    fn clean_scales() {
        // 1920x1080: 1.25 (150) → 1536x864 temiz; 1.6 (192) → 1200x675 temiz; 1.333 (160) → 1440x810.
        for s in [120, 150, 160, 192, 240] {
            assert!(is_clean_scale(1920, 1080, s), "{s}");
        }
        // 1.5 → 1280x720 temiz; 1.37 (≈164) değil.
        assert!(is_clean_scale(1920, 1080, 180));
        assert!(!is_clean_scale(1920, 1080, 164));
        assert_eq!(nearest_clean_scale(1920, 1080, 164), Some(160));
        // 2560x1440: 1.75 (210) → 1462.857 temiz değil.
        assert!(!is_clean_scale(2560, 1440, 210));
        assert!(suggested_scales(3840, 2160).contains(&180));
        assert!(!suggested_scales(2560, 1440).contains(&210));
        assert_eq!(logical_size(Mode { w: 3840, h: 2160, mhz: 60_000 }, 180, Transform::NORMAL), Some((2560, 1440)));
        assert_eq!(logical_size(Mode { w: 1920, h: 1080, mhz: 60_000 }, 120, Transform::new(1).unwrap()), Some((1080, 1920)));
        assert_eq!(logical_size(Mode { w: 1920, h: 1080, mhz: 60_000 }, 164, Transform::NORMAL), None);
        assert_eq!(scale_str(180), "1.5");
        assert_eq!(scale_str(160), "1.333333");
        assert_eq!(scale_str(120), "1");
        assert_eq!(scale_from_f64(1.333333), Some(160));
    }

    #[test]
    fn parses_vm_monitor() {
        let hw = Hardware::parse(VM).unwrap();
        let m = &hw.monitors[0];
        assert_eq!(m.connector, "Virtual-1");
        assert_eq!(m.id, DisplayId::new("Red Hat, Inc.", "QEMU Monitor", ""));
        assert!(!m.id_stable, "seri numarası boş");
        assert_eq!(m.key(), "Red Hat, Inc.|QEMU Monitor||Virtual-1");
        assert_eq!(m.mode, Mode { w: 1920, h: 1080, mhz: 30_000 });
        assert!(m.supports(&m.mode), "şu anki kip listede olmasa da seçilebilir");
        assert_eq!(m.preferred_mode(), Mode { w: 3840, h: 2160, mhz: 60_000 });
        assert_eq!(hw.set_key(), vec![m.key()]);
    }

    #[test]
    fn parses_dual_and_identity() {
        let hw = Hardware::parse(DUAL).unwrap();
        assert_eq!(hw.monitors.len(), 2);
        let dell = hw.by_connector("DP-1").unwrap();
        assert!(dell.id_stable);
        assert_eq!(dell.key(), "Dell Inc.|DELL U2720Q|ABC123");
        assert!(dell.vrr_active);
        assert_eq!(dell.mode.mhz, 59_951);
        assert_eq!(hw.by_connector("eDP-1").unwrap().scale_120, 240);
        // Aynı seri iki kez: kalıcı değil.
        let twin = DUAL.replace("\"BOE\"", "\"Dell Inc.\"").replace("\"0x0BCA\",\"serial\":\"\"", "\"DELL U2720Q\",\"serial\":\"ABC123\"");
        let hw2 = Hardware::parse(&twin).unwrap();
        assert!(hw2.monitors.iter().all(|m| !m.id_stable));
        assert_ne!(hw2.monitors[0].key(), hw2.monitors[1].key());
    }

    #[test]
    fn hostile_input_is_bounded() {
        assert!(Hardware::parse("not json").is_err());
        let evil = r#"[{"id":0,"name":"DP-1;reload","make":"a|b\n","model":"m","serial":"s","width":1,"height":1,"refreshRate":60}]"#;
        assert!(Hardware::parse(evil).unwrap().monitors.is_empty(), "bağlayıcı adı güvensiz");
        let ok = r#"[{"id":0,"name":"DP-1","make":"a|b\n","model":"m","serial":"s","width":0,"height":0,"refreshRate":0,"disabled":true,"mirrorOf":"7"}]"#;
        let hw = Hardware::parse(ok).unwrap();
        assert_eq!(hw.monitors[0].id.make, "a/b");
        assert!(hw.monitors[0].mode.is_sane());
        assert_eq!(hw.monitors[0].mirror_of, None, "bilinmeyen id");
        let many: Vec<String> = (0..40).map(|i| format!(r#"{{"id":{i},"name":"DP-{i}","width":800,"height":600,"refreshRate":60}}"#)).collect();
        assert_eq!(Hardware::parse(&format!("[{}]", many.join(","))).unwrap().monitors.len(), MAX_DISPLAYS);
    }

    #[test]
    fn mirror_resolves_to_connector() {
        let json = DUAL.replacen("\"mirrorOf\":\"none\"", "\"mirrorOf\":\"1\"", 1);
        let hw = Hardware::parse(&json).unwrap();
        assert_eq!(hw.by_connector("eDP-1").unwrap().mirror_of.as_deref(), Some("DP-1"));
        let layout = hw.current_layout();
        assert_eq!(layout.displays[0].mirror_of.as_deref(), Some("Dell Inc.|DELL U2720Q|ABC123"));
    }
}
