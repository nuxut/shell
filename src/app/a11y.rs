//! Ekran okuyucu (framework ADR-0011): ana yüzeylerin AT-SPI ağacı, AT eylemleri ve klavye
//! gezinmesinin etkinleştirmesi.
//!
//! **Güvenlik (2026-10-17, HIGH bulgu):** a11y bus'ı oturumdaki her sürece (Flatpak'te
//! varsayılan izinli) açıktır; AT eylemi gerçek kullanıcı niyeti değildir. Kural: **AT yalnız
//! okur, odak taşır ve gezinir** ([`Source::Assistive`]). AT `Click`'i kabuğun kendi işaretçi
//! işleyicisine benzetilir ama sonuç eylemi yalnız gezinmeyse uygulanır (yaprak aç/kapat,
//! çalışma alanı, takvim ayı: [`assistive_allowed`]); uygulama başlatma, pano öğesi seçme,
//! Bluetooth eşleştirme onayı, ağ bağlantısı, oturum eylemleri, bildirim eylemleri, ayar
//! anahtarları ve kaydırıcı değeri AT'den yapılamaz. Bunlar **gerçek girdiyle** olur: işaretçi ya
//! da kabuğun klavye gezinmesi (Tab/oklar → Enter; Wayland klavye olayı, [`Source::Keyboard`]).
//! Orca kullanıcısı odağı taşır, onayı kendi klavyesi verir. AT `Focus` gezinme odağını taşır.
//!
//! Benzetilen tıklama: düğümün ortasına hareket → bas → bırak, ardından işaretçi eski yerine.
//! Kullanıcının pencerelerine girdi gönderilmez: olay yalnız kabuğun kendi işleyicisine verilir.

use std::time::Instant;

use nuxut::access::{AccessAction, AccessRequest, AccessTree};
use nuxut::{Context, PointerEvent, PointerKind, SurfaceId};

use super::Shell;
use crate::widgets::Action;

/// Etkinleştirmenin kaynağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    /// a11y bus'ından gelen AT isteği: yalnız gezinme.
    Assistive,
    /// Kabuğun klavye gezinmesi (gerçek Wayland tuşu): tam etkinleştirme.
    Keyboard,
}

/// AT tıklamasının sonucu olarak uygulanabilen eylemler: yalnız gezinme (erişim ya da kalıcı
/// değişiklik vermeyen). Liste dışı her eylem AT'den gelince atılır.
pub(super) fn assistive_allowed(a: &Action) -> bool {
    matches!(
        a,
        Action::TogglePanel(_)
            | Action::ClosePanel(_)
            | Action::PinPanel(..)
            | Action::SwitchWorkspace(_)
            | Action::RefreshInput
            | Action::CalendarRange { .. }
    )
}

impl Shell {
    pub(super) fn accessibility_inner(&mut self, cx: &mut Context<Self>, surface: SurfaceId, tree: &mut AccessTree) {
        if self.osk_access(surface, tree) {
            return;
        }
        let theme = cx.theme().clone();
        let config = self.model.config.clone();
        let text = cx.text();
        if let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) {
            s.access(tree, text, &config, &theme, Instant::now());
        }
    }

    /// a11y bus'ından gelen istek: `Focus` gezinme odağını taşır, `Click` yalnız gezinme
    /// sonucuyla; diğer her şey (kaydırıcı değeri, metin) reddedilir.
    pub(super) fn accessibility_action_inner(&mut self, cx: &mut Context<Self>, surface: SurfaceId, request: AccessRequest) {
        match request.action {
            AccessAction::Focus => {
                if let Some(s) = self.screens.iter_mut().find(|s| s.main == surface)
                    && s.nav_focus_to(request.node)
                {
                    cx.request_frame(surface);
                    cx.a11y_invalidate(surface);
                }
            }
            AccessAction::Click => self.activate(cx, surface, request, Source::Assistive),
            _ => tracing::info!(action = ?request.action, "AT eylemi reddedildi (yalnız gezinme ve odak)"),
        }
    }

    /// Düğümü etkinleştirir. Klavyeden: bileşenin kendi eylemi (kaydırıcı adımı, bildirim kartı)
    /// ya da düğümün ortasına tıklama, sonuç eylemi uygulanır. AT'den: yalnız benzetilen
    /// tıklama ve yalnız [`assistive_allowed`] sonuçlar.
    pub(super) fn activate(&mut self, cx: &mut Context<Self>, surface: SurfaceId, request: AccessRequest, source: Source) {
        let theme = cx.theme().clone();
        let config = self.model.config.clone();
        if source == Source::Keyboard
            && let Some(s) = self.screens.iter_mut().find(|s| s.main == surface)
            && let Some((redraw, action)) = s.access_action(&request, &config, &theme, Instant::now())
        {
            if redraw {
                cx.request_frame(surface);
            }
            if let Some(a) = action {
                self.act(cx, surface, a);
            }
            return;
        }
        if request.action != AccessAction::Click {
            return;
        }
        let Some(s) = self.screens.iter().find(|s| s.main == surface) else { return };
        let Some(at) = s.access_point(request.node) else { return };
        let before = s.pointer_position();
        let left = nuxut::Button::Left;
        let mut kinds = vec![PointerKind::Motion, PointerKind::Press(left), PointerKind::Release(left)];
        kinds.push(match before {
            Some(_) => PointerKind::Motion,
            None => PointerKind::Leave,
        });
        let mut dropped = false;
        for (i, kind) in kinds.into_iter().enumerate() {
            let p = if i == 3 { before.unwrap_or(at) } else { at };
            let event = PointerEvent::new(p, kind);
            if source == Source::Keyboard {
                nuxut::App::pointer(self, cx, surface, event);
                continue;
            }
            let now = Instant::now();
            let text = cx.text();
            let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) else { return };
            let (redraw, action) = s.pointer(text, &config, &theme, event, now);
            if redraw {
                cx.request_frame(surface);
            }
            match action {
                Some(a) if assistive_allowed(&a) => self.act(cx, surface, a),
                Some(_) => {
                    // Değişmez 9: eylem içeriği yazılmaz.
                    tracing::info!("AT tıklaması reddedildi: sonuç gezinme değil (gerçek girdi gerekir)");
                    dropped = true;
                }
                None => {}
            }
        }
        if dropped {
            // Bileşenin iç durumu (anahtar görünümü, onay hapı) modele geri dönsün.
            self.model_changed(cx);
        }
        self.refresh_inputs(cx);
    }
}

impl Shell {
    /// Klavye gezinmesi: tuş gezinme tuşuysa işlenir (`true`). Etkinleştirme ve kaydırıcı adımı
    /// AT eylemiyle aynı yoldan gider; odak değişince ağaç yenilenir (Orca yeni odağı okur).
    pub(super) fn nav_key(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: &nuxut::KeyEvent) -> bool {
        let theme = cx.theme().clone();
        let config = self.model.config.clone();
        let now = Instant::now();
        let outcome = {
            let text = cx.text();
            let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) else { return false };
            s.nav_key(text, &config, &theme, event, now)
        };
        let Some(outcome) = outcome else { return false };
        match outcome {
            crate::screen::NavOutcome::Moved => {}
            crate::screen::NavOutcome::Exit => {
                if let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) {
                    Self::apply_input(cx, s, &config);
                }
            }
            crate::screen::NavOutcome::Action(request) => {
                let click = request.action == AccessAction::Click;
                // Gerçek tuş: tam etkinleştirme.
                self.activate(cx, surface, request, Source::Keyboard);
                if click {
                    let text = cx.text();
                    if let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) {
                        s.nav_follow_opened(text, &config, &theme, Instant::now());
                    }
                }
            }
        }
        cx.request_frame(surface);
        cx.a11y_invalidate(surface);
        true
    }

    /// `msg bar focus`: monitörün bar'ında klavye gezinmesi başlar (ekran okuyucu ve yalnız klavye
    /// kullananlar için bar'a giriş yolu; Escape çıkar).
    pub(super) fn bar_focus(&mut self, cx: &mut Context<Self>, idx: usize) -> Result<String, String> {
        let theme = cx.theme().clone();
        let config = self.model.config.clone();
        let text = cx.text();
        let s = &mut self.screens[idx];
        if !s.start_bar_nav(text, &config, &theme, Instant::now()) {
            return Err("bar görünmüyor ya da boş".into()); // i18n: tanı msg yanıtı (çevrilmez)
        }
        Self::apply_input(cx, s, &config);
        cx.request_frame(s.main);
        cx.a11y_invalidate(s.main);
        Ok("ok".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{CopyText, PanelId};

    /// AT tıklaması yalnız gezinme sonuçlarını uygular; erişim ya da kalıcı değişiklik veren
    /// her eylem (uygulama başlatma, pano, oturum, bildirim eylemi, ayar, büyüteç…) atılır.
    #[test]
    fn assistive_clicks_only_navigate() {
        assert!(assistive_allowed(&Action::TogglePanel(PanelId::Clock)));
        assert!(assistive_allowed(&Action::SwitchWorkspace(2)));
        assert!(assistive_allowed(&Action::CalendarRange { from: 0, to: 1 }));
        for a in [
            Action::Session(crate::services::session::SessionAction::PowerOff),
            Action::IdleInhibit(true),
            Action::SetThemeMode(nuxut::theme::ThemeMode::Light),
            Action::Magnifier(crate::services::magnifier::Command::On),
            Action::Copy(CopyText::new("x".into())),
            Action::RequestPaste,
            Action::KeyboardLayoutNext,
            Action::Notify(crate::widgets::NotifyCommand::Invoke { id: 1, key: "default".into() }),
            Action::Clipboard(crate::services::clipboard::ClipboardCommand::Clear),
            Action::PickEmoji("😀"),
        ] {
            assert!(!assistive_allowed(&a), "{a:?}");
        }
    }
}
