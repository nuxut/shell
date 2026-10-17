//! Saat dilimi değişimi: `org.freedesktop.timedate1` (systemd-timedated, sistem bus'ı)
//! `PropertiesChanged(Timezone)` dinlenir; değişince [`TimezoneEvent::Changed`] yayınlanır.
//!
//! **Neden gerekli** (t119, `docs/design/clock-locale.md`): `chrono::Local` (0.4.45,
//! `offset/local/unix.rs`) `/etc/localtime`'ı iş parçacığı başına önbelleğe alır ve en çok
//! saniyede bir sembolik bağın `mtime`'ına bakarak yeniler; timedated `SetTimezone` bağı
//! yeniden yazdığı için *bir sonraki* `Local::now()` çağrısı (≥ 1 sn sonra) yeni dilimi görür.
//! Ama shell saati dakikada bir çizer: yeni dilim en geç bir dakika sonra, takvimin "bugün"ü ve
//! etkinlik saatleri ise bir sonraki yeniden kuruluşa kadar eski kalırdı. Bu servis değişimi
//! anında bildirir; uygulama chrono önbelleğinin süresi dolsun diye kısa bir gecikmeyle
//! ([`SETTLE`]) saati, takvimi ve metin kuran her şeyi yeniden hesaplar.
//!
//! **Salt dinleme.** Hiçbir yöntem çağrılmaz, timedated etkinleştirilmez (`GetNameOwner`
//! etkinleştirmez); kullanıcının saat dilimine dokunulmaz. Bu yüzden test örneği
//! (`NUXUT_SHELL_INSTANCE`) de gerçek sistem bus'ını dinleyebilir.
//!
//! **Taklit koruması.** timedated boşta kalınca çıkar, her etkinleşmede benzersiz adı değişir.
//! Sinyal yalnızca göndericisi `timedate1` adının *o anki* sahibiyse kabul edilir: önbellekteki
//! sahip tutmazsa `GetNameOwner` ile yeniden sorulur (dilim değişimi seyrektir; tur maliyeti
//! önemsiz). Sahte sinyal en kötü ihtimalle zararsız bir yeniden çizim olurdu, yine de elenir.

use std::io;
use std::time::Duration;

use futures_lite::future::FutureExt;
use zbus::{Connection, Message};

use nuxut_dbus::{
    Backoff, BusKind, PROPERTIES, ServiceContext, ServiceThread, clean_str, close, connect, forward, get_name_owner,
    parse_properties_changed, signals_from, tick_connection, value_str,
};

pub const TIMEDATE1: &str = "org.freedesktop.timedate1";
pub const TIMEDATE1_PATH: &str = "/org/freedesktop/timedate1";
/// `chrono::Local`'ın `/etc/localtime` önbelleği en çok saniyede bir yenilenir: değişimden
/// sonra saat bu kadar beklenip yeniden hesaplanır (aksi hâlde eski dilim okunabilir).
pub const SETTLE: Duration = Duration::from_millis(1100);
const MAX_CHANGED: usize = 32;
/// IANA dilim adı sınırı (en uzunu ~32 bayt).
const MAX_ZONE: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimezoneEvent {
    /// Dilim değişti; `zone` sinyal taşıyorsa yeni ad (yalnızca günlük için; saat
    /// `/etc/localtime`'dan okunur).
    Changed { zone: Option<String> },
}

#[derive(Debug, Clone)]
pub struct TimezoneOptions {
    /// D-Bus adresi; `None`: sistem bus'ı (testler sahte timedate1 adresi verir).
    pub bus_address: Option<String>,
    pub call_timeout: Duration,
    pub max_backoff: Duration,
}

impl Default for TimezoneOptions {
    fn default() -> Self {
        Self { bus_address: None, call_timeout: Duration::from_secs(3), max_backoff: Duration::from_secs(60) }
    }
}

/// Çalışan dinleyici; düşürülünce thread join edilir.
#[derive(Debug)]
pub struct TimezoneWatch {
    _thread: ServiceThread<()>,
}

impl TimezoneWatch {
    /// `sink` servis thread'inden çağrılır; hızlı dönmeli.
    pub fn spawn<F>(options: TimezoneOptions, sink: F) -> io::Result<Self>
    where
        F: Fn(TimezoneEvent) + Send + 'static,
    {
        let thread = ServiceThread::spawn("nuxut-timezone", 1, move |ctx| run(options, sink, ctx))?;
        Ok(Self { _thread: thread })
    }
}

/// `PropertiesChanged` gövdesi saat dilimi değişimi mi: `org.freedesktop.timedate1`
/// arayüzünde `Timezone` değişmiş ya da geçersizlenmiş. Değişmişse yeni ad da döner.
pub(crate) fn timezone_change(iface: &str, changed: &nuxut_dbus::PropMap, invalidated: &[String]) -> Option<Option<String>> {
    if iface != TIMEDATE1 {
        return None;
    }
    if let Some(v) = changed.get("Timezone") {
        return Some(value_str(v, MAX_ZONE).filter(|s| !s.is_empty()));
    }
    invalidated.iter().any(|p| p == "Timezone").then_some(None)
}

enum Wake {
    Stop,
    Lost,
    Signal(Message),
}

async fn run<F: Fn(TimezoneEvent)>(options: TimezoneOptions, sink: F, ctx: ServiceContext<()>) {
    let mut backoff = Backoff::new(options.max_backoff);
    loop {
        match connect(BusKind::System, options.bus_address.as_deref(), options.call_timeout).await {
            Ok(conn) => {
                backoff.reset();
                let ticker = tick_connection(&ctx.ex, &conn);
                let stopped = listen(&ctx, &conn, &sink).await;
                drop(ticker);
                close(conn).await;
                if stopped {
                    return;
                }
                tracing::warn!("saat dilimi: sistem bus bağlantısı koptu, yeniden bağlanılacak");
            }
            Err(e) => tracing::warn!("saat dilimi: sistem bus'ına bağlanılamadı: {e}"),
        }
        if !ctx.sleep(backoff.next_delay()).await {
            return;
        }
    }
}

/// Bağlantı boyunca dinler; durdurulduysa `true`, bağlantı koptuysa `false`.
async fn listen<F: Fn(TimezoneEvent)>(ctx: &ServiceContext<()>, conn: &Connection, sink: &F) -> bool {
    let signals = match signals_from(conn, TIMEDATE1, Some(TIMEDATE1_PATH), Some(PROPERTIES)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("saat dilimi: timedate1 aboneliği kurulamadı: {e}");
            return false;
        }
    };
    let (tx, rx) = async_channel::bounded::<Message>(16);
    let _task = forward(&ctx.ex, signals, tx, |m| m);
    // Çalışmıyorsa `None` (etkinleştirilmez); ilk sinyalde yeniden sorulur.
    let mut owner = get_name_owner(conn, TIMEDATE1).await;
    loop {
        let wake = async { rx.recv().await.map_or(Wake::Lost, Wake::Signal) }
            .or(async { ctx.stop.recv().await.map_or(Wake::Stop, |()| Wake::Stop) })
            .await;
        let msg = match wake {
            Wake::Stop => return true,
            Wake::Lost => return false,
            Wake::Signal(m) => m,
        };
        let header = msg.header();
        if header.member().map(|m| m.as_str()) != Some("PropertiesChanged")
            || header.path().map(|p| p.as_str()) != Some(TIMEDATE1_PATH)
        {
            continue;
        }
        let Some(sender) = header.sender().map(|s| s.as_str().to_owned()) else { continue };
        if owner.as_deref() != Some(sender.as_str()) {
            // timedated yeniden etkinleşmiş olabilir: güncel sahibi sor.
            owner = get_name_owner(conn, TIMEDATE1).await;
            if owner.as_deref() != Some(sender.as_str()) {
                tracing::debug!(sender = %clean_str(&sender, 64), "saat dilimi: timedate1 sahibi olmayan göndericinin sinyali atıldı");
                continue;
            }
        }
        let Some((iface, changed, invalidated)) = parse_properties_changed(&msg, MAX_CHANGED) else { continue };
        if let Some(zone) = timezone_change(&iface, &changed, &invalidated) {
            tracing::info!(zone = zone.as_deref().unwrap_or("?"), "saat dilimi değişti");
            sink(TimezoneEvent::Changed { zone });
        }
    }
}

#[cfg(test)]
mod tests {
    use zbus::zvariant::{OwnedValue, Value};

    use super::*;
    use nuxut_dbus::PropMap;

    fn props(pairs: &[(&str, Value<'_>)]) -> PropMap {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), OwnedValue::try_from(v.clone()).unwrap())).collect()
    }

    #[test]
    fn only_timezone_of_timedate1_counts() {
        let tz = props(&[("Timezone", Value::from("Europe/Istanbul"))]);
        assert_eq!(timezone_change(TIMEDATE1, &tz, &[]), Some(Some("Europe/Istanbul".into())));
        assert_eq!(timezone_change(TIMEDATE1, &PropMap::new(), &["Timezone".into()]), Some(None));
        // Başka özellik (NTP) ya da başka arayüz: dilim değişimi değil.
        assert_eq!(timezone_change(TIMEDATE1, &props(&[("NTP", Value::from(true))]), &[]), None);
        assert_eq!(timezone_change("org.example.Other", &tz, &[]), None);
        // Boş/çok uzun ya da yanlış türde ad: değişim yine bildirilir, ad yok/kesik.
        assert_eq!(timezone_change(TIMEDATE1, &props(&[("Timezone", Value::from(""))]), &[]), Some(None));
        assert_eq!(timezone_change(TIMEDATE1, &props(&[("Timezone", Value::from(7u32))]), &[]), Some(None));
        let long = "A".repeat(500);
        let got = timezone_change(TIMEDATE1, &props(&[("Timezone", Value::from(long.as_str()))]), &[]).unwrap().unwrap();
        assert_eq!(got.len(), MAX_ZONE);
    }
}
