//! Yerleşim doğrulaması ve normalleştirme (ADR-0005 §3.3). Saf.
//!
//! Kurallar: her bağlı monitör tam bir kez; kip o monitörde var; ölçek 0.5..=4 ve temiz bölen;
//! en az bir etkin ve yansıtmayan monitör (hepsini kapatma engeli); yansıtma hedefi etkin ve
//! kendisi yansıtmayan başka bir monitör (zincir yok); etkin monitörlerin mantıksal
//! dikdörtgenleri çakışmaz ve kenar/köşe temasıyla bağlıdır; birincil etkin ve yansıtmayan.
//! Geçerli yerleşim normalleştirilir: en küçük x ve y 0'a kaydırılır, birincil yoksa sol-üst.

use super::model::{DisplayLayout, Hardware, MAX_COORD, MAX_SCALE_120, MIN_SCALE_120, nearest_clean_scale, scale_str};

/// Hata kodları (IPC'de `error: <kod>: <mesaj>`; ADR-0005 §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueCode {
    UnknownDisplay,
    MissingDisplay,
    InvalidMode,
    InvalidScale,
    InvalidPosition,
    Overlap,
    DisconnectedLayout,
    NoActiveDisplay,
    MirrorChain,
    InvalidPrimary,
}

impl IssueCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownDisplay => "unknown_display",
            Self::MissingDisplay => "missing_display",
            Self::InvalidMode => "invalid_mode",
            Self::InvalidScale => "invalid_scale",
            Self::InvalidPosition => "invalid_position",
            Self::Overlap => "overlap",
            Self::DisconnectedLayout => "disconnected_layout",
            Self::NoActiveDisplay => "no_active_display",
            Self::MirrorChain => "mirror_chain",
            Self::InvalidPrimary => "invalid_primary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub code: IssueCode,
    /// İlgili monitörün anahtarı.
    pub display: Option<String>,
    pub message: String,
    /// `invalid_scale`: en yakın geçerli ölçek (×120).
    pub suggest_scale_120: Option<u32>,
}

impl Issue {
    fn new(code: IssueCode, display: Option<&str>, message: impl Into<String>) -> Self {
        Self { code, display: display.map(str::to_owned), message: message.into(), suggest_scale_120: None }
    }
}

/// Mantıksal dikdörtgen `(x, y, w, h)`.
type R = (i64, i64, i64, i64);

fn overlaps(a: R, b: R) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Kenar ya da köşe teması (alan kesişimi olmadan).
fn touches(a: R, b: R) -> bool {
    a.0 <= b.0 + b.2 && b.0 <= a.0 + a.2 && a.1 <= b.1 + b.3 && b.1 <= a.1 + a.3
}

/// Doğrular; geçerliyse normalleştirilmiş yerleşimi döndürür.
pub fn validate(layout: &DisplayLayout, hw: &Hardware) -> Result<DisplayLayout, Vec<Issue>> {
    let mut issues = Vec::new();
    for (i, d) in layout.displays.iter().enumerate() {
        if hw.by_key(&d.key).is_none() {
            issues.push(Issue::new(IssueCode::UnknownDisplay, Some(&d.key), "bağlı değil"));
        }
        if layout.displays[..i].iter().any(|o| o.key == d.key) {
            issues.push(Issue::new(IssueCode::UnknownDisplay, Some(&d.key), "iki kez verilmiş"));
        }
    }
    for m in &hw.monitors {
        if layout.get(&m.key()).is_none() {
            issues.push(Issue::new(IssueCode::MissingDisplay, Some(&m.key()), "istekte yok (tam durum gönderilmeli)"));
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }

    for d in &layout.displays {
        let Some(m) = hw.by_key(&d.key) else { continue };
        if !d.enabled {
            continue;
        }
        if !m.supports(&d.mode) {
            issues.push(Issue::new(IssueCode::InvalidMode, Some(&d.key), format!("{} bu monitörde yok", d.mode.rule())));
        }
        if !(MIN_SCALE_120..=MAX_SCALE_120).contains(&d.scale_120) {
            issues.push(Issue::new(IssueCode::InvalidScale, Some(&d.key), "ölçek 0.5 ile 4 arasında olmalı"));
        } else if d.logical().is_none() {
            let mut issue = Issue::new(IssueCode::InvalidScale, Some(&d.key), "");
            issue.suggest_scale_120 = nearest_clean_scale(d.mode.w, d.mode.h, d.scale_120);
            issue.message = match issue.suggest_scale_120 {
                Some(s) => format!("{} bu çözünürlüğü tam bölmüyor; en yakın: {}", scale_str(d.scale_120), scale_str(s)), // i18n: tanı Issue ayrıntısı; sözleşme IssueCode, Ayarlar çevirir
                None => format!("{} bu çözünürlüğü tam bölmüyor", scale_str(d.scale_120)), // i18n: tanı Issue ayrıntısı; sözleşme IssueCode, Ayarlar çevirir
            };
            issues.push(issue);
        }
        if d.pos.0.unsigned_abs() > MAX_COORD.unsigned_abs() || d.pos.1.unsigned_abs() > MAX_COORD.unsigned_abs() {
            issues.push(Issue::new(IssueCode::InvalidPosition, Some(&d.key), "konum sınır dışı"));
        }
        if let Some(target) = &d.mirror_of {
            match layout.get(target) {
                Some(t) if t.key != d.key && t.is_placed() => {}
                _ => issues.push(Issue::new(
                    IssueCode::MirrorChain,
                    Some(&d.key),
                    "yansıtma hedefi etkin ve kendisi yansıtmayan başka bir monitör olmalı",
                )),
            }
        }
    }
    if !layout.displays.iter().any(|d| d.is_placed()) {
        issues.push(Issue::new(IssueCode::NoActiveDisplay, None, "en az bir monitör açık kalmalı"));
    }
    if !issues.is_empty() {
        return Err(issues);
    }

    // Yerleşim: yalnızca etkin, yansıtmayan monitörler.
    let placed: Vec<(&str, R)> = layout
        .displays
        .iter()
        .filter(|d| d.is_placed())
        .filter_map(|d| {
            let (w, h) = d.logical()?;
            Some((d.key.as_str(), (i64::from(d.pos.0), i64::from(d.pos.1), i64::from(w), i64::from(h))))
        })
        .collect();
    for (i, (ka, a)) in placed.iter().enumerate() {
        for (kb, b) in &placed[i + 1..] {
            if overlaps(*a, *b) {
                issues.push(Issue::new(IssueCode::Overlap, Some(ka), format!("{kb} ile çakışıyor")));
            }
        }
    }
    // Bağlılık: ilk dikdörtgenden temasla ulaşılamayan varsa kopuk.
    let mut reached = vec![false; placed.len()];
    let mut stack = vec![0];
    reached[0] = true;
    while let Some(i) = stack.pop() {
        for j in 0..placed.len() {
            if !reached[j] && touches(placed[i].1, placed[j].1) {
                reached[j] = true;
                stack.push(j);
            }
        }
    }
    if let Some(j) = reached.iter().position(|r| !r) {
        issues.push(Issue::new(IssueCode::DisconnectedLayout, Some(placed[j].0), "diğer monitörlere değmiyor (fare geçemez)"));
    }
    if let Some(p) = &layout.primary
        && !layout.get(p).is_some_and(|d| d.is_placed())
    {
        issues.push(Issue::new(IssueCode::InvalidPrimary, Some(p), "birincil monitör açık ve yansıtmayan olmalı"));
    }
    if !issues.is_empty() {
        return Err(issues);
    }

    // Normalleştirme.
    let min_x = placed.iter().map(|(_, r)| r.0).min().unwrap_or(0);
    let min_y = placed.iter().map(|(_, r)| r.1).min().unwrap_or(0);
    let mut out = layout.clone();
    for d in &mut out.displays {
        if d.is_placed() {
            d.pos = ((i64::from(d.pos.0) - min_x) as i32, (i64::from(d.pos.1) - min_y) as i32);
        }
    }
    // Yansıtan monitörün konumu hedefinkine eşitlenir (Hyprland da öyle yapar).
    let targets: Vec<(String, (i32, i32))> = out.displays.iter().filter(|d| d.is_placed()).map(|d| (d.key.clone(), d.pos)).collect();
    for d in &mut out.displays {
        if let Some(t) = &d.mirror_of
            && let Some((_, pos)) = targets.iter().find(|(k, _)| k == t)
        {
            d.pos = *pos;
        }
    }
    if out.primary.is_none() {
        out.primary = out.displays.iter().filter(|d| d.is_placed()).min_by_key(|d| (d.pos.1, d.pos.0)).map(|d| d.key.clone());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::model::fixtures::DUAL;
    use super::super::model::{DisplayConfig, Mode, Transform, Vrr};
    use super::*;

    const EDP: &str = "BOE|0x0BCA||eDP-1";
    const DELL: &str = "Dell Inc.|DELL U2720Q|ABC123";

    fn hw() -> Hardware {
        Hardware::parse(DUAL).unwrap()
    }

    fn base() -> DisplayLayout {
        hw().current_layout()
    }

    fn set(l: &mut DisplayLayout, key: &str, f: impl FnOnce(&mut DisplayConfig)) {
        f(l.displays.iter_mut().find(|d| d.key == key).unwrap());
    }

    fn codes(r: Result<DisplayLayout, Vec<Issue>>) -> Vec<&'static str> {
        r.err().unwrap_or_default().iter().map(|i| i.code.as_str()).collect()
    }

    #[test]
    fn current_state_is_valid_and_primary_is_top_left() {
        let out = validate(&base(), &hw()).unwrap();
        assert_eq!(out.primary.as_deref(), Some(EDP));
    }

    #[test]
    fn rejects_turning_everything_off() {
        let mut l = base();
        for d in &mut l.displays {
            d.enabled = false;
        }
        assert_eq!(codes(validate(&l, &hw())), ["no_active_display"]);
        // Biri kapalı, diğeri onu yansıtıyor → yine hiç etkin monitör yok.
        let mut l = base();
        set(&mut l, EDP, |d| d.enabled = false);
        set(&mut l, DELL, |d| d.mirror_of = Some(EDP.into()));
        assert!(codes(validate(&l, &hw())).contains(&"no_active_display"));
    }

    #[test]
    fn mirror_rules() {
        let mut l = base();
        set(&mut l, DELL, |d| d.mirror_of = Some(EDP.into()));
        let out = validate(&l, &hw()).unwrap();
        assert_eq!(out.get(DELL).unwrap().pos, (0, 0), "yansıtan hedefin konumunu alır");
        // Zincir/karşılıklı yansıtma.
        set(&mut l, EDP, |d| d.mirror_of = Some(DELL.into()));
        assert!(codes(validate(&l, &hw())).contains(&"mirror_chain"));
        // Kendini yansıtma.
        let mut l = base();
        set(&mut l, DELL, |d| d.mirror_of = Some(DELL.into()));
        assert!(codes(validate(&l, &hw())).contains(&"mirror_chain"));
    }

    #[test]
    fn scale_must_divide_cleanly_and_suggests() {
        let mut l = base();
        set(&mut l, DELL, |d| d.scale_120 = 210); // 1.75: 2560/1.75 tam değil
        let issues = validate(&l, &hw()).unwrap_err();
        assert_eq!(issues[0].code, IssueCode::InvalidScale);
        let s = issues[0].suggest_scale_120.unwrap();
        assert!(super::super::model::is_clean_scale(2560, 1440, s));
        assert!(issues[0].message.contains("en yakın"));
        set(&mut l, DELL, |d| d.scale_120 = 30);
        assert_eq!(codes(validate(&l, &hw())), ["invalid_scale"]);
    }

    #[test]
    fn extreme_positions_are_rejected_without_panic() {
        for pos in [(i32::MIN, 0), (0, i32::MIN), (i32::MAX, 0), (MAX_COORD + 1, 0)] {
            let mut l = base();
            set(&mut l, DELL, |d| d.pos = pos);
            assert_eq!(codes(validate(&l, &hw())), ["invalid_position"], "{pos:?}");
        }
    }

    #[test]
    fn mode_must_exist() {
        let mut l = base();
        set(&mut l, DELL, |d| d.mode = Mode { w: 1234, h: 567, mhz: 60_000 });
        assert_eq!(codes(validate(&l, &hw())), ["invalid_mode"]);
        // Kapalı monitörün kipi denetlenmez.
        set(&mut l, DELL, |d| d.enabled = false);
        validate(&l, &hw()).unwrap();
    }

    #[test]
    fn overlap_and_disconnection() {
        let mut l = base();
        set(&mut l, DELL, |d| d.pos = (1000, 0));
        assert_eq!(codes(validate(&l, &hw())), ["overlap"]);
        set(&mut l, DELL, |d| d.pos = (1930, 0));
        assert_eq!(codes(validate(&l, &hw())), ["disconnected_layout"]);
        // Köşe teması bağlı sayılır.
        set(&mut l, DELL, |d| d.pos = (1920, 1200));
        validate(&l, &hw()).unwrap();
        // Altında, ortalanmış.
        set(&mut l, DELL, |d| d.pos = (-320, 1200));
        let out = validate(&l, &hw()).unwrap();
        assert_eq!(out.get(DELL).unwrap().pos, (0, 1200), "normalleştirildi");
        assert_eq!(out.get(EDP).unwrap().pos, (320, 0));
    }

    #[test]
    fn rotation_changes_logical_size() {
        let mut l = base();
        // Dikey Dell (1440x2560 mantıksal) sağda: hâlâ temas var.
        set(&mut l, DELL, |d| d.transform = Transform::new(1).unwrap());
        validate(&l, &hw()).unwrap();
        // Dikey eDP (1200x1920) → Dell 1920'de kopuk kalır.
        set(&mut l, EDP, |d| d.transform = Transform::new(3).unwrap());
        assert_eq!(codes(validate(&l, &hw())), ["disconnected_layout"]);
    }

    #[test]
    fn completeness_and_primary() {
        let mut l = base();
        l.displays.pop();
        assert_eq!(codes(validate(&l, &hw())), ["missing_display"]);
        let mut l = base();
        l.displays.push(DisplayConfig {
            key: "x|y|z".into(),
            enabled: true,
            mode: Mode { w: 800, h: 600, mhz: 60_000 },
            pos: (0, 0),
            scale_120: 120,
            transform: Transform::NORMAL,
            vrr: Vrr::Off,
            mirror_of: None,
        });
        assert_eq!(codes(validate(&l, &hw())), ["unknown_display"]);
        let mut l = base();
        l.primary = Some(DELL.into());
        assert_eq!(validate(&l, &hw()).unwrap().primary.as_deref(), Some(DELL));
        set(&mut l, DELL, |d| d.enabled = false);
        assert_eq!(codes(validate(&l, &hw())), ["invalid_primary"]);
    }
}
