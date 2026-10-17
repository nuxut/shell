//! Temel katman (ADR-0006, tasarım `docs/design/hyprland-base.md`): kullanıcının Hyprland
//! ayarının **başında** dahil edilen, Nuxut'un ürettiği dosya — `nuxut-base.conf` (hyprlang) ya da
//! `nuxut/base.lua` (Lua). Bütün kısayollar (kabuk eylemleri + pencere/çalışma alanı yönetimi),
//! pencere ve katman kuralları, yerleşim/jest/davranış varsayılanları burada; kullanıcının
//! satırları sonra okunur ve istediğini ezer (aynı tuşu bağladıysa Nuxut'unki hiç üretilmez:
//! [`super::keybinds::split_bindings`]; kabuk eylemleri ayar katmanında devralınır: [`conf_takeover_section`]).
//!
//! Saf: girdi doğrulanmış ve çakışmalardan arındırılmış bind'ler + seçenekler, çıktı metin. Lua
//! tarafında her dize [`lua_str`]'den geçer; hyprlang'da değerler yalnızca koddaki sabitlerden ve
//! doğrulanmış tuş adlarından gelir ([`safe_value`] ile ayrıca denetlenir).

use std::fmt::Write as _;

use super::generate::{safe_exe, safe_value};
use super::keybinds::{Action, BindKind, Binding, Combo, Concrete, Dir, Dispatch, SCRATCHPAD, Target};
use super::lua::lua_str;

/// Temel katman dosyalarının adı (ana ayar dosyasının dizinine göre).
pub const CONF_NAME: &str = "nuxut-base.conf";
pub const LUA_REL: &str = "nuxut/base.lua";
/// Lua modül adı (`require("nuxut.base")` → `<dizin>/nuxut/base.lua`).
pub const LUA_MODULE: &str = "nuxut.base";
/// Kullanıcının Lua dosyasının **başına** eklenen kod (satırın açıklaması aynı satırda: [`LUA_NOTE`]).
pub const LUA_INCLUDE_LINE: &str = "pcall(require, \"nuxut.base\")";

/// Başa eklenen satırın sonundaki açıklama (dosya içeriği; çevrilmez). Tek satır: açıklama ayrı satır
/// olsaydı iki parçanın başa ekleme sırası geri yüklemede ters dönebilirdi.
pub const CONF_NOTE: &str = "# Nuxut temel katmanı: önce okunur, sonraki satırlarınız ezer"; // i18n: tanı hyprland.conf'a yazılan yorum (dosya içeriği)
pub const LUA_NOTE: &str = "-- Nuxut temel katmanı: önce okunur, sonraki satırlarınız ezer"; // i18n: tanı hyprland.lua'ya yazılan yorum (dosya içeriği)

/// Üretici girdisi.
#[derive(Debug, Clone, PartialEq)]
pub struct BaseInput {
    /// Kurulu kabuk ikilisinin yolu (exec bind'leri; güvensizse PATH'teki `nuxut-shell`).
    pub shell_exe: String,
    /// Çakışmalardan arındırılmış bind'ler (kabuk + temel katman eylemleri).
    pub binds: Vec<Binding>,
    /// `None`: `[compositor] base_layer = false` — kural ve varsayılan yok, yalnızca bind'ler.
    pub layer: Option<BaseLayer>,
}

/// Temel katmanın bind dışı içeriği.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BaseLayer {
    /// Nuxut katmanlarının bulanıklığı: `Some(eşik)` = `blur` + `ignore_alpha eşik` (pencere
    /// bulanıklığı açıkken), `None` = bulanıklık yok.
    pub layer_blur: Option<f32>,
}

/// `ignore_alpha` eşiği: gölgenin en koyu yeri bunun altında kalmalı (gölgenin arkası
/// bulanıklaşmasın), panel zemini üstünde. Gölge opaklığından türetilir, [0.2, 0.9].
pub fn ignore_alpha_for(shadow_alpha: Option<f32>) -> f32 {
    let a = shadow_alpha.filter(|a| a.is_finite()).unwrap_or(0.0);
    ((a + 0.05).clamp(0.2, 0.9) * 100.0).round() / 100.0
}

// ---------------------------------------------------------------------------------------
// Varsayılanlar (gerekçeler: docs/design/hyprland-base.md §5)

/// Bir seçenek değeri (her iki arka uç için).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Val {
    Bool(bool),
    Int(i32),
}

impl Val {
    fn text(self) -> String {
        match self {
            Val::Bool(b) => b.to_string(),
            Val::Int(i) => i.to_string(),
        }
    }
}

/// (hyprlang `kategori:anahtar` yolu, değer). Lua tablosu aynı yoldan kurulur.
const OPTIONS: &[(&str, Val)] = &[
    // Kenardan sürükleyerek boyutlandırma (başlık çubuğu yok; GNOME/KDE/Windows'ta olduğu gibi).
    ("general:resize_on_border", Val::Bool(true)),
    // Yüzen pencereler kenarlara ve birbirine yapışır.
    ("general:snap:enabled", Val::Bool(true)),
    // Tıklayınca odak (GNOME/KDE/Windows/macOS); tekerlek imlecin altındaki pencereye gider.
    ("input:follow_mouse", Val::Int(2)),
    // dwindle: bölme yönü korunur, yeni pencere sağa/alta (öngörülebilir yerleşim).
    ("dwindle:preserve_split", Val::Bool(true)),
    ("dwindle:force_split", Val::Int(2)),
    // Hyprland'in maskot duvar kağıdı, logosu ve açılış yazısı yok (zemin Nuxut'un).
    ("misc:disable_hyprland_logo", Val::Bool(true)),
    ("misc:disable_splash_rendering", Val::Bool(true)),
    ("misc:force_default_wallpaper", Val::Int(0)),
    // Geçerli etkinleştirme isteği (bağlantı → tarayıcı) pencereyi öne getirir.
    ("misc:focus_on_activate", Val::Bool(true)),
    // Kapalı ekranı fare ve klavye uyandırır.
    ("misc:mouse_move_enables_dpms", Val::Bool(true)),
    ("misc:key_press_enables_dpms", Val::Bool(true)),
    // Kilit istemcisi çökerse yeni bir kilit (`msg lock`, logind Lock) oturumu yeniden kilitleyebilir;
    // kapalıyken Hyprland yalnızca "kilit çöktü" ekranı gösterir ve yeni kilidi reddeder.
    ("misc:allow_session_lock_restore", Val::Bool(true)),
    // Çalışma alanı değişince karalama alanı kapanır.
    ("binds:hide_special_on_workspace_change", Val::Bool(true)),
    // Hyprland sürüm haberi açılır penceresi masaüstünde görünmez (güncelleme Nuxut'un işi).
    ("ecosystem:no_update_news", Val::Bool(true)),
];

/// Pencere kuralı: (Lua adı, eşleşmeler, etkiler).
struct WindowRule {
    name: &'static str,
    matches: &'static [(&'static str, Effect)],
    effects: &'static [(&'static str, Effect)],
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Effect {
    On,
    Bool(bool),
    Str(&'static str),
}

/// Diyalogların ortak etkisi.
const FLOAT_CENTER: &[(&str, Effect)] = &[("float", Effect::On), ("center", Effect::On)];

/// Pencere kuralları. Düzenli ifadeler RE2; hyprlang'da alan ayırıcısı olan virgül içermez (testli).
const WINDOW_RULES: &[WindowRule] = &[
    // Kipli diyaloglar (üst pencereli/sabit boyutlu olanları Hyprland zaten yüzdürür).
    WindowRule { name: "nuxut-modal", matches: &[("modal", Effect::Bool(true))], effects: FLOAT_CENTER },
    // Portal dosya seçicileri (uygulamanın penceresinden ayrı süreç; üst pencere ilişkisi yok).
    WindowRule {
        name: "nuxut-file-chooser",
        matches: &[("class", Effect::Str(r"^(xdg-desktop-portal-gtk|xdg-desktop-portal-gnome|org\.freedesktop\.impl\.portal\.desktop\.kde)$"))],
        effects: FLOAT_CENTER,
    },
    // Nuxut'un kendi portal dosya seçicisi (app-id `nuxut-filechooser`, en küçük
    // boyutu 640×420 uygulama verir).
    WindowRule {
        name: "nuxut-filechooser",
        matches: &[("class", Effect::Str(r"^(nuxut-filechooser)$"))],
        effects: &[("float", Effect::On), ("center", Effect::On), ("size", Effect::Str("960 640"))],
    },
    // Uygulama içi Aç/Kaydet diyalogları (başlıktan; İngilizce başlıklar).
    WindowRule {
        name: "nuxut-open-save",
        matches: &[("title", Effect::Str(r"^((Open|Save|Select|Choose) (a )?(File|Files|Folder|Directory)|Save As)( .*)?$"))],
        effects: FLOAT_CENTER,
    },
    // Tarayıcıların portalsız diyalogları: Firefox "File Upload", Chromium "<site> wants to
    // save/open"; Firefox Kütüphane penceresi (yalnızca Firefox türevlerinde, başlık çok genel).
    WindowRule {
        name: "nuxut-browser-dialogs",
        matches: &[("title", Effect::Str(r"^(File Upload|.+ wants to (save|open))( .*)?$"))],
        effects: FLOAT_CENTER,
    },
    WindowRule {
        name: "nuxut-firefox-library",
        matches: &[("class", Effect::Str(r"^(firefox|firefox-esr|librewolf|zen|floorp|waterfox)$")), ("title", Effect::Str(r"^(Library)$"))],
        effects: FLOAT_CENTER,
    },
    // Ekran paylaşım göstergesi (Chromium "… is sharing your screen." çubuğu, Firefox "Sharing
    // Indicator"): döşenip yer kaplamaz, odak çalmaz, her alanda görünür, alt ortada.
    WindowRule {
        name: "nuxut-share-indicator",
        matches: &[("title", Effect::Str(r"^(.+ is sharing (your screen|a window|a tab|this tab)\.?|(.+ — )?Sharing Indicator)$"))],
        effects: &[
            ("float", Effect::On),
            ("pin", Effect::On),
            ("no_initial_focus", Effect::On),
            ("move", Effect::Str("monitor_w*0.5-window_w*0.5 monitor_h-window_h-16")),
        ],
    },
    // Başka polkit ajanları (Nuxut'unki katman yüzeyi: nuxut-polkit).
    WindowRule {
        name: "nuxut-polkit-agents",
        matches: &[(
            "class",
            Effect::Str(r"^(polkit-gnome-authentication-agent-1|hyprpolkitagent|org\.kde\.polkit-kde-authentication-agent-1|lxqt-policykit-agent|polkit-mate-authentication-agent-1)$"),
        )],
        effects: FLOAT_CENTER,
    },
    // Resim içinde resim: sabit, sağ alt köşe, ekran genişliğinin dörtte biri, 16:9.
    WindowRule {
        name: "nuxut-pip",
        matches: &[("title", Effect::Str(r"^(Picture-in-Picture|Picture in picture|Picture in Picture)$"))],
        effects: &[
            ("float", Effect::On),
            ("pin", Effect::On),
            ("keep_aspect_ratio", Effect::On),
            ("size", Effect::Str("monitor_w*0.25 monitor_w*0.140625")),
            ("move", Effect::Str("monitor_w-window_w-32 monitor_h-window_h-32")),
        ],
    },
    // Hoş geldin penceresi: yüzer, ortada; boyutu istemci verir (xdg_toplevel min=max 720×640).
    WindowRule { name: "nuxut-welcome", matches: &[("class", Effect::Str(r"^(nuxut-welcome)$"))], effects: FLOAT_CENTER },
    // Ayarlar: yüzer, ortada, uygulamanın kendi varsayılan boyutu.
    WindowRule {
        name: "nuxut-settings",
        matches: &[("class", Effect::Str(r"^(nuxut-settings)$"))],
        effects: &[("float", Effect::On), ("center", Effect::On), ("size", Effect::Str("1040 720"))],
    },
    // Tam ekran video/oyun boşta kalmayı engeller (içerik türünü bildiren istemciler; tarayıcı ve
    // oynatıcılar oynatırken zaten kendileri engeller).
    WindowRule {
        name: "nuxut-idle-inhibit",
        matches: &[("content", Effect::Str("^(video|game)$"))],
        effects: &[("idle_inhibit", Effect::Str("fullscreen"))],
    },
    // XWayland sürükle-bırak kaynak penceresi odak çalmasın (Hyprland örnek ayarındaki düzeltme).
    WindowRule {
        name: "nuxut-xwayland-drag",
        matches: &[
            ("class", Effect::Str("^$")),
            ("title", Effect::Str("^$")),
            ("xwayland", Effect::Bool(true)),
            ("float", Effect::Bool(true)),
            ("fullscreen", Effect::Bool(false)),
            ("pin", Effect::Bool(false)),
        ],
        effects: &[("no_focus", Effect::On)],
    },
];

/// Nuxut katmanının animasyonu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerAnim {
    /// Yüzey kendi içeriğini canlandırır ya da anında görünmeli.
    None,
    Fade,
}

/// (ad alanı, animasyon, bulanıklık alır mı).
const LAYERS: &[(&str, LayerAnim, bool)] = &[
    // Tek tam ekran kabuk yüzeyi: bar, paneller, başlatıcı, bildirimler kendi hareketiyle.
    ("nuxut", LayerAnim::None, true),
    ("nuxut-polkit", LayerAnim::Fade, true),
    ("nuxut-wallpaper", LayerAnim::None, false),
    ("nuxut-dim", LayerAnim::None, false),
    ("nuxut-exclusion", LayerAnim::None, false),
    // Ekran görüntüsü ve paylaşım seçici: anında (görüntüye geçiş izi düşmesin).
    ("nuxut-screenshot", LayerAnim::None, false),
    ("nuxut-sharepicker", LayerAnim::None, false),
];

// ---------------------------------------------------------------------------------------
// Bind çevirisi

fn settings_exe(shell_exe: &str) -> String {
    let exe = safe_exe(shell_exe);
    match exe.rsplit_once('/') {
        Some((dir, _)) if exe.starts_with('/') => format!("{dir}/nuxut-settings"),
        _ => "nuxut-settings".to_owned(),
    }
}

/// Exec hedeflerinin komut satırı; dispatcher hedefinde `None`.
fn command(target: Target, shell_exe: &str) -> Option<String> {
    Some(match target {
        Target::Shell(args) => format!("{} {args}", safe_exe(shell_exe)),
        Target::Playerctl(args) => format!("playerctl {args}"),
        Target::Hyprctl(args) => format!("hyprctl {args}"),
        Target::ShellOr(args, fallback) => format!("{} {args} || {fallback}", safe_exe(shell_exe)),
        Target::Settings => settings_exe(shell_exe),
        Target::Dispatch(_) => return None,
    })
}

fn step(s: i8) -> String {
    format!("{s:+}")
}

/// hyprlang: (dispatcher, argüman).
fn conf_dispatch(d: Dispatch, index: Option<u8>) -> (&'static str, String) {
    let n = index.unwrap_or(1).to_string();
    match d {
        Dispatch::Close => ("killactive", String::new()),
        Dispatch::Fullscreen => ("fullscreen", "0".into()),
        Dispatch::Maximize => ("fullscreen", "1".into()),
        Dispatch::ToggleFloating => ("togglefloating", String::new()),
        Dispatch::Pin => ("pin", String::new()),
        Dispatch::Focus(dir) => ("movefocus", dir.letter().into()),
        Dispatch::Move(dir) => ("movewindow", dir.letter().into()),
        Dispatch::Resize(x, y) => ("resizeactive", format!("{x} {y}")),
        Dispatch::Workspace => ("workspace", n),
        Dispatch::MoveToWorkspace => ("movetoworkspace", n),
        Dispatch::WorkspaceStep(s) => ("workspace", format!("r{}", step(s))),
        Dispatch::MoveToWorkspaceStep(s) => ("movetoworkspace", format!("r{}", step(s))),
        Dispatch::WorkspaceScroll(s) => ("workspace", format!("e{}", step(s))),
        Dispatch::WorkspacePrevious => ("workspace", "previous".into()),
        Dispatch::Cycle(next) => ("cyclenext", if next { String::new() } else { "prev".into() }),
        Dispatch::ToggleScratchpad => ("togglespecialworkspace", SCRATCHPAD.into()),
        Dispatch::MoveToScratchpad => ("movetoworkspacesilent", format!("special:{SCRATCHPAD}")),
        Dispatch::ToggleSplit => ("layoutmsg", "togglesplit".into()),
        Dispatch::MouseMove => ("movewindow", String::new()),
        Dispatch::MouseResize => ("resizewindow", String::new()),
    }
}

/// Lua: `hl.dsp…` ifadesi.
fn lua_dispatch(d: Dispatch, index: Option<u8>) -> String {
    let n = index.unwrap_or(1);
    let dir = |d: Dir| lua_str(d.word());
    match d {
        Dispatch::Close => "hl.dsp.window.close()".into(),
        Dispatch::Fullscreen => "hl.dsp.window.fullscreen()".into(),
        Dispatch::Maximize => "hl.dsp.window.fullscreen({ mode = \"maximized\" })".into(),
        Dispatch::ToggleFloating => "hl.dsp.window.float()".into(),
        Dispatch::Pin => "hl.dsp.window.pin()".into(),
        Dispatch::Focus(d) => format!("hl.dsp.focus({{ direction = {} }})", dir(d)),
        Dispatch::Move(d) => format!("hl.dsp.window.move({{ direction = {} }})", dir(d)),
        Dispatch::Resize(x, y) => format!("hl.dsp.window.resize({{ x = {x}, y = {y}, relative = true }})"),
        Dispatch::Workspace => format!("hl.dsp.focus({{ workspace = {n} }})"),
        Dispatch::MoveToWorkspace => format!("hl.dsp.window.move({{ workspace = {n} }})"),
        Dispatch::WorkspaceStep(s) => format!("hl.dsp.focus({{ workspace = {} }})", lua_str(&format!("r{}", step(s)))),
        Dispatch::MoveToWorkspaceStep(s) => format!("hl.dsp.window.move({{ workspace = {} }})", lua_str(&format!("r{}", step(s)))),
        Dispatch::WorkspaceScroll(s) => format!("hl.dsp.focus({{ workspace = {} }})", lua_str(&format!("e{}", step(s)))),
        Dispatch::WorkspacePrevious => "hl.dsp.focus({ workspace = \"previous\" })".into(),
        Dispatch::Cycle(next) => format!("hl.dsp.window.cycle_next({{ next = {next} }})"),
        Dispatch::ToggleScratchpad => format!("hl.dsp.workspace.toggle_special({})", lua_str(SCRATCHPAD)),
        Dispatch::MoveToScratchpad => format!("hl.dsp.window.move({{ workspace = {}, follow = false }})", lua_str(&format!("special:{SCRATCHPAD}"))),
        Dispatch::ToggleSplit => "hl.dsp.layout(\"togglesplit\")".into(),
        Dispatch::MouseMove => "hl.dsp.window.drag()".into(),
        Dispatch::MouseResize => "hl.dsp.window.resize()".into(),
    }
}

/// `nuxut:<eylem>` açıklaması: `hyprctl binds`'te Nuxut'un bind'leri böyle ayırt edilir.
pub fn description(action: Action) -> String {
    format!("nuxut:{}", action.name())
}

/// hyprlang bind satırları (açıklamalı: `bind…d = MOD, tuş, nuxut:eylem, dispatcher, arg`).
pub fn conf_bind_lines(b: &Binding, shell_exe: &str) -> Vec<String> {
    let keyword = format!("bind{}d", b.action.kind().flags());
    b.expand()
        .into_iter()
        .filter_map(|Concrete { combo, index }| {
            let (dispatcher, arg) = match b.action.target() {
                Target::Dispatch(d) => {
                    let (disp, arg) = conf_dispatch(d, index);
                    (disp.to_owned(), arg)
                }
                t => ("exec".to_owned(), command(t, shell_exe)?),
            };
            let head = format!("{keyword} = {}, {}, {dispatcher}", combo.to_hypr(), description(b.action));
            // `bindm`: dispatcher alanı yok (fare eylemi adı yerinde).
            let line = if b.action.kind() == BindKind::Mouse { head } else { format!("{head}, {arg}") };
            safe_value(&line).then_some(line)
        })
        .collect()
}

/// Ayar katmanının (`nuxut.conf`, kullanıcının dosyasından SONRA okunur) sonuna eklenen devralınan
/// kısayollar: kullanıcının önceki bind'i `unbind`, ardından Nuxut'unki. Temel katmana konmaz:
/// oradaki bind kullanıcınınkinden önce okunduğu için `unbind` onu da silerdi. Boşsa boş metin.
pub fn conf_takeover_section(taken: &[(Binding, Vec<Combo>)], shell_exe: &str) -> String {
    let mut text = String::new();
    for (b, unbind) in taken {
        let binds = conf_bind_lines(b, shell_exe);
        let unbinds: Vec<String> = unbind.iter().map(|c| format!("unbind = {}", c.to_hypr())).filter(|l| safe_value(l)).collect();
        // Kaldırılıp yerine bağlanamayan kısayol olmaz: ikisi birden yazılır ya da hiçbiri.
        if binds.is_empty() || unbinds.len() != unbind.len() {
            continue;
        }
        if text.is_empty() {
            text.push_str(
                "\n# Devralınan kısayollar: ayarlarınızdaki aynı tuşlar Nuxut'a geçer (dosyanız değişmez).\n\
                 # Geri almak: shell.toml [compositor] take_over_keys = false\n",
            );
        }
        for l in unbinds.iter().chain(&binds) {
            let _ = writeln!(text, "{l}");
        }
    }
    text
}

/// Lua bind satırları (`hl.bind(tuş, dsp, { …, description = "nuxut:eylem" })`).
pub fn lua_bind_lines(b: &Binding, shell_exe: &str) -> Vec<String> {
    let opts = match b.action.kind() {
        BindKind::Plain | BindKind::Mouse => "",
        BindKind::Locked => "locked = true, ",
        BindKind::LockedRepeat => "locked = true, repeating = true, ",
        BindKind::Repeat => "repeating = true, ",
        BindKind::Release => "release = true, ",
    };
    b.expand()
        .into_iter()
        .filter_map(|Concrete { combo, index }| {
            let dsp = match b.action.target() {
                Target::Dispatch(d) => lua_dispatch(d, index),
                t => format!("hl.dsp.exec_cmd({})", lua_str(&command(t, shell_exe)?)),
            };
            Some(format!("hl.bind({}, {dsp}, {{ {opts}description = {} }})", lua_str(&combo.to_lua()), lua_str(&description(b.action))))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// Dosyalar

fn conf_effect(name: &str, e: Effect) -> String {
    match e {
        Effect::On => format!("{name} on"),
        Effect::Bool(b) => format!("{name} {b}"),
        Effect::Str(s) => format!("{name} {s}"),
    }
}

fn lua_effect(e: Effect) -> String {
    match e {
        Effect::On => "true".into(),
        Effect::Bool(b) => b.to_string(),
        Effect::Str(s) => lua_str(s),
    }
}

fn conf_layer_effects(anim: LayerAnim, blur: Option<f32>) -> Vec<String> {
    let mut out = Vec::new();
    match anim {
        LayerAnim::None => out.push("no_anim on".to_owned()),
        LayerAnim::Fade => out.push("animation fade".to_owned()),
    }
    if let Some(a) = blur {
        out.push("blur on".to_owned());
        out.push(format!("ignore_alpha {a}"));
    }
    out
}

// i18n: tanı üretilen temel katman dosyasının başlığı (dosya içeriği)
const HEADER_CONF: &str = "\
# Nuxut — temel katman; nuxut-shell tarafından üretildi, her değişiklikte üzerine yazılır.
# hyprland.conf'un BAŞINDA source edilir: oradaki kendi satırlarınız sonra okunur ve buradaki her
# değeri ve kuralı ezer. Aynı tuşu kendiniz bağladıysanız Nuxut o kısayolu burada üretmez;
# kabuk kısayollarını (ses, parlaklık, medya, ekran görüntüsü…) nuxut.conf devralır. Bir Nuxut
# kısayolunu kaldırmak için Ayarlar → Kısayollar ya da hyprland.conf'ta `unbind = MOD, tuş`.
";

/// hyprlang temel katman dosyası.
pub fn generate_base_conf(input: &BaseInput) -> String {
    let mut text = String::from(HEADER_CONF);
    if let Some(layer) = &input.layer {
        text.push_str("\n# Davranış ve yerleşim\n");
        for (key, v) in OPTIONS {
            let _ = writeln!(text, "{key} = {}", v.text());
        }
        text.push_str("\n# Dokunmatik yüzey: üç parmakla yatay kaydırma çalışma alanı değiştirir\ngesture = 3, horizontal, workspace\n");
        text.push_str("\n# Pencere kuralları\n");
        for r in WINDOW_RULES {
            let fields: Vec<String> = r
                .matches
                .iter()
                .map(|(k, e)| match e {
                    Effect::Str(s) => format!("match:{k} {s}"),
                    Effect::Bool(b) => format!("match:{k} {b}"),
                    Effect::On => format!("match:{k} true"),
                })
                .chain(r.effects.iter().map(|(k, e)| conf_effect(k, *e)))
                .collect();
            let line = format!("windowrule = {}", fields.join(", "));
            if safe_value(&line) {
                let _ = writeln!(text, "{line}");
            }
        }
        text.push_str("\n# Nuxut katmanları\n");
        for (ns, anim, blur) in LAYERS {
            let effects = conf_layer_effects(*anim, layer.layer_blur.filter(|_| *blur));
            let _ = writeln!(text, "layerrule = match:namespace ^({ns})$, {}", effects.join(", "));
        }
    }
    if !input.binds.is_empty() {
        text.push_str("\n# Kısayollar\n");
        for b in &input.binds {
            for l in conf_bind_lines(b, &input.shell_exe) {
                let _ = writeln!(text, "{l}");
            }
        }
    }
    text
}

/// `kategori:alt:anahtar = değer` yollarından iç içe Lua tablosu (sıra korunur).
fn lua_options_table() -> String {
    #[derive(Default)]
    struct Node {
        children: Vec<(String, Node)>,
        value: Option<String>,
    }
    fn insert(node: &mut Node, path: &[&str], value: String) {
        let Some((first, rest)) = path.split_first() else { return };
        let idx = match node.children.iter().position(|(k, _)| k == first) {
            Some(i) => i,
            None => {
                node.children.push(((*first).to_owned(), Node::default()));
                node.children.len() - 1
            }
        };
        let child = &mut node.children[idx].1;
        if rest.is_empty() {
            child.value = Some(value);
        } else {
            insert(child, rest, value);
        }
    }
    fn render(node: &Node) -> String {
        let parts: Vec<String> = node
            .children
            .iter()
            .map(|(k, n)| match &n.value {
                Some(v) => format!("{k} = {v}"),
                None => format!("{k} = {{ {} }}", render(n)),
            })
            .collect();
        parts.join(", ")
    }
    let mut root = Node::default();
    for (key, v) in OPTIONS {
        let path: Vec<&str> = key.split(':').collect();
        insert(&mut root, &path, v.text());
    }
    render(&root)
}

// i18n: tanı üretilen temel katman Lua modülünün başlığı (dosya içeriği)
const HEADER_LUA: &str = "\
-- Nuxut — temel katman; nuxut-shell tarafından üretildi, her değişiklikte üzerine yazılır.
-- hyprland.lua'nın BAŞINDA pcall(require, \"nuxut.base\") ile yüklenir: kendi satırlarınız sonra
-- çalışır ve buradaki her değeri ve kuralı ezer. Aynı tuşu kendiniz bağladıysanız Nuxut o
-- kısayolu hiç üretmez; bir Nuxut kısayolunu kaldırmak için Ayarlar → Kısayollar ya da hl.unbind.
";

/// Lua temel katman modülü.
pub fn generate_base_lua(input: &BaseInput) -> String {
    let mut body = String::new();
    if let Some(layer) = &input.layer {
        let _ = writeln!(body, "  hl.config({{ {} }})", lua_options_table());
        body.push_str("  hl.gesture({ fingers = 3, direction = \"horizontal\", action = \"workspace\" })\n");
        for r in WINDOW_RULES {
            let matches: Vec<String> = r.matches.iter().map(|(k, e)| format!("{k} = {}", lua_effect(*e))).collect();
            let effects: Vec<String> = r.effects.iter().map(|(k, e)| format!("{k} = {}", lua_effect(*e))).collect();
            let _ = writeln!(
                body,
                "  hl.window_rule({{ name = {}, match = {{ {} }}, {} }})",
                lua_str(r.name),
                matches.join(", "),
                effects.join(", ")
            );
        }
        for (ns, anim, blur) in LAYERS {
            let mut fields = vec![format!("name = {}", lua_str(&format!("nuxut-layer-{ns}"))), format!("match = {{ namespace = {} }}", lua_str(&format!("^({ns})$")))];
            match anim {
                LayerAnim::None => fields.push("no_anim = true".into()),
                LayerAnim::Fade => fields.push("animation = \"fade\"".into()),
            }
            if let Some(a) = layer.layer_blur.filter(|_| *blur) {
                fields.push("blur = true".into());
                fields.push(format!("ignore_alpha = {a}"));
            }
            let _ = writeln!(body, "  hl.layer_rule({{ {} }})", fields.join(", "));
        }
    }
    for b in &input.binds {
        for l in lua_bind_lines(b, &input.shell_exe) {
            let _ = writeln!(body, "  {l}");
        }
    }
    let mut text = String::from(HEADER_LUA);
    text.push_str("\nlocal ok, err = pcall(function()\n");
    text.push_str(&body);
    text.push_str(
        "end)\nif not ok then\n  hl.notification.create({ text = \"Nuxut temel katmanı uygulanamadı: \" .. tostring(err), timeout = 8000 })\nend\n",
    );
    text
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::keybinds::resolve_settings;
    use super::*;

    fn input() -> BaseInput {
        let (binds, w) = resolve_settings(&BTreeMap::new());
        assert!(w.is_empty());
        BaseInput { shell_exe: "/home/u/.local/bin/nuxut-shell".into(), binds, layer: Some(BaseLayer { layer_blur: Some(0.47) }) }
    }

    #[test]
    fn conf_binds_cover_every_shape() {
        let t = generate_base_conf(&input());
        for line in [
            "bindrd = SUPER, super_l, nuxut:launcher, exec, /home/u/.local/bin/nuxut-shell msg launcher toggle",
            "bindrd = SUPER, super_r, nuxut:launcher, exec, /home/u/.local/bin/nuxut-shell msg launcher toggle",
            "bindd = SUPER, l, nuxut:lock, exec, /home/u/.local/bin/nuxut-shell msg lock || loginctl lock-session",
            "bindd = CTRL ALT, delete, nuxut:session_menu, exec, /home/u/.local/bin/nuxut-shell msg session menu",
            "bindd = SUPER, i, nuxut:settings, exec, /home/u/.local/bin/nuxut-settings",
            "bindd = CTRL ALT, t, nuxut:terminal, exec, /home/u/.local/bin/nuxut-shell msg defaults open terminal",
            "bindd = SUPER, q, nuxut:close_window, killactive, ",
            "bindd = ALT, f4, nuxut:close_window_alt, killactive, ",
            "bindd = SUPER, f, nuxut:fullscreen, fullscreen, 0",
            "bindd = SUPER, m, nuxut:maximize, fullscreen, 1",
            "bindd = SUPER SHIFT, space, nuxut:toggle_floating, togglefloating, ",
            "bindd = SUPER, left, nuxut:focus_left, movefocus, l",
            "bindd = SUPER SHIFT, down, nuxut:move_down, movewindow, d",
            "binded = SUPER CTRL, left, nuxut:shrink_width, resizeactive, -40 0",
            "bindd = SUPER, code:10, nuxut:workspace, workspace, 1",
            "bindd = SUPER, code:19, nuxut:workspace, workspace, 10",
            "bindd = SUPER SHIFT, code:12, nuxut:move_to_workspace, movetoworkspace, 3",
            "bindd = SUPER, page_down, nuxut:workspace_next, workspace, r+1",
            "bindd = SUPER SHIFT, page_up, nuxut:move_to_workspace_prev, movetoworkspace, r-1",
            "bindd = SUPER, tab, nuxut:workspace_last, workspace, previous",
            "bindd = ALT SHIFT, tab, nuxut:cycle_windows_back, cyclenext, prev",
            "bindd = SUPER, s, nuxut:scratchpad, togglespecialworkspace, scratchpad",
            "bindd = SUPER SHIFT, s, nuxut:move_to_scratchpad, movetoworkspacesilent, special:scratchpad",
            "bindmd = SUPER, mouse:272, nuxut:window_drag, movewindow",
            "bindmd = SUPER, mouse:273, nuxut:window_resize_drag, resizewindow",
            "bindd = SUPER, mouse_down, nuxut:workspace_scroll_next, workspace, e+1",
            "bindeld = , xf86audioraisevolume, nuxut:volume_up, exec, /home/u/.local/bin/nuxut-shell msg volume up",
            "bindld = , xf86audioplay, nuxut:media_play, exec, playerctl play-pause",
            "bindld = , xf86audiopause, nuxut:media_pause, exec, playerctl play-pause",
            "bindd = CTRL ALT, escape, nuxut:force_kill, exec, hyprctl kill",
        ] {
            assert!(t.contains(&format!("{line}\n")), "{line}\n---\n{t}");
        }
        assert_eq!(t.matches("nuxut:workspace,").count(), 10);
        // Her satır güvenli; bind satırı virgülle tam 5 alan (bindm: 4).
        for l in t.lines().filter(|l| l.starts_with("bind") && !l.starts_with("binds:")) {
            assert!(safe_value(l), "{l}");
            let fields = l.split_once(" = ").unwrap().1.split(',').count();
            assert_eq!(fields, if l.starts_with("bindmd") { 4 } else { 5 }, "{l}");
        }
    }

    #[test]
    fn conf_rules_and_options() {
        let t = generate_base_conf(&input());
        assert!(t.contains("\ninput:follow_mouse = 2\n"));
        assert!(t.contains("\ndwindle:preserve_split = true\ndwindle:force_split = 2\n"));
        assert!(t.contains("\ngesture = 3, horizontal, workspace\n"));
        assert!(!t.contains("no_hardware_cursors"), "imleç donanımı kullanıcının");
        assert!(t.contains("windowrule = match:modal true, float on, center on\n"));
        assert!(t.contains("windowrule = match:class ^(nuxut-welcome)$, float on, center on\n"));
        assert!(t.contains("windowrule = match:class ^(nuxut-settings)$, float on, center on, size 1040 720\n"));
        assert!(t.contains("windowrule = match:content ^(video|game)$, idle_inhibit fullscreen\n"));
        assert!(t.contains("match:title ^(Picture-in-Picture|Picture in picture|Picture in Picture)$, float on, pin on, keep_aspect_ratio on, size monitor_w*0.25 monitor_w*0.140625, move monitor_w-window_w-32 monitor_h-window_h-32\n"));
        assert!(t.contains("\nmisc:allow_session_lock_restore = true\n"));
        assert!(t.contains("windowrule = match:title ^(File Upload|.+ wants to (save|open))( .*)?$, float on, center on\n"));
        assert!(t.contains("windowrule = match:class ^(firefox|firefox-esr|librewolf|zen|floorp|waterfox)$, match:title ^(Library)$, float on, center on\n"));
        assert!(t.contains(
            "windowrule = match:title ^(.+ is sharing (your screen|a window|a tab|this tab)\\.?|(.+ — )?Sharing Indicator)$, float on, pin on, no_initial_focus on, move monitor_w*0.5-window_w*0.5 monitor_h-window_h-16\n"
        ));
        assert!(t.contains("layerrule = match:namespace ^(nuxut)$, no_anim on, blur on, ignore_alpha 0.47\n"));
        assert!(t.contains("layerrule = match:namespace ^(nuxut-polkit)$, animation fade, blur on, ignore_alpha 0.47\n"));
        assert!(t.contains("layerrule = match:namespace ^(nuxut-screenshot)$, no_anim on\n"));
        assert!(t.contains("layerrule = match:namespace ^(nuxut-sharepicker)$, no_anim on\n"));
        // Kural değerlerinde virgül yok (hyprlang alan ayırıcısı).
        for r in WINDOW_RULES {
            for (_, e) in r.matches.iter().chain(r.effects) {
                if let Effect::Str(s) = e {
                    assert!(!s.contains(',') && safe_value(s), "{s}");
                }
            }
        }
        // Bulanıklık kapalı: katman kuralında blur yok; temel katman kapalı: yalnızca bind.
        let t = generate_base_conf(&BaseInput { layer: Some(BaseLayer { layer_blur: None }), ..input() });
        assert!(!t.contains("blur on"));
        let t = generate_base_conf(&BaseInput { layer: None, ..input() });
        assert!(!t.contains("windowrule") && !t.contains("gesture") && !t.contains("follow_mouse"));
        assert!(t.contains("nuxut:launcher"));
    }

    #[test]
    fn lua_module_shape() {
        let t = generate_base_lua(&input());
        assert!(t.starts_with("-- Nuxut — temel katman"));
        assert!(crate::services::apply::is_managed(t.as_bytes()) && crate::services::apply::is_managed(generate_base_conf(&input()).as_bytes()));
        assert!(t.contains("local ok, err = pcall(function()\n"));
        assert!(t.contains(
            "  hl.config({ general = { resize_on_border = true, snap = { enabled = true } }, input = { follow_mouse = 2 }, dwindle = { preserve_split = true, force_split = 2 }, misc = { "
        ));
        assert!(t.contains("  hl.gesture({ fingers = 3, direction = \"horizontal\", action = \"workspace\" })\n"));
        assert!(t.contains("hl.bind(\"SUPER + super_l\", hl.dsp.exec_cmd(\"/home/u/.local/bin/nuxut-shell msg launcher toggle\"), { release = true, description = \"nuxut:launcher\" })"));
        assert!(t.contains("hl.bind(\"SUPER + code:10\", hl.dsp.focus({ workspace = 1 }), { description = \"nuxut:workspace\" })"));
        assert!(t.contains("hl.bind(\"SUPER + SHIFT + code:19\", hl.dsp.window.move({ workspace = 10 }), { description = \"nuxut:move_to_workspace\" })"));
        assert!(t.contains("hl.bind(\"SUPER + CTRL + right\", hl.dsp.window.resize({ x = 40, y = 0, relative = true }), { repeating = true, description = \"nuxut:grow_width\" })"));
        assert!(t.contains("hl.bind(\"SUPER + mouse:272\", hl.dsp.window.drag(), { description = \"nuxut:window_drag\" })"));
        assert!(t.contains("hl.bind(\"ALT + SHIFT + tab\", hl.dsp.window.cycle_next({ next = false }), { description = \"nuxut:cycle_windows_back\" })"));
        assert!(t.contains("hl.window_rule({ name = \"nuxut-pip\", match = { title = \"^(Picture-in-Picture|Picture in picture|Picture in Picture)$\" }, float = true, pin = true"));
        assert!(t.contains("hl.window_rule({ name = \"nuxut-modal\", match = { modal = true }, float = true, center = true })"));
        assert!(t.contains("hl.bind(\"CTRL + ALT + escape\", hl.dsp.exec_cmd(\"hyprctl kill\"), { description = \"nuxut:force_kill\" })"));
        assert!(t.contains("hl.bind(\"xf86audiopause\", hl.dsp.exec_cmd(\"playerctl play-pause\"), { locked = true, description = \"nuxut:media_pause\" })"));
        assert!(t.contains("allow_session_lock_restore = true"));
        assert!(t.contains("match = { class = \"^(firefox|firefox-esr|librewolf|zen|floorp|waterfox)$\", title = \"^(Library)$\" }, float = true, center = true"));
        assert!(t.contains("hl.layer_rule({ name = \"nuxut-layer-nuxut\", match = { namespace = \"^(nuxut)$\" }, no_anim = true, blur = true, ignore_alpha = 0.47 })"));
        // Lua dizesinde `\.` kaçışlı.
        assert!(t.contains("org\\\\.kde\\\\.polkit"));
        for (o, c) in [('(', ')'), ('{', '}')] {
            assert_eq!(t.matches(o).count(), t.matches(c).count(), "{o}{c}");
        }
    }

    #[test]
    fn unsafe_exe_and_settings_path() {
        assert_eq!(settings_exe("/opt/n/bin/nuxut-shell"), "/opt/n/bin/nuxut-settings");
        assert_eq!(settings_exe("/tmp/x; rm -rf ~"), "nuxut-settings");
        assert_eq!(settings_exe(""), "nuxut-settings");
        let t = generate_base_conf(&BaseInput { shell_exe: "/tmp/x;y".into(), ..input() });
        assert!(t.contains(", exec, nuxut-shell msg launcher toggle\n") && t.contains(", exec, nuxut-settings\n"));
    }

    /// Devralınan kısayol: önce kullanıcının tuşu `unbind`, sonra Nuxut'unki; boşsa bölüm yok.
    /// Hyprland kuruluysa gerçek `--verify-config` (compositor başlatmaz) sözdizimini doğrular.
    #[test]
    fn takeover_section_unbinds_then_binds() {
        use super::super::keybinds::parse_hypr_combo;
        assert_eq!(conf_takeover_section(&[], "/x/nuxut-shell"), "");
        let b = |a: Action, c: &str| Binding { action: a, combo: super::super::keybinds::parse_action_combo(a, c).unwrap() };
        let taken = vec![
            (b(Action::VolumeUp, "XF86AudioRaiseVolume"), vec![parse_hypr_combo("", "XF86AudioRaiseVolume").unwrap()]),
            (b(Action::Emoji, "SUPER+period"), vec![parse_hypr_combo("SUPER", "period").unwrap()]),
        ];
        let t = conf_takeover_section(&taken, "/home/u/.local/bin/nuxut-shell");
        let lines: Vec<&str> = t.lines().filter(|l| !l.starts_with('#') && !l.is_empty()).collect();
        assert_eq!(
            lines,
            [
                "unbind = , xf86audioraisevolume",
                "bindeld = , xf86audioraisevolume, nuxut:volume_up, exec, /home/u/.local/bin/nuxut-shell msg volume up",
                "unbind = SUPER, period",
                "bindd = SUPER, period, nuxut:emoji, exec, /home/u/.local/bin/nuxut-shell msg launcher emoji",
            ]
        );
        if std::process::Command::new("Hyprland").arg("--version").output().is_err() {
            eprintln!("Hyprland yok; gerçek doğrulama atlandı");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let conf = tmp.path().join("hyprland.conf");
        std::fs::write(&conf, format!("bind = SUPER, period, exec, true\nbindl = , XF86AudioRaiseVolume, exec, true\n{t}")).unwrap();
        let out = std::process::Command::new("Hyprland")
            .arg("--verify-config")
            .arg("-c")
            .arg(&conf)
            .env("HOME", tmp.path())
            .env("XDG_CONFIG_HOME", tmp.path())
            .output()
            .unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(text.contains("config ok"), "{text}");
    }

    #[test]
    fn ignore_alpha_follows_shadow() {
        assert_eq!(ignore_alpha_for(Some(0.42)), 0.47);
        assert_eq!(ignore_alpha_for(None), 0.2);
        assert_eq!(ignore_alpha_for(Some(1.0)), 0.9);
        assert_eq!(ignore_alpha_for(Some(f32::NAN)), 0.2);
    }

    /// Elle sözdizimi doğrulaması için dosyaları yazar (Hyprland'in yalnız-doğrulama kipi:
    /// `Hyprland --verify-config -c <dizin>/hyprland.conf`; compositor başlatmaz — ADR-0006 §7):
    /// `NUXUT_HYPR_DUMP=<dizin> cargo test --lib dump_for_verify -- --ignored`.
    #[test]
    #[ignore = "elle doğrulama: NUXUT_HYPR_DUMP dizinine yazar"]
    fn dump_for_verify() {
        use crate::services::hypr_config::generate::{BlurLook, GenerateInput, LookGroup, MotionGroup, WindowsGroup, generate_conf};
        use crate::services::hypr_config::lua::generate_lua;
        let Some(dir) = std::env::var_os("NUXUT_HYPR_DUMP").map(std::path::PathBuf::from) else { return };
        let theme = nuxut::Theme::default();
        let tail = GenerateInput {
            launch: false,
            shell_exe: "/usr/bin/nuxut-shell".into(),
            look: Some(LookGroup::from_theme(&theme, Some(BlurLook { size: 8, passes: 2 }))),
            motion: Some(MotionGroup::from_theme(&theme)),
            windows: Some(WindowsGroup::from_theme(&theme, 4, 8, 2)),
            hide_defaults: true,
            displays: None,
            env: Vec::new(),
            input: None,
        };
        std::fs::create_dir_all(dir.join("nuxut")).unwrap();
        let d = dir.display();
        std::fs::write(dir.join("hyprland.conf"), format!("source = {d}/{CONF_NAME}\nsource = {d}/nuxut.conf\n")).unwrap();
        std::fs::write(dir.join(CONF_NAME), generate_base_conf(&input())).unwrap();
        std::fs::write(dir.join("nuxut.conf"), generate_conf(&tail).text).unwrap();
        std::fs::write(dir.join("hyprland.lua"), format!("{LUA_INCLUDE_LINE}\npcall(require, \"nuxut\")\n")).unwrap();
        std::fs::write(dir.join(LUA_REL), generate_base_lua(&input())).unwrap();
        std::fs::write(dir.join("nuxut.lua"), generate_lua(&tail).text).unwrap();
    }
}
