//! Sahte MPRIS oynatıcısı (yalnızca izole test ortamında görsel test için): bar'ın
//! `now_playing` widget'ını ve medya kartını gerçek bir oynatıcı olmadan çizdirir. Oynat/duraklat,
//! sonraki/önceki çağrıları durumu gerçekten değiştirir (widget'ın tepkisi görülsün).
//!
//! ```sh
//! dbus-run-session -- sh betik.sh               # özel oturum bus'ı
//! cargo run --example fake_mpris -- --title "Parça" --artist "Sanatçı" --art /yol/kapak.png &
//! nuxut-shell
//! ```
//!
//! Seçenekler: `--title`, `--artist`, `--art <mutlak yol>` (file:// kapak), `--paused`,
//! `--name <ek>` (bus adı `org.mpris.MediaPlayer2.<ek>`, varsayılan `nuxuttest`).
//!
//! Güvenlik: yalnızca `DBUS_SESSION_BUS_ADDRESS` bus'ına bağlanır; bu bus `dbus-run-session`'ın
//! özel bus'ı olmalıdır (kullanıcının oturumundaki oynatıcılarla karışmaz).

use std::collections::HashMap;
use std::time::Duration;

use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PATH: &str = "/org/mpris/MediaPlayer2";

struct Root;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> String {
        "Nuxut Test".into()
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "nuxut-test-player".into()
    }
    fn raise(&self) {}
}

struct Player {
    playing: bool,
    track: u32,
    titles: Vec<String>,
    artist: String,
    art: String,
}

impl Player {
    fn status(&self) -> String {
        if self.playing { "Playing" } else { "Paused" }.into()
    }

    fn meta(&self) -> HashMap<String, OwnedValue> {
        let title = &self.titles[self.track as usize % self.titles.len()];
        let mut m = HashMap::new();
        let v = |x: Value<'_>| OwnedValue::try_from(x).expect("sabit değer");
        m.insert("xesam:title".into(), v(Value::from(title.as_str())));
        m.insert("xesam:artist".into(), v(Value::from(vec![self.artist.as_str()])));
        m.insert("xesam:album".into(), v(Value::from("Test Albümü")));
        m.insert("mpris:length".into(), OwnedValue::from(215_000_000i64));
        let id = OwnedObjectPath::try_from(format!("/org/nuxut/track/{}", self.track)).expect("geçerli yol");
        m.insert("mpris:trackid".into(), v(Value::from(id)));
        if !self.art.is_empty() {
            m.insert("mpris:artUrl".into(), v(Value::from(self.art.as_str())));
        }
        m
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.status()
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        self.meta()
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        42_000_000
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
    async fn play_pause(&mut self, #[zbus(signal_emitter)] e: SignalEmitter<'_>) {
        self.playing = !self.playing;
        eprintln!("fake_mpris: PlayPause → {}", self.status());
        let _ = self.playback_status_changed(&e).await;
    }
    async fn next(&mut self, #[zbus(signal_emitter)] e: SignalEmitter<'_>) {
        self.track = self.track.wrapping_add(1);
        eprintln!("fake_mpris: Next → {}", self.track);
        let _ = self.metadata_changed(&e).await;
    }
    async fn previous(&mut self, #[zbus(signal_emitter)] e: SignalEmitter<'_>) {
        self.track = self.track.wrapping_sub(1);
        eprintln!("fake_mpris: Previous → {}", self.track);
        let _ = self.metadata_changed(&e).await;
    }
}

fn main() -> zbus::Result<()> {
    let mut title = "Uçamayan Penguen".to_owned();
    let mut artist = "Nuxut Orkestrası".to_owned();
    let mut art = String::new();
    let mut playing = true;
    let mut name = "nuxuttest".to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--title" => title = args.next().unwrap_or_default(),
            "--artist" => artist = args.next().unwrap_or_default(),
            "--art" => art = args.next().map(|p| format!("file://{p}")).unwrap_or_default(),
            "--paused" => playing = false,
            "--name" => name = args.next().filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or(name),
            other => eprintln!("fake_mpris: bilinmeyen seçenek {other}"),
        }
    }
    let titles = vec![title, "İkinci Parça".into(), "Üçüncü Parça (Uzun Bir Başlıkla Birlikte Gelen Sürüm)".into()];
    let _conn = zbus::blocking::connection::Builder::session()?
        .serve_at(PATH, Root)?
        .serve_at(PATH, Player { playing, track: 0, titles, artist, art })?
        .name(format!("org.mpris.MediaPlayer2.{name}"))?
        .build()?;
    eprintln!("fake_mpris: hazır (org.mpris.MediaPlayer2.{name})");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
