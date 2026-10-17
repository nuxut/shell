//! Sistem durumu (ses, ağ, Bluetooth, güç, parlaklık, medya): servis olaylarından beslenen
//! [`SystemModel`], hızlı ayar widget'larının ürettiği [`SystemCommand`]'lar ve servislerin
//! yaşam döngüsü ([`SystemServices`]).
//!
//! Akış: servis thread'leri olayları [`SystemEvent`] olarak sink'e verir → `App` ana döngüde
//! [`SystemServices::on_event`] çağırır (modeli günceller, yan etkileri — kapak isteme/çözme,
//! cevapsız kalacak NM/BlueZ ajan isteklerini reddetme — burada yapılır) → widget'lar modeli okur,
//! komut döndürür → `App` [`SystemServices::command`] ile uygular. Bir servisin başlamaması
//! shell'i durdurmaz; o bölüm "kullanılamıyor" görünür.

use std::sync::Arc;

use nuxut_image::{DecodeOptions, Fit, IsolatedDecoder, Limits, LoadRequest, Loader, Source, TargetSize, Trust};

use nuxut::i18n::t;
use crate::services::audio::{AudioBackend, AudioEvent, AudioOptions, AudioService, AudioState, AudioTarget, DiagnoseReply, SummaryReply};
use crate::services::bluetooth::{
    BluetoothEvent, BluetoothOptions, BluetoothService, BluetoothState, BluezError, PairingKind, PairingReply,
    PairingRequest,
};
use crate::services::brightness::{BrightnessEvent, BrightnessOptions, BrightnessService, Display, DisplayKind};
use crate::services::media::{
    ArtSource, MAX_ART_BYTES, MediaAction, MediaEvent, MediaOptions, MediaService, Player, active_player,
};
use crate::services::network::{
    NetworkEvent, NetworkOptions, NetworkService, NetworkState, NmError, Secret, SecretRequest, TEST_AGENT_ENV,
    agent_id_for,
};
use crate::services::power::{PowerError, PowerEvent, PowerOptions, PowerProfile, PowerService, PowerState};

/// Çözülmüş kapağın kenarı (fiziksel piksel; kart en fazla bunu gösterir).
pub const ART_PX: u32 = 256;
/// Hata iletisi sınırı (servis iletileri dış kaynaklı olabilir).
const MAX_ERROR: usize = 160;

/// Servislerden gelen olaylar (tek kanal).
#[derive(Debug)]
pub enum SystemEvent {
    Audio(AudioEvent),
    Network(NetworkEvent),
    Bluetooth(BluetoothEvent),
    Power(PowerEvent),
    Brightness(BrightnessEvent),
    Media(MediaEvent),
    /// Kapak çözüldü (`None`: çözülemedi).
    ArtDecoded { url: String, image: Option<ArtImage> },
}

/// Çözülmüş kapak: önceden çarpılmış RGBA (`ImageData::pixels`).
#[derive(Clone, PartialEq, Eq)]
pub struct ArtImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

impl std::fmt::Debug for ArtImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ArtImage({}x{})", self.width, self.height)
    }
}

/// Hatanın kaynağı (kartta yalnızca ilgili widget gösterir).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSource {
    Audio,
    Network,
    Bluetooth,
    Power,
    Brightness,
    Media,
}

/// Medya düğmeleri (`MediaAction`'ın UI'dan gelebilen, karşılaştırılabilir alt kümesi).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaControl {
    PlayPause,
    Next,
    Previous,
    /// Oynatıcı penceresini öne getir.
    Raise,
}

impl From<MediaControl> for MediaAction {
    fn from(c: MediaControl) -> Self {
        match c {
            MediaControl::PlayPause => MediaAction::PlayPause,
            MediaControl::Next => MediaAction::Next,
            MediaControl::Previous => MediaAction::Previous,
            MediaControl::Raise => MediaAction::Raise,
        }
    }
}

/// Hızlı ayar widget'larından çıkan komutlar. Seviyeler tam sayı (binde / yüzde) ki
/// `Action` karşılaştırılabilir kalsın.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemCommand {
    /// Algısal seviye, binde (1000 = %100).
    SetVolume { target: AudioTarget, permille: u16 },
    ToggleMute(AudioTarget),
    SetDefaultAudio(u32),
    SetWifi(bool),
    /// Erişim noktasına bağlan (`AccessPoint::path`); kayıtlı/açık ağda şifre yok.
    ConnectAccessPoint { path: String, password: Option<Secret> },
    /// NM ajanının şifre isteğine cevap (`None`: vazgeç).
    ProvideSecret { id: u64, secret: Option<Secret> },
    /// Etkin bağlantıyı kes (`ActiveConnection::path`).
    Deactivate(String),
    SetBluetooth(bool),
    BluetoothConnect(String),
    BluetoothDisconnect(String),
    /// Eşleştirme isteği cevabı (yalnızca kabul/ret; PIN girişi ayarlar uygulamasında).
    PairingReply { id: u64, accept: bool },
    SetPowerProfile(PowerProfile),
    /// Pil şarj sınırı aç/kapa (güç kartı anahtarı).
    SetChargeLimit(bool),
    /// `Display::id`, yüzde.
    SetBrightness { id: String, percent: u8 },
    Media { player: String, control: MediaControl },
    /// Hata satırını kapat.
    DismissError(ErrorSource),
    // --- OSD tuşları ve `nuxut-shell msg volume|mic|brightness`
    /// Ses/mikrofon seviyesi, yüzde (`Set` 0..=150'ye sınırlanır; servis ayrıca
    /// `[audio] max_volume`'a sınırlar).
    Volume { target: AudioTarget, level: Level },
    SetMute { target: AudioTarget, mute: Mute },
    /// Parlaklık, yüzde (0..=100; dahili panelin tabanı serviste). `id`: `Display::id`.
    Brightness { id: String, level: Level },
}

/// Seviye değişimi (yüzde puanı).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Set(u16),
    /// Göreli (+5 / -5).
    Step(i16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mute {
    On,
    Off,
    Toggle,
}

/// Ses seviyesi üst sınırı (yüzde): `[audio] max_volume` en fazla bu kadardır.
pub const MAX_VOLUME_PERCENT: u16 = 150;

/// Sistem durumu. Servisler tam anlık görüntü gönderir; model son geçerli durumu tutar.
#[derive(Debug, Clone)]
pub struct SystemModel {
    /// Ses kaydırıcısının üst sınırı (algısal; 1.0 = %100), `AudioOptions::max_volume`.
    pub max_volume: f32,
    /// `[media] remote_art`: kapalıyken ağ kapağı hiç istenmez.
    pub remote_art: bool,
    pub audio: AudioState,
    pub network: NetworkState,
    pub bluetooth: BluetoothState,
    pub power: PowerState,
    /// Parlaklığı ayarlanabilen ekranlar.
    pub displays: Vec<Display>,
    pub players: Vec<Player>,
    /// Etkin oynatıcının çözülmüş kapağı (adresiyle).
    pub art: Option<(String, ArtImage)>,
    /// NM'nin beklediği şifre (yanlış/eksik şifre).
    pub secret_request: Option<SecretRequest>,
    /// BlueZ ajanının cevap beklediği istek.
    pub pairing: Option<PairingRequest>,
    /// Son hata (kaynağına göre ilgili widget gösterir).
    pub error: Option<(ErrorSource, String)>,
    /// Güç kartından gelen, sonucu beklenen şarj sınırı istekleri (`PowerService` kimliği).
    /// `msg` istekleri burada değil (yanıtları `app/power.rs`); yalnızca bunların hatası kartta.
    pub charge_requests: Vec<u64>,
}

impl Default for SystemModel {
    fn default() -> Self {
        Self::new(&SystemOptions::default())
    }
}

impl SystemModel {
    pub fn new(options: &SystemOptions) -> Self {
        Self {
            max_volume: options.audio.max_volume,
            remote_art: options.remote_art,
            audio: AudioState::default(),
            network: NetworkState::default(),
            bluetooth: BluetoothState::default(),
            power: PowerState::default(),
            displays: Vec::new(),
            players: Vec::new(),
            art: None,
            secret_request: None,
            pairing: None,
            error: None,
            charge_requests: Vec::new(),
        }
    }

    /// Etkin oynatıcı ([`active_player`]).
    pub fn player(&self) -> Option<&Player> {
        active_player(&self.players)
    }

    /// Etkin oynatıcının kapak adresi (yerel ya da ağ; indirme kuralları medya servisinde).
    /// `remote_art` kapalıyken ağ kapağı istenmez: UI yer tutucu gösterir.
    pub fn wanted_art(&self) -> Option<String> {
        let art = self.player()?.metadata.art.as_ref()?;
        if matches!(art, ArtSource::Remote(_)) && !self.remote_art {
            return None;
        }
        Some(art.url()).filter(|u| !u.is_empty())
    }

    /// Etkin oynatıcının kapağı (çözülmüşse ve hâlâ geçerliyse).
    pub fn art_image(&self) -> Option<&ArtImage> {
        let wanted = self.wanted_art()?;
        self.art.as_ref().filter(|(u, _)| *u == wanted).map(|(_, i)| i)
    }

    /// `msg brightness` / OSD tuşunun hedefi: çıktısı `output` olan ekran; yoksa birincil
    /// arka ışık (ilk dahili); o da yoksa ve çıktı verilmemişse ilk ekran.
    pub fn brightness_display(&self, output: Option<&str>) -> Option<&Display> {
        let exact = output.and_then(|o| self.displays.iter().find(|d| d.output.as_deref() == Some(o)));
        exact
            .or_else(|| self.displays.iter().find(|d| d.kind == DisplayKind::Internal))
            .or_else(|| if output.is_none() { self.displays.first() } else { None })
    }

    pub fn error_for(&self, source: ErrorSource) -> Option<&str> {
        self.error.as_ref().filter(|(s, _)| *s == source).map(|(_, m)| m.as_str())
    }

    fn set_error(&mut self, source: ErrorSource, message: impl AsRef<str>) {
        self.error = Some((source, truncate(message.as_ref(), MAX_ERROR)));
    }

    /// Olayı uygular (yan etkisiz). Görünür durum değiştiyse `true`.
    pub fn apply(&mut self, event: &SystemEvent) -> bool {
        match event {
            SystemEvent::Audio(e) => match e {
                AudioEvent::State(s) => replace(&mut self.audio, s),
                // OSD ayrı yüzeyin işi; durum zaten `State` ile gelir.
                AudioEvent::Osd(_) => false,
                AudioEvent::Failed { message, .. } => {
                    self.set_error(ErrorSource::Audio, message);
                    true
                }
            },
            SystemEvent::Network(e) => match e {
                NetworkEvent::State(s) => replace(&mut self.network, s),
                NetworkEvent::SecretRequest(r) => {
                    self.secret_request = Some(r.clone());
                    true
                }
                NetworkEvent::SecretDone { id } => {
                    let done = self.secret_request.as_ref().is_some_and(|r| r.id == *id);
                    if done {
                        self.secret_request = None;
                    }
                    done
                }
                NetworkEvent::Failed { error, .. } => {
                    self.set_error(ErrorSource::Network, network_error_text(error));
                    true
                }
                // Ayarlar'a özgü olaylar (aşama 2 yazma API'leri); kabuk kullanmaz.
                // Ayarlar'a özgü ve gelecekteki olaylar (olay enum'u non_exhaustive).
                _ => false,
            },
            SystemEvent::Bluetooth(e) => match e {
                BluetoothEvent::State(s) => replace(&mut self.bluetooth, s),
                BluetoothEvent::Pairing(r) => {
                    self.pairing = Some(r.clone());
                    true
                }
                BluetoothEvent::PairingDone { id } => {
                    let done = self.pairing.as_ref().is_some_and(|r| r.id == *id);
                    if done {
                        self.pairing = None;
                    }
                    done
                }
                BluetoothEvent::Failed { error, .. } => {
                    self.set_error(ErrorSource::Bluetooth, bluetooth_error_text(error));
                    true
                }
                // İstek kimlikli başarı olayı (ve gelecekteki olaylar) Ayarlar içindir; kabuk
                // durumdan izler.
                _ => false,
            },
            SystemEvent::Power(e) => match e {
                PowerEvent::Changed(s) => replace(&mut self.power, s),
                // Düşük pil bildirimi bildirim merkezinin işi.
                PowerEvent::BatteryWarning(_) => false,
                PowerEvent::ProfileDone { result: Ok(()), .. } => false,
                PowerEvent::ProfileDone { result: Err(e), .. } => {
                    self.set_error(ErrorSource::Power, profile_error_text(e));
                    true
                }
                // Kartın isteğiyse hata kartta; `msg` isteğinin yanıtı `app/power.rs`'te.
                PowerEvent::ChargeLimitDone { id, result } => {
                    let Some(i) = self.charge_requests.iter().position(|r| r == id) else { return false };
                    self.charge_requests.remove(i);
                    match result {
                        Ok(()) => false,
                        Err(e) => {
                            self.set_error(ErrorSource::Power, charge_error_text(e));
                            true
                        }
                    }
                }
            },
            SystemEvent::Brightness(e) => match e {
                BrightnessEvent::Displays(d) => replace(&mut self.displays, d),
                BrightnessEvent::Changed(d) => match self.displays.iter_mut().find(|x| x.id == d.id) {
                    Some(x) if x != d => {
                        *x = d.clone();
                        true
                    }
                    Some(_) => false,
                    None => {
                        self.displays.push(d.clone());
                        true
                    }
                },
                BrightnessEvent::Failed { message, .. } => {
                    self.set_error(ErrorSource::Brightness, message);
                    true
                }
            },
            SystemEvent::Media(e) => match e {
                MediaEvent::Players(p) => replace(&mut self.players, p),
                // Baytlar `SystemServices` tarafından çözülür; model yalnızca sonucu tutar.
                MediaEvent::Art { .. } => false,
                MediaEvent::Failed { message, .. } => {
                    self.set_error(ErrorSource::Media, message);
                    true
                }
            },
            SystemEvent::ArtDecoded { url, image } => match image {
                Some(i) => {
                    self.art = Some((url.clone(), i.clone()));
                    true
                }
                None => false,
            },
        }
    }
}

/// Kartta yanıt bekleyen en fazla şarj sınırı isteği (servis hiç yanıtlamasa da büyümez).
const MAX_CHARGE_REQUESTS: usize = 8;

/// Şarj sınırı hatası → kart metni.
pub fn charge_error_text(e: &PowerError) -> String {
    match e {
        PowerError::Denied => t!("shell-power-error-charge-denied"),
        PowerError::Unsupported => t!("shell-power-error-charge-unsupported"),
        PowerError::Unavailable => t!("shell-power-error-charge-unavailable"),
        PowerError::Failed(name) => t!("shell-power-error-charge-failed", detail = name.as_str()),
    }
}

/// Profil yazma hatası → kart metni (profil yalnızca karttan değişir; `msg` karşılığı yok).
pub fn profile_error_text(e: &PowerError) -> String {
    match e {
        PowerError::Unavailable => t!("shell-power-error-daemon-unavailable"),
        PowerError::Failed(name) => t!("shell-power-error-profile-set", detail = name.as_str()),
        other => t!("shell-power-error-profile-set", detail = other.code()),
    }
}

fn replace<T: Clone + PartialEq>(slot: &mut T, new: &T) -> bool {
    if slot == new {
        return false;
    }
    slot.clone_from(new);
    true
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

// ---------------------------------------------------------------------------------------
// Paylaşılan istemcilerin (nuxut-nm, nuxut-bluez) hata türleri → kullanıcı metni. Crate'ler
// UI'sızdır; metin yalnızca burada, tek yerde üretilir.

/// [`NmError`] → kartın hata satırı (`shell-network-error-*`).
fn network_error_text(error: &NmError) -> String {
    match error {
        NmError::CallFailed(detail) => t!("shell-network-error-call-failed", detail = detail.clone()),
        NmError::NmUnavailable => t!("shell-network-error-nm-unavailable"),
        NmError::ConnectionUnknown => t!("shell-network-error-connection-unknown"),
        NmError::ConnectionInactive => t!("shell-network-error-connection-inactive"),
        NmError::ApUnknown => t!("shell-network-error-ap-unknown"),
        NmError::Enterprise => t!("shell-network-error-enterprise"),
        NmError::InvalidPassword => t!("shell-network-error-invalid-password"),
        NmError::NoWifiDevice => t!("shell-network-error-no-wifi-device"),
        NmError::InvalidSsid => t!("shell-network-error-invalid-ssid"),
        NmError::DeviceUnknown => t!("shell-network-error-device-unknown"),
        NmError::Denied => t!("shell-network-error-denied"),
        NmError::Unsupported => t!("shell-network-error-unsupported"),
        NmError::SecretNotSaved => t!("shell-network-error-secret-not-saved"),
        // Gelecekteki türler (non_exhaustive): genel satır, ayrıntı Display.
        other => t!("shell-network-error-call-failed", detail = other.to_string()),
    }
}

/// [`BluezError`] → kartın hata satırı (`shell-bluetooth-error-*`).
fn bluetooth_error_text(error: &BluezError) -> String {
    match error {
        BluezError::CallFailed(detail) => t!("shell-bluetooth-error-call-failed", detail = detail.clone()),
        BluezError::Timeout => t!("shell-bluetooth-error-timeout"),
        BluezError::NoAdapter => t!("shell-bluetooth-error-no-adapter"),
        BluezError::BluezUnavailable => t!("shell-bluetooth-error-bluez-unavailable"),
        BluezError::UnknownDevice => t!("shell-bluetooth-error-unknown-device"),
        BluezError::AuthenticationRejected => t!("shell-bluetooth-error-auth-rejected"),
        BluezError::AuthenticationFailed => t!("shell-bluetooth-error-auth-failed"),
        BluezError::AuthenticationCanceled => t!("shell-bluetooth-error-auth-canceled"),
        BluezError::AuthenticationTimeout => t!("shell-bluetooth-error-auth-timeout"),
        BluezError::ConnectionAttemptFailed => t!("shell-bluetooth-error-connection-attempt-failed"),
        BluezError::AlreadyExists => t!("shell-bluetooth-error-already-exists"),
        BluezError::InProgress => t!("shell-bluetooth-error-in-progress"),
        BluezError::NotReady => t!("shell-bluetooth-error-not-ready"),
        BluezError::InvalidValue => t!("shell-bluetooth-error-invalid-value"),
        BluezError::RfkillBlocked { hard: Some(true) } => t!("shell-bluetooth-error-rfkill-hard"),
        BluezError::RfkillBlocked { .. } => t!("shell-bluetooth-error-rfkill"),
        // Gelecekteki türler (non_exhaustive): genel satır, ayrıntı Display.
        other => t!("shell-bluetooth-error-call-failed", detail = other.to_string()),
    }
}

/// Servis seçenekleri (config'ten).
#[derive(Debug, Clone)]
pub struct SystemOptions {
    pub audio: AudioOptions,
    /// `[media] remote_art`: ağ kapakları istensin mi.
    pub remote_art: bool,
}

impl Default for SystemOptions {
    fn default() -> Self {
        Self { audio: AudioOptions::default(), remote_art: true }
    }
}

/// Config'ten servis seçenekleri.
pub fn system_options(config: &crate::config::ShellConfig) -> SystemOptions {
    SystemOptions {
        audio: AudioOptions {
            max_volume: AudioOptions::max_volume_from_percent(config.audio.max_volume),
            backend: config.audio.backend,
            ..AudioOptions::default()
        },
        remote_art: config.media.remote_art,
    }
}

type Sink = Arc<dyn Fn(SystemEvent) + Send + Sync>;

/// Çalışan sistem servisleri. Düşürülünce hepsi temiz kapanır.
pub struct SystemServices {
    audio: Option<AudioService>,
    /// Ses servisinin yazım yolu (`[audio] backend` değişince yeniden başlatmak için).
    audio_backend: AudioBackend,
    network: Option<NetworkService>,
    bluetooth: Option<BluetoothService>,
    power: Option<PowerService>,
    brightness: Option<BrightnessService>,
    media: Option<MediaService>,
    /// Kapak çözücü (tek thread; ilk istekte başlar).
    art_loader: Loader,
    /// Son istenen kapak (aynı adres yeniden istenmez).
    art_requested: Option<String>,
    sink: Sink,
}

impl std::fmt::Debug for SystemServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemServices")
            .field("audio", &self.audio.is_some())
            .field("network", &self.network.is_some())
            .field("bluetooth", &self.bluetooth.is_some())
            .field("power", &self.power.is_some())
            .field("brightness", &self.brightness.is_some())
            .field("media", &self.media.is_some())
            .finish_non_exhaustive()
    }
}

/// Kabuğun ağ seçenekleri: crate varsayılanı ajansızdır, kabuk şifre ajanını açıkça ister.
/// Sistem bus'ı kullanıcıyla ortak: test örneği kullanıcının NM ajanını engellemesin
/// ([`agent_id_for`]).
fn network_options(instance: Option<&str>, opt_in: bool) -> NetworkOptions {
    match agent_id_for(instance, opt_in) {
        Some(id) => NetworkOptions::with_agent(id),
        None => NetworkOptions::default(),
    }
}

/// Kabuğun Bluetooth seçenekleri: eşleştirme ajanı açıkça (crate varsayılanı ajansız).
fn bluetooth_options() -> BluetoothOptions {
    BluetoothOptions::with_agent()
}

/// Servisi başlatır; başlamazsa uyarı ve `None`.
fn start<S, E, F>(name: &str, sink: &Sink, wrap: fn(E) -> SystemEvent, spawn: F) -> Option<S>
where
    E: 'static,
    F: FnOnce(Box<dyn Fn(E) + Send>) -> std::io::Result<S>,
{
    let sink = Arc::clone(sink);
    match spawn(Box::new(move |e| sink(wrap(e)))) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("{name} servisi başlatılamadı: {e}");
            None
        }
    }
}

impl SystemServices {
    /// Servisleri başlatır (`nuxut::run` başladıktan sonra çağrılmalı; bkz. `services`).
    pub fn spawn(options: SystemOptions, sink: impl Fn(SystemEvent) + Send + Sync + 'static) -> Self {
        let sink: Sink = Arc::new(sink);
        Self {
            audio_backend: options.audio.backend,
            audio: start("ses", &sink, SystemEvent::Audio, |f| AudioService::spawn(options.audio, f)),
            network: start("ağ", &sink, SystemEvent::Network, |f| {
                let instance = std::env::var(crate::ipc::INSTANCE_ENV).ok();
                let opt_in = std::env::var(TEST_AGENT_ENV).is_ok_and(|v| v == "1");
                NetworkService::spawn(network_options(instance.as_deref(), opt_in), f)
            }),
            bluetooth: start("Bluetooth", &sink, SystemEvent::Bluetooth, |f| {
                BluetoothService::spawn(bluetooth_options(), f)
            }),
            power: start("güç", &sink, SystemEvent::Power, |f| PowerService::spawn(PowerOptions::default(), f)),
            brightness: start("parlaklık", &sink, SystemEvent::Brightness, |f| {
                BrightnessService::spawn(BrightnessOptions::default(), f)
            }),
            media: start("medya", &sink, SystemEvent::Media, |f| MediaService::spawn(MediaOptions::default(), f)),
            art_loader: Loader::new(1),
            art_requested: None,
            sink,
        }
    }

    /// Kapak çözücüsüne shell'in paylaşılan Sandbox'ını bağlar (ADR-0007): kapak baytları
    /// oynatıcıdan ya da ağdan gelir, ayrı kısıtlı işçide çözülür. Bağlanmazsa bugünkü davranış
    /// (SVG reddi, raster süreç içinde).
    #[must_use]
    pub fn with_art_sandbox(mut self, sandbox: Arc<dyn IsolatedDecoder>) -> Self {
        self.art_loader = std::mem::take(&mut self.art_loader).with_sandbox(sandbox);
        self
    }

    /// Olayı modele uygular ve yan etkilerini yapar. Görünür durum değiştiyse `true`.
    pub fn on_event(&mut self, event: SystemEvent, model: &mut SystemModel) -> bool {
        let changed = model.apply(&event);
        match event {
            SystemEvent::Media(MediaEvent::Art { url, result: Ok(bytes) })
                if model.wanted_art().as_deref() == Some(url.as_str()) =>
            {
                // Kaynak anahtarı etkin oynatıcının bus adı (yoksa adres): oynatıcı başına tek iş.
                let player = model.player().map_or_else(|| url.clone(), |p| p.bus_name.clone());
                self.decode_art(url, bytes, &player);
            }
            // PIN/passkey girişi (ve kartın tanımadığı yeni istek türleri) kartta yok (ayarlar
            // uygulamasının işi): beklemeden reddet.
            SystemEvent::Bluetooth(BluetoothEvent::Pairing(r)) if !card_handles(&r.kind) => {
                if let Some(b) = &self.bluetooth {
                    b.reply(r.id, PairingReply::Reject);
                }
                model.pairing = None;
                model.set_error(ErrorSource::Bluetooth, t!("shell-bluetooth-error-pin-required"));
            }
            _ => {}
        }
        self.sync_art(model);
        changed
    }

    /// Etkin oynatıcının kapağı henüz istenmediyse ister.
    fn sync_art(&mut self, model: &SystemModel) {
        let wanted = model.wanted_art();
        if wanted == self.art_requested {
            return;
        }
        self.art_requested.clone_from(&wanted);
        if let (Some(url), Some(media)) = (wanted, &self.media)
            && model.art.as_ref().is_none_or(|(u, _)| *u != url)
        {
            media.fetch_art(&url);
        }
    }

    fn decode_art(&mut self, url: String, bytes: Arc<[u8]>, player: &str) {
        let options = DecodeOptions {
            // Kapak baytları oynatıcıdan ya da ağdan gelir: her zaman güvenilmez (Sandbox işçisinde
            // çözülür; işçi yoksa SVG reddedilir, ADR-0007 K2-b).
            limits: Limits { max_file_bytes: MAX_ART_BYTES, trust: Trust::Untrusted, ..Limits::ICON },
            target: Some(TargetSize::new(ART_PX, ART_PX, Fit::Cover)),
            animate: false,
        };
        let sink = Arc::clone(&self.sink);
        let key = crate::services::image_sandbox::source_key(crate::services::image_sandbox::KeyDomain::Media, player);
        // Hata (yalıtım dahil) → `image: None`: kart yer tutucuyu gösterir.
        self.art_loader.load_keyed(key, LoadRequest { source: Source::Bytes(bytes), options }, move |result| {
            let image = result.ok().and_then(|d| {
                let image = d.into_first_frame();
                let (width, height) = image.size();
                (width > 0 && height > 0).then(|| ArtImage { width, height, pixels: image.into_pixels().into() })
            });
            sink(SystemEvent::ArtDecoded { url, image });
        });
    }

    /// Komutu ilgili servise iletir. Servis yoksa uyarı ve `false`.
    pub fn command(&self, cmd: SystemCommand, model: &mut SystemModel) -> bool {
        fn missing(what: &str) -> bool {
            tracing::warn!("{what} servisi yok, komut yapılamadı");
            false
        }
        match cmd {
            SystemCommand::SetVolume { target, permille } => match &self.audio {
                Some(a) => {
                    a.set_volume(target, f32::from(permille) / 1000.0);
                }
                None => return missing("ses"),
            },
            SystemCommand::ToggleMute(target) => match &self.audio {
                Some(a) => {
                    a.toggle_muted(target);
                }
                None => return missing("ses"),
            },
            SystemCommand::SetDefaultAudio(id) => match &self.audio {
                Some(a) => {
                    a.set_default(id);
                }
                None => return missing("ses"),
            },
            SystemCommand::SetWifi(on) => match &self.network {
                Some(n) => {
                    n.set_wifi_enabled(on);
                }
                None => return missing("ağ"),
            },
            SystemCommand::ConnectAccessPoint { path, password } => match &self.network {
                Some(n) => {
                    n.connect_access_point(&path, password);
                }
                None => return missing("ağ"),
            },
            SystemCommand::ProvideSecret { id, secret } => {
                if model.secret_request.as_ref().is_some_and(|r| r.id == id) {
                    model.secret_request = None;
                }
                match &self.network {
                    Some(n) => {
                        n.provide_secret(id, secret);
                    }
                    None => return missing("ağ"),
                }
            }
            SystemCommand::Deactivate(path) => match &self.network {
                Some(n) => {
                    n.deactivate(&path);
                }
                None => return missing("ağ"),
            },
            SystemCommand::SetBluetooth(on) => match &self.bluetooth {
                Some(b) => {
                    b.set_powered(on);
                }
                None => return missing("Bluetooth"),
            },
            SystemCommand::BluetoothConnect(d) => match &self.bluetooth {
                Some(b) => {
                    b.connect(&d);
                }
                None => return missing("Bluetooth"),
            },
            SystemCommand::BluetoothDisconnect(d) => match &self.bluetooth {
                Some(b) => {
                    b.disconnect(&d);
                }
                None => return missing("Bluetooth"),
            },
            SystemCommand::PairingReply { id, accept } => {
                if model.pairing.as_ref().is_some_and(|r| r.id == id) {
                    model.pairing = None;
                }
                match &self.bluetooth {
                    Some(b) => {
                        b.reply(id, if accept { PairingReply::Accept } else { PairingReply::Reject });
                    }
                    None => return missing("Bluetooth"),
                }
            }
            SystemCommand::SetPowerProfile(p) => match &self.power {
                Some(s) => {
                    s.set_profile(p);
                }
                None => return missing("güç"),
            },
            SystemCommand::SetChargeLimit(on) => match self.set_charge_limit(on) {
                Some(id) => {
                    if model.charge_requests.len() >= MAX_CHARGE_REQUESTS {
                        model.charge_requests.remove(0);
                    }
                    model.charge_requests.push(id);
                }
                None => {
                    model.set_error(ErrorSource::Power, charge_error_text(&PowerError::Unavailable));
                    return false;
                }
            },
            SystemCommand::SetBrightness { id, percent } => match &self.brightness {
                Some(b) => b.set(&id, f64::from(percent.min(100))),
                None => return missing("parlaklık"),
            },
            SystemCommand::Media { player, control } => match &self.media {
                Some(m) => {
                    m.action(&player, control.into());
                }
                None => return missing("medya"),
            },
            SystemCommand::DismissError(source) => {
                if model.error.as_ref().is_some_and(|(s, _)| *s == source) {
                    model.error = None;
                }
            }
            SystemCommand::Volume { target, level } => match &self.audio {
                Some(a) => {
                    match level {
                        Level::Set(p) => a.set_volume(target, f32::from(p.min(MAX_VOLUME_PERCENT)) / 100.0),
                        Level::Step(d) => a.adjust_volume(target, f32::from(d) / 100.0),
                    };
                }
                None => return missing("ses"),
            },
            SystemCommand::SetMute { target, mute } => match &self.audio {
                Some(a) => {
                    match mute {
                        Mute::On => a.set_muted(target, true),
                        Mute::Off => a.set_muted(target, false),
                        Mute::Toggle => a.toggle_muted(target),
                    };
                }
                None => return missing("ses"),
            },
            SystemCommand::Brightness { id, level } => match &self.brightness {
                Some(b) => match level {
                    Level::Set(p) => b.set(&id, f64::from(p.min(100))),
                    Level::Step(d) => b.adjust(&id, f64::from(d)),
                },
                None => return missing("parlaklık"),
            },
        }
        true
    }

    /// Çalışan ses servisinin yazım yolu.
    pub fn audio_backend(&self) -> AudioBackend {
        self.audio_backend
    }

    /// Pil şarj sınırını aç/kapat (`msg power charge-limit`). Sonuç
    /// `PowerEvent::ChargeLimitDone` ile dönen kimlikle gelir; servis yok/meşgulse `None`.
    pub fn set_charge_limit(&self, on: bool) -> Option<u64> {
        self.power.as_ref().and_then(|p| p.set_charge_limit(on))
    }

    /// `msg audio-diagnose`: rapor `reply`'a ses servisinden gelir. Servis yoksa `false`.
    pub fn audio_diagnose(&self, reply: DiagnoseReply) -> bool {
        self.audio.as_ref().is_some_and(|a| a.diagnose(reply))
    }

    /// `msg doctor`: ses özeti `reply`'a ses thread'inden gelir. Servis yoksa `false`.
    pub fn audio_summary(&self, reply: SummaryReply) -> bool {
        self.audio.as_ref().is_some_and(|a| a.summary(reply))
    }

    /// `[media] remote_art` değişti: açılınca etkin oynatıcının ağ kapağı hemen istenir,
    /// kapanınca istenmez (gösterilen kapak yer tutucuya döner).
    pub fn set_remote_art(&mut self, on: bool, model: &mut SystemModel) {
        model.remote_art = on;
        self.sync_art(model);
    }

    /// Ses seçenekleri değişti (`[audio] max_volume`): ses servisi yeni seçeneklerle yeniden
    /// başlar (eskisi düşürülünce temiz kapanır).
    pub fn set_audio_options(&mut self, options: AudioOptions, model: &mut SystemModel) {
        model.max_volume = options.max_volume;
        self.audio_backend = options.backend;
        self.audio = None;
        self.audio = start("ses", &self.sink, SystemEvent::Audio, |f| AudioService::spawn(options, f));
    }
}

/// Hızlı kartın gösterip cevaplayabildiği eşleştirme isteği mi (onay/izin/kod gösterimi)?
/// PIN/passkey girişi ve tanınmayan yeni türler kartta yok: beklemeden reddedilir.
fn card_handles(kind: &PairingKind) -> bool {
    matches!(
        kind,
        PairingKind::Confirm { .. }
            | PairingKind::Authorize
            | PairingKind::AuthorizeService { .. }
            | PairingKind::DisplayPinCode { .. }
            | PairingKind::DisplayPasskey { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::brightness::DisplayKind;
    use crate::services::media::Metadata;
    use std::time::Instant;

    /// Kart yalnız onay/izin/kod gösterimini cevaplar; PIN/passkey (ve yeni türler) reddedilir.
    #[test]
    fn card_handles_only_known_prompts() {
        assert!(card_handles(&PairingKind::Confirm { passkey: 1 }));
        assert!(card_handles(&PairingKind::Authorize));
        assert!(!card_handles(&PairingKind::PinCode));
        assert!(!card_handles(&PairingKind::Passkey));
    }

    /// Crate varsayılanı ajansız; kabuk NM/BlueZ ajanlarını açıkça ister (davranış aşama 1 ile aynı).
    #[test]
    fn shell_requests_agents_explicitly() {
        use crate::services::bluetooth::AGENT_PATH;
        use crate::services::network::AGENT_ID;
        assert_eq!(NetworkOptions::default().agent_id, None);
        assert_eq!(BluetoothOptions::default().agent_path, None);
        assert_eq!(network_options(None, false).agent_id.as_deref(), Some(AGENT_ID));
        assert_eq!(network_options(Some("test-x"), false).agent_id, None, "test örneği kaydolmaz");
        assert_eq!(network_options(Some("test-x"), true).agent_id.as_deref(), Some("org.nuxut.shell.test-x"));
        assert_eq!(bluetooth_options().agent_path.as_deref(), Some(AGENT_PATH));
        let d = NetworkOptions::default();
        let n = network_options(None, false);
        assert_eq!((n.call_timeout, n.secret_timeout, n.throttle), (d.call_timeout, d.secret_timeout, d.throttle));
    }

    /// Her hata türünün kendi katalog metni var (anahtar eksikse metin anahtarın kendisi olur);
    /// türler aynı satıra düşmez, D-Bus hata adı ya da log metni kullanıcıya çıkmaz.
    #[test]
    fn every_error_kind_has_its_own_text() {
        crate::i18n::use_turkish_for_tests();
        let network = [
            NmError::NmUnavailable,
            NmError::ConnectionUnknown,
            NmError::ConnectionInactive,
            NmError::ApUnknown,
            NmError::Enterprise,
            NmError::InvalidPassword,
            NmError::NoWifiDevice,
            NmError::InvalidSsid,
            NmError::DeviceUnknown,
        ]
        .map(|e| network_error_text(&e));
        let bluetooth = [
            BluezError::Timeout,
            BluezError::NoAdapter,
            BluezError::BluezUnavailable,
            BluezError::UnknownDevice,
            BluezError::AuthenticationRejected,
            BluezError::AuthenticationFailed,
            BluezError::AuthenticationCanceled,
            BluezError::AuthenticationTimeout,
            BluezError::ConnectionAttemptFailed,
            BluezError::AlreadyExists,
            BluezError::InProgress,
            BluezError::NotReady,
            BluezError::InvalidValue,
        ]
        .map(|e| bluetooth_error_text(&e));
        for texts in [&network[..], &bluetooth[..]] {
            for (i, t) in texts.iter().enumerate() {
                assert!(!t.starts_with("shell-") && !t.contains("org.bluez"), "{t}");
                assert!(!texts[..i].contains(t), "aynı metin iki türde: {t}");
            }
        }
    }

    fn display(id: &str, percent: f64) -> Display {
        Display { id: id.into(), kind: DisplayKind::Internal, output: None, percent }
    }

    #[test]
    fn snapshots_report_change_only_when_different() {
        let mut m = SystemModel::default();
        let s = AudioState { available: true, ..AudioState::default() };
        assert!(m.apply(&SystemEvent::Audio(AudioEvent::State(s.clone()))));
        assert!(!m.apply(&SystemEvent::Audio(AudioEvent::State(s))));
        assert!(!m.apply(&SystemEvent::Power(PowerEvent::Changed(PowerState::default()))));
    }

    #[test]
    fn brightness_changes_merge_by_id() {
        let mut m = SystemModel::default();
        assert!(m.apply(&SystemEvent::Brightness(BrightnessEvent::Displays(vec![display("a", 10.0)]))));
        assert!(!m.apply(&SystemEvent::Brightness(BrightnessEvent::Changed(display("a", 10.0)))));
        assert!(m.apply(&SystemEvent::Brightness(BrightnessEvent::Changed(display("a", 40.0)))));
        assert!(m.apply(&SystemEvent::Brightness(BrightnessEvent::Changed(display("b", 5.0)))));
        assert_eq!(m.displays.iter().map(|d| d.percent).collect::<Vec<_>>(), [40.0, 5.0]);
    }

    #[test]
    fn charge_limit_errors_reach_card_only_for_card_requests() {
        crate::i18n::use_turkish_for_tests();
        let done = |id, result| SystemEvent::Power(PowerEvent::ChargeLimitDone { id, result });
        let mut m = SystemModel::default();
        // `msg` isteği (kartın listesinde değil): kart hata göstermez.
        assert!(!m.apply(&done(7, Err(PowerError::Denied))));
        assert!(m.error_for(ErrorSource::Power).is_none());
        // Kart isteği: ret anlaşılır metinle kartta; başarı sessiz; kimlik bir kez kullanılır.
        m.charge_requests = vec![1, 2];
        assert!(!m.apply(&done(1, Ok(()))));
        assert!(m.apply(&done(2, Err(PowerError::Denied))));
        assert!(m.error_for(ErrorSource::Power).is_some_and(|e| e.contains("izin yok")));
        assert!(m.charge_requests.is_empty());
        assert!(!m.apply(&done(2, Err(PowerError::Denied))));
    }

    #[test]
    fn errors_are_scoped_and_bounded() {
        let mut m = SystemModel::default();
        let long = "x".repeat(1000);
        m.charge_requests = vec![1];
        m.apply(&SystemEvent::Power(PowerEvent::ChargeLimitDone { id: 1, result: Err(PowerError::Failed(long)) }));
        assert!(m.error_for(ErrorSource::Audio).is_none());
        let e = m.error_for(ErrorSource::Power).unwrap();
        assert_eq!(e.chars().count(), MAX_ERROR + 1);
    }

    #[test]
    fn brightness_target_prefers_output_then_backlight() {
        let ext = Display { id: "ddc:DP-1".into(), kind: DisplayKind::External, output: Some("DP-1".into()), percent: 50.0 };
        let mut m = SystemModel { displays: vec![ext.clone(), display("backlight:intel", 30.0)], ..SystemModel::default() };
        let id = |m: &SystemModel, o: Option<&str>| m.brightness_display(o).map(|d| d.id.clone());
        assert_eq!(id(&m, Some("DP-1")).as_deref(), Some("ddc:DP-1"));
        assert_eq!(id(&m, Some("eDP-1")).as_deref(), Some("backlight:intel"));
        assert_eq!(id(&m, None).as_deref(), Some("backlight:intel"));
        // Masaüstü: arka ışık yok — çıktı verilmemişse ilk ekran, bilinmeyen çıktıda hiçbiri.
        m.displays = vec![ext];
        assert_eq!(id(&m, None).as_deref(), Some("ddc:DP-1"));
        assert_eq!(id(&m, Some("HDMI-A-1")), None);
        m.displays.clear();
        assert_eq!(id(&m, None), None);
    }

    fn player_with_art(art: Option<ArtSource>) -> Player {
        let mut p = Player::new("org.mpris.MediaPlayer2.test".into(), Instant::now());
        p.metadata = Metadata { art, ..Metadata::default() };
        p
    }

    #[test]
    fn art_is_wanted_for_active_player() {
        let mut m = SystemModel {
            players: vec![player_with_art(Some(ArtSource::Remote("https://example.org/a.png".into())))],
            ..SystemModel::default()
        };
        assert_eq!(m.wanted_art().as_deref(), Some("https://example.org/a.png"));
        m.players = vec![player_with_art(Some(ArtSource::File("/tmp/a b.png".into())))];
        assert_eq!(m.wanted_art().as_deref(), Some("file:///tmp/a%20b.png"));
        // Gizlilik anahtarı kapalı: ağ kapağı istenmez, yerel kapak etkilenmez.
        m.remote_art = false;
        assert_eq!(m.wanted_art().as_deref(), Some("file:///tmp/a%20b.png"));
        m.players = vec![player_with_art(Some(ArtSource::Remote("https://example.org/a.png".into())))];
        assert_eq!(m.wanted_art(), None);
        m.players = vec![player_with_art(None)];
        assert_eq!(m.wanted_art(), None);
    }

    #[test]
    fn stale_art_is_not_shown() {
        let image = ArtImage { width: 1, height: 1, pixels: Arc::from(vec![0u8; 4]) };
        let mut m = SystemModel {
            players: vec![player_with_art(Some(ArtSource::File("/tmp/a.png".into())))],
            ..SystemModel::default()
        };
        assert!(m.apply(&SystemEvent::ArtDecoded { url: "file:///tmp/a.png".into(), image: Some(image) }));
        assert!(m.art_image().is_some());
        m.players = vec![player_with_art(Some(ArtSource::File("/tmp/b.png".into())))];
        assert!(m.art_image().is_none());
    }

    #[test]
    fn secret_request_cleared_only_by_matching_id() {
        let mut m = SystemModel::default();
        let r = SecretRequest {
            id: 7,
            connection: "c".into(),
            ssid: None,
            kind: crate::services::network::SecretKind::WifiPassword,
            retry: true,
        };
        assert!(m.apply(&SystemEvent::Network(NetworkEvent::SecretRequest(r))));
        assert!(!m.apply(&SystemEvent::Network(NetworkEvent::SecretDone { id: 8 })));
        assert!(m.secret_request.is_some());
        assert!(m.apply(&SystemEvent::Network(NetworkEvent::SecretDone { id: 7 })));
        assert!(m.secret_request.is_none());
    }
}
