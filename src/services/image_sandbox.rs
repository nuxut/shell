//! Güvenilmeyen görsellerin yalıtılmış çözümü (framework ADR-0007), shell bağlantısı.
//!
//! Shell açılışta **tek** bir [`Sandbox`] kurar ([`new_shared`]); tepsi, bildirim, medya kapağı
//! (MPRIS) ve başlatıcı ikonu Loader'ları onu `Loader::with_sandbox` ile paylaşır: işçi havuzu,
//! hız sınırı, anahtar susturma ve negatif önbellek tüm kaynaklar için ortak. İşler kaynak
//! anahtarıyla ([`source_key`]) gönderilir: tepside öğe kimliği, MPRIS'te oynatıcı adı
//! (`load_keyed`: iptal ve kaynak anahtarı aynı).
//!
//! Bildirimde iki anahtar ayrıdır (`Loader::load_with_keys`): iptal anahtarı gönderen, yuva ve
//! bildirim kimliği — art arda gelen *farklı* bildirimler birbirinin görselini iptal etmez,
//! birleştirme yalnızca `replaces_id` ile olur; kaynak anahtarı gönderen sürecin D-Bus ile
//! doğrulanmış kimliği ([`KeyDomain::NotifySender`], `notifications::sender`) — susturma, hız
//! sınırı ve negatif önbellek gönderen başına ortak. İstemcinin bildirdiği `app_name` anahtara
//! girmez: girseydi bir uygulama başka bir uygulamanın adıyla onun görsellerini susturtabilirdi.
//!
//! İşçi ikilisi `nuxut-image-worker` framework'teki ayrı pakettir (shell [[bin]] tanımlamaz; JXL
//! çözücüsü kabuk ikilisine girmez): `cargo build --release --manifest-path ../framework/Cargo.toml
//! -p nuxut-image-worker`, install.sh bunu `~/.local/lib/nuxut/` altına kurar. Arama sırası
//! framework'te: `NUXUT_IMAGE_WORKER` → çalışan ikilinin yanı → `../lib/nuxut` →
//! `~/.local/lib/nuxut` → `/usr/lib/nuxut`. Bulunamazsa `Isolation::Unavailable`: kapılar
//! (susturma, negatif önbellek, hız sınırı) yine uygulanır, yalnızca PNG/JPEG/WebP/GIF/QOI
//! süreç içinde ilk kare; SVG/JXL/AVIF yedek ikonla. Sandbox hiç kurulmazsa (testler) güvenilmeyen
//! SVG reddedilir, raster süreç içinde.
//!
//! Hata (`Err`) her zaman yedek ikona düşer; yedek, kaynağın mevcut yoludur (tepside öğe
//! pixmap'i ya da yer tutucu, bildirimde semantik ikon, kapakta yer tutucu).
//!
//! **Duvar kağıdı** ayrı profildedir ([`new_wallpaper_decoder`]): kullanıcı dosyası, her biçim
//! (JXL dahil) güven ne olursa olsun `SandboxConfig::wallpaper()` işçisinde çözülür (framework
//! "Duvar kağıdı yalıtımı"). Süreç başına tek `WallpaperDecoder`; bütün çıkışların yüzeyleri
//! aynı örneği paylaşır (aynı dosya + boyut tek çözüm, birleştirme ve önbellek ortak). Kendi
//! `Sandbox`'ı vardır: tepsi ikonu fırtınası duvar kağıdını hız sınırına sokamaz. [`ENABLED`]
//! kapısı duvar kağıdına uygulanmaz (süreç içine düşen sandbox'sız yol yoktur; işçi ikilisi
//! yoksa framework PNG/JPEG/WebP/GIF/QOI'yi ilk kare olarak süreç içinde çözer, diğerleri
//! zemin rengine düşer).

use std::hash::{BuildHasher, Hash, RandomState};
use std::sync::{Arc, OnceLock};

use nuxut_image::{IsolatedDecoder, Isolation, NO_KEY, WallpaperDecodeStats, WallpaperDecoder, WallpaperDecoderOptions};
pub use nuxut_image_sandbox::{Sandbox, SandboxConfig, SandboxStats};

/// Kaynak anahtarının alanı: aynı ad farklı alanlarda farklı anahtar verir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyDomain {
    /// Tepsi öğesi (`ItemId`: benzersiz bus adı + nesne yolu).
    Tray,
    /// Bildirimin uygulama simgesi (gönderen + bildirim kimliği).
    NotifyAppIcon,
    /// Bildirimin içerik görseli (gönderen + bildirim kimliği).
    NotifyImage,
    /// Bildirimin kaynak anahtarı: gönderen sürecin kimliği (çalıştırılabilir + cgroup; susturma
    /// gönderen başına, istemcinin bildirdiği uygulama adı değil).
    NotifySender,
    /// Medya kapağı (MPRIS oynatıcı bus adı).
    Media,
}

/// Süreç başına rastgele anahtarlı SipHash: anahtar dışarıdan tahmin edilemez, böylece bir
/// uygulama başka bir kaynağın anahtarıyla çakışan ad seçip onu susturamaz.
fn hasher() -> &'static RandomState {
    static STATE: OnceLock<RandomState> = OnceLock::new();
    STATE.get_or_init(RandomState::new)
}

/// Kaynağın Loader/Sandbox anahtarı. Hiçbir zaman [`NO_KEY`] (0) değildir: 0 "anahtarsız"
/// demektir ve birleştirme/susturma uygulanmaz. Süreç içinde kararlıdır. Anahtar loglanmaz.
pub fn source_key<T: Hash + ?Sized>(domain: KeyDomain, id: &T) -> u64 {
    match hasher().hash_one((domain, id)) {
        NO_KEY => 1,
        h => h,
    }
}

/// Açılışta bir kez: varsayılan ayarlarla paylaşılan Sandbox (süreç başlatmaz; tembel).
pub fn new_shared() -> Arc<Sandbox> {
    let sandbox = Arc::new(Sandbox::new(SandboxConfig::default()));
    tracing::info!(isolation = ?sandbox.isolation(), "görsel yalıtımı");
    sandbox
}

/// Açılışta bir kez: sürecin paylaşılan duvar kağıdı çözücüsü (duvar kağıdı profilli kendi
/// `Sandbox`'ı; varsayılan önbellek 96 MiB, `linger` 10 sn). Süreç ve thread başlatmaz (tembel).
pub fn new_wallpaper_decoder() -> Arc<WallpaperDecoder> {
    let sandbox = Sandbox::new(SandboxConfig::wallpaper());
    tracing::info!(isolation = ?sandbox.isolation(), "duvar kağıdı yalıtımı");
    wallpaper_decoder_with(Arc::new(sandbox))
}

/// [`new_wallpaper_decoder`]'ın çözücüsü verilmiş biçimi (testler: işçisiz `Sandbox`, sahte çözücü).
pub fn wallpaper_decoder_with(decoder: Arc<dyn IsolatedDecoder>) -> Arc<WallpaperDecoder> {
    WallpaperDecoder::new(decoder, WallpaperDecoderOptions::default())
}

/// Yalıtım Loader'lara bağlı mı. Framework güvenlik incelemesi (11065ae) ve işçi paketi
/// ayrımıyla AÇIK; install.sh işçiyi kurar (`INSTALL_HELPERS=1`). Kapatmak
/// (`false`) Loader'ları sandbox'sız bırakır (güvenilmeyen SVG reddedilir, raster süreç içinde)
/// ve `msg images status` `sandbox: false` döner.
pub const ENABLED: bool = true;

/// `Loader::with_sandbox`'ın beklediği tür; [`ENABLED`] kapalıyken `None`.
pub fn decoder(sandbox: &Arc<Sandbox>) -> Option<Arc<dyn IsolatedDecoder>> {
    ENABLED.then(|| isolated(sandbox))
}

/// Kapıdan bağımsız çözücü (testler ve [`decoder`]).
pub fn isolated(sandbox: &Arc<Sandbox>) -> Arc<dyn IsolatedDecoder> {
    Arc::clone(sandbox) as Arc<dyn IsolatedDecoder>
}

/// `msg images status` için: [`ENABLED`] kapalıyken `None` (`sandbox: false`).
pub fn active(sandbox: &Sandbox) -> Option<&Sandbox> {
    ENABLED.then_some(sandbox)
}

fn isolation_name(i: Isolation) -> &'static str {
    match i {
        Isolation::Process => "process",
        Isolation::Unavailable => "unavailable",
    }
}

/// `msg images status`'un `wallpaper` nesnesi: paylaşılan duvar kağıdı çözücüsünün sayaçları
/// (çok çıkışta `decodes` 1, diğerleri `joined`/`hits`/`derived`).
fn wallpaper_json(s: &WallpaperDecodeStats) -> serde_json::Value {
    serde_json::json!({
        "decodes": s.decodes,
        "hits": s.hits,
        "joined": s.joined,
        "derived": s.derived,
        "cancelled": s.cancelled,
        "failed": s.failed,
        "resident_bytes": s.resident_bytes,
        "pending": s.pending,
    })
}

/// `msg images status`: tek satır JSON, içerik (yol, ad, anahtar) taşımaz. Çevrilmez
/// (ADR-0010: `msg` çıktısı makine içindir).
pub fn status_json(sandbox: Option<&Sandbox>, wallpaper: &WallpaperDecodeStats) -> String {
    let Some(sb) = sandbox else {
        return serde_json::json!({ "sandbox": false, "isolation": "none", "wallpaper": wallpaper_json(wallpaper) }).to_string();
    };
    let s = sb.stats();
    serde_json::json!({
        "wallpaper": wallpaper_json(wallpaper),
        "sandbox": true,
        "isolation": isolation_name(sb.isolation()),
        "spawned": s.spawned,
        "live": s.live,
        "idle": s.idle,
        "decoded": s.decoded,
        "failures": s.failures,
        "rate_limited": s.rate_limited,
        "negative_hits": s.negative_hits,
        "in_process_fallbacks": s.in_process_fallbacks,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAINS: [KeyDomain; 5] = [KeyDomain::Tray, KeyDomain::NotifyAppIcon, KeyDomain::NotifyImage, KeyDomain::NotifySender, KeyDomain::Media];

    #[test]
    fn source_key_is_never_zero_and_separates_domains() {
        for id in ["", "a", ":1.42/StatusNotifierItem", "org.mpris.MediaPlayer2.mpv", "Firefox"] {
            for d in DOMAINS {
                assert_ne!(source_key(d, id), NO_KEY, "{d:?} {id:?}");
                // Süreç içinde kararlı (aynı kaynak aynı anahtar: birleştirme ve susturma için).
                assert_eq!(source_key(d, id), source_key(d, id));
            }
        }
        for n in 0..1000u32 {
            assert_ne!(source_key(KeyDomain::NotifyImage, &("app", n)), NO_KEY);
        }
        assert_ne!(source_key(KeyDomain::NotifyAppIcon, "x"), source_key(KeyDomain::NotifyImage, "x"));
        assert_ne!(source_key(KeyDomain::Tray, "x"), source_key(KeyDomain::Media, "x"));
        assert_ne!(source_key(KeyDomain::Tray, "a"), source_key(KeyDomain::Tray, "b"));
        // Aynı uygulamanın farklı bildirimleri ayrı anahtar (birbirinin görselini iptal etmez).
        assert_ne!(source_key(KeyDomain::NotifyImage, &("a", 1u32)), source_key(KeyDomain::NotifyImage, &("a", 2u32)));
    }

    #[test]
    fn status_json_carries_counts_only() {
        let none: serde_json::Value = serde_json::from_str(&status_json(None, &WallpaperDecodeStats::default())).unwrap();
        assert_eq!(none["sandbox"], false);
        // Var olmayan işçi yolu: Unavailable, süreç yok.
        let sb = Sandbox::new(SandboxConfig { worker: Some("/nonexistent/nuxut-image-worker".into()), ..SandboxConfig::default() });
        let text = status_json(Some(&sb), &WallpaperDecodeStats { decodes: 1, joined: 2, ..WallpaperDecodeStats::default() });
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["sandbox"], true);
        assert_eq!(v["isolation"], "unavailable");
        assert_eq!(v["spawned"], 0);
        assert_eq!(v["wallpaper"]["decodes"], 1);
        assert_eq!(v["wallpaper"]["joined"], 2);
        assert!(!text.contains("nonexistent"), "{text}");
    }
}
