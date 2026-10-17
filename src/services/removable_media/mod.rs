//! Çıkarılabilir medya: udisks2 (`org.freedesktop.UDisks2`, sistem bus'ı) izleyicisi ve
//! "Aç" / "Çıkar" eylemleri. Tasarım ve kararlar: `docs/design/removable-media.md`.
//!
//! **Durum.** udisks'in ObjectManager'ı ([`UDISKS_ROOT`]) `GetManagedObjects` ile okunur;
//! `InterfacesAdded`/`InterfacesRemoved` ve `PropertiesChanged` ile güncellenir. Her değişimde
//! birim listesi ([`model`]) yeniden hesaplanır; değiştiyse [`MediaEvent::Volumes`]. Listeye
//! *sonradan* giren birim [`MediaEvent::Added`] (bildirim), çıkan [`MediaEvent::Removed`] üretir.
//! Her tam okuma (açılış, udisks yeniden başlaması) **taban çizgisidir**: o an takılı olanlar
//! için `Added` yok.
//!
//! **Etkinleştirme.** udisksd D-Bus ile etkinleşir (systemd `udisks2.service`, statik).
//! Çalışmıyorsa çıkarılabilir aygıtlar hiç nesne olarak görünmez; bu yüzden masaüstü örneği
//! ([`MediaOptions::activate`]) `GetManagedObjects` çağrısıyla onu başlatır (salt okunur bir
//! çağrı; dosya yöneticileri de aynısını yapar). Test örneği gerçek sistem bus'ında
//! etkinleştirmez ve eylem çağırmaz (`app/removable_media.rs`: [`MediaOptions::actions`]).
//!
//! **Taklit koruması.** Sinyaller yalnızca `org.freedesktop.UDisks2` adının o anki benzersiz
//! sahibinden ve son tam okumadan yeniyse uygulanır ([`NameOwner`]).
//!
//! **Eylemler** ayrı görevlerde (polkit istemi onlarca saniye sürebilir; döngü beklemez):
//! - Aç: bağlı değilse `Filesystem.Mount({})` (yetki gerekirse polkit ajanımız sorar) →
//!   [`MediaEvent::Opened`] (bağlama noktası; açmayı uygulama yapar).
//! - Çıkar: sürücüdeki bağlı **her** dosya sistemi (birim süzgecine takılanlar dahil)
//!   `Unmount({})` — zaman aşımsız (hata → [`MediaEvent::Failed`]; meşgulse [`MediaError::Busy`]),
//!   kilidi açılmış kripto aygıtlarının arka aygıtı `Encrypted.Lock({})`, sonra sürücü
//!   `CanPowerOff` ise `PowerOff({})`, değilse `Ejectable` ise `Eject({})` → [`MediaEvent::Ejected`].
//!   Güç kesme `DeviceBusy` dönerse `Busy`; kilit ve diğer güç kesme hataları yalnızca loglanır
//!   (birimler ayrıldı, veri güvende). Aynı sürücüye süren çıkarma varken gelen ikinci istek yok
//!   sayılır. Eylemler yöntem zaman aşımı olmayan ayrı bir bağlantıdan gider.

pub mod model;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use futures_lite::future::FutureExt;
use nuxut::i18n::t;
use crate::schema::SettingSchema;
use serde::Deserialize;
use zbus::zvariant::Value;
use zbus::{Connection, Message};

pub use model::{BLOCK_PREFIX, UDISKS_ROOT, Volume, VolumeName, size_text, valid_id};

use nuxut_dbus::{
    Backoff, BusKind, NameOwner, ObjectStore, ServiceContext, ServiceThread, apply_object_signal, clean_str, close,
    connect, forward, get_managed_objects, get_name_owner, owner_changes, parse_owner_change, signals_from,
    tick_connection,
};
use model::{DRIVE_IFACE, ENCRYPTED_IFACE, FILESYSTEM_IFACE};

pub const UDISKS: &str = "org.freedesktop.UDisks2";
/// `GetManagedObjects`'te kabul edilen en fazla nesne.
const MAX_OBJECTS: usize = 2048;
const INPUT_CAPACITY: usize = 512;

// ---------------------------------------------------------------------------------------
// Config

/// `[removable_media]`. Kendiliğinden bağlama (automount) v1'de yok (belgede gerekçe).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RemovableMediaConfig {
    /// İzleyici çalışsın mı (`false`: udisks'e hiç bağlanılmaz).
    pub enabled: bool,
    /// Aygıt takılınca "Aç / Çıkar" bildirimi.
    pub notify: bool,
}

impl Default for RemovableMediaConfig {
    fn default() -> Self {
        Self { enabled: true, notify: true }
    }
}

impl RemovableMediaConfig {
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(
            SettingSchema::bool("removable_media.enabled", t!("shell-config-removable-media-enabled-label"), d.enabled)
                .describe(t!("shell-config-removable-media-enabled-describe")),
        );
        out.push(SettingSchema::bool("removable_media.notify", t!("shell-config-removable-media-notify-label"), d.notify));
    }
}

// ---------------------------------------------------------------------------------------
// Komutlar (`nuxut-shell msg removable …`)

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaCommand {
    /// Tek satır JSON: birimler.
    List,
    /// Bağla (gerekirse) ve varsayılan uygulamayla aç.
    Open(String),
    /// Ayır ve güvenle çıkarılabilir yap.
    Eject(String),
}

pub const USAGE: &str = "kullanım: removable list | removable open <id> | removable eject <id>"; // i18n: tanı msg yanıtı/bildirim ayrıntısı

/// `removable` sonrası argümanlar. Kimlik biçimi [`valid_id`]; bilinip bilinmediğine
/// yürütücü bakar.
pub fn parse_command(rest: &[&str]) -> Result<MediaCommand, String> {
    match rest {
        ["list"] => Ok(MediaCommand::List),
        [sub @ ("open" | "eject"), id] => {
            if !valid_id(id) {
                return Err(format!("geçersiz birim kimliği \"{}\"", clean_str(id, 32))); // i18n: tanı msg yanıtı/bildirim ayrıntısı
            }
            let id = (*id).to_owned();
            Ok(if *sub == "open" { MediaCommand::Open(id) } else { MediaCommand::Eject(id) })
        }
        _ => Err(USAGE.into()),
    }
}

/// `msg removable list` biçim sürümü.
pub const LIST_VERSION: u32 = 1;

/// `msg removable list` (tek satır JSON v1).
pub fn list_json(available: bool, volumes: &[Volume]) -> String {
    let items: Vec<serde_json::Value> = volumes
        .iter()
        .map(|v| {
            serde_json::json!({
                "id": v.id,
                "name": v.display_name(),
                "size": v.size,
                "mount_points": v.mount_points.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({ "v": LIST_VERSION, "available": available, "volumes": items }).to_string()
}

// ---------------------------------------------------------------------------------------
// Olaylar

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Open,
    Eject,
}

/// Eylem hatası.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// Birimdeki dosyalar açık (`org.freedesktop.UDisks2.Error.DeviceBusy`).
    Busy,
    /// polkit izin vermedi.
    NotAuthorized,
    /// Kullanıcı polkit istemini kapattı: sessiz.
    Cancelled,
    /// udisks çalışmıyor / bağlantı yok.
    Unavailable,
    /// Bu kimlikte birim yok (sökülmüş olabilir).
    Unknown,
    /// Bu örnek eylem çağırmaz (gerçek sistem bus'ındaki test örneği).
    Disabled,
    Timeout,
    Other(String),
}

impl MediaError {
    /// D-Bus hata adından (ve zaman aşımından) sınıf.
    pub fn classify(name: Option<&str>, timed_out: bool, detail: &str) -> Self {
        if timed_out {
            return Self::Timeout;
        }
        match name.unwrap_or_default() {
            "org.freedesktop.UDisks2.Error.DeviceBusy" => Self::Busy,
            "org.freedesktop.UDisks2.Error.NotAuthorized" | "org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain" => {
                Self::NotAuthorized
            }
            "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed" | "org.freedesktop.UDisks2.Error.Cancelled" => {
                Self::Cancelled
            }
            "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner" => {
                Self::Unavailable
            }
            _ => Self::Other(clean_str(detail, 200)),
        }
    }

    fn from_zbus(e: &zbus::Error) -> Self {
        match e {
            zbus::Error::MethodError(name, desc, _) => {
                Self::classify(Some(name.as_str()), false, desc.as_deref().unwrap_or(name.as_str()))
            }
            zbus::Error::InputOutput(err) if err.kind() == io::ErrorKind::TimedOut => Self::classify(None, true, ""),
            e => Self::classify(None, false, &e.to_string()),
        }
    }

    /// Bildirim gösterilmez (kullanıcı kendisi vazgeçti).
    pub fn silent(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Bildirim gövdesi.
    pub fn message(&self) -> String {
        match self {
            Self::Busy => t!("shell-media-error-busy"),
            Self::NotAuthorized => t!("shell-media-error-not-authorized"),
            Self::Cancelled => t!("shell-media-error-cancelled"),
            Self::Unavailable => t!("shell-media-error-unavailable"),
            Self::Unknown => t!("shell-media-error-gone"),
            Self::Disabled => t!("shell-media-error-disabled"),
            Self::Timeout => t!("shell-media-error-timeout"),
            Self::Other(detail) => t!("shell-media-error-other", detail = detail.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaEvent {
    /// Birim listesi değişti (tam liste; udisks yoksa boş ve `available = false`).
    Volumes { available: bool, volumes: Vec<Volume> },
    /// Açılıştan (son tam okumadan) sonra takılan birim: bildirim.
    Added(Volume),
    /// Birim gitti (söküldü, kilitlendi, gizlendi).
    Removed { id: String },
    /// Aç: birim bağlı; bu dizin açılmalı.
    Opened { id: String, mount_point: PathBuf },
    /// Çıkar: sürücü güvenle çıkarılabilir. `name`: görünen ad (birim artık listede olmayabilir).
    Ejected { id: String, name: String },
    Failed { id: String, name: String, action: MediaAction, error: MediaError },
}

#[derive(Debug, Clone)]
pub struct MediaOptions {
    /// D-Bus adresi; `None`: sistem bus'ı (`DBUS_SYSTEM_BUS_ADDRESS` geçerli).
    pub bus_address: Option<String>,
    pub call_timeout: Duration,
    /// `Mount`/`Lock`/`PowerOff`/`Eject` zaman aşımı (polkit istemi dahil). `Unmount` sınırsız.
    pub action_timeout: Duration,
    pub max_backoff: Duration,
    /// udisks çalışmıyorsa `GetManagedObjects` ile etkinleştir.
    pub activate: bool,
    /// Bağla/ayır/çıkar çağrılabilir mi (`false`: yalnızca izleme; eylemler [`MediaError::Disabled`]).
    pub actions: bool,
}

impl Default for MediaOptions {
    fn default() -> Self {
        Self {
            bus_address: None,
            call_timeout: Duration::from_secs(5),
            action_timeout: Duration::from_secs(120),
            max_backoff: Duration::from_secs(60),
            activate: true,
            actions: true,
        }
    }
}

#[derive(Debug)]
enum Command {
    Open(String),
    Eject(String),
}

/// Çalışan izleyici; düşürülünce thread durur ve join edilir.
#[derive(Debug)]
pub struct MediaService {
    thread: ServiceThread<Command>,
}

impl MediaService {
    /// `sink` servis thread'inden çağrılır; hızlı dönmeli.
    pub fn spawn<F>(options: MediaOptions, sink: F) -> io::Result<Self>
    where
        F: Fn(MediaEvent) + Send + 'static,
    {
        let thread = ServiceThread::spawn("nuxut-media", 16, move |ctx| run(options, sink, ctx))?;
        Ok(Self { thread })
    }

    pub fn open(&self, id: &str) -> bool {
        self.thread.send(Command::Open(id.to_owned()))
    }

    pub fn eject(&self, id: &str) -> bool {
        self.thread.send(Command::Eject(id.to_owned()))
    }
}

// ---------------------------------------------------------------------------------------
// Döngü

#[derive(Debug)]
enum Input {
    Owner,
    Ignore,
    Signal(Message),
    Done(MediaEvent),
    /// Çıkarma bitti: sürücü sürmekte olan eylemlerden düşer.
    EjectDone { drive: String, event: MediaEvent },
}

enum Wake {
    Stop,
    Lost,
    Input(Input),
    Command(Command),
}

async fn run<F: Fn(MediaEvent)>(options: MediaOptions, sink: F, ctx: ServiceContext<Command>) {
    let mut backoff = Backoff::new(options.max_backoff);
    loop {
        match connect(BusKind::System, options.bus_address.as_deref(), options.call_timeout).await {
            Ok(conn) => {
                backoff.reset();
                let ticker = tick_connection(&ctx.ex, &conn);
                // Eylemler yöntem zaman aşımı olmayan ayrı bağlantıda (bkz. `Runtime::actions`).
                match action_connection(options.bus_address.as_deref()).await {
                    Ok(actions) => {
                        let actions_ticker = tick_connection(&ctx.ex, &actions);
                        let mut rt = Runtime {
                            sink: &sink,
                            options: &options,
                            conn: conn.clone(),
                            actions: actions.clone(),
                            ejecting: HashSet::new(),
                            udisks: NameOwner::new(),
                            objects: model::new_objects(),
                            volumes: Vec::new(),
                            emitted: None,
                        };
                        let stopped = rt.session(&ctx).await;
                        drop(rt);
                        drop(actions_ticker);
                        drop(ticker);
                        close(actions).await;
                        close(conn).await;
                        if stopped {
                            return;
                        }
                        tracing::warn!("çıkarılabilir medya: sistem bus bağlantısı koptu, yeniden bağlanılacak");
                    }
                    Err(e) => {
                        tracing::warn!("çıkarılabilir medya: eylem bağlantısı kurulamadı: {e}");
                        drop(ticker);
                        close(conn).await;
                    }
                }
            }
            Err(e) => tracing::warn!("çıkarılabilir medya: sistem bus'ına bağlanılamadı: {e}"),
        }
        sink(MediaEvent::Volumes { available: false, volumes: Vec::new() });
        if !ctx.sleep(backoff.next_delay()).await {
            return;
        }
    }
}

struct Runtime<'a, F> {
    sink: &'a F,
    options: &'a MediaOptions,
    conn: Connection,
    /// Yöntem zaman aşımı olmayan bağlantı: `Mount` (polkit istemi) ve `Unmount` (yavaş USB'de
    /// önbellek boşaltma) [`MediaOptions::call_timeout`]'u kat kat aşabilir; süreyi her çağrı
    /// kendisi seçer ([`call`]).
    actions: Connection,
    /// Çıkarılmakta olan sürücüler: aynı sürücüye ikinci istek (düğme + `msg`) yok sayılır.
    ejecting: HashSet<String>,
    udisks: NameOwner,
    objects: ObjectStore,
    /// Son hesaplanan birimler.
    volumes: Vec<Volume>,
    /// Son yayınlanan liste (aynısı tekrar yayınlanmaz).
    emitted: Option<(bool, Vec<Volume>)>,
}

impl<F: Fn(MediaEvent)> Runtime<'_, F> {
    /// Bağlantı boyunca; durdurulduysa `true`, bağlantı koptuysa `false`.
    async fn session(&mut self, ctx: &ServiceContext<Command>) -> bool {
        let (tx, rx) = async_channel::bounded::<Input>(INPUT_CAPACITY);
        let streams = async {
            Ok::<_, zbus::Error>((
                owner_changes(&self.conn, UDISKS).await?,
                signals_from(&self.conn, UDISKS, Some(UDISKS_ROOT), None).await?,
            ))
        };
        let (owners, signals) = match streams.await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("çıkarılabilir medya: sinyal aboneliği kurulamadı: {e}");
                return false;
            }
        };
        let _tasks = [
            forward(&ctx.ex, owners, tx.clone(), |m| {
                if parse_owner_change(&m, UDISKS).is_some() { Input::Owner } else { Input::Ignore }
            }),
            forward(&ctx.ex, signals, tx.clone(), Input::Signal),
        ];
        self.reload().await;
        loop {
            let wake = async { rx.recv().await.map_or(Wake::Lost, Wake::Input) }
                .or(async { ctx.commands.recv().await.map_or(Wake::Stop, Wake::Command) })
                .or(async { ctx.stop.recv().await.map_or(Wake::Stop, |()| Wake::Stop) })
                .await;
            match wake {
                Wake::Stop => return true,
                Wake::Lost => return false,
                Wake::Command(c) => self.command(ctx, &tx, c),
                Wake::Input(Input::Ignore) => {}
                Wake::Input(Input::Owner) => {
                    let owner = get_name_owner(&self.conn, UDISKS).await;
                    if owner.is_none() || owner != self.udisks.owner {
                        self.reload().await;
                    }
                }
                Wake::Input(Input::Signal(msg)) => {
                    if self.udisks.accepts(&msg) && apply_object_signal(&mut self.objects, &msg) {
                        self.refresh(false);
                    }
                }
                Wake::Input(Input::Done(e)) => (self.sink)(e),
                Wake::Input(Input::EjectDone { drive, event }) => {
                    self.ejecting.remove(&drive);
                    (self.sink)(event);
                }
            }
        }
    }

    /// Tam okuma (taban çizgisi): o an takılı olanlar için `Added` yok.
    async fn reload(&mut self) {
        self.objects.clear();
        let running = get_name_owner(&self.conn, UDISKS).await;
        self.udisks.set_owner(None);
        if running.is_some() || self.options.activate {
            // udisks çalışmıyorsa bu çağrı onu etkinleştirir (bkz. modül belgesi).
            match get_managed_objects(&self.conn, UDISKS, UDISKS_ROOT, MAX_OBJECTS).await {
                Ok((objects, reply)) => {
                    let sender = reply.header().sender().map(|s| s.as_str().to_owned());
                    self.udisks.set_owner(sender.filter(|s| NameOwner::is_unique(s)));
                    self.udisks.loaded(&reply);
                    for (path, ifaces) in objects {
                        self.objects.add(path.as_str(), ifaces);
                    }
                }
                Err(e) => tracing::warn!("çıkarılabilir medya: udisks okunamadı: {e}"),
            }
        }
        self.refresh(true);
    }

    /// Birimleri yeniden hesaplar; farkları yayınlar.
    fn refresh(&mut self, baseline: bool) {
        let volumes = model::volumes(&self.objects);
        let available = self.udisks.owner.is_some();
        let before: BTreeSet<&str> = self.volumes.iter().map(|v| v.id.as_str()).collect();
        let after: BTreeSet<&str> = volumes.iter().map(|v| v.id.as_str()).collect();
        let added: Vec<Volume> =
            if baseline { Vec::new() } else { volumes.iter().filter(|v| !before.contains(v.id.as_str())).cloned().collect() };
        let removed: Vec<String> = before.difference(&after).map(|s| (*s).to_owned()).collect();
        self.volumes = volumes;
        let snapshot = (available, self.volumes.clone());
        if self.emitted.as_ref() != Some(&snapshot) {
            (self.sink)(MediaEvent::Volumes { available, volumes: self.volumes.clone() });
            self.emitted = Some(snapshot);
        }
        for id in removed {
            (self.sink)(MediaEvent::Removed { id });
        }
        for v in added {
            tracing::info!(id = %v.id, "çıkarılabilir birim takıldı");
            (self.sink)(MediaEvent::Added(v));
        }
    }

    fn command(&mut self, ctx: &ServiceContext<Command>, tx: &Sender<Input>, command: Command) {
        let (id, action) = match &command {
            Command::Open(id) => (id.clone(), MediaAction::Open),
            Command::Eject(id) => (id.clone(), MediaAction::Eject),
        };
        let volume = self.volumes.iter().find(|v| v.id == id).cloned();
        let name = volume.as_ref().map(Volume::display_name).unwrap_or_else(|| id.clone());
        let fail = |error| MediaEvent::Failed { id: id.clone(), name: name.clone(), action, error };
        let error = if !self.options.actions {
            Some(MediaError::Disabled)
        } else if self.udisks.owner.is_none() {
            Some(MediaError::Unavailable)
        } else if volume.is_none() {
            Some(MediaError::Unknown)
        } else {
            None
        };
        if let Some(e) = error {
            (self.sink)(fail(e));
            return;
        }
        let Some(volume) = volume else { return };
        match action {
            MediaAction::Open => {
                if let Some(mount_point) = volume.mount_points.first().cloned() {
                    (self.sink)(MediaEvent::Opened { id: id.clone(), mount_point });
                    return;
                }
                let conn = self.actions.clone();
                let timeout = self.options.action_timeout;
                spawn_action(ctx, tx, None, fail(MediaError::Unknown), async move {
                    let path = mount(&conn, &volume.object_path(), timeout).await?;
                    Ok(MediaEvent::Opened { id: volume.id, mount_point: path })
                });
            }
            MediaAction::Eject => {
                // Düğme + `msg` ya da çift tık: aynı sürücüde süren çıkarma varsa ikincisi yok
                // sayılır (paralel görev ikinci kez ayırmaya çalışıp sahte hata üretirdi).
                if !self.ejecting.insert(volume.drive.clone()) {
                    tracing::info!(%id, "çıkarılabilir medya: sürücü zaten çıkarılıyor, istek yok sayıldı");
                    return;
                }
                // Sürücüdeki TÜM dosya sistemleri (süzgece takılanlar dahil): biri bağlı kalırsa
                // gücü kesilemez.
                let filesystems = model::drive_filesystems(&self.objects, &volume.drive);
                let conn = self.actions.clone();
                let timeout = self.options.action_timeout;
                let drive = volume.drive.clone();
                spawn_action(ctx, tx, Some(drive), fail(MediaError::Unknown), async move {
                    eject(&conn, &volume, &filesystems, timeout).await?;
                    Ok(MediaEvent::Ejected { id: volume.id.clone(), name: volume.display_name() })
                });
            }
        }
    }
}

/// Eylemi ayrı görevde çalıştırır; sonuç (ya da `failed` şablonuna hata) döngüye döner.
/// `eject_drive`: çıkarmaysa sürücü (bitince sürmekte olanlardan düşer).
fn spawn_action(
    ctx: &ServiceContext<Command>,
    tx: &Sender<Input>,
    eject_drive: Option<String>,
    failed: MediaEvent,
    action: impl std::future::Future<Output = Result<MediaEvent, MediaError>> + 'static,
) {
    let tx = tx.clone();
    ctx.ex
        .spawn(async move {
            let event = match action.await {
                Ok(e) => e,
                Err(error) => match failed {
                    MediaEvent::Failed { id, name, action, .. } => {
                        tracing::warn!(%id, ?action, ?error, "çıkarılabilir medya eylemi başarısız");
                        MediaEvent::Failed { id, name, action, error }
                    }
                    other => other,
                },
            };
            let input = match eject_drive {
                Some(drive) => Input::EjectDone { drive, event },
                None => Input::Done(event),
            };
            let _ = tx.send(input).await;
        })
        .detach();
}

fn no_options() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

/// Yöntem zaman aşımı olmayan bağlantı (eylemler; süreyi [`call`] seçer).
async fn action_connection(address: Option<&str>) -> zbus::Result<Connection> {
    let builder = match address {
        Some(a) => zbus::connection::Builder::address(a)?,
        None => zbus::connection::Builder::system()?,
    };
    builder.internal_executor(false).build().await
}

/// udisks yöntem çağrısı; `timeout` `None` ise süre sınırı yok (yalnızca `Unmount`).
async fn call<B>(
    conn: &Connection,
    path: &str,
    iface: &str,
    method: &str,
    body: &B,
    timeout: Option<Duration>,
) -> Result<Message, MediaError>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let call = conn.call_method(Some(UDISKS), path, Some(iface), method, body);
    let result = match timeout {
        Some(timeout) => {
            call.or(async {
                async_io::Timer::after(timeout).await;
                Err(zbus::Error::InputOutput(Arc::new(io::Error::from(io::ErrorKind::TimedOut))))
            })
            .await
        }
        None => call.await,
    };
    result.map_err(|e| MediaError::from_zbus(&e))
}

/// `Filesystem.Mount({})` → bağlama noktası (mutlak yol olmalı).
async fn mount(conn: &Connection, path: &str, timeout: Duration) -> Result<PathBuf, MediaError> {
    let reply = call(conn, path, FILESYSTEM_IFACE, "Mount", &(no_options(),), Some(timeout)).await?;
    let mount_point: String = reply.body().deserialize().map_err(|e| MediaError::Other(clean_str(&e.to_string(), 200)))?;
    if !mount_point.starts_with('/') || mount_point.len() > 4096 || mount_point.contains('\0') {
        return Err(MediaError::Other(clean_str(&mount_point, 200)));
    }
    Ok(PathBuf::from(mount_point))
}

/// Sürücüdeki bağlı her dosya sistemini ayırır (zaman aşımsız: yavaş USB'de önbellek boşaltma
/// dakikalar sürebilir; yarıda bırakmak veriyi riske atar), açık metin aygıtlarının arka aygıtını
/// kilitler, sonra gücü keser / çıkarır. Sürücü meşgulse (`DeviceBusy`) `Busy`; diğer güç kesme
/// hataları yalnızca loglanır (birimler ayrıldı, veri güvende).
async fn eject(
    conn: &Connection,
    volume: &Volume,
    filesystems: &[model::DriveFilesystem],
    timeout: Duration,
) -> Result<(), MediaError> {
    for fs in filesystems.iter().filter(|f| f.mounted) {
        call(conn, &fs.path, FILESYSTEM_IFACE, "Unmount", &(no_options(),), None).await?;
    }
    for backing in filesystems.iter().filter_map(|f| f.crypto_backing.as_deref()) {
        if let Err(e) = call(conn, backing, ENCRYPTED_IFACE, "Lock", &(no_options(),), Some(timeout)).await {
            tracing::warn!(?e, "çıkarılabilir medya: kripto aygıtı kilitlenemedi");
        }
    }
    let method = if volume.can_power_off {
        Some("PowerOff")
    } else if volume.ejectable {
        Some("Eject")
    } else {
        None
    };
    if let Some(method) = method {
        match call(conn, &volume.drive, DRIVE_IFACE, method, &(no_options(),), Some(timeout)).await {
            Ok(_) => {}
            // Sürücü hâlâ kullanımda (başka bir süreç aygıtı tutuyor): "güvenle çıkarılabilir"
            // denemez.
            Err(MediaError::Busy) => return Err(MediaError::Busy),
            Err(e) => tracing::warn!(method, ?e, "çıkarılabilir medya: sürücü kapatılamadı (birimler ayrıldı)"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_parsing() {
        assert_eq!(parse_command(&["list"]), Ok(MediaCommand::List));
        assert_eq!(parse_command(&["open", "sdb1"]), Ok(MediaCommand::Open("sdb1".into())));
        assert_eq!(parse_command(&["eject", "mmcblk0p1"]), Ok(MediaCommand::Eject("mmcblk0p1".into())));
        for bad in [&["open"][..], &["eject", "../sda"], &["open", "a", "b"], &["mount", "sdb1"], &[], &["list", "x"]] {
            assert!(parse_command(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn errors_are_classified() {
        let c = |n: &str| MediaError::classify(Some(n), false, "ayrıntı");
        assert_eq!(c("org.freedesktop.UDisks2.Error.DeviceBusy"), MediaError::Busy);
        assert_eq!(c("org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain"), MediaError::NotAuthorized);
        assert_eq!(c("org.freedesktop.UDisks2.Error.NotAuthorizedDismissed"), MediaError::Cancelled);
        assert_eq!(c("org.freedesktop.DBus.Error.ServiceUnknown"), MediaError::Unavailable);
        assert_eq!(c("org.freedesktop.UDisks2.Error.Failed"), MediaError::Other("ayrıntı".into()));
        assert_eq!(MediaError::classify(None, true, ""), MediaError::Timeout);
        assert!(MediaError::Cancelled.silent() && !MediaError::Busy.silent());
    }

    #[test]
    fn error_messages_are_localized() {
        crate::i18n::use_turkish_for_tests();
        assert!(MediaError::Busy.message().contains("açık"), "{}", MediaError::Busy.message());
        assert!(MediaError::Other("x".into()).message().contains('x'));
    }

    #[test]
    fn list_json_shape() {
        let v = Volume {
            id: "sdb1".into(),
            name: VolumeName::Label("YEDEK".into()),
            mount_points: vec!["/run/media/u/YEDEK".into()],
            drive: "/d".into(),
            ejectable: true,
            can_power_off: true,
            crypto_backing: None,
            size: 1,
        };
        let j: serde_json::Value = serde_json::from_str(&list_json(true, &[v])).unwrap();
        assert_eq!(j["v"], 1);
        assert_eq!(j["volumes"][0]["id"], "sdb1");
        assert_eq!(j["volumes"][0]["name"], "YEDEK");
        assert_eq!(j["volumes"][0]["mount_points"][0], "/run/media/u/YEDEK");
    }
}
