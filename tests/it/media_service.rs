//! Medya servisi: sahte MPRIS oynatıcılarıyla özel bus'ta uçtan uca.
//!
//! Kapsam: keşif (başlangıçta var olan ve sonradan gelen), playerctld'nin atlanması, aynı
//! sahibin iki adının tek oynatıcı olması, metadata/durum değişimi, Seeked, sahte (unicast)
//! sinyalin reddi, eylemler, oynatıcının bus'tan düşmesi, kapak URL doğrulaması.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nuxut_shell::services::media::{
    ArtSource, MediaAction, MediaEvent, MediaOptions, MediaService, PlaybackStatus, Player,
};
use crate::support::{Events, PrivateBus, channel, wait_until};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const PATH: &str = "/org/mpris/MediaPlayer2";

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn push(&self, s: String) {
        self.0.lock().unwrap().push(s);
    }
    fn has(&self, s: &str) -> bool {
        self.0.lock().unwrap().iter().any(|e| e == s)
    }
}

struct Root {
    identity: String,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.clone()
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "fakeplayer".into()
    }
    fn raise(&self) {}
}

struct FakePlayer {
    status: String,
    title: String,
    art: String,
    log: Log,
}

fn meta(title: &str, art: &str) -> HashMap<String, OwnedValue> {
    let mut m = HashMap::new();
    m.insert("xesam:title".into(), OwnedValue::try_from(Value::from(title)).unwrap());
    m.insert("xesam:artist".into(), OwnedValue::try_from(Value::from(vec!["Sanatçı"])).unwrap());
    m.insert("mpris:length".into(), OwnedValue::from(240_000_000i64));
    m.insert(
        "mpris:trackid".into(),
        OwnedValue::try_from(Value::from(OwnedObjectPath::try_from(format!("/track/{}", title.len())).unwrap()))
            .unwrap(),
    );
    m.insert("mpris:artUrl".into(), OwnedValue::try_from(Value::from(art)).unwrap());
    m
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl FakePlayer {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.status.clone()
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        meta(&self.title, &self.art)
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        30_000_000
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
    fn can_control(&self) -> bool {
        true
    }
    fn play_pause(&self) {
        self.log.push("PlayPause".into());
    }
    fn next(&self) {
        self.log.push("Next".into());
    }
    fn seek(&self, offset: i64) {
        self.log.push(format!("Seek {offset}"));
    }
    fn set_position(&self, track: ObjectPath<'_>, pos: i64) {
        self.log.push(format!("SetPosition {track} {pos}"));
    }
    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
}

fn serve(bus: &PrivateBus, names: &[&str], identity: &str, title: &str, log: &Log) -> zbus::blocking::Connection {
    let conn = bus.connect();
    conn.object_server().at(PATH, Root { identity: identity.into() }).unwrap();
    conn.object_server()
        .at(
            PATH,
            FakePlayer {
                status: "Playing".into(),
                title: title.into(),
                art: "file:///tmp/kapak.png".into(),
                log: log.clone(),
            },
        )
        .unwrap();
    for n in names {
        conn.request_name(*n).unwrap();
    }
    conn
}

fn start(bus: &PrivateBus) -> (MediaService, Events<MediaEvent>) {
    let (sink, events) = channel();
    let options = MediaOptions {
        bus_address: Some(bus.address.clone()),
        call_timeout: Duration::from_millis(500),
        coalesce: Duration::from_millis(10),
        max_backoff: Duration::from_millis(200),
    };
    (MediaService::spawn(options, sink).unwrap(), events)
}

fn players(events: &Events<MediaEvent>, what: &str, pred: impl Fn(&[Player]) -> bool) -> Vec<Player> {
    match events.wait_for(what, |e| matches!(e, MediaEvent::Players(p) if pred(p))) {
        MediaEvent::Players(p) => p,
        _ => unreachable!(),
    }
}

fn set_title(conn: &zbus::blocking::Connection, title: &str, status: &str) {
    let iface = conn.object_server().interface::<_, FakePlayer>(PATH).unwrap();
    let emitter = iface.signal_emitter().clone();
    let mut p = iface.get_mut();
    p.title = title.into();
    p.status = status.into();
    zbus::block_on(async {
        p.metadata_changed(&emitter).await.unwrap();
        p.playback_status_changed(&emitter).await.unwrap();
    });
}

#[test]
fn discovery_metadata_and_actions() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    // Başlangıçta var: iki adla aynı sahip (tek oynatıcı olmalı) + playerctld vekili.
    let a = serve(&bus, &["org.mpris.MediaPlayer2.vlc", "org.mpris.MediaPlayer2.vlc.instance7"], "VLC", "Bir", &log);
    let _proxy = serve(&bus, &["org.mpris.MediaPlayer2.playerctld"], "playerctld", "Vekil", &Log::default());
    let (svc, events) = start(&bus);

    // İki ad ayrı yüklenir: kısa ad yüklenene dek uzun ad görünebilir.
    let list = players(&events, "ilk oynatıcı", |p| p.len() == 1 && p[0].bus_name == "org.mpris.MediaPlayer2.vlc");
    let p = &list[0];
    assert_eq!(p.bus_name, "org.mpris.MediaPlayer2.vlc");
    assert_eq!(p.identity, "VLC");
    assert_eq!(p.desktop_entry.as_deref(), Some("fakeplayer"));
    assert_eq!(p.status, PlaybackStatus::Playing);
    assert_eq!(p.metadata.title, "Bir");
    assert_eq!(p.metadata.artists, ["Sanatçı"]);
    assert_eq!(p.metadata.length, Some(Duration::from_secs(240)));
    assert_eq!(p.metadata.art, Some(ArtSource::File("/tmp/kapak.png".into())));
    assert!(p.position_at(Instant::now()) >= Duration::from_secs(30));
    assert!(p.can_go_next && p.can_control);

    // Sonradan gelen ikinci oynatıcı.
    let b = serve(&bus, &["org.mpris.MediaPlayer2.spotify"], "Spotify", "İki", &log);
    players(&events, "ikinci oynatıcı", |p| p.len() == 2);

    // Parça değişimi.
    set_title(&a, "Üç", "Paused");
    players(&events, "yeni parça", |p| {
        p.iter().any(|p| p.metadata.title == "Üç" && p.status == PlaybackStatus::Paused)
    });

    // Seeked.
    let iface = a.object_server().interface::<_, FakePlayer>(PATH).unwrap();
    zbus::block_on(FakePlayer::seeked(iface.signal_emitter(), 99_000_000)).unwrap();
    players(&events, "seeked", |p| {
        p.iter().any(|p| p.bus_name.ends_with("vlc") && p.position_at(Instant::now()) == Duration::from_secs(99))
    });

    // Eylemler.
    svc.action("org.mpris.MediaPlayer2.vlc", MediaAction::PlayPause);
    svc.action("org.mpris.MediaPlayer2.vlc", MediaAction::Next);
    svc.action("org.mpris.MediaPlayer2.vlc", MediaAction::Seek(-5_000_000));
    svc.action("org.mpris.MediaPlayer2.vlc", MediaAction::SetPosition(Duration::from_secs(10)));
    wait_until("eylemler", || {
        log.has("PlayPause") && log.has("Next") && log.has("Seek -5000000") && log.has("SetPosition /track/4 10000000")
    });
    svc.action("org.mpris.MediaPlayer2.yok", MediaAction::Play);
    events.wait_for("bilinmeyen oynatıcı", |e| matches!(e, MediaEvent::Failed { .. }));

    // Oynatıcı kapanır.
    drop(b);
    players(&events, "spotify gitti", |p| p.len() == 1);
    drop(svc);
}

#[test]
fn spoofed_signal_is_ignored_and_art_is_validated() {
    let Some(bus) = PrivateBus::start() else { return };
    let log = Log::default();
    let _a = serve(&bus, &["org.mpris.MediaPlayer2.mpv"], "mpv", "Gerçek", &log);
    let (svc, events) = start(&bus);
    players(&events, "oynatıcı", |p| p.len() == 1);

    let attacker = bus.connect();
    let names: Vec<String> = attacker
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "ListNames",
            &(),
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    let changed: HashMap<&str, Value<'_>> = HashMap::from([("PlaybackStatus", Value::from("Stopped"))]);
    for name in names.iter().filter(|n| n.starts_with(':')) {
        let _ = attacker.emit_signal(
            Some(name.as_str()),
            PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &("org.mpris.MediaPlayer2.Player", &changed, Vec::<String>::new()),
        );
        // Sahte NameOwnerChanged: bus'tan gelmediği için yok sayılmalı.
        let _ = attacker.emit_signal(
            Some(name.as_str()),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameOwnerChanged",
            &("org.mpris.MediaPlayer2.mpv", ":1.1", ""),
        );
    }
    let bad = events.collect(Duration::from_millis(300)).into_iter().any(|e| {
        matches!(&e, MediaEvent::Players(p) if p.is_empty() || p.iter().any(|p| p.status == PlaybackStatus::Stopped))
    });
    assert!(!bad, "sahte sinyal kabul edildi");

    // Yerel kapak güvenli okunur; görsel olmayan dosya ve data: URL'si reddedilir.
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("kapak.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n0000").unwrap();
    let png_url = nuxut_shell::services::media::ArtSource::File(png).url();
    svc.fetch_art(&png_url);
    events.wait_for("yerel kapak", |e| matches!(e, MediaEvent::Art { url, result: Ok(d) } if *url == png_url && d.len() == 12));
    svc.fetch_art("file:///etc/passwd");
    events.wait_for("file reddi", |e| matches!(e, MediaEvent::Art { result: Err(_), .. }));
    svc.fetch_art("data:image/png;base64,AAAA");
    events.wait_for("data reddi", |e| matches!(e, MediaEvent::Art { result: Err(_), .. }));

    // Ağ kapakları SSRF-güvenli istemciyle: loopback'e, özel ağa ve metadata adresine (http ya
    // da https) hiç bağlantı yapılmaz. Gerçek internete çıkan adres testte kullanılmaz.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let local = format!("http://127.0.0.1:{port}/kapak.png");
    // IPv6 loopback yoksa (bazı CI'lar) yalnızca IPv4 dinleyicisiyle doğrulanır.
    let listener6 = std::net::TcpListener::bind("[::1]:0").ok();
    if let Some(l) = &listener6 {
        l.set_nonblocking(true).unwrap();
    }
    let local6 = listener6.as_ref().map(|l| format!("https://[::1]:{}/kapak.png", l.local_addr().unwrap().port()));
    let mut urls = vec![
        local.clone(),
        "http://169.254.169.254/latest/meta-data/".into(),
        "http://10.0.0.1/kapak.png".into(),
        "http://192.168.1.1:631/".into(),
        format!("https://127.0.0.1:{port}/kapak.png"),
        "https://169.254.169.254/latest/meta-data/".into(),
        "https://10.0.0.1/kapak.png".into(),
    ];
    urls.extend(local6.clone());
    for url in &urls {
        let url = url.as_str();
        svc.fetch_art(url);
        events.wait_for("ağ kapağı reddi", |e| {
            matches!(e, MediaEvent::Art { url: u, result: Err(m) } if u == url && !m.is_empty())
        });
    }
    std::thread::sleep(Duration::from_millis(200));
    assert!(listener.accept().is_err(), "loopback'e bağlantı yapıldı");
    if let Some(l) = &listener6 {
        assert!(l.accept().is_err(), "::1'e bağlantı yapıldı");
    }
}
