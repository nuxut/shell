//! Kapak görseli yükleme: yalnızca istek üzerine, ayrı thread'de. Yerel dosya (`file://`)
//! [`safe_file`] kurallarıyla; ağ kapağı (`https://`) framework'ün SSRF-güvenli istemcisiyle
//! ([`HttpClient::fetch_untrusted_media`]: yalnızca genel internet, yönlendirmeler dahil;
//! yalnızca https; boyut ve süre sınırlı). Her iki durumda içerik bilinen bir görsel imzası
//! taşımalı. Aynı anda tek iş; kuyrukta bekleyen eski istekler en yenisiyle değiştirilir.
//! Son başarılı sonuçlar önbellekte tutulur.

use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use nuxut_net::HttpClient;

use super::model::{ArtSource, parse_art_url};
use crate::services::safe_file;

/// Kapak dosyasının üst sınırı.
pub const MAX_ART_BYTES: u64 = 8 * 1024 * 1024;
/// Önbellekte tutulan sonuç sayısı.
const CACHE: usize = 4;

type Done = Box<dyn Fn(String, Result<Arc<[u8]>, String>) + Send>;
type Worker = (Sender<(String, Done)>, JoinHandle<()>);

#[derive(Default)]
pub(crate) struct ArtCache {
    cache: Vec<(String, Arc<[u8]>)>,
    worker: Option<Worker>,
}

impl std::fmt::Debug for ArtCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtCache").field("cached", &self.cache.len()).finish()
    }
}

impl ArtCache {
    pub fn get(&self, url: &str) -> Option<Arc<[u8]>> {
        self.cache.iter().find(|(u, _)| u == url).map(|(_, d)| d.clone())
    }

    pub fn store(&mut self, url: &str, data: Arc<[u8]>) {
        self.cache.retain(|(u, _)| u != url);
        self.cache.insert(0, (url.to_owned(), data));
        self.cache.truncate(CACHE);
    }

    /// İndirmeyi kuyruğa koyar; `done` indirme thread'inden çağrılır.
    pub fn fetch(&mut self, url: String, done: impl Fn(String, Result<Arc<[u8]>, String>) + Send + 'static) {
        if self.worker.is_none() {
            let (tx, rx) = mpsc::channel::<(String, Done)>();
            let spawned = std::thread::Builder::new().name("nuxut-media-art".into()).spawn(move || {
                // İlk ağ kapağında kurulur (TLS kökleri yüklenir); yerel kapakta hiç kurulmaz.
                let mut http: Option<HttpClient> = None;
                while let Ok(first) = rx.recv() {
                    // Bekleyenlerden yalnızca en yenisi.
                    let (url, done) = rx.try_iter().last().unwrap_or(first);
                    let result = load(&url, || http.get_or_insert_with(media_client));
                    if let Err(e) = &result {
                        tracing::debug!("kapak okunamadı: {e}");
                    }
                    done(url, result);
                }
            });
            match spawned {
                Ok(thread) => self.worker = Some((tx, thread)),
                Err(e) => {
                    done(url, Err(format!("thread başlatılamadı: {e}")));
                    return;
                }
            }
        }
        if let Some((tx, _)) = &self.worker {
            let _ = tx.send((url, Box::new(done)));
        }
    }
}

impl Drop for ArtCache {
    fn drop(&mut self) {
        if let Some((tx, thread)) = self.worker.take() {
            drop(tx);
            // Okuma sürüyorsa bitmesini bekler (yerel dosya, boyut sınırlı).
            let _ = thread.join();
        }
    }
}

/// Kapak indirme istemcisi: varsayılan (en sıkı) politika; loopback/özel ağ hiçbir zaman.
fn media_client() -> HttpClient {
    HttpClient::new()
}

/// Kapağı yükler. `file://`: [`safe_file::read_untrusted`]. Ağ: oynatıcının verdiği adres
/// güvenilmez (yerel ağa/loopback'e/metadata adresine istek attırabilir), bu yüzden yalnızca
/// [`HttpClient::fetch_untrusted_media`] ile; hedef denetimi ve sınırlar istemcide.
fn load<'a>(url: &str, http: impl FnOnce() -> &'a HttpClient) -> Result<Arc<[u8]>, String> {
    let data = match parse_art_url(url) {
        Some(ArtSource::File(path)) => safe_file::read_untrusted(&path, MAX_ART_BYTES).map_err(|e| e.to_string())?,
        Some(ArtSource::Remote(u)) => {
            let u = nuxut_net::Url::parse(&u).map_err(|_| "geçersiz kapak adresi".to_owned())?;
            let r = http().fetch_untrusted_media(u).map_err(|e| e.to_string())?;
            if !r.is_success() {
                return Err(format!("kapak alınamadı (HTTP {})", r.status.as_u16()));
            }
            if r.body.len() as u64 > MAX_ART_BYTES {
                return Err("kapak çok büyük".into());
            }
            r.body
        }
        None => return Err("desteklenmeyen kapak adresi".into()),
    };
    if !looks_like_image(&data) {
        return Err("görsel değil".into());
    }
    Ok(data.into())
}

/// Bilinen görsel biçimlerinin imzası (PNG, JPEG, GIF, WebP, BMP, AVIF/HEIF, JPEG XL, SVG).
fn looks_like_image(d: &[u8]) -> bool {
    d.starts_with(b"\x89PNG\r\n\x1a\n")
        || d.starts_with(&[0xff, 0xd8, 0xff])
        || d.starts_with(b"GIF8")
        || (d.starts_with(b"RIFF") && d.get(8..12) == Some(b"WEBP"))
        || d.starts_with(b"BM")
        || d.get(4..8) == Some(b"ftyp")
        || d.starts_with(&[0xff, 0x0a])
        || d.starts_with(b"\0\0\0\x0cJXL ")
        || {
            let head = String::from_utf8_lossy(&d[..d.len().min(512)]);
            let t = head.trim_start_matches('\u{feff}').trim_start();
            t.starts_with("<svg") || (t.starts_with("<?xml") && head.contains("<svg"))
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_is_bounded_and_mru() {
        let mut c = ArtCache::default();
        for i in 0..6 {
            c.store(&format!("u{i}"), Arc::from(vec![i as u8]));
        }
        assert!(c.get("u0").is_none());
        assert_eq!(c.get("u5").unwrap()[0], 5);
        c.store("u2", Arc::from(vec![9u8]));
        assert_eq!(c.get("u2").unwrap()[0], 9);
        assert_eq!(c.cache.len(), CACHE);
    }

    #[test]
    fn image_sniffing() {
        assert!(looks_like_image(b"\x89PNG\r\n\x1a\nrest"));
        assert!(looks_like_image(&[0xff, 0xd8, 0xff, 0xe0]));
        assert!(looks_like_image(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(looks_like_image(b"\0\0\0\x1cftypavif"));
        assert!(looks_like_image(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"\"/>"));
        assert!(!looks_like_image(b"root:x:0:0:root:/root:/bin/bash"));
        assert!(!looks_like_image(b""));
    }

    /// Yerel kapak: istemci hiç kurulmamalı.
    fn load_local(url: &str) -> Result<Arc<[u8]>, String> {
        load(url, || unreachable!("yerel kapakta HTTP istemcisi istenmez"))
    }

    /// Test istemcisi: test CA'sına güvenir, her adı loopback'e çözer (sahte sunucuya ulaşmak
    /// için `allow_insecure_loopback` yalnızca burada).
    fn test_client(ca: &[u8]) -> HttpClient {
        use nuxut_net::testing::LoopbackLookup;
        let policy = nuxut_net::Policy { allow_insecure_loopback: true, ..Default::default() };
        HttpClient::with_test_tls(policy, ca, Arc::new(LoopbackLookup))
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000";

    #[test]
    fn production_client_keeps_strict_policy() {
        let policy = media_client().policy();
        assert_eq!(policy, nuxut_net::Policy::default());
        assert!(!policy.allow_insecure_loopback);
    }

    #[test]
    fn remote_art_is_fetched_over_tls() {
        use nuxut_net::testing::{FakeServer, Reply, TEST_CA_DER};
        let server = FakeServer::start_tls(|_| Reply::status(200).bytes(PNG.to_vec()));
        let http = test_client(TEST_CA_DER);
        let url = server.url_at("nuxut.test", "/cover.png").to_string();
        let data = load(&url, || &http).expect("kapak");
        assert_eq!(&data[..], PNG);
        assert_eq!(server.requests().len(), 1);
    }

    #[test]
    fn remote_art_rejects_wrong_ca_status_size_and_content() {
        use nuxut_net::testing::{FakeServer, OTHER_CA_DER, Reply, TEST_CA_DER};
        let big = MAX_ART_BYTES as usize + 1;
        let server = FakeServer::start_tls(move |r| match r.path.as_str() {
            "/yok.png" => Reply::status(404),
            "/buyuk.png" => Reply::status(200).bytes(PNG.iter().copied().chain(std::iter::repeat_n(0, big)).collect()),
            "/metin.png" => Reply::status(200).body("root:x:0:0"),
            _ => Reply::status(200).bytes(PNG.to_vec()),
        });
        let url = |p: &str| server.url_at("nuxut.test", p).to_string();

        let wrong_ca = test_client(OTHER_CA_DER);
        assert!(load(&url("/cover.png"), || &wrong_ca).is_err(), "yanlış CA");
        assert!(server.requests().is_empty(), "TLS hatasında istek okunmamalı");

        let http = test_client(TEST_CA_DER);
        assert!(load(&url("/yok.png"), || &http).unwrap_err().contains("404"));
        assert!(load(&url("/buyuk.png"), || &http).unwrap_err().contains("büyük"), "boyut sınırı");
        assert_eq!(load(&url("/metin.png"), || &http).unwrap_err(), "görsel değil");
    }

    #[test]
    fn remote_art_never_reaches_private_targets() {
        let http = HttpClient::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        for url in [
            format!("https://127.0.0.1:{port}/kapak.png"),
            format!("http://127.0.0.1:{port}/kapak.png"),
            format!("https://localhost:{port}/kapak.png"),
            "https://169.254.169.254/latest/meta-data/".into(),
            "https://10.0.0.1/kapak.png".into(),
            "https://192.168.1.1/kapak.png".into(),
            "https://[::1]/kapak.png".into(),
            "https://[fd00::1]/kapak.png".into(),
            "http://example.com/kapak.png".into(),
        ] {
            assert!(load(&url, || &http).is_err(), "{url}");
        }
        assert!(listener.accept().is_err(), "loopback'e bağlantı yapıldı");
    }

    #[test]
    fn local_files_go_through_safe_file() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("kapak.png");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\n0000").unwrap();
        let url = |p: &std::path::Path| url::Url::from_file_path(p).unwrap().to_string();
        assert!(load_local(&url(&png)).is_ok());
        let link = dir.path().join("bag.png");
        std::os::unix::fs::symlink(&png, &link).unwrap();
        assert!(load_local(&url(&link)).is_err(), "son bileşen symlink");
        let text = dir.path().join("metin.png");
        std::fs::write(&text, b"gizli").unwrap();
        assert!(load_local(&url(&text)).is_err(), "görsel olmayan içerik");
        assert!(load_local("file:///proc/self/environ").is_err());
        assert!(load_local("data:image/png;base64,AAAA").is_err());
    }
}
