//! `nuxut-shell msg displays …` JSON şeması (ADR-0005 §7). Saf: cevap üretimi ve `apply`
//! isteğinin katı ayrıştırması.

use serde::{Deserialize, Serialize};

use super::model::{DisplayConfig, DisplayLayout, Hardware, MAX_DISPLAYS, MAX_FIELD, Mode, Transform, Vrr, scale_str, suggested_scales};
use crate::services::hypr_config::sync::ConfigKind;

/// Şema sürümü (`v`).
pub const VERSION: u32 = 1;
/// `apply` JSON üst sınırı (framework IPC satırı 4096 bayt; K3).
pub const MAX_APPLY_BYTES: usize = 4000;
/// `list` cevabında monitör başına çözünürlük ve çözünürlük başına hız sınırları.
pub const MAX_RESOLUTIONS: usize = 64;
pub const MAX_RATES: usize = 16;

/// IPC hata kodları (`error: <kod>: <mesaj>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub fn line(&self) -> String {
        if self.message.is_empty() { format!("error: {}", self.code) } else { format!("error: {}: {}", self.code, self.message) }
    }
}

#[derive(Debug, Serialize)]
pub struct PendingJson {
    pub token: String,
    pub remaining_ms: u64,
    /// Onay penceresi gösteriliyor (değilse yalnızca doğrulama bekleniyor).
    pub confirm: bool,
}

/// Son işlemin sonucu (`status.last`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LastJson {
    pub token: String,
    /// `confirmed|applied|reverted|timeout|not_applied|hotplug|not_shown|reloaded|error`
    pub result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusJson<'a> {
    pub v: u32,
    pub r#gen: u64,
    /// `lua|hyprlang|null` (Hyprland'e erişim yoksa null).
    pub provider: Option<&'static str>,
    /// Uygulama mümkün mü (değilse `apply` → `unavailable`).
    pub available: bool,
    pub pending: Option<PendingJson>,
    pub last: Option<&'a LastJson>,
}

pub fn provider_name(kind: Option<ConfigKind>) -> Option<&'static str> {
    kind.map(|k| match k {
        ConfigKind::Lua => "lua",
        ConfigKind::Hyprlang => "hyprlang",
    })
}

#[derive(Debug, Serialize)]
struct ModeJson {
    w: u32,
    h: u32,
    mhz: u32,
}

impl From<Mode> for ModeJson {
    fn from(m: Mode) -> Self {
        Self { w: m.w, h: m.h, mhz: m.mhz }
    }
}

#[derive(Debug, Serialize)]
struct ResolutionJson {
    w: u32,
    h: u32,
    rates_mhz: Vec<u32>,
    preferred: bool,
    /// Önerilen temiz ölçekler (ondalık) ve aynısı ×120.
    scales: Vec<f64>,
    scales_120: Vec<u32>,
}

#[derive(Debug, Serialize)]
struct DisplayJson {
    id: String,
    id_stable: bool,
    connector: String,
    make: String,
    model: String,
    serial: String,
    description: String,
    physical_mm: [u32; 2],
    enabled: bool,
    mode: ModeJson,
    pos: [i32; 2],
    scale: f64,
    scale_120: u32,
    transform: u8,
    vrr: &'static str,
    vrr_active: bool,
    /// Hyprland vermiyor: her zaman null (bilinmiyor).
    vrr_capable: Option<bool>,
    mirror_of: Option<String>,
    logical: Option<[i32; 2]>,
    primary: bool,
    resolutions: Vec<ResolutionJson>,
}

#[derive(Debug, Serialize)]
struct ListJson {
    v: u32,
    r#gen: u64,
    provider: Option<&'static str>,
    available: bool,
    pending: Option<PendingJson>,
    primary: Option<String>,
    displays: Vec<DisplayJson>,
}

/// `list` cevabı. `desired`: monitörün yapılandırılmış (kayıtlı) değerleri — `vrr` ve birincil
/// Hyprland'den güvenilir okunamaz (ADR-0005 §3.2).
pub fn list_json(
    hw: &Hardware,
    desired: Option<&DisplayLayout>,
    r#gen: u64,
    provider: Option<ConfigKind>,
    available: bool,
    pending: Option<PendingJson>,
) -> String {
    let primary = desired.and_then(|l| l.primary.clone());
    let displays = hw
        .monitors
        .iter()
        .map(|m| {
            let key = m.key();
            let want = desired.and_then(|l| l.get(&key));
            let mut sizes: Vec<(u32, u32)> = Vec::new();
            for md in m.modes.iter().chain(std::iter::once(&m.mode)) {
                if !sizes.contains(&(md.w, md.h)) && sizes.len() < MAX_RESOLUTIONS {
                    sizes.push((md.w, md.h));
                }
            }
            sizes.sort_by_key(|&(w, h)| std::cmp::Reverse((u64::from(w) * u64::from(h), w)));
            let preferred = m.preferred_mode();
            let resolutions = sizes
                .into_iter()
                .map(|(w, h)| {
                    let mut rates: Vec<u32> = m.modes.iter().chain(std::iter::once(&m.mode)).filter(|x| x.w == w && x.h == h).map(|x| x.mhz).collect();
                    rates.sort_unstable_by(|a, b| b.cmp(a));
                    rates.dedup();
                    rates.truncate(MAX_RATES);
                    let s120 = suggested_scales(w, h);
                    ResolutionJson {
                        w,
                        h,
                        rates_mhz: rates,
                        preferred: preferred.w == w && preferred.h == h,
                        scales: s120.iter().map(|s| scale_str(*s).parse().unwrap_or(1.0)).collect(),
                        scales_120: s120,
                    }
                })
                .collect();
            DisplayJson {
                id: key.clone(),
                id_stable: m.id_stable,
                connector: m.connector.clone(),
                make: m.id.make.clone(),
                model: m.id.model.clone(),
                serial: m.id.serial.clone(),
                description: m.description.clone(),
                physical_mm: [m.physical_mm.0, m.physical_mm.1],
                enabled: m.enabled,
                mode: m.mode.into(),
                pos: [m.pos.0, m.pos.1],
                scale: scale_str(m.scale_120).parse().unwrap_or(1.0),
                scale_120: m.scale_120,
                transform: m.transform.get(),
                vrr: want.map_or(if m.vrr_active { Vrr::On } else { Vrr::Off }, |d| d.vrr).name(),
                vrr_active: m.vrr_active,
                vrr_capable: None,
                mirror_of: m.mirror_of.as_deref().and_then(|n| hw.by_connector(n)).map(|t| t.key()),
                logical: (m.enabled).then(|| super::model::logical_size(m.mode, m.scale_120, m.transform)).flatten().map(|(w, h)| [w, h]),
                primary: primary.as_deref() == Some(key.as_str()),
                resolutions,
            }
        })
        .collect();
    let out = ListJson { v: VERSION, r#gen, provider: provider_name(provider), available, pending, primary, displays };
    serde_json::to_string(&out).unwrap_or_else(|_| "{\"v\":1,\"displays\":[]}".into())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyModeJson {
    w: u32,
    h: u32,
    mhz: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyDisplayJson {
    id: String,
    enabled: bool,
    mode: ApplyModeJson,
    pos: [i32; 2],
    scale_120: u32,
    transform: u8,
    vrr: String,
    #[serde(default)]
    mirror_of: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyJson {
    v: u32,
    base_gen: u64,
    #[serde(default)]
    primary: Option<String>,
    displays: Vec<ApplyDisplayJson>,
}

/// Ayrıştırılmış `apply` isteği.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyRequest {
    pub base_gen: u64,
    pub layout: DisplayLayout,
}

fn field_ok(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= MAX_FIELD * 4 + 3 && !s.chars().any(char::is_control)
}

/// `apply` JSON'u: katı tipli (`deny_unknown_fields`), sınırlı; doğrulama (kip/ölçek/yerleşim)
/// sonra donanıma karşı yapılır.
pub fn parse_apply(json: &str) -> Result<ApplyRequest, ApiError> {
    if json.len() > MAX_APPLY_BYTES {
        return Err(ApiError::new("too_large", format!("istek en çok {MAX_APPLY_BYTES} bayt")));
    }
    let a: ApplyJson = serde_json::from_str(json).map_err(|e| ApiError::new("bad_json", crate::services::hyprland::json_error_summary(&e)))?;
    if a.v != VERSION {
        return Err(ApiError::new("bad_json", format!("v = {} desteklenmiyor (beklenen {VERSION})", a.v)));
    }
    if a.displays.is_empty() || a.displays.len() > MAX_DISPLAYS {
        return Err(ApiError::new("bad_json", "displays 1..=16 öğe olmalı"));
    }
    let mut displays = Vec::new();
    for d in a.displays {
        if !field_ok(&d.id) || d.mirror_of.as_deref().is_some_and(|m| !field_ok(m)) {
            return Err(ApiError::new("bad_json", "geçersiz id"));
        }
        let mode = Mode { w: d.mode.w, h: d.mode.h, mhz: d.mode.mhz };
        if !mode.is_sane() {
            return Err(ApiError::new("invalid_mode", format!("{}: kip sınır dışı", d.id)));
        }
        let transform = Transform::new(d.transform).ok_or_else(|| ApiError::new("bad_json", "transform 0..7"))?;
        let vrr = Vrr::from_name(&d.vrr).ok_or_else(|| ApiError::new("bad_json", "vrr off|on|fullscreen|fullscreen-media"))?;
        displays.push(DisplayConfig {
            key: d.id,
            enabled: d.enabled,
            mode,
            pos: (d.pos[0], d.pos[1]),
            scale_120: d.scale_120,
            transform,
            vrr,
            mirror_of: d.mirror_of.filter(|m| !m.is_empty()),
        });
    }
    if a.primary.as_deref().is_some_and(|p| !field_ok(p)) {
        return Err(ApiError::new("bad_json", "geçersiz primary"));
    }
    Ok(ApplyRequest { base_gen: a.base_gen, layout: DisplayLayout { displays, primary: a.primary.filter(|p| !p.is_empty()) } })
}

/// Ayarlar'ın göndereceği biçimde `apply` JSON'u (testler ve örnek için).
pub fn apply_json(base_gen: u64, layout: &DisplayLayout) -> String {
    let displays: Vec<serde_json::Value> = layout
        .displays
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.key, "enabled": d.enabled,
                "mode": {"w": d.mode.w, "h": d.mode.h, "mhz": d.mode.mhz},
                "pos": [d.pos.0, d.pos.1], "scale_120": d.scale_120, "transform": d.transform.get(),
                "vrr": d.vrr.name(), "mirror_of": d.mirror_of,
            })
        })
        .collect();
    serde_json::json!({"v": VERSION, "base_gen": base_gen, "primary": layout.primary, "displays": displays}).to_string()
}

#[cfg(test)]
mod tests {
    use super::super::model::fixtures::{DUAL, VM};
    use super::*;

    #[test]
    fn list_shape() {
        let hw = Hardware::parse(DUAL).unwrap();
        let mut desired = hw.current_layout();
        desired.primary = Some("Dell Inc.|DELL U2720Q|ABC123".into());
        let text = list_json(&hw, Some(&desired), 7, Some(ConfigKind::Lua), true, None);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["gen"], 7);
        assert_eq!(v["provider"], "lua");
        let dell = &v["displays"][1];
        assert_eq!(dell["id"], "Dell Inc.|DELL U2720Q|ABC123");
        assert_eq!(dell["id_stable"], true);
        assert_eq!(dell["primary"], true);
        assert_eq!(dell["logical"], serde_json::json!([2560, 1440]));
        assert_eq!(dell["vrr"], "on");
        assert!(dell["vrr_capable"].is_null());
        let r0 = &dell["resolutions"][0];
        assert_eq!((r0["w"].as_u64(), r0["h"].as_u64()), (Some(3840), Some(2160)));
        assert_eq!(r0["rates_mhz"], serde_json::json!([60000, 30000]));
        assert_eq!(r0["preferred"], true);
        assert!(r0["scales_120"].as_array().unwrap().contains(&serde_json::json!(180)));
        assert!(text.len() < 64 * 1024);
        // VM: şu anki kip (30 Hz) listede olmasa da çözünürlüklere girer.
        let vm = Hardware::parse(VM).unwrap();
        let v: serde_json::Value = serde_json::from_str(&list_json(&vm, None, 1, None, false, None)).unwrap();
        assert!(v["provider"].is_null());
        let fhd = v["displays"][0]["resolutions"].as_array().unwrap().iter().find(|r| r["w"] == 1920).unwrap().clone();
        assert_eq!(fhd["rates_mhz"], serde_json::json!([60000, 50000, 30000]));
    }

    #[test]
    fn apply_roundtrip_and_size() {
        let hw = Hardware::parse(DUAL).unwrap();
        let l = hw.current_layout();
        let json = apply_json(3, &l);
        let req = parse_apply(&json).unwrap();
        assert_eq!(req.base_gen, 3);
        assert_eq!(req.layout, l);
        // 16 monitör sığar (K3: ~230 bayt/monitör).
        let mut big = l.clone();
        big.displays = (0..16).map(|i| { let mut d = l.displays[1].clone(); d.key = format!("Manufacturer {i}|Model Name XYZ|SERIAL{i:06}"); d }).collect();
        assert!(apply_json(1, &big).len() <= MAX_APPLY_BYTES, "{}", apply_json(1, &big).len());
    }

    #[test]
    fn apply_rejects() {
        let hw = Hardware::parse(DUAL).unwrap();
        let json = apply_json(1, &hw.current_layout());
        let code = |s: &str| parse_apply(s).unwrap_err().code;
        assert_eq!(code("{"), "bad_json");
        assert_eq!(code(&json.replace("\"v\":1", "\"v\":2")), "bad_json");
        assert_eq!(code(&json.replacen("\"enabled\"", "\"bogus\":1,\"enabled\"", 1)), "bad_json");
        assert_eq!(code(&json.replacen("\"vrr\":\"off\"", "\"vrr\":\"max\"", 1)), "bad_json");
        assert_eq!(code(&json.replacen("\"transform\":0", "\"transform\":9", 1)), "bad_json");
        assert_eq!(code(&json.replacen("\"w\":3840", "\"w\":0", 1)), "invalid_mode");
        assert_eq!(code(&" ".repeat(MAX_APPLY_BYTES + 1)), "too_large");
        assert_eq!(code(r#"{"v":1,"base_gen":0,"displays":[]}"#), "bad_json");
        assert_eq!(ApiError::new("stale", "x").line(), "error: stale: x");
    }
}
