//! Duvar kağıdı yüzeyleri: config eşleme, çözüm bitişi, çizim. Kare her zaman durağandır
//! (animasyonlu kaynakta ilk kare): animasyon timer'ı yok.

use std::time::Instant;

use nuxut::{Context, OutputId, Painter, Size, SurfaceId};

use super::{Shell, ShellMessage};
use crate::wallpaper::WallpaperSurface;

impl Shell {
    /// Ekranın duvar kağıdı yüzeyini config'le eşler: yoksa yaratır, kaldırıldıysa yok eder,
    /// kaynak/yerleşim/zemin değiştiyse günceller (çapraz geçiş bileşende) ve kare ister.
    pub(super) fn sync_wallpaper(&mut self, cx: &mut Context<Self>, idx: usize) {
        let wanted = self.model.config.wallpaper.for_output(&self.screens[idx].name);
        let output = self.screens[idx].output;
        match (self.screens[idx].wallpaper.as_mut(), wanted) {
            (Some(wp), Some(w)) => {
                if wp.wanted() != &w {
                    wp.set_wanted(w, cx.theme());
                    cx.request_frame(wp.surface);
                }
            }
            (Some(_), None) => {
                if let Some(wp) = self.screens[idx].wallpaper.take() {
                    destroy_wallpaper(cx, &wp);
                }
            }
            (None, Some(w)) => {
                let surface = cx.create_layer_surface(WallpaperSurface::spec(output));
                cx.a11y_exclude(surface);
                cx.set_input_region(surface, &[]);
                let sender = cx.sender();
                let theme = cx.theme().clone();
                // Süreçteki tek çözücü: bütün çıkışlar paylaşır.
                let wp = WallpaperSurface::new(surface, w, &theme, self.wallpaper_decoder.clone(), self.wallpaper_builtins.clone(), move || {
                    let _ = sender.send(ShellMessage::WallpaperReady(output));
                });
                self.screens[idx].wallpaper = Some(wp);
            }
            (None, None) => {}
        }
    }

    /// Duvar kağıdı çözümü bitti (başarı ya da hata): o monitörün yüzeyine kare iste. Sonuç
    /// çizimde alınır; hata orada loglanır ve zemin rengi kalır.
    pub(super) fn on_wallpaper_ready(&mut self, cx: &mut Context<Self>, output: OutputId) {
        if let Some(wp) = self.screens.iter().find(|s| s.output == output).and_then(|s| s.wallpaper.as_ref()) {
            cx.request_frame(wp.surface);
        }
    }

    /// Duvar kağıdı yüzeyiyse boyutu uygular ve `true` döner.
    pub(super) fn wallpaper_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, scale: f32) -> bool {
        let Some(wp) = self.wallpaper_mut(surface) else { return false };
        wp.resized(size, scale);
        cx.request_frame(surface);
        true
    }

    /// Duvar kağıdı yüzeyiyse çizer ve `true` döner.
    pub(super) fn draw_wallpaper(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) -> bool {
        let Some(wp) = self.wallpaper_mut(surface) else { return false };
        if wp.draw(painter, now) {
            cx.request_frame(surface);
        }
        true
    }

    fn wallpaper_mut(&mut self, surface: SurfaceId) -> Option<&mut WallpaperSurface> {
        self.screens.iter_mut().filter_map(|s| s.wallpaper.as_mut()).find(|w| w.surface == surface)
    }
}

/// Yüzeyi kaldırır (bekleyen çözüm bileşenle birlikte düşer: bilet iptal edilir).
pub(super) fn destroy_wallpaper(cx: &mut Context<Shell>, wp: &WallpaperSurface) {
    cx.destroy_surface(wp.surface);
}
