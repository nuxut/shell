//! Hyprland pencere geometrisi: `j/clients` (bölge seçiminde pencere vurgusu) ve
//! `j/activewindow` (`shot window`). Alanlar toleranslı okunur (`serde_json::Value`): sürüm
//! farkı ya da tür uyuşmazlığı o pencereyi atlar, listeyi düşürmez.

use serde_json::Value;

use super::geometry::LogicalRect;
use crate::services::hyprland::{HyprError, HyprlandClient};

/// Değerlendirilen en çok pencere (dev listeler sınırlı).
const MAX_CLIENTS: usize = 512;

/// Görünür pencerelerin dikdörtgenleri, tıklama/vurgu önceliğiyle: tam ekran, sonra yüzen,
/// sonra döşenmiş; grup içinde küçük alan önce (üstteki küçük pencere büyük olanın içinde
/// seçilebilsin). `visible`: görünen workspace kimlikleri (monitörlerin etkin + açık özel
/// workspace'leri).
pub fn parse_clients(json: &str, visible: &[i64]) -> Vec<LogicalRect> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    let mut out: Vec<(u8, i64, LogicalRect)> = items
        .iter()
        .take(MAX_CLIENTS)
        .filter_map(|c| {
            if c.get("mapped").and_then(Value::as_bool) == Some(false) || c.get("hidden").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            let ws = c.get("workspace")?.get("id")?.as_i64()?;
            if !visible.contains(&ws) {
                return None;
            }
            let rect = rect_of(c)?;
            let fullscreen = match c.get("fullscreen") {
                Some(Value::Number(n)) => n.as_u64().is_some_and(|v| v > 0),
                Some(Value::Bool(b)) => *b,
                _ => false,
            };
            let floating = c.get("floating").and_then(Value::as_bool).unwrap_or(false);
            let group = if fullscreen { 0 } else if floating { 1 } else { 2 };
            Some((group, i64::from(rect.w) * i64::from(rect.h), rect))
        })
        .collect();
    out.sort_by_key(|&(g, area, _)| (g, area));
    out.into_iter().map(|(.., r)| r).collect()
}

/// Odaklı pencerenin dikdörtgeni (`{}` ya da geçersizse `None`).
pub fn parse_active_window(json: &str) -> Option<LogicalRect> {
    let v: Value = serde_json::from_str(json).ok()?;
    rect_of(&v)
}

fn rect_of(c: &Value) -> Option<LogicalRect> {
    let pair = |key: &str| -> Option<(i32, i32)> {
        let a = c.get(key)?.as_array()?;
        let x = i32::try_from(a.first()?.as_i64()?).ok()?;
        let y = i32::try_from(a.get(1)?.as_i64()?).ok()?;
        Some((x, y))
    };
    let (x, y) = pair("at")?;
    let (w, h) = pair("size")?;
    let r = LogicalRect::new(x, y, w, h);
    (!r.is_empty()).then_some(r)
}

/// Görünür pencereler (bloklar; arka plan thread'inde).
pub fn visible_windows(client: &HyprlandClient, visible: &[i64]) -> Result<Vec<LogicalRect>, HyprError> {
    Ok(parse_clients(&client.request("j/clients")?, visible))
}

/// Odaklı pencere (bloklar).
pub fn active_window(client: &HyprlandClient) -> Result<Option<LogicalRect>, HyprError> {
    Ok(parse_active_window(&client.request("j/activewindow")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clients_filtered_and_ordered() {
        let json = r#"[
            {"at":[0,0],"size":[1000,800],"workspace":{"id":1},"floating":false,"fullscreen":0,"mapped":true,"hidden":false},
            {"at":[100,100],"size":[200,150],"workspace":{"id":1},"floating":true,"fullscreen":0},
            {"at":[10,10],"size":[50,50],"workspace":{"id":1},"floating":false},
            {"at":[0,0],"size":[1920,1080],"workspace":{"id":2},"fullscreen":2},
            {"at":[5,5],"size":[5,5],"workspace":{"id":9}},
            {"at":[5,5],"size":[5,5],"workspace":{"id":1},"hidden":true},
            {"at":[5,5],"size":[5,5],"workspace":{"id":1},"mapped":false},
            {"at":[5,5],"size":[0,5],"workspace":{"id":1}},
            {"at":"bozuk","size":[5,5],"workspace":{"id":1}},
            {"at":[3000,0],"size":[100,100],"workspace":{"id":-98},"fullscreen":true}
        ]"#;
        let r = parse_clients(json, &[1, 2, -98]);
        assert_eq!(
            r,
            vec![
                LogicalRect::new(3000, 0, 100, 100),
                LogicalRect::new(0, 0, 1920, 1080),
                LogicalRect::new(100, 100, 200, 150),
                LogicalRect::new(10, 10, 50, 50),
                LogicalRect::new(0, 0, 1000, 800),
            ]
        );
        assert!(parse_clients("{}", &[1]).is_empty());
        assert!(parse_clients("bozuk", &[1]).is_empty());
    }

    #[test]
    fn active_window_geometry() {
        assert_eq!(
            parse_active_window(r#"{"address":"0x1","at":[-1920,40],"size":[800,600]}"#),
            Some(LogicalRect::new(-1920, 40, 800, 600))
        );
        assert_eq!(parse_active_window("{}"), None);
        assert_eq!(parse_active_window(r#"{"at":[0,0],"size":[0,0]}"#), None);
        assert_eq!(parse_active_window(r#"{"at":[0,0],"size":[99999999999,1]}"#), None);
    }
}
