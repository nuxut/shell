//! Ekran görüntüsü servisi (tasarım: `docs/design/screenshot.md`): komut modeli, `[screenshot]`
//! config'i ve UI'dan bağımsız işler — yakalama (`grim`), kesim/birleştirme, PNG, kayıt, pano.
//!
//! Akış: `shot region` önce tüm çıkışları yakalar (dondurulmuş kare, [`capture_outputs`]),
//! sonra UI seçim katmanını açar; onayda kesim dondurulmuş kareden yapılır (ikinci yakalama
//! yok: görülen = kaydedilen, katman görüntüye girmez). `window`/`output`/`screen` katmansız:
//! yakala → (birleştir) → teslim et ([`deliver`]) tek arka plan işinde. Bloklayan her şey arka
//! plan thread'lerinde; sonuç [`ShotEvent`] olarak ana döngüye döner.

pub mod capture;
pub mod geometry;
pub mod hypr;
pub mod save;
pub mod selection;

use std::path::PathBuf;

use nuxut::i18n::t;
use serde::Deserialize;

use self::capture::Source;
use self::geometry::{LogicalRect, RgbaImage, Tile};
use crate::schema::SettingSchema;

/// Ne yakalanacak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotMode {
    /// Dondurulmuş kare üstünde seçim (seçim yokken tıklanan pencere = pencere).
    Region,
    /// Odaklı pencere (Hyprland `activewindow`).
    Window,
    /// Odaklı ya da `--output` çıkışı.
    Output,
    /// Tüm çıkışlar, düzenlerine göre birleştirilmiş.
    Screen,
}

impl ShotMode {
    pub fn name(self) -> &'static str {
        match self {
            ShotMode::Region => "region",
            ShotMode::Window => "window",
            ShotMode::Output => "output",
            ShotMode::Screen => "screen",
        }
    }
}

/// `shot …` komutu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShotCommand {
    Capture { mode: ShotMode, output: Option<String>, no_save: bool, no_copy: bool },
    /// Bildirim düğmesi: kaydedilen dosyayı aç (bellekteki kayıt numarası).
    Open(u32),
    /// Bildirim düğmesi: dosyayı klasöründe göster.
    Reveal(u32),
}

pub const USAGE: &str = "kullanım: shot region|window|output|screen [--output <ad>] [--no-save] [--no-copy] | shot open|reveal <n>"; // i18n: tanı msg yanıtı/bildirim ayrıntısı

/// `shot` sonrası argümanlar (`--output` `ipc` tarafından zaten ayrıldı).
pub fn parse_command(rest: &[&str], output: Option<String>) -> Result<ShotCommand, String> {
    let (sub, flags) = rest.split_first().ok_or(USAGE)?;
    let mode = match *sub {
        "region" => ShotMode::Region,
        "window" => ShotMode::Window,
        "output" => ShotMode::Output,
        "screen" => ShotMode::Screen,
        "open" | "reveal" => {
            let n = match (flags, &output) {
                ([n], None) => n.parse::<u32>().ok().filter(|v| *v > 0 && !n.starts_with(['0', '+'])),
                _ => None,
            }
            .ok_or(USAGE)?;
            return Ok(if *sub == "open" { ShotCommand::Open(n) } else { ShotCommand::Reveal(n) });
        }
        _ => return Err(USAGE.into()),
    };
    if output.is_some() && matches!(mode, ShotMode::Window | ShotMode::Screen) {
        return Err(format!("--output {} ile kullanılamaz", mode.name())); // i18n: tanı msg yanıtı/bildirim ayrıntısı
    }
    let (mut no_save, mut no_copy) = (false, false);
    for f in flags {
        match *f {
            "--no-save" if !no_save => no_save = true,
            "--no-copy" if !no_copy => no_copy = true,
            _ => return Err(USAGE.into()),
        }
    }
    if no_save && no_copy {
        return Err("--no-save ve --no-copy birlikte: yapılacak bir şey yok".into()); // i18n: tanı msg yanıtı/bildirim ayrıntısı
    }
    Ok(ShotCommand::Capture { mode, output, no_save, no_copy })
}

/// `[screenshot]` (shell.toml).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ScreenshotConfig {
    /// Kayıt dizini; boş = `$XDG_PICTURES_DIR/<yerelleştirilmiş "Ekran Görüntüleri">`. Mutlak ya
    /// da `~/` ile başlar.
    pub dir: String,
    /// Panoya kopyala.
    pub copy: bool,
    /// Dosyaya kaydet.
    pub save: bool,
    /// Bitince bildirim.
    pub notify: bool,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self { dir: String::new(), copy: true, save: true, notify: true }
    }
}

const MAX_DIR_BYTES: usize = 4096;

impl ScreenshotConfig {
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(SettingSchema::text("screenshot.dir", t!("shell-config-screenshot-dir-label"), d.dir).describe(t!("shell-config-screenshot-dir-describe")));
        out.push(SettingSchema::bool("screenshot.save", t!("shell-config-screenshot-save-label"), d.save));
        out.push(SettingSchema::bool("screenshot.copy", t!("shell-config-screenshot-copy-label"), d.copy));
        out.push(SettingSchema::bool("screenshot.notify", t!("shell-config-screenshot-notify-label"), d.notify));
    }

    pub fn validate(&mut self, w: &mut Vec<String>) {
        let d = self.dir.trim();
        let ok = d.is_empty()
            || (d.len() <= MAX_DIR_BYTES
                && (d.starts_with('/') || d.starts_with("~/"))
                && !d.chars().any(char::is_control)
                && !std::path::Path::new(d).components().any(|c| matches!(c, std::path::Component::ParentDir)));
        if !ok {
            w.push(t!("shell-config-screenshot-dir-invalid"));
            self.dir.clear();
        } else if d.len() != self.dir.len() {
            self.dir = d.to_owned();
        }
        if !self.save && !self.copy {
            w.push(t!("shell-config-screenshot-nothing-enabled"));
            self.save = true;
        }
    }
}

/// Bir yakalamanın nereye teslim edileceği (ana thread'de çözülür: yerelleştirilmiş adlar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// Kayıt dizini; `None` = kaydetme.
    pub dir: Option<PathBuf>,
    /// Dosya adı gövdesi (`.png` ve çakışma soneki hariç).
    pub stem: String,
    pub copy: bool,
    /// Bildirim önizlemesi üretilsin mi (`screenshot.notify`).
    pub preview: bool,
}

impl Delivery {
    /// Config + bayraklar + şimdiki zaman → teslim planı. Ev dizini yoksa kayıt yapılamaz.
    pub fn plan(config: &ScreenshotConfig, no_save: bool, no_copy: bool, now: chrono::DateTime<chrono::Local>) -> Result<Self, String> {
        let save = config.save && !no_save;
        let copy = config.copy && !no_copy;
        if !save && !copy {
            return Err("kaydetme ve kopyalama kapalı (screenshot.save/copy ya da --no-save/--no-copy)".into()); // i18n: tanı msg yanıtı/bildirim ayrıntısı
        }
        let dir = if save {
            let (home, config_home) = save::home_and_config().ok_or("HOME bulunamadı")?; // i18n: tanı msg yanıtı/bildirim ayrıntısı
            let pictures = save::pictures_dir(&home, &config_home);
            Some(save::save_dir(&config.dir, &home, &pictures, &t!("shell-screenshot-dir-name")))
        } else {
            None
        };
        let stem = t!("shell-screenshot-file-name", time = save::timestamp(now));
        Ok(Self { dir, stem, copy, preview: config.notify })
    }
}

/// Teslim sonucu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Delivered {
    pub saved: Option<PathBuf>,
    pub copied: bool,
    /// Kısmi hata (biri başarılı, öbürü değil).
    pub warning: Option<String>,
    /// Bildirim önizlemesi: en uzun kenarı [`PREVIEW_MAX_SIDE`] olan, alan ortalamasıyla
    /// küçültülmüş kopya (teslim thread'inde üretilir; ana thread yalnızca taşır).
    pub preview: Option<RgbaImage>,
}

/// Önizlemenin en uzun kenarı: bildirim içerik görseli sınırı.
pub const PREVIEW_MAX_SIDE: u32 = crate::services::notifications::images::IMAGE_MAX_SIDE;

/// Kodla, kaydet, kopyala (bloklar). İstenen her şey başarısızsa hata; biri başarılıysa
/// diğerinin hatası `warning`.
pub fn deliver(image: &RgbaImage, d: &Delivery) -> Result<Delivered, String> {
    let bytes = save::encode_png(image).map_err(|e| e.to_string())?;
    let mut out = Delivered::default();
    let mut errors = Vec::new();
    if let Some(dir) = &d.dir {
        match save::save_png(dir, &d.stem, &bytes) {
            Ok(p) => out.saved = Some(p),
            Err(e) => errors.push(e.to_string()),
        }
    }
    if d.copy {
        match save::copy_png(&bytes) {
            Ok(()) => out.copied = true,
            Err(e) => errors.push(e.to_string()),
        }
    }
    if out.saved.is_none() && !out.copied {
        return Err(errors.join("; "));
    }
    out.warning = (!errors.is_empty()).then(|| errors.join("; "));
    if d.preview {
        out.preview = geometry::thumbnail(image, PREVIEW_MAX_SIDE);
    }
    Ok(out)
}

/// `msg shot` yanıtı (tek satır JSON).
pub fn result_json(result: Option<&Delivered>) -> String {
    match result {
        None => serde_json::json!({ "cancelled": true }).to_string(),
        Some(d) => {
            let mut v = serde_json::json!({
                "saved": d.saved.as_ref().map(|p| p.to_string_lossy().into_owned()),
                "copied": d.copied,
            });
            if let (Some(w), Some(o)) = (&d.warning, v.as_object_mut()) {
                o.insert("warning".into(), w.clone().into());
            }
            v.to_string()
        }
    }
}

/// Bir çıkışın adı ve mantıksal düzeni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputLayout {
    pub name: String,
    pub layout: LogicalRect,
}

/// Yakalanmış çıkış (dondurulmuş kare).
#[derive(Debug, Clone)]
pub struct Frame {
    pub name: String,
    pub layout: LogicalRect,
    pub image: RgbaImage,
}

/// Çıkışları paralel yakalar (çıkış başına bir grim). Biri başarısızsa hata.
pub fn capture_outputs(outputs: &[OutputLayout]) -> Result<Vec<Frame>, String> {
    if outputs.is_empty() {
        return Err("çıkış yok".into()); // i18n: tanı msg yanıtı/bildirim ayrıntısı
    }
    let results: Vec<Result<RgbaImage, String>> = std::thread::scope(|s| {
        let handles: Vec<_> = outputs
            .iter()
            .map(|o| s.spawn(move || capture::capture(&Source::Output(o.name.clone())).map_err(|e| format!("{}: {e}", o.name))))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("yakalama thread'i çöktü".into()))).collect() // i18n: tanı msg yanıtı/bildirim ayrıntısı
    });
    outputs
        .iter()
        .zip(results)
        .map(|(o, r)| r.map(|image| Frame { name: o.name.clone(), layout: o.layout, image }))
        .collect()
}

/// Çerçeveleri düzenlerine göre birleştirir (`screen`).
pub fn stitch_frames(frames: &[Frame]) -> Result<RgbaImage, String> {
    let tiles: Vec<Tile<'_>> = frames.iter().map(|f| Tile { layout: f.layout, image: &f.image }).collect();
    geometry::stitch(&tiles).ok_or_else(|| "çıkışlar birleştirilemedi".into()) // i18n: tanı msg yanıtı/bildirim ayrıntısı
}

/// Arka plan işlerinden ana döngüye.
#[derive(Debug)]
pub enum ShotEvent {
    /// `region`: dondurulmuş kareler ve görünür pencereler hazır.
    Frames { session: u64, result: Result<(Vec<Frame>, Vec<LogicalRect>), String> },
    /// Teslim bitti (her kip).
    Done { session: u64, result: Result<Delivered, String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(mode: ShotMode, output: Option<&str>, no_save: bool, no_copy: bool) -> ShotCommand {
        ShotCommand::Capture { mode, output: output.map(Into::into), no_save, no_copy }
    }

    #[test]
    fn parse_shot_commands() {
        assert_eq!(parse_command(&["region"], None), Ok(capture(ShotMode::Region, None, false, false)));
        assert_eq!(parse_command(&["output", "--no-copy"], Some("DP-1".into())), Ok(capture(ShotMode::Output, Some("DP-1"), false, true)));
        assert_eq!(parse_command(&["screen", "--no-save"], None), Ok(capture(ShotMode::Screen, None, true, false)));
        assert_eq!(parse_command(&["region", "--no-save"], Some("HDMI-A-1".into())), Ok(capture(ShotMode::Region, Some("HDMI-A-1"), true, false)));
        assert!(parse_command(&["window"], Some("DP-1".into())).is_err());
        assert!(parse_command(&["screen"], Some("DP-1".into())).is_err());
        assert!(parse_command(&["region", "--no-save", "--no-copy"], None).is_err());
        assert!(parse_command(&["region", "--no-save", "--no-save"], None).is_err());
        assert!(parse_command(&["region", "x"], None).is_err());
        assert!(parse_command(&[], None).is_err());
        assert!(parse_command(&["full"], None).is_err());
        assert_eq!(parse_command(&["open", "3"], None), Ok(ShotCommand::Open(3)));
        assert_eq!(parse_command(&["reveal", "12"], None), Ok(ShotCommand::Reveal(12)));
        assert!(parse_command(&["open", "0"], None).is_err());
        assert!(parse_command(&["open", "03"], None).is_err());
        assert!(parse_command(&["open", "-1"], None).is_err());
        assert!(parse_command(&["open"], None).is_err());
        assert!(parse_command(&["open", "1"], Some("DP-1".into())).is_err());
    }

    #[test]
    fn config_validation() {
        let mut w = Vec::new();
        let mut c = ScreenshotConfig { dir: "relative/x".into(), ..Default::default() };
        c.validate(&mut w);
        assert_eq!((c.dir.as_str(), w.len()), ("", 1));
        let mut c = ScreenshotConfig { dir: " ~/Shots ".into(), ..Default::default() };
        c.validate(&mut w);
        assert_eq!((c.dir.as_str(), w.len()), ("~/Shots", 1));
        let mut c = ScreenshotConfig { dir: "/a/../b".into(), ..Default::default() };
        c.validate(&mut w);
        assert_eq!(c.dir, "");
        let mut c = ScreenshotConfig { save: false, copy: false, ..Default::default() };
        let mut w = Vec::new();
        c.validate(&mut w);
        assert!(c.save && w.len() == 1);
    }

    #[test]
    fn delivery_plan_respects_flags() {
        crate::i18n::use_turkish_for_tests();
        use chrono::TimeZone;
        let now = chrono::Local.with_ymd_and_hms(2026, 10, 12, 14, 3, 22).unwrap();
        let c = ScreenshotConfig { dir: "/tmp/nuxut-shots".into(), ..Default::default() };
        let d = Delivery::plan(&c, false, false, now).unwrap();
        assert_eq!(d.dir, Some(PathBuf::from("/tmp/nuxut-shots")));
        assert!(d.copy);
        assert_eq!(d.stem, "Ekran görüntüsü 2026-10-12 14-03-22");
        let d = Delivery::plan(&c, true, false, now).unwrap();
        assert_eq!((d.dir, d.copy), (None, true));
        let only_copy = ScreenshotConfig { save: false, ..c.clone() };
        assert!(Delivery::plan(&only_copy, false, true, now).is_err());
    }

    #[test]
    fn json_replies() {
        assert_eq!(result_json(None), r#"{"cancelled":true}"#);
        let d = Delivered { saved: Some("/p/a b.png".into()), copied: true, warning: None, preview: None };
        let v: serde_json::Value = serde_json::from_str(&result_json(Some(&d))).unwrap();
        assert_eq!(v["saved"], "/p/a b.png");
        assert_eq!(v["copied"], true);
        let d = Delivered { saved: None, copied: true, warning: Some("x".into()), preview: None };
        let v: serde_json::Value = serde_json::from_str(&result_json(Some(&d))).unwrap();
        assert!(v["saved"].is_null());
        assert_eq!(v["warning"], "x");
    }

    #[test]
    fn deliver_saves_without_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let img = RgbaImage::new(1, 1, vec![1, 2, 3, 255]).unwrap();
        let d = Delivery { dir: Some(tmp.path().join("s")), stem: "a".into(), copy: false, preview: false };
        let r = deliver(&img, &d).unwrap();
        assert_eq!(r.saved, Some(tmp.path().join("s/a.png")));
        assert!(!r.copied && r.warning.is_none());
        let bytes = std::fs::read(tmp.path().join("s/a.png")).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
        assert!(r.preview.is_none(), "önizleme istenmedi");
        // Önizleme istenince en uzun kenar sınıra iner, en-boy korunur.
        let wide = RgbaImage::new(600, 200, [9, 8, 7, 255].repeat(600 * 200)).unwrap();
        let d = Delivery { dir: Some(tmp.path().join("s")), stem: "b".into(), copy: false, preview: true };
        let p = deliver(&wide, &d).unwrap().preview.unwrap();
        assert_eq!((p.width, p.height), (PREVIEW_MAX_SIDE, 85));
        assert_eq!(p.pixel(10, 10), [9, 8, 7, 255]);
    }
}
