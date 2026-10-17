//! Kilit ekranı görünümü: çıkış başına arka plan + saat; giriş alanı yalnızca "ev sahibi"
//! çıkışta (klavye odağı). Tüm renk/yarıçap/hareket temadan.
//!
//! Katmanlar: opak `surface` zemini → duvar kağıdı → karartma → içerik (saat, tarih; ev
//! sahibinde avatar, ad, şifre alanı, durum satırları). İçerik açılırken `effects` ile solar.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nuxut::i18n::t;
use nuxut::render::{ImageFit, TextureOptions};
use nuxut::theme::{ColorRole, ThemeMode};
use nuxut::ui::{Avatar, AvatarSize, BrandMark, Label, Spinner, TextInput, TextRole, Ui, text_style};
use nuxut::{Animated, Color, Corners, Painter, Point, Rect, RectStyle, Size, Theme};
use nuxut_image::{Wallpaper, WallpaperDecoder};
use nuxut_pam::{MessageKind, Outcome};
use nuxut_wallpapers::{ColorThemeId, Renderer, Slot, color_theme_for};

use crate::assets::WallpaperSource;
use crate::fingerprint::{FpEvent, RetryReason};
use crate::state::AuthStatus;

/// Duvar kağıdı bulanıklık yarıçapı (mantıksal px), `lock.toml blur = true` iken.
const BLUR_RADIUS: f32 = 32.0;
/// Saatin üstündeki marka işaretinin kenarı (mantıksal px; DESIGN "Marka işareti": ≥24).
pub const MARK_SIDE: f32 = 32.0;
/// İşaretin opaklığı: ikincil rol üstüne soluk, saatle yarışmaz.
pub const MARK_ALPHA: f32 = 0.6;
const _: () = assert!(MARK_SIDE >= 24.0 && MARK_ALPHA > 0.0 && MARK_ALPHA < 1.0);

/// Görünümler arasında paylaşılan durum (tek şifre alanı, kullanıcı).
#[derive(Debug)]
pub struct Shared {
    /// Tek şifre alanı; yalnızca ev sahibi çıkışta çizilir.
    pub input: TextInput,
    pub caps_lock: bool,
    pub fingerprint: Option<FpEvent>,
    pub display_name: String,
    /// Avatar görseli (çözülmüş, GPU'ya ilk çizimde yüklenir).
    pub avatar_image: Option<nuxut::render::ImageData>,
    avatar_texture: Option<nuxut::render::Texture>,
}

impl Shared {
    pub fn new(theme: &Theme, display_name: String, max_bytes: usize) -> Self {
        let input = TextInput::new(theme)
            .password(true)
            .with_max_bytes(max_bytes)
            .pill_shaped(true);
        Self {
            input,
            caps_lock: false,
            fingerprint: None,
            display_name,
            avatar_image: None,
            avatar_texture: None,
        }
    }

    /// Alanın yer tutucusu: parmak izi taranıyorsa onu da söyler.
    pub fn placeholder(&self) -> String {
        match self.fingerprint {
            Some(FpEvent::Scanning | FpEvent::NoMatch { .. } | FpEvent::Retry(_)) => {
                t!("shell-lock-password-placeholder-fingerprint")
            }
            _ => t!("shell-lock-password-placeholder"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Error,
}

/// Şifre alanının altındaki satırlar (en çok üç), öncelik sırasıyla.
pub fn status_lines(
    status: &AuthStatus,
    backoff: Option<Duration>,
    caps_lock: bool,
    fingerprint: Option<&FpEvent>,
) -> Vec<(String, Tone)> {
    let mut lines = Vec::new();
    match status {
        AuthStatus::Verifying => lines.push((t!("shell-lock-status-verifying"), Tone::Normal)),
        AuthStatus::Failed { outcome, messages } => {
            // PAM modülünün mesajı (faillock: "hesap kilitli") varsa genel metnin yerine o.
            let pam = messages
                .iter()
                .find(|m| m.kind == MessageKind::Error)
                .or(messages.first());
            let text = match (pam, outcome) {
                (Some(m), _) => m.text.clone(),
                (None, Outcome::Denied) => t!("shell-lock-error-wrong-password"),
                (None, Outcome::MaxTries) => t!("shell-lock-error-max-tries"),
                (None, Outcome::Unavailable) => t!("shell-lock-error-unavailable"),
                (None, Outcome::UnsupportedPrompt) => t!("shell-lock-error-unsupported"),
                (None, _) => t!("shell-lock-error-generic"),
            };
            lines.push((text, Tone::Error));
        }
        AuthStatus::Idle => {}
    }
    if let Some(left) = backoff {
        let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        lines.push((t!("shell-lock-backoff", count = secs), Tone::Normal));
    }
    if caps_lock {
        lines.push((t!("shell-lock-caps-lock"), Tone::Error));
    }
    match fingerprint {
        Some(FpEvent::NoMatch { remaining }) => {
            lines.push((
                t!("shell-lock-fingerprint-no-match", count = remaining),
                Tone::Normal,
            ));
        }
        Some(FpEvent::Retry(reason)) => {
            let text = match reason {
                RetryReason::Scan => t!("shell-lock-fingerprint-retry-scan"),
                RetryReason::TooShort => t!("shell-lock-fingerprint-retry-too-short"),
                RetryReason::NotCentered => t!("shell-lock-fingerprint-retry-not-centered"),
                RetryReason::RemoveAndRetry => t!("shell-lock-fingerprint-retry-remove"),
            };
            lines.push((text, Tone::Normal));
        }
        Some(FpEvent::Exhausted) => lines.push((
            t!("shell-lock-fingerprint-exhausted"),
            Tone::Normal,
        )),
        _ => {}
    }
    lines.truncate(3);
    lines
}

/// Kilit saati: Display rolünün üç katı (ekranın tek odağı).
fn clock_style(theme: &Theme) -> nuxut::TextStyle {
    let mut style = text_style(theme, TextRole::Display);
    style.size = (style.size * 3.0).round();
    style
}

/// Saatin üstündeki işaretin dikdörtgeni: yatayda ortalı, alt kenarı saat kutusunun üstünden
/// `space(2)` yukarıda (saat etiketinin kendi üst satır payı ayrıca boşluk verir).
fn mark_rect(center_x: f32, clock_y: f32, theme: &Theme) -> Rect {
    Rect::new(center_x - MARK_SIDE * 0.5, clock_y - theme.space(2) - MARK_SIDE, MARK_SIDE, MARK_SIDE)
}

/// Çıkış başına görünüm.
pub struct OutputView {
    size: Size,
    scale: f32,
    wallpaper: Wallpaper,
    dim: f32,
    /// Saatin üstünde küçük, soluk Nuxut işareti.
    mark: BrandMark,
    clock: Label,
    date: Label,
    name: Label,
    status: Vec<(Label, Tone)>,
    avatar: Avatar,
    spinner: Spinner,
    /// Hatalı şifrede alanın yatay ofseti (yay darbesi).
    shake: Animated<f32>,
    /// İçerik opaklığı; açılırken 0'a.
    content: Animated<f32>,
    /// Son loglanan duvar kağıdı hatası (her karede yinelenmesin).
    wallpaper_error: Option<String>,
    /// Yerleşik duvar kağıdı (`nuxut:<id>`): süreçteki tek render'ı ve bu görünümün isteği.
    builtins: Arc<Renderer>,
    builtin: Slot,
    notify: Arc<dyn Fn() + Send + Sync>,
    /// Son istenen kaynak (tema kipi değişince kipi izleyen yerleşik yeniden istenir).
    source: Option<WallpaperSource>,
    dark: bool,
    /// Yerleşiğin boyandığı renk teması (değişince yeniden istenir).
    color_theme: ColorThemeId,
}

impl std::fmt::Debug for OutputView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputView")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl OutputView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        theme: &Theme,
        display_name: &str,
        blur: bool,
        dim: f32,
        // Süreçteki tek duvar kağıdı çözücüsü (`LockImages`; bütün kilit yüzeyleri paylaşır).
        decoder: Arc<WallpaperDecoder>,
        // Süreçteki tek yerleşik render'ı (`LockImages::builtins`; bellek içi kare).
        builtins: Arc<Renderer>,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let wake = notify.clone();
        let mut wallpaper = Wallpaper::new(ImageFit::Cover, theme.motion.effects, decoder, move || wake());
        // Kaynaktan önce: görsel hiç keskin görünmez (ilk kare bulanık kopya hazır olunca).
        wallpaper.set_blur(blur.then_some(BLUR_RADIUS));
        Self {
            size: Size::ZERO,
            scale: 1.0,
            wallpaper,
            dim: dim.clamp(0.0, 0.9),
            mark: BrandMark::new().with_role(ColorRole::OnSurfaceVariant).with_size(MARK_SIDE),
            clock: Label::new("", clock_style(theme)),
            date: Label::new("", text_style(theme, TextRole::Title)),
            name: Label::new(display_name, text_style(theme, TextRole::Title)),
            status: Vec::new(),
            avatar: Avatar::new(theme, display_name, AvatarSize::Large),
            spinner: Spinner::new(theme),
            shake: Animated::new(0.0, theme.motion.spatial_fast),
            content: Animated::new(1.0, theme.motion.effects),
            wallpaper_error: None,
            builtins,
            builtin: Slot::default(),
            notify,
            source: None,
            dark: theme.mode == ThemeMode::Dark,
            color_theme: color_theme_for(theme),
        }
    }

    /// Yüzey boyutu/ölçeği; duvar kağıdı fiziksel boyuta çözülür. Yerleşik kaynak önce
    /// önbellek PNG'sine render edilir (çoğunlukla shell'in yazdığı dosyaya isabet), dosya yine
    /// yalıtılmış çözücüden geçer.
    pub fn set_size(&mut self, size: Size, scale: f32, wallpaper: Option<&WallpaperSource>) {
        self.size = size;
        self.scale = scale;
        self.source = wallpaper.cloned();
        self.apply_source();
    }

    fn px(&self) -> (u32, u32) {
        ((self.size.width * self.scale).round() as u32, (self.size.height * self.scale).round() as u32)
    }

    fn apply_source(&mut self) {
        let px = self.px();
        if px.0 == 0 || px.1 == 0 {
            return;
        }
        match &self.source {
            Some(WallpaperSource::File(path)) => {
                self.builtin.clear();
                self.wallpaper.set_source(path, px);
            }
            Some(WallpaperSource::Builtin(sel)) => {
                let variant = sel.variant_for(self.dark);
                self.builtin.want(&self.builtins, sel.builtin, variant, self.color_theme, px, self.notify.clone());
            }
            None => self.builtin.clear(),
        }
    }

    /// Yerleşik render'ın sonucu geldiyse kareyi bileşene bellekten verir (gömülü güvenilir
    /// kaynak; dosya/işçi turu yok).
    fn poll_builtin(&mut self) {
        let Some(result) = self.builtin.poll() else { return };
        let Some((id, _, _)) = self.builtin.wanted() else { return };
        match result {
            Ok(frame) => self.wallpaper.set_image(frame.key, frame),
            Err(e) => tracing::warn!(id, "kilit: yerleşik duvar kağıdı hazırlanamadı (zemin rengi kalıyor): {e}"),
        }
    }

    pub fn set_time(&mut self, clock: &str, date: &str) -> bool {
        self.clock.set_text(clock) | self.date.set_text(date)
    }

    pub fn shake(&mut self, theme: &Theme, now: Instant) {
        self.shake.kick(theme.space(1) * 400.0, now);
    }

    pub fn begin_unlock(&mut self, now: Instant) {
        self.content.set_target(0.0, now);
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.mark.set_theme(theme, now);
        self.clock.set_style(clock_style(theme));
        self.date.set_style(text_style(theme, TextRole::Title));
        self.name.set_style(text_style(theme, TextRole::Title));
        self.avatar.set_theme(theme, now);
        self.spinner.set_theme(theme, now);
        self.shake.set_motion(theme.motion.spatial_fast, now);
        self.content.set_motion(theme.motion.effects, now);
        self.wallpaper.set_motion(theme.motion.effects, now);
        // Kip ya da renk teması değişince yerleşik yeniden istenir (çapraz geçişle gelir).
        let dark = theme.mode == ThemeMode::Dark;
        let color_theme = color_theme_for(theme);
        if dark != self.dark || color_theme != self.color_theme {
            self.dark = dark;
            self.color_theme = color_theme;
            self.apply_source();
        }
    }

    /// İçerik tamamen soldu mu (açma animasyonu bitti).
    pub fn faded_out(&self, now: Instant) -> bool {
        self.content.target() == 0.0 && self.content.value(now) <= 0.01
    }

    /// Çizer. Dönüş: başka kare gerekiyor mu.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        painter: &mut Painter,
        theme: &Theme,
        now: Instant,
        shared: &mut Shared,
        status: &AuthStatus,
        backoff: Option<Duration>,
        host: bool,
    ) -> bool {
        let c = &theme.colors;
        let full = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        // Opak zemin: duvar kağıdı yüklenmeden de altta hiçbir şey görünmez.
        painter.rect(
            full,
            &RectStyle::filled(c.surface.with_alpha(1.0), Corners::ZERO),
        );
        self.poll_builtin();
        let wp = self.wallpaper.draw(painter, full, now);
        // Hata (işçi yok, protokol, zaman aşımı, bozuk dosya): zemin rengi kalır, bir kez loglanır.
        if self.wallpaper.last_error() != self.wallpaper_error.as_deref() {
            self.wallpaper_error = self.wallpaper.last_error().map(str::to_owned);
            if let Some(e) = &self.wallpaper_error {
                tracing::warn!("kilit duvar kağıdı yüklenemedi (önceki görsel ya da zemin rengi kalıyor): {e}");
            }
        }
        painter.rect(
            full,
            &RectStyle::filled(c.surface.with_alpha(self.dim), Corners::ZERO),
        );

        let mut ui = Ui::new(painter, theme, now);
        let alpha = self.content.value(now).clamp(0.0, 1.0);
        let center_x = self.size.width * 0.5;

        ui.with_alpha(alpha, |ui| {
            let clock = self.clock.measure_in(ui.painter);
            let clock_y = self.size.height * 0.22;
            self.mark.set_rect(mark_rect(center_x, clock_y, theme));
            ui.with_alpha(MARK_ALPHA, |ui| self.mark.draw(ui));
            self.clock.draw(
                ui.painter,
                Point::new(center_x - clock.width * 0.5, clock_y),
                ui.fade(c.on_surface),
            );
            let date = self.date.measure_in(ui.painter);
            let date_y = clock_y + clock.height + theme.space(1);
            self.date.draw(
                ui.painter,
                Point::new(center_x - date.width * 0.5, date_y),
                ui.fade(c.on_surface_variant),
            );
            if host {
                self.draw_login(ui, theme, now, shared, status, backoff);
            }
        });

        let animating =
            !self.content.is_settled(now) || !self.shake.is_settled(now) || ui.wants_frame();
        animating || wp.redraw
    }

    fn draw_login(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        now: Instant,
        shared: &mut Shared,
        status: &AuthStatus,
        backoff: Option<Duration>,
    ) {
        let c = &theme.colors;
        let center_x = self.size.width * 0.5;
        if let Some(data) = shared.avatar_image.take() {
            shared.avatar_texture = Some(
                ui.painter
                    .upload_texture(data, TextureOptions { mipmaps: false }),
            );
        }
        self.avatar.set_image(shared.avatar_texture.clone());

        let side = AvatarSize::Large.side(theme);
        let mut y = self.size.height * 0.55;
        self.avatar.set_center(Point::new(center_x, y + side * 0.5));
        self.avatar.draw(ui);
        y += side + theme.space(3);

        let name = self.name.measure_in(ui.painter);
        self.name.draw(
            ui.painter,
            Point::new(center_x - name.width * 0.5, y),
            ui.fade(c.on_surface),
        );
        y += name.height + theme.space(4);

        let width = (self.size.width - theme.space(16)).clamp(theme.space(40), theme.space(90));
        let pref = shared.input.preferred_size(theme, width);
        let offset = self.shake.value(now);
        shared.input.set_placeholder(&shared.placeholder());
        shared.input.set_rect(Rect::new(
            center_x - width * 0.5 + offset,
            y,
            width,
            pref.height,
        ));
        shared.input.draw(ui);
        y += pref.height + theme.space(3);

        if matches!(status, AuthStatus::Verifying) {
            let s = self.spinner.preferred_size(theme);
            self.spinner.set_rect(Rect::new(
                center_x - s.width * 0.25,
                y,
                s.width * 0.5,
                s.height * 0.5,
            ));
            self.spinner.draw(ui);
            y += s.height * 0.5 + theme.space(2);
        }

        let lines = status_lines(
            status,
            backoff,
            shared.caps_lock,
            shared.fingerprint.as_ref(),
        );
        self.status.resize_with(lines.len(), || {
            (
                Label::new("", text_style(theme, TextRole::Body)),
                Tone::Normal,
            )
        });
        for ((label, tone), (text, t)) in self.status.iter_mut().zip(lines) {
            label.set_text(&text);
            label.set_max_width(Some(width));
            *tone = t;
            let size = label.measure_in(ui.painter);
            let color: Color = match tone {
                Tone::Normal => c.on_surface_variant,
                Tone::Error => c.error,
            };
            label.draw(
                ui.painter,
                Point::new(center_x - size.width * 0.5, y),
                ui.fade(color),
            );
            y += size.height + theme.space(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_pam::PamMessage;

    #[test]
    fn mark_sits_centered_above_clock() {
        let theme = Theme::default();
        let (w, h) = (1920.0, 1080.0);
        let clock_y = h * 0.22;
        let r = mark_rect(w * 0.5, clock_y, &theme);
        assert_eq!((r.width, r.height), (MARK_SIDE, MARK_SIDE));
        assert!((r.center().x - w * 0.5).abs() < 1e-3);
        assert!(r.bottom() < clock_y && r.y > 0.0, "{r:?}");
    }

    fn texts(lines: Vec<(String, Tone)>) -> Vec<String> {
        lines.into_iter().map(|(t, _)| t).collect()
    }

    #[test]
    fn pam_message_replaces_generic_text() {
        crate::i18n::use_turkish_for_tests();
        let failed = AuthStatus::Failed {
            outcome: Outcome::Denied,
            messages: vec![
                PamMessage {
                    kind: MessageKind::Info,
                    text: "bilgi".into(),
                },
                PamMessage {
                    kind: MessageKind::Error,
                    text: "Hesap 10 dakika kilitli".into(),
                },
            ],
        };
        assert_eq!(
            texts(status_lines(&failed, None, false, None)),
            ["Hesap 10 dakika kilitli"]
        );
        let plain = AuthStatus::Failed {
            outcome: Outcome::Denied,
            messages: vec![],
        };
        assert_eq!(
            status_lines(&plain, None, false, None),
            [("Yanlış şifre".to_owned(), Tone::Error)]
        );
    }

    #[test]
    fn backoff_caps_and_fingerprint_lines() {
        crate::i18n::use_turkish_for_tests();
        let lines = status_lines(
            &AuthStatus::Idle,
            Some(Duration::from_millis(1500)),
            true,
            Some(&FpEvent::NoMatch { remaining: 2 }),
        );
        assert_eq!(
            texts(lines),
            [
                "2 sn sonra yeniden deneyin",
                "Caps Lock açık",
                "Parmak izi eşleşmedi (2 deneme kaldı)"
            ]
        );
        assert!(status_lines(&AuthStatus::Idle, None, false, None).is_empty());
    }

    #[test]
    fn at_most_three_lines() {
        crate::i18n::use_turkish_for_tests();
        let failed = AuthStatus::Failed {
            outcome: Outcome::Error(4),
            messages: vec![],
        };
        let lines = status_lines(
            &failed,
            Some(Duration::from_secs(4)),
            true,
            Some(&FpEvent::Exhausted),
        );
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].0, "Kimlik doğrulama hatası");
    }
}

// Ekran okuyucu (framework ADR-0011 madde 8, 2026-10-17).
// AT_POLICY: kilit ekranında AT yalnız okur ve kilitliyken hiçbir şey sızdırmaz. Ağaçta yalnız
// saat ve tarih (her çıkış), giriş çıkışında kullanıcı adı, parola alanı (yalnız dolu/boş; değer
// ve uzunluk yok) ve arındırılmış durum metni (canlı bölge; PAM/faillock mesajı, kalan deneme ve
// geri sayım yok — `access_status`). Kilit altındaki masaüstü ayrı süreç: kabuk kilitliyken
// ağacını askıya alır (`set_a11y_suspended`). Hiçbir düğümde eylem yok: kilidi açma ve parola
// yalnız gerçek klavyeyle (açma düğmesi yok; Enter). Eylemler `app.rs` `accessibility_action`'da
// sessizce reddedilir.
impl OutputView {
    pub fn access(&self, tree: &mut nuxut::access::AccessTree, shared: &Shared, status: &AuthStatus, waiting: bool, host: bool) {
        use nuxut::access::{Live, Role};
        let full = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        tree.root().label(&t!("shell-lock-a11y-title"));
        let time = [self.clock.text(), self.date.text()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
        tree.push("time", Role::Label).value(&time).bounds(full);
        if !host {
            return;
        }
        tree.push("user", Role::Label).value(self.name.text()).bounds(full);
        password_access(tree, &shared.input, &t!("shell-lock-a11y-password"));
        let lines = access_status(status, waiting, shared.caps_lock, shared.fingerprint.as_ref());
        if !lines.is_empty() {
            tree.push("status", Role::Label).value(&lines.join(". ")).bounds(full).live(Live::Polite);
        }
    }
}

/// Parola alanının düğümü: yalnız ad ve "boş"/"dolu" (açıklama). Değer, uzunluk, maske, metin
/// parçası ve eylem yok → AT-SPI'da `Text`/`EditableText`/`Action` arayüzü ve text-changed olayı
/// yok; tuş başına ağaç değişmez (yalnız boş↔dolu geçişinde). Bilerek `TextInput::access`
/// kullanılmaz: metin parçası accesskit'te `EditableText` açar ve uzunluk maskesini taşır.
fn password_access(tree: &mut nuxut::access::AccessTree, input: &TextInput, label: &str) {
    let state = if input.is_empty() { t!("shell-lock-a11y-password-empty") } else { t!("shell-lock-a11y-password-filled") };
    tree.push("password", nuxut::access::Role::PasswordInput)
        .label(label)
        .description(&state)
        .bounds(input.rect())
        .focused_if(input.is_focused());
}

/// Ekran okuyucunun durum metni: görsel satırların saldırgana yaramayan alt kümesi. PAM modülü
/// mesajı (faillock "hesap N dk kilitli"), "çok fazla deneme", parmak izinde kalan deneme sayısı
/// ve saniye saniye geri sayım okunmaz; yerine sabit genel metin.
fn access_status(status: &AuthStatus, waiting: bool, caps_lock: bool, fingerprint: Option<&FpEvent>) -> Vec<String> {
    let mut lines = Vec::new();
    match status {
        AuthStatus::Verifying => lines.push(t!("shell-lock-status-verifying")),
        AuthStatus::Failed { outcome: Outcome::Denied | Outcome::MaxTries, .. } => lines.push(t!("shell-lock-error-wrong-password")),
        AuthStatus::Failed { .. } => lines.push(t!("shell-lock-error-generic")),
        AuthStatus::Idle => {}
    }
    if waiting {
        lines.push(t!("shell-lock-a11y-wait"));
    }
    if caps_lock {
        lines.push(t!("shell-lock-caps-lock"));
    }
    match fingerprint {
        Some(FpEvent::NoMatch { .. }) => lines.push(t!("shell-lock-a11y-fingerprint-no-match")),
        Some(FpEvent::Retry(_)) => lines.push(t!("shell-lock-fingerprint-retry-scan")),
        Some(FpEvent::Exhausted) => lines.push(t!("shell-lock-fingerprint-exhausted")),
        _ => {}
    }
    lines
}

#[cfg(test)]
mod access_tests {
    use super::*;
    use nuxut::access::{AccessTree, Action as AxAction, Role};
    use nuxut_pam::PamMessage;

    fn tree_text(tree: &AccessTree) -> String {
        // Kök (çocuk kimlikleri sayısal karma) hariç: parola düğümünün içeriği.
        format!("{:?}", tree.nodes().skip(1).map(|(_, n)| n).collect::<Vec<_>>())
    }

    /// Sahte parola "abc123" ağaçta yok; değeri, uzunluğu ve maskesi de yok. Ağaç yalnız boş↔dolu
    /// geçişinde değişir: 1 ve 6 karakterde aynı (tuş başına güncelleme/olay yok, uzunluk çıkmaz).
    #[test]
    fn password_leaks_neither_value_nor_length() {
        let theme = Theme::default();
        let mut shared = Shared::new(&theme, "Test".into(), 256);
        let build = |shared: &Shared| {
            let mut tree = AccessTree::new();
            password_access(&mut tree, &shared.input, "Şifre");
            tree
        };
        let empty = tree_text(&build(&shared));
        shared.input.set_text("a");
        let one = tree_text(&build(&shared));
        shared.input.set_text("abc123");
        let tree = build(&shared);
        let six = tree_text(&tree);
        assert_ne!(empty, one, "boş/dolu okunur");
        assert_eq!(one, six, "uzunluk ağaçtan çıkarılabiliyor");
        for leak in ["abc123", "abc", "\u{2022}", "6", "TextRun"] {
            assert!(!six.contains(leak), "{leak}: {six}");
        }
        let nodes: Vec<_> = tree.nodes().collect();
        assert_eq!(nodes.len(), 2, "kök + parola");
        let (_, n) = nodes[1];
        assert_eq!(n.role(), Role::PasswordInput);
        assert_eq!(n.value(), None);
        assert!(n.children().is_empty());
        for a in [AxAction::Click, AxAction::Focus, AxAction::SetValue, AxAction::ReplaceSelectedText, AxAction::SetTextSelection] {
            assert!(!n.supports_action(a), "{a:?}");
        }
    }

    /// Durum metni PAM/faillock mesajını, kalan denemeyi ve geri sayımı taşımaz.
    #[test]
    fn status_hides_attacker_details() {
        let faillock = AuthStatus::Failed {
            outcome: Outcome::Denied,
            messages: vec![PamMessage { kind: MessageKind::Error, text: "Hesap 10 dakika kilitli (3 deneme kaldı)".into() }],
        };
        let max = AuthStatus::Failed { outcome: Outcome::MaxTries, messages: Vec::new() };
        let fp = FpEvent::NoMatch { remaining: 2 };
        for status in [&faillock, &max] {
            let text = access_status(status, true, false, Some(&fp)).join(". ");
            for leak in ["Hesap", "kilitli", "dakika", "deneme", "3", "2", "sn"] {
                assert!(!text.contains(leak), "{leak}: {text}");
            }
            assert!(text.contains(&t!("shell-lock-error-wrong-password")));
        }
    }
}
