//! Sistem görünümü (Plymouth / GRUB / Limine) — kullanıcı tarafı: `nuxut-shell system-theme`.
//!
//! Burada yalnız okuma, hesap ve önizleme var; sisteme yazan her şey kök yardımcıda
//! (`crates/nuxut-system-helper`, ADR-0008), `pkexec` ile ve kendi polkit eylemiyle. Tasarım:
//! `docs/design/boot-theme.md`.
//!
//! - `status [--json]`: algılama + plan (yetki istemez; ESP okunamıyorsa "yönetici adımında
//!   denetlenecek"), seçili paletin koyu kipinden renkler, önyükleyici ölçeği, yardımcı kurulu mu.
//! - `preview --out <dizin> [--size GxY]`: sistem dosyasına dokunmadan PNG önizlemeler.
//! - `apply [--json] [--targets …] [--dry-run]`: yardımcıyı pkexec ile çalıştırır, olayları akıtır.
//! - `revert [--json] [--targets …]`: geri alma.
//!
//! CLI metni tanı çıktısıdır, çevrilmez (ADR-0010); Hoş geldin `--json` olaylarını kendi dilinde
//! gösterir.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use nuxut_boot_theme::detect::{self, Probe, Read};
use nuxut_boot_theme::plan::{self, ChangeKind, Note, Plan, State, Target};
use nuxut_boot_theme::preview::{self, GrubFonts, PlymouthScene};
use nuxut_boot_theme::{BootColors, Lang, Rgb, ThemeOptions, fontsel, grub, pf2, plymouth, raster};
use nuxut_palette::{Mode, Scheme};
use nuxut_system_helper::boot_theme::ApplyRequest;
use nuxut_system_helper::events::{Event, Outcome};
use serde::Serialize;

use crate::services::look::palette::{self as look_palette, RoleName, Seed};
use crate::services::system_bundle::{self, Stage};

/// Kök dosya sistemini düz okuyan algılama erişimi (kullanıcı yetkisiyle; yalnız gösterim için —
/// yardımcı her şeyi kendi doğrulanmış katmanıyla yeniden okur).
#[derive(Debug, Clone)]
pub struct StdProbe {
    pub root: PathBuf,
}

impl StdProbe {
    pub fn system() -> Self {
        Self { root: PathBuf::from("/") }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
}

impl Probe for StdProbe {
    fn read(&self, path: &str, max: usize) -> Option<Read> {
        use std::io::Read as _;
        match std::fs::File::open(self.path(path)) {
            Ok(f) => {
                let mut buf = Vec::new();
                f.take(max as u64 + 1).read_to_end(&mut buf).ok()?;
                (buf.len() <= max).then_some(Read::Data(buf))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Read::Missing),
            Err(_) => Some(Read::Denied),
        }
    }

    fn exists(&self, path: &str) -> bool {
        self.path(path).symlink_metadata().is_ok()
    }

    fn list(&self, dir: &str, max: usize) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.path(dir))
            .map(|it| it.flatten().filter_map(|e| e.file_name().into_string().ok()).take(max).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
}

fn rgb(c: nuxut::Color) -> Rgb {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb(b(c.r), b(c.g), b(c.b))
}

/// Seçili paletin koyu kipinden önyükleme renkleri (GTK/Qt'ye giden rol kümesiyle aynı türetme:
/// `look::palette::build`; koyu kipteyken Nuxut'un kendi token'ları birebir).
pub fn boot_colors(theme: &nuxut::Theme, wallpaper_meta: Option<&nuxut::theme::PaletteMeta>) -> BootColors {
    let mode = Mode::from(theme.mode);
    let palette = look_palette::build(&theme.colors, Seed::new(&theme.colors, wallpaper_meta), mode, theme.contrast);
    from_scheme(&palette.dark)
}

/// Koyu rol kümesinden.
pub fn from_scheme(s: &Scheme) -> BootColors {
    let c = |n: &str| rgb(s.c(n));
    let primary = s.c("primary");
    BootColors {
        surface: c("surface"),
        surface_container: c("surface_container"),
        surface_container_highest: c("surface_container_highest"),
        on_surface: c("on_surface"),
        on_surface_variant: c("on_surface_variant"),
        primary: rgb(primary),
        on_primary: c("on_primary"),
        primary_container: c("primary_container"),
        on_primary_container: c("on_primary_container"),
        secondary: c("secondary"),
        tertiary: c("tertiary"),
        outline_variant: c("outline_variant"),
        error: c("error"),
        logo_shade: rgb(nuxut::ui::brand_shade(primary)),
    }
}

/// Önyükleyici menüsünün ölçeği: GRUB/Limine çözünürlüğe uyamaz; ürün yazılımı genelde ekranın
/// doğal çözünürlüğünü verir. En büyük fiziksel yükseklik ve kullanıcının Hyprland ölçeğinden
/// büyüğü (1–3). `(fiziksel yükseklik, Hyprland ölçeği)`.
pub fn boot_scale(monitors: &[(u32, f64)]) -> u32 {
    let by_height = |h: u32| match h {
        h if h >= 2800 => 3,
        h if h >= 1800 => 2,
        _ => 1,
    };
    monitors
        .iter()
        .map(|&(h, s)| by_height(h).max(s.round().clamp(1.0, 3.0) as u32))
        .max()
        .unwrap_or(1)
        .clamp(1, ThemeOptions::MAX_SCALE)
}

fn monitors() -> Vec<(u32, f64)> {
    use crate::services::hyprland::{HyprlandClient, HyprlandPaths};
    HyprlandPaths::from_env()
        .ok()
        .and_then(|p| HyprlandClient::new(p.socket1).monitors().ok())
        .map(|ms| ms.iter().filter(|m| !m.disabled).map(|m| (m.height, m.scale)).collect())
        .unwrap_or_default()
}

fn lang() -> Lang {
    let var = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
    var.map_or(Lang::En, |v| Lang::from_locale(&v))
}

/// Yardımcının kurulum durumu: kökün sahip olduğu, başkasınca yazılamayan bir dosya mı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HelperState {
    Installed,
    Missing,
    /// Var ama güvensiz (kökün değil ya da yazılabilir): kullanılmaz.
    Unsafe,
}

pub fn helper_state(path: &Path) -> HelperState {
    use std::os::unix::fs::MetadataExt;
    let ok = |p: &Path| p.symlink_metadata().ok().filter(|m| !m.file_type().is_symlink() && m.uid() == 0 && m.mode() & 0o022 == 0);
    match path.symlink_metadata() {
        Err(_) => HelperState::Missing,
        Ok(_) => {
            let parent_ok = path.parent().is_some_and(|d| ok(d).is_some());
            if ok(path).is_some() && parent_ok { HelperState::Installed } else { HelperState::Unsafe }
        }
    }
}

/// `status --json` çıktısı (Hoş geldin sözleşmesi, `docs/design/boot-theme.md`).
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// Sözleşme sürümü.
    pub version: u32,
    pub plan: Plan,
    pub helper: HelperState,
    pub helper_path: &'static str,
    /// Kaynak kurulumunun hazırladığı kök dosyaları var: yardımcı yoksa ya da eskiyse uygulama
    /// önce onları kurar (tek yönetici onayı; `system_bundle`, 2026-10-17).
    pub helper_installable: bool,
    /// Kurulu kök dosyaları hazırlıktakilerden eski (Ayarlar "Güncelle" düğmesi →
    /// `system-theme install-helper`).
    pub helper_update: bool,
    /// Uygulanacak hedefler (hazır ya da zaten uygulanmış); Hoş geldin adımı bunlar boşsa gizlenir.
    pub actionable: Vec<Target>,
    pub scale: u32,
    pub lang: &'static str,
    pub colors: Vec<(&'static str, String)>,
}

pub const STATUS_VERSION: u32 = 1;

/// Mevcut temadan apply isteği (tüm uygulanabilir hedefler ya da `only`).
fn request(theme: &nuxut::Theme, only: Option<Vec<Target>>, plan: &Plan) -> ApplyRequest {
    let framework = crate::services::look::read_framework();
    let meta = if framework.wallpaper { crate::services::look::palette_meta() } else { None };
    ApplyRequest {
        targets: only.unwrap_or_else(|| plan.actionable()),
        colors: boot_colors(theme, meta.as_ref()),
        scale: boot_scale(&monitors()),
        lang: lang(),
    }
}

fn cmdline() -> String {
    std::fs::read_to_string("/proc/cmdline").unwrap_or_default()
}

pub fn status(theme: &nuxut::Theme) -> Status {
    let plan = plan::build(&detect::system(&StdProbe::system(), &cmdline()));
    let req = request(theme, None, &plan);
    let stage = system_bundle::stage_dir();
    Status {
        version: STATUS_VERSION,
        actionable: plan.actionable(),
        plan,
        helper: helper_state(Path::new(nuxut_system_helper::INSTALL_PATH)),
        helper_path: nuxut_system_helper::INSTALL_PATH,
        helper_installable: stage.as_deref().is_some_and(|s| system_bundle::stage_state(s) == Stage::Ready),
        helper_update: stage.as_deref().is_some_and(|s| system_bundle::update_available(s, Path::new("/"))),
        scale: req.scale,
        lang: req.lang.code(),
        colors: req.colors.pairs().into_iter().map(|(n, c)| (n, c.css())).collect(),
    }
}

fn note_text(n: &Note) -> String {
    match n {
        Note::PlymouthNotInInitramfs => "Plymouth initramfs'te değil (mkinitcpio HOOKS'ta plymouth yok): açılışın başında görünmez".into(),
        Note::PlymouthNoSplash => "çekirdek komut satırında 'splash' yok: açılış ekranı gösterilmez".into(),
        Note::PlymouthManualRebuild { command } if command.is_empty() => "initramfs yeniden kurulunca görünür; Nuxut bu sistemde kendisi kuramıyor".into(),
        Note::PlymouthManualRebuild { command } => format!("initramfs ({command}) yeniden kurulunca görünür; Nuxut bu sistemde kendisi kuramıyor"),
        Note::GrubTextTerminal => "GRUB metin terminalinde (GRUB_TERMINAL_OUTPUT=console): tema görünmez".into(),
        Note::GrubHandWritten => "grub.cfg elle yazılmış (grub-mkconfig üretimi değil): dokunulmaz".into(),
        Note::GrubCfgUnreadable => "grub.cfg okunamadı: yönetici adımında denetlenecek".into(),
        Note::GrubNoDropins => "grub-mkconfig /etc/default/grub.d damlalarını okumuyor (eski GRUB): dokunulmaz".into(),
        Note::GrubOverridden { path } => format!("{path} GRUB_THEME'i eziyor: dokunulmaz"),
        Note::LimineAmbiguous { paths } => format!("birden çok limine.conf ({}): hangisinin okunduğu bilinmiyor, dokunulmaz", paths.join(", ")),
        Note::LimineUnreadable => "limine.conf okunamadı (ESP izinleri): yönetici adımında denetlenecek".into(),
        Note::LimineUnsupported { reason } => reason.clone(),
        Note::LimineEnrolled { binary } => format!("{binary} içinde kayıtlı yapılandırma sağlama toplamı var (limine enroll-config): dokunmak açılışı bozar"),
    }
}

fn state_text(s: State) -> &'static str {
    match s {
        State::Ready => "uygulanabilir",
        State::Applied => "uygulanmış (yeniden uygulama renkleri tazeler)",
        State::Absent => "kurulu değil",
        State::Blocked => "dokunulmayacak",
    }
}

/// İnsan okur plan metni (install.sh).
pub fn plan_text(st: &Status) -> String {
    let mut s = String::new();
    for tp in &st.plan.targets {
        s.push_str(&format!("{}: {}\n", tp.target.name(), state_text(tp.state)));
        for n in &tp.notes {
            s.push_str(&format!("  ! {}\n", note_text(n)));
        }
        if matches!(tp.state, State::Ready | State::Applied) {
            for c in &tp.changes {
                let k = match c.kind {
                    ChangeKind::Create => "kurulur ",
                    ChangeKind::Modify => "değişir (önce yedeklenir)",
                    ChangeKind::Run => "çalışır ",
                };
                s.push_str(&format!("  {k}  {}\n", c.what));
            }
        }
    }
    if st.plan.systemd_boot {
        s.push_str("systemd-boot: temalanamaz (yalnız metin menüsü; Plymouth teması yine uygulanır)\n");
    }
    s.push_str(&format!("ölçek: {}×  dil: {}  yardımcı: {:?} ({})\n", st.scale, st.lang, st.helper, st.helper_path));
    s
}

fn event_text(e: &Event) -> Option<String> {
    Some(match e {
        Event::Plan { .. } => return None,
        Event::Step { target, step, state } => format!("[{}] {step}: {}", target.name(), if matches!(state, nuxut_system_helper::events::StepState::Start) { "başladı" } else { "tamam" }),
        Event::Log { source, line } => format!("    {source}: {line}"),
        Event::Warn { target, message } => format!("[{}] uyarı: {message}", target.map_or("-", |t| t.name())),
        Event::Target { target, outcome, message } => {
            let o = match outcome {
                Outcome::Applied => "uygulandı",
                Outcome::Reverted => "geri alındı",
                Outcome::RolledBack => "UYGULANAMADI, değişiklikler geri alındı",
                Outcome::Skipped => "atlandı",
                Outcome::Inconsistent => "HATA: geri alma da başarısız — yedekler yerinde, elle bakın",
            };
            match message {
                Some(m) => format!("[{}] {o}: {m}", target.name()),
                None => format!("[{}] {o}", target.name()),
            }
        }
        Event::LoginScreen { .. } => return None,
        Event::LoginStep { step, state } => format!("[giriş ekranı] {step}: {}", if matches!(state, nuxut_system_helper::events::StepState::Start) { "başladı" } else { "tamam" }),
        Event::LoginResult { outcome, message } => {
            let o = match outcome {
                Outcome::Applied => "uygulandı (sonraki açılışta)",
                Outcome::Reverted => "geri alındı (sonraki açılışta)",
                Outcome::RolledBack => "UYGULANAMADI, değişiklikler geri alındı",
                Outcome::Skipped => "atlandı",
                Outcome::Inconsistent => "HATA: geri alma da başarısız — kayıt /var/lib/nuxut/greeter",
            };
            match message {
                Some(m) => format!("[giriş ekranı] {o}: {m}"),
                None => format!("[giriş ekranı] {o}"),
            }
        }
        Event::Done { ok, reboot_hint } => format!(
            "{}{}",
            if *ok { "bitti" } else { "bitti (hatalarla)" },
            if *reboot_hint { " — görmek için yeniden başlatın" } else { "" }
        ),
    })
}

/// Hazırlanmış kök dosyalarını (`system_bundle`) tek yönetici onayıyla kurar. Çıkış: 0 kuruldu,
/// 2 hazırlık yok/kurulamadı, 3 onay verilmedi.
pub(crate) fn install_helper() -> ExitCode {
    let Some(stage) = system_bundle::stage_dir().filter(|s| system_bundle::stage_state(s) == Stage::Ready) else {
        eprintln!("nuxut-shell system-theme: kurulacak sistem dosyaları hazırlanmamış (paketli kurulumda paket kurar)");
        return ExitCode::from(2);
    };
    let dir = match work_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("nuxut-shell system-theme: geçici dizin: {e}");
            return ExitCode::from(2);
        }
    };
    let archive = dir.path().join("system.tar");
    let mtime = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&archive)
        .map_err(system_bundle::BundleError::from)
        .and_then(|f| {
            let mut w = std::io::BufWriter::new(f);
            system_bundle::write_archive(&stage, &mut w, mtime)?;
            w.flush()?;
            Ok(())
        });
    if let Err(e) = written {
        eprintln!("nuxut-shell system-theme: {e}");
        return ExitCode::from(2);
    }
    let status = Command::new("/usr/bin/pkexec").args(system_bundle::extract_args(&archive)).stdin(Stdio::null()).stdout(Stdio::null()).status();
    match status.map(|s| s.code()) {
        Ok(Some(0)) if helper_state(Path::new(nuxut_system_helper::INSTALL_PATH)) == HelperState::Installed => ExitCode::SUCCESS,
        Ok(Some(126 | 127)) => {
            eprintln!("nuxut-shell system-theme: yönetici onayı verilmedi; hiçbir şey değişmedi");
            ExitCode::from(3)
        }
        other => {
            eprintln!("nuxut-shell system-theme: sistem dosyaları kurulamadı ({other:?})");
            ExitCode::from(2)
        }
    }
}

/// Uygulamadan önce: yardımcı yoksa ya da hazırlıktakinden eskiyse ve hazırlık varsa önce kurar.
/// Hazırlık yoksa (paketli kurulum) dokunmaz; kurulu yardımcının durumu `run_helper`'da denetlenir.
pub(crate) fn ensure_helper() -> Result<(), ExitCode> {
    let Some(stage) = system_bundle::stage_dir().filter(|s| system_bundle::stage_state(s) == Stage::Ready) else { return Ok(()) };
    let missing = helper_state(Path::new(nuxut_system_helper::INSTALL_PATH)) == HelperState::Missing;
    if !missing && !system_bundle::update_available(&stage, Path::new("/")) {
        return Ok(());
    }
    match install_helper() {
        c if c == ExitCode::SUCCESS => Ok(()),
        c => Err(c),
    }
}

/// pkexec ile yardımcıyı çalıştırır; olayları `json` ise olduğu gibi, değilse metin olarak yazar.
pub(crate) fn run_helper(args: &[String], json: bool) -> ExitCode {
    match helper_state(Path::new(nuxut_system_helper::INSTALL_PATH)) {
        HelperState::Installed => {}
        other => {
            eprintln!("nuxut-shell system-theme: yardımcı kullanılamıyor ({other:?}): {}", nuxut_system_helper::INSTALL_PATH);
            return ExitCode::from(2);
        }
    }
    let child = Command::new("/usr/bin/pkexec")
        .arg(nuxut_system_helper::INSTALL_PATH)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nuxut-shell system-theme: yönetici onayı istenemedi: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(out) = child.stdout.take() {
        let mut stdout = std::io::stdout();
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if json {
                let _ = writeln!(stdout, "{line}");
            } else if let Ok(e) = serde_json::from_str::<Event>(&line)
                && let Some(t) = event_text(&e)
            {
                let _ = writeln!(stdout, "{t}");
            }
            let _ = stdout.flush();
        }
    }
    match child.wait().map(|s| s.code()) {
        Ok(Some(0)) => ExitCode::SUCCESS,
        // pkexec: 126 = kimlik doğrulama reddedildi/iptal, 127 = yetkisiz.
        Ok(Some(126 | 127)) => {
            eprintln!("nuxut-shell system-theme: yönetici onayı verilmedi; hiçbir şey değişmedi");
            ExitCode::from(3)
        }
        _ => ExitCode::FAILURE,
    }
}

/// Önizleme fontları için grub-mkfont (kullanıcı yetkisiyle, geçici dizine).
fn make_pf2(src: &str, family: &str, size: u32, dir: &Path) -> Option<pf2::Font> {
    let out = dir.join(format!("{}-{size}.pf2", family.replace(' ', "-").to_lowercase()));
    let ok = Command::new("/usr/bin/grub-mkfont")
        .args(["-s", &size.to_string(), "-n", family, "-o"])
        .arg(&out)
        .arg(Path::new("/").join(src))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return None;
    }
    pf2::Font::parse(std::fs::read(&out).ok()?).ok()
}

/// Önizleme PNG'lerini `out`'a yazar. Dönen: yazılan dosya adları.
pub fn render_previews(theme: &nuxut::Theme, out: &Path, w: u32, h: u32) -> Result<Vec<String>, String> {
    let probe = StdProbe::system();
    let plan = plan::build(&detect::system(&probe, &cmdline()));
    let req = request(theme, None, &plan);
    let fonts = fontsel::choose_fonts(&probe);
    let opts = ThemeOptions {
        colors: req.colors,
        scale: req.scale,
        lang: req.lang,
        family: fonts.as_ref().map_or("Sans", |f| f.0.family).to_owned(),
        mono_family: fonts.as_ref().map_or("monospace", |f| f.1.family).to_owned(), // font-istisnası: önyükleme teması fontu sistem dosyasından (fontsel); bulunamazsa fontconfig genel adı
    };
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let work_dir = work_dir()?;
    let work = work_dir.path().to_path_buf();
    let tr = req.lang == Lang::Tr;
    let mut written = Vec::new();
    let mut save = |name: &str, img: &raster::Image| -> Result<(), String> {
        let png = raster::encode_png(img).map_err(|e| e.to_string())?;
        std::fs::write(out.join(name), png).map_err(|e| format!("{name}: {e}"))?;
        written.push(name.to_owned());
        Ok(())
    };

    let ui_font = |size: u32| fonts.as_ref().and_then(|(ui, _)| make_pf2(ui.path, grub::FAMILY, size, &work));
    let prompt_font = ui_font(16);
    let ply = plymouth::generate(&opts).map_err(|e| e.to_string())?;
    save("plymouth.png", &preview::plymouth(&ply, &opts.colors, w, h, PlymouthScene::Boot { progress_permille: 420 }, "", None))?;
    let prompt = if tr { "Disk parolası (cryptroot)" } else { "Please enter passphrase for disk cryptroot" };
    save("plymouth-password.png", &preview::plymouth(&ply, &opts.colors, w, h, PlymouthScene::Password { bullets: 7 }, prompt, prompt_font.as_ref()))?;

    let g = grub::generate(&opts).map_err(|e| e.to_string())?;
    let mut gf = GrubFonts::default();
    if let Some((ui, mono)) = &fonts {
        for spec in grub::fonts(opts.scale) {
            let src = if spec.mono { mono.path } else { ui.path };
            if let Some(f) = make_pf2(src, spec.family, spec.size, &work) {
                gf.fonts.push((spec.grub_name(), f));
            }
        }
    }
    let entries = ["Arch Linux", if tr { "Arch Linux için gelişmiş seçenekler" } else { "Advanced options for Arch Linux" }, "UEFI Firmware Settings"];
    // Temanın kendi metinleri (GRUB `%d`'yi saniyeyle değiştirir).
    let timeout = grub::timeout_text(opts.lang).replace("%d", "4");
    let timeout = timeout.as_str();
    let help = grub::keymap_text(opts.lang);
    save("grub.png", &preview::grub(&g, &opts.colors, opts.scale, &gf, w, h, preview::GrubText { entries: &entries, timeout, help }))?;

    let mono16 = fonts.as_ref().and_then(|(_, m)| make_pf2(m.path, grub::MONO_FAMILY, 16 * opts.scale, &work));
    let lim_help = "ARROWS Select    ENTER Boot    E Edit    C Console";
    save("limine.png", &preview::limine(&opts.colors, w, h, mono16.as_ref(), &["Arch Linux", "Arch Linux (fallback)", "Windows Boot Manager"], lim_help))?;
    drop(work_dir);
    Ok(written)
}

/// Önizleme fontlarının geçici dizini (güvenlik incelemesi "insecure-temp-file", 2026-10-17):
/// `$XDG_RUNTIME_DIR` (yoksa sistem geçici dizini) altında rastgele adla, `mkdir` ile yalnız
/// yoksa yaratılır (başkasının önceden kurduğu dizin ya da sembolik bağ kullanılmaz), izni 0700,
/// düşünce silinir. Önceki sürüm tahmin edilebilir `nuxut-system-theme-<pid>` adını
/// `create_dir_all` ile (var olanı da kabul ederek) kullanıyordu. Dizin yalnız kullanıcı
/// yetkisindeki önizlemede kullanılır; kök yardımcıya dosya ya da yol geçmez (yalnız argüman).
fn work_dir() -> Result<tempfile::TempDir, String> {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(std::env::temp_dir);
    work_dir_in(&base)
}

fn work_dir_in(base: &Path) -> Result<tempfile::TempDir, String> {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::Builder::new()
        .prefix("nuxut-system-theme-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in(base)
        .map_err(|e| format!("{}: {e}", base.display()))?;
    Ok(d)
}

fn parse_targets(v: &str) -> Result<Vec<Target>, String> {
    v.split(',').map(|n| Target::parse(n).ok_or_else(|| format!("bilinmeyen hedef: {n}"))).collect()
}

fn parse_size(v: &str) -> Option<(u32, u32)> {
    let (w, h) = v.split_once('x')?;
    let (w, h) = (w.parse().ok()?, h.parse().ok()?);
    ((320..=raster::MAX_SIDE).contains(&w) && (240..=raster::MAX_SIDE).contains(&h)).then_some((w, h))
}

pub const USAGE: &str = "kullanım: nuxut-shell system-theme status [--json] | install-helper | preview --out <dizin> [--size GxY] | apply [--json] [--targets plymouth,grub,limine] [--dry-run] | revert [--json] [--targets …]";

/// `nuxut-shell system-theme …`.
pub fn run_cli(args: &[String]) -> ExitCode {
    let Some(verb) = args.first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let mut json = false;
    let mut dry = false;
    let mut targets = None;
    let mut out = None;
    let mut size = (1920, 1080);
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match (a.as_str(), it.clone().next()) {
            ("--json", _) => json = true,
            ("--dry-run", _) => dry = true,
            ("--targets", Some(v)) => match parse_targets(v) {
                Ok(t) => {
                    targets = Some(t);
                    it.next();
                }
                Err(e) => {
                    eprintln!("{e}\n{USAGE}");
                    return ExitCode::from(2);
                }
            },
            ("--out", Some(v)) => {
                out = Some(PathBuf::from(v));
                it.next();
            }
            ("--size", Some(v)) => match parse_size(v) {
                Some(s) => {
                    size = s;
                    it.next();
                }
                None => {
                    eprintln!("geçersiz boyut: {v}\n{USAGE}");
                    return ExitCode::from(2);
                }
            },
            _ => {
                eprintln!("beklenmeyen argüman: {a}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let theme = match nuxut::Theme::load() {
        Ok(l) => l.value,
        Err(e) => {
            eprintln!("framework.toml okunamadı ({e}); varsayılan tema kullanılıyor");
            nuxut::Theme::default()
        }
    };
    match verb.as_str() {
        "status" => {
            let st = status(&theme);
            if json {
                println!("{}", serde_json::to_string_pretty(&st).unwrap_or_default());
            } else {
                print!("{}", plan_text(&st));
            }
            ExitCode::SUCCESS
        }
        "preview" => {
            let Some(out) = out else {
                eprintln!("--out gerekli\n{USAGE}");
                return ExitCode::from(2);
            };
            match render_previews(&theme, &out, size.0, size.1) {
                Ok(files) => {
                    println!("{}", serde_json::json!({ "dir": out, "files": files }));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("önizleme üretilemedi: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "apply" => {
            let plan = status(&theme).plan;
            let req = request(&theme, targets, &plan);
            if req.targets.is_empty() {
                eprintln!("uygulanacak hedef yok");
                return ExitCode::SUCCESS;
            }
            let argv = nuxut_system_helper::args::apply_args(&req);
            if dry {
                println!("yönetici onayıyla çalışır: {} {}", nuxut_system_helper::INSTALL_PATH, argv.join(" "));
                return ExitCode::SUCCESS;
            }
            if let Err(code) = ensure_helper() {
                return code;
            }
            run_helper(&argv, json)
        }
        "install-helper" => {
            if dry {
                println!("yönetici onayıyla kurulur: {}", system_bundle::FILES.iter().map(|f| f.path).collect::<Vec<_>>().join(" "));
                return ExitCode::SUCCESS;
            }
            install_helper()
        }
        "revert" => {
            let mut argv = vec!["boot-theme".to_owned(), "revert".to_owned()];
            if let Some(t) = targets {
                argv.push("--targets".into());
                argv.push(t.iter().map(|t| t.name()).collect::<Vec<_>>().join(","));
            }
            if dry {
                println!("yönetici onayıyla çalışır: {} {}", nuxut_system_helper::INSTALL_PATH, argv.join(" "));
                return ExitCode::SUCCESS;
            }
            run_helper(&argv, json)
        }
        other => {
            eprintln!("bilinmeyen eylem: {other}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Güvenlik incelemesi "insecure-temp-file" (2026-10-17): geçici dizin tahmin edilebilir
    /// adı kullanmaz, başkasının önceden kurduğu dizine ya da sembolik bağa girmez, 0700'dür ve
    /// bırakılınca silinir.
    #[test]
    fn work_dir_is_private_unique_and_removed() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let base = tempfile::tempdir().unwrap();
        // Saldırgan eski adı ve bir sembolik bağı önceden kurar.
        let old = base.path().join(format!("nuxut-system-theme-{}", std::process::id()));
        std::fs::create_dir(&old).unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o777)).unwrap();
        std::os::unix::fs::symlink("/", base.path().join("nuxut-system-theme-link")).unwrap();
        let a = work_dir_in(base.path()).unwrap();
        let b = work_dir_in(base.path()).unwrap();
        assert_ne!(a.path(), b.path());
        for d in [a.path(), b.path()] {
            assert_ne!(d, old.as_path(), "eski tahmin edilebilir ad kullanılmaz");
            let m = std::fs::symlink_metadata(d).unwrap();
            assert!(m.is_dir() && !m.file_type().is_symlink());
            assert_eq!(m.mode() & 0o777, 0o700);
            assert!(d.file_name().unwrap().to_string_lossy().len() > "nuxut-system-theme-".len() + 4, "rastgele son ek");
        }
        let path = a.path().to_path_buf();
        drop(a);
        assert!(!path.exists(), "bırakılınca silinir");
    }

    #[test]
    fn scale_from_monitors() {
        assert_eq!(boot_scale(&[]), 1);
        assert_eq!(boot_scale(&[(1080, 1.0)]), 1);
        assert_eq!(boot_scale(&[(1440, 1.25)]), 1);
        assert_eq!(boot_scale(&[(1600, 1.6)]), 2, "yüksek DPI dizüstü: kullanıcı ölçeği");
        assert_eq!(boot_scale(&[(1080, 1.0), (2160, 1.5)]), 2);
        assert_eq!(boot_scale(&[(2880, 2.0)]), 3);
        assert_eq!(boot_scale(&[(4320, 4.0)]), 3, "üst sınır");
    }

    #[test]
    fn colors_from_default_theme_are_dark() {
        let theme = nuxut::Theme::default();
        let c = boot_colors(&theme, None);
        let lum = |c: Rgb| 0.2126 * f64::from(c.0) + 0.7152 * f64::from(c.1) + 0.0722 * f64::from(c.2);
        assert!(lum(c.surface) < 60.0, "koyu zemin: {}", c.surface);
        assert!(lum(c.on_surface) > 180.0);
        assert_eq!(c.primary, rgb(theme.colors.primary), "koyu temada Nuxut'un kendi vurgusu");
        // Yardımcı argümanlarına dönüşüm geri okunabilir.
        let args = c.to_args();
        assert_eq!(BootColors::from_args(args.iter().map(String::as_str)).unwrap(), c);
    }

    #[test]
    fn helper_state_requires_root_ownership() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("helper");
        assert_eq!(helper_state(&p), HelperState::Missing);
        std::fs::write(&p, "").unwrap();
        // Testi çalıştıran kök değil: dosya bizim → güvensiz sayılır.
        assert_eq!(helper_state(&p), HelperState::Unsafe);
    }

    #[test]
    fn status_text_lists_changes_and_notes() {
        let info = detect::system(
            &detect::MapProbe::default()
                .with(detect::GRUB_MKCONFIG, "/default/grub.d/*.cfg")
                .with("boot/grub/grub.cfg", "# It is automatically generated by grub-mkconfig\nmenuentry 'a' {\n}\n"),
            "quiet",
        );
        let st = Status {
            version: STATUS_VERSION,
            actionable: plan::build(&info).actionable(),
            plan: plan::build(&info),
            helper: HelperState::Missing,
            helper_path: "/x",
            helper_installable: false,
            helper_update: false,
            scale: 1,
            lang: "tr",
            colors: vec![],
        };
        let t = plan_text(&st);
        assert!(t.contains("grub: uygulanabilir"), "{t}");
        assert!(t.contains("/etc/default/grub.d/90-nuxut-theme.cfg"));
        assert!(t.contains("plymouth: kurulu değil"));
        let json = serde_json::to_value(&st).unwrap();
        assert_eq!(json["actionable"], serde_json::json!(["grub"]));
        assert_eq!(json["plan"]["targets"][1]["state"], "ready");
    }

    #[test]
    fn size_parsing() {
        assert_eq!(parse_size("1920x1080"), Some((1920, 1080)));
        assert_eq!(parse_size("10x10"), None);
        assert_eq!(parse_size("99999x1080"), None);
        assert_eq!(parse_size("axb"), None);
    }
}
