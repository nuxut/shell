//! Duvar kağıdı: monitör başına Background katmanında bir yüzey ve `nuxut-image` `Wallpaper`
//! bileşeni (paylaşılan çözücüde arka planda çözme, kaynak değişince `motion.effects` ile çapraz
//! geçiş, boşta iş yok).
//!
//! **Çözüm yalıtılmış işçide** (framework "Duvar kağıdı yalıtımı", ADR-0007 takip): bütün
//! yüzeyler süreçteki tek `WallpaperDecoder`'ı paylaşır (`Shell::wallpaper_decoder`,
//! `image_sandbox::new_wallpaper_decoder`); aynı dosya ve fiziksel boyuttaki çıkışlar tek çözüm,
//! daha küçük çıkış büyük kareden türetilir. Kaynak çözümlemesi tek noktadadır
//! ([`WallpaperSurface`]'ın `apply_source`'u): `[wallpaper] path` ya dosya yoludur ya da
//! **yerleşik** `nuxut:<id>[/dark|light]` (`docs/design/wallpapers.md`).
//!
//! **Yerleşik kaynak**: gömülü SVG, `nuxut_wallpapers::Renderer`'da (süreç başına tek, arka plan
//! thread'i, aynı anahtar birleşir) çıkışın tam fiziksel boyutunda **bellekte** render edilir ve
//! kare bileşene `Wallpaper::set_image` ile kopyasız verilir: kaynak gömülü ve güvenilir, disk
//! önbelleği, PNG kodlama ve yalıtılmış çözüm turu yoktur (4K'da ~0,5 sn kazanç). Varyant yazılmamışsa tema kipini izler: kip değişince yeni
//! varyant render edilir ve çapraz geçişle gelir. Render hatası: önceki görsel ya da zemin
//! rengi kalır, bir kez `warn`.
//!
//! **Animasyonlu kaynak** (GIF/WebP/APNG, S2-a): işçi yalnızca ilk kareyi verir, duvar
//! kağıdı durağandır; kare/timer istenmez. Kaynak animasyonluysa bir kez bilgi loglanır;
//! hareketli duvar kağıdı `nuxut-video` içe aktarmasıyla (ayarlar) video yoluna gider.
//!
//! **Hata** (işçi yok/`Unavailable`, `Protocol`, zaman aşımı, bozuk/çok büyük dosya): panik
//! yok; önceki görsel kalır, hiç yoksa zemin rengi; hata çizimde bir kez loglanır (yol değil,
//! framework'ün hata metni).
//!
//! `[wallpaper] path` boşsa yüzey yaratılmaz: kullanıcı başka bir araç (hyprpaper vb.)
//! kullanıyor olabilir. Bileşenin `notify`'ı (çözüm bitti) ana döngüyü bir mesajla uyandırır.
//!
//! **Yüzey her zaman tamamen opaktır.** Görselin altına önce `[wallpaper] backdrop_color`
//! tema rengi (alfası 1'e zorlanmış) tüm yüzeye boyanır: görsel henüz çözülmemişken ilk
//! kareler ve saydam pikselli görseller Hyprland'in kendi varsayılan duvar kağıdını/logosunu
//! göstermez (pano t095). Hyprland tarafında da bunlar kapatılır (`misc:disable_hyprland_logo`,
//! `misc:force_default_wallpaper = 0`; bkz. `hypr_config::GenerateInput::hide_defaults`).
//!
//! **Letterbox zemini** (`[wallpaper] backdrop`, t095 katman 3): `contain` kipinde görselin
//! kaplamadığı kenarlara varsayılan olarak aynı görselin ekranı kaplayan bulanık kopyası
//! (`backdrop_blur` mantıksal px) boyanır; `color` seçilirse düz tema rengi. Kopyayı
//! framework (`Wallpaper::set_backdrop`) katman başına bir kez üretir; diğer kiplerde ve kopya
//! hazır olana dek yedek renk boyanır.

use std::sync::Arc;
use std::time::Instant;

use nuxut::render::ImageFit;
use nuxut::{
    Anchor, Color, Corners, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, Painter, Point, Rect, RectStyle, Size, SurfaceId,
    Theme,
};
use nuxut::theme::ThemeMode;
use nuxut_image::{Wallpaper, WallpaperBackdrop, WallpaperDecoder};
use nuxut_wallpapers::{ColorThemeId, Renderer, Slot, color_theme_for};

use crate::config::{
    WallpaperBackdropColor, WallpaperBackdropKind, WallpaperBackdropSpec, WallpaperFit, WallpaperSource, WallpaperWanted,
};

pub const NAMESPACE: &str = "nuxut-wallpaper";

pub fn image_fit(f: WallpaperFit) -> ImageFit {
    match f {
        WallpaperFit::Cover => ImageFit::Cover,
        WallpaperFit::Contain => ImageFit::Contain,
        WallpaperFit::Fill => ImageFit::Fill,
    }
}

/// Zeminin tema rengi, her zaman opak (yarı saydam bir tema rengi alttaki compositor zeminini
/// sızdırmasın).
pub fn backdrop_color(theme: &Theme, role: WallpaperBackdropColor) -> Color {
    let c = &theme.colors;
    match role {
        WallpaperBackdropColor::Surface => c.surface,
        WallpaperBackdropColor::SurfaceContainer => c.surface_container,
        WallpaperBackdropColor::SurfaceBright => c.surface_bright,
    }
    .with_alpha(1.0)
}

/// Config + tema → bileşenin zemini.
pub fn backdrop(spec: WallpaperBackdropSpec, theme: &Theme) -> WallpaperBackdrop {
    let color = backdrop_color(theme, spec.color);
    match spec.kind {
        // `u32 → f32` kayıpsız (yarıçap config'te 4..=128 ile sınırlı).
        WallpaperBackdropKind::Blur => WallpaperBackdrop::Blur { radius: spec.blur as f32, fallback: color },
        WallpaperBackdropKind::Color => WallpaperBackdrop::Color(color),
    }
}

/// `msg wallpaper builtins` yanıtı (tek satır JSON; ayarlar sözleşmesi
/// `docs/design/wallpapers.md`): `[{id, source, name, variants, default, seed: {dark, light}}]`.
/// Ad süreç geneli katalogdan (shell'in dili).
pub fn builtins_json() -> String {
    let list: Vec<serde_json::Value> = nuxut_wallpapers::BUILTINS
        .iter()
        .map(|b| {
            serde_json::json!({
                "id": b.id,
                "source": b.source(),
                "name": b.name(),
                "variants": nuxut_wallpapers::Variant::ALL.map(nuxut_wallpapers::Variant::name),
                "default": b.is_default(),
                "seed": {
                    "dark": b.seed(nuxut_wallpapers::Variant::Dark),
                    "light": b.seed(nuxut_wallpapers::Variant::Light),
                },
            })
        })
        .collect();
    serde_json::Value::Array(list).to_string()
}

/// Bir monitörün duvar kağıdı yüzeyi.
pub struct WallpaperSurface {
    pub surface: SurfaceId,
    component: Wallpaper,
    /// İstenen kaynak, yerleşim ve zemin (config).
    wanted: WallpaperWanted,
    /// Süreçteki tek yerleşik render'ı ve bu yüzeyin isteği.
    builtins: Arc<Renderer>,
    builtin: Slot,
    /// Çözüm/render bitti: ana döngüyü uyandırır (bileşenle ortak).
    notify: Arc<dyn Fn() + Send + Sync>,
    /// Tema kipi koyu mu (varyantı yazılmamış yerleşik bunu izler).
    dark: bool,
    /// Yerleşiklerin boyandığı renk teması (duvar kâğıdından renk kaynağında Nuxut mavisi).
    color_theme: ColorThemeId,
    /// Yüzeyin mantıksal boyutu ve ölçeği (configure).
    size: Size,
    scale: f32,
    /// Görselin ve zeminin altındaki opak taban ([`backdrop_color`]).
    base: Color,
    /// Taban rengi değişti: bir sonraki karede tüm yüzey hasarlı sayılır.
    base_dirty: bool,
    /// Son loglanan sonuç (aynı hata/animasyon bilgisi her karede yinelenmesin).
    reported: Reported,
}

impl std::fmt::Debug for WallpaperSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WallpaperSurface")
            .field("surface", &self.surface)
            .field("component", &self.component)
            .field("builtin", &self.builtin)
            .field("dark", &self.dark)
            .finish_non_exhaustive()
    }
}

/// Bileşenin son çözüm sonucunun loglanmış özeti.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum Reported {
    #[default]
    Nothing,
    /// Kaynak animasyonluydu; ilk kare gösteriliyor.
    Animated,
    Error(String),
}

impl Reported {
    fn of(component: &Wallpaper) -> Self {
        Self::from_parts(component.last_error(), component.animated_source())
    }

    fn from_parts(error: Option<&str>, animated: bool) -> Self {
        match (error, animated) {
            (Some(e), _) => Self::Error(e.to_owned()),
            (None, true) => Self::Animated,
            (None, false) => Self::Nothing,
        }
    }
}

impl WallpaperSurface {
    pub fn spec(output: OutputId) -> LayerSurfaceSpec {
        LayerSurfaceSpec {
            layer: Layer::Background,
            anchor: Anchor::ALL,
            // Rezerve alanları yok say: tüm ekranı kaplar.
            exclusive_zone: -1,
            keyboard: KeyboardMode::None,
            ..LayerSurfaceSpec::new(NAMESPACE, output)
        }
    }

    /// `decoder`: süreçteki tek duvar kağıdı çözücüsü; `builtins`: süreçteki tek yerleşik
    /// render'ı (bütün çıkışlar aynı `Arc`'lar).
    pub fn new(
        surface: SurfaceId,
        wanted: WallpaperWanted,
        theme: &Theme,
        decoder: Arc<WallpaperDecoder>,
        builtins: Arc<Renderer>,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(notify);
        let wake = notify.clone();
        let mut component = Wallpaper::new(image_fit(wanted.fit), theme.motion.effects, decoder, move || wake());
        component.set_backdrop(backdrop(wanted.backdrop, theme));
        Self {
            surface,
            component,
            base: backdrop_color(theme, wanted.backdrop.color),
            wanted,
            builtins,
            builtin: Slot::default(),
            notify,
            dark: theme.mode == ThemeMode::Dark,
            color_theme: color_theme_for(theme),
            size: Size::ZERO,
            scale: 1.0,
            base_dirty: true,
            reported: Reported::Nothing,
        }
    }

    pub fn wanted(&self) -> &WallpaperWanted {
        &self.wanted
    }

    /// Config değişti. Kaynak değiştiyse çapraz geçişle yenisine geçilir; yerleşim ve zemin
    /// hemen (çağıran bir kare ister).
    pub fn set_wanted(&mut self, wanted: WallpaperWanted, theme: &Theme) {
        if wanted.fit != self.wanted.fit {
            self.component.set_fit(image_fit(wanted.fit));
        }
        if wanted.backdrop != self.wanted.backdrop {
            self.apply_backdrop(wanted.backdrop, theme);
        }
        self.wanted = wanted;
        self.apply_source();
    }

    fn apply_backdrop(&mut self, spec: WallpaperBackdropSpec, theme: &Theme) {
        self.component.set_backdrop(backdrop(spec, theme));
        let base = backdrop_color(theme, spec.color);
        if base != self.base {
            self.base = base;
            self.base_dirty = true;
        }
    }

    /// Yüzey boyutu/ölçeği belli oldu: kaynak bu fiziksel boyuta göre çözülür.
    pub fn resized(&mut self, size: Size, scale: f32) {
        self.size = size;
        self.scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        self.apply_source();
    }

    fn apply_source(&mut self) {
        if self.size.width <= 0.0 || self.size.height <= 0.0 {
            return;
        }
        let px = ((self.size.width * self.scale).round() as u32, (self.size.height * self.scale).round() as u32);
        match &self.wanted.source {
            // Aynı (dosya, boyut, yerleşim) isteğini bileşen yinelemez.
            WallpaperSource::File(path) => {
                self.builtin.clear();
                self.component.set_source(path, px);
            }
            // Render bitince (`notify` → kare) `poll_builtin` kareyi bileşene verir; o zamana
            // dek önceki görsel (ya da zemin) kalır.
            WallpaperSource::Builtin(sel) => {
                let variant = sel.variant_for(self.dark);
                self.builtin.want(&self.builtins, sel.builtin, variant, self.color_theme, px, self.notify.clone());
            }
        }
    }

    /// Yerleşik render'ın sonucu geldiyse kareyi bileşene bellekten verir (güvenilir gömülü
    /// kaynak; dosya ve yalıtılmış çözücü turu yok). Hata bir kez loglanır; önceki görsel ya da
    /// zemin kalır.
    fn poll_builtin(&mut self) {
        let Some(result) = self.builtin.poll() else { return };
        let Some((id, variant, _)) = self.builtin.wanted() else { return };
        match result {
            Ok(frame) => self.component.set_image(frame.key, frame),
            Err(e) => tracing::warn!(id, variant = variant.name(), "yerleşik duvar kağıdı hazırlanamadı (zemin rengi kalıyor): {e}"),
        }
    }

    /// Tema değişti. Zemin rengi değiştiyse `true` (yeniden çizilmeli). Kip değiştiyse kipi
    /// izleyen yerleşik yeni varyantı, renk teması değiştiyse yeni temaya boyanmışını ister
    /// (sonuç gelince çapraz geçişle; ayrıca kare istenir).
    pub fn set_theme(&mut self, theme: &Theme, now: Instant) -> bool {
        let dark = theme.mode == ThemeMode::Dark;
        let color_theme = color_theme_for(theme);
        if dark != self.dark || color_theme != self.color_theme {
            self.dark = dark;
            self.color_theme = color_theme;
            self.apply_source();
        }
        self.component.set_motion(theme.motion.effects, now);
        let before = (self.component.backdrop(), self.base);
        self.apply_backdrop(self.wanted.backdrop, theme);
        before != (self.component.backdrop(), self.base)
    }

    /// Tam ekranda görünmez (yalnızca kaydedilir; duvar kağıdı durağandır).
    pub fn set_visible(&mut self, visible: bool, now: Instant) {
        self.component.set_visible(visible, now);
    }

    pub fn last_error(&self) -> Option<&str> {
        self.component.last_error()
    }

    /// Çizer. Döner: geçiş/bulanık kopya sürüyor → kare iste. Bileşen sonucu çizimde alır;
    /// yeni hata ya da animasyonlu kaynak burada bir kez loglanır.
    pub fn draw(&mut self, painter: &mut Painter, now: Instant) -> bool {
        let rect = Rect::from_origin_size(Point::ZERO, painter.size());
        // Önce opak taban (zemin kopyası saydam pikselli olsa da), sonra zemin + görsel.
        painter.rect(rect, &RectStyle::filled(self.base, Corners::ZERO));
        if std::mem::take(&mut self.base_dirty) {
            painter.damage(rect);
        }
        self.poll_builtin();
        let st = self.component.draw(painter, rect, now);
        self.report();
        st.redraw
    }

    /// Son sonucu, değiştiyse loglar.
    fn report(&mut self) {
        let now = Reported::of(&self.component);
        if now == self.reported {
            return;
        }
        match &now {
            Reported::Error(e) => tracing::warn!("duvar kağıdı yüklenemedi (önceki görsel ya da zemin rengi kalıyor): {e}"),
            Reported::Animated => {
                tracing::info!("duvar kağıdı animasyonlu: ilk kare gösteriliyor (hareketli duvar kağıdı için ayarlardan video olarak içe aktarın)");
            }
            Reported::Nothing => {}
        }
        self.reported = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: WallpaperBackdropKind, color: WallpaperBackdropColor) -> WallpaperBackdropSpec {
        WallpaperBackdropSpec { kind, blur: 48, color }
    }

    #[test]
    fn backdrop_color_is_opaque_theme_role() {
        let mut theme = Theme::default();
        assert_eq!(backdrop_color(&theme, WallpaperBackdropColor::Surface), theme.colors.surface.with_alpha(1.0));
        assert_eq!(
            backdrop_color(&theme, WallpaperBackdropColor::SurfaceBright),
            theme.colors.surface_bright.with_alpha(1.0)
        );
        theme.colors.surface = theme.colors.surface.with_alpha(0.3);
        assert_eq!(backdrop_color(&theme, WallpaperBackdropColor::Surface).a, 1.0, "yarı saydam tema yüzeyi zemini saydam yapmaz");
    }

    #[test]
    fn backdrop_maps_config() {
        let theme = Theme::default();
        let surface = theme.colors.surface.with_alpha(1.0);
        assert_eq!(
            backdrop(spec(WallpaperBackdropKind::Blur, WallpaperBackdropColor::Surface), &theme),
            WallpaperBackdrop::Blur { radius: 48.0, fallback: surface }
        );
        assert_eq!(
            backdrop(spec(WallpaperBackdropKind::Color, WallpaperBackdropColor::SurfaceContainer), &theme),
            WallpaperBackdrop::Color(theme.colors.surface_container.with_alpha(1.0))
        );
    }

    /// Sahte yalıtılmış çözücü: `decode_file` çağrılarını sayar; `fail` doluysa o hatayı döner.
    #[derive(Debug, Default)]
    struct FakeIsolated {
        calls: std::sync::atomic::AtomicUsize,
        fail: Option<nuxut_image::SandboxFailure>,
    }

    impl nuxut_image::IsolatedDecoder for FakeIsolated {
        fn isolation(&self) -> nuxut_image::Isolation {
            nuxut_image::Isolation::Process
        }
        fn decode(&self, _: u64, _: Arc<[u8]>, _: &nuxut_image::DecodeOptions) -> Result<nuxut_image::Image, nuxut_image::ImageError> {
            unreachable!("duvar kağıdı dosyadan çözülür")
        }
        fn decode_file(
            &self,
            _key: u64,
            _file: std::fs::File,
            opts: &nuxut_image::DecodeOptions,
            _cancel: &nuxut_image::CancelFlag,
        ) -> Result<nuxut_image::DecodedFile, nuxut_image::ImageError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Birleştirme penceresi: ikinci çıkışın isteği bu iş sürerken gelir.
            std::thread::sleep(std::time::Duration::from_millis(50));
            if let Some(f) = self.fail {
                return Err(nuxut_image::ImageError::Sandbox(f));
            }
            let t = opts.target.expect("hedef boyut");
            let image = nuxut_image::Image::from_straight(t.width, t.height, vec![255; (t.width * t.height * 4) as usize]).unwrap();
            Ok(nuxut_image::DecodedFile::from_source(image, &[], &opts.limits))
        }
    }

    fn wanted(path: &std::path::Path) -> WallpaperWanted {
        WallpaperWanted {
            source: WallpaperSource::File(path.to_owned()),
            fit: WallpaperFit::Cover,
            backdrop: spec(WallpaperBackdropKind::Color, WallpaperBackdropColor::Surface),
        }
    }

    /// `n` çıkış aynı çözücüyle aynı dosyayı ister; her bildirim kanala düşer.
    fn surfaces(decoder: &Arc<WallpaperDecoder>, path: &std::path::Path, n: u64) -> (Vec<WallpaperSurface>, std::sync::mpsc::Receiver<u64>) {
        let theme = Theme::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let list = (0..n)
            .map(|i| {
                let tx = tx.clone();
                let mut s = WallpaperSurface::new(SurfaceId::for_test(i), wanted(path), &theme, decoder.clone(), Renderer::new(), move || {
                    let _ = tx.send(i);
                });
                s.resized(Size::new(64.0, 32.0), 1.0);
                s
            })
            .collect();
        (list, rx)
    }

    #[test]
    fn outputs_share_one_decoder_and_one_decode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wp.png");
        std::fs::write(&path, b"not-really-a-png").unwrap();
        let fake = Arc::new(FakeIsolated::default());
        let decoder = crate::services::image_sandbox::wallpaper_decoder_with(fake.clone());
        let (_surfaces, rx) = surfaces(&decoder, &path, 3);
        let mut seen: Vec<u64> = (0..3).map(|_| rx.recv_timeout(std::time::Duration::from_secs(5)).expect("bildirim")).collect();
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2], "her çıkış kendi sonucunu alır");
        assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 1, "üç çıkış, tek yalıtılmış çözüm");
        let st = decoder.stats();
        assert_eq!(st.decodes, 1);
        assert_eq!(st.joined + st.hits, 2, "{st:?}");
    }

    #[test]
    fn isolated_failure_notifies_without_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wp.jxl");
        std::fs::write(&path, b"x").unwrap();
        for f in [nuxut_image::SandboxFailure::Unavailable, nuxut_image::SandboxFailure::Protocol, nuxut_image::SandboxFailure::Timeout] {
            let fake = Arc::new(FakeIsolated { fail: Some(f), ..FakeIsolated::default() });
            let decoder = crate::services::image_sandbox::wallpaper_decoder_with(fake.clone());
            let (surfaces, rx) = surfaces(&decoder, &path, 1);
            rx.recv_timeout(std::time::Duration::from_secs(5)).expect("hata da bildirilir (kare istenir, zemin rengi çizilir)");
            assert_eq!(decoder.stats().failed, 1, "{f:?}");
            drop(surfaces);
        }
        // Var olmayan dosya: işçiye gitmeden Io hatası, yine panik yok.
        let fake = Arc::new(FakeIsolated::default());
        let decoder = crate::services::image_sandbox::wallpaper_decoder_with(fake.clone());
        let (_s, rx) = surfaces(&decoder, &dir.path().join("yok.png"), 1);
        rx.recv_timeout(std::time::Duration::from_secs(5)).expect("bildirim");
        assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn report_logs_each_outcome_once() {
        assert_eq!(Reported::from_parts(None, false), Reported::Nothing);
        assert_eq!(Reported::from_parts(None, true), Reported::Animated);
        assert_eq!(Reported::from_parts(Some("yalıtım kullanılamıyor"), true), Reported::Error("yalıtım kullanılamıyor".into()));
    }

    /// Yerleşik kaynak: bellekte render edilir, kare bileşene `set_image` ile gider (yalıtılmış
    /// çözücüye hiç uğramaz, dosya yazılmaz); tema kipi değişince öteki varyant istenir; dosya
    /// kaynağına geçişte bekleyen render atılır.
    #[test]
    fn builtin_renders_in_memory_and_follows_theme_mode() {
        use nuxut_wallpapers::Variant;
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeIsolated::default());
        let decoder = crate::services::image_sandbox::wallpaper_decoder_with(fake.clone());
        let renderer = Renderer::new();
        let pair = nuxut_wallpapers::default_builtin();
        let sel = nuxut_wallpapers::Selection { builtin: pair, variant: None };
        let w = WallpaperWanted { source: WallpaperSource::Builtin(sel), ..wanted(dir.path()) };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut theme = Theme { mode: ThemeMode::Dark, ..Theme::default() };
        let mut s = WallpaperSurface::new(SurfaceId::for_test(9), w, &theme, decoder, renderer.clone(), move || {
            let _ = tx.send(());
        });
        let image = |key| Some(nuxut_image::WallpaperSource::Image(key));
        s.resized(Size::new(64.0, 36.0), 1.5);
        let wait = || rx.recv_timeout(std::time::Duration::from_secs(20)).expect("bildirim");
        wait(); // render bitti
        assert_eq!(s.builtin.wanted(), Some(("pair-flight", Variant::Dark, (96, 54))), "tam fiziksel boyut");
        s.poll_builtin();
        assert_eq!(s.component.source().cloned(), image(nuxut_wallpapers::image_key(pair, Variant::Dark, ColorThemeId::Blue)), "kare bellekten verildi");
        assert_eq!(s.component.last_error(), None);

        // Kip açığa döndü: açık varyant.
        theme.mode = ThemeMode::Light;
        s.set_theme(&theme, Instant::now());
        wait();
        s.poll_builtin();
        assert_eq!(s.component.source().cloned(), image(nuxut_wallpapers::image_key(pair, Variant::Light, ColorThemeId::Blue)));

        // Renk teması değişti: aynı varyantın temaya boyanmışı; aynı tema yeniden istenmez.
        theme.color_theme = Some(ColorThemeId::Sage);
        s.set_theme(&theme, Instant::now());
        wait();
        s.poll_builtin();
        assert_eq!(s.component.source().cloned(), image(nuxut_wallpapers::image_key(pair, Variant::Light, ColorThemeId::Sage)));
        s.set_theme(&theme, Instant::now());
        theme.color_theme = None;
        s.set_theme(&theme, Instant::now());
        wait();
        s.poll_builtin();
        assert_eq!(s.component.source().cloned(), image(nuxut_wallpapers::image_key(pair, Variant::Light, ColorThemeId::Blue)), "duvar kâğıdından renkte Nuxut mavisi");

        // Sabit varyant kipi izlemez.
        let fixed = nuxut_wallpapers::Selection { variant: Some(Variant::Dark), ..sel };
        let w = WallpaperWanted { source: WallpaperSource::Builtin(fixed), ..s.wanted().clone() };
        s.set_wanted(w, &theme);
        assert_eq!(s.builtin.wanted().map(|k| k.1), Some(Variant::Dark));
        wait();
        s.poll_builtin();
        assert_eq!(s.component.source().cloned(), image(nuxut_wallpapers::image_key(pair, Variant::Dark, ColorThemeId::Blue)));
        assert_eq!(renderer.completed(), 5);
        assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 0, "yerleşik yalıtılmış çözücüye gitmez");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "dosya yazılmadı");

        // Dosya kaynağına geçiş: yerleşik istek temizlenir, dosya yalıtılmış çözücüye gider.
        let file = dir.path().join("wp.png");
        std::fs::write(&file, b"x").unwrap();
        s.set_wanted(wanted(&file), &theme);
        assert_eq!(s.builtin.wanted(), None);
        assert_eq!(s.component.source().cloned(), Some(nuxut_image::WallpaperSource::File(file)));
        wait();
        assert_eq!(fake.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn builtins_json_contract() {
        crate::i18n::use_turkish_for_tests();
        let v: serde_json::Value = serde_json::from_str(&builtins_json()).unwrap();
        let list = v.as_array().unwrap();
        assert_eq!(list.len(), nuxut_wallpapers::BUILTINS.len());
        let defaults: Vec<_> = list.iter().filter(|b| b["default"] == true).collect();
        assert_eq!(defaults.len(), 1);
        let d = defaults[0];
        assert_eq!(d["id"], "pair-flight");
        assert_eq!(d["source"], "nuxut:pair-flight");
        assert_eq!(d["name"], "Eş uçuş");
        assert_eq!(d["variants"], serde_json::json!(["dark", "light"]));
        assert_eq!(d["seed"], serde_json::json!({"dark": "#9ec5ff", "light": "#3f6fc6"}));
        assert!(list.iter().all(|b| b["name"].as_str().is_some_and(|n| !n.starts_with("wallpapers-"))));
    }

    #[test]
    fn fit_mapping_is_total() {
        assert_eq!(WallpaperFit::ALL.map(image_fit), [ImageFit::Cover, ImageFit::Contain, ImageFit::Fill]);
    }
}
