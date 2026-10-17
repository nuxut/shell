//! Shell'in kendi (dahili) bildirimleri: D-Bus'a gitmeden doğrudan
//! [`NotificationCenter`](super::NotificationCenter)'a eklenir
//! ([`NotificationCenter::add_internal`](super::NotificationCenter::add_internal)).
//!
//! - `popup = false` (sessiz): ekranda popup yok, ses yok; bildirim merkezinde okunmamış görünür
//!   (ör. shell.toml sürüm göçü, ADR-0002).
//! - `popup = true`: sıradan bildirim gibi popup (DND ve `[notifications.apps."Nuxut"]` kuralı
//!   geçerli), ardından merkezde kalır (ör. Hyprland ayar bağlantısı, ADR-0003).
//!
//! Düğmeler bir shell iç komutuna yönlenir: [`InternalAction::command`] `nuxut-shell msg`
//! sözdizimindeki komut satırıdır (ör. `config undo-migration`, `hypr apply-switch`); düğmeye
//! basılınca UI onu `ipc::parse` ile ayrıştırıp `msg` ile aynı yoldan yürütür ve bildirimi kapatır.
//! Kimlikler [`INTERNAL_ID_BASE`] ve üstündedir; D-Bus sunucusu bu aralığı hiç vermez.
//!
//! İsteğe bağlı içerik görseli ([`InternalNotice::with_image`], ör. ekran görüntüsü önizlemesi)
//! çağıranın ürettiği, zaten küçültülmüş piksellerdir: sunucu sınırı ([`IMAGE_MAX_SIDE`]) ve
//! boyut tutarlılığı modelde denetlenir, uymayan görsel atılır (bildirim görselsiz kalır).
//!
//! Maskot ([`InternalNotice::with_mascot`]): YALNIZ dahili bildirimlerde. Model bir ruh hâli
//! taşır, piksel değil; kart onu `nuxut::ui::Mascot` ile canlı çizer (tema/kip ve
//! `[appearance] mascot` değişimine uyar; gizliyken maskotsuz normal kart). D-Bus bildirimleri
//! bu alanı ayarlayamaz: sunucu hep `None` kurar, model dış `Added` olayında alanı siler.
//! Onaylı yerleşimler `docs/design/mascot-placement.md`'de; [`NoticeMood`] yalnızca onlardır.

/// Dahili bildirimlerin uygulama adı (geçmiş grubu ve uygulama kuralı anahtarı).
pub const INTERNAL_APP_NAME: &str = "Nuxut";
/// Dahili bildirim kimliklerinin alt sınırı; sunucu kimlikleri bunun altında kalır.
pub const INTERNAL_ID_BASE: u32 = 0xF000_0000;
use super::hints::NotifyIcon;
use super::images::IMAGE_MAX_SIDE;

/// Komut satırının en uzun hali (bayt); daha uzun eylem atılır.
pub const MAX_INTERNAL_COMMAND_BYTES: usize = 256;

/// Dahili görsel modele girebilir mi: tema adı her zaman; pixmap boş değil, en uzun kenarı
/// [`IMAGE_MAX_SIDE`] içinde ve `width * height * 4` bayt.
pub(crate) fn image_fits(image: &NotifyIcon) -> bool {
    match image {
        NotifyIcon::None | NotifyIcon::Name(_) => true,
        NotifyIcon::Pixmap { width, height, rgba } => {
            (1..=IMAGE_MAX_SIDE).contains(width)
                && (1..=IMAGE_MAX_SIDE).contains(height)
                && rgba.len() as u64 == u64::from(*width) * u64::from(*height) * 4
        }
    }
}

/// Kimlik dahili bildirime mi ait.
pub fn is_internal_id(id: u32) -> bool {
    id >= INTERNAL_ID_BASE
}

/// Shell'in kendi bildirimi. Sınırlar sunucununkiyle aynıdır (özet tek satır ve
/// `MAX_SUMMARY_CHARS`, gövde `MAX_BODY_BYTES`, en çok `MAX_ACTIONS` düğme); aşanlar kırpılır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalNotice {
    pub summary: String,
    /// Düz metin (markup yorumlanmaz).
    pub body: String,
    pub actions: Vec<InternalAction>,
    /// `false`: yalnızca merkeze düşer (popup ve ses yok).
    pub popup: bool,
    /// İçerik görseli; varsayılan yok.
    pub image: NotifyIcon,
    /// Kartta maskot; varsayılan yok. Varsa içerik görselinin yerini alır.
    pub mascot: Option<NoticeMood>,
}

/// Dahili bildirim kartında maskotun ruh hâli. Yalnızca onaylı bildirim yerleşimleri
/// (`docs/design/mascot-placement.md`); yenisi önce oraya yazılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoticeMood {
    /// "Güncelleme bitti" (2026-10-13).
    Celebrate,
}

impl InternalNotice {
    /// Sessiz bildirim: popup yok, merkezde okunmamış.
    pub fn silent(summary: impl Into<String>, body: impl Into<String>, actions: Vec<InternalAction>) -> Self {
        Self { summary: summary.into(), body: body.into(), actions, popup: false, image: NotifyIcon::None, mascot: None }
    }

    /// Popup'lı bildirim.
    pub fn popup(summary: impl Into<String>, body: impl Into<String>, actions: Vec<InternalAction>) -> Self {
        Self { summary: summary.into(), body: body.into(), actions, popup: true, image: NotifyIcon::None, mascot: None }
    }

    /// Kartta maskot (canlı çizilir; `[appearance] mascot = false` iken maskotsuz normal kart).
    /// İçerik görseliyle birlikte verilirse maskot kazanır, görsel atılır.
    #[must_use]
    pub fn with_mascot(mut self, mood: NoticeMood) -> Self {
        self.mascot = Some(mood);
        self
    }

    /// İçerik görseli: tema adı ya da en uzun kenarı [`IMAGE_MAX_SIDE`]'ı geçmeyen sıkı paketli
    /// RGBA8 (`NotifyIcon::Pixmap`). Sınırı aşan ya da bayt sayısı tutmayan pixmap modelde atılır.
    #[must_use]
    pub fn with_image(mut self, image: NotifyIcon) -> Self {
        self.image = image;
        self
    }
}

/// Bildirim düğmesi → shell iç komutu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalAction {
    pub label: String,
    /// `nuxut-shell msg` sözdiziminde komut satırı (ör. `config undo-migration`).
    pub command: String,
}

impl InternalAction {
    pub fn new(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self { label: label.into(), command: command.into() }
    }
}
