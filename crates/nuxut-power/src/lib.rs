//! Güç: UPower (pil yüzdesi, şarj durumu, kalan süre, AC, pil şarj sınırı) ve
//! power-profiles-daemon (profil; yoksa "kullanılamıyor"), düşük pil olayları.
//!
//! Sistem bus'ında; polling yok. UPower'ın `DisplayDevice` nesnesi (tüm pillerin birleşimi)
//! ve yönetici nesnesi bir kez `GetAll` ile okunur, sonra `PropertiesChanged` ile güncellenir.
//! Şarj sınırı birleşik nesnede değil gerçek pil cihazlarındadır: `EnumerateDevices` +
//! `DeviceAdded`/`DeviceRemoved` ile sistem pilleri (`Type` = pil, `PowerSupply`) izlenir.
//! UPower ya da power-profiles-daemon bus'tan düşüp geri gelirse (`NameOwnerChanged`) durum
//! temizlenir ve yeniden okunur. Sinyaller yalnızca adın şu anki benzersiz sahibinden kabul
//! edilir ([`nuxut_dbus`]). Aynı durum iki kez yayınlanmaz.
//!
//! Kabuk ve Ayarlar paylaşır (`docs/design/power.md`, veri yolu B). UI'sız: kullanıcı metni
//! yok, hatalar tür olarak ([`PowerError`]); metni çağıran çevirir. Yazmalar (profil, şarj
//! sınırı) istek kimliği döndürür; sonuç kimlikle gelir ve yeni durum sonuçtan **önce**
//! [`PowerEvent::Changed`] ile yayınlanır (iyimser olmayan arayüz için).

mod model;

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_lite::future::FutureExt;
use zbus::zvariant::{OwnedObjectPath, Value};
use zbus::{Connection, Message};

pub use model::{
    Battery, ChargeLimit, ChargeState, PowerProfile, PowerState, Profiles, WarningLevel, warning_transition,
};

pub use nuxut_dbus::{BusKind, PropMap};

use nuxut_dbus::{
    Backoff, MAX_PATH, NameOwner, PROPERTIES, ServiceContext, ServiceThread, close, connect, forward,
    get_all, get_name_owner, owner_changes, parse_owner_change, parse_properties_changed, set_property,
    signals_from, tick_connection,
};

const UPOWER: &str = "org.freedesktop.UPower";
const UPOWER_PATH: &str = "/org/freedesktop/UPower";
const UPOWER_IFACE: &str = "org.freedesktop.UPower";
const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const DEVICE_IFACE: &str = "org.freedesktop.UPower.Device";
/// UPower cihaz nesnelerinin yol öneki.
const DEVICES_PREFIX: &str = "/org/freedesktop/UPower/devices/";
/// `EnumerateDevices`'tan en fazla bu kadar cihaz okunur (fare, klavye, AC dahil).
const MAX_DEVICES: usize = 32;
const PPD: &str = "org.freedesktop.UPower.PowerProfiles";
const PPD_PATH: &str = "/org/freedesktop/UPower/PowerProfiles";
const PPD_IFACE: &str = "org.freedesktop.UPower.PowerProfiles";

/// Bir `PropertiesChanged`'de kabul edilen en fazla özellik.
const MAX_CHANGED: usize = 64;
const INPUT_CAPACITY: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub enum PowerEvent {
    /// Durum değişti (tam anlık görüntü).
    Changed(PowerState),
    /// Pilden çalışırken uyarı seviyesi yükseldi (seviye başına bir kez).
    BatteryWarning(WarningLevel),
    /// [`PowerService::set_profile`] isteğinin sonucu (`id` o çağrının döndürdüğü). Başarıda
    /// yeni profil bu olaydan önce `Changed` ile yayınlanmış olur.
    ProfileDone { id: u64, result: Result<(), PowerError> },
    /// [`PowerService::set_charge_limit`] isteğinin sonucu (`id` o çağrının döndürdüğü).
    /// Başarıda yeni durum bu olaydan önce `Changed` ile yayınlanmış olur.
    ChargeLimitDone { id: u64, result: Result<(), PowerError> },
}

/// Yazma (profil ya da şarj sınırı) yapılamadı. İçerik yalnızca D-Bus hata adıdır (servisin
/// iletisi taşınmaz); log ve `msg` yanıtı için güvenli.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerError {
    /// Hizmet çalışmıyor: sistem bus'ı ya da `upowerd` (şarj sınırı) / power-profiles-daemon
    /// (profil) yok.
    Unavailable,
    /// Şarj sınırı: destekleyen pil yok ya da UPower desteklemediğini söyledi (`NotSupported`).
    /// Profil: daemon bu profili sunmuyor.
    Unsupported,
    /// polkit reddetti ya da kimlik doğrulama iptal edildi
    /// (`org.freedesktop.UPower.enable-charging-limit`; varsayılan: etkin yerel oturumda izinli,
    /// etkin olmayan oturumda hayır). UPower denetimi etkileşimsiz yapar: parola sorulmaz.
    Denied,
    /// Başka hata (UPower `GeneralError`: sysfs yazılamadı vb., zaman aşımı); D-Bus hata adı.
    Failed(String),
}

impl PowerError {
    /// Makinece okunur kod (`msg` yanıtında `error: <kod>: …`).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Unsupported => "unsupported",
            Self::Denied => "denied",
            Self::Failed(_) => "failed",
        }
    }
}

impl std::fmt::Display for PowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed(name) => write!(f, "failed ({name})"),
            other => f.write_str(other.code()),
        }
    }
}

/// UPower `EnableChargeThreshold` hatasını türe çevirir. upowerd (1.91.5) polkit reddini
/// `GeneralError` + "Operation is not allowed." ile döndürür; aynı ad sysfs yazma hatalarında
/// da kullanıldığı için ileti metnine bakılır.
fn charge_error(e: &zbus::Error) -> PowerError {
    use zbus::fdo::Error as Fdo;
    match e {
        zbus::Error::MethodError(name, detail, _) => match name.as_str() {
            "org.freedesktop.UPower.NotSupported" => PowerError::Unsupported,
            "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
            | "org.freedesktop.PolicyKit1.Error.NotAuthorized"
            | "org.freedesktop.PolicyKit1.Error.Cancelled" => PowerError::Denied,
            "org.freedesktop.UPower.GeneralError"
                if detail.as_deref().is_some_and(|d| d.to_ascii_lowercase().contains("not allowed")) =>
            {
                PowerError::Denied
            }
            "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner" => {
                PowerError::Unavailable
            }
            n => PowerError::Failed(nuxut_dbus::clean_str(n, 128)),
        },
        zbus::Error::FDO(f) => match f.as_ref() {
            Fdo::AccessDenied(_) | Fdo::InteractiveAuthorizationRequired(_) => PowerError::Denied,
            Fdo::ServiceUnknown(_) | Fdo::NameHasNoOwner(_) => PowerError::Unavailable,
            Fdo::NotSupported(_) => PowerError::Unsupported,
            _ => PowerError::Failed("fdo".into()),
        },
        zbus::Error::InputOutput(_) => PowerError::Failed("io".into()),
        _ => PowerError::Failed("dbus".into()),
    }
}

/// power-profiles-daemon `ActiveProfile` yazma hatasını türe çevirir (ppd 0.30: polkit reddi
/// `AccessDenied`, bilinmeyen profil `InvalidArgs`).
fn profile_error(e: &zbus::Error) -> PowerError {
    use zbus::fdo::Error as Fdo;
    match e {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
            | "org.freedesktop.PolicyKit1.Error.NotAuthorized"
            | "org.freedesktop.PolicyKit1.Error.Cancelled" => PowerError::Denied,
            "org.freedesktop.DBus.Error.InvalidArgs" => PowerError::Unsupported,
            "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner" => {
                PowerError::Unavailable
            }
            n => PowerError::Failed(nuxut_dbus::clean_str(n, 128)),
        },
        zbus::Error::FDO(f) => match f.as_ref() {
            Fdo::AccessDenied(_) | Fdo::InteractiveAuthorizationRequired(_) => PowerError::Denied,
            Fdo::InvalidArgs(_) => PowerError::Unsupported,
            Fdo::ServiceUnknown(_) | Fdo::NameHasNoOwner(_) => PowerError::Unavailable,
            _ => PowerError::Failed("fdo".into()),
        },
        zbus::Error::InputOutput(_) => PowerError::Failed("io".into()),
        _ => PowerError::Failed("dbus".into()),
    }
}

#[derive(Debug, Clone)]
pub struct PowerOptions {
    /// Hangi bus (varsayılan sistem). Sahte UPower/ppd ile canlı testte `bus_from_env()` ile
    /// seçilir (`test-support` özelliği).
    pub bus: BusKind,
    /// D-Bus adresi (testler); verilirse [`PowerOptions::bus`] yerine bu adrese bağlanılır.
    pub bus_address: Option<String>,
    pub call_timeout: Duration,
    /// Bus bağlantısı koparsa yeniden deneme beklemesinin üst sınırı.
    pub max_backoff: Duration,
}

impl Default for PowerOptions {
    fn default() -> Self {
        Self {
            bus: BusKind::System,
            bus_address: None,
            call_timeout: Duration::from_secs(3),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// Test bus seçimi ortam değişkeni ([`bus_from_env`]).
pub const BUS_ENV: &str = "NUXUT_POWER_BUS";

/// `NUXUT_POWER_BUS=session` ise oturum bus'ı, değilse sistem bus'ı. Yalnız `test-support`
/// özelliğiyle derlenir (üründe yok); çağıran açıkça kullanır.
#[cfg(feature = "test-support")]
pub fn bus_from_env() -> BusKind {
    BusKind::from_env(BUS_ENV)
}

#[derive(Debug)]
enum Command {
    SetProfile { id: u64, profile: PowerProfile },
    SetChargeLimit { id: u64, on: bool },
}

/// Çalışan güç servisi. Düşürülünce temiz kapanır.
#[derive(Debug)]
pub struct PowerService {
    thread: ServiceThread<Command>,
    next_id: AtomicU64,
}

impl PowerService {
    /// `sink` servis thread'inden çağrılır; hızlı dönmeli.
    pub fn spawn<F>(options: PowerOptions, sink: F) -> io::Result<Self>
    where
        F: Fn(PowerEvent) + Send + 'static,
    {
        let thread = ServiceThread::spawn("nuxut-power", 8, move |ctx| run(options, sink, ctx))?;
        Ok(Self { thread, next_id: AtomicU64::new(1) })
    }

    /// Profili değiştirir. Sonuç [`PowerEvent::ProfileDone`] ile, dönen kimlikle gelir.
    /// `None`: servis meşgul ya da durmuş (olay gelmez).
    pub fn set_profile(&self, profile: PowerProfile) -> Option<u64> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.thread.send(Command::SetProfile { id, profile }).then_some(id)
    }

    /// Pil şarj sınırını açar/kapatır (destekleyen tüm sistem pillerinde). Sonuç
    /// [`PowerEvent::ChargeLimitDone`] ile, dönen kimlikle gelir. `None`: servis meşgul ya da
    /// durmuş (olay gelmez).
    pub fn set_charge_limit(&self, on: bool) -> Option<u64> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.thread.send(Command::SetChargeLimit { id, on }).then_some(id)
    }
}

#[derive(Debug)]
enum Input {
    /// Doğrulanmış `NameOwnerChanged` (sahip yine `GetNameOwner` ile sorulur).
    UpOwner,
    PpdOwner,
    /// Doğrulamadan geçmeyen mesaj.
    Ignore,
    UpSignal(Message),
    PpdSignal(Message),
    /// `ActiveProfile` yazımı bitti (başarıda profiller yeniden okunur).
    ProfileResult { id: u64, result: Result<(), PowerError> },
    /// `EnableChargeThreshold` çağrıları bitti; `paths` çağrılan piller (başarıda yeniden okunur).
    ChargeResult { id: u64, paths: Vec<String>, result: Result<(), PowerError> },
}

enum End {
    Stop,
    Lost,
}

async fn run<F: Fn(PowerEvent)>(options: PowerOptions, sink: F, ctx: ServiceContext<Command>) {
    let mut backoff = Backoff::new(options.max_backoff);
    let mut rt = Runtime { sink, state: PowerState::default(), emitted: None, warning: WarningLevel::None };
    loop {
        match connect(options.bus, options.bus_address.as_deref(), options.call_timeout).await {
            Ok(conn) => {
                backoff.reset();
                let ticker = tick_connection(&ctx.ex, &conn);
                let end = rt.session(&ctx, &conn).await;
                drop(ticker);
                close(conn).await;
                if matches!(end, End::Stop) {
                    return;
                }
                tracing::warn!("güç: sistem bus bağlantısı koptu, yeniden bağlanılacak");
            }
            Err(e) => tracing::warn!("güç: sistem bus'ına bağlanılamadı: {e}"),
        }
        rt.state = PowerState::default();
        rt.publish();
        // Bağlı değilken gelen istekler hemen yanıtlanır: `msg power charge-limit` beklemede kalmaz.
        let sink = &rt.sink;
        let awake = ctx
            .sleep_with(backoff.next_delay(), |c| match c {
                Command::SetChargeLimit { id, .. } => {
                    sink(PowerEvent::ChargeLimitDone { id, result: Err(PowerError::Unavailable) });
                }
                Command::SetProfile { id, .. } => sink(PowerEvent::ProfileDone { id, result: Err(PowerError::Unavailable) }),
            })
            .await;
        if !awake {
            return;
        }
    }
}

struct Runtime<F> {
    sink: F,
    state: PowerState,
    emitted: Option<PowerState>,
    warning: WarningLevel,
}

impl<F: Fn(PowerEvent)> Runtime<F> {
    fn publish(&mut self) {
        if self.emitted.as_ref() == Some(&self.state) {
            return;
        }
        self.emitted = Some(self.state.clone());
        (self.sink)(PowerEvent::Changed(self.state.clone()));
        let level = self.state.battery().map_or(WarningLevel::None, |b| b.warning);
        if let Some(level) = warning_transition(self.warning, &self.state) {
            (self.sink)(PowerEvent::BatteryWarning(level));
        }
        // Şarja takılınca ya da seviye düşünce (şarj oldu) sıfırla.
        self.warning = if self.state.on_battery { level } else { WarningLevel::None };
    }

    async fn session(&mut self, ctx: &ServiceContext<Command>, conn: &Connection) -> End {
        let (tx, rx) = async_channel::bounded::<Input>(INPUT_CAPACITY);
        let mut up = NameOwner::new();
        let mut ppd = NameOwner::new();
        // Akışlar sahip/özellik okunmadan önce kurulur: aradaki değişiklik kaçmaz.
        let streams = async {
            Ok::<_, zbus::Error>((
                owner_changes(conn, UPOWER).await?,
                owner_changes(conn, PPD).await?,
                signals_from(conn, UPOWER, Some(UPOWER_PATH), Some(PROPERTIES)).await?,
                // `DeviceAdded` / `DeviceRemoved` (yönetici arayüzü).
                signals_from(conn, UPOWER, Some(UPOWER_PATH), Some(UPOWER_IFACE)).await?,
                signals_from(conn, PPD, Some(PPD_PATH), Some(PROPERTIES)).await?,
            ))
        };
        let (up_owner, ppd_owner, up_signals, up_devices, ppd_signals) = match streams.await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("güç: sinyal aboneliği kurulamadı: {e}");
                return End::Lost;
            }
        };
        let _tasks = [
            forward(&ctx.ex, up_owner, tx.clone(), |m| owner_input(&m, UPOWER, Input::UpOwner)),
            forward(&ctx.ex, ppd_owner, tx.clone(), |m| owner_input(&m, PPD, Input::PpdOwner)),
            forward(&ctx.ex, up_signals, tx.clone(), Input::UpSignal),
            forward(&ctx.ex, up_devices, tx.clone(), Input::UpSignal),
            forward(&ctx.ex, ppd_signals, tx.clone(), Input::PpdSignal),
        ];
        self.upower_owner(conn, &mut up, get_name_owner(conn, UPOWER).await).await;
        self.ppd_owner(conn, &mut ppd, get_name_owner(conn, PPD).await).await;
        self.publish();

        // Şarj sınırı istekleri sırayla: bir sonraki, öncekinin sonucu (ve yeniden okunan durum)
        // geldikten sonra değerlendirilir. Aksi hâlde uçuştaki "aç"ın ardından gelen "kapat"
        // bayat duruma bakıp "zaten kapalı" derdi.
        let mut charge = ChargeQueue::default();
        let end = loop {
            let wake = async { rx.recv().await.map_or(Wake::Lost, Wake::Input) }
                .or(async { ctx.commands.recv().await.map_or(Wake::Stop, Wake::Command) })
                .or(async { ctx.stop.recv().await.map_or(Wake::Stop, |()| Wake::Stop) })
                .await;
            match wake {
                Wake::Stop => break End::Stop,
                Wake::Lost => break End::Lost,
                Wake::Command(Command::SetProfile { id, profile }) => {
                    if ppd.owner.is_none() {
                        (self.sink)(PowerEvent::ProfileDone { id, result: Err(PowerError::Unavailable) });
                        continue;
                    }
                    let conn = conn.clone();
                    let tx = tx.clone();
                    ctx.ex
                        .spawn(async move {
                            let result = set_property(
                                &conn,
                                PPD,
                                PPD_PATH,
                                PPD_IFACE,
                                "ActiveProfile",
                                Value::from(profile.as_str()),
                            )
                            .await
                            .map_err(|e| profile_error(&e));
                            let _ = tx.send(Input::ProfileResult { id, result }).await;
                        })
                        .detach();
                }
                Wake::Input(Input::ProfileResult { id, result }) => {
                    match &result {
                        // Yeni profil yanıttan önce yayınlansın (ppd'nin `PropertiesChanged`'i
                        // yöntem yanıtından sonra gelebilir).
                        Ok(()) => {
                            self.reload_profiles(conn, &ppd).await;
                            self.publish();
                        }
                        Err(e) => tracing::warn!(error = %e, "güç profili değiştirilemedi"),
                    }
                    (self.sink)(PowerEvent::ProfileDone { id, result });
                }
                Wake::Command(Command::SetChargeLimit { id, on }) => {
                    charge.waiting.push_back((id, on));
                    self.next_charge(ctx, conn, &tx, &mut charge);
                }
                Wake::Input(Input::ChargeResult { id, paths, result }) => {
                    if let Err(e) = &result {
                        tracing::warn!(error = %e, "şarj sınırı değiştirilemedi");
                    }
                    // Yeni durum yanıttan önce yayınlansın: UPower'ın `PropertiesChanged`'i yöntem
                    // yanıtından sonra gelebilir. Hatada da okunur: çok pilde ilk pil değişip
                    // ikincisi reddedilirse model yarım değişikliği göstermeli (karışık durum).
                    for path in &paths {
                        self.load_battery(conn, &up, path).await;
                    }
                    self.publish();
                    (self.sink)(PowerEvent::ChargeLimitDone { id, result });
                    charge.in_flight = None;
                    self.next_charge(ctx, conn, &tx, &mut charge);
                }
                Wake::Input(input) => {
                    if self.input(conn, &mut up, &mut ppd, input).await {
                        self.publish();
                    }
                }
            }
        };
        // Bağlantı bitti: bekleyen her istek yanıt alsın (çağıran asılı kalmasın).
        for id in charge.in_flight.into_iter().chain(charge.waiting.into_iter().map(|(id, _)| id)) {
            (self.sink)(PowerEvent::ChargeLimitDone { id, result: Err(PowerError::Unavailable) });
        }
        end
    }

    /// Uçuşta istek yoksa sıradakileri başlatır: çağrı gerekmeyenler (zaten o durumda,
    /// desteklenmiyor, UPower yok) hemen yanıtlanır, ilk çağrı gerektiren uçuşa çıkar.
    fn next_charge(
        &mut self,
        ctx: &ServiceContext<Command>,
        conn: &Connection,
        tx: &async_channel::Sender<Input>,
        charge: &mut ChargeQueue,
    ) {
        while charge.in_flight.is_none() {
            let Some((id, on)) = charge.waiting.pop_front() else { return };
            let paths = if self.state.upower {
                self.state.charge_targets(on).ok_or(PowerError::Unsupported)
            } else {
                Err(PowerError::Unavailable)
            };
            let paths = match paths {
                Ok(p) if !p.is_empty() => p,
                // Zaten istenen durumda: çağrı yok.
                Ok(_) => {
                    (self.sink)(PowerEvent::ChargeLimitDone { id, result: Ok(()) });
                    continue;
                }
                Err(e) => {
                    (self.sink)(PowerEvent::ChargeLimitDone { id, result: Err(e) });
                    continue;
                }
            };
            charge.in_flight = Some(id);
            let conn = conn.clone();
            let tx = tx.clone();
            ctx.ex
                .spawn(async move {
                    let mut result = Ok(());
                    for path in &paths {
                        let r = conn
                            .call_method(Some(UPOWER), path.as_str(), Some(DEVICE_IFACE), "EnableChargeThreshold", &(on,))
                            .await;
                        if let Err(e) = r {
                            result = Err(charge_error(&e));
                            break;
                        }
                    }
                    let _ = tx.send(Input::ChargeResult { id, paths, result }).await;
                })
                .detach();
        }
    }

    /// Girdiyi uygular; durum değişmiş olabilirse `true`.
    async fn input(&mut self, conn: &Connection, up: &mut NameOwner, ppd: &mut NameOwner, input: Input) -> bool {
        match input {
            Input::UpOwner => {
                let owner = get_name_owner(conn, UPOWER).await;
                self.upower_owner(conn, up, owner).await;
                true
            }
            Input::PpdOwner => {
                let owner = get_name_owner(conn, PPD).await;
                self.ppd_owner(conn, ppd, owner).await;
                true
            }
            Input::UpSignal(msg) => {
                if !up.accepts(&msg) {
                    return false;
                }
                let (path, iface, member) = {
                    let h = msg.header();
                    let s = |v: Option<&str>| v.unwrap_or_default().to_owned();
                    (s(h.path().map(|p| p.as_str())), s(h.interface().map(|i| i.as_str())), s(h.member().map(|m| m.as_str())))
                };
                match (path.as_str(), iface.as_str(), member.as_str()) {
                    (UPOWER_PATH, UPOWER_IFACE, "DeviceAdded") => match device_arg(&msg) {
                        Some(device) => self.load_battery(conn, up, &device).await,
                        None => false,
                    },
                    (UPOWER_PATH, UPOWER_IFACE, "DeviceRemoved") => {
                        device_arg(&msg).is_some_and(|device| self.state.untrack_battery(&device))
                    }
                    (_, PROPERTIES, "PropertiesChanged") => {
                        let Some((iface, changed, _)) = parse_properties_changed(&msg, MAX_CHANGED) else {
                            return false;
                        };
                        match (path.as_str(), iface.as_str()) {
                            (UPOWER_PATH, UPOWER_IFACE) => self.state.apply_manager(&changed),
                            (DISPLAY_DEVICE, DEVICE_IFACE) => self.state.apply_device(&changed),
                            (p, DEVICE_IFACE) if self.state.tracks_battery(p) => {
                                let serial = msg.primary_header().serial_num().get();
                                return self.state.apply_battery(p, &changed, serial);
                            }
                            _ => return false,
                        }
                        true
                    }
                    _ => false,
                }
            }
            Input::PpdSignal(msg) => {
                if !ppd.accepts(&msg) || msg.header().member().map(|m| m.as_str()) != Some("PropertiesChanged") {
                    return false;
                }
                let Some((iface, changed, _)) = parse_properties_changed(&msg, MAX_CHANGED) else {
                    return false;
                };
                if iface != PPD_IFACE || msg.header().path().map(|p| p.as_str()) != Some(PPD_PATH) {
                    return false;
                }
                self.state.apply_profiles(&changed);
                true
            }
            // Ana döngüde işlenir (yanıt sırası); buraya gelmez.
            Input::Ignore | Input::ProfileResult { .. } | Input::ChargeResult { .. } => false,
        }
    }

    /// Profilleri yeniden okur (yazma başarılı oldu; yanıt yalnızca şu anki sahipten kabul).
    async fn reload_profiles(&mut self, conn: &Connection, ppd: &NameOwner) {
        if ppd.owner.is_none() {
            return;
        }
        match get_all(conn, PPD, PPD_PATH, PPD_IFACE).await {
            Ok((props, reply)) if ppd.accepts(&reply) => self.state.apply_profiles(&props),
            Ok(_) => {}
            Err(e) => tracing::debug!("güç: profiller yeniden okunamadı: {e}"),
        }
    }

    async fn upower_owner(&mut self, conn: &Connection, up: &mut NameOwner, owner: Option<String>) {
        if owner.is_some() && owner == up.owner {
            return;
        }
        up.set_owner(owner);
        self.state.clear_upower();
        if up.owner.is_none() {
            return;
        }
        match (
            get_all(conn, UPOWER, UPOWER_PATH, UPOWER_IFACE).await,
            get_all(conn, UPOWER, DISPLAY_DEVICE, DEVICE_IFACE).await,
        ) {
            (Ok((manager, reply_a)), Ok((device, reply_b))) => {
                up.loaded(&reply_a);
                up.loaded(&reply_b);
                self.state.upower = true;
                self.state.apply_manager(&manager);
                self.state.apply_device(&device);
            }
            (Err(e), _) | (_, Err(e)) => {
                tracing::warn!("güç: UPower okunamadı: {e}");
                return;
            }
        }
        // Şarj sınırı gerçek pil cihazlarında. Okunamazsa yalnızca şarj sınırı "desteklenmiyor"
        // görünür; pil göstergesi etkilenmez.
        let devices = conn
            .call_method(Some(UPOWER), UPOWER_PATH, Some(UPOWER_IFACE), "EnumerateDevices", &())
            .await
            .and_then(|r| r.body().deserialize::<Vec<OwnedObjectPath>>());
        match devices {
            Ok(devices) => {
                for device in devices.iter().take(MAX_DEVICES) {
                    self.load_battery(conn, up, device.as_str()).await;
                }
            }
            Err(e) => tracing::warn!("güç: UPower cihazları listelenemedi: {e}"),
        }
    }

    /// Cihazı tam okur; sistem piliyse izler (değilse ya da okunamazsa izlemeden çıkarır).
    /// Görünür durum değişmiş olabilirse `true`. Cevabın seri numarası o pilin sınırıdır:
    /// paylaşılan `NameOwner` sınırı yükseltilmez (sıradaki başka nesnelerin sinyalleri atılmasın).
    async fn load_battery(&mut self, conn: &Connection, up: &NameOwner, path: &str) -> bool {
        if !valid_device_path(path) {
            return false;
        }
        match get_all(conn, UPOWER, path, DEVICE_IFACE).await {
            Ok((props, reply)) if up.accepts(&reply) => {
                self.state.track_battery(path, &props, reply.primary_header().serial_num().get())
            }
            Ok(_) => false,
            Err(e) => {
                tracing::debug!("güç: cihaz okunamadı: {e}");
                self.state.untrack_battery(path)
            }
        }
    }

    async fn ppd_owner(&mut self, conn: &Connection, ppd: &mut NameOwner, owner: Option<String>) {
        if owner.is_some() && owner == ppd.owner {
            return;
        }
        ppd.set_owner(owner);
        self.state.profiles = None;
        if ppd.owner.is_none() {
            return;
        }
        match get_all(conn, PPD, PPD_PATH, PPD_IFACE).await {
            Ok((props, reply)) => {
                ppd.loaded(&reply);
                self.state.apply_profiles(&props);
            }
            Err(e) => tracing::warn!("güç: power-profiles-daemon okunamadı: {e}"),
        }
    }
}

/// Şarj sınırı istek sırası (oturum başına).
#[derive(Default)]
struct ChargeQueue {
    waiting: std::collections::VecDeque<(u64, bool)>,
    in_flight: Option<u64>,
}

enum Wake {
    Stop,
    Lost,
    Input(Input),
    Command(Command),
}

/// UPower cihaz nesnesi yolu mu (birleşik `DisplayDevice` hariç)?
fn valid_device_path(path: &str) -> bool {
    path.len() <= MAX_PATH && path.starts_with(DEVICES_PREFIX) && path != DISPLAY_DEVICE
}

/// `DeviceAdded(o)` / `DeviceRemoved(o)` argümanı.
fn device_arg(msg: &Message) -> Option<String> {
    let (path,): (OwnedObjectPath,) = msg.body().deserialize().ok()?;
    valid_device_path(path.as_str()).then(|| path.to_string())
}

fn owner_input(msg: &Message, name: &str, input: Input) -> Input {
    if parse_owner_change(msg, name).is_some() { input } else { Input::Ignore }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method_error(name: &str, detail: &str) -> zbus::Error {
        let msg = zbus::message::Message::method_call("/", "M").unwrap().build(&()).unwrap();
        zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from(name.to_owned()).unwrap(),
            Some(detail.to_owned()),
            msg,
        )
    }

    #[test]
    fn errors_map_to_kinds() {
        let p = |n| profile_error(&method_error(n, "x"));
        assert_eq!(p("org.freedesktop.DBus.Error.AccessDenied"), PowerError::Denied);
        assert_eq!(p("org.freedesktop.DBus.Error.InvalidArgs"), PowerError::Unsupported);
        assert_eq!(p("org.freedesktop.DBus.Error.ServiceUnknown"), PowerError::Unavailable);
        assert_eq!(p("org.example.Odd"), PowerError::Failed("org.example.Odd".into()));
        let c = |n, d| charge_error(&method_error(n, d));
        assert_eq!(c("org.freedesktop.UPower.GeneralError", "Operation is not allowed."), PowerError::Denied);
        assert_eq!(c("org.freedesktop.UPower.GeneralError", "sysfs"), PowerError::Failed("org.freedesktop.UPower.GeneralError".into()));
        assert_eq!(c("org.freedesktop.UPower.NotSupported", ""), PowerError::Unsupported);
        assert_eq!(PowerError::Failed("a".into()).code(), "failed");
    }
}
