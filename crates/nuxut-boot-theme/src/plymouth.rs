//! Plymouth teması (`/usr/share/plymouth/themes/nuxut/`).
//!
//! Modül **two-step** (Plymouth'un C eklentisi; Fedora/GNOME'un `spinner`/`bgrt` temaları):
//! kendi betik (`script`) modülümüzü yazmadık, çünkü LUKS parola diyaloğu, büyük harf kilidi ve
//! klavye düzeni göstergesi, çoklu ekran ve sıcak takma two-step'te sınanmış C kodudur; betikteki
//! bir hata parola sorusunu görünmez bırakabilirdi (kullanıcı kör yazar). Karar:
//! 2026-10-16 (`docs/design/boot-theme.md`).
//!
//! Düzen: ortada logo (watermark), altında ince ilerleme çubuğu (açılış) ya da sakin dönen
//! gösterge (kapanış/yeniden başlatma); parola diyaloğu logonun altında. Renkler seçili paletin
//! koyu kipinden. HiDPI: varlıklar 1× mantıksal boyutta; Plymouth yüksek DPI ekranda
//! (`DeviceScale`) kendisi büyütür, metni ise o ölçekte çizer (yukarı akış temalarıyla aynı).

use crate::art;
use crate::files::{Lang, ThemeFiles, ThemeOptions};
use crate::raster::RasterError;

/// Tema adı (dizin ve `.plymouth` dosyası).
pub const THEME_NAME: &str = "nuxut";
/// Kurulu tema dizini (`ImageDir`).
pub const THEME_DIR: &str = "/usr/share/plymouth/themes/nuxut";

/// Düzen: two-step hizalamaları (0–1, öğenin ekran içindeki konumu) ve boyutlar (mantıksal px).
/// Önizleme aynı değerleri kullanır.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub logo: u32,
    pub logo_y: f32,
    pub progress_y: f32,
    pub progress_w: u32,
    pub progress_h: u32,
    pub throbber: u32,
    pub dialog_y: f32,
    pub entry_w: u32,
    pub entry_h: u32,
    pub bullet: u32,
    pub icon: u32,
    pub title_y: f32,
}

pub const LAYOUT: Layout = Layout {
    logo: 128,
    logo_y: 0.42,
    progress_y: 0.56,
    progress_w: 200,
    progress_h: 4,
    throbber: 28,
    dialog_y: 0.60,
    entry_w: 300,
    entry_h: 40,
    bullet: 8,
    icon: 24,
    title_y: 0.30,
};

/// Dönen göstergenin kare sayısı (two-step ~30 kare/sn oynatır: 36 kare ≈ 1,2 sn tur; sakin).
pub const THROBBER_FRAMES: u32 = 36;

/// Güncelleme kiplerinin başlıkları (two-step `[updates]` vb.).
fn titles(lang: Lang) -> [(&'static str, &'static str, &'static str); 4] {
    match lang {
        Lang::Tr => [
            ("updates", "Güncellemeler kuruluyor…", "Bilgisayarı kapatmayın"),
            ("system-upgrade", "Sistem yükseltiliyor…", "Bilgisayarı kapatmayın"),
            ("firmware-upgrade", "Ürün yazılımı yükseltiliyor…", "Bilgisayarı kapatmayın"),
            ("system-reset", "Sistem sıfırlanıyor…", "Bilgisayarı kapatmayın"),
        ],
        Lang::En => [
            ("updates", "Installing Updates…", "Do not turn off your computer"),
            ("system-upgrade", "Upgrading System…", "Do not turn off your computer"),
            ("firmware-upgrade", "Upgrading Firmware…", "Do not turn off your computer"),
            ("system-reset", "Resetting System…", "Do not turn off your computer"),
        ],
    }
}

/// `nuxut.plymouth` içeriği.
pub fn config(o: &ThemeOptions) -> String {
    let c = &o.colors;
    let l = LAYOUT;
    let mut s = String::new();
    s.push_str("# Nuxut önyükleme teması. nuxut-system-helper üretir; elle değiştirmeyin\n");
    s.push_str("# (Nuxut Hoş geldin / `nuxut-shell system-theme apply` yeniden yazar).\n");
    s.push_str("[Plymouth Theme]\nName=Nuxut\nDescription=Nuxut boot splash\nDescription[tr]=Nuxut açılış ekranı\nModuleName=two-step\n\n");
    s.push_str("[two-step]\n");
    s.push_str(&format!("Font={} 12\nTitleFont={} Light 22\nMonospaceFont={} 11\n", o.family, o.family, o.mono_family));
    s.push_str(&format!("ImageDir={THEME_DIR}\n"));
    s.push_str(&format!("DialogHorizontalAlignment=.5\nDialogVerticalAlignment={}\n", l.dialog_y));
    s.push_str(&format!("TitleHorizontalAlignment=.5\nTitleVerticalAlignment={}\n", l.title_y));
    s.push_str(&format!("HorizontalAlignment=.5\nVerticalAlignment={}\n", l.progress_y));
    s.push_str(&format!("WatermarkHorizontalAlignment=.5\nWatermarkVerticalAlignment={}\n", l.logo_y));
    s.push_str("Transition=none\nTransitionDuration=0.0\n");
    s.push_str(&format!("BackgroundStartColor={}\nBackgroundEndColor={}\n", c.surface.plymouth(), c.surface.plymouth()));
    s.push_str(&format!(
        "ProgressBarBackgroundColor={}\nProgressBarForegroundColor={}\n",
        c.surface_container_highest.plymouth(),
        c.primary.plymouth()
    ));
    s.push_str(&format!(
        "ProgressBarHorizontalAlignment=.5\nProgressBarVerticalAlignment={}\nProgressBarWidth={}\nProgressBarHeight={}\n",
        l.progress_y, l.progress_w, l.progress_h
    ));
    s.push_str(&format!("ConsoleLogBackgroundColor={}\nConsoleLogTextColor={}\n", c.surface.plymouth(), c.on_surface.plymouth()));
    s.push_str("MessageBelowAnimation=true\n\n");
    s.push_str("[boot-up]\nUseEndAnimation=false\nUseFirmwareBackground=false\nUseProgressBar=true\nUseAnimation=false\n\n");
    for mode in ["shutdown", "reboot"] {
        s.push_str(&format!("[{mode}]\nUseEndAnimation=false\nUseFirmwareBackground=false\nUseProgressBar=false\nUseAnimation=true\n\n"));
    }
    for (mode, title, sub) in titles(o.lang) {
        s.push_str(&format!(
            "[{mode}]\nSuppressMessages=true\nProgressBarShowPercentComplete=true\nUseProgressBar=true\nTitle={title}\nSubTitle={sub}\n\n"
        ));
    }
    s
}

/// Temanın tüm dosyaları. `keymap-render.png` (klavye düzeni adları şeridi) burada üretilmez:
/// yardımcı varsa sistemdeki `spinner` temasınınkini kopyalar; yoksa two-step düzen adını göstermez.
pub fn generate(o: &ThemeOptions) -> Result<ThemeFiles, RasterError> {
    o.validate().map_err(RasterError)?;
    let c = &o.colors;
    let l = LAYOUT;
    let mut f = ThemeFiles::default();
    f.push(format!("{THEME_NAME}.plymouth"), config(o).into_bytes());
    f.push("watermark.png", art::logo_png(c, l.logo)?);
    f.push("entry.png", art::svg_entry(l.entry_w, l.entry_h, c)?);
    f.push("bullet.png", art::dot_png(l.bullet, c.on_surface)?);
    f.push("lock.png", art::lock_png(l.icon, c.on_surface_variant)?);
    f.push("capslock.png", art::capslock_png(l.icon, c.error)?);
    f.push("keyboard.png", art::keyboard_png(l.icon, c.on_surface_variant)?);
    for i in 0..THROBBER_FRAMES {
        let png = art::throbber_png(l.throbber, i, THROBBER_FRAMES, c.primary, c.surface_container_highest)?;
        // two-step hem `throbber-*` (dönen) hem `animation-*` (bitiş animasyonu) dizilerini arar;
        // bitiş animasyonu kapalı ama eksik dizi sürüme göre temayı düşürmesin diye ikisi de var.
        f.push(format!("animation-{:04}.png", i + 1), png.clone());
        f.push(format!("throbber-{:04}.png", i + 1), png);
    }
    Ok(f)
}

/// `plymouthd.conf`'ta `[Daemon] Theme=` değerini `theme` yapar; diğer her satır korunur.
/// Bölüm yoksa eklenir. Satır sonu biçimi korunur, sonda tek `\n` garanti edilir.
pub fn set_daemon_theme(conf: &str, theme: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_daemon = false;
    let mut seen_daemon = false;
    let mut written = false;
    for line in conf.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            if in_daemon && !written {
                out.push(format!("Theme={theme}"));
                written = true;
            }
            in_daemon = t.eq_ignore_ascii_case("[daemon]");
            seen_daemon |= in_daemon;
            out.push(line.to_owned());
            continue;
        }
        if in_daemon
            && !t.starts_with('#')
            && !t.starts_with(';')
            && let Some((k, _)) = t.split_once('=')
            && k.trim() == "Theme"
        {
            if !written {
                out.push(format!("Theme={theme}"));
                written = true;
            }
            continue;
        }
        out.push(line.to_owned());
    }
    if in_daemon && !written {
        out.push(format!("Theme={theme}"));
        written = true;
    }
    if !seen_daemon {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push("[Daemon]".into());
        out.push(format!("Theme={theme}"));
        written = true;
    }
    debug_assert!(written);
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// `plymouthd.conf`'taki etkin tema (`[Daemon] Theme=`), yoksa `None`.
pub fn daemon_theme(conf: &str) -> Option<String> {
    let mut in_daemon = false;
    let mut found = None;
    for line in conf.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_daemon = t.eq_ignore_ascii_case("[daemon]");
        } else if in_daemon
            && let Some((k, v)) = t.split_once('=')
            && k.trim() == "Theme"
        {
            found = Some(v.trim().to_owned());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_is_two_step_with_palette_colors() {
        let o = ThemeOptions::sample();
        let cfg = config(&o);
        assert!(cfg.contains("ModuleName=two-step"));
        assert!(cfg.contains(&format!("ImageDir={THEME_DIR}\n")));
        assert!(cfg.contains("BackgroundStartColor=0x12141a"));
        assert!(cfg.contains("ProgressBarForegroundColor=0x9ec5ff"));
        assert!(cfg.contains("Font=Noto Sans 12"));
        assert!(cfg.contains("Title=Güncellemeler kuruluyor…"));
        // Her satır bölüm, yorum, boş ya da anahtar=değer.
        for line in cfg.lines() {
            let t = line.trim();
            assert!(t.is_empty() || t.starts_with('#') || (t.starts_with('[') && t.ends_with(']')) || t.contains('='), "{line}");
        }
    }

    #[test]
    fn generate_has_required_assets() {
        let f = generate(&ThemeOptions::sample()).unwrap();
        for name in ["nuxut.plymouth", "watermark.png", "entry.png", "bullet.png", "lock.png", "capslock.png", "keyboard.png", "throbber-0001.png", "animation-0036.png"] {
            assert!(f.get(name).is_some(), "{name}");
        }
        assert!(f.files.iter().all(|(n, _)| crate::files::valid_file_name(n)));
        assert!(f.size() < 512 * 1024, "initramfs'e girer: küçük kalmalı ({} bayt)", f.size());
    }

    #[test]
    fn daemon_theme_edit_preserves_other_lines() {
        let conf = "# yorum\n[Daemon]\nShowDelay=0\nTheme=bgrt\nDeviceTimeout=8\n";
        let out = set_daemon_theme(conf, "nuxut");
        assert_eq!(out, "# yorum\n[Daemon]\nShowDelay=0\nTheme=nuxut\nDeviceTimeout=8\n");
        assert_eq!(daemon_theme(&out).as_deref(), Some("nuxut"));
        // Bölüm yoksa eklenir; boş dosya.
        assert_eq!(set_daemon_theme("", "nuxut"), "[Daemon]\nTheme=nuxut\n");
        assert_eq!(set_daemon_theme("[Other]\nA=1", "nuxut"), "[Other]\nA=1\n\n[Daemon]\nTheme=nuxut\n");
        // Daemon var ama Theme yok: bölümün sonuna (sonraki bölümden önce).
        assert_eq!(set_daemon_theme("[Daemon]\nShowDelay=0\n[X]\n", "nuxut"), "[Daemon]\nShowDelay=0\nTheme=nuxut\n[X]\n");
        // Yinelenen Theme satırları teke iner; yorumdaki Theme dokunulmaz.
        assert_eq!(set_daemon_theme("[Daemon]\n#Theme=a\nTheme=a\nTheme=b\n", "nuxut"), "[Daemon]\n#Theme=a\nTheme=nuxut\n");
        // Başka bölümdeki Theme anahtarı değişmez.
        assert_eq!(set_daemon_theme("[X]\nTheme=a\n", "nuxut"), "[X]\nTheme=a\n\n[Daemon]\nTheme=nuxut\n");
    }
}
