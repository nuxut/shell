//! D-Bus sistem servisleri (ağ, Bluetooth, güç, parlaklık, medya) için ortak altyapı.
//!
//! **Thread modeli.** Her servis kendi thread'inde tek iş parçacıklı bir yürütücü
//! (`LocalExecutor`) ve kendi bus bağlantısıyla çalışır; zbus'un yürütücüsü de o thread'de
//! döndürülür (`internal_executor(false)`). Boşta thread `epoll`'da bekler: iş yoktur.
//! Bağlantı başına maliyet bir soket; servisler birbirini bekletmez, biri takılırsa ya da
//! düşerse diğerleri etkilenmez. [`ServiceThread`] düşürülünce döngü durur, thread join edilir.
//!
//! **Taklit koruması.** Sinyaller iyi bilinen ada (ör. `org.freedesktop.NetworkManager`)
//! match kuralıyla abone olunur; bus yalnızca o adın sahibinin yayınlarını iletir. Ancak bir
//! istemci doğrudan bize (unicast) sahte sinyal gönderebilir ve zbus iyi bilinen göndereni
//! istemci tarafında doğrulamaz. Bu yüzden her servis adın **şu anki benzersiz sahibini**
//! ([`NameOwner`]) `NameOwnerChanged` ile izler ve yalnızca o göndericiden gelen mesajları
//! kabul eder ([`NameOwner::accepts`]). `NameOwnerChanged`'in kendisi de yalnızca bus'tan
//! (`org.freedesktop.DBus`) gelirse kabul edilir.
//!
//! **Sınırlar.** D-Bus'tan gelen her dize [`clean_str`] ile uzunluk sınırlı ve kontrol
//! karaktersizdir; diziler [`prop_paths`] gibi yardımcılarla eleman sayısı sınırlıdır.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::rc::Rc;
use std::thread::JoinHandle;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use async_executor::{LocalExecutor, Task};
use futures_lite::StreamExt;
use futures_lite::future::{self, FutureExt};
use zbus::message::Type as MessageType;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, MatchRule, Message, MessageStream};

pub const DBUS_NAME: &str = "org.freedesktop.DBus";
pub const DBUS_PATH: &str = "/org/freedesktop/DBus";
pub const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
pub const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

/// Sinyal akışı başına zbus kuyruğu (taşarsa en eski atılır; sonraki tam okuma düzeltir).
pub const SIGNAL_QUEUE: usize = 256;
/// Bağlantı kapatılırken en fazla bu kadar beklenir.
const CLOSE_TIMEOUT: Duration = Duration::from_millis(300);

pub type PropMap = HashMap<String, OwnedValue>;

/// Bir eylem isteğinin kimliği (servis başına artan); sonuç olayları (`Done`/`Failed`) bunu
/// taşır. NM ve BlueZ istemcilerinde aynı tür.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// Servisin kullanılamama nedeni (durumda `available == false` iken).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unavailable {
    /// Bus'a bağlanılamadı ya da bağlantı koptu (yeniden denenir).
    BusUnreachable,
    /// Bus var, servis (NetworkManager / BlueZ) çalışmıyor (bus'ta adın sahibi yok).
    ServiceNotRunning,
}

/// Hangi bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BusKind {
    #[default]
    System,
    Session,
}

impl BusKind {
    /// Test anahtarı: `var` ortam değişkeni tam olarak `session` ise oturum bus'ı, değilse
    /// sistem bus'ı. Crate'ler ortamı kendiliğinden okumaz; bunu çağıran (ör. Ayarlar'ın
    /// canlı testi, `dbus-run-session` içinde sahte servisle) açıkça çağırır. Yalnız
    /// `test-support` özelliğiyle derlenir (üründe yok).
    #[cfg(feature = "test-support")]
    pub fn from_env(var: &str) -> Self {
        Self::parse(std::env::var(var).ok().as_deref())
    }

    /// [`BusKind::from_env`]'in saf hâli (`Some("session")` → oturum).
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("session") => Self::Session,
            _ => Self::System,
        }
    }
}

/// Sahte servisin (examples/fake_*) adını alması için açık işaret: `1` olmalı.
#[cfg(feature = "test-support")]
pub const FAKE_BUS_ENV: &str = "NUXUT_FAKE_BUS";

/// Sahte servis koruması (yalnız `test-support`): oturum bus'ı `dbus-run-session`'ın yalıtılmış
/// bus'ı mı? `flag`: [`FAKE_BUS_ENV`] değeri (tam `1` olmalı); `address`:
/// `DBUS_SESSION_BUS_ADDRESS`; `runtime_dir`: `XDG_RUNTIME_DIR`. Reddedilir: işaret yok, adres
/// boş, kullanıcının gerçek oturum bus'ı (`unix:path=$XDG_RUNTIME_DIR/bus`,
/// `unix:path=/run/user/*/bus`) ya da `unix:abstract=` biçimli adres (dbus-launch/X oturumları;
/// gerçek oturum sayılır). `Err`: reddin nedeni.
#[cfg(feature = "test-support")]
pub fn fake_bus_guard(flag: Option<&str>, address: Option<&str>, runtime_dir: Option<&str>) -> Result<(), &'static str> {
    if flag != Some("1") {
        return Err("NUXUT_FAKE_BUS=1 missing");
    }
    let address = address.unwrap_or_default().trim();
    if address.is_empty() {
        return Err("DBUS_SESSION_BUS_ADDRESS empty");
    }
    let user_bus = runtime_dir.map(|d| format!("unix:path={}/bus", d.trim_end_matches('/')));
    // Adres `;` ile ayrılmış bir liste olabilir (D-Bus spesifikasyonu; istemci sırayla dener):
    // her girdi ayrı denetlenir, biri bile gerçek oturumsa red.
    // Girdi biçimi `taşıma:anahtar=değer,anahtar=değer` (anahtar sırası serbest).
    let mut any = false;
    for entry in address.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        any = true;
        let (transport, keys) = entry.split_once(':').unwrap_or((entry, ""));
        let keys: Vec<&str> = keys.split(',').map(str::trim).collect();
        // `unix:runtime=yes` = `$XDG_RUNTIME_DIR/bus`, yani kullanıcının oturum bus'ı.
        if transport == "unix" && keys.contains(&"runtime=yes") {
            return Err("unix:runtime=yes is the user session bus");
        }
        if transport == "unix" && keys.iter().any(|k| k.starts_with("abstract=")) {
            return Err("unix:abstract session bus (treated as real session)");
        }
        for path in keys.iter().filter_map(|k| k.strip_prefix("path=")) {
            let user_default = path.starts_with("/run/user/") && path.ends_with("/bus");
            if user_default || user_bus.as_deref() == Some(format!("unix:path={path}").as_str()) {
                return Err("user session bus");
            }
        }
    }
    if !any {
        return Err("DBUS_SESSION_BUS_ADDRESS empty");
    }
    Ok(())
}

/// Bus bağlantısı kurar. `address` verilirse (testler) o adrese bağlanır.
pub async fn connect(kind: BusKind, address: Option<&str>, timeout: Duration) -> zbus::Result<Connection> {
    let builder = match (address, kind) {
        (Some(address), _) => zbus::connection::Builder::address(address)?,
        (None, BusKind::System) => zbus::connection::Builder::system()?,
        (None, BusKind::Session) => zbus::connection::Builder::session()?,
    };
    builder.internal_executor(false).method_timeout(timeout).build().await
}

/// Bağlantının yürütücüsünü bu thread'de döndüren görev. Görev düşürülünce durur.
pub fn tick_connection(ex: &LocalExecutor<'static>, conn: &Connection) -> Task<()> {
    let conn = conn.clone();
    ex.spawn(async move {
        loop {
            conn.executor().tick().await;
        }
    })
}

/// Bağlantıyı en fazla [`CLOSE_TIMEOUT`] bekleyerek kapatır (adlar ve match kuralları hemen bırakılır).
pub async fn close(conn: Connection) {
    let _ = conn
        .close()
        .or(async {
            async_io::Timer::after(CLOSE_TIMEOUT).await;
            Ok(())
        })
        .await;
}

// ---------------------------------------------------------------------------------------
// Dizeler ve özellikler

/// Kontrol karakterlerini atar (satır sonu/sekme boşluğa döner), en fazla `max_bytes` bayt
/// tutar (karakter sınırında keser).
pub fn clean_str(s: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_bytes));
    for c in s.chars() {
        let c = match c {
            '\n' | '\t' | '\r' => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        if out.len() + c.len_utf8() > max_bytes {
            break;
        }
        out.push(c);
    }
    out
}

/// İç içe `v` sarmalarını açar.
fn inner<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
    let mut v = v;
    for _ in 0..4 {
        match v {
            Value::Value(b) => v = b,
            _ => break,
        }
    }
    v
}

pub fn value_str(v: &Value<'_>, max: usize) -> Option<String> {
    match inner(v) {
        Value::Str(s) => Some(clean_str(s.as_str(), max)),
        Value::ObjectPath(p) => Some(clean_str(p.as_str(), max)),
        _ => None,
    }
}

pub fn value_bool(v: &Value<'_>) -> Option<bool> {
    match inner(v) {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

pub fn value_i64(v: &Value<'_>) -> Option<i64> {
    match inner(v) {
        Value::U8(n) => Some(i64::from(*n)),
        Value::I16(n) => Some(i64::from(*n)),
        Value::U16(n) => Some(i64::from(*n)),
        Value::I32(n) => Some(i64::from(*n)),
        Value::U32(n) => Some(i64::from(*n)),
        Value::I64(n) => Some(*n),
        Value::U64(n) => i64::try_from(*n).ok(),
        _ => None,
    }
}

pub fn value_f64(v: &Value<'_>) -> Option<f64> {
    match inner(v) {
        Value::F64(n) if n.is_finite() => Some(*n),
        _ => value_i64(v).map(|n| n as f64),
    }
}

/// `ay` → bayt dizisi (en fazla `max`; uzunsa `None`).
pub fn value_bytes(v: &Value<'_>, max: usize) -> Option<Vec<u8>> {
    match inner(v) {
        Value::Array(a) if a.len() <= max => a
            .inner()
            .iter()
            .map(|e| match e {
                Value::U8(b) => Some(*b),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// `ao`/`as` → dizeler (en fazla `max_items`, her biri `max_len` bayt).
pub fn value_strs(v: &Value<'_>, max_items: usize, max_len: usize) -> Vec<String> {
    match inner(v) {
        Value::Array(a) => a.inner().iter().take(max_items).filter_map(|e| value_str(e, max_len)).collect(),
        _ => Vec::new(),
    }
}

pub fn prop_str(props: &PropMap, key: &str, max: usize) -> Option<String> {
    props.get(key).and_then(|v| value_str(v, max))
}

pub fn prop_bool(props: &PropMap, key: &str) -> Option<bool> {
    props.get(key).and_then(|v| value_bool(v))
}

pub fn prop_i64(props: &PropMap, key: &str) -> Option<i64> {
    props.get(key).and_then(|v| value_i64(v))
}

pub fn prop_u32(props: &PropMap, key: &str) -> Option<u32> {
    prop_i64(props, key).and_then(|n| u32::try_from(n).ok())
}

pub fn prop_f64(props: &PropMap, key: &str) -> Option<f64> {
    props.get(key).and_then(|v| value_f64(v))
}

pub fn prop_bytes(props: &PropMap, key: &str, max: usize) -> Option<Vec<u8>> {
    props.get(key).and_then(|v| value_bytes(v, max))
}

/// Nesne yolu dizisi (en fazla `max_items`).
pub fn prop_paths(props: &PropMap, key: &str, max_items: usize) -> Vec<String> {
    props.get(key).map(|v| value_strs(v, max_items, MAX_PATH)).unwrap_or_default()
}

/// Nesne yolu sınırı.
pub const MAX_PATH: usize = 256;

/// `PropertiesChanged(s interface, a{sv} changed, as invalidated)` gövdesi. Değişen
/// özellik sayısı sınırlı değilse bile bus mesaj boyutu (128 MiB) ile sınırlıdır; burada en
/// fazla `max` özellik alınır.
pub fn parse_properties_changed(msg: &Message, max: usize) -> Option<(String, PropMap, Vec<String>)> {
    let (iface, changed, invalidated): (String, PropMap, Vec<String>) = msg.body().deserialize().ok()?;
    if changed.len() > max || invalidated.len() > max {
        return None;
    }
    Some((iface, changed, invalidated))
}

// ---------------------------------------------------------------------------------------
// Ad sahibi

/// İzlenen iyi bilinen adın benzersiz sahibi.
#[derive(Debug, Clone, Default)]
pub struct NameOwner {
    pub owner: Option<String>,
    /// Son tam okumanın cevap seri numarası: sahibin bundan önce gönderdiği sinyaller (okuma
    /// cevabından eski) atılır. Aynı göndericinin seri numaraları artandır.
    min_serial: u32,
}

impl NameOwner {
    pub fn new() -> Self {
        Self { owner: None, min_serial: 0 }
    }

    pub fn set_owner(&mut self, owner: Option<String>) {
        self.owner = owner;
        self.min_serial = 0;
    }

    /// Tam okuma cevabı geldi: ondan eski sinyaller artık geçersiz.
    pub fn loaded(&mut self, reply: &Message) {
        if self.accepts(reply) {
            self.min_serial = self.min_serial.max(reply.primary_header().serial_num().get());
        }
    }

    /// Mesaj adın şu anki benzersiz sahibinden mi (ve son tam okumadan yeni mi)?
    pub fn accepts(&self, msg: &Message) -> bool {
        let header = msg.header();
        match (header.sender(), &self.owner) {
            (Some(sender), Some(owner)) => {
                sender.as_str() == owner && msg.primary_header().serial_num().get() > self.min_serial
            }
            _ => false,
        }
    }

    /// Benzersiz ad (`:1.42`) mı? Kendi kendini doğrulama: iyi bilinen ad sahip olamaz.
    pub fn is_unique(name: &str) -> bool {
        name.starts_with(':') && name.len() <= 255
    }
}

/// Adın şu anki sahibi (`GetNameOwner`; etkinleştirme tetiklemez).
pub async fn get_name_owner(conn: &Connection, name: &str) -> Option<String> {
    let reply = conn.call_method(Some(DBUS_NAME), DBUS_PATH, Some(DBUS_NAME), "GetNameOwner", &(name,)).await.ok()?;
    let owner: String = reply.body().deserialize().ok()?;
    NameOwner::is_unique(&owner).then_some(owner)
}

/// `name` için `NameOwnerChanged` akışı.
pub async fn owner_changes(conn: &Connection, name: &'static str) -> zbus::Result<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender(DBUS_NAME)?
        .interface(DBUS_NAME)?
        .member("NameOwnerChanged")?
        .arg(0, name)?
        .build();
    MessageStream::for_match_rule(rule, conn, Some(16)).await
}

/// `NameOwnerChanged` mesajını çözer: yalnızca bus'tan geldiyse ve `name` içinse yeni sahip
/// (`Some(None)`: ad bırakıldı).
pub fn parse_owner_change(msg: &Message, name: &str) -> Option<Option<String>> {
    let header = msg.header();
    if header.sender().map(|s| s.as_str()) != Some(DBUS_NAME)
        || header.member().map(|m| m.as_str()) != Some("NameOwnerChanged")
    {
        return None;
    }
    let (changed, _old, new): (String, String, String) = msg.body().deserialize().ok()?;
    if changed != name {
        return None;
    }
    Some(NameOwner::is_unique(&new).then_some(new))
}

/// Bir iyi bilinen adın gönderdiği sinyaller (isteğe bağlı yol ad alanı ve arayüz).
pub async fn signals_from(
    conn: &Connection,
    sender: &'static str,
    path_namespace: Option<&'static str>,
    interface: Option<&'static str>,
) -> zbus::Result<MessageStream> {
    let mut rule = MatchRule::builder().msg_type(MessageType::Signal).sender(sender)?;
    if let Some(ns) = path_namespace {
        rule = rule.path_namespace(ns)?;
    }
    if let Some(iface) = interface {
        rule = rule.interface(iface)?;
    }
    MessageStream::for_match_rule(rule.build(), conn, Some(SIGNAL_QUEUE)).await
}

/// Akıştaki her mesajı `map` ile çevirip kanala ileten görev. Kanal kapanınca biter.
pub fn forward<T: 'static>(
    ex: &LocalExecutor<'static>,
    mut stream: MessageStream,
    tx: Sender<T>,
    map: impl Fn(Message) -> T + 'static,
) -> Task<()> {
    ex.spawn(async move {
        while let Some(msg) = stream.next().await {
            let Ok(msg) = msg else { continue };
            if tx.send(map(msg)).await.is_err() {
                break;
            }
        }
    })
}

// ---------------------------------------------------------------------------------------
// Çağrılar

/// `Properties.GetAll` (özellik sayısı sınırlı) ve cevap mesajı (seri numarası için).
pub async fn get_all(
    conn: &Connection,
    dest: &str,
    path: &str,
    iface: &str,
) -> zbus::Result<(PropMap, Message)> {
    let reply = conn.call_method(Some(dest), path, Some(PROPERTIES), "GetAll", &(iface,)).await?;
    let props: PropMap = reply.body().deserialize()?;
    if props.len() > 256 {
        return Err(zbus::Error::Failure("çok fazla özellik".into()));
    }
    Ok((props, reply))
}

pub async fn set_property(
    conn: &Connection,
    dest: &str,
    path: &str,
    iface: &str,
    name: &str,
    value: Value<'_>,
) -> zbus::Result<()> {
    conn.call_method(Some(dest), path, Some(PROPERTIES), "Set", &(iface, name, value)).await.map(drop)
}

// ---------------------------------------------------------------------------------------
// ObjectManager nesne ağacı

/// `org.freedesktop.DBus.ObjectManager` ham ağacı: yol → arayüz → özellikler. Yalnızca
/// `limit`'in tanıdığı arayüzler tutulur ve her arayüzden en fazla o kadar nesne; yollar
/// `prefix` altında ve en fazla [`MAX_PATH`] bayt olmalı.
#[derive(Debug)]
pub struct ObjectStore {
    objects: std::collections::BTreeMap<String, std::collections::BTreeMap<String, PropMap>>,
    prefix: &'static str,
    limit: fn(&str) -> Option<usize>,
}

impl ObjectStore {
    pub fn new(prefix: &'static str, limit: fn(&str) -> Option<usize>) -> Self {
        Self { objects: Default::default(), prefix, limit }
    }

    pub fn clear(&mut self) {
        self.objects.clear();
    }

    fn count(&self, iface: &str) -> usize {
        self.objects.values().filter(|i| i.contains_key(iface)).count()
    }

    /// `InterfacesAdded` / `GetManagedObjects` girdisi. Sınır aşılırsa yeni nesne atılır.
    pub fn add(&mut self, path: &str, ifaces: impl IntoIterator<Item = (String, PropMap)>) -> bool {
        if !path.starts_with(self.prefix) || path.len() > MAX_PATH {
            return false;
        }
        let mut changed = false;
        for (iface, props) in ifaces {
            let Some(limit) = (self.limit)(&iface) else { continue };
            let exists = self.objects.get(path).is_some_and(|o| o.contains_key(&iface));
            if !exists && self.count(&iface) >= limit {
                tracing::debug!("{iface} sınırı ({limit}) aşıldı, {path} atlandı");
                continue;
            }
            self.objects.entry(path.to_owned()).or_default().insert(iface, props);
            changed = true;
        }
        changed
    }

    pub fn remove(&mut self, path: &str, ifaces: &[String]) -> bool {
        let Some(o) = self.objects.get_mut(path) else { return false };
        let before = o.len();
        o.retain(|k, _| !ifaces.contains(k));
        let changed = o.len() != before;
        if o.is_empty() {
            self.objects.remove(path);
        }
        changed
    }

    /// `PropertiesChanged`: yalnızca bilinen nesnenin bilinen arayüzüne uygulanır.
    pub fn update(&mut self, path: &str, iface: &str, changed: PropMap, invalidated: &[String]) -> bool {
        let Some(props) = self.objects.get_mut(path).and_then(|o| o.get_mut(iface)) else { return false };
        for k in invalidated {
            props.remove(k);
        }
        let any = !changed.is_empty() || !invalidated.is_empty();
        props.extend(changed);
        any
    }

    pub fn has(&self, path: &str, iface: &str) -> bool {
        self.objects.get(path).is_some_and(|o| o.contains_key(iface))
    }

    pub fn get(&self, path: &str, iface: &str) -> Option<&PropMap> {
        self.objects.get(path)?.get(iface)
    }

    /// `iface`'i olan nesneler (yol sırasıyla).
    pub fn with<'a>(&'a self, iface: &'a str) -> impl Iterator<Item = (&'a str, &'a PropMap)> + 'a {
        self.objects.iter().filter_map(move |(p, i)| i.get(iface).map(|props| (p.as_str(), props)))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &std::collections::BTreeMap<String, PropMap>)> {
        self.objects.iter().map(|(p, i)| (p.as_str(), i))
    }
}

/// ObjectManager sinyali (`InterfacesAdded`/`InterfacesRemoved`) ya da `PropertiesChanged`'i
/// ağaca uygular; değiştiyse `true`. Mesajın göndericisi çağıran tarafından doğrulanmış olmalı.
pub fn apply_object_signal(store: &mut ObjectStore, msg: &Message) -> bool {
    let header = msg.header();
    let member = header.member().map(|m| m.as_str()).unwrap_or_default();
    let iface = header.interface().map(|i| i.as_str()).unwrap_or_default();
    match (iface, member) {
        (OBJECT_MANAGER, "InterfacesAdded") => {
            let Ok((path, ifaces)) =
                msg.body().deserialize::<(zbus::zvariant::OwnedObjectPath, HashMap<String, PropMap>)>()
            else {
                return false;
            };
            ifaces.len() <= 32 && store.add(path.as_str(), ifaces)
        }
        (OBJECT_MANAGER, "InterfacesRemoved") => {
            let Ok((path, ifaces)) = msg.body().deserialize::<(zbus::zvariant::OwnedObjectPath, Vec<String>)>() else {
                return false;
            };
            ifaces.len() <= 32 && store.remove(path.as_str(), &ifaces)
        }
        (PROPERTIES, "PropertiesChanged") => {
            let Some(path) = header.path().map(|p| p.to_string()) else { return false };
            let Some((iface, changed, invalidated)) = parse_properties_changed(msg, 128) else { return false };
            store.update(&path, &iface, changed, &invalidated)
        }
        _ => false,
    }
}

/// `GetManagedObjects` (en fazla `max` nesne) ve cevap mesajı.
pub async fn get_managed_objects(
    conn: &Connection,
    dest: &str,
    path: &str,
    max: usize,
) -> zbus::Result<(HashMap<zbus::zvariant::OwnedObjectPath, HashMap<String, PropMap>>, Message)> {
    let reply = conn.call_method(Some(dest), path, Some(OBJECT_MANAGER), "GetManagedObjects", &()).await?;
    let objects: HashMap<zbus::zvariant::OwnedObjectPath, HashMap<String, PropMap>> = reply.body().deserialize()?;
    if objects.len() > max {
        return Err(zbus::Error::Failure("çok fazla nesne".into()));
    }
    Ok((objects, reply))
}

// ---------------------------------------------------------------------------------------
// Servis thread'i

/// Bir servisin thread'i: komut kuyruğu + durdurma kanalı. Düşürülünce durur ve join edilir.
#[derive(Debug)]
pub struct ServiceThread<C> {
    tx: Sender<C>,
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
    name: &'static str,
}

/// Servis gövdesine verilenler.
#[derive(Debug)]
pub struct ServiceContext<C> {
    pub ex: Rc<LocalExecutor<'static>>,
    pub commands: Receiver<C>,
    pub stop: Receiver<()>,
}

impl<C> ServiceContext<C> {
    /// `d` kadar bekler; arada durdurma gelirse `false`. Beklerken gelen komutlar atılır
    /// (servis bağlı değilken iş yapılamaz).
    pub async fn sleep(&self, d: Duration) -> bool {
        self.sleep_with(d, |_| tracing::debug!("servis bağlı değil, komut atıldı")).await
    }

    /// [`Self::sleep`] gibi; beklerken gelen komutlar atılmaz, `on_command`'a verilir (yanıt
    /// bekleyen istekler "kullanılamıyor" diye hemen yanıtlansın, çağıran asılı kalmasın).
    pub async fn sleep_with(&self, d: Duration, mut on_command: impl FnMut(C)) -> bool {
        let deadline = std::time::Instant::now() + d;
        loop {
            let wake = async {
                async_io::Timer::at(deadline).await;
                Wake::<C>::Timer
            }
            .or(async { self.stop.recv().await.map_or(Wake::Stop, |()| Wake::Stop) })
            .or(async { self.commands.recv().await.map_or(Wake::Stop, Wake::Command) })
            .await;
            match wake {
                Wake::Timer => return true,
                Wake::Stop => return false,
                Wake::Command(c) => on_command(c),
            }
        }
    }
}

enum Wake<C> {
    Timer,
    Stop,
    Command(C),
}

impl<C: Send + 'static> ServiceThread<C> {
    /// Thread'i başlatır; `body` thread içinde yürütücüde çalışır.
    pub fn spawn<F, Fut>(name: &'static str, capacity: usize, body: F) -> io::Result<Self>
    where
        F: FnOnce(ServiceContext<C>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let (tx, commands) = async_channel::bounded(capacity);
        let (stop_tx, stop) = async_channel::bounded(1);
        let thread = std::thread::Builder::new().name(name.into()).spawn(move || {
            let ex = Rc::new(LocalExecutor::new());
            let ctx = ServiceContext { ex: Rc::clone(&ex), commands, stop };
            future::block_on(ex.run(body(ctx)));
            // Kalan görevler (yönlendiriciler, bekleyen çağrılar) burada düşer.
            drop(ex);
        })?;
        Ok(Self { tx, stop: Some(stop_tx), thread: Some(thread), name })
    }

    /// Komut gönderir; bloklamaz. Kuyruk doluysa ya da servis durmuşsa `false`.
    pub fn send(&self, command: C) -> bool {
        let ok = self.tx.try_send(command).is_ok();
        if !ok {
            tracing::warn!(service = self.name, "komut kuyruğu dolu veya servis durmuş, komut atıldı");
        }
        ok
    }
}

impl<C> Drop for ServiceThread<C> {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!(service = self.name, "servis thread'i panikle sonlandı");
        }
    }
}

/// Yeniden bağlanma beklemesi: 250 ms'den başlar, her denemede iki katı, en fazla `max`.
#[derive(Debug, Clone)]
pub struct Backoff {
    next: Duration,
    max: Duration,
}

impl Backoff {
    pub fn new(max: Duration) -> Self {
        Self { next: Duration::from_millis(250).min(max), max }
    }

    pub fn reset(&mut self) {
        self.next = Duration::from_millis(250).min(self.max);
    }

    pub fn next_delay(&mut self) -> Duration {
        let d = self.next;
        self.next = (self.next * 2).min(self.max);
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_str_limits_and_strips() {
        assert_eq!(clean_str("a\u{7}b\nc", 16), "ab c");
        assert_eq!(clean_str("ğğğ", 3), "ğ");
        assert_eq!(clean_str("abc", 0), "");
    }

    #[test]
    fn bus_kind_parse() {
        assert_eq!(BusKind::parse(Some("session")), BusKind::Session);
        assert_eq!(BusKind::parse(Some("Session")), BusKind::System);
        assert_eq!(BusKind::parse(Some("")), BusKind::System);
        assert_eq!(BusKind::parse(None), BusKind::System);
        assert_eq!(BusKind::default(), BusKind::System);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn fake_bus_guard_rejects_real_sessions() {
        let run = Some("/run/user/1000");
        let iso = Some("unix:path=/tmp/dbus-AbC,guid=1");
        assert_eq!(fake_bus_guard(Some("1"), iso, run), Ok(()));
        assert!(fake_bus_guard(None, iso, run).is_err(), "işaret şart");
        assert!(fake_bus_guard(Some("yes"), iso, run).is_err());
        assert!(fake_bus_guard(Some("1"), None, run).is_err());
        assert!(fake_bus_guard(Some("1"), Some("unix:path=/run/user/1000/bus"), run).is_err());
        assert!(fake_bus_guard(Some("1"), Some("unix:path=/run/user/1000/bus,guid=x"), None).is_err());
        assert!(fake_bus_guard(Some("1"), Some("unix:path=/srv/rt/bus"), Some("/srv/rt/")).is_err());
        assert!(fake_bus_guard(Some("1"), Some("unix:abstract=/tmp/dbus-XYZ,guid=1"), run).is_err());
        assert!(fake_bus_guard(Some("1"), Some("unix:path=/tmp/x;unix:abstract=/tmp/y"), run).is_err());
        // Review notu (a): her `;` girdisi ayrı denetlenir; anahtar sırası serbest.
        assert!(fake_bus_guard(Some("1"), Some("unix:path=/tmp/x;unix:path=/run/user/1000/bus"), run).is_err());
        assert!(fake_bus_guard(Some("1"), Some("unix:path=/tmp/x,guid=1;unix:path=/srv/rt/bus"), Some("/srv/rt")).is_err());
        assert!(fake_bus_guard(Some("1"), Some("unix:guid=1,abstract=/tmp/y"), run).is_err());
        assert!(fake_bus_guard(Some("1"), Some(" ; "), run).is_err());
        assert_eq!(fake_bus_guard(Some("1"), Some("unix:path=/tmp/a;unix:path=/tmp/b"), run), Ok(()));
        assert_eq!(fake_bus_guard(Some("1"), Some("tcp:host=localhost,port=1"), run), Ok(()));
        // Review şartı (3): runtime=yes kullanıcının bus'ıdır.
        assert!(fake_bus_guard(Some("1"), Some("unix:runtime=yes"), run).is_err());
        assert!(fake_bus_guard(Some("1"), Some("unix:path=/tmp/x;unix:guid=1,runtime=yes"), run).is_err());
    }

    #[test]
    fn backoff_doubles_to_max() {
        let mut b = Backoff::new(Duration::from_secs(1));
        let v: Vec<_> = (0..5).map(|_| b.next_delay().as_millis()).collect();
        assert_eq!(v, [250, 500, 1000, 1000, 1000]);
        b.reset();
        assert_eq!(b.next_delay().as_millis(), 250);
    }

    #[test]
    fn values_are_typed_and_bounded() {
        let bytes = Value::from(vec![1u8, 2, 3]);
        assert_eq!(value_bytes(&bytes, 3), Some(vec![1, 2, 3]));
        assert_eq!(value_bytes(&bytes, 2), None);
        assert_eq!(value_i64(&Value::U32(7)), Some(7));
        assert_eq!(value_f64(&Value::F64(f64::NAN)), None);
        assert_eq!(value_str(&Value::Value(Box::new(Value::from("x\0y"))), 8).as_deref(), Some("xy"));
    }
}
