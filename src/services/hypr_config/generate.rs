//! Ayarlar → `nuxut.conf` metni ve canlı uygulama için `keyword` listesi (tek kaynak).
//!
//! Saf: girdi tema/shell.toml'dan türetilmiş değerlerdir ([`LookGroup::from_theme`] vb.),
//! çıktı metin + anahtar/değer çiftleri. `None` grup = o grup eşitlenmiyor (satırı yok).
//! Üretilen her değer [`safe_value`] ile denetlenir: `;` (batch ayırıcısı), `#` (yorum),
//! satır sonu ve denetim karakteri yoktur.

use std::fmt::Write as _;

use nuxut::Color;
use nuxut::Theme;
use nuxut::motion::{CubicBezier, Motion};
use nuxut::theme::contrast::NON_TEXT_MIN;
use nuxut::ui::style::legible;

use super::input::InputSettings;


/// Nuxut'un yönetebileceği anahtarlar (kullanıcının zincirinde ezilen değerleri bulmak için).
/// Statik üst küme: tarama bunların yerini bulur, override ise yalnızca bu turda gerçekten
/// üretilen anahtarlar için raporlanır (`Syncer::run`, `Generated::keywords`) — `[input]`'ta
/// yazılmamış anahtar hiç üretilmediğinden override sayılmaz.
pub const MANAGED_KEYS: &[&str] = &[
    "input:kb_layout",
    "input:kb_variant",
    "input:kb_options",
    "input:repeat_rate",
    "input:repeat_delay",
    "input:numlock_by_default",
    "input:sensitivity",
    "input:accel_profile",
    "input:natural_scroll",
    "input:touchpad:natural_scroll",
    "input:touchpad:tap-to-click",
    "input:touchpad:disable_while_typing",
    "input:touchpad:scroll_factor",
    "input:scroll_method",
    "decoration:rounding",
    "decoration:shadow:enabled",
    "decoration:shadow:range",
    "decoration:shadow:offset",
    "decoration:shadow:color",
    "decoration:shadow:render_power",
    "decoration:blur:enabled",
    "decoration:blur:size",
    "decoration:blur:passes",
    "animations:enabled",
    "general:gaps_in",
    "general:gaps_out",
    "general:border_size",
    "general:col.active_border",
    "general:col.inactive_border",
    "misc:disable_hyprland_logo",
    "misc:force_default_wallpaper",
];

/// Köşe + gölge + bulanıklık.
#[derive(Debug, Clone, PartialEq)]
pub struct LookGroup {
    pub rounding: f32,
    pub shadow: Option<ShadowLook>,
    pub blur: Option<BlurLook>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowLook {
    pub range: f32,
    pub offset_y: f32,
    /// Opaklık uygulanmış renk.
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlurLook {
    pub size: u32,
    pub passes: u32,
}

impl LookGroup {
    /// Temadan: pencere köşesi = panel köşesi (`shape.radius_large`), gölge temanınki.
    pub fn from_theme(theme: &Theme, blur: Option<BlurLook>) -> Self {
        let s = theme.shadow;
        Self {
            rounding: theme.shape.radius_large,
            shadow: s.enabled.then_some(ShadowLook { range: s.blur, offset_y: s.offset_y, color: s.color }),
            blur,
        }
    }
}

/// Hareket rolleri (tema hareketi: `enabled` ve `speed` uygulanmış).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionGroup {
    pub enabled: bool,
    /// Kullanıcının hız çarpanı (rollere zaten uygulanmış; bant hesabında arındırmak için).
    pub speed: f32,
    pub spatial: Motion,
    pub spatial_fast: Motion,
    pub effects: Motion,
}

impl MotionGroup {
    pub fn from_theme(theme: &Theme) -> Self {
        let m = &theme.motion;
        Self { enabled: m.enabled, speed: m.speed, spatial: m.spatial, spatial_fast: m.spatial_fast, effects: m.effects }
    }
}

/// Pencere aralığı + kenarlık.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowsGroup {
    pub gaps_in: u32,
    pub gaps_out: u32,
    pub border: u32,
    pub active_border: Color,
    pub inactive_border: Color,
}

/// Pasif kenarlığın standart opaklığı (`outline` × bu).
const INACTIVE_BORDER_ALPHA: f32 = 0.4;

impl WindowsGroup {
    /// Kenarlık renkleri temadan: odaklı = `primary`, diğerleri = `outline` × 0.4.
    ///
    /// Yüksek karşıtlıkta (`theme.contrast`) iki kenarlık da ölçüm zemini [`border_backdrop`]'a
    /// karşı en az 3:1'e (WCAG 1.4.11, metin dışı) en az değişimle itilir
    /// ([`nuxut::ui::style::legible`]: önce opaklık, yetmezse renk uca). `primary` yüksek
    /// paletinde zaten eşikte olduğundan odaklı kenarlık pratikte değişmez. Standart kipte
    /// çıktı bayt bayt eskisidir (ADR-0004 §18.6).
    pub fn from_theme(theme: &Theme, gaps_in: u32, gaps_out: u32, border: u32) -> Self {
        let bg = [border_backdrop(theme)];
        let shown = |c: Color| legible(theme, c, &bg, NON_TEXT_MIN);
        Self {
            gaps_in,
            gaps_out,
            border,
            active_border: shown(theme.colors.primary),
            inactive_border: shown(theme.colors.outline.multiply_alpha(INACTIVE_BORDER_ALPHA)),
        }
    }
}

/// Kenarlık karşıtlığının ölçüldüğü zemin: tema `surface`'i. Gerçek zemin bilinmez (kenarlığın
/// bir yanı pencere içeriği, öbür yanı boşluktaki duvar kâğıdı ya da komşu pencere); birleşik
/// görünüm açıkken uygulamalar pencere zeminini temanın `surface`'inden alır, bu yüzden
/// kenarlığın her zaman bitiştiği yüzeyin en iyi temsilcisi odur (ADR-0004 §18.6).
pub fn border_backdrop(theme: &Theme) -> Color {
    theme.colors.surface
}

/// Üretici girdisi.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateInput {
    /// `true` = oturum başında kabuğu başlatan kanca yazılır ([`start_command`]; kullanıcının
    /// ayarında başka başlatıcı yoksa — ADR-0003 Karar 5).
    pub launch: bool,
    /// Kurulu ikilinin yolu (bind'lerde ve başlatmada). Güvensiz karakter içerirse yalın
    /// `nuxut-shell` (PATH) kullanılır ([`safe_exe`]).
    pub shell_exe: String,
    pub look: Option<LookGroup>,
    pub motion: Option<MotionGroup>,
    pub windows: Option<WindowsGroup>,
    /// Masaüstü zeminini shell çiziyor (duvar kağıdı ayarlı): Hyprland'in kendi varsayılan
    /// duvar kağıdı ve logosu kapatılır (`misc:disable_hyprland_logo`,
    /// `misc:force_default_wallpaper = 0`); yüzey gelmeden önceki ilk karelerde ve olası
    /// bir boşlukta bunlar görünmesin.
    pub hide_defaults: bool,
    /// Ekranlar'ın ürettiği monitör dosyası varsa onun gösterim yolu (ADR-0005 §6.2):
    /// hyprlang'da `source = <yol>`, Lua'da `pcall(require, "nuxut.displays")` eklenir.
    /// Dosya yoksa `None` (hyprlang eksik `source`'u hata sayar).
    pub displays: Option<String>,
    /// Ortam değişkenleri (birleşik görünümün imleç değişkenleri, ADR-0004 §5.4): hyprlang `env =`,
    /// Lua `hl.env`. Canlı uygulanmaz (imleç `setcursor` ile canlı gelir); Hyprland'in bundan sonra
    /// okuduğu ayarla başlattığı süreçler alır. Güvensiz çift atlanır ([`safe_env`]).
    pub env: Vec<(String, String)>,
    /// Doğrulanmış `[input]` (`sync_input` kapalıysa `None`). Yalnızca yazılı anahtarlar satır üretir.
    pub input: Option<InputSettings>,
}

/// Ortam çifti üretilen dosyaya güvenle girer mi (ad `[A-Z0-9_]`, değer tek satır, `;`/`#`/`"`/`\`
/// yok, sınırlı)?
pub fn safe_env(key: &str, value: &str) -> bool {
    let key_ok = !key.is_empty() && key.len() <= 64 && key.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
    key_ok && value.len() <= 256 && safe_value(value) && !value.contains(['"', '\\'])
}

/// Üretici çıktısı.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Generated {
    pub text: String,
    /// hyprlang: canlı uygulama için `keyword <anahtar> <değer>` çiftleri (bind'ler hariç:
    /// onlar `reload` ile gelir). Lua'da boş.
    pub keywords: Vec<(String, String)>,
    /// Lua: canlı uygulama için tek satırlık `hyprctl eval` kodu (değerler, eğriler,
    /// animasyonlar; bind'siz). hyprlang'da `None`.
    pub eval: Option<String>,
}

/// systemd kullanıcı biriminin adı (install.sh kurar).
pub const UNIT_NAME: &str = "nuxut-shell.service";

/// Yol kabuk komutuna güvenle girer mi? Değilse yalın `nuxut-shell` (PATH).
pub fn safe_exe(exe: &str) -> &str {
    if !exe.is_empty() && exe.starts_with('/') && exe.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._+-".contains(&b)) {
        exe
    } else {
        "nuxut-shell"
    }
}

/// Her zaman PATH'te olan dizinler: kabuk buradaysa PATH satırı yazılmaz.
const SYSTEM_BIN_DIRS: [&str; 5] = ["/usr/bin", "/bin", "/usr/local/bin", "/usr/sbin", "/sbin"];

/// Oturumun PATH'ine eklenecek kabuk dizini (ör. `~/.local/bin`): terminaller, betikler ve
/// `nuxut-shell msg …` ikiliyi adla bulsun. Kullanıcı isteği 2026-10-16: kabuk başlangıç
/// dosyalarına (.bash_profile, .profile, fish) yazılmaz, elle adım yok; ayarı Hyprland ortamına
/// Nuxut'un katmanı koyar. Sistem dizini ya da güvensiz yolda `None`.
pub fn session_path_dir(exe: &str) -> Option<&str> {
    let (dir, _) = safe_exe(exe).rsplit_once('/')?;
    (!dir.is_empty() && !SYSTEM_BIN_DIRS.contains(&dir)).then_some(dir)
}

/// Hyprland ortamı her yeniden okumada yeniden kurulur (`handleEnv`, 0.56.2) ve değerdeki `$PATH`
/// o anki ortamdan açılır (hyprlang): koşulsuz satır her yeniden yüklemede dizini bir kez daha
/// eklerdi. Blok oturumda yalnızca bir kez çalışır (`# hyprlang if !…`, işaret sonda); `envd`
/// PATH'i systemd kullanıcı yöneticisine ve D-Bus etkinleştirme ortamına da aktarır.
pub const SESSION_PATH_MARKER: &str = "NUXUT_PATH_ADDED";

/// Oturum başı kancası: ortamı systemd'ye aktar, birimi başlat; systemd kullanıcı oturumu ya da
/// birim yoksa doğrudan başlat (ADR-0003 Karar 5). Hyprland'in kendi içe aktarması eşzamansız
/// olduğu için burada da yapılır.
pub fn start_command(exe: &str) -> String {
    format!(
        "systemctl --user import-environment PATH WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP && systemctl --user start {UNIT_NAME} || exec {}",
        safe_exe(exe)
    )
}

/// Oturum sonu kancası (en iyi çaba).
pub fn stop_command() -> String {
    format!("systemctl --user stop {UNIT_NAME}")
}

/// polkit ajanının systemd kullanıcı birimi (install.sh kurar; `polkit/dist/`).
pub const POLKIT_UNIT_NAME: &str = "nuxut-polkit.service";

/// polkit ajanı oturum başı kancası. Yalnızca birim **etkinse** (install.sh başka ajan yokken
/// etkinleştirir; kullanıcı `systemctl --user enable` ile seçer) ortam aktarılıp başlatılır —
/// kullanıcının kendi ajanı (hyprpolkitagent…) seçiliyse hiçbir şey yapılmaz. Doğrudan başlatma
/// yedeği yok: systemd'siz oturumda Nuxut polkit ajanı başlamaz, kullanıcının zinciri geçerli kalır.
/// Diyalog Wayland'e polkit ajanının ortamından bağlandığı için içe aktarma başlatmadan önce yapılır.
pub fn polkit_start_command() -> String {
    format!(
        "systemctl --user is-enabled --quiet {POLKIT_UNIT_NAME} && systemctl --user import-environment WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP && systemctl --user start {POLKIT_UNIT_NAME}"
    )
}

/// polkit ajanı oturum sonu kancası (bir sonraki oturumda eski `WAYLAND_DISPLAY` kalmasın).
pub fn polkit_stop_command() -> String {
    format!("systemctl --user stop {POLKIT_UNIT_NAME}")
}

/// Değer `keyword` batch'ine ve conf satırına güvenle girer mi?
pub fn safe_value(v: &str) -> bool {
    !v.chars().any(|c| matches!(c, ';' | '#') || c.is_control())
}

/// `rgba(rrggbbaa)`.
pub fn hypr_color(c: Color) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("rgba({:02x}{:02x}{:02x}{:02x})", b(c.r), b(c.g), b(c.b), b(c.a))
}

fn num(v: f32) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let r = (v * 1000.0).round() / 1000.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

fn px(v: f32) -> String {
    let v = if v.is_finite() { v.clamp(0.0, 1000.0) } else { 0.0 };
    format!("{}", v.round() as i64)
}

/// Bir rolün Hyprland karşılığı: (eğri, süre ms); anında ise `None`.
pub fn role_curve(m: Motion) -> Option<(CubicBezier, f32)> {
    match m {
        Motion::Spring(s) => {
            if s.is_instant() || !s.response.is_finite() {
                return None;
            }
            let d = s.damping.max(0.05);
            let factor = if d < 1.0 { 1.6 - 0.6 * d } else { d };
            let ms = s.response * 1000.0 * factor;
            let bezier = if d < 1.0 {
                let o = (-std::f32::consts::PI * d / (1.0 - d * d).sqrt()).exp().min(0.5);
                CubicBezier::new(0.3, 1.0 + o, 0.4, 1.0)
            } else {
                CubicBezier::DECELERATE
            };
            Some((bezier, ms))
        }
        Motion::Curve(c) => (!c.is_instant() && c.duration.is_finite()).then_some((c.easing, c.duration * 1000.0)),
    }
}

/// Masaüstü animasyon eğrileri (ADR-0006 §4): framework'ün Material 3 eğrileri; hiçbiri taşmaz
/// (y ∈ [0, 1]). Giren öğe vurgulu yavaşlama, çıkan vurgulu hızlanma, yer değiştiren standart.
pub const CURVES: [(&str, CubicBezier); 3] =
    [("nuxut_decel", CubicBezier::DECELERATE), ("nuxut_accel", CubicBezier::ACCELERATE), ("nuxut_std", CubicBezier::STANDARD)];

/// Bir animasyon yaprağı: süre `None` = kapalı (rolü anında).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anim {
    pub leaf: &'static str,
    pub curve: &'static str,
    pub ms: Option<f32>,
    /// Hyprland stili (`popin 92%`, `slide`…); boş = yok.
    pub style: &'static str,
}

/// Süre bantları (ms, hız çarpanı 1'de): pencere girişi/taşıma, çıkış ve soluklaşma, çalışma alanı.
pub const ENTER_MS: (f32, f32) = (180.0, 250.0);
pub const EXIT_MS: (f32, f32) = (120.0, 200.0);
pub const WORKSPACE_MS: (f32, f32) = (200.0, 300.0);

/// Masaüstü animasyon planı (ADR-0006 §4): rolün oturma süresi (`role_curve`, hız uygulanmış)
/// hız çarpanından arındırılır, bandına kıstırılır, sonra kullanıcının hızı yeniden uygulanır.
/// `spatial_fast` → pencere girişi/taşıma ve çalışma alanı; `effects` → çıkış, soluklaşma,
/// kenarlık, katmanlar. Hareket kapalıysa (azaltılmış hareket) `None`: `animations:enabled = false`.
pub fn anim_plan(m: &MotionGroup) -> Option<Vec<Anim>> {
    if !m.enabled {
        return None;
    }
    let speed = if m.speed.is_finite() && m.speed > 0.0 { m.speed } else { 1.0 };
    let dur = |role: Motion, (lo, hi): (f32, f32)| role_curve(role).map(|(_, ms)| (ms * speed).clamp(lo, hi) / speed);
    let enter = dur(m.spatial_fast, ENTER_MS);
    let exit = dur(m.effects, EXIT_MS);
    let ws = dur(m.spatial_fast, WORKSPACE_MS);
    if enter.is_none() && exit.is_none() {
        return None;
    }
    let a = |leaf, curve, ms, style| Anim { leaf, curve, ms, style };
    // Ebeveyn önce (Hyprland ağacında alt yapraklar ebeveynden miras alır), sonra ayrıntı.
    Some(vec![
        a("fade", "nuxut_std", exit, ""),
        a("border", "nuxut_std", exit, ""),
        a("windowsIn", "nuxut_decel", enter, "popin 92%"),
        a("windowsOut", "nuxut_accel", exit, "popin 92%"),
        a("windowsMove", "nuxut_std", enter, ""),
        a("fadeIn", "nuxut_decel", enter, ""),
        a("fadeOut", "nuxut_accel", exit, ""),
        a("layersIn", "nuxut_decel", exit, "fade"),
        a("layersOut", "nuxut_accel", exit, "fade"),
        a("fadeLayersIn", "nuxut_decel", exit, ""),
        a("fadeLayersOut", "nuxut_accel", exit, ""),
        a("workspaces", "nuxut_decel", ws, "slide"),
        a("specialWorkspace", "nuxut_decel", ws, "slidefadevert 15%"),
        a("zoomFactor", "nuxut_std", exit, ""),
    ])
}

/// Hyprland hızı: desisaniye, bir ondalık, [0.5, 30].
fn deciseconds(ms: f32) -> String {
    num(((ms / 100.0) * 10.0).round().clamp(5.0, 300.0) / 10.0)
}

/// Ayarlar → nuxut.conf metni + keyword listesi (hyprlang arka ucu).
pub fn generate_conf(input: &GenerateInput) -> Generated {
    let mut g = Generated::default();
    // i18n: tanı üretilen nuxut.conf başlığı (dosya içeriği; Hyprland ayarı, çevrilmez)
    let mut text = String::from(
        "# Nuxut — nuxut-shell tarafından üretildi; her değişiklikte üzerine yazılır.\n\
         # Elle düzenlemeyin: kendi ayarlarınızı hyprland.conf'a yazın. Nuxut'un bir grubu\n\
         # yönetmesini istemiyorsanız shell.toml [compositor] sync_* anahtarını kapatın.\n",
    );
    let exe = safe_exe(&input.shell_exe);

    if let Some(dir) = session_path_dir(&input.shell_exe) {
        // i18n: tanı üretilen nuxut.conf bölüm yorumu (dosya içeriği)
        text.push_str("\n# Kabuğun dizini oturumun PATH'inde (terminaller, betikler); oturumda bir kez\n");
        let _ = write!(text, "# hyprlang if !{SESSION_PATH_MARKER}\nenvd = PATH,{dir}:$PATH\nenv = {SESSION_PATH_MARKER},1\n# hyprlang endif\n");
    }

    let env: Vec<_> = input.env.iter().filter(|(k, v)| safe_env(k, v)).collect();
    if !env.is_empty() {
        text.push_str("\n# Oturum ortamı (portal) ve imleç ortamı (birleşik görünüm)\n"); // i18n: tanı üretilen nuxut.conf bölüm yorumu (dosya içeriği)
        for (k, v) in env {
            let _ = writeln!(text, "env = {k},{v}");
        }
    }

    if input.launch {
        let _ = write!(text, "\nexec-once = {}\n", start_command(exe));
        let _ = writeln!(text, "exec-once = {}", polkit_start_command());
    }

    let mut kv = |text: &mut String, key: &str, value: String| {
        debug_assert!(safe_value(&value), "{key} = {value}");
        if !safe_value(&value) {
            return;
        }
        let _ = writeln!(text, "{key} = {value}");
        g.keywords.push((key.to_owned(), value));
    };

    if let Some(look) = &input.look {
        text.push_str("\n# Köşeler, gölge, bulanıklık\n"); // i18n: tanı üretilen nuxut.conf bölüm yorumu (dosya içeriği)
        kv(&mut text, "decoration:rounding", px(look.rounding));
        match look.shadow {
            Some(s) => {
                kv(&mut text, "decoration:shadow:enabled", "true".into());
                kv(&mut text, "decoration:shadow:range", px(s.range));
                kv(&mut text, "decoration:shadow:offset", format!("0 {}", px(s.offset_y)));
                kv(&mut text, "decoration:shadow:render_power", "3".into());
                kv(&mut text, "decoration:shadow:color", hypr_color(s.color));
            }
            None => kv(&mut text, "decoration:shadow:enabled", "false".into()),
        }
        match look.blur {
            Some(b) => {
                kv(&mut text, "decoration:blur:enabled", "true".into());
                kv(&mut text, "decoration:blur:size", b.size.clamp(1, 32).to_string());
                kv(&mut text, "decoration:blur:passes", b.passes.clamp(1, 4).to_string());
            }
            None => kv(&mut text, "decoration:blur:enabled", "false".into()),
        }
    }

    if let Some(w) = &input.windows {
        text.push_str("\n# Pencereler\n");
        kv(&mut text, "general:gaps_in", w.gaps_in.min(64).to_string());
        kv(&mut text, "general:gaps_out", w.gaps_out.min(128).to_string());
        kv(&mut text, "general:border_size", w.border.min(16).to_string());
        kv(&mut text, "general:col.active_border", hypr_color(w.active_border));
        kv(&mut text, "general:col.inactive_border", hypr_color(w.inactive_border));
    }

    // Girdi aygıtları: yalnızca shell.toml'da yazılı anahtarlar (gerisi kullanıcının ayarında kalır).
    let input_lines = input.input.as_ref().map(InputSettings::lines).unwrap_or_default();
    if !input_lines.is_empty() {
        text.push_str("\n# Klavye, fare, dokunmatik yüzey (yalnızca Ayarlar'da değiştirilenler)\n"); // i18n: tanı üretilen nuxut.conf bölüm yorumu (dosya içeriği)
        for l in input_lines {
            kv(&mut text, l.key.hypr, l.value.conf());
        }
    }

    if input.hide_defaults {
        text.push_str("\n# Masaüstü zemini (duvar kağıdını Nuxut çizer)\n"); // i18n: tanı üretilen nuxut.conf bölüm yorumu (dosya içeriği)
        kv(&mut text, "misc:disable_hyprland_logo", "true".into());
        kv(&mut text, "misc:force_default_wallpaper", "0".into());
    }

    if let Some(m) = &input.motion {
        text.push_str("\n# Hareket (vurgulu yavaşlama, taşma yok; süreler hareket token'larından)\n"); // i18n: tanı üretilen nuxut.conf bölüm yorumu (dosya içeriği)
        let plan = anim_plan(m);
        kv(&mut text, "animations:enabled", plan.is_some().to_string());
        if let Some(plan) = plan {
            for (name, b) in CURVES {
                kv(&mut text, "bezier", format!("{name}, {}, {}, {}, {}", num(b.x1), num(b.y1), num(b.x2), num(b.y2)));
            }
            for a in &plan {
                let value = match a.ms {
                    Some(ms) if a.style.is_empty() => format!("{}, 1, {}, {}", a.leaf, deciseconds(ms), a.curve),
                    Some(ms) => format!("{}, 1, {}, {}, {}", a.leaf, deciseconds(ms), a.curve, a.style),
                    None => format!("{}, 0", a.leaf),
                };
                kv(&mut text, "animation", value);
            }
        }
    }

    // Ekranlar (ADR-0005 §6.2): ayrı dosya, en sonda (kullanıcının monitör kurallarını ezer).
    if let Some(path) = input.displays.as_deref().filter(|p| safe_value(p) && !p.contains(',')) {
        let _ = write!(text, "\n# Ekranlar (Ayarlar → Ekranlar)\nsource = {path}\n");
    }

    g.text = text;
    g
}

#[cfg(test)]
mod tests {
    use nuxut::motion::{Curve, Spring};

    use super::*;

    fn input() -> GenerateInput {
        let theme = Theme::default();
        GenerateInput {
            launch: true,
            shell_exe: "/home/u/.local/bin/nuxut-shell".into(),
            look: Some(LookGroup::from_theme(&theme, Some(BlurLook { size: 8, passes: 2 }))),
            motion: Some(MotionGroup::from_theme(&theme)),
            windows: Some(WindowsGroup::from_theme(&theme, 4, 8, 2)),
            hide_defaults: true,
            displays: None,
            env: Vec::new(),
            input: None,
        }
    }

    fn value<'a>(g: &'a Generated, key: &str) -> Vec<&'a str> {
        g.keywords.iter().filter(|(k, _)| k == key).map(|(_, v)| v.as_str()).collect()
    }

    #[test]
    fn full_output_maps_theme() {
        let theme = Theme::default();
        let g = generate_conf(&input());
        assert_eq!(value(&g, "decoration:rounding"), [px(theme.shape.radius_large)]);
        assert_eq!(value(&g, "decoration:shadow:enabled"), ["true"]);
        assert_eq!(value(&g, "decoration:shadow:color"), [hypr_color(theme.shadow.color)]);
        assert_eq!(value(&g, "general:gaps_out"), ["8"]);
        assert_eq!(value(&g, "animations:enabled"), ["true"]);
        assert_eq!(value(&g, "animation").len(), 14);
        assert_eq!(value(&g, "bezier"), ["nuxut_decel, 0.05, 0.7, 0.1, 1", "nuxut_accel, 0.3, 0, 0.8, 0.15", "nuxut_std, 0.2, 0, 0, 1"]);
        // Sakin ön ayar: pencere girişi bandın üstünde (250 ms), çalışma alanı 272 ms yatay kaydırma.
        assert!(value(&g, "animation").contains(&"windowsIn, 1, 2.5, nuxut_decel, popin 92%"));
        assert!(value(&g, "animation").contains(&"windowsOut, 1, 2, nuxut_accel, popin 92%"));
        assert!(value(&g, "animation").contains(&"workspaces, 1, 2.7, nuxut_decel, slide"));
        assert!(value(&g, "animation").contains(&"layersIn, 1, 2, nuxut_decel, fade"));
        assert_eq!(value(&g, "misc:disable_hyprland_logo"), ["true"]);
        assert_eq!(value(&g, "misc:force_default_wallpaper"), ["0"]);
        assert!(g.text.contains("\nmisc:disable_hyprland_logo = true\nmisc:force_default_wallpaper = 0\n"));
        assert!(g.text.contains(&format!("\nexec-once = {}\n", start_command("/home/u/.local/bin/nuxut-shell"))));
        // polkit ajanı: yalnızca etkin birim; doğrudan başlatma yedeği yok.
        assert!(g.text.contains(&format!("\nexec-once = {}\n", polkit_start_command())));
        assert!(polkit_start_command().starts_with("systemctl --user is-enabled --quiet nuxut-polkit.service && "));
        assert!(!polkit_start_command().contains("exec ") && !polkit_start_command().contains("||"));
        // Kısayollar ve kurallar temel katmanda (base.rs); bu dosyada bind/kural yok.
        assert!(!g.text.contains("\nbind") && !g.text.contains("windowrule"));
        // Her keyword metinde de aynen var (tek kaynak) ve güvenli.
        for (k, v) in &g.keywords {
            assert!(g.text.contains(&format!("{k} = {v}\n")), "{k}");
            assert!(safe_value(v));
        }
        // Yönetilen anahtar listesi eksiksiz.
        for (k, _) in &g.keywords {
            assert!(MANAGED_KEYS.contains(&k.as_str()) || k == "bezier" || k == "animation", "{k}");
        }
    }

    #[test]
    fn groups_off_produce_no_lines() {
        let g = generate_conf(&GenerateInput {
            launch: false,
            shell_exe: String::new(),
            look: None,
            motion: None,
            windows: None,
            hide_defaults: false,
            displays: None,
            env: Vec::new(),
            input: None,
        });
        assert!(g.keywords.is_empty());
        assert!(!g.text.contains("exec-once"));
        assert!(g.text.lines().all(|l| l.is_empty() || l.starts_with('#')));
    }

    /// Birleşik görünümün imleç ortamı: `env =` satırları, canlı batch'e girmez; güvensiz çift atlanır.
    #[test]
    fn cursor_env_lines() {
        let env = vec![
            ("XCURSOR_THEME".to_owned(), "Bibata-Modern-Classic".to_owned()),
            ("XCURSOR_SIZE".to_owned(), "24".to_owned()),
            ("HYPRCURSOR_THEME".to_owned(), "x;reload".to_owned()),
            ("bad key".to_owned(), "1".to_owned()),
        ];
        let g = generate_conf(&GenerateInput { env, ..input() });
        assert!(g.text.contains("\nenv = XCURSOR_THEME,Bibata-Modern-Classic\nenv = XCURSOR_SIZE,24\n"), "{}", g.text);
        assert!(!g.text.contains("reload") && !g.text.contains("bad key"));
        assert!(g.keywords.iter().all(|(k, _)| k != "env"));
        assert!(!generate_conf(&input()).text.contains("\nenv = XCURSOR"));
        assert!(safe_env("XCURSOR_SIZE", "") && !safe_env("X", "a\"b") && !safe_env("", "a"));
    }

    fn input_settings(text: &str) -> InputSettings {
        let (c, w) = crate::config::ShellConfig::parse(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        c.input.settings
    }

    /// `[input]`: yalnızca yazılı anahtarlar satır üretir (kb_layout yazılmadıysa kullanıcının
    /// değeri kalır); düzen + varyant birlikte; boş seçenek listesi açıkça temizler.
    #[test]
    fn input_lines_only_for_written_keys() {
        assert!(!generate_conf(&input()).text.contains("input:"));
        let only_rate = input_settings("[input.keyboard]\nrepeat_rate = 70\n");
        let g = generate_conf(&GenerateInput { input: Some(only_rate), ..input() });
        assert_eq!(g.text.lines().filter(|l| l.starts_with("input:")).collect::<Vec<_>>(), ["input:repeat_rate = 70"]);
        assert_eq!(value(&g, "input:repeat_rate"), ["70"]);
        assert!(value(&g, "input:kb_layout").is_empty(), "yazılmamış düzen: tr korunur");

        let s = input_settings(
            "[input.keyboard]\nlayouts = [{ layout = \"tr\" }, { layout = \"us\", variant = \"intl\" }]\noptions = []\n\
             [input.touchpad]\ntap_to_click = false\nscroll_factor = 0.5\n[input.mouse]\nsensitivity = -0.25\n",
        );
        let g = generate_conf(&GenerateInput { input: Some(s), ..input() });
        assert!(
            g.text.contains(
                "input:kb_layout = tr,us\ninput:kb_variant = ,intl\ninput:kb_options = \ninput:sensitivity = -0.25\n\
                 input:touchpad:tap-to-click = false\ninput:touchpad:scroll_factor = 0.5\n"
            ),
            "{}",
            g.text
        );
        // sync_input kapalı (None) ya da boş bölüm: satır yok.
        let g = generate_conf(&GenerateInput { input: Some(InputSettings::default()), ..input() });
        assert!(!g.text.contains("input:") && !g.text.contains("Klavye"));
    }

    #[test]
    fn defaults_hidden_only_when_shell_draws_wallpaper() {
        let mut i = input();
        i.hide_defaults = false;
        let g = generate_conf(&i);
        assert!(value(&g, "misc:disable_hyprland_logo").is_empty());
        assert!(value(&g, "misc:force_default_wallpaper").is_empty());
        assert!(!g.text.contains("misc:"));
    }

    #[test]
    fn unsafe_exe_falls_back_to_path() {
        let mut i = input();
        i.shell_exe = "/tmp/x; rm -rf ~ #".into();
        let g = generate_conf(&i);
        assert!(g.text.contains("|| exec nuxut-shell\n"));
        assert!(!g.text.contains("rm -rf"));
        assert!(!g.text.contains("envd = PATH"), "güvensiz yolda PATH satırı yok");
    }

    /// Kabuğun dizini oturumun PATH'ine: oturumda bir kez (yeniden yüklemede büyümez), systemd'ye de
    /// (`envd`); sistem dizininde satır yok.
    #[test]
    fn session_path_block_once_per_session() {
        let g = generate_conf(&input());
        assert!(
            g.text.contains(
                "\n# hyprlang if !NUXUT_PATH_ADDED\nenvd = PATH,/home/u/.local/bin:$PATH\nenv = NUXUT_PATH_ADDED,1\n# hyprlang endif\n"
            ),
            "{}",
            g.text
        );
        assert!(g.keywords.iter().all(|(k, _)| k != "env" && k != "envd"), "canlı batch'e girmez");
        assert_eq!(session_path_dir("/usr/bin/nuxut-shell"), None);
        assert_eq!(session_path_dir("nuxut-shell"), None);
        assert_eq!(session_path_dir("/opt/nuxut/bin/nuxut-shell"), Some("/opt/nuxut/bin"));
        let g = generate_conf(&GenerateInput { shell_exe: "/usr/bin/nuxut-shell".into(), ..input() });
        assert!(!g.text.contains("envd = PATH"));
    }

    #[test]
    fn displays_file_is_sourced_last_when_present() {
        assert!(!generate_conf(&input()).text.contains("nuxut-displays"));
        let g = generate_conf(&GenerateInput { displays: Some("~/.config/hypr/nuxut-displays.conf".into()), ..input() });
        assert!(g.text.ends_with("\nsource = ~/.config/hypr/nuxut-displays.conf\n"), "{}", g.text);
        assert!(!g.keywords.iter().any(|(k, _)| k == "source"), "canlı keyword değil");
        let g = generate_conf(&GenerateInput { displays: Some("/x;reload".into()), ..input() });
        assert!(!g.text.contains("source ="));
    }

    #[test]
    fn motion_disabled_or_instant() {
        let mut i = input();
        i.motion.as_mut().unwrap().enabled = false;
        let g = generate_conf(&i);
        assert_eq!(value(&g, "animations:enabled"), ["false"]);
        assert!(value(&g, "animation").is_empty());

        let mut i = input();
        let m = i.motion.as_mut().unwrap();
        m.effects = Motion::Spring(Spring::INSTANT);
        let g = generate_conf(&i);
        assert!(value(&g, "animation").contains(&"fade, 0"));
        assert!(value(&g, "animation").contains(&"windowsOut, 0"));
        assert!(value(&g, "animation").iter().any(|v| v.starts_with("windowsIn, 1, ")));
    }

    fn plan_ms(m: &MotionGroup, leaf: &str) -> Option<f32> {
        anim_plan(m).unwrap().iter().find(|a| a.leaf == leaf).and_then(|a| a.ms)
    }

    /// Ön ayarlar bantlara oturur; hız çarpanı bandın üstüne uygulanır (2× → yarı süre); hiçbir
    /// eğri taşmaz ve hiçbir pencere stili `popin`'i %90'ın altına indirmez (patlama yok).
    #[test]
    fn anim_plan_bands_speed_and_no_overshoot() {
        let theme = |preset: &str, speed: f32| {
            let (t, w) = Theme::parse(&format!("[motion]\npreset = \"{preset}\"\nspeed = {speed}\n")).unwrap();
            assert!(w.is_empty(), "{w:?}");
            MotionGroup::from_theme(&t)
        };
        for preset in ["calm", "lively", "snappy"] {
            let m = theme(preset, 1.0);
            let enter = plan_ms(&m, "windowsIn").unwrap();
            assert!((ENTER_MS.0..=ENTER_MS.1).contains(&enter), "{preset}: {enter}");
            let ws = plan_ms(&m, "workspaces").unwrap();
            assert!((WORKSPACE_MS.0..=WORKSPACE_MS.1).contains(&ws), "{preset}: {ws}");
            let exit = plan_ms(&m, "windowsOut").unwrap();
            assert!((EXIT_MS.0..=EXIT_MS.1).contains(&exit), "{preset}: {exit}");
            let fast = theme(preset, 2.0);
            assert!((plan_ms(&fast, "windowsIn").unwrap() - enter / 2.0).abs() < 0.5, "{preset}");
        }
        assert!(anim_plan(&theme("instant", 1.0)).is_none(), "anında ön ayar = animasyon kapalı");
        for (_, b) in CURVES {
            assert!((0.0..=1.0).contains(&b.y1) && (0.0..=1.0).contains(&b.y2), "taşma: {b:?}");
        }
        for a in anim_plan(&theme("calm", 1.0)).unwrap() {
            if let Some(p) = a.style.strip_prefix("popin ") {
                assert!(p.trim_end_matches('%').parse::<u32>().unwrap() >= 90, "{}", a.style);
            }
        }
    }

    #[test]
    fn role_mapping() {
        // Az sönümlü yay taşar, kritik yay taşmaz.
        let (b, ms) = role_curve(Motion::Spring(Spring::new(0.42, 0.70))).unwrap();
        assert!(b.y1 > 1.0 && b.y1 <= 1.5);
        assert!((ms - 0.42 * 1000.0 * (1.6 - 0.42)).abs() < 1.0);
        let (b, ms) = role_curve(Motion::Spring(Spring::new(0.30, 1.0))).unwrap();
        assert_eq!(b, CubicBezier::DECELERATE);
        assert!((ms - 300.0).abs() < 0.01);
        // Eğri: aynen.
        let c = Curve::new(CubicBezier::EASE, 0.2);
        assert_eq!(role_curve(Motion::Curve(c)), Some((CubicBezier::EASE, 200.0)));
        assert_eq!(role_curve(Motion::Curve(Curve::new(CubicBezier::EASE, 0.0))), None);
        assert_eq!(deciseconds(380.0), "3.8");
        assert_eq!(deciseconds(1.0), "0.5");
        assert_eq!(deciseconds(1e9), "30");
    }

    /// `mode` kipinde, `seed` tohumlu duvar kâğıdı paletinin 11 token'ı + karşıtlık kipiyle tema
    /// (gerçek akış: `framework.toml [colors]` → `Theme`, yüksekte palet de itilir).
    fn seeded_theme(seed: &str, mode: &str, contrast: &str) -> Theme {
        use nuxut_palette::{Mode, Palette, Variant};
        let m = if mode == "dark" { Mode::Dark } else { Mode::Light };
        let p = Palette::generate(Color::from_hex(seed).unwrap(), Variant::TonalSpot, m, 0.0);
        let mut toml = format!("[colors]\nmode = \"{mode}\"\ncontrast = \"{contrast}\"\n");
        for (name, c) in p.theme_colors_for(m) {
            let _ = writeln!(toml, "{name} = \"{}\"", c.to_hex());
        }
        let (t, w) = Theme::parse(&toml).unwrap();
        assert!(w.is_empty(), "{w:?}");
        t
    }

    const SEEDS: [&str; 5] = ["#6750a4", "#ff0000", "#00a0ff", "#7d8c3a", "#808080"];

    #[test]
    fn border_colors_standard_unchanged() {
        // Standart kipte eski ifade birebir: odaklı = primary, pasif = outline × 0.4.
        let mut themes = vec![Theme::default(), Theme::parse("[colors]\nmode = \"light\"\n").unwrap().0];
        for seed in SEEDS {
            for mode in ["dark", "light"] {
                themes.push(seeded_theme(seed, mode, "standard"));
            }
        }
        for t in themes {
            let w = WindowsGroup::from_theme(&t, 4, 8, 2);
            assert_eq!(w.active_border, t.colors.primary);
            assert_eq!(w.inactive_border, t.colors.outline.multiply_alpha(0.4));
            assert_eq!(hypr_color(w.inactive_border), hypr_color(t.colors.outline.multiply_alpha(0.4)));
        }
    }

    #[test]
    fn border_colors_high_contrast_reach_non_text_min() {
        use nuxut::theme::ContrastMode;
        use nuxut::ui::style::worst_over;
        let mut themes = Vec::new();
        for mode in ["dark", "light"] {
            themes.push(Theme::parse(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"high\"\n")).unwrap().0);
            for seed in SEEDS {
                themes.push(seeded_theme(seed, mode, "high"));
            }
        }
        for t in themes {
            assert_eq!(t.contrast, ContrastMode::High);
            let bg = [t.colors.surface];
            let w = WindowsGroup::from_theme(&t, 4, 8, 2);
            // Ekranda görüneceği gibi (8 bit, surface üstüne bindirilmiş) ≥ 3:1.
            assert!(worst_over(w.inactive_border, &bg) >= NON_TEXT_MIN, "{:?}: {}", t.mode, hypr_color(w.inactive_border));
            assert!(worst_over(w.active_border, &bg) >= NON_TEXT_MIN, "{:?}: {}", t.mode, hypr_color(w.active_border));
            // Odaklı kenarlık zaten eşikte: dokunulmaz.
            assert_eq!(w.active_border, t.colors.primary);
            // Pasif kenarlık rengi (ton) korunur; yalnız opaklığı artar ya da gerekirse uca karışır.
            assert!(w.inactive_border.a >= INACTIVE_BORDER_ALPHA);
            // Üretilen satır güvenli.
            assert!(safe_value(&hypr_color(w.inactive_border)));
        }
    }

    #[test]
    fn colors_and_numbers() {
        assert_eq!(hypr_color(Color::rgba(1.0, 0.0, 0.5, 0.42)), "rgba(ff00806b)");
        assert_eq!(num(2.0), "2");
        assert_eq!(num(0.05), "0.05");
        assert_eq!(px(f32::NAN), "0");
        assert!(!safe_value("a;b"));
        assert!(!safe_value("a\nb"));
        assert!(!safe_value("#fff"));
    }
}
