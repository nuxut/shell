//! Takvim istemcisi: `nuxut-calendard` (`org.nuxut.Calendar1`, oturum bus'ı) üzerinden
//! etkinlik oluşumları.
//!
//! Tek thread, üzerinde zbus'un async API'si (`async-io` yürütücüsü; ek runtime yok). UI hangi
//! aralığı gösterdiğini [`CalendarService::set_range`] ile bildirir (ay görünümü: 6 hafta);
//! servis o aralığı sorgular ve daemon'un `OccurrencesChanged`/`CalendarsChanged`
//! sinyallerinde, ayrıca daemon bus'a gelince yeniden sorgular (150 ms birleştirme). Sinyaller
//! yalnızca `org.nuxut.Calendar1` adının güncel sahibinden kabul edilir ([`OwnerGate`]) ve
//! içerikleri okunmaz: sinyal yalnızca "yenile" tetikleyicisidir. Daemon
//! yoksa (paket kurulmamış) [`CalendarEvent::Unavailable`] gelir; shell takvimi boş gösterir,
//! hata vermez. Dış girdi sınırlı: en çok [`MAX_ENTRIES`] oluşum, metinler kırpılır, renk
//! yalnızca `#rrggbb`.

use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use futures_lite::StreamExt;
use nuxut_calendar_ipc::{BUS_NAME, CalendarProxy, INTERFACE, OccurrenceEntry};

/// Bir yanıtta en çok kaç oluşum tutulur (daemon da 5000 ile sınırlar).
pub const MAX_ENTRIES: usize = 5000;
/// Gösterilen metinlerin karakter sınırı.
const MAX_TEXT: usize = 256;
/// Sinyal patlamasını tek sorguya indiren sessizlik.
const COALESCE: Duration = Duration::from_millis(150);

#[derive(Clone, PartialEq, Eq)]
pub enum CalendarEvent {
    /// `[from, to)` aralığının oluşumları (başlangıca göre sıralı, temizlenmiş).
    Occurrences { from: i64, to: i64, entries: Vec<OccurrenceEntry> },
    /// Daemon yok ya da cevap vermiyor; takvim boş gösterilir.
    Unavailable,
}

/// Debug redakte (log politikası): etkinlik başlığı/konumu yazılmaz, yalnızca sayı.
impl std::fmt::Debug for CalendarEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Occurrences { from, to, entries } => f
                .debug_struct("Occurrences")
                .field("from", from)
                .field("to", to)
                .field("entries", &entries.len())
                .finish(),
            Self::Unavailable => f.write_str("Unavailable"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CalendarOptions {
    /// D-Bus adresi; `None`: oturum bus'ı.
    pub bus_address: Option<String>,
}

#[derive(Debug)]
enum Command {
    Range { from: i64, to: i64 },
}

/// Çalışan takvim istemcisi. Düşürülünce kuyruk kapanır, thread çıkar ve join edilir.
#[derive(Debug)]
pub struct CalendarService {
    tx: Option<async_channel::Sender<Command>>,
    thread: Option<JoinHandle<()>>,
}

impl CalendarService {
    pub fn spawn<F>(options: CalendarOptions, sink: F) -> std::io::Result<Self>
    where
        F: Fn(CalendarEvent) + Send + 'static,
    {
        let (tx, rx) = async_channel::bounded::<Command>(16);
        let thread = std::thread::Builder::new()
            .name("nuxut-calendar".into())
            .spawn(move || async_io::block_on(run(options, rx, sink)))?;
        Ok(Self { tx: Some(tx), thread: Some(thread) })
    }

    /// Gösterilen aralığı bildirir; servis sorgular ve [`CalendarEvent::Occurrences`] yayınlar.
    /// Bloklamaz (kuyruk doluysa en yeni istek yine gelir: eski aralık önemsizdir).
    pub fn set_range(&self, from: i64, to: i64) {
        if let Some(tx) = &self.tx
            && tx.try_send(Command::Range { from, to }).is_err()
        {
            tracing::debug!("takvim komut kuyruğu dolu/kapalı");
        }
    }
}

impl Drop for CalendarService {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            tx.close();
        }
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("takvim thread'i panikle sonlandı");
        }
    }
}

enum Next {
    Command(Option<Command>),
    Signal(SignalInput),
    Tick,
}

async fn run<F: Fn(CalendarEvent)>(options: CalendarOptions, rx: async_channel::Receiver<Command>, sink: F) {
    let conn = match connect(&options).await {
        Ok(c) => c,
        Err(e) => {
            tracing::info!("takvim: oturum bus'ına bağlanılamadı ({e}); takvim boş");
            sink(CalendarEvent::Unavailable);
            // Kuyruk kapanana kadar bekle ki çağıran komut gönderirken hata görmesin.
            while rx.recv().await.is_ok() {}
            return;
        }
    };
    // Daemon sinyalleri + daemon'un bus'a gelip gitmesi.
    let mut signals = match signal_stream(&conn).await {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("takvim sinyallerine abone olunamadı: {e}; yalnızca istek üzerine güncellenir");
            None
        }
    };

    let mut gate = OwnerGate::new(if signals.is_some() { current_owner(&conn).await } else { None });
    let mut range: Option<(i64, i64)> = None;
    let mut dirty_since: Option<Instant> = None;
    loop {
        let next = {
            let cmd = async { Next::Command(rx.recv().await.ok()) };
            let sig = async {
                match signals.as_mut() {
                    Some(s) => match s.next().await {
                        Some(input) => Next::Signal(input),
                        // Akış bitti (bağlantı koptu): yalnızca komutları dinle.
                        None => std::future::pending().await,
                    },
                    None => std::future::pending().await,
                }
            };
            let tick = async {
                match dirty_since {
                    Some(t) => {
                        async_io::Timer::at(t + COALESCE).await;
                        Next::Tick
                    }
                    None => std::future::pending().await,
                }
            };
            futures_lite::future::or(cmd, futures_lite::future::or(sig, tick)).await
        };
        match next {
            Next::Command(None) => return,
            Next::Command(Some(Command::Range { from, to })) => {
                range = Some((from, to));
                dirty_since = None;
                query(&conn, from, to, &sink).await;
            }
            Next::Signal(input) => {
                // Patlamayı birleştir: ilk kabul edilen sinyalden COALESCE sonra tek sorgu.
                if gate.accept(input) {
                    dirty_since.get_or_insert_with(Instant::now);
                }
            }
            Next::Tick => {
                dirty_since = None;
                if let Some((from, to)) = range {
                    query(&conn, from, to, &sink).await;
                }
            }
        }
    }
}

async fn connect(options: &CalendarOptions) -> zbus::Result<zbus::Connection> {
    let builder = match &options.bus_address {
        Some(a) => zbus::connection::Builder::address(a.as_str())?,
        None => zbus::connection::Builder::session()?,
    };
    builder.method_timeout(Duration::from_secs(3)).build().await
}

/// Bus sürücüsünün adı: `NameOwnerChanged` yalnızca ondan gelirse geçerlidir.
const BUS_DRIVER: &str = "org.freedesktop.DBus";

/// Sinyal akışından gelen, içeriği atılmış girdi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalInput {
    /// `org.nuxut.Calendar1` arayüzünde bir sinyal; gönderenin tekil adı.
    Daemon { sender: Option<String> },
    /// `org.nuxut.Calendar1` adının sahibi değişti (`None`: daemon gitti).
    OwnerChanged { new_owner: Option<String> },
}

/// Taklit koruması: daemon sinyali yalnızca `org.nuxut.Calendar1` adının güncel sahibinden
/// (tekil ad) kabul edilir. Sinyal içeriği hiçbir zaman kullanılmaz; kabul edilen sinyal
/// yalnızca "yeniden sorgula" tetikleyicisidir, veri her zaman adın sahibinden sorguyla gelir.
#[derive(Debug, Default)]
pub struct OwnerGate {
    owner: Option<String>,
}

impl OwnerGate {
    pub fn new(owner: Option<String>) -> Self {
        Self { owner }
    }

    /// Girdiyi uygular; yeniden sorgu gerekiyorsa `true`.
    pub fn accept(&mut self, input: SignalInput) -> bool {
        match input {
            SignalInput::OwnerChanged { new_owner } => {
                // Daemon geldi/gitti/yeniden başladı: her durumda sorgu (gittiyse Unavailable).
                self.owner = new_owner.filter(|o| !o.is_empty());
                true
            }
            SignalInput::Daemon { sender } => sender.is_some() && sender == self.owner,
        }
    }
}

async fn signal_stream(conn: &zbus::Connection) -> zbus::Result<futures_lite::stream::Boxed<SignalInput>> {
    let daemon = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(INTERFACE)?
        .build();
    let owner = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(BUS_DRIVER)?
        .interface(BUS_DRIVER)?
        .member("NameOwnerChanged")?
        .add_arg(BUS_NAME)?
        .build();
    let a = zbus::MessageStream::for_match_rule(daemon, conn, Some(64)).await?.filter_map(|m| {
        let m = m.ok()?;
        Some(SignalInput::Daemon { sender: m.header().sender().map(|s| s.to_string()) })
    });
    let b = zbus::MessageStream::for_match_rule(owner, conn, Some(8)).await?.filter_map(|m| {
        let m = m.ok()?;
        // Yalnızca bus sürücüsünden ve yalnızca bizim adımız için.
        if m.header().sender().map(|s| s.as_str()) != Some(BUS_DRIVER) {
            return None;
        }
        let (name, _old, new): (String, String, String) = m.body().deserialize().ok()?;
        (name == BUS_NAME).then_some(SignalInput::OwnerChanged { new_owner: Some(new) })
    });
    Ok(a.or(b).boxed())
}

/// Adın şu anki sahibi (tekil ad); yoksa `None`. Sinyal akışı kurulduktan sonra çağrılır ki
/// aradaki sahip değişimi kaçmasın.
async fn current_owner(conn: &zbus::Connection) -> Option<String> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = zbus::names::BusName::try_from(BUS_NAME).ok()?;
    dbus.get_name_owner(name).await.ok().map(|o| o.to_string())
}

async fn query<F: Fn(CalendarEvent)>(conn: &zbus::Connection, from: i64, to: i64, sink: &F) {
    let result = async {
        let proxy = CalendarProxy::new(conn).await?;
        proxy.get_occurrences(from, to).await
    }
    .await;
    match result {
        Ok(entries) => sink(CalendarEvent::Occurrences { from, to, entries: sanitize(entries) }),
        Err(e) => {
            tracing::debug!("takvim sorgulanamadı: {e}");
            sink(CalendarEvent::Unavailable);
        }
    }
}

/// Dış girdiyi sınırlar: sayı, metin uzunluğu, renk biçimi; başlangıca göre sıralar.
pub fn sanitize(mut entries: Vec<OccurrenceEntry>) -> Vec<OccurrenceEntry> {
    entries.truncate(MAX_ENTRIES);
    for e in &mut entries {
        clip(&mut e.summary);
        clip(&mut e.location);
        if !is_hex_color(&e.color) {
            e.color.clear();
        }
    }
    entries.sort_by_key(|e| (e.start, e.end));
    entries
}

fn clip(s: &mut String) {
    if let Some((i, _)) = s.char_indices().nth(MAX_TEXT) {
        s.truncate(i);
    }
    if s.chars().any(char::is_control) {
        *s = s.chars().filter(|c| !c.is_control()).collect();
    }
}

/// `#rrggbb` mi? (Takvim rengi daemon'dan gelen veridir, koda gömülü renk değildir.)
pub fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(start: i64, summary: &str, color: &str) -> OccurrenceEntry {
        OccurrenceEntry { start, end: start + 3600, summary: summary.into(), color: color.into(), ..Default::default() }
    }

    #[test]
    fn sanitize_sorts_clips_and_validates_colors() {
        let long = "x".repeat(1000);
        let out = sanitize(vec![
            entry(200, "ikinci", "#00ff00"),
            entry(100, &long, "red"),
            entry(150, "a\u{1b}[2Jb", "#12345"),
        ]);
        assert_eq!(out.iter().map(|e| e.start).collect::<Vec<_>>(), [100, 150, 200]);
        assert_eq!(out[0].summary.chars().count(), MAX_TEXT);
        assert_eq!(out[0].color, "", "geçersiz renk silinir");
        assert_eq!(out[1].summary, "a[2Jb", "kontrol karakteri atılır");
        assert_eq!(out[1].color, "");
        assert_eq!(out[2].color, "#00ff00");
    }

    #[test]
    fn owner_gate_accepts_only_current_owner() {
        let mut g = OwnerGate::new(Some(":1.5".into()));
        assert!(g.accept(SignalInput::Daemon { sender: Some(":1.5".into()) }));
        assert!(!g.accept(SignalInput::Daemon { sender: Some(":1.9".into()) }), "taklitçi");
        assert!(!g.accept(SignalInput::Daemon { sender: None }));
        // Daemon yeniden başladı: yeni sahip, eskisi artık yok sayılır.
        assert!(g.accept(SignalInput::OwnerChanged { new_owner: Some(":1.7".into()) }));
        assert!(!g.accept(SignalInput::Daemon { sender: Some(":1.5".into()) }));
        assert!(g.accept(SignalInput::Daemon { sender: Some(":1.7".into()) }));
        // Daemon gitti: hiçbir sinyal kabul edilmez.
        assert!(g.accept(SignalInput::OwnerChanged { new_owner: Some(String::new()) }));
        assert!(!g.accept(SignalInput::Daemon { sender: Some(":1.7".into()) }));
        let mut none = OwnerGate::default();
        assert!(!none.accept(SignalInput::Daemon { sender: Some(":1.1".into()) }));
    }

    #[test]
    fn debug_is_redacted() {
        let e = CalendarEvent::Occurrences { from: 0, to: 1, entries: vec![entry(0, "gizli toplantı", "")] };
        let s = format!("{e:?}");
        assert!(!s.contains("gizli"), "{s}");
        assert!(s.contains("entries: 1"), "{s}");
    }

    #[test]
    fn caps_entry_count() {
        let many = (0..(MAX_ENTRIES as i64 + 10)).map(|i| entry(i, "e", "")).collect();
        assert_eq!(sanitize(many).len(), MAX_ENTRIES);
    }

    #[test]
    fn missing_daemon_reports_unavailable_and_shuts_down() {
        let (tx, rx) = std::sync::mpsc::channel();
        let svc = CalendarService::spawn(
            CalendarOptions { bus_address: Some("unix:path=/nonexistent/nuxut-test-bus".into()) },
            move |e| {
                let _ = tx.send(e);
            },
        )
        .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), CalendarEvent::Unavailable);
        svc.set_range(0, 86_400);
        let t = Instant::now();
        drop(svc);
        assert!(t.elapsed() < Duration::from_secs(1));
    }
}
