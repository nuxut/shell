//! Saf MPRIS modeli: oynatıcı durumu, metadata çözümleme, konum tahmini, etkin oynatıcı.

use std::time::{Duration, Instant};

use zbus::zvariant::Value;

use nuxut_dbus::{PropMap, clean_str, prop_bool, prop_f64, prop_i64, prop_str, value_i64, value_str};

/// Metin alanı sınırı (başlık, albüm, sanatçı başına).
pub const MAX_TEXT: usize = 512;
/// En fazla sanatçı.
const MAX_ARTISTS: usize = 8;
/// Kapak URL'si sınırı.
pub const MAX_URL: usize = 2048;
/// Uzunluk sınırı: 7 gün (µs) — saçma değerleri at.
const MAX_LENGTH_US: i64 = 7 * 24 * 3600 * 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    #[default]
    Stopped,
}

impl PlaybackStatus {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "Playing" => Some(Self::Playing),
            "Paused" => Some(Self::Paused),
            "Stopped" => Some(Self::Stopped),
            _ => None,
        }
    }
}

/// Kapak görselinin kaynağı (doğrulanmış).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtSource {
    /// Yerel dosya (mutlak yol; `file://` URL'sinden çözülmüş).
    File(std::path::PathBuf),
    /// Uzak görsel. Şimdilik yüklenmez (SSRF; UI yer tutucu gösterir).
    Remote(String),
}

impl ArtSource {
    /// [`super::MediaService::fetch_art`]'a verilecek (ve [`super::MediaEvent::Art`]'ta dönen)
    /// adres. UI yerel dosyayı kendisi açmaz.
    pub fn url(&self) -> String {
        match self {
            Self::File(p) => url::Url::from_file_path(p).map(String::from).unwrap_or_default(),
            Self::Remote(u) => u.clone(),
        }
    }
}

/// `mpris:artUrl` doğrulaması: yalnızca `file://` (mutlak yol, yüzde kodlu çözülür) ve
/// `http(s)://`; diğer her şey (data:, ftp:, göreli) reddedilir.
pub fn parse_art_url(s: &str) -> Option<ArtSource> {
    if s.len() > MAX_URL || s.chars().any(char::is_control) {
        return None;
    }
    let url = url::Url::parse(s).ok()?;
    match url.scheme() {
        "file" => {
            if url.host_str().is_some_and(|h| !h.is_empty() && h != "localhost") {
                return None;
            }
            let path = url.to_file_path().ok()?;
            path.is_absolute().then_some(ArtSource::File(path))
        }
        "http" | "https" if url.host_str().is_some_and(|h| !h.is_empty()) && url.username().is_empty() => {
            Some(ArtSource::Remote(url.to_string()))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Metadata {
    pub track_id: Option<String>,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub length: Option<Duration>,
    pub art: Option<ArtSource>,
}

impl Metadata {
    /// `Metadata` (`a{sv}`) çözümü; tanınmayan/yanlış tipli alanlar atlanır.
    pub(crate) fn parse(v: &Value<'_>) -> Self {
        let mut m = Self::default();
        let v = match v {
            Value::Value(b) => b.as_ref(),
            v => v,
        };
        let Value::Dict(d) = v else { return m };
        for (k, v) in d.iter() {
            let Value::Str(k) = k else { continue };
            match k.as_str() {
                "mpris:trackid" => m.track_id = value_str(v, 256),
                "xesam:title" => m.title = value_str(v, MAX_TEXT).unwrap_or_default(),
                "xesam:album" => m.album = value_str(v, MAX_TEXT).unwrap_or_default(),
                "xesam:artist" => {
                    m.artists = match v {
                        Value::Value(b) => artists(b),
                        v => artists(v),
                    }
                }
                "mpris:length" => {
                    m.length = value_i64(v)
                        .filter(|&us| us > 0 && us <= MAX_LENGTH_US)
                        .map(|us| Duration::from_micros(us as u64))
                }
                "mpris:artUrl" => m.art = value_str(v, MAX_URL + 1).as_deref().and_then(parse_art_url),
                _ => {}
            }
        }
        m
    }
}

/// `xesam:artist` normalde `as`; bazı oynatıcılar düz `s` gönderir.
fn artists(v: &Value<'_>) -> Vec<String> {
    match v {
        Value::Array(a) => a
            .inner()
            .iter()
            .take(MAX_ARTISTS)
            .filter_map(|e| value_str(e, MAX_TEXT))
            .filter(|s| !s.is_empty())
            .collect(),
        Value::Str(s) => vec![clean_str(s, MAX_TEXT)],
        _ => Vec::new(),
    }
}

/// Bir MPRIS oynatıcısı.
#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    /// Bus adı (`org.mpris.MediaPlayer2.spotify`): kimlik.
    pub bus_name: String,
    /// Görünen ad (`Identity`).
    pub identity: String,
    /// `.desktop` dosya adı (ikon için).
    pub desktop_entry: Option<String>,
    pub status: PlaybackStatus,
    pub metadata: Metadata,
    /// Son bilinen konum ve okunduğu an; şu anki konum [`Player::position_at`].
    pub position: Duration,
    pub position_at: Instant,
    /// Oynatma hızı (1.0 normal).
    pub rate: f64,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub can_seek: bool,
    pub can_control: bool,
    /// 0..=1 (destekliyorsa).
    pub volume: Option<f64>,
    /// En son "Playing" olduğu an (etkin oynatıcı seçimi).
    pub last_playing: Option<Instant>,
}

impl Player {
    pub fn new(bus_name: String, now: Instant) -> Self {
        let identity = bus_name.strip_prefix(super::MPRIS_PREFIX).unwrap_or(&bus_name).to_owned();
        Self {
            bus_name,
            identity,
            desktop_entry: None,
            status: PlaybackStatus::Stopped,
            metadata: Metadata::default(),
            position: Duration::ZERO,
            position_at: now,
            rate: 1.0,
            can_play: false,
            can_pause: false,
            can_go_next: false,
            can_go_previous: false,
            can_seek: false,
            can_control: false,
            volume: None,
            last_playing: None,
        }
    }

    /// `now` anındaki tahmini konum (oynarken hızla ilerler, uzunlukla sınırlı).
    pub fn position_at(&self, now: Instant) -> Duration {
        let mut p = self.position;
        if self.status == PlaybackStatus::Playing && self.rate > 0.0 {
            p += now.saturating_duration_since(self.position_at).mul_f64(self.rate.min(16.0));
        }
        match self.metadata.length {
            Some(len) => p.min(len),
            None => p,
        }
    }

    pub(crate) fn set_position_us(&mut self, us: i64, now: Instant) {
        self.position = Duration::from_micros(us.clamp(0, MAX_LENGTH_US) as u64);
        self.position_at = now;
    }

    /// Kök arayüz (`org.mpris.MediaPlayer2`) özellikleri.
    pub(crate) fn apply_root(&mut self, props: &PropMap) {
        if let Some(s) = prop_str(props, "Identity", 128).filter(|s| !s.is_empty()) {
            self.identity = s;
        }
        if props.contains_key("DesktopEntry") {
            self.desktop_entry = prop_str(props, "DesktopEntry", 128).filter(|s| !s.is_empty());
        }
    }

    /// `org.mpris.MediaPlayer2.Player` özellikleri. Konum değişebilecek bir şey olduysa `true`
    /// (çağıran `Position`'ı yeniden okur).
    pub(crate) fn apply_player(&mut self, props: &PropMap, now: Instant) -> bool {
        let mut position_stale = false;
        if let Some(s) = prop_str(props, "PlaybackStatus", 16).as_deref().and_then(PlaybackStatus::parse) {
            // Konumu durum değişmeden önceki tahminle dondur.
            self.position = self.position_at(now);
            self.position_at = now;
            self.status = s;
            position_stale = true;
            if s == PlaybackStatus::Playing {
                self.last_playing = Some(now);
            }
        }
        if let Some(v) = props.get("Metadata") {
            let m = Metadata::parse(v);
            if m.track_id != self.metadata.track_id || m.title != self.metadata.title {
                self.position = Duration::ZERO;
                self.position_at = now;
                position_stale = true;
            }
            self.metadata = m;
        }
        if let Some(r) = prop_f64(props, "Rate") {
            self.position = self.position_at(now);
            self.position_at = now;
            self.rate = r.clamp(0.0, 16.0);
            position_stale = true;
        }
        if let Some(us) = prop_i64(props, "Position") {
            self.set_position_us(us, now);
        }
        if let Some(v) = prop_f64(props, "Volume") {
            self.volume = Some(v.clamp(0.0, 1.0));
        }
        for (key, field) in [
            ("CanPlay", &mut self.can_play),
            ("CanPause", &mut self.can_pause),
            ("CanGoNext", &mut self.can_go_next),
            ("CanGoPrevious", &mut self.can_go_previous),
            ("CanSeek", &mut self.can_seek),
            ("CanControl", &mut self.can_control),
        ] {
            if let Some(b) = prop_bool(props, key) {
                *field = b;
            }
        }
        position_stale
    }
}

/// Etkin oynatıcı: çalan varsa en son çalmaya başlayan, yoksa en son çalmış olan, yoksa ilki.
pub fn active_player(players: &[Player]) -> Option<&Player> {
    players
        .iter()
        .filter(|p| p.status == PlaybackStatus::Playing)
        .max_by_key(|p| p.last_playing)
        .or_else(|| players.iter().filter(|p| p.last_playing.is_some()).max_by_key(|p| p.last_playing))
        .or_else(|| players.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{Dict, OwnedValue, Signature};

    fn meta(entries: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
        let mut d = Dict::new(&Signature::Str, &Signature::Variant);
        for (k, v) in entries {
            d.append(Value::from(k), Value::Value(Box::new(v))).unwrap();
        }
        Value::Dict(d)
    }

    #[test]
    fn art_urls_are_restricted() {
        assert_eq!(parse_art_url("file:///tmp/a%20b.png"), Some(ArtSource::File("/tmp/a b.png".into())));
        assert_eq!(
            parse_art_url("https://i.scdn.co/image/ab"),
            Some(ArtSource::Remote("https://i.scdn.co/image/ab".into()))
        );
        assert_eq!(parse_art_url("data:image/png;base64,AAAA"), None);
        assert_eq!(parse_art_url("file://evil.host/etc/passwd"), None);
        assert_eq!(parse_art_url("http://user:pw@host/x"), None);
        assert_eq!(parse_art_url("ftp://host/x"), None);
        assert_eq!(parse_art_url("relative.png"), None);
        assert_eq!(parse_art_url(&format!("https://h/{}", "a".repeat(MAX_URL))), None);
    }

    #[test]
    fn metadata_parses_tolerantly() {
        let m = Metadata::parse(&meta(vec![
            ("xesam:title", Value::from("Şarkı\n1")),
            ("xesam:artist", Value::from(vec!["A", "", "B"])),
            ("mpris:length", Value::U64(180_000_000)),
            ("mpris:artUrl", Value::from("file:///tmp/c.jpg")),
            ("mpris:trackid", Value::from("/org/mpris/MediaPlayer2/Track/1")),
            ("xesam:album", Value::I32(5)),
        ]));
        assert_eq!(m.title, "Şarkı 1");
        assert_eq!(m.artists, ["A", "B"]);
        assert_eq!(m.length, Some(Duration::from_secs(180)));
        assert_eq!(m.art, Some(ArtSource::File("/tmp/c.jpg".into())));
        assert_eq!(m.album, "", "yanlış tip atlanır");
        let m = Metadata::parse(&meta(vec![("xesam:artist", Value::from("Tek")), ("mpris:length", Value::I64(-5))]));
        assert_eq!(m.artists, ["Tek"]);
        assert_eq!(m.length, None);
    }

    #[test]
    fn position_extrapolates_and_freezes() {
        let t0 = Instant::now();
        let mut p = Player::new("org.mpris.MediaPlayer2.x".into(), t0);
        let props: PropMap = [
            ("PlaybackStatus".to_owned(), OwnedValue::try_from(Value::from("Playing")).unwrap()),
            ("Position".to_owned(), OwnedValue::from(10_000_000i64)),
        ]
        .into_iter()
        .collect();
        p.apply_player(&props, t0);
        assert_eq!(p.position_at(t0 + Duration::from_secs(2)), Duration::from_secs(12));
        let paused: PropMap =
            [("PlaybackStatus".to_owned(), OwnedValue::try_from(Value::from("Paused")).unwrap())].into_iter().collect();
        assert!(p.apply_player(&paused, t0 + Duration::from_secs(3)));
        assert_eq!(p.position_at(t0 + Duration::from_secs(60)), Duration::from_secs(13));
        assert_eq!(p.identity, "x");
    }

    #[test]
    fn active_prefers_latest_playing() {
        let t0 = Instant::now();
        let mut a = Player::new("a".into(), t0);
        let mut b = Player::new("b".into(), t0);
        assert_eq!(active_player(&[a.clone(), b.clone()]).unwrap().bus_name, "a");
        a.status = PlaybackStatus::Paused;
        a.last_playing = Some(t0 + Duration::from_secs(5));
        b.last_playing = Some(t0 + Duration::from_secs(1));
        assert_eq!(active_player(&[a.clone(), b.clone()]).unwrap().bus_name, "a");
        b.status = PlaybackStatus::Playing;
        assert_eq!(active_player(&[a, b]).unwrap().bus_name, "b");
    }
}
