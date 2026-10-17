//! Pano: odaklı metin alanının Ctrl+V isteği → `request_paste` → sonuç isteyen yüzeyin açık
//! paneline; kopyala/kes → `set_clipboard`. Metin kullanıcı verisidir: loglanmaz, kopyası
//! tutulmaz (okunduktan sonra sıfırlanır).

use std::time::Instant;

use nuxut::clipboard::{ClipboardText, PasteError, Selection};
use nuxut::wayland::PasteRequestId;
use nuxut::{Context, SurfaceId};
use zeroize::Zeroizing;

use super::Shell;

impl Shell {
    /// Yalnızca son istek geçerli: arka arkaya Ctrl+V'de eskisinin sonucu yok sayılır.
    pub(super) fn request_paste(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {
        self.paste_request = Some((cx.request_paste(Selection::Clipboard), surface));
    }

    /// Seçili metni (kopyala/kes) panoya yazar.
    pub(super) fn copy_to_clipboard(&mut self, cx: &mut Context<Self>, text: &str) {
        if let Err(e) = cx.set_clipboard(Selection::Clipboard, text) {
            tracing::debug!(error = %e, "panoya yazılamadı");
        }
    }

    pub(super) fn on_paste(
        &mut self,
        cx: &mut Context<Self>,
        id: PasteRequestId,
        result: Result<ClipboardText, PasteError>,
    ) {
        let Some((_, surface)) = self.paste_request.take_if(|(r, _)| *r == id) else { return };
        let text = match result {
            Ok(t) => Zeroizing::new(t.into_string()),
            Err(e) => {
                tracing::debug!(error = %e, "yapıştırılamadı");
                return;
            }
        };
        let theme = cx.theme().clone();
        let config = self.model.config.clone();
        let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) else { return };
        let Some((redraw, action)) = s.paste(&config, &theme, &text, Instant::now()) else { return };
        if redraw {
            cx.request_frame(surface);
        }
        if let Some(a) = action {
            self.act(cx, surface, a);
        }
        // Başlatıcı aramasının yeni sonuçları ikon isteyebilir.
        self.pump_launcher_icons(cx);
    }
}
