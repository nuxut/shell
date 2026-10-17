//! `Notify` hint'leri: D-Bus'tan bellek dostu çözme, doğrulama ve simge/görsel kaynakları.
//!
//! `a{sv}` değerleri `zvariant::Value` yerine [`HintValue`] ile çözülür: `Value` bir `ay`
//! dizisini bayt başına bir `Value::U8` olarak tutar (4 MiB görsel ≈ 100+ MiB bellek). Burada
//! görsel baytları tek `Vec<u8>` kopyasıdır, tanınmayan değerler bellek ayırmadan atlanır.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::Arc;

use serde::de::{self, Deserializer, IgnoredAny, SeqAccess, Visitor};
use zbus::zvariant::{Signature, Type};

use super::markup::{sanitize, truncate_chars};

/// Görselin kabul edilen en büyük kenarı (piksel).
pub const MAX_IMAGE_SIDE: i32 = 512;
/// Satır sonu dolgusu için izin verilen en fazla fazlalık (bayt).
const MAX_ROW_PADDING: usize = 64;
/// Simge adı / yol metninin en uzun hali.
pub const MAX_ICON_REF_BYTES: usize = 1024;
/// `image-path` ile gösterilen dosyanın en büyük boyutu (çözmeyi UI yapar).
pub const MAX_IMAGE_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// `category` ve `desktop-entry` hint'lerinin en uzun hali (karakter).
pub const MAX_HINT_TEXT_CHARS: usize = 256;

/// Çözülmüş tek hint değeri. Sayılar tek türde toplanır (`urgency` bazen `y`, bazen `i`/`u`).
#[derive(Debug, Clone, PartialEq)]
pub enum HintValue {
    Str(String),
    Bool(bool),
    Int(i64),
    Image(ImageDataHint),
    /// Tanınmayan imza; değer atlandı.
    Other,
}

/// `(iiibiiay)` görsel hint'i, doğrulanmamış.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageDataHint {
    pub width: i32,
    pub height: i32,
    pub rowstride: i32,
    pub has_alpha: bool,
    pub bits_per_sample: i32,
    pub channels: i32,
    pub data: Vec<u8>,
}

impl fmt::Debug for ImageDataHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageDataHint")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rowstride", &self.rowstride)
            .field("has_alpha", &self.has_alpha)
            .field("bits_per_sample", &self.bits_per_sample)
            .field("channels", &self.channels)
            .field("data_len", &self.data.len())
            .finish()
    }
}

impl Type for HintValue {
    const SIGNATURE: &'static Signature = &Signature::Variant;
}

type ImageTuple = (i32, i32, i32, bool, i32, i32, Vec<u8>);

impl<'de> serde::Deserialize<'de> for HintValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // D-Bus biçiminde variant, (imza, değer) dizisi olarak ziyaret edilir.
        deserializer.deserialize_any(HintVisitor)
    }
}

struct HintVisitor;

impl<'de> Visitor<'de> for HintVisitor {
    type Value = HintValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a D-Bus variant")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<HintValue, A::Error> {
        let signature: Signature =
            seq.next_element()?.ok_or_else(|| de::Error::invalid_length(0, &"variant signature"))?;
        let missing = || de::Error::invalid_length(1, &"variant value");
        let value = match signature {
            Signature::Str | Signature::ObjectPath => {
                HintValue::Str(seq.next_element::<String>()?.ok_or_else(missing)?)
            }
            Signature::Bool => HintValue::Bool(seq.next_element()?.ok_or_else(missing)?),
            Signature::U8 => HintValue::Int(seq.next_element::<u8>()?.ok_or_else(missing)?.into()),
            Signature::I16 => HintValue::Int(seq.next_element::<i16>()?.ok_or_else(missing)?.into()),
            Signature::U16 => HintValue::Int(seq.next_element::<u16>()?.ok_or_else(missing)?.into()),
            Signature::I32 => HintValue::Int(seq.next_element::<i32>()?.ok_or_else(missing)?.into()),
            Signature::U32 => HintValue::Int(seq.next_element::<u32>()?.ok_or_else(missing)?.into()),
            Signature::I64 => HintValue::Int(seq.next_element()?.ok_or_else(missing)?),
            ref sig if sig == <ImageTuple as Type>::SIGNATURE => {
                let image: ImageWire = seq.next_element()?.ok_or_else(missing)?;
                HintValue::Image(image.0)
            }
            _ => {
                seq.next_element::<IgnoredAny>()?;
                HintValue::Other
            }
        };
        Ok(value)
    }
}

/// `(iiibiiay)`; `ay` tek kopyayla `deserialize_bytes` üzerinden okunur.
struct ImageWire(ImageDataHint);

impl<'de> serde::Deserialize<'de> for ImageWire {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ImageWire;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("(iiibiiay)")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ImageWire, A::Error> {
                let field = |i| de::Error::invalid_length(i, &"7 fields");
                let width = seq.next_element()?.ok_or_else(|| field(0))?;
                let height = seq.next_element()?.ok_or_else(|| field(1))?;
                let rowstride = seq.next_element()?.ok_or_else(|| field(2))?;
                let has_alpha = seq.next_element()?.ok_or_else(|| field(3))?;
                let bits_per_sample = seq.next_element()?.ok_or_else(|| field(4))?;
                let channels = seq.next_element()?.ok_or_else(|| field(5))?;
                let data: ByteBuf = seq.next_element()?.ok_or_else(|| field(6))?;
                Ok(ImageWire(ImageDataHint {
                    width,
                    height,
                    rowstride,
                    has_alpha,
                    bits_per_sample,
                    channels,
                    data: data.0,
                }))
            }
        }
        deserializer.deserialize_tuple(7, V)
    }
}

struct ByteBuf(Vec<u8>);

impl<'de> serde::Deserialize<'de> for ByteBuf {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ByteBuf;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a byte array")
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<ByteBuf, E> {
                Ok(ByteBuf(v.to_vec()))
            }
            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<ByteBuf, E> {
                Ok(ByteBuf(v))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ByteBuf, A::Error> {
                let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1 << 20));
                while let Some(b) = seq.next_element()? {
                    out.push(b);
                }
                Ok(ByteBuf(out))
            }
        }
        deserializer.deserialize_bytes(V)
    }
}

// ---------------------------------------------------------------------------------------
// Görsel ve simge kaynakları

/// Modeldeki simge/görsel: tema adı ya da çözülmüş, küçültülmüş pikseller. Ham dosya baytı
/// modele hiç girmez.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum NotifyIcon {
    #[default]
    None,
    /// Tema simge adı (`dialog-information`, `firefox`); UI temadan yükler.
    Name(String),
    /// Sıkı paketli düz (önçarpılmamış) RGBA8, `width * height * 4` bayt, satır dolgusu yok.
    /// Sunucudan çıkan görsel en uzun kenarda [`super::images::IMAGE_MAX_SIDE`] (app simgesi
    /// [`super::images::APP_ICON_MAX_SIDE`]) pikseli geçmez — `image-data` hariç, bkz. o modül.
    Pixmap { width: u32, height: u32, rgba: Arc<[u8]> },
}

impl NotifyIcon {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Bellekte tuttuğu piksel baytı.
    pub fn byte_size(&self) -> usize {
        match self {
            Self::Pixmap { rgba, .. } => rgba.len(),
            _ => 0,
        }
    }
}

impl fmt::Debug for NotifyIcon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Name(name) => f.debug_tuple("Name").field(name).finish(),
            Self::Pixmap { width, height, .. } => write!(f, "Pixmap({width}x{height})"),
        }
    }
}

/// Sunucunun D-Bus girdisinden çıkardığı, henüz çözülmemiş simge/görsel. Yalnızca sunucu
/// içinde yaşar; `Encoded` arka planda çözülüp [`NotifyIcon::Pixmap`] olur.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum RawIcon {
    #[default]
    None,
    /// Tema simge adı (`dialog-information`, `firefox`).
    Name(String),
    /// Yerel dosyanın (`image-path`/`app_icon` yolu) sunucuda bir kez, güvenli açılıp okunmuş
    /// kodlanmış içeriği (PNG/SVG/JPEG...). Yol tutulmaz, dosya bir daha açılmaz (TOCTOU
    /// yok). `file_name` yalnızca biçim ipucudur (uzantı).
    Encoded { bytes: Arc<[u8]>, file_name: String },
    /// Hint ile gelen, doğrulanmış piksel verisi (düz RGBA8, sıkı paketli).
    Pixmap { width: u32, height: u32, rgba: Arc<[u8]> },
}

impl RawIcon {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl fmt::Debug for RawIcon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Name(name) => f.debug_tuple("Name").field(name).finish(),
            Self::Encoded { bytes, file_name } => write!(f, "Encoded({file_name:?}, {} bayt)", bytes.len()),
            Self::Pixmap { width, height, .. } => write!(f, "Pixmap({width}x{height})"),
        }
    }
}

/// Neden reddedildi (log için).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    #[error("geçersiz boyut {0}x{1} (izin verilen 1..=512)")]
    Size(i32, i32),
    #[error("desteklenmeyen biçim: bits_per_sample={bits}, channels={channels}, has_alpha={alpha}")]
    Format { bits: i32, channels: i32, alpha: bool },
    #[error("geçersiz rowstride {rowstride} (satır {row} bayt)")]
    Rowstride { rowstride: i32, row: usize },
    #[error("veri uzunluğu {len}, beklenen {min}..={max}")]
    Length { len: usize, min: usize, max: usize },
}

impl ImageDataHint {
    /// Boyut, biçim, rowstride ve uzunluğu doğrular; [`RawIcon::Pixmap`]'e (RGBA8) çevirir.
    pub fn decode(&self) -> Result<RawIcon, ImageError> {
        let (w, h) = (self.width, self.height);
        if !(1..=MAX_IMAGE_SIDE).contains(&w) || !(1..=MAX_IMAGE_SIDE).contains(&h) {
            return Err(ImageError::Size(w, h));
        }
        let expected_channels = if self.has_alpha { 4 } else { 3 };
        if self.bits_per_sample != 8 || self.channels != expected_channels {
            return Err(ImageError::Format {
                bits: self.bits_per_sample,
                channels: self.channels,
                alpha: self.has_alpha,
            });
        }
        // Sınırlar yukarıda kontrol edildi: çarpımlar usize'da taşmaz.
        let (w, h, channels) = (w as usize, h as usize, expected_channels as usize);
        let row = w * channels;
        let rowstride = usize::try_from(self.rowstride).unwrap_or(0);
        if rowstride < row || rowstride > row + MAX_ROW_PADDING {
            return Err(ImageError::Rowstride { rowstride: self.rowstride, row });
        }
        // Son satırın dolgusu olmayabilir (GdkPixbuf böyle gönderir).
        let min = rowstride * (h - 1) + row;
        let max = rowstride * h;
        if !(min..=max).contains(&self.data.len()) {
            return Err(ImageError::Length { len: self.data.len(), min, max });
        }
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let line = &self.data[y * rowstride..y * rowstride + row];
            if channels == 4 {
                rgba.extend_from_slice(line);
            } else {
                for px in line.chunks_exact(3) {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
        Ok(RawIcon::Pixmap { width: w as u32, height: h as u32, rgba: rgba.into() })
    }
}

/// `app_icon` veya `image-path` değerini çözer: tema simge adı ya da mutlak yol / `file://`
/// URI'si. Yol [`read_image_file`] ile hemen okunur. Başka şema (http, smb, ...), göreli yol,
/// `..` ve reddedilen dosyalar [`RawIcon::None`] olur.
pub fn parse_icon_ref(raw: &str) -> RawIcon {
    let s = raw.trim();
    if s.is_empty() || s.len() > MAX_ICON_REF_BYTES || s.chars().any(char::is_control) {
        return RawIcon::None;
    }
    let path = if let Some(rest) = s.strip_prefix("file://") {
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        match percent_decode(rest) {
            Some(bytes) if rest.starts_with('/') => PathBuf::from(OsString::from_vec(bytes)),
            _ => return RawIcon::None,
        }
    } else if s.starts_with('/') {
        PathBuf::from(s)
    } else if s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+')) && !s.starts_with('.')
    {
        return RawIcon::Name(s.to_owned());
    } else {
        return RawIcon::None;
    };
    match read_image_file(&path) {
        Ok(icon) => icon,
        Err(e) => {
            tracing::warn!(error = %e, "bildirim görsel dosyası reddedildi");
            RawIcon::None
        }
    }
}

pub use crate::services::safe_file::FileError;

/// Yerel görsel dosyasını güvenli okur ([`crate::services::safe_file::read_untrusted`]:
/// mutlak yol, `..` yok, `O_NOFOLLOW`/`O_NONBLOCK` tek açılış, normal dosya + sahte dosya
/// sistemi reddi, boyut 1..=[`MAX_IMAGE_FILE_BYTES`], sınırlı okuma).
pub fn read_image_file(path: &std::path::Path) -> Result<RawIcon, FileError> {
    let bytes = crate::services::safe_file::read_untrusted(path, MAX_IMAGE_FILE_BYTES)?;
    let file_name = path.file_name().map(|n| truncate_chars(&n.to_string_lossy(), 255).to_owned()).unwrap_or_default();
    Ok(RawIcon::Encoded { bytes: bytes.into(), file_name })
}

fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            let b = u8::from_str_radix(hex, 16).ok()?;
            if b == 0 {
                return None;
            }
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------
// Hint'ler

/// Aciliyet (`urgency` hint'i).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

/// Ayıklanmış hint'ler. `x`/`y` (konum) ve ses hint'leri yok sayılır.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hints {
    pub urgency: Urgency,
    pub category: Option<String>,
    pub desktop_entry: Option<String>,
    /// `image-data` > `image_data` > `image-path` > `image_path` > `icon_data` önceliğiyle.
    pub image: RawIcon,
    pub resident: bool,
    pub transient: bool,
    pub suppress_sound: bool,
    pub action_icons: bool,
}

impl Hints {
    /// Hint sözlüğünü ayıklar. Geçersiz değerler log'lanır ve yok sayılır.
    pub fn parse(map: &HashMap<String, HintValue>) -> Self {
        let text = |key: &str| match map.get(key) {
            Some(HintValue::Str(s)) => {
                let s = sanitize(truncate_chars(s.trim(), MAX_HINT_TEXT_CHARS));
                (!s.is_empty()).then_some(s)
            }
            _ => None,
        };
        let flag = |key: &str| match map.get(key) {
            Some(HintValue::Bool(b)) => *b,
            Some(HintValue::Int(i)) => *i != 0,
            _ => false,
        };
        let urgency = match map.get("urgency") {
            Some(HintValue::Int(0)) => Urgency::Low,
            Some(HintValue::Int(2)) => Urgency::Critical,
            _ => Urgency::Normal,
        };

        let mut image = RawIcon::None;
        for key in ["image-data", "image_data", "image-path", "image_path", "icon_data"] {
            image = match map.get(key) {
                Some(HintValue::Image(data)) => match data.decode() {
                    Ok(img) => img,
                    Err(e) => {
                        tracing::warn!(hint = key, error = %e, "bildirim görseli reddedildi");
                        RawIcon::None
                    }
                },
                Some(HintValue::Str(path)) if key.contains("path") => parse_icon_ref(path),
                _ => RawIcon::None,
            };
            if !image.is_none() {
                break;
            }
        }

        Self {
            urgency,
            category: text("category"),
            desktop_entry: text("desktop-entry"),
            image,
            resident: flag("resident"),
            transient: flag("transient"),
            suppress_sound: flag("suppress-sound"),
            action_icons: flag("action-icons"),
        }
    }
}
