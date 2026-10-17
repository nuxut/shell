//! Kilidin kullanıcı dosyası görselleri: duvar kağıdı ve avatar (`~/.face`). İkisi de yalıtılmış
//! işçide, framework'ün **duvar kağıdı profiliyle** çözülür (framework ARCHITECTURE "Duvar kağıdı
//! yalıtımı"; ADR-0007 takip): kullanıcı dosyası internetten inmiş olabilir ve kilit süreci
//! PAM sırrını tutar — çözücü açığı bu süreçte çalışmamalı.
//!
//! Süreç başına tek `Sandbox` (`SandboxConfig::wallpaper()`; tek işçi, boşta süreç yok) ve onun
//! üstünde tek `WallpaperDecoder` (`linger: ZERO`: kuyruk boşalınca kareler bırakılır; aynı
//! turdaki çıkışlar yine paylaşır). Avatar aynı `Sandbox`'tan `decode_file` ile çözülür, anahtar
//! `file_key("avatar", yol)`. İşçi ikilisi yoksa framework PNG/JPEG/WebP/GIF/QOI'yi ilk kare
//! olarak süreç içinde çözer (S1, süreç başına tek uyarı); diğerleri `Unavailable`.
//!
//! Hata hiçbir zaman panik değildir: duvar kağıdında zemin rengi, avatarda baş harfler kalır;
//! log yalnızca hata metnini taşır (yol değil).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nuxut_image::{
    CancelFlag, DecodeOptions, Image, ImageError, IsolatedDecoder, Limits, TargetSize, WallpaperDecoder,
    WallpaperDecoderOptions,
};
use nuxut_image_sandbox::{Sandbox, SandboxConfig};

/// Avatarın çözüm kutusu (fiziksel px; ortadan kırpılmış kare).
pub const AVATAR_PX: u32 = 256;

/// Kilidin paylaşılan görsel çözücüleri.
#[derive(Clone)]
pub struct LockImages {
    isolated: Arc<dyn IsolatedDecoder>,
    wallpaper: Arc<WallpaperDecoder>,
    /// Yerleşik duvar kağıtlarının render'ı (`nuxut:<id>`; önbellek dizini shell'le ortak).
    builtins: Arc<nuxut_wallpapers::Renderer>,
}

impl std::fmt::Debug for LockImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockImages")
            .field("isolation", &self.isolated.isolation())
            .finish_non_exhaustive()
    }
}

impl LockImages {
    /// Üretim: duvar kağıdı profilli tek `Sandbox` (süreç başlatmaz; tembel).
    pub fn new() -> Self {
        let sandbox = Sandbox::new(SandboxConfig::wallpaper());
        tracing::info!(isolation = ?sandbox.isolation(), "kilit görsel yalıtımı");
        Self::with_decoder(Arc::new(sandbox))
    }

    /// Çözücü verilmiş biçim (testler: sahte ya da işçisiz `Sandbox`).
    pub fn with_decoder(isolated: Arc<dyn IsolatedDecoder>) -> Self {
        let wallpaper = WallpaperDecoder::new(
            isolated.clone(),
            WallpaperDecoderOptions {
                linger: Duration::ZERO,
                ..WallpaperDecoderOptions::default()
            },
        );
        Self { isolated, wallpaper, builtins: nuxut_wallpapers::Renderer::new() }
    }

    /// Bütün kilit yüzeylerinin paylaştığı yerleşik duvar kağıdı render'ı.
    pub fn builtins(&self) -> &Arc<nuxut_wallpapers::Renderer> {
        &self.builtins
    }

    /// Bütün kilit yüzeylerinin paylaştığı duvar kağıdı çözücüsü.
    pub fn wallpaper(&self) -> &Arc<WallpaperDecoder> {
        &self.wallpaper
    }

    /// İlk var olan aday dosyayı yalıtılmış işçide çözer; yoksa ya da çözülemezse `None`
    /// (baş harfler kalır). Arka plan thread'inden çağrılır (bloklar).
    pub fn load_avatar(&self, candidates: &[PathBuf]) -> Option<Image> {
        let path = crate::assets::first_file(candidates)?;
        decode_avatar(&path, &*self.isolated)
            .inspect_err(|e| tracing::debug!("avatar çözülemedi: {e}"))
            .ok()
    }
}

impl Default for LockImages {
    fn default() -> Self {
        Self::new()
    }
}

/// Avatar çözüm seçenekleri: duvar kağıdı sınırları (kullanıcı fotoğrafı büyük olabilir),
/// `AVATAR_PX` kareye ortadan kırpma, yalnızca ilk kare.
pub fn avatar_options() -> DecodeOptions {
    DecodeOptions {
        limits: Limits::WALLPAPER,
        target: Some(TargetSize::cover_crop(AVATAR_PX, AVATAR_PX)),
        animate: false,
    }
}

/// `path`'i açar (normal dosya, `O_NONBLOCK`: FIFO'da takılmaz) ve yalıtılmış çözücüye akıtır.
pub fn decode_avatar(path: &Path, decoder: &dyn IsolatedDecoder) -> Result<Image, ImageError> {
    let file = nuxut_image::open_user_file(path)?;
    let key = nuxut_image::file_key("avatar", path);
    decoder
        .decode_file(key, file, &avatar_options(), &CancelFlag::new())
        .map(|d| d.image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_image::{DecodedFile, Isolation, SandboxFailure};
    use std::sync::Mutex;

    /// Sahte yalıtılmış çözücü: çağrıları (anahtar, seçenek) kaydeder.
    #[derive(Debug, Default)]
    struct Fake {
        calls: Mutex<Vec<(u64, DecodeOptions)>>,
        fail: Option<SandboxFailure>,
    }

    impl IsolatedDecoder for Fake {
        fn isolation(&self) -> Isolation {
            Isolation::Process
        }
        fn decode(&self, _: u64, _: Arc<[u8]>, _: &DecodeOptions) -> Result<Image, ImageError> {
            unreachable!("avatar dosyadan akıtılır, belleğe alınmaz")
        }
        fn decode_file(
            &self,
            key: u64,
            _file: std::fs::File,
            opts: &DecodeOptions,
            _cancel: &CancelFlag,
        ) -> Result<DecodedFile, ImageError> {
            self.calls.lock().unwrap().push((key, *opts));
            if let Some(f) = self.fail {
                return Err(ImageError::Sandbox(f));
            }
            let image = Image::from_straight(2, 2, vec![255; 16]).unwrap();
            Ok(DecodedFile::from_source(image, &[], &opts.limits))
        }
    }

    #[test]
    fn avatar_goes_through_isolated_decoder() {
        let dir = tempfile::tempdir().unwrap();
        let face = dir.path().join(".face");
        std::fs::write(&face, b"\x89PNG kotu").unwrap();
        let fake = Arc::new(Fake::default());
        let images = LockImages::with_decoder(fake.clone());
        let missing = dir.path().join("yok");
        let img = images.load_avatar(&[missing, face.clone()]).expect("avatar");
        assert_eq!((img.width(), img.height()), (2, 2));
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "tek yalıtılmış çağrı");
        let (key, opts) = calls[0];
        assert_eq!(key, nuxut_image::file_key("avatar", &face));
        assert_ne!(key, nuxut_image::file_key("wallpaper", &face), "susturma kovası duvar kağıdından ayrı");
        assert_eq!(opts, avatar_options());
        assert_eq!(opts.trust(), nuxut_image::Trust::Untrusted);
        assert!(opts.target.unwrap().crops());
    }

    #[test]
    fn avatar_failures_fall_back_to_initials() {
        let dir = tempfile::tempdir().unwrap();
        let face = dir.path().join(".face");
        std::fs::write(&face, b"x").unwrap();
        for f in [
            SandboxFailure::Unavailable,
            SandboxFailure::Protocol,
            SandboxFailure::Timeout,
            SandboxFailure::Killed,
        ] {
            let images = LockImages::with_decoder(Arc::new(Fake {
                fail: Some(f),
                ..Fake::default()
            }));
            assert!(images.load_avatar(std::slice::from_ref(&face)).is_none(), "{f:?}");
        }
        // Aday yok / dizin: işçiye gidilmez.
        let fake = Arc::new(Fake::default());
        let images = LockImages::with_decoder(fake.clone());
        assert!(images.load_avatar(&[dir.path().join("yok"), dir.path().to_owned()]).is_none());
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn workerless_sandbox_rejects_svg_avatar() {
        // İşçi ikilisi yok: SVG süreç içine düşmez (Unavailable), panik yok.
        let dir = tempfile::tempdir().unwrap();
        let face = dir.path().join(".face");
        std::fs::write(&face, br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#).unwrap();
        let sandbox = Sandbox::new(SandboxConfig {
            worker: Some("/nonexistent/nuxut-image-worker".into()),
            ..SandboxConfig::wallpaper()
        });
        let err = decode_avatar(&face, &sandbox).unwrap_err();
        assert!(matches!(err, ImageError::Sandbox(SandboxFailure::Unavailable)), "{err:?}");
    }

    #[test]
    fn views_share_one_wallpaper_decoder() {
        let images = LockImages::with_decoder(Arc::new(Fake::default()));
        let clone = images.clone();
        assert!(Arc::ptr_eq(images.wallpaper(), clone.wallpaper()));
    }
}
