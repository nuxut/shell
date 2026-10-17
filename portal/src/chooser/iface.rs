//! `org.freedesktop.impl.portal.FileChooser` ve istek başına `org.freedesktop.impl.portal.Request`.
//!
//! Her çağrı: seçenekler → [`ChooserRequest`] → `nuxut-filechooser` süreci ([`child::spawn`]) →
//! sonuç doğrulanır ([`options::finish`]) → `(yanıt, sonuçlar)`. Çağrı sürerken `handle` yolunda
//! `Request` nesnesi sunulur; `Close` seçiciyi iptal eder ve çağrı 1 ile döner. Çağrılar zbus
//! yürütücüsünde eşzamanlı yürür (bekleme async kanalda; bloklayan iş seçici thread'inde).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use zbus::object_server::ObjectServer;
use zbus::zvariant::OwnedObjectPath;

use super::child::{self, Outcome};
use super::options::{self, Options, RealFs, Results};
use super::protocol::Mode;

/// Yanıt kodları.
pub const RESPONSE_OK: u32 = 0;
pub const RESPONSE_CANCELLED: u32 = 1;
pub const RESPONSE_OTHER: u32 = 2;

/// Aynı anda en çok açık seçici.
pub const MAX_CONCURRENT: usize = 8;

/// Seçici ikilisinin adı (`$LIBEXEC`'te, portalın yanında).
pub const CHOOSER_BIN: &str = "nuxut-filechooser";
/// Test/geliştirme: seçici ikilisinin yolu.
pub const CHOOSER_BIN_ENV: &str = "NUXUT_FILECHOOSER_BIN";

/// Seçici ikilisi: [`CHOOSER_BIN_ENV`] → çalışan portal ikilisinin dizini.
pub fn locate_bin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(CHOOSER_BIN_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(CHOOSER_BIN))
}

/// D-Bus nesnesi.
#[derive(Debug, Clone)]
pub struct FileChooser {
    bin: Option<PathBuf>,
    active: Arc<AtomicUsize>,
}

/// Etkin seçici sayacını düşüren bekçi.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl FileChooser {
    /// `bin`: seçici ikilisi (`None`: her çağrı 2 döner).
    pub fn new(bin: Option<PathBuf>) -> Self {
        Self { bin, active: Arc::new(AtomicUsize::new(0)) }
    }

    async fn run(&self, server: &ObjectServer, handle: OwnedObjectPath, mode: Mode, (app_id, parent_window, title): (&str, &str, &str), options: &Options) -> (u32, Results) {
        let fail = (RESPONSE_OTHER, Results::new());
        let Some(bin) = self.bin.as_deref() else {
            tracing::warn!(?mode, "seçici ikilisi yok");
            return fail;
        };
        if self.active.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT {
            self.active.fetch_sub(1, Ordering::SeqCst);
            tracing::warn!(?mode, "çok sayıda açık seçici; istek reddedildi");
            return fail;
        }
        let _slot = Slot(Arc::clone(&self.active));
        if let Err(reason) = options::validate(mode, options) {
            tracing::warn!(?mode, reason, "istek reddedildi");
            return fail;
        }
        let request = options::request(mode, app_id, parent_window, title, options);
        tracing::info!(?request, "seçici açılıyor");

        let (done_tx, done_rx) = async_channel::bounded::<Outcome>(1);
        let running = match child::spawn(bin, &request, move |o| {
            let _ = done_tx.try_send(o);
        }) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(kind = ?e.kind(), "seçici başlatılamadı");
                return fail;
            }
        };
        let (close_tx, close_rx) = async_channel::bounded::<()>(1);
        let exported = match server.at(&handle, Request { close: close_tx }).await {
            Ok(true) => true,
            Ok(false) => {
                tracing::warn!("istek yolu zaten kullanımda");
                false
            }
            Err(e) => {
                tracing::warn!("istek nesnesi sunulamadı: {e}");
                false
            }
        };

        let outcome = futures_lite::future::or(async { done_rx.recv().await.unwrap_or(Outcome::Failed("seçici sonucu kayboldu")) }, async {
            // Close gelmezse (gönderen düşer) sonsuza dek bekler.
            if close_rx.recv().await.is_ok() {
                running.cancel();
                // Seçici kapanana dek bekle (Close'dan sonra ikinci pencere kalmasın).
                let _ = done_rx.recv().await;
                Outcome::Cancelled
            } else {
                std::future::pending().await
            }
        })
        .await;

        if exported {
            let _ = server.remove::<Request, _>(&handle).await;
        }
        match outcome {
            Outcome::Selected(result) => match options::finish(&request, &result, &RealFs) {
                Ok(results) => {
                    tracing::info!(?mode, ?result, "seçildi");
                    (RESPONSE_OK, results)
                }
                Err(reason) => {
                    tracing::warn!(?mode, reason, "seçici sonucu reddedildi");
                    fail
                }
            },
            Outcome::Cancelled => (RESPONSE_CANCELLED, Results::new()),
            Outcome::Failed(reason) => {
                tracing::warn!(?mode, reason, "seçici başarısız");
                fail
            }
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooser {
    #[zbus(out_args("response", "results"))]
    async fn open_file(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: Options,
    ) -> (u32, Results) {
        self.run(server, handle, Mode::Open, (&app_id, &parent_window, &title), &options).await
    }

    #[zbus(out_args("response", "results"))]
    async fn save_file(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: Options,
    ) -> (u32, Results) {
        self.run(server, handle, Mode::Save, (&app_id, &parent_window, &title), &options).await
    }

    #[zbus(out_args("response", "results"))]
    async fn save_files(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        handle: OwnedObjectPath,
        app_id: String,
        parent_window: String,
        title: String,
        options: Options,
    ) -> (u32, Results) {
        self.run(server, handle, Mode::SaveFiles, (&app_id, &parent_window, &title), &options).await
    }
}

/// `org.freedesktop.impl.portal.Request`: yalnızca `Close`.
#[derive(Debug)]
struct Request {
    close: async_channel::Sender<()>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    fn close(&self) {
        let _ = self.close.try_send(());
    }
}
