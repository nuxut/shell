//! Canlı görsel test kaynağı (yalnızca `clipboard-demo` özelliği + `NUXUT_CLIPBOARD_DEMO=1`).
//!
//! Framework `DataControl` gelene dek başlatıcının pano kipini izole test compositor'ında
//! görebilmek için birkaç sabit teklifi aralıklı sunar (art arda gelen tekliflerden yalnızca
//! sonuncusu okunduğu için). Gerçek panoya dokunmaz: `set_selection` yalnızca kabul eder.
//! Ürün derlemesine girmez.

use std::sync::Arc;
use std::time::Duration;

use zeroize::Zeroizing;

use super::{ClipboardSource, Inbox, OfferId, ReadError, SetError, SourceEvent, channel, filter};

/// Sunulan sabit içerik: (MIME, bayt).
struct Demo {
    offers: Vec<(&'static str, Vec<u8>)>,
}

impl ClipboardSource for Demo {
    fn read(&self, id: OfferId, mime: &str, max_bytes: usize, _timeout: Duration) -> Result<Zeroizing<Vec<u8>>, ReadError> {
        let (m, bytes) = self.offers.get(id.0 as usize).ok_or(ReadError::Gone)?;
        if *m != mime {
            return Err(ReadError::Gone);
        }
        if bytes.len() > max_bytes {
            return Err(ReadError::TooLarge);
        }
        Ok(Zeroizing::new(bytes.clone()))
    }

    fn set_selection(&self, _data: Vec<(String, Arc<[u8]>)>) -> Result<(), SetError> {
        Ok(())
    }
}

/// Küçük bir degrade PNG (küçük resim yolunu göstermek için).
fn gradient_png() -> Vec<u8> {
    let (w, h) = (160u32, 96u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 200, 255]);
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    if let Ok(mut wr) = enc.write_header() {
        let _ = wr.write_image_data(&px);
    }
    out
}

/// Ortam değişkeni açıksa sahte kaynak ve gelen kutusu; teklifler ayrı thread'den aralıklı gelir.
pub fn connect() -> Option<(Arc<dyn ClipboardSource>, Inbox)> {
    if std::env::var_os("NUXUT_CLIPBOARD_DEMO").is_none_or(|v| v != "1") {
        return None;
    }
    let offers: Vec<(&'static str, Vec<u8>)> = vec![
        ("text/plain;charset=utf-8", b"cargo test --workspace && cargo clippy --all-targets".to_vec()),
        ("image/png", gradient_png()),
        ("text/plain;charset=utf-8", b"Meeting notes\nsecond line is not shown".to_vec()),
        ("text/plain;charset=utf-8", b"https://example.org/nuxut".to_vec()),
        ("text/plain;charset=utf-8", b"Hello world - Nuxut clipboard history".to_vec()),
    ];
    let mimes: Vec<&'static str> = offers.iter().map(|(m, _)| *m).collect();
    let (sink, inbox) = channel();
    sink.set_available(true);
    let spawned = std::thread::Builder::new().name("nuxut-clipboard-demo".into()).spawn(move || {
        for (i, m) in mimes.into_iter().enumerate() {
            std::thread::sleep(Duration::from_millis(150));
            sink.send(SourceEvent::Offered { id: OfferId(i as u64), mimes: vec![m.to_owned()], flags: filter::Flags::NONE });
        }
    });
    if spawned.is_err() {
        return None;
    }
    tracing::info!("pano geçmişi: demo kaynağı (NUXUT_CLIPBOARD_DEMO)");
    Some((Arc::new(Demo { offers }), inbox))
}
