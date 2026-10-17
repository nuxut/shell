//! Medya: MPRIS oynatıcıları (oturum bus'ı) — şu an çalan, kapak, oynat/duraklat/ileri/geri,
//! konum.
//!
//! **Keşif.** `org.mpris.MediaPlayer2.*` adları `ListNames` ve `NameOwnerChanged`
//! (`arg0namespace`) ile izlenir. Her oynatıcı bir kez `GetAll` ile okunur, sonra
//! `PropertiesChanged` ve `Seeked` ile güncellenir. `playerctld` (başka oynatıcıların
//! vekili) atlanır; aynı benzersiz sahibin birden çok adı (VLC: `vlc` + `vlc.instance…`) tek
//! oynatıcı olarak gösterilir.
//!
//! **Taklit koruması.** Sinyaller herkese açık bir kuralla (`/org/mpris/MediaPlayer2`) alınır,
//! ama yalnızca gönderen bir oynatıcı adının bus'ın bildirdiği **şu anki** benzersiz
//! sahibiyse o oynatıcıya uygulanır; `NameOwnerChanged` yalnızca bus'tan kabul edilir.
//!
//! **Konum.** MPRIS konumu sinyal olarak yayınlamaz; servis konumu yalnızca durum/parça/hız
//! değişince ve `Seeked`'de okur, UI [`Player::position_at`] ile tahmin eder (boşta iş yok).
//!
//! **Kapak.** `mpris:artUrl` yalnızca `file://` ve `http(s)://` kabul edilir
//! ([`parse_art_url`]). Yerel görsel yalnızca UI [`MediaService::fetch_art`] isteyince, ayrı
//! bir thread'de `services::safe_file` kurallarıyla (`O_NOFOLLOW`, normal dosya, sahte dosya
//! sistemi değil, [`MAX_ART_BYTES`]) okunur; içerik görsel imzası taşımalı. Ağ kapağı
//! oynatıcının verdiği güvenilmez bir adrestir (SSRF: yerel ağa/loopback'e istek attırabilir):
//! yalnızca `nuxut_net::HttpClient::fetch_untrusted_media` ile indirilir — yalnızca genel
//! internet (yönlendirmeler dahil), yalnızca https, boyut ve süre sınırlı; içerik yine görsel
//! imzası taşımalı. Reddedilen adreste UI yer tutucu gösterir.
//!
//! **Birleştirme.** Parça değişiminde oynatıcılar art arda birkaç `PropertiesChanged`
//! gönderir; anlık görüntü sessizlikten ([`MediaOptions::coalesce`]) sonra tek olay olarak
//! yayınlanır.

mod art;
mod model;

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_channel::Sender;
use futures_lite::future::FutureExt;
use nuxut::i18n::t;
use zbus::message::Type as MessageType;
use zbus::zvariant::{ObjectPath, Value};
use zbus::{Connection, MatchRule, Message, MessageStream};

pub use art::MAX_ART_BYTES;
pub use model::{ArtSource, Metadata, PlaybackStatus, Player, active_player, parse_art_url};

use nuxut_dbus::{
    Backoff, BusKind, DBUS_NAME, DBUS_PATH, NameOwner, PROPERTIES, PropMap, SIGNAL_QUEUE, ServiceContext,
    ServiceThread, clean_str, close, connect, forward, get_all, get_name_owner, parse_properties_changed,
    tick_connection,
};

pub(crate) const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_NAMESPACE: &str = "org.mpris.MediaPlayer2";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
/// Vekil oynatıcılar: kendileri bir oynatıcı değil.
const IGNORED: [&str; 1] = ["org.mpris.MediaPlayer2.playerctld"];
/// Aynı anda izlenen en fazla oynatıcı adı.
pub const MAX_PLAYERS: usize = 16;
const INPUT_CAPACITY: usize = 512;
const MAX_CHANGED: usize = 64;

#[derive(Clone, PartialEq)]
pub enum MediaEvent {
    /// Oynatıcılar (keşif sırasıyla, tekrarsız). Etkin olanı [`active_player`] seçer.
    Players(Vec<Player>),
    /// [`MediaService::fetch_art`] sonucu.
    Art { url: String, result: Result<Arc<[u8]>, String> },
    /// Bir komut oynatıcıda başarısız oldu.
    Failed { player: String, message: String },
}

/// Redakte: başlık/sanatçı/URL yazılmaz (log politikası).
impl std::fmt::Debug for MediaEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Players(p) => f.debug_tuple("Players").field(&p.len()).finish(),
            Self::Art { result, .. } => f.debug_struct("Art").field("ok", &result.is_ok()).finish_non_exhaustive(),
            Self::Failed { .. } => f.write_str("Failed(..)"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaOptions {
    /// D-Bus adresi; `None`: oturum bus'ı.
    pub bus_address: Option<String>,
    pub call_timeout: Duration,
    /// Değişiklik patlaması bu kadar sessizlikten sonra tek olay olur.
    pub coalesce: Duration,
    pub max_backoff: Duration,
}

impl Default for MediaOptions {
    fn default() -> Self {
        Self {
            bus_address: None,
            call_timeout: Duration::from_secs(2),
            coalesce: Duration::from_millis(25),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// Oynatıcıya gönderilen eylem.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaAction {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    /// Göreli atlama (mikrosaniye, negatif geri).
    Seek(i64),
    /// Mutlak konum (geçerli parçada).
    SetPosition(Duration),
    /// 0..=1.
    SetVolume(f64),
    /// Uygulama penceresini öne getir.
    Raise,
}

#[derive(Debug)]
enum Command {
    Action { player: String, action: MediaAction },
    RefreshPosition(String),
    FetchArt(String),
}

/// Çalışan medya servisi. Düşürülünce temiz kapanır.
#[derive(Debug)]
pub struct MediaService {
    thread: ServiceThread<Command>,
}

impl MediaService {
    pub fn spawn<F>(options: MediaOptions, sink: F) -> io::Result<Self>
    where
        F: Fn(MediaEvent) + Send + 'static,
    {
        let thread = ServiceThread::spawn("nuxut-media", 64, move |ctx| run(options, sink, ctx))?;
        Ok(Self { thread })
    }

    /// `player`: [`Player::bus_name`].
    pub fn action(&self, player: &str, action: MediaAction) -> bool {
        self.thread.send(Command::Action { player: player.to_owned(), action })
    }

    /// Konumu oynatıcıdan yeniden okur (panel açılınca; tahmin kaydıysa).
    pub fn refresh_position(&self, player: &str) -> bool {
        self.thread.send(Command::RefreshPosition(player.to_owned()))
    }

    /// Kapak görselini yükler ([`ArtSource::url`]; yerel dosya güvenli okunur, ağ kapağı
    /// yalnızca genel internetten https ile). Sonuç [`MediaEvent::Art`]; UI dosyayı ya da
    /// adresi kendisi açmaz.
    pub fn fetch_art(&self, url: &str) -> bool {
        self.thread.send(Command::FetchArt(url.to_owned()))
    }
}

#[derive(Debug)]
enum Input {
    OwnerChanged(Message),
    Signal(Message),
    Loaded { name: String, owner: String, result: zbus::Result<(PropMap, PropMap, Message)> },
    Position { name: String, owner: String, us: i64 },
    Failed { player: String, message: String },
    Art { url: String, result: Result<Arc<[u8]>, String> },
}

enum End {
    Stop,
    Lost,
}

async fn run<F: Fn(MediaEvent)>(options: MediaOptions, sink: F, ctx: ServiceContext<Command>) {
    let mut backoff = Backoff::new(options.max_backoff);
    let mut art = art::ArtCache::default();
    loop {
        match connect(BusKind::Session, options.bus_address.as_deref(), options.call_timeout).await {
            Ok(conn) => {
                backoff.reset();
                let ticker = tick_connection(&ctx.ex, &conn);
                let mut rt = Runtime {
                    sink: &sink,
                    options: &options,
                    conn: conn.clone(),
                    entries: Vec::new(),
                    dirty: None,
                    emitted: None,
                    art: &mut art,
                };
                let end = rt.session(&ctx).await;
                drop(rt);
                drop(ticker);
                close(conn).await;
                if matches!(end, End::Stop) {
                    return;
                }
                tracing::warn!("medya: oturum bus bağlantısı koptu, yeniden bağlanılacak");
            }
            Err(e) => tracing::warn!("medya: oturum bus'ına bağlanılamadı: {e}"),
        }
        sink(MediaEvent::Players(Vec::new()));
        if !ctx.sleep(backoff.next_delay()).await {
            return;
        }
    }
}

struct Entry {
    owner: NameOwner,
    player: Player,
    /// İlk okuma tamamlandı mı (öncesinde yayınlanmaz).
    loaded: bool,
}

struct Runtime<'a, F> {
    sink: &'a F,
    options: &'a MediaOptions,
    conn: Connection,
    entries: Vec<Entry>,
    /// Bekleyen yayın zamanı.
    dirty: Option<Instant>,
    emitted: Option<Vec<Player>>,
    art: &'a mut art::ArtCache,
}

enum Wake {
    Stop,
    Lost,
    Input(Input),
    Command(Command),
    Flush,
}

impl<F: Fn(MediaEvent)> Runtime<'_, F> {
    async fn session(&mut self, ctx: &ServiceContext<Command>) -> End {
        let (tx, rx) = async_channel::bounded::<Input>(INPUT_CAPACITY);
        let streams = async {
            let owners = MatchRule::builder()
                .msg_type(MessageType::Signal)
                .sender(DBUS_NAME)?
                .interface(DBUS_NAME)?
                .member("NameOwnerChanged")?
                .arg0ns(MPRIS_NAMESPACE)?
                .build();
            let signals = MatchRule::builder().msg_type(MessageType::Signal).path(MPRIS_PATH)?.build();
            Ok::<_, zbus::Error>((
                MessageStream::for_match_rule(owners, &self.conn, Some(64)).await?,
                MessageStream::for_match_rule(signals, &self.conn, Some(SIGNAL_QUEUE)).await?,
            ))
        };
        let (owners, signals) = match streams.await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("medya: sinyal aboneliği kurulamadı: {e}");
                return End::Lost;
            }
        };
        let _tasks = [
            forward(&ctx.ex, owners, tx.clone(), Input::OwnerChanged),
            forward(&ctx.ex, signals, tx.clone(), Input::Signal),
        ];

        match list_names(&self.conn).await {
            Ok(names) => {
                for name in names.into_iter().filter(|n| is_player_name(n)) {
                    if let Some(owner) = get_name_owner(&self.conn, &name).await {
                        self.add(ctx, &tx, name, owner);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("medya: ListNames: {e}");
                return End::Lost;
            }
        }
        // Hiç oynatıcı yoksa da ilk durum yayınlanır.
        self.dirty = Some(Instant::now());

        loop {
            let flush_at = self.dirty;
            let wake = async { rx.recv().await.map_or(Wake::Lost, Wake::Input) }
                .or(async { ctx.commands.recv().await.map_or(Wake::Stop, Wake::Command) })
                .or(async { ctx.stop.recv().await.map_or(Wake::Stop, |()| Wake::Stop) })
                .or(async {
                    match flush_at {
                        Some(at) => {
                            async_io::Timer::at(at).await;
                        }
                        None => std::future::pending::<()>().await,
                    }
                    Wake::Flush
                })
                .await;
            match wake {
                Wake::Stop => return End::Stop,
                Wake::Lost => return End::Lost,
                Wake::Flush => self.flush(),
                Wake::Command(c) => self.command(ctx, &tx, c),
                Wake::Input(i) => self.input(ctx, &tx, i),
            }
        }
    }

    fn mark_dirty(&mut self) {
        let at = Instant::now() + self.options.coalesce;
        self.dirty = Some(self.dirty.map_or(at, |d| d.min(at).max(Instant::now())));
    }

    /// Oynatıcı listesi (yüklenmiş, sahibi tekrarsız: aynı sahibin adlarından en kısası).
    fn players(&self) -> Vec<Player> {
        let mut out: Vec<(&str, &Player)> = Vec::new();
        for e in self.entries.iter().filter(|e| e.loaded) {
            let Some(owner) = e.owner.owner.as_deref() else { continue };
            match out.iter_mut().find(|(o, _)| *o == owner) {
                Some(slot) => {
                    let (a, b) = (&e.player.bus_name, &slot.1.bus_name);
                    if (a.len(), a) < (b.len(), b) {
                        slot.1 = &e.player;
                    }
                }
                None => out.push((owner, &e.player)),
            }
        }
        out.into_iter().map(|(_, p)| p.clone()).collect()
    }

    fn flush(&mut self) {
        self.dirty = None;
        let players = self.players();
        if self.emitted.as_ref() == Some(&players) {
            return;
        }
        self.emitted = Some(players.clone());
        (self.sink)(MediaEvent::Players(players));
    }

    fn add(&mut self, ctx: &ServiceContext<Command>, tx: &Sender<Input>, name: String, owner: String) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.player.bus_name == name) {
            if e.owner.owner.as_deref() == Some(owner.as_str()) {
                return;
            }
            e.owner.set_owner(Some(owner.clone()));
            e.loaded = false;
            e.player = Player::new(name.clone(), Instant::now());
        } else {
            if self.entries.len() >= MAX_PLAYERS {
                tracing::warn!("medya: oynatıcı sınırı ({MAX_PLAYERS}) aşıldı, {name} yok sayıldı");
                return;
            }
            let mut owner_state = NameOwner::new();
            owner_state.set_owner(Some(owner.clone()));
            self.entries.push(Entry {
                owner: owner_state,
                player: Player::new(name.clone(), Instant::now()),
                loaded: false,
            });
        }
        let conn = self.conn.clone();
        let tx = tx.clone();
        ctx.ex
            .spawn(async move {
                let result = async {
                    let (root, _) = get_all(&conn, &owner, MPRIS_PATH, ROOT_IFACE).await?;
                    let (player, reply) = get_all(&conn, &owner, MPRIS_PATH, PLAYER_IFACE).await?;
                    Ok((root, player, reply))
                }
                .await;
                let _ = tx.send(Input::Loaded { name, owner, result }).await;
            })
            .detach();
    }

    fn remove(&mut self, name: &str) {
        let before = self.entries.len();
        self.entries.retain(|e| e.player.bus_name != name);
        if self.entries.len() != before {
            self.mark_dirty();
        }
    }

    fn input(&mut self, ctx: &ServiceContext<Command>, tx: &Sender<Input>, input: Input) {
        let now = Instant::now();
        match input {
            Input::OwnerChanged(msg) => {
                let Some((name, new_owner)) = parse_mpris_owner_change(&msg) else { return };
                match new_owner {
                    Some(owner) => self.add(ctx, tx, name, owner),
                    None => self.remove(&name),
                }
            }
            Input::Loaded { name, owner, result } => {
                let Some(e) = self.entries.iter_mut().find(|e| e.player.bus_name == name) else { return };
                if e.owner.owner.as_deref() != Some(owner.as_str()) {
                    return; // Sahip bu arada değişti; yeni okuma yolda.
                }
                match result {
                    Ok((root, player, reply)) => {
                        e.owner.loaded(&reply);
                        e.player.apply_root(&root);
                        e.player.apply_player(&player, now);
                        if e.player.status == PlaybackStatus::Playing {
                            e.player.last_playing = Some(now);
                        }
                        e.loaded = true;
                    }
                    Err(err) => {
                        tracing::debug!(name, "medya: oynatıcı okunamadı: {err}");
                        self.entries.retain(|e| e.player.bus_name != name);
                    }
                }
                self.mark_dirty();
            }
            Input::Signal(msg) => self.signal(ctx, tx, &msg, now),
            Input::Position { name, owner, us } => {
                if let Some(e) = self
                    .entries
                    .iter_mut()
                    .find(|e| e.player.bus_name == name && e.owner.owner.as_deref() == Some(owner.as_str()))
                {
                    e.player.set_position_us(us, now);
                    self.mark_dirty();
                }
            }
            Input::Failed { player, message } => (self.sink)(MediaEvent::Failed { player, message }),
            Input::Art { url, result } => {
                if let Ok(data) = &result {
                    self.art.store(&url, data.clone());
                }
                (self.sink)(MediaEvent::Art { url, result });
            }
        }
    }

    fn signal(&mut self, ctx: &ServiceContext<Command>, tx: &Sender<Input>, msg: &Message, now: Instant) {
        let header = msg.header();
        let member = header.member().map(|m| m.to_string()).unwrap_or_default();
        let iface = header.interface().map(|i| i.to_string()).unwrap_or_default();
        let mut refresh = Vec::new();
        let mut changed = false;
        for e in self.entries.iter_mut().filter(|e| e.loaded && e.owner.accepts(msg)) {
            match (iface.as_str(), member.as_str()) {
                (PROPERTIES, "PropertiesChanged") => {
                    let Some((target, props, _)) = parse_properties_changed(msg, MAX_CHANGED) else { continue };
                    match target.as_str() {
                        PLAYER_IFACE => {
                            if e.player.apply_player(&props, now) && e.player.status != PlaybackStatus::Stopped {
                                refresh.push(e.player.bus_name.clone());
                            }
                        }
                        ROOT_IFACE => e.player.apply_root(&props),
                        _ => continue,
                    }
                    changed = true;
                }
                (PLAYER_IFACE, "Seeked") => {
                    if let Ok(us) = msg.body().deserialize::<i64>() {
                        e.player.set_position_us(us, now);
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        if changed {
            self.mark_dirty();
        }
        for name in refresh {
            self.read_position(ctx, tx, &name);
        }
    }

    fn read_position(&self, ctx: &ServiceContext<Command>, tx: &Sender<Input>, name: &str) {
        let Some(owner) = self.entries.iter().find(|e| e.player.bus_name == name).and_then(|e| e.owner.owner.clone())
        else {
            return;
        };
        let conn = self.conn.clone();
        let tx = tx.clone();
        let name = name.to_owned();
        ctx.ex
            .spawn(async move {
                let reply = conn
                    .call_method(Some(owner.as_str()), MPRIS_PATH, Some(PROPERTIES), "Get", &(PLAYER_IFACE, "Position"))
                    .await;
                let us = reply.ok().and_then(|r| {
                    r.body().deserialize::<Value<'_>>().ok().and_then(|v| match v {
                        Value::Value(b) => nuxut_dbus::value_i64(&b),
                        v => nuxut_dbus::value_i64(&v),
                    })
                });
                if let Some(us) = us {
                    let _ = tx.send(Input::Position { name, owner, us }).await;
                }
            })
            .detach();
    }

    fn command(&mut self, ctx: &ServiceContext<Command>, tx: &Sender<Input>, command: Command) {
        match command {
            Command::RefreshPosition(name) => self.read_position(ctx, tx, &name),
            Command::FetchArt(url) => {
                if let Some(data) = self.art.get(&url) {
                    (self.sink)(MediaEvent::Art { url, result: Ok(data) });
                    return;
                }
                if parse_art_url(&url).is_none() {
                    (self.sink)(MediaEvent::Art { url, result: Err("yalnızca file:// ve https:// kapaklar".into()) });
                    return;
                }
                let tx = tx.clone();
                self.art.fetch(url, move |url, result| {
                    let _ = tx.send_blocking(Input::Art { url, result });
                });
            }
            Command::Action { player, action } => {
                let Some(e) = self.entries.iter().find(|e| e.player.bus_name == player && e.loaded) else {
                    (self.sink)(MediaEvent::Failed { player, message: t!("shell-media-error-player-missing") });
                    return;
                };
                let Some(owner) = e.owner.owner.clone() else { return };
                let track = e.player.metadata.track_id.clone();
                let conn = self.conn.clone();
                let tx = tx.clone();
                ctx.ex
                    .spawn(async move {
                        if let Err(err) = perform(&conn, &owner, track.as_deref(), &action).await {
                            // `Failure`: `perform`'un kendi (katalogdan gelen) metni; D-Bus hataları ayrıntı olur.
                            let message = match &err {
                                zbus::Error::Failure(m) => m.clone(),
                                _ => t!("shell-media-error-action-failed", detail = clean_str(&err.to_string(), 200)),
                            };
                            let _ = tx.send(Input::Failed { player, message }).await;
                        }
                    })
                    .detach();
            }
        }
    }
}

async fn perform(conn: &Connection, owner: &str, track: Option<&str>, action: &MediaAction) -> zbus::Result<()> {
    let call = |method: &'static str| async move {
        conn.call_method(Some(owner), MPRIS_PATH, Some(PLAYER_IFACE), method, &()).await.map(drop)
    };
    match action {
        MediaAction::PlayPause => call("PlayPause").await,
        MediaAction::Play => call("Play").await,
        MediaAction::Pause => call("Pause").await,
        MediaAction::Stop => call("Stop").await,
        MediaAction::Next => call("Next").await,
        MediaAction::Previous => call("Previous").await,
        MediaAction::Raise => conn.call_method(Some(owner), MPRIS_PATH, Some(ROOT_IFACE), "Raise", &()).await.map(drop),
        MediaAction::Seek(us) => {
            conn.call_method(Some(owner), MPRIS_PATH, Some(PLAYER_IFACE), "Seek", &(*us,)).await.map(drop)
        }
        MediaAction::SetPosition(pos) => {
            let track = track.and_then(|t| ObjectPath::try_from(t).ok()).ok_or_else(|| {
                zbus::Error::Failure(t!("shell-media-error-no-track-id"))
            })?;
            let us = i64::try_from(pos.as_micros()).unwrap_or(i64::MAX);
            conn.call_method(Some(owner), MPRIS_PATH, Some(PLAYER_IFACE), "SetPosition", &(track, us)).await.map(drop)
        }
        MediaAction::SetVolume(v) => {
            let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { return Ok(()) };
            nuxut_dbus::set_property(conn, owner, MPRIS_PATH, PLAYER_IFACE, "Volume", Value::F64(v))
                .await
        }
    }
}

fn is_player_name(name: &str) -> bool {
    name.starts_with(MPRIS_PREFIX) && name.len() <= 255 && !IGNORED.contains(&name)
}

async fn list_names(conn: &Connection) -> zbus::Result<Vec<String>> {
    let reply = conn.call_method(Some(DBUS_NAME), DBUS_PATH, Some(DBUS_NAME), "ListNames", &()).await?;
    let names: Vec<String> = reply.body().deserialize()?;
    Ok(names)
}

/// Bus'tan gelen MPRIS `NameOwnerChanged`: (ad, yeni sahip).
fn parse_mpris_owner_change(msg: &Message) -> Option<(String, Option<String>)> {
    let header = msg.header();
    if header.sender().map(|s| s.as_str()) != Some(DBUS_NAME) {
        return None;
    }
    let (name, _old, new): (String, String, String) = msg.body().deserialize().ok()?;
    if !is_player_name(&name) {
        return None;
    }
    Some((name, NameOwner::is_unique(&new).then_some(new)))
}
