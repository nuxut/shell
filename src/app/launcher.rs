//! Uygulama başlatıcısı bağlantısı: servis, ikon yükleme, xdg-activation belirteciyle başlatma.
//!
//! Başlatma iki adımdır: `Action::Launch` gelince compositor'dan etkinleştirme belirteci
//! istenir (son girdinin serial'iyle; uygulama odağı alabilsin), belirteç
//! `App::activation_token` ile gelince istek servise verilir. Belirteç yoksa (protokol yok,
//! kilit) yine başlatılır. Belirteç, uygulama adı ve arama metni loglanmaz.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nuxut::{ActivationRequestId, ActivationToken, Context, SurfaceId, TimerId};

use super::{Shell, ShellMessage};
use crate::launcher_view::{ICON_SCALE, ICON_SIZE, LauncherMode};
use crate::services::launcher::{DesktopEntry, LaunchRequest, LauncherEvent, LauncherOptions, LauncherService};
use crate::widgets::{Action, PanelId};

/// Belirteç bekleyen en çok başlatma (compositor yanıt vermezse sınırsız büyümesin).
const MAX_PENDING: usize = 8;

/// Etkinleştirme belirteci için en uzun bekleme; sonra belirteçsiz başlatılır.
const TOKEN_TIMEOUT: Duration = Duration::from_secs(3);

/// Başlatıcı ikonlarını çözen arka plan yükleyicisi ve ikon teması.
#[derive(Debug)]
pub(super) struct LauncherIcons {
    loader: nuxut_image::Loader,
    theme: Arc<nuxut_image::IconTheme>,
}

impl Default for LauncherIcons {
    fn default() -> Self {
        Self { loader: nuxut_image::Loader::new(2), theme: Arc::new(nuxut_image::IconTheme::system()) }
    }
}

impl LauncherIcons {
    /// Güveni `Untrusted`'a düşen ikonlar (Flatpak/Snap dışa aktarımı, ev dizini) shell'in
    /// paylaşılan Sandbox'ında çözülür (ADR-0007): SVG'leri de görünür, süreç içine girmez.
    pub(super) fn with_sandbox(mut self, sandbox: Option<Arc<dyn nuxut_image::IsolatedDecoder>>) -> Self {
        if let Some(sandbox) = sandbox {
            self.loader = std::mem::take(&mut self.loader).with_sandbox(sandbox);
        }
        self
    }
}

/// Başlatıcı ikonunun yükleme isteği: sistem ikon temasından ad ile seçilen ikon `System`
/// istenir. Güveni framework yalnızca düşürür (`IconTheme::load_as`): `.desktop`'taki ad bir
/// yol/URI ise ya da dosya ev dizininde, kullanıcıya ait ya da Flatpak/Snap dışa aktarım
/// dizinindeyse `Untrusted` kalır ve Sandbox işçisinde çözülür (ADR-0007; işçi yoksa SVG çözülmez).
fn icon_request(theme: &Arc<nuxut_image::IconTheme>, name: String) -> nuxut_image::LoadRequest {
    nuxut_image::LoadRequest::icon(theme.clone(), name, ICON_SIZE, ICON_SCALE).with_trust(nuxut_image::Trust::System)
}

impl Shell {
    pub(super) fn start_launcher(&mut self, cx: &mut Context<Self>) {
        let sender = cx.sender();
        let options = LauncherOptions::from_env(self.model.config.launcher.launch_options());
        match LauncherService::spawn(options, move |e| {
            let _ = sender.send(ShellMessage::Launcher(e));
        }) {
            Ok(service) => self.launcher = Some(service),
            Err(e) => tracing::warn!("başlatıcı servisi başlatılamadı: {e}"),
        }
    }

    pub(super) fn on_launcher(&mut self, cx: &mut Context<Self>, e: LauncherEvent) {
        match e {
            LauncherEvent::Catalog(c) => self.model.launcher_catalog = c,
            LauncherEvent::Usage(u) => self.model.launcher_usage = u,
            // Servis hata türünü zaten logladı (ad/komut yazılmaz).
            LauncherEvent::LaunchFailed(_) => return,
        }
        self.model_changed(cx);
        self.pump_launcher_icons(cx);
    }

    pub(super) fn on_launcher_icon(&mut self, cx: &mut Context<Self>, name: String, image: Option<nuxut_image::Image>) {
        for s in &mut self.screens {
            if s.launcher_icon_loaded(&name, image.clone()) {
                cx.request_frame(s.main);
            }
        }
    }

    /// Config yeniden yüklendi: başlatma ayarları servise.
    pub(super) fn launcher_config_changed(&self) {
        if let Some(l) = &self.launcher {
            l.set_options(self.model.config.launcher.launch_options());
        }
    }

    /// Başlatıcı görünümlerinin istediği ikonları arka plan yükleyicisine verir.
    pub(super) fn pump_launcher_icons(&mut self, cx: &mut Context<Self>) {
        // Pano küçük resimleri güvenilmeyen baytlardır: yalnızca yalıtılmış işçide çözülür; işçi
        // kapalıysa küçük resim yok (satır yer tutucu ikonla kalır).
        let isolated = crate::services::image_sandbox::decoder(&self.image_sandbox).is_some();
        for s in &mut self.screens {
            for thumb in s.take_launcher_thumb_requests() {
                let sender = cx.sender();
                let name = thumb.key;
                if !isolated {
                    let _ = sender.send(ShellMessage::LauncherIcon { name, image: None });
                    continue;
                }
                let request = nuxut_image::LoadRequest {
                    source: nuxut_image::Source::Bytes(thumb.data),
                    options: super::clipboard_history::thumb_options(),
                };
                self.launcher_icons.loader.load(request, move |result| {
                    let image = match result {
                        Ok(d) => Some(d.into_first_frame()),
                        Err(_) => None,
                    };
                    let _ = sender.send(ShellMessage::LauncherIcon { name, image });
                });
            }
        }
        for s in &mut self.screens {
            for name in s.take_launcher_icon_requests() {
                let sender = cx.sender();
                let request = icon_request(&self.launcher_icons.theme, name.clone());
                self.launcher_icons.loader.load(request, move |result| {
                    let image = match result {
                        Ok(nuxut_image::Decoded::Still(img)) => Some(img),
                        // Animasyonlu ikon beklenmez; yer tutucu kalır.
                        _ => None,
                    };
                    let _ = sender.send(ShellMessage::LauncherIcon { name, image });
                });
            }
        }
    }

    /// Başlatma: önce belirteç istenir, istek belirteç gelince servise gider; yaprak kapanır.
    /// Belirteç [`TOKEN_TIMEOUT`] içinde gelmezse belirteçsiz başlatılır.
    pub(super) fn launch_app(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        entry: Arc<DesktopEntry>,
        action: Option<String>,
    ) {
        if self.launcher.is_none() {
            tracing::warn!("başlatıcı servisi yok; uygulama başlatılamadı");
        } else if self.pending_launches.len() >= MAX_PENDING {
            tracing::warn!("belirteç bekleyen başlatma sınırı; istek atıldı");
        } else {
            let app_id = entry.id.strip_suffix(".desktop").unwrap_or(&entry.id).to_owned();
            let id = cx.request_activation_token(surface, Some(&app_id));
            let request = LaunchRequest { entry, action, activation_token: None };
            self.pending_launches.push(id, request, Instant::now() + TOKEN_TIMEOUT);
            if self.launch_timer.is_none() {
                self.launch_timer = Some(cx.set_timer(TOKEN_TIMEOUT));
            }
        }
        self.act(cx, surface, Action::ClosePanel(PanelId::Launcher));
    }

    /// `msg launcher <clipboard|emoji>`: başlatıcıyı o kipte açar; o kipte açıksa kapatır, başka
    /// kipte açıksa kipi değiştirir.
    pub(super) fn launcher_mode(&mut self, cx: &mut Context<Self>, mode: LauncherMode, output: Option<&str>) -> Result<String, String> {
        let idx = self.target_screen(output)?;
        let surface = self.screens[idx].main;
        let theme = cx.theme().clone();
        let now = Instant::now();
        match self.screens[idx].launcher_mode() {
            Some(m) if m == mode => self.act(cx, surface, Action::ClosePanel(PanelId::Launcher)),
            Some(_) => {
                if self.screens[idx].set_launcher_mode(mode, &theme, now) {
                    cx.request_frame(surface);
                }
                self.pump_launcher_icons(cx);
            }
            None => {
                self.screens[idx].set_launcher_mode(mode, &theme, now);
                self.act(cx, surface, Action::TogglePanel(PanelId::Launcher));
                if self.screens[idx].launcher_mode().is_some() {
                    // Bind'le açıldı: işaretçi bir pencerede; klavye kapanana kadar bizde kalsın.
                    let config = self.model.config.clone();
                    self.screens[idx].hold_keyboard();
                    Self::apply_input(cx, &mut self.screens[idx], &config);
                } else {
                    // Açılmadı (`launcher.enabled = false`): istek sonraki açılışa sızmasın.
                    self.screens[idx].set_launcher_mode(LauncherMode::Apps, &theme, now);
                }
            }
        }
        Ok("ok".into())
    }

    /// Emoji kipinde seçim: panoya (kabuğun pano sahipliği; otomatik yazma yok), sık
    /// kullanılanlara kayıt (servis thread'i yazar), başlatıcı kapanır.
    pub(super) fn pick_emoji(&mut self, cx: &mut Context<Self>, surface: SurfaceId, emoji: &'static str) {
        self.copy_to_clipboard(cx, emoji);
        if let Some(l) = &self.launcher {
            l.record_emoji(emoji);
        }
        self.act(cx, surface, Action::ClosePanel(PanelId::Launcher));
    }

    /// Kısayolla açılan varsayılan uygulama (terminal, dosya yöneticisi, tarayıcı): yüzey yok,
    /// etkinleştirme belirteci istenmez; yeni pencereye odağı compositor verir.
    pub(super) fn launch_default(&mut self, entry: Arc<DesktopEntry>) {
        match &self.launcher {
            Some(l) => l.launch(LaunchRequest { entry, action: None, activation_token: None }),
            None => tracing::warn!("başlatıcı servisi yok; varsayılan uygulama başlatılamadı"),
        }
    }

    pub(super) fn on_activation_token(&mut self, id: ActivationRequestId, token: Option<ActivationToken>) {
        let Some(mut request) = self.pending_launches.take(&id) else { return };
        request.activation_token = token.map(ActivationToken::into_string);
        if let Some(l) = &self.launcher {
            l.launch(request);
        }
    }

    /// Belirteç zaman aşımı timer'ıysa süresi dolanları belirteçsiz başlatır (odak kararı
    /// compositor'ın) ve `true` döner; bekleyen kaldıysa timer en yakın süreye kurulur.
    pub(super) fn launch_timer_fired(&mut self, cx: &mut Context<Self>, id: TimerId) -> bool {
        if self.launch_timer != Some(id) {
            return false;
        }
        self.launch_timer = None;
        let now = Instant::now();
        let expired = self.pending_launches.expire(now);
        if !expired.is_empty() {
            tracing::debug!(count = expired.len(), "etkinleştirme belirteci gelmedi; belirteçsiz başlatılıyor");
        }
        if let Some(l) = &self.launcher {
            for request in expired {
                l.launch(request);
            }
        }
        if let Some(deadline) = self.pending_launches.next_deadline() {
            self.launch_timer = Some(cx.set_timer(deadline.saturating_duration_since(now)));
        }
        true
    }
}

/// Belirteç bekleyen başlatmalar (istek kimliği, istek, son süre). Kimlik türü genel: testte
/// framework kimliği üretilemez.
#[derive(Debug)]
pub(super) struct PendingLaunches<I> {
    items: Vec<(I, LaunchRequest, Instant)>,
}

impl<I> Default for PendingLaunches<I> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<I: PartialEq> PendingLaunches<I> {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn push(&mut self, id: I, request: LaunchRequest, deadline: Instant) {
        self.items.push((id, request, deadline));
    }

    /// Belirteci gelen isteği çıkarır (süresi dolup başlatılmışsa `None`: geç belirteç yok sayılır).
    pub fn take(&mut self, id: &I) -> Option<LaunchRequest> {
        let i = self.items.iter().position(|(p, ..)| p == id)?;
        Some(self.items.remove(i).1)
    }

    /// Süresi dolanları istek sırasıyla çıkarır.
    pub fn expire(&mut self, now: Instant) -> Vec<LaunchRequest> {
        let (expired, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.items).into_iter().partition(|(.., d)| *d <= now);
        self.items = kept;
        expired.into_iter().map(|(_, r, _)| r).collect()
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.items.iter().map(|(.., d)| *d).min()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::services::launcher::desktop;

    use super::*;

    fn request(name: &str) -> LaunchRequest {
        let id = format!("{name}.desktop");
        let text = "[Desktop Entry]\nType=Application\nName=Uyg\nExec=uyg\n";
        let entry = desktop::parse(&id, std::path::Path::new("/apps/uyg.desktop"), text, &desktop::Locale::default()).unwrap();
        LaunchRequest { entry: Arc::new(entry), action: None, activation_token: None }
    }

    #[test]
    fn token_before_deadline_takes_request() {
        let t0 = Instant::now();
        let mut p = PendingLaunches::default();
        p.push(1, request("a"), t0 + TOKEN_TIMEOUT);
        assert!(p.expire(t0).is_empty());
        assert_eq!(p.take(&1).map(|r| r.entry.id.clone()), Some("a.desktop".into()));
        assert_eq!(p.len(), 0);
        assert!(p.expire(t0 + TOKEN_TIMEOUT * 2).is_empty());
    }

    #[test]
    fn expired_launch_without_token_and_late_token_ignored() {
        let t0 = Instant::now();
        let mut p = PendingLaunches::default();
        p.push(1, request("a"), t0 + TOKEN_TIMEOUT);
        p.push(2, request("b"), t0 + TOKEN_TIMEOUT + Duration::from_secs(1));
        assert_eq!(p.next_deadline(), Some(t0 + TOKEN_TIMEOUT));
        let expired = p.expire(t0 + TOKEN_TIMEOUT);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].entry.id, "a.desktop");
        assert!(expired[0].activation_token.is_none(), "belirteçsiz başlatılır");
        assert!(p.take(&1).is_none(), "geç gelen belirteç yok sayılır (çift başlatma yok)");
        assert_eq!(p.next_deadline(), Some(t0 + TOKEN_TIMEOUT + Duration::from_secs(1)));
        assert_eq!(p.len(), 1);
    }
}
