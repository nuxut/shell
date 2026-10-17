//! Bildirim görsellerini arka planda çözme ve kart boyutuna küçültme (`nuxut-image` Loader).
//!
//! Sunucu `image-path`/`app_icon` dosyasını güvenle okur ([`super::hints::read_image_file`]);
//! kodlanmış baytlar ([`RawIcon::Encoded`]) ve tema ikon adları Loader'ın kendi thread'inde
//! çözülür ve küçültülür: içerik görseli en uzun kenarda [`IMAGE_MAX_SIDE`], uygulama simgesi
//! [`APP_ICON_LOGICAL`] mantıksal × çıkış ölçeği fiziksel piksel (üst sınır
//! [`APP_ICON_MAX_SIDE`]; bkz. [`app_icon_side`]). Modele yalnızca küçük RGBA girer, ham
//! baytlar iş bitince bırakılır. Çözme hiçbir zaman ana döngüde ya da D-Bus thread'inde
//! yapılmaz.
//!
//! Bellek bütçesi (en kötü durum, bildirim başına): görsel 256×256×4 = 256 KiB + app simgesi
//! ölçek 1'de 64×64×4 = 16 KiB (ölçek 2'de 128×128×4 = 64 KiB) → 200 bildirimlik merkezde
//! ≈ 53 MiB (ölçek 2: ≈ 62.5 MiB), varsayılan 100'lük geçmişte yarısı. İstisna: `image-data`
//! ham pikselleri (≤ 512×512 = 1 MiB) `nuxut-image`'te ham piksel küçültme API'si gelene
//! kadar küçültülmeden geçer.

use std::sync::Arc;

use nuxut_image::{DecodeOptions, Decoded, Fit, IconTheme, Limits, LoadRequest, Source, TargetSize, Trust};

use super::hints::{MAX_IMAGE_FILE_BYTES, NotifyIcon, RawIcon};

/// İçerik görselinin (avatar, ekran görüntüsü) en uzun kenarı (fiziksel piksel).
pub const IMAGE_MAX_SIDE: u32 = 256;
/// Uygulama simgesinin mantıksal boyutu.
pub const APP_ICON_LOGICAL: u32 = 64;
/// Uygulama simgesinin fiziksel üst sınırı.
pub const APP_ICON_MAX_SIDE: u32 = 128;

/// Çıkış ölçeğine göre uygulama simgesinin fiziksel kenarı: `64 × ölçek` (yukarı yuvarlanır),
/// 1..=[`APP_ICON_MAX_SIDE`]. Geçersiz ölçek → 1.
pub fn app_icon_side(scale: f32) -> u32 {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    ((APP_ICON_LOGICAL as f32 * scale).ceil() as u32).clamp(1, APP_ICON_MAX_SIDE)
}

/// Görselin bildirimdeki yeri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    AppIcon,
    Image,
}

impl Slot {
    /// En uzun kenar (fiziksel). `app_icon`: [`app_icon_side`] sonucu.
    pub fn max_side(self, app_icon: u32) -> u32 {
        match self {
            Self::AppIcon => app_icon.min(APP_ICON_MAX_SIDE),
            Self::Image => IMAGE_MAX_SIDE,
        }
    }

    /// Çözme ayarları: ikon sınırları (dosya sınırı sunucununkiyle aynı), yalnızca ilk kare,
    /// kutuya sığacak şekilde küçültme (raster büyütülmez; SVG hedef boyutta çizilir).
    pub fn decode_options(self, app_icon: u32) -> DecodeOptions {
        let side = self.max_side(app_icon);
        DecodeOptions {
            limits: Limits { max_file_bytes: MAX_IMAGE_FILE_BYTES, ..Limits::ICON },
            target: Some(TargetSize::new(side, side, Fit::Contain)),
            animate: false,
        }
    }
}

/// Çözülecek kaynak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    /// Dosyadan güvenle okunmuş kodlanmış baytlar.
    Bytes(Arc<[u8]>),
    /// Tema ikon adı: ikon temasında aranıp çözülür (bulunamazsa modelde ad kalır).
    IconName(String),
}

/// Çözülmesi bekleyen görsel.
#[derive(Debug, Clone)]
pub struct PendingImage {
    pub slot: Slot,
    pub source: ImageSource,
}

impl PendingImage {
    /// Loader isteği; `app_icon`: uygulama simgesinin fiziksel kenarı ([`app_icon_side`]).
    pub fn request(&self, theme: &Arc<IconTheme>, app_icon: u32) -> LoadRequest {
        match &self.source {
            ImageSource::Bytes(bytes) => {
                LoadRequest { source: Source::Bytes(bytes.clone()), options: self.slot.decode_options(app_icon) }
            }
            ImageSource::IconName(name) => {
                // Yalnızca doğrulanmış tema adı System ister; framework dosyayı kullanıcının
                // yazabildiği tema dizininde bulursa Untrusted'a düşürür. Dosya yolu ve
                // image-data baytları (Bytes) Limits::ICON varsayılanıyla Untrusted kalır.
                LoadRequest::icon(theme.clone(), name.clone(), self.slot.max_side(app_icon), 1).with_trust(Trust::System)
            }
        }
    }
}
/// Ham girdiyi modele uygun hale getirir: hazır olanlar hemen döner; kodlanmış baytlar
/// [`NotifyIcon::None`], tema adları [`NotifyIcon::Name`] ile döner ve çözülmek üzere ayrılır.
pub fn split(raw: RawIcon, slot: Slot) -> (NotifyIcon, Option<PendingImage>) {
    match raw {
        RawIcon::None => (NotifyIcon::None, None),
        RawIcon::Name(name) => {
            let job = PendingImage { slot, source: ImageSource::IconName(name.clone()) };
            (NotifyIcon::Name(name), Some(job))
        }
        RawIcon::Pixmap { width, height, rgba } => (NotifyIcon::Pixmap { width, height, rgba }, None),
        RawIcon::Encoded { bytes, .. } => {
            (NotifyIcon::None, Some(PendingImage { slot, source: ImageSource::Bytes(bytes) }))
        }
    }
}

/// `app_icon` boşsa masaüstü girdisinin `Icon=` değeri (yoksa girdi adının kendisi) tema ikon
/// adı olarak denenir. Küçük bir `.desktop` dosyası okur; çözme yapmaz.
pub fn desktop_entry_icon(desktop_entry: &str) -> Option<RawIcon> {
    let name = nuxut_image::icon::desktop_icon_name(desktop_entry).unwrap_or_else(|| desktop_entry.to_owned());
    match super::hints::parse_icon_ref(&name) {
        RawIcon::None => None,
        icon => Some(icon),
    }
}

/// Çözme sonucunu modele çevirir (premultiplied → düz RGBA; ilk kare).
pub fn to_icon(decoded: Decoded) -> NotifyIcon {
    let image = decoded.into_first_frame();
    let (width, height) = image.size();
    NotifyIcon::Pixmap { width, height, rgba: image.to_straight().into() }
}
