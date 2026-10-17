//! Ekran paylaşım seçicisinin pencere önizleme yolunu canlı Hyprland'de doğrular
//! (docs/design/screenshare-picker.md, "Canlı deneme"): yüzey açmaz, odak almaz, girdi
//! göndermez. Verilen Hyprland pencere adresini (`hyprctl clients -j` → `address`, onaltılık)
//! seçicinin yaptığı gibi `toplevel_by_hyprland_address` ile tutamaca çevirir ve
//! `CaptureOptions::preview` ile tek kare yakalar. Pikseller yalnız bellekte denetlenir (boyut,
//! en-boy, opaklık, renk çeşitliliği); dosya yazılmaz, piksel loglanmaz.
//!
//! Yalnız test için açılmış KENDİ penceren (ör. `kitty --class nuxut-test`) ile kullan:
//!
//! ```sh
//! cargo run --example sharepicker_preview_probe -- 0x55d0c0ffee00 320 200
//! ```

use std::collections::HashSet;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nuxut::{
    App, CaptureError, CaptureId, CaptureOptions, CaptureSource, CapturedImage, Context, OutputId, OutputInfo, PointerEvent,
    Size, SurfaceId, TimerId, ToplevelEvent,
};

static PASSED: AtomicBool = AtomicBool::new(false);

struct Probe {
    address: u64,
    box_px: (u32, u32),
    started: bool,
    pending: Option<CaptureId>,
    deadline: Option<TimerId>,
}

impl Probe {
    fn try_capture(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let Some(id) = cx.toplevel_by_hyprland_address(self.address) else { return };
        println!("tutamaç bulundu; destek: {:?}", cx.capture_support());
        self.pending = Some(cx.capture(CaptureSource::Toplevel(id), CaptureOptions::preview(self.box_px.0, self.box_px.1)));
    }

    fn finish(cx: &mut Context<Self>) {
        cx.toplevels_stop();
        cx.quit();
    }
}

impl App for Probe {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, _: &OutputInfo) {
        if self.started {
            return;
        }
        self.started = true;
        self.deadline = Some(cx.set_timer(Duration::from_secs(10)));
        if !cx.capture_support().toplevel {
            println!("FAIL compositor pencere yakalamayı sunmuyor: {:?}", cx.capture_support());
            return Self::finish(cx);
        }
        if let Err(e) = cx.toplevels_start() {
            println!("FAIL pencere listesi: {e}");
            return Self::finish(cx);
        }
        self.try_capture(cx);
    }

    fn output_removed(&mut self, _: &mut Context<Self>, _: OutputId) {}
    fn surface_resized(&mut self, _: &mut Context<Self>, _: SurfaceId, _: Size, _: f32) {}
    fn pointer(&mut self, _: &mut Context<Self>, _: SurfaceId, _: PointerEvent) {}

    fn toplevel(&mut self, cx: &mut Context<Self>, event: ToplevelEvent) {
        if let ToplevelEvent::Added(_) = event {
            self.try_capture(cx);
        }
    }

    fn captured(&mut self, cx: &mut Context<Self>, id: CaptureId, result: Result<CapturedImage, CaptureError>) {
        if Some(id) != self.pending {
            return;
        }
        match result {
            Err(e) => println!("FAIL yakalama: {e:?}"),
            Ok(c) => {
                let (w, h) = (c.image.width, c.image.height);
                let (sw, sh) = c.source_size;
                let colors: HashSet<[u8; 4]> =
                    c.image.pixels.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
                let opaque = c.image.pixels.chunks_exact(4).filter(|p| p[3] == 255).count();
                let total = (w * h) as usize;
                println!("kare {w}x{h} (kaynak {sw}x{sh}), {} renk, opak %{}", colors.len(), opaque * 100 / total.max(1));
                // Kutuya sığmış (en-boy korunmuş) ve boş değil.
                let fits = w <= self.box_px.0 && h <= self.box_px.1 && (w == self.box_px.0 || h == self.box_px.1);
                let aspect = |a: u32, b: u32| a as f32 / b.max(1) as f32;
                let aspect_ok = (aspect(w, h) - aspect(sw, sh)).abs() < 0.05;
                let ok = fits && aspect_ok && colors.len() > 1 && opaque * 2 > total;
                println!("{} sığma={fits} en-boy={aspect_ok}", if ok { "PASS" } else { "FAIL" });
                PASSED.store(ok, Ordering::Relaxed);
            }
        }
        Self::finish(cx);
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if Some(id) == self.deadline {
            println!("FAIL zaman aşımı (tutamaç {})", if self.pending.is_some() { "bulundu" } else { "bulunamadı" });
            Self::finish(cx);
        }
    }

    fn draw(&mut self, _: &mut Context<Self>, _: SurfaceId, _: std::time::Instant, _: &mut nuxut::Painter) {}
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parse_address = |s: &str| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok();
    let (Some(address), Some(w), Some(h)) = (
        args.first().and_then(|a| parse_address(a)),
        args.get(1).and_then(|v| v.parse().ok()),
        args.get(2).and_then(|v| v.parse().ok()),
    ) else {
        eprintln!("kullanım: sharepicker_preview_probe <0xadres> <genişlik> <yükseklik>");
        return ExitCode::from(2);
    };
    let probe = Probe { address, box_px: (w, h), started: false, pending: None, deadline: None };
    if let Err(e) = nuxut::run(probe) {
        eprintln!("çalışma zamanı: {e}");
        return ExitCode::from(2);
    }
    if PASSED.load(Ordering::Relaxed) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
