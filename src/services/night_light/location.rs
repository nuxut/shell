//! Saat diliminden yaklaşık konum: `latitude`/`longitude` yazılmamışsa güneş zamanlaması
//! saat diliminin temsil ettiği şehrin koordinatını kullanır (tzdata `zone1970.tab`; yoksa
//! `zone.tab`). Ağ ve konum servisi yok (geoclue v1 dışı).
//!
//! Saat dilimi adı: `TZ` ortam değişkeni (`Europe/Istanbul`, `:Europe/Istanbul`), yoksa
//! `/etc/localtime` bağlantısının `zoneinfo/` sonrası. Okuma küçük ve sınırlı (tab dosyaları
//! ~20 KB); yalnızca açılışta ve config değişince.

use std::io::Read;
use std::path::{Path, PathBuf};

/// tzdata dizini.
const ZONEINFO: &str = "/usr/share/zoneinfo";
/// Okunacak tab dosyası en çok bu kadar bayt.
const MAX_TAB_BYTES: u64 = 256 * 1024;
/// Saat dilimi adı sınırı (en uzun gerçek ad ~32).
const MAX_ZONE_NAME: usize = 64;

/// `±DDMM` / `±DDMMSS` (enlem, 2 haneli derece) ya da `±DDDMM` / `±DDDMMSS` (boylam).
fn parse_angle(s: &str, degree_digits: usize) -> Option<f64> {
    let (sign, digits) = match s.as_bytes().first()? {
        b'+' => (1.0, &s[1..]),
        b'-' => (-1.0, &s[1..]),
        _ => return None,
    };
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let num = |a: usize, b: usize| digits.get(a..b).and_then(|x| x.parse::<f64>().ok());
    let d = num(0, degree_digits)?;
    let m = num(degree_digits, degree_digits + 2)?;
    let sec = match digits.len() - degree_digits {
        2 => 0.0,
        4 => num(degree_digits + 2, degree_digits + 4)?,
        _ => return None,
    };
    if m >= 60.0 || sec >= 60.0 {
        return None;
    }
    Some(sign * (d + m / 60.0 + sec / 3600.0))
}

/// ISO 6709 koordinatı (`+4101+02858`, `-332000+1510000`) → (enlem, boylam).
pub fn parse_coordinates(s: &str) -> Option<(f64, f64)> {
    let split = s.char_indices().skip(1).find(|(_, c)| *c == '+' || *c == '-')?.0;
    let (lat, lon) = s.split_at(split);
    let lat = parse_angle(lat, 2)?;
    let lon = parse_angle(lon, 3)?;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

/// Tab dosyası metninde saat diliminin koordinatı. Sütunlar: ülke kodu(ları), koordinat,
/// saat dilimi, [yorum]; `#` satırları yorum.
pub fn lookup(tab: &str, zone: &str) -> Option<(f64, f64)> {
    tab.lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .find(|cols| cols.get(2) == Some(&zone))
        .and_then(|cols| parse_coordinates(cols.get(1)?))
}

/// Saat dilimi adı geçerli görünüyor mu (`Area/City`, yol kaçışı yok).
fn valid_zone(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_ZONE_NAME
        && !name.split('/').any(|p| p.is_empty() || p == "." || p == "..")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_-+".contains(&b))
}

/// `TZ` değeri ya da `/etc/localtime` hedefi → saat dilimi adı.
pub fn zone_name(tz_env: Option<&str>, localtime_target: Option<&Path>) -> Option<String> {
    if let Some(tz) = tz_env.map(|t| t.strip_prefix(':').unwrap_or(t)).filter(|t| !t.is_empty()) {
        // Mutlak yol (`TZ=/usr/share/zoneinfo/Europe/Istanbul`) da olabilir.
        let name = tz.rsplit_once("zoneinfo/").map_or(tz, |(_, n)| n);
        return valid_zone(name).then(|| name.to_owned());
    }
    let target = localtime_target?.to_str()?;
    let name = target.rsplit_once("zoneinfo/")?.1;
    valid_zone(name).then(|| name.to_owned())
}

fn read_tab(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    f.take(MAX_TAB_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

/// Sistemin saat diliminden yaklaşık konum (bulunamazsa `None`; çağıran elle saatlere düşer).
pub fn from_system() -> Option<(String, (f64, f64))> {
    let link: Option<PathBuf> = std::fs::read_link("/etc/localtime").ok();
    let zone = zone_name(std::env::var("TZ").ok().as_deref(), link.as_deref())?;
    ["zone1970.tab", "zone.tab"]
        .into_iter()
        .find_map(|f| lookup(&read_tab(&Path::new(ZONEINFO).join(f))?, &zone))
        .map(|c| (zone, c))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAB: &str = "# tzdb timezone descriptions\n\
        #country-\n\
        TR\t+4101+02858\tEurope/Istanbul\n\
        AU\t-3352+15113\tAustralia/Sydney\tNew South Wales (most areas)\n\
        US\t+404251-0740023\tAmerica/New_York\tEastern (most areas)\n\
        NO,SJ\t+5955+01045\tEurope/Oslo\n";

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn parses_iso6709_minutes_and_seconds() {
        assert!(close(parse_coordinates("+4101+02858").unwrap(), (41.0 + 1.0 / 60.0, 28.0 + 58.0 / 60.0)));
        assert!(close(parse_coordinates("-3352+15113").unwrap(), (-(33.0 + 52.0 / 60.0), 151.0 + 13.0 / 60.0)));
        let ny = parse_coordinates("+404251-0740023").unwrap();
        assert!(close(ny, (40.0 + 42.0 / 60.0 + 51.0 / 3600.0, -(74.0 + 0.0 / 60.0 + 23.0 / 3600.0))));
        for bad in ["", "4101+02858", "+4101", "+4161+02858", "+41a1+02858", "+410+02858", "+9901+00000"] {
            assert_eq!(parse_coordinates(bad), None, "{bad}");
        }
    }

    #[test]
    fn looks_up_zone_rows() {
        assert!(close(lookup(TAB, "Europe/Istanbul").unwrap(), (41.0167, 28.9667)));
        assert!(lookup(TAB, "Europe/Oslo").is_some(), "çok ülkeli satır");
        assert_eq!(lookup(TAB, "Europe/Nowhere"), None);
        assert_eq!(lookup(TAB, "country-"), None);
    }

    #[test]
    fn zone_name_from_env_or_link() {
        let link = Path::new("/usr/share/zoneinfo/Europe/Istanbul");
        assert_eq!(zone_name(None, Some(link)).as_deref(), Some("Europe/Istanbul"));
        assert_eq!(zone_name(Some(":America/New_York"), Some(link)).as_deref(), Some("America/New_York"));
        assert_eq!(zone_name(Some("/usr/share/zoneinfo/Asia/Tokyo"), None).as_deref(), Some("Asia/Tokyo"));
        assert_eq!(zone_name(Some(""), Some(link)).as_deref(), Some("Europe/Istanbul"), "boş TZ yok sayılır");
        assert_eq!(zone_name(Some("../../etc/passwd"), None), None);
        assert_eq!(zone_name(None, Some(Path::new("/etc/foo"))), None);
        assert_eq!(zone_name(None, None), None);
    }
}
