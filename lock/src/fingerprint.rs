//! fprintd ile parmak izi (sistem bus'ı, `net.reactivated.Fprint`), şifreyle paralel.
//!
//! Akış: `Manager.GetDefaultDevice` → `ListEnrolledFingers(user)` (boşsa kapalı) →
//! `Claim(user)` → `VerifyStart("any")` → `VerifyStatus(result, done)` sinyalleri. `done` ise
//! yeniden taramadan önce `VerifyStop`. Eşleşme → [`FpEvent::Matched`]; `tries` başarısız
//! okumadan sonra durur. Durdurmada `VerifyStop` + `Release` (süreç ölürse fprintd cihazı
//! kendisi bırakır).
//!
//! Güvenlik (fail-closed): varsayılan sonuç hiçbir zaman "eşleşti" değildir. Bir
//! `VerifyStatus` ancak (1) fprintd'nin `Claim` öncesi kaydedilen **benzersiz** adından,
//! (2) `Claim` ettiğimiz cihaz yolundan ve (3) bu neslin `VerifyStart` yanıtından sonra
//! alınmışsa (`accept`) değerlendirilir; önceki taramanın ya da başka istemcinin eski
//! sinyali yok sayılır. Ad sahibi değişirse (fprintd düştü/yeniden başladı), akış biterse ya da
//! herhangi bir çağrı hata verirse sonuç [`FpEvent::Unavailable`]. [`Fingerprint::stop`]'tan
//! sonra hiçbir olay iletilmez. "Eşleşti" yalnızca bir istektir: kilidi açan, kilit durum
//! makinesinin tek açma kapısıdır (`state::Machine`, PAM ile aynı kapı, yarışta tek kez).

use std::sync::Arc;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use futures_lite::StreamExt;
use futures_lite::future;
use zbus::zvariant::OwnedObjectPath;
use zbus::{Connection, Proxy};

pub const FPRINT: &str = "net.reactivated.Fprint";
pub const MANAGER_PATH: &str = "/net/reactivated/Fprint/Manager";
pub const MANAGER: &str = "net.reactivated.Fprint.Manager";
pub const DEVICE: &str = "net.reactivated.Fprint.Device";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FpEvent {
    /// Tarama bekleniyor ("parmağınızı okutun").
    Scanning,
    /// Eşleşmedi; kalan deneme.
    NoMatch {
        remaining: u32,
    },
    /// Okuma kötü; aynı denemede yeniden okut.
    Retry(RetryReason),
    Matched,
    /// Deneme hakkı bitti; şifre kalır.
    Exhausted,
    /// fprintd/cihaz/kayıtlı parmak yok ya da hata; özellik kapalı.
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryReason {
    Scan,
    TooShort,
    NotCentered,
    RemoveAndRetry,
}

/// `VerifyStatus.result` sınıflandırması.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Match,
    NoMatch,
    Retry(RetryReason),
    Fatal,
}

pub fn classify(result: &str) -> Status {
    match result {
        "verify-match" => Status::Match,
        "verify-no-match" => Status::NoMatch,
        "verify-retry-scan" => Status::Retry(RetryReason::Scan),
        "verify-swipe-too-short" => Status::Retry(RetryReason::TooShort),
        "verify-finger-not-centered" => Status::Retry(RetryReason::NotCentered),
        "verify-remove-and-retry" => Status::Retry(RetryReason::RemoveAndRetry),
        // verify-disconnected, verify-unknown-error ve bilinmeyen her şey.
        _ => Status::Fatal,
    }
}

#[derive(Debug, Clone)]
pub struct FingerprintOptions {
    /// `None`: sistem bus'ı. Testte özel bus adresi.
    pub bus_address: Option<String>,
    pub user: String,
    /// 1..=10 başarısız okuma.
    pub tries: u32,
    pub call_timeout: Duration,
}

/// Çalışan parmak izi doğrulaması. Düşünce durur.
///
/// [`stop`](Self::stop)'tan sonra `sink` **hiç** çağrılmaz (thread o anda bir sonuç üretmiş
/// olsa bile): eski bir doğrulama oturumunun "eşleşti"si yeni bir kilide sızamaz. `sink`
/// bloklamamalı (kanala gönderim yeter): `stop` süren bir `sink` çağrısının bitmesini bekler.
#[derive(Debug)]
pub struct Fingerprint {
    stop: async_channel::Sender<()>,
    active: Arc<Mutex<bool>>,
    thread: Option<JoinHandle<()>>,
}

impl Fingerprint {
    pub fn spawn(
        options: FingerprintOptions,
        sink: impl Fn(FpEvent) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (stop, stop_rx) = async_channel::bounded(1);
        let active = Arc::new(Mutex::new(true));
        // Her olay kapıdan geçer: durdurulmuş oturumun olayı düşer.
        let sink = gated(Arc::clone(&active), sink);
        let thread = std::thread::Builder::new()
            .name("nuxut-fprint".into())
            .spawn(move || {
                let result = async_io::block_on(run(&options, &stop_rx, &sink));
                match result {
                    Ok(Some(event)) => sink(event),
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("parmak izi kullanılamıyor: {e}");
                        sink(FpEvent::Unavailable);
                    }
                }
            })?;
        Ok(Self {
            stop,
            active,
            thread: Some(thread),
        })
    }

    /// Doğrulamayı durdurur (`VerifyStop` + `Release`). Thread'i beklemez (yalnızca süren bir
    /// `sink` çağrısını); dönüşten sonra olay gelmez.
    pub fn stop(&mut self) {
        // Kilit, sürmekte olan bir `sink` çağrısının bitmesini bekler: dönüşten sonra olay yok.
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) = false;
        let _ = self.stop.try_send(());
        self.thread.take();
    }
}

impl Drop for Fingerprint {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `active` yanlışsa olayı düşüren sarmalayıcı. Kilit `sink` boyunca tutulur; böylece
/// denetim ile iletim arasında `stop` araya giremez.
fn gated(active: Arc<Mutex<bool>>, sink: impl Fn(FpEvent)) -> impl Fn(FpEvent) {
    move |e| {
        let active = active.lock().unwrap_or_else(|e| e.into_inner());
        if *active {
            sink(e);
        }
    }
}

/// Bir `VerifyStatus` sinyali kabul edilir mi? Hepsi gerekli:
/// - gönderen, `Claim` öncesi kaydedilen fprintd **benzersiz** adı (ad el değiştirdiyse zaten
///   durmuş oluruz),
/// - nesne yolu `Claim` ettiğimiz cihaz,
/// - sinyal bu neslin `VerifyStart` yanıtından **sonra** alınmış (aynı bağlantıdaki alış
///   sırası): önceki taramanın ya da başka bir istemcinin eski sinyali yok sayılır.
pub(crate) fn accept<P: Ord>(
    sender: Option<&str>,
    path: Option<&str>,
    received: P,
    owner: &str,
    device: &str,
    generation_start: Option<P>,
) -> bool {
    sender == Some(owner)
        && path == Some(device)
        && generation_start.is_some_and(|start| received > start)
}

enum Next {
    Signal(zbus::Message),
    OwnerChanged,
    Stop,
    Ended,
}

/// Son olay (varsa) döner; ara olaylar `sink`'e gider. Hata = `Unavailable` (asla eşleşme).
async fn run(
    options: &FingerprintOptions,
    stop: &async_channel::Receiver<()>,
    sink: &(impl Fn(FpEvent) + Send),
) -> zbus::Result<Option<FpEvent>> {
    let builder = match &options.bus_address {
        Some(addr) => zbus::connection::Builder::address(addr.as_str())?,
        None => zbus::connection::Builder::system()?,
    };
    let conn: Connection = builder.method_timeout(options.call_timeout).build().await?;
    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let owner = dbus.get_name_owner(FPRINT.try_into()?).await?.to_string();
    let owner_name: zbus::names::BusName<'static> =
        zbus::names::UniqueName::try_from(owner.clone())?.into();
    let manager = Proxy::new(&conn, owner_name.clone(), MANAGER_PATH, MANAGER).await?;
    let device_path: OwnedObjectPath = manager.call("GetDefaultDevice", &()).await?;
    // Proxy benzersiz ada bağlı: çağrılar yalnızca bu sahibe gider.
    let device = Proxy::new(&conn, owner_name.clone(), device_path.clone(), DEVICE).await?;

    let enrolled: Vec<String> = match device
        .call("ListEnrolledFingers", &(options.user.as_str(),))
        .await
    {
        Ok(v) => v,
        Err(_) => return Ok(Some(FpEvent::Unavailable)),
    };
    if enrolled.is_empty() {
        return Ok(Some(FpEvent::Unavailable));
    }

    // Sahiplik değişimi (fprintd yeniden başladı/düştü) → dur.
    let wk = Proxy::new(&conn, FPRINT, device_path.clone(), DEVICE).await?;
    let mut owner_changed = wk.receive_owner_changed().await?;
    let mut signals = device.receive_signal("VerifyStatus").await?;
    device
        .call::<_, _, ()>("Claim", &(options.user.as_str(),))
        .await?;
    let ctx = Verify {
        device: &device,
        owner: &owner,
        path: device_path.as_str(),
    };
    let outcome = ctx
        .run(
            &mut signals,
            &mut owner_changed,
            options.tries.clamp(1, 10),
            stop,
            sink,
        )
        .await;
    // Son olay bırakmadan önce iletilir: VerifyStop/Release zaman aşımı açmayı geciktirmez.
    let outcome = match outcome {
        Ok(Some(event)) => {
            sink(event);
            Ok(None)
        }
        other => other,
    };
    // Her yolda cihazı bırak; sonuçları önemsiz.
    let _ = device.call::<_, _, ()>("VerifyStop", &()).await;
    let _ = device.call::<_, _, ()>("Release", &()).await;
    outcome
}

struct Verify<'a> {
    device: &'a Proxy<'a>,
    owner: &'a str,
    path: &'a str,
}

impl Verify<'_> {
    async fn run(
        &self,
        signals: &mut zbus::proxy::SignalStream<'_>,
        owner_changed: &mut zbus::proxy::OwnerChangedStream<'_>,
        tries: u32,
        stop: &async_channel::Receiver<()>,
        sink: &(impl Fn(FpEvent) + Send),
    ) -> zbus::Result<Option<FpEvent>> {
        let mut failures = 0;
        'scan: loop {
            // Nesil: bu yanıttan önce alınan her sinyal eskidir.
            let reply = self.device.call_method("VerifyStart", &("any",)).await?;
            let generation = Some(reply.recv_position());
            sink(FpEvent::Scanning);
            loop {
                let next = future::or(
                    async { signals.next().await.map_or(Next::Ended, Next::Signal) },
                    future::or(
                        async {
                            owner_changed
                                .next()
                                .await
                                .map_or(Next::Ended, |_| Next::OwnerChanged)
                        },
                        async {
                            let _ = stop.recv().await;
                            Next::Stop
                        },
                    ),
                )
                .await;
                let msg = match next {
                    Next::Signal(msg) => msg,
                    Next::Stop => return Ok(None),
                    Next::OwnerChanged | Next::Ended => return Ok(Some(FpEvent::Unavailable)),
                };
                let header = msg.header();
                if !accept(
                    header.sender().map(|s| s.as_str()),
                    header.path().map(|p| p.as_str()),
                    msg.recv_position(),
                    self.owner,
                    self.path,
                    generation,
                ) {
                    tracing::debug!("parmak izi: kabul edilmeyen VerifyStatus yok sayıldı");
                    continue;
                }
                let Ok((result, done)) = msg.body().deserialize::<(String, bool)>() else {
                    continue;
                };
                match classify(&result) {
                    Status::Match => return Ok(Some(FpEvent::Matched)),
                    Status::NoMatch => {
                        failures += 1;
                        if failures >= tries {
                            return Ok(Some(FpEvent::Exhausted));
                        }
                        sink(FpEvent::NoMatch {
                            remaining: tries - failures,
                        });
                    }
                    Status::Retry(reason) => sink(FpEvent::Retry(reason)),
                    Status::Fatal => return Ok(Some(FpEvent::Unavailable)),
                }
                if done {
                    // Yeni nesil: bu taramanın kalan (geç) sinyalleri bir sonrakinde eskidir.
                    self.device.call::<_, _, ()>("VerifyStop", &()).await?;
                    continue 'scan;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_requires_owner_device_and_generation() {
        let ok = |s, p, r, g| accept(s, p, r, ":1.5", "/dev/0", g);
        assert!(ok(Some(":1.5"), Some("/dev/0"), 11u64, Some(10)));
        // Başka gönderici (sahte ya da eski fprintd sahibi).
        assert!(!ok(Some(":1.9"), Some("/dev/0"), 11, Some(10)));
        assert!(!ok(None, Some("/dev/0"), 11, Some(10)));
        // Başka cihaz yolu.
        assert!(!ok(Some(":1.5"), Some("/dev/1"), 11, Some(10)));
        assert!(!ok(Some(":1.5"), None, 11, Some(10)));
        // VerifyStart yanıtından önce ya da aynı anda alınmış: eski nesil.
        assert!(!ok(Some(":1.5"), Some("/dev/0"), 9, Some(10)));
        assert!(!ok(Some(":1.5"), Some("/dev/0"), 10, Some(10)));
        // Nesil yok (VerifyStart yapılmadı).
        assert!(!ok(Some(":1.5"), Some("/dev/0"), 11, None));
    }

    #[test]
    fn gate_drops_events_after_stop() {
        let active = Arc::new(Mutex::new(true));
        let seen = std::sync::Mutex::new(Vec::new());
        let sink = gated(Arc::clone(&active), |e| seen.lock().unwrap().push(e));
        sink(FpEvent::Scanning);
        *active.lock().unwrap() = false;
        sink(FpEvent::Matched);
        assert_eq!(*seen.lock().unwrap(), [FpEvent::Scanning]);
    }

    #[test]
    fn classification() {
        assert_eq!(classify("verify-match"), Status::Match);
        assert_eq!(classify("verify-no-match"), Status::NoMatch);
        assert_eq!(
            classify("verify-retry-scan"),
            Status::Retry(RetryReason::Scan)
        );
        for fatal in [
            "verify-disconnected",
            "verify-unknown-error",
            "",
            "verify-match ",
            "VERIFY-MATCH",
        ] {
            assert_eq!(classify(fatal), Status::Fatal, "{fatal:?}");
        }
    }
}
