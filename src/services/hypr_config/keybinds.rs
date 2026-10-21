//! Kısayol modeli: kapalı eylem kümesi, tuş dizgesi ayrıştırma, normalleştirme ve çakışma.
//!
//! `shell.toml` `[keybinds]` `eylem = "MOD+MOD+Tuş"` (boş = atanmamış). Kullanıcının kendi
//! Hyprland bind'leriyle çakışan Nuxut bind'i üretilmez; kabuk eylemleri ise ayar katmanından
//! önce okunan kullanıcı bind'ini devralır ([`split_bindings`]). Kullanıcının dosyasına dokunulmaz.
//!
//! Eylemin biçimi ([`Shape`]) değerin nasıl okunacağını belirler: tek tuş birleşimi, tek
//! değiştiriciye dokunma (`launcher = "SUPER"`: Super_L/Super_R bırakılınca) ya da rakam satırı
//! ailesi (`workspace = "SUPER"`: değiştirici + 1…0, tuş kodlarıyla — klavye düzeninden bağımsız).
//! Tasarım ve tablo: `docs/design/hyprland-base.md`.

use std::collections::BTreeMap;
use std::fmt;

use nuxut::i18n::t;

/// Değiştirici bitleri (Hyprland'in `stringToModMask` sırası değil; yalnızca karşılaştırma).
pub const MOD_SHIFT: u8 = 1;
pub const MOD_CTRL: u8 = 2;
pub const MOD_ALT: u8 = 4;
pub const MOD_SUPER: u8 = 8;
pub const MOD_OTHER: u8 = 16; // MOD2/MOD3/MOD5/CAPS: nadir; yine de ayırt edilir

const MOD_NAMES: [(u8, &str); 4] = [(MOD_SUPER, "SUPER"), (MOD_CTRL, "CTRL"), (MOD_ALT, "ALT"), (MOD_SHIFT, "SHIFT")];

/// Tuş adı sınırı.
const MAX_KEY_LEN: usize = 32;

/// Rakam satırının evdev+8 tuş kodları (1…9, 0): ABD, Türkçe Q/F, AZERTY… hepsinde aynı fiziksel
/// tuşlar. Hyprland `code:<n>` bind'ini düzenden bağımsız eşler (keysym bind'i ise ilk düzenin
/// değiştiricisiz sembolüyle: AZERTY'de `1` değil `ampersand`).
pub const DIGIT_CODES: [u16; 10] = [10, 11, 12, 13, 14, 15, 16, 17, 18, 19];

/// Normalleştirilmiş tuş birleşimi: değiştirici maskesi + küçük harf tuş adı
/// (xkb keysym adları büyük/küçük harfe duyarsız aranır). Dokunma ve rakam ailesi eylemlerinde
/// ayar değeri yalnızca değiştiricidir: `key` boştur ([`Binding::expand`] somut tuşları verir).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Combo {
    pub mods: u8,
    pub key: String,
}

impl fmt::Display for Combo {
    /// `SUPER+SHIFT+n`, yalnız değiştiricide `SUPER+SHIFT` (Ayarlar'da gösterim ve raporlar için).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (bit, name) in MOD_NAMES {
            if self.mods & bit != 0 {
                if !first {
                    f.write_str("+")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        if !self.key.is_empty() {
            if !first {
                f.write_str("+")?;
            }
            f.write_str(&self.key)?;
        }
        Ok(())
    }
}

impl Combo {
    /// Hyprland bind'inin ilk iki alanı: `SUPER SHIFT, n`.
    pub fn to_hypr(&self) -> String {
        format!("{}, {}", self.mod_words().join(" "), self.key)
    }

    /// Lua `hl.bind` tuş dizgesi: `SUPER + SHIFT + n` (değiştiricisiz: `xf86audiomute`).
    pub fn to_lua(&self) -> String {
        let mut parts = self.mod_words();
        parts.push(&self.key);
        parts.join(" + ")
    }

    fn mod_words(&self) -> Vec<&'static str> {
        MOD_NAMES.iter().filter(|(b, _)| self.mods & b != 0).map(|(_, n)| *n).collect()
    }

    /// Çakışma karşılaştırmasının biçimi: aynı fiziksel tuşa giden eş adlar birleşir —
    /// rakam satırı kodu (`code:10`) ↔ ABD/Türkçe Q'daki keysym'i (`1`), `prior` ↔ `page_up`,
    /// `next` ↔ `page_down`, `enter` ↔ `return`.
    pub fn canonical(&self) -> Combo {
        let key = match self.key.as_str() {
            "prior" => "page_up".to_owned(),
            "next" => "page_down".to_owned(),
            "enter" => "return".to_owned(),
            k => match k.strip_prefix("code:").and_then(|c| c.parse::<u16>().ok()) {
                Some(c) => match DIGIT_CODES.iter().position(|d| *d == c) {
                    Some(i) => ((i + 1) % 10).to_string(),
                    None => k.to_owned(),
                },
                None => k.to_owned(),
            },
        };
        Combo { mods: self.mods, key }
    }

    /// Değiştirici tuşunun kendisi mi (`super_l`, `alt_r`…)? Dokunma bind'lerinde maske Hyprland'de
    /// basışta/bırakışta farklı görüldüğünden çakışmada maske yok sayılır.
    fn is_modifier_key(&self) -> bool {
        MODIFIER_KEYS.iter().any(|(_, keys)| keys.contains(&self.key.as_str()))
    }

    /// İki birleşim Hyprland'de aynı tuşu tetikler mi (aynı anda ikisi de çalışırdı)?
    pub fn clashes(&self, other: &Combo) -> bool {
        let (a, b) = (self.canonical(), other.canonical());
        a.key == b.key && (a.mods == b.mods || a.is_modifier_key())
    }
}

/// Dokunma eylemlerinin değiştirici → sol/sağ tuş adları (xkb keysym, küçük harf).
const MODIFIER_KEYS: [(u8, [&str; 2]); 4] = [
    (MOD_SUPER, ["super_l", "super_r"]),
    (MOD_CTRL, ["control_l", "control_r"]),
    (MOD_ALT, ["alt_l", "alt_r"]),
    (MOD_SHIFT, ["shift_l", "shift_r"]),
];

fn valid_key(key: &str) -> bool {
    if let Some(code) = key.strip_prefix("code:") {
        return !code.is_empty() && code.len() <= 4 && code.bytes().all(|b| b.is_ascii_digit());
    }
    if let Some(button) = key.strip_prefix("mouse:") {
        return !button.is_empty() && button.len() <= 4 && button.bytes().all(|b| b.is_ascii_digit());
    }
    !key.is_empty() && key.len() <= MAX_KEY_LEN && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Hyprland'in değiştirici dizgesi: büyük harfe çevrilip **içerme** ile aranır
/// (`SUPER_SHIFT`, `SUPER SHIFT`, `SUPERSHIFT` aynı).
fn hypr_mod_mask(mods: &str) -> u8 {
    let m = mods.to_ascii_uppercase();
    let mut mask = 0;
    if m.contains("SHIFT") {
        mask |= MOD_SHIFT;
    }
    if m.contains("CTRL") || m.contains("CONTROL") {
        mask |= MOD_CTRL;
    }
    if m.contains("ALT") || m.contains("MOD1") {
        mask |= MOD_ALT;
    }
    if ["SUPER", "WIN", "LOGO", "MOD4", "META"].iter().any(|n| m.contains(n)) {
        mask |= MOD_SUPER;
    }
    if ["CAPS", "MOD2", "MOD3", "MOD5"].iter().any(|n| m.contains(n)) {
        mask |= MOD_OTHER;
    }
    mask
}

/// Kullanıcının Hyprland bind'indeki (değiştiriciler, tuş) alanları. Tuş geçersizse `None`.
pub fn parse_hypr_combo(mods: &str, key: &str) -> Option<Combo> {
    let key = key.trim().to_ascii_lowercase();
    valid_key(&key).then(|| Combo { mods: hypr_mod_mask(mods), key })
}

fn setting_mod(word: &str) -> Option<u8> {
    match word.to_ascii_uppercase().as_str() {
        "SUPER" | "WIN" | "META" => Some(MOD_SUPER),
        "CTRL" | "CONTROL" => Some(MOD_CTRL),
        "ALT" => Some(MOD_ALT),
        "SHIFT" => Some(MOD_SHIFT),
        _ => None,
    }
}

/// `shell.toml` tuş dizgesi (`SUPER+SHIFT+N`, `XF86AudioMute`, `CTRL+code:10`). Katı:
/// bilinmeyen değiştirici ya da geçersiz tuş hata; tuş yerinde değiştirici adı da hata
/// (yalnız değiştirici yalnızca dokunma/rakam ailesi eylemlerinde: [`parse_action_combo`]).
pub fn parse_setting_combo(text: &str) -> Result<Combo, String> {
    let combo = parse_parts(text)?;
    if combo.key.is_empty() {
        return Err("tuş eksik (yalnız değiştirici)".into());
    }
    Ok(combo)
}

/// Değiştiriciler + (varsa) tuş. Son parça bir değiştirici adıysa tuş boştur.
fn parse_parts(text: &str) -> Result<Combo, String> {
    let text = text.trim();
    if text.len() > 96 {
        return Err("kısayol çok uzun".into());
    }
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (last, mods) = parts.split_last().ok_or_else(|| "boş kısayol".to_owned())?;
    let mut mask = 0;
    for m in mods {
        mask |= setting_mod(m).ok_or_else(|| format!("bilinmeyen değiştirici '{}'", sanitize(m)))?;
    }
    if let Some(bit) = setting_mod(last) {
        return Ok(Combo { mods: mask | bit, key: String::new() });
    }
    let key = last.to_ascii_lowercase();
    if !valid_key(&key) {
        return Err(format!("geçersiz tuş '{}'", sanitize(&key)));
    }
    Ok(Combo { mods: mask, key })
}

/// Eylemin biçimine göre ayar değeri.
pub fn parse_action_combo(action: Action, text: &str) -> Result<Combo, String> {
    match action.shape() {
        Shape::Key => parse_setting_combo(text),
        Shape::Tap => {
            let c = parse_parts(text)?;
            if !c.key.is_empty() || c.mods.count_ones() != 1 {
                return Err("tek bir değiştirici olmalı (ör. SUPER)".into());
            }
            Ok(c)
        }
        Shape::Digits => {
            let c = parse_parts(text)?;
            if !c.key.is_empty() || c.mods == 0 {
                return Err("yalnız değiştirici olmalı (ör. SUPER+SHIFT); tuşlar 1…0".into());
            }
            Ok(c)
        }
    }
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(32).collect()
}

/// Hyprland bind türü (bayraklar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindKind {
    /// `bind`
    Plain,
    /// `bindl`: kilit ekranında da.
    Locked,
    /// `bindel`: kilitte de, basılı tutunca tekrarlar.
    LockedRepeat,
    /// `binde`: basılı tutunca tekrarlar.
    Repeat,
    /// `bindr`: bırakılınca (değiştiriciye dokunma; araya başka tuş/düğme girerse Hyprland iptal eder).
    Release,
    /// `bindm`: fareyle sürükleme (taşı/boyutlandır).
    Mouse,
}

impl BindKind {
    /// hyprlang anahtar sözcüğünün bayrakları (açıklama bayrağı `d` hariç).
    pub fn flags(self) -> &'static str {
        match self {
            BindKind::Plain => "",
            BindKind::Locked => "l",
            BindKind::LockedRepeat => "el",
            BindKind::Repeat => "e",
            BindKind::Release => "r",
            BindKind::Mouse => "m",
        }
    }
}

/// Eylem değerinin biçimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `MOD+…+Tuş`.
    Key,
    /// Tek değiştiriciye dokunup bırakma (`SUPER` → Super_L ve Super_R).
    Tap,
    /// Değiştirici + rakam satırı 1…0 (on bind; tuş kodlarıyla).
    Digits,
}

/// Yön.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    /// hyprlang `movefocus`/`movewindow` harfi.
    pub fn letter(self) -> &'static str {
        match self {
            Dir::Left => "l",
            Dir::Right => "r",
            Dir::Up => "u",
            Dir::Down => "d",
        }
    }

    /// Lua `direction` değeri.
    pub fn word(self) -> &'static str {
        match self {
            Dir::Left => "left",
            Dir::Right => "right",
            Dir::Up => "up",
            Dir::Down => "down",
        }
    }
}

/// Klavyeyle boyutlandırmanın adımı (mantıksal px).
pub const RESIZE_STEP: i32 = 40;

/// Özel (karalama) çalışma alanının adı.
pub const SCRATCHPAD: &str = "scratchpad";

/// Compositor eylemi (Hyprland dispatcher'ı). İki arka uçta ([`super::base`]) birebir karşılığı var.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    Close,
    /// Gerçek tam ekran (bar dahil her şeyin üstünde).
    Fullscreen,
    /// Ekranı kapla (boşluklar ve bar kalır).
    Maximize,
    ToggleFloating,
    /// Sabitle: her çalışma alanında görünür (yalnızca yüzen pencere).
    Pin,
    Focus(Dir),
    Move(Dir),
    /// Etkin pencereyi (dx, dy) kadar büyüt/küçült (göreli).
    Resize(i32, i32),
    /// Rakam ailesi: alan `n` (bind başına ayrı; [`Binding::expand`]).
    Workspace,
    MoveToWorkspace,
    /// Komşu çalışma alanı (`r±1`: boş olanlar dahil, kimlik sırasıyla).
    WorkspaceStep(i8),
    MoveToWorkspaceStep(i8),
    /// Açık çalışma alanları arasında (`e±1`; fare tekerleği).
    WorkspaceScroll(i8),
    /// Bir önceki çalışma alanı.
    WorkspacePrevious,
    /// Çalışma alanındaki pencereler arasında (true = sonraki).
    Cycle(bool),
    ToggleScratchpad,
    MoveToScratchpad,
    /// dwindle bölme yönünü çevir.
    ToggleSplit,
    /// Fareyle taşı / boyutlandır (`bindm`).
    MouseMove,
    MouseResize,
}

/// Komutun hedefi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// `nuxut-shell <argümanlar>` (kurulu ikilinin mutlak yolu ile).
    Shell(&'static str),
    /// `playerctl <argümanlar>` (MPRIS; shell'den geçmez).
    Playerctl(&'static str),
    /// `nuxut-shell <argümanlar> || <yedek>`: kabuk çalışmıyorsa yedek komut (kilit: `msg lock`, yoksa
    /// `loginctl lock-session` — logind üzerinden, `nuxut-lock` yine açılır).
    ShellOr(&'static str, &'static str),
    /// `hyprctl <argümanlar>` (Hyprland'in dispatcher olmayan komutları; `kill` = tıklama kipi).
    Hyprctl(&'static str),
    /// Ayarlar uygulaması (kabukla aynı dizindeki `nuxut-settings`).
    Settings,
    /// Hyprland dispatcher'ı.
    Dispatch(Dispatch),
}

macro_rules! actions {
    ($( $variant:ident = $name:literal, $default:literal, $kind:ident, $shape:ident, $target:expr; )*) => {
        /// Kısayol atanabilen eylemler (kapalı küme). Sıra: Ayarlar'daki sıra ve aynı tuş iki
        /// eyleme verildiğinde hangisinin kalacağı (ilk).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Action { $($variant),* }

        impl Action {
            pub const ALL: &'static [Action] = &[$(Action::$variant),*];

            /// `shell.toml` `[keybinds]` anahtarı.
            pub fn name(self) -> &'static str {
                match self { $(Action::$variant => $name),* }
            }

            /// Varsayılan tuş (`""` = atanmamış).
            pub fn default_combo(self) -> &'static str {
                match self { $(Action::$variant => $default),* }
            }

            pub fn kind(self) -> BindKind {
                match self { $(Action::$variant => BindKind::$kind),* }
            }

            pub fn shape(self) -> Shape {
                match self { $(Action::$variant => Shape::$shape),* }
            }

            pub fn target(self) -> Target {
                match self { $(Action::$variant => $target),* }
            }
        }
    };
}

use Dispatch as D;

actions! {
    VolumeUp = "volume_up", "XF86AudioRaiseVolume", LockedRepeat, Key, Target::Shell("msg volume up");
    VolumeDown = "volume_down", "XF86AudioLowerVolume", LockedRepeat, Key, Target::Shell("msg volume down");
    VolumeMute = "volume_mute", "XF86AudioMute", Locked, Key, Target::Shell("msg volume mute");
    MicMute = "mic_mute", "XF86AudioMicMute", Locked, Key, Target::Shell("msg mic mute");
    BrightnessUp = "brightness_up", "XF86MonBrightnessUp", LockedRepeat, Key, Target::Shell("msg brightness up");
    BrightnessDown = "brightness_down", "XF86MonBrightnessDown", LockedRepeat, Key, Target::Shell("msg brightness down");
    MediaPlay = "media_play", "XF86AudioPlay", Locked, Key, Target::Playerctl("play-pause");
    // Ayrı duraklat tuşu (bazı klavyeler, Bluetooth kulaklıklar): durumu tutmayan aygıt yanlış
    // tuşu gönderebildiği için o da oynat/duraklat (dots-hyprland ile aynı; GNOME yalnız duraklatır).
    MediaPause = "media_pause", "XF86AudioPause", Locked, Key, Target::Playerctl("play-pause");
    MediaNext = "media_next", "XF86AudioNext", Locked, Key, Target::Playerctl("next");
    MediaPrev = "media_prev", "XF86AudioPrev", Locked, Key, Target::Playerctl("previous");
    PanelRight = "panel_right", "", Plain, Key, Target::Shell("msg panel toggle right");
    PanelLeft = "panel_left", "", Plain, Key, Target::Shell("msg panel toggle left");
    PanelClock = "panel_clock", "", Plain, Key, Target::Shell("msg panel toggle clock");
    BarToggle = "bar_toggle", "", Plain, Key, Target::Shell("msg bar toggle");
    // Bar'da klavye gezinmesi (ekran okuyucu; 2026-10-17, docs/design/screen-reader.md). GNOME'un
    // "sistem denetimlerine geç" kısayolu: ekran okuyucu kullanıcısının alışkanlığı.
    BarFocus = "bar_focus", "CTRL+ALT+Tab", Plain, Key, Target::Shell("msg bar focus");
    ShotRegion = "shot_region", "Print", Plain, Key, Target::Shell("msg shot region");
    ShotOutput = "shot_output", "SHIFT+Print", Plain, Key, Target::Shell("msg shot output");
    ShotWindow = "shot_window", "ALT+Print", Plain, Key, Target::Shell("msg shot window");
    Clipboard = "clipboard", "SUPER+SHIFT+V", Plain, Key, Target::Shell("msg launcher clipboard");
    // Büyüteç (t124; klavye düzeninden bağımsız tuşlar — 2026-10-14). Kilit
    // ekranında da çalışır (parola alanını büyütmek erişilebilirlik gereği).
    MagnifierIn = "magnifier_in", "SUPER+ALT+Up", LockedRepeat, Key, Target::Shell("msg magnifier in");
    MagnifierOut = "magnifier_out", "SUPER+ALT+Down", LockedRepeat, Key, Target::Shell("msg magnifier out");
    MagnifierReset = "magnifier_reset", "SUPER+ALT+Escape", Locked, Key, Target::Shell("msg magnifier reset");
    // Başlatıcının emoji kipi; SUPER+. (Windows emoji paneli alışkanlığı). Karar:
    // 2026-10-13, kullanıcı onayına açık (docs/design/emoji.md).
    Emoji = "emoji", "SUPER+period", Plain, Key, Target::Shell("msg launcher emoji");
    // Ekran klavyesi (t125; docs/design/osk.md "Kısayol"). Kilit ekranında v1 çalışmaz: Plain.
    OskToggle = "osk_toggle", "SUPER+ALT+K", Plain, Key, Target::Shell("msg osk toggle");
    // Temel katman (kullanıcı kararı 2026-10-14; docs/design/hyprland-base.md §3). Harfler keysym
    // (tuş üstündeki harf), rakamlar tuş kodu, oklar/sayfa/Tab/Return düzenden bağımsız keysym.
    Launcher = "launcher", "SUPER", Release, Tap, Target::Shell("msg launcher toggle");
    Settings = "settings", "SUPER+I", Plain, Key, Target::Settings;
    Terminal = "terminal", "CTRL+ALT+T", Plain, Key, Target::Shell("msg defaults open terminal");
    FileManager = "file_manager", "SUPER+E", Plain, Key, Target::Shell("msg defaults open file_manager");
    Browser = "browser", "SUPER+B", Plain, Key, Target::Shell("msg defaults open browser");
    Lock = "lock", "SUPER+L", Plain, Key, Target::ShellOr("msg lock", "loginctl lock-session");
    SessionMenu = "session_menu", "CTRL+ALT+Delete", Plain, Key, Target::Shell("msg session menu");
    CloseWindow = "close_window", "SUPER+Q", Plain, Key, Target::Dispatch(D::Close);
    CloseWindowAlt = "close_window_alt", "ALT+F4", Plain, Key, Target::Dispatch(D::Close);
    // Yanıt vermeyen pencere: tıklanan pencere zorla sonlandırılır (Hyprland tıklama kipi; Esc
    // vazgeçer). KDE ile aynı tuş; tek tuşla odaktakini öldürmek kayıp riskli.
    ForceKill = "force_kill", "CTRL+ALT+Escape", Plain, Key, Target::Hyprctl("kill");
    Fullscreen = "fullscreen", "SUPER+F", Plain, Key, Target::Dispatch(D::Fullscreen);
    Maximize = "maximize", "SUPER+M", Plain, Key, Target::Dispatch(D::Maximize);
    ToggleFloating = "toggle_floating", "SUPER+SHIFT+space", Plain, Key, Target::Dispatch(D::ToggleFloating);
    Pin = "pin", "SUPER+P", Plain, Key, Target::Dispatch(D::Pin);
    CycleWindows = "cycle_windows", "ALT+Tab", Plain, Key, Target::Dispatch(D::Cycle(true));
    CycleWindowsBack = "cycle_windows_back", "ALT+SHIFT+Tab", Plain, Key, Target::Dispatch(D::Cycle(false));
    ToggleSplit = "toggle_split", "SUPER+J", Plain, Key, Target::Dispatch(D::ToggleSplit);
    FocusLeft = "focus_left", "SUPER+Left", Plain, Key, Target::Dispatch(D::Focus(Dir::Left));
    FocusRight = "focus_right", "SUPER+Right", Plain, Key, Target::Dispatch(D::Focus(Dir::Right));
    FocusUp = "focus_up", "SUPER+Up", Plain, Key, Target::Dispatch(D::Focus(Dir::Up));
    FocusDown = "focus_down", "SUPER+Down", Plain, Key, Target::Dispatch(D::Focus(Dir::Down));
    MoveLeft = "move_left", "SUPER+SHIFT+Left", Plain, Key, Target::Dispatch(D::Move(Dir::Left));
    MoveRight = "move_right", "SUPER+SHIFT+Right", Plain, Key, Target::Dispatch(D::Move(Dir::Right));
    MoveUp = "move_up", "SUPER+SHIFT+Up", Plain, Key, Target::Dispatch(D::Move(Dir::Up));
    MoveDown = "move_down", "SUPER+SHIFT+Down", Plain, Key, Target::Dispatch(D::Move(Dir::Down));
    ShrinkWidth = "shrink_width", "SUPER+CTRL+Left", Repeat, Key, Target::Dispatch(D::Resize(-RESIZE_STEP, 0));
    GrowWidth = "grow_width", "SUPER+CTRL+Right", Repeat, Key, Target::Dispatch(D::Resize(RESIZE_STEP, 0));
    ShrinkHeight = "shrink_height", "SUPER+CTRL+Up", Repeat, Key, Target::Dispatch(D::Resize(0, -RESIZE_STEP));
    GrowHeight = "grow_height", "SUPER+CTRL+Down", Repeat, Key, Target::Dispatch(D::Resize(0, RESIZE_STEP));
    Workspace = "workspace", "SUPER", Plain, Digits, Target::Dispatch(D::Workspace);
    MoveToWorkspace = "move_to_workspace", "SUPER+SHIFT", Plain, Digits, Target::Dispatch(D::MoveToWorkspace);
    WorkspaceNext = "workspace_next", "SUPER+Page_Down", Plain, Key, Target::Dispatch(D::WorkspaceStep(1));
    WorkspacePrev = "workspace_prev", "SUPER+Page_Up", Plain, Key, Target::Dispatch(D::WorkspaceStep(-1));
    MoveToWorkspaceNext = "move_to_workspace_next", "SUPER+SHIFT+Page_Down", Plain, Key, Target::Dispatch(D::MoveToWorkspaceStep(1));
    MoveToWorkspacePrev = "move_to_workspace_prev", "SUPER+SHIFT+Page_Up", Plain, Key, Target::Dispatch(D::MoveToWorkspaceStep(-1));
    WorkspaceLast = "workspace_last", "SUPER+Tab", Plain, Key, Target::Dispatch(D::WorkspacePrevious);
    Scratchpad = "scratchpad", "SUPER+S", Plain, Key, Target::Dispatch(D::ToggleScratchpad);
    MoveToScratchpad = "move_to_scratchpad", "SUPER+SHIFT+S", Plain, Key, Target::Dispatch(D::MoveToScratchpad);
    WindowDrag = "window_drag", "SUPER+mouse:272", Mouse, Key, Target::Dispatch(D::MouseMove);
    WindowResizeDrag = "window_resize_drag", "SUPER+mouse:273", Mouse, Key, Target::Dispatch(D::MouseResize);
    WorkspaceScrollNext = "workspace_scroll_next", "SUPER+mouse_down", Plain, Key, Target::Dispatch(D::WorkspaceScroll(1));
    WorkspaceScrollPrev = "workspace_scroll_prev", "SUPER+mouse_up", Plain, Key, Target::Dispatch(D::WorkspaceScroll(-1));
}

impl Action {
    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }

    /// Temel katman eylemi mi (pencere/çalışma alanı yönetimi, uygulama açma)? `[compositor]
    /// base_layer = false` iken üretilmez; kabuk eylemleri (ses, panel, büyüteç…) her zaman.
    pub fn is_base(self) -> bool {
        self >= Action::Launcher
    }

    /// Ayarlar'da görünen ad (katalogdan).
    pub fn title(self) -> String {
        match self {
            Action::VolumeUp => t!("shell-keybinds-volume-up-label"),
            Action::VolumeDown => t!("shell-keybinds-volume-down-label"),
            Action::VolumeMute => t!("shell-keybinds-volume-mute-label"),
            Action::MicMute => t!("shell-keybinds-mic-mute-label"),
            Action::BrightnessUp => t!("shell-keybinds-brightness-up-label"),
            Action::BrightnessDown => t!("shell-keybinds-brightness-down-label"),
            Action::MediaPlay => t!("shell-keybinds-media-play-label"),
            Action::MediaPause => t!("shell-keybinds-media-pause-label"),
            Action::MediaNext => t!("shell-keybinds-media-next-label"),
            Action::MediaPrev => t!("shell-keybinds-media-prev-label"),
            Action::PanelRight => t!("shell-keybinds-panel-right-label"),
            Action::PanelLeft => t!("shell-keybinds-panel-left-label"),
            Action::PanelClock => t!("shell-keybinds-panel-clock-label"),
            Action::BarToggle => t!("shell-keybinds-bar-toggle-label"),
            Action::BarFocus => t!("shell-keybinds-bar-focus-label"),
            Action::ShotRegion => t!("shell-keybinds-shot-region-label"),
            Action::ShotOutput => t!("shell-keybinds-shot-output-label"),
            Action::ShotWindow => t!("shell-keybinds-shot-window-label"),
            Action::Clipboard => t!("shell-keybinds-clipboard-label"),
            Action::MagnifierIn => t!("shell-keybinds-magnifier-in-label"),
            Action::MagnifierOut => t!("shell-keybinds-magnifier-out-label"),
            Action::MagnifierReset => t!("shell-keybinds-magnifier-reset-label"),
            Action::Emoji => t!("shell-keybinds-emoji-label"),
            Action::OskToggle => t!("shell-keybinds-osk-toggle-label"),
            Action::Launcher => t!("shell-keybinds-launcher-label"),
            Action::Lock => t!("shell-keybinds-lock-label"),
            Action::SessionMenu => t!("shell-keybinds-session-menu-label"),
            Action::Settings => t!("shell-keybinds-settings-label"),
            Action::Terminal => t!("shell-keybinds-terminal-label"),
            Action::FileManager => t!("shell-keybinds-file-manager-label"),
            Action::Browser => t!("shell-keybinds-browser-label"),
            Action::CloseWindow => t!("shell-keybinds-close-window-label"),
            Action::CloseWindowAlt => t!("shell-keybinds-close-window-alt-label"),
            Action::ForceKill => t!("shell-keybinds-force-kill-label"),
            Action::Fullscreen => t!("shell-keybinds-fullscreen-label"),
            Action::Maximize => t!("shell-keybinds-maximize-label"),
            Action::ToggleFloating => t!("shell-keybinds-toggle-floating-label"),
            Action::Pin => t!("shell-keybinds-pin-label"),
            Action::FocusLeft => t!("shell-keybinds-focus-left-label"),
            Action::FocusRight => t!("shell-keybinds-focus-right-label"),
            Action::FocusUp => t!("shell-keybinds-focus-up-label"),
            Action::FocusDown => t!("shell-keybinds-focus-down-label"),
            Action::MoveLeft => t!("shell-keybinds-move-left-label"),
            Action::MoveRight => t!("shell-keybinds-move-right-label"),
            Action::MoveUp => t!("shell-keybinds-move-up-label"),
            Action::MoveDown => t!("shell-keybinds-move-down-label"),
            Action::ShrinkWidth => t!("shell-keybinds-shrink-width-label"),
            Action::GrowWidth => t!("shell-keybinds-grow-width-label"),
            Action::ShrinkHeight => t!("shell-keybinds-shrink-height-label"),
            Action::GrowHeight => t!("shell-keybinds-grow-height-label"),
            Action::Workspace => t!("shell-keybinds-workspace-label"),
            Action::MoveToWorkspace => t!("shell-keybinds-move-to-workspace-label"),
            Action::WorkspaceNext => t!("shell-keybinds-workspace-next-label"),
            Action::WorkspacePrev => t!("shell-keybinds-workspace-prev-label"),
            Action::MoveToWorkspaceNext => t!("shell-keybinds-move-to-workspace-next-label"),
            Action::MoveToWorkspacePrev => t!("shell-keybinds-move-to-workspace-prev-label"),
            Action::WorkspaceLast => t!("shell-keybinds-workspace-last-label"),
            Action::CycleWindows => t!("shell-keybinds-cycle-windows-label"),
            Action::CycleWindowsBack => t!("shell-keybinds-cycle-windows-back-label"),
            Action::Scratchpad => t!("shell-keybinds-scratchpad-label"),
            Action::MoveToScratchpad => t!("shell-keybinds-move-to-scratchpad-label"),
            Action::ToggleSplit => t!("shell-keybinds-toggle-split-label"),
            Action::WindowDrag => t!("shell-keybinds-window-drag-label"),
            Action::WindowResizeDrag => t!("shell-keybinds-window-resize-drag-label"),
            Action::WorkspaceScrollNext => t!("shell-keybinds-workspace-scroll-next-label"),
            Action::WorkspaceScrollPrev => t!("shell-keybinds-workspace-scroll-prev-label"),
        }
    }
}

/// Atanmış eylem. `combo` ayar değeridir (dokunma/rakam ailesinde yalnız değiştirici).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub action: Action,
    pub combo: Combo,
}

/// Üretilecek tek bir Hyprland bind'i: somut tuş + (rakam ailesinde) çalışma alanı numarası.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Concrete {
    pub combo: Combo,
    /// Rakam ailesi: 1…10.
    pub index: Option<u8>,
}

impl Binding {
    /// Hyprland'e gidecek somut bind'ler: tek tuş → bir; dokunma → sol + sağ değiştirici;
    /// rakam ailesi → 1…0 (tuş kodlarıyla).
    pub fn expand(&self) -> Vec<Concrete> {
        match self.action.shape() {
            Shape::Key => vec![Concrete { combo: self.combo.clone(), index: None }],
            Shape::Tap => MODIFIER_KEYS
                .iter()
                .filter(|(bit, _)| self.combo.mods == *bit)
                .flat_map(|(bit, keys)| keys.iter().map(move |k| Concrete { combo: Combo { mods: *bit, key: (*k).to_owned() }, index: None }))
                .collect(),
            Shape::Digits => DIGIT_CODES
                .iter()
                .enumerate()
                .map(|(i, code)| Concrete { combo: Combo { mods: self.combo.mods, key: format!("code:{code}") }, index: Some(i as u8 + 1) })
                .collect(),
        }
    }

    /// Bu eylemin herhangi bir somut bind'i ötekininkiyle çakışıyor mu?
    pub fn clashes(&self, other: &Binding) -> bool {
        let theirs = other.expand();
        self.expand().iter().any(|a| theirs.iter().any(|b| a.combo.clashes(&b.combo)))
    }
}

/// `[keybinds]` tablosunu çözer: verilmeyen eylem varsayılanını alır, `""` atanmamış;
/// bilinmeyen eylem ve geçersiz tuş uyarıyla atlanır. Aynı tuş ikinci kez atanırsa ikincisi
/// (eylem sırasıyla) atılır.
pub fn resolve_settings(table: &BTreeMap<String, String>) -> (Vec<Binding>, Vec<String>) {
    let mut warnings = Vec::new();
    for name in table.keys() {
        if Action::from_name(name).is_none() {
            warnings.push(format!("keybinds.{}: bilinmeyen eylem, yok sayıldı", sanitize(name)));
        }
    }
    let mut out: Vec<Binding> = Vec::new();
    for &action in Action::ALL {
        let text = table.get(action.name()).map_or(action.default_combo(), String::as_str);
        if text.trim().is_empty() {
            continue;
        }
        match parse_action_combo(action, text) {
            Ok(combo) => {
                let b = Binding { action, combo };
                if let Some(other) = out.iter().find(|o| o.clashes(&b)) {
                    warnings.push(format!("keybinds.{}: {} zaten {} için atanmış, yok sayıldı", action.name(), b.combo, other.action.name()));
                } else {
                    out.push(b);
                }
            }
            Err(e) => warnings.push(format!("keybinds.{}: {e}", action.name())),
        }
    }
    (out, warnings)
}

/// Çakışan tuşta kimin bind'i çalışsın (`shell.toml` `[keybind_owner] <eylem> = "nuxut"|"user"`;
/// Ayarlar › Kısayollar'daki "Nuxut'unkini kullan" / "Benimkini koru", 2026-10-17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOwner {
    Nuxut,
    User,
}

impl KeyOwner {
    pub fn name(self) -> &'static str {
        match self {
            KeyOwner::Nuxut => "nuxut",
            KeyOwner::User => "user",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "nuxut" => Some(KeyOwner::Nuxut),
            "user" => Some(KeyOwner::User),
            _ => None,
        }
    }
}

/// Bir eylem için verilen karar: kimin bind'i ve bu karar kullanıcıdan mı geldi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPref {
    pub owner: KeyOwner,
    /// `[keybind_owner]`'da açıkça seçildi ya da `[compositor] take_over_keys = false` (kullanıcının
    /// genel seçimi). `false`: Nuxut'un varsayılanı (kabuk eylemi → Nuxut, temel katman → kullanıcı).
    pub chosen: bool,
}

impl KeyPref {
    /// Eylemin kararı: açık seçim > `take_over_keys` (yalnızca kabuk eylemleri) > varsayılan.
    /// Temel katman eylemlerinde varsayılan olarak kullanıcınınki kazanır (ADR-0006).
    pub fn resolve(action: Action, explicit: Option<KeyOwner>, take_over_keys: bool) -> Self {
        match explicit {
            Some(owner) => KeyPref { owner, chosen: true },
            None if action.is_base() => KeyPref { owner: KeyOwner::User, chosen: false },
            None if take_over_keys => KeyPref { owner: KeyOwner::Nuxut, chosen: false },
            None => KeyPref { owner: KeyOwner::User, chosen: true },
        }
    }
}

/// Kullanıcının bind'i neden `unbind` ile devralınamıyor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    /// Nuxut'un ayar katmanından (`nuxut.conf`) SONRA okunuyor: Nuxut'unkini yeniden bağlardı.
    ReadAfter,
    /// `nuxut.conf` yüklenmiyor (dahil etme satırı yok/kapalı).
    NotLoaded,
    /// CAPS/MOD2… değiştiricisi: `unbind` ile güvenle yazılamıyor.
    SpecialMods,
    /// Yeri bilinmiyor (Lua ayarı: bind'ler çalışan Hyprland'den okunur).
    Unlocated,
}

/// Çakışmanın nedeni (`msg hypr status` `conflicts[].reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReason {
    /// Temel katman eylemi: kullanıcınınki bilerek kazanır (ADR-0006). Sorun değil, bilgi.
    UserWins,
    /// Kullanıcı kendi bind'ini seçti (`[keybind_owner]` ya da `take_over_keys = false`). Bilgi.
    UserChoice,
    /// Nuxut'unki isteniyor ama devralınamıyor: tuş hiçbirinde düzgün çalışmayabilir. Uyarı.
    Blocked(Blocker),
}

impl ConflictReason {
    /// `msg hypr status` adı.
    pub fn name(self) -> &'static str {
        match self {
            ConflictReason::UserWins => "user_wins",
            ConflictReason::UserChoice => "user_choice",
            ConflictReason::Blocked(Blocker::ReadAfter) => "read_after",
            ConflictReason::Blocked(Blocker::NotLoaded) => "not_loaded",
            ConflictReason::Blocked(Blocker::SpecialMods) => "special_mods",
            ConflictReason::Blocked(Blocker::Unlocated) => "unlocated",
        }
    }

    /// Bilinçli karar mı (doctor uyarı değil bilgi gösterir)?
    pub fn is_intended(self) -> bool {
        matches!(self, ConflictReason::UserWins | ConflictReason::UserChoice)
    }
}

/// Kullanıcının bind'iyle çakıştığı için üretilmeyen Nuxut bind'i.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict<L> {
    pub action: Action,
    /// Çakışan somut tuş (rakam ailesinde ör. `SUPER+code:10`).
    pub combo: Combo,
    /// Kullanıcının bind'inin yeri.
    pub at: L,
    pub reason: ConflictReason,
    /// "Nuxut'unkini kullan" seçilirse devralınabilir mi (çakışan her bind'in engeli yok).
    pub can_take: bool,
}

/// Kullanıcının zincirindeki bir bind (çakışma girdisi).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserKey<L> {
    pub combo: Combo,
    pub at: L,
    /// Nuxut bu bind'i devralamıyorsa nedeni (`None`: ayar katmanından ÖNCE okunuyor ve tuş
    /// `unbind` ile yazılabiliyor; kararı çağıran verir; bkz. [`Taken`]).
    pub blocker: Option<Blocker>,
}

/// Nuxut'un devraldığı kısayol (kullanıcı isteği 2026-10-16, "Nuxut yapabiliyorsa varsayılan
/// Nuxut"): kullanıcının önceki bind'i ayar katmanında `unbind` ile kaldırılır, ardından Nuxut'unki
/// bağlanır. Kullanıcının dosyasına dokunulmaz; `[keybind_owner] <eylem> = "user"` ya da
/// `[compositor] take_over_keys = false` geri alır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken<L> {
    pub binding: Binding,
    /// Kaldırılan kullanıcı tuşları, kullanıcının yazdığı biçimde (tekrarsız).
    pub unbind: Vec<Combo>,
    /// Kullanıcının bind'lerinin yerleri.
    pub at: Vec<L>,
    /// Açık seçimle mi (`[keybind_owner]`) yoksa varsayılanla mı devralındı.
    pub chosen: bool,
}

/// [`split_bindings`] sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split<L> {
    /// Çakışmayanlar (temel katmana).
    pub keep: Vec<Binding>,
    /// Devralınanlar (ayar katmanına, `unbind` ile).
    pub taken: Vec<Taken<L>>,
    /// Üretilmeyenler: kullanıcınınki tek başına çalışır.
    pub conflicts: Vec<Conflict<L>>,
}

/// Kullanıcının bind'leriyle çakışanları ayırır. Bir eylemin somut bind'lerinden biri bile
/// çakışırsa eylem bütün olarak ele alınır (rakam ailesi ya da sol/sağ değiştirici yarım kalmaz).
/// `pref` eylemin kararını verir ([`KeyPref::resolve`]): Nuxut'unki isteniyor ve çakışan her
/// kullanıcı bind'inin engeli yoksa devralınır; yoksa kullanıcınınki kalır (neden raporlanır).
pub fn split_bindings<L: Clone>(bindings: Vec<Binding>, user: &[UserKey<L>], pref: impl Fn(Action) -> KeyPref) -> Split<L> {
    let mut out = Split { keep: Vec::new(), taken: Vec::new(), conflicts: Vec::new() };
    for b in bindings {
        let concrete = b.expand();
        let hits: Vec<(&Combo, &UserKey<L>)> =
            concrete.iter().flat_map(|c| user.iter().filter(|u| c.combo.clashes(&u.combo)).map(move |u| (&c.combo, u))).collect();
        let Some(&(first, first_user)) = hits.first() else {
            out.keep.push(b);
            continue;
        };
        let blocker = hits.iter().find_map(|(_, u)| u.blocker);
        let p = pref(b.action);
        let reason = match (p.owner, blocker) {
            (KeyOwner::Nuxut, None) => {
                let mut unbind: Vec<Combo> = Vec::new();
                for (_, u) in &hits {
                    if !unbind.contains(&u.combo) {
                        unbind.push(u.combo.clone());
                    }
                }
                let at = hits.iter().map(|(_, u)| u.at.clone()).collect();
                out.taken.push(Taken { binding: b, unbind, at, chosen: p.chosen });
                continue;
            }
            (KeyOwner::Nuxut, Some(x)) => ConflictReason::Blocked(x),
            (KeyOwner::User, _) if p.chosen => ConflictReason::UserChoice,
            (KeyOwner::User, _) => ConflictReason::UserWins,
        };
        // Engelli bind'in yeri raporlanır (neden onu gösterir); yoksa ilk çakışan.
        let (combo, at) = hits.iter().find(|(_, u)| u.blocker.is_some()).map_or((first, first_user), |&(c, u)| (c, u));
        out.conflicts.push(Conflict { action: b.action, combo: combo.clone(), at: at.at.clone(), reason, can_take: blocker.is_none() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Devralmasız ayrım: (üretilecekler, çakışmalar). Kullanıcınınki seçili.
    fn split_conflicts<L: Clone>(bindings: Vec<Binding>, user: &[(Combo, L)]) -> (Vec<Binding>, Vec<Conflict<L>>) {
        let user: Vec<UserKey<L>> = user.iter().map(|(combo, at)| UserKey { combo: combo.clone(), at: at.clone(), blocker: None }).collect();
        let s = split_bindings(bindings, &user, |a| KeyPref::resolve(a, None, false));
        assert!(s.taken.is_empty());
        (s.keep, s.conflicts)
    }

    /// Kabuk eylemleri devralınır (kullanıcınınki `unbind`), temel katman eylemleri devralınmaz;
    /// devralınamayan tek bir bind bile eylemi çakışmaya düşürür (çift tetikleme yok).
    #[test]
    fn shell_actions_take_over_earlier_user_binds() {
        let (b, _) = resolve_settings(&BTreeMap::new());
        let key = |m: &str, k: &str, at: usize, blocker: Option<Blocker>| UserKey { combo: parse_hypr_combo(m, k).unwrap(), at, blocker };
        let user = vec![
            key("", "XF86AudioRaiseVolume", 1, None),
            key("SUPER", "period", 2, None),
            key("", "Print", 3, Some(Blocker::ReadAfter)), // ayar katmanından sonra okunuyor
            key("SUPER", "q", 4, None), // temel katman eylemi
            key("", "xf86audioraisevolume", 5, None), // aynı tuş ikinci kez: tek unbind
        ];
        let s = split_bindings(b, &user, |a| KeyPref::resolve(a, None, true));
        let names: Vec<_> = s.taken.iter().map(|t| t.binding.action.name()).collect();
        assert_eq!(names, ["volume_up", "emoji"]);
        assert_eq!(s.taken[0].unbind, vec![parse_hypr_combo("", "xf86audioraisevolume").unwrap()]);
        assert_eq!(s.taken[0].at, vec![1, 5]);
        assert!(!s.taken[0].chosen);
        let dropped: Vec<_> = s.conflicts.iter().map(|c| (c.action.name(), c.reason.name(), c.can_take)).collect();
        assert_eq!(dropped, [("shot_region", "read_after", false), ("close_window", "user_wins", true)]);
        assert!(s.keep.iter().all(|k| !matches!(k.action, Action::VolumeUp | Action::Emoji | Action::ShotRegion | Action::CloseWindow)));
    }

    /// Kararın önceliği: açık seçim > `take_over_keys` > varsayılan (kabuk → Nuxut, temel → kullanıcı).
    #[test]
    fn key_pref_resolution() {
        let r = KeyPref::resolve;
        assert_eq!(r(Action::VolumeUp, None, true), KeyPref { owner: KeyOwner::Nuxut, chosen: false });
        assert_eq!(r(Action::VolumeUp, None, false), KeyPref { owner: KeyOwner::User, chosen: true });
        assert_eq!(r(Action::VolumeUp, Some(KeyOwner::Nuxut), false), KeyPref { owner: KeyOwner::Nuxut, chosen: true });
        assert_eq!(r(Action::CloseWindow, None, true), KeyPref { owner: KeyOwner::User, chosen: false });
        assert_eq!(r(Action::CloseWindow, Some(KeyOwner::Nuxut), true), KeyPref { owner: KeyOwner::Nuxut, chosen: true });
        assert_eq!(r(Action::Emoji, Some(KeyOwner::User), true), KeyPref { owner: KeyOwner::User, chosen: true });
        for o in [KeyOwner::Nuxut, KeyOwner::User] {
            assert_eq!(KeyOwner::from_name(o.name()), Some(o));
        }
        assert_eq!(KeyOwner::from_name("Nuxut"), None);
    }

    /// Temel katman eylemi açıkça Nuxut'a verilince devralınır; kabuk eylemi kullanıcıya bırakılınca
    /// bilgi olarak raporlanır. Engel her durumda devralmayı durdurur ve nedeni taşır.
    #[test]
    fn explicit_owner_moves_base_and_shell_actions() {
        let (b, _) = resolve_settings(&BTreeMap::new());
        let key = |m: &str, k: &str, at: usize, blocker| UserKey { combo: parse_hypr_combo(m, k).unwrap(), at, blocker };
        let user = vec![
            key("SUPER", "q", 1, None),
            key("", "XF86AudioMute", 2, None),
            key("SUPER", "f", 3, Some(Blocker::ReadAfter)),
            key("SUPER CAPS", "e", 4, Some(Blocker::SpecialMods)),
        ];
        let pref = |a: Action| {
            let explicit = match a {
                Action::CloseWindow | Action::Fullscreen => Some(KeyOwner::Nuxut),
                Action::VolumeMute => Some(KeyOwner::User),
                _ => None,
            };
            KeyPref::resolve(a, explicit, true)
        };
        let s = split_bindings(b, &user, pref);
        assert_eq!(s.taken.iter().map(|t| (t.binding.action.name(), t.chosen)).collect::<Vec<_>>(), [("close_window", true)]);
        let c: Vec<_> = s.conflicts.iter().map(|c| (c.action.name(), c.reason.name(), c.reason.is_intended(), c.at)).collect();
        assert_eq!(c, [("volume_mute", "user_choice", true, 2), ("fullscreen", "read_after", false, 3)]);
        // CAPS'lı bind başka tuş birleşimidir: SUPER+E ile çakışmaz.
        assert!(s.keep.iter().any(|k| k.action == Action::FileManager));
    }

    #[test]
    fn hypr_mods_are_substring_matched() {
        let c = parse_hypr_combo("SUPER_SHIFT", "N").unwrap();
        assert_eq!(c, parse_hypr_combo("super shift", "n").unwrap());
        assert_eq!(c.mods, MOD_SUPER | MOD_SHIFT);
        assert_eq!(parse_hypr_combo("", "XF86AudioMute").unwrap().mods, 0);
        assert_eq!(parse_hypr_combo("ALT", "code:10").unwrap().key, "code:10");
        assert!(parse_hypr_combo("SUPER", "a b").is_none());
    }

    #[test]
    fn setting_combo_is_strict() {
        let c = parse_setting_combo("SUPER+Shift+N").unwrap();
        assert_eq!(c.to_string(), "SUPER+SHIFT+n");
        assert_eq!(c.to_hypr(), "SUPER SHIFT, n");
        assert_eq!(c.to_lua(), "SUPER + SHIFT + n");
        assert_eq!(parse_setting_combo("XF86AudioMute").unwrap().to_hypr(), ", xf86audiomute");
        assert_eq!(parse_setting_combo("XF86AudioMute").unwrap().to_lua(), "xf86audiomute");
        assert!(parse_setting_combo("HYPER+N").is_err());
        assert!(parse_setting_combo("SUPER+").is_err());
        assert!(parse_setting_combo("SUPER").is_err(), "yalnız değiştirici tek tuş eyleminde geçersiz");
        assert!(parse_setting_combo("SUPER+N, exec, rm").is_err());
        assert!(parse_setting_combo("SUPER+N;reload").is_err());
        assert!(parse_setting_combo(&"A".repeat(200)).is_err());
    }

    #[test]
    fn tap_and_digit_shapes() {
        assert_eq!(parse_action_combo(Action::Launcher, "super").unwrap(), Combo { mods: MOD_SUPER, key: String::new() });
        assert!(parse_action_combo(Action::Launcher, "SUPER+SHIFT").is_err(), "dokunma tek değiştirici");
        assert!(parse_action_combo(Action::Launcher, "SUPER+L").is_err());
        assert!(parse_action_combo(Action::Workspace, "SUPER+1").is_err(), "aile tuş almaz");
        let ws = parse_action_combo(Action::MoveToWorkspace, "SUPER+SHIFT").unwrap();
        assert_eq!(ws.to_string(), "SUPER+SHIFT");
        let b = Binding { action: Action::MoveToWorkspace, combo: ws };
        let e = b.expand();
        assert_eq!(e.len(), 10);
        assert_eq!(e[0].combo.to_hypr(), "SUPER SHIFT, code:10");
        assert_eq!(e[0].index, Some(1));
        assert_eq!(e[9].combo.key, "code:19");
        assert_eq!(e[9].index, Some(10));
        let tap = Binding { action: Action::Launcher, combo: parse_action_combo(Action::Launcher, "SUPER").unwrap() };
        let keys: Vec<_> = tap.expand().into_iter().map(|c| c.combo.to_lua()).collect();
        assert_eq!(keys, ["SUPER + super_l", "SUPER + super_r"]);
    }

    /// Türkçe Q ve AZERTY: rakam satırı tuş kodu (düzenden bağımsız); ABD/Türkçe Q'da keysym `1`
    /// bind'i aynı fiziksel tuştur → çakışma sayılır (çift tetikleme olmaz).
    #[test]
    fn digit_codes_clash_with_digit_keysyms() {
        let code = Combo { mods: MOD_SUPER, key: "code:10".into() };
        assert!(code.clashes(&parse_hypr_combo("SUPER", "1").unwrap()));
        assert!(Combo { mods: MOD_SUPER, key: "code:19".into() }.clashes(&parse_hypr_combo("SUPER", "0").unwrap()));
        assert!(!code.clashes(&parse_hypr_combo("SUPER SHIFT", "1").unwrap()));
        assert!(parse_hypr_combo("SUPER", "Prior").unwrap().clashes(&parse_setting_combo("SUPER+Page_Up").unwrap()));
        // Dokunma: kullanıcının değiştiricisiz `bindr = , Super_L` bind'i de çakışır.
        let super_l = Combo { mods: MOD_SUPER, key: "super_l".into() };
        assert!(super_l.clashes(&parse_hypr_combo("", "Super_L").unwrap()));
    }

    #[test]
    fn settings_defaults_overrides_and_duplicates() {
        let mut t = BTreeMap::new();
        t.insert("panel_right".to_owned(), "SUPER+N".to_owned());
        t.insert("bar_toggle".to_owned(), "super+n".to_owned()); // panel_right ile aynı
        t.insert("volume_mute".to_owned(), String::new()); // atanmamış
        t.insert("nope".to_owned(), "SUPER+X".to_owned());
        t.insert("panel_left".to_owned(), "SUPER+".to_owned());
        let (b, w) = resolve_settings(&t);
        let names: Vec<_> = b.iter().map(|b| b.action.name()).collect();
        assert!(names.contains(&"volume_up") && names.contains(&"panel_right"));
        assert!(!names.contains(&"volume_mute") && !names.contains(&"bar_toggle") && !names.contains(&"panel_left"));
        assert_eq!(w.len(), 3, "{w:?}");
        // Bir ailenin tuşunu tek tuş eylemine vermek de çakışmadır.
        let t = BTreeMap::from([("panel_left".to_owned(), "SUPER+code:12".to_owned())]);
        let (b, w) = resolve_settings(&t);
        assert!(b.iter().any(|b| b.action == Action::PanelLeft));
        assert!(!b.iter().any(|b| b.action == Action::Workspace), "{w:?}");
    }

    #[test]
    fn conflicts_with_user_binds_are_skipped() {
        let mut t = BTreeMap::new();
        t.insert("panel_right".to_owned(), "SUPER+N".to_owned());
        let (b, _) = resolve_settings(&t);
        let user = vec![(parse_hypr_combo("SUPER", "n").unwrap(), 7usize)];
        let (keep, conflicts) = split_conflicts(b, &user);
        assert!(keep.iter().all(|b| b.action != Action::PanelRight));
        assert_eq!(conflicts, vec![Conflict { action: Action::PanelRight, combo: parse_setting_combo("SUPER+N").unwrap(), at: 7, reason: ConflictReason::UserChoice, can_take: true }]);
    }

    /// Bütün varsayılanlar birbirinden farklı (hiçbiri çakışma yüzünden düşmez); Hyprland'in örnek
    /// ayarıyla (v0.56.2 `/usr/share/hypr/hyprland.lua`) çakışanlar bilinçli ve listelidir: o ayar
    /// duruyorsa kullanıcınınki çalışır, Nuxut'unki üretilmez (çift tetikleme yok).
    #[test]
    fn defaults_are_distinct_and_stock_overlap_is_known() {
        let (b, w) = resolve_settings(&BTreeMap::new());
        assert!(w.is_empty(), "{w:?}");
        let assigned = Action::ALL.iter().filter(|a| !a.default_combo().is_empty()).count();
        assert_eq!(b.len(), assigned);
        let hypr = |a: Action| b.iter().find(|x| x.action == a).map(|x| x.combo.to_hypr());
        assert_eq!(hypr(Action::MagnifierIn).as_deref(), Some("SUPER ALT, up"));
        assert_eq!(hypr(Action::MagnifierReset).as_deref(), Some("SUPER ALT, escape"));
        assert_eq!(Action::MagnifierIn.kind(), BindKind::LockedRepeat);
        let mut stock: Vec<(&str, String)> = ["q", "c", "m", "e", "v", "r", "p", "j", "left", "right", "up", "down", "s", "mouse_down", "mouse_up", "mouse:272", "mouse:273"]
            .iter()
            .map(|k| ("SUPER", (*k).to_owned()))
            .collect();
        for i in 0..=9 {
            stock.push(("SUPER", i.to_string()));
            stock.push(("SUPER SHIFT", i.to_string()));
        }
        stock.push(("SUPER SHIFT", "s".into()));
        let user: Vec<_> = stock.iter().map(|(m, k)| (parse_hypr_combo(m, k).unwrap(), ())).collect();
        let (keep, conflicts) = split_conflicts(b, &user);
        let mut dropped: Vec<_> = conflicts.iter().map(|c| c.action.name()).collect();
        dropped.sort_unstable();
        assert_eq!(
            dropped,
            [
                "close_window", "file_manager", "focus_down", "focus_left", "focus_right", "focus_up", "maximize", "move_to_scratchpad",
                "move_to_workspace", "pin", "scratchpad", "toggle_split", "window_drag", "window_resize_drag", "workspace",
                "workspace_scroll_next", "workspace_scroll_prev"
            ]
        );
        // Kabuk eylemleri ve kapatma/başlatıcı yedekleri yine çalışır.
        for a in [Action::MagnifierIn, Action::Launcher, Action::CloseWindowAlt, Action::Lock, Action::Settings] {
            assert!(keep.iter().any(|k| k.action == a), "{}", a.name());
        }
    }

    /// Ekran klavyesi kısayolu (t125): düzenden bağımsız harf (TR-Q ve US'te aynı tuş), kilit
    /// ekranında tetiklenmez; stok bind'lerle çakışmazlık yukarıdaki testte bütün varsayılanlar için.
    #[test]
    fn osk_toggle_default() {
        let (b, _) = resolve_settings(&BTreeMap::new());
        let bind = b.iter().find(|x| x.action == Action::OskToggle).expect("varsayılanı var");
        assert_eq!(bind.combo.to_hypr(), "SUPER ALT, k");
        assert_eq!(Action::OskToggle.kind(), BindKind::Plain);
        assert_eq!(Action::OskToggle.target(), Target::Shell("msg osk toggle"));
    }

    #[test]
    fn default_combos_do_not_collide_and_emoji_opens_emoji_mode() {
        let defaults: Vec<Binding> = Action::ALL
            .iter()
            .filter(|a| !a.default_combo().is_empty())
            .map(|&a| Binding { action: a, combo: parse_action_combo(a, a.default_combo()).unwrap() })
            .collect();
        for (i, b) in defaults.iter().enumerate() {
            assert!(defaults[i + 1..].iter().all(|o| !o.clashes(b)), "iki eylemin varsayılanı aynı: {}", b.combo);
        }
        let (b, w) = resolve_settings(&BTreeMap::new());
        assert!(w.is_empty(), "{w:?}");
        let emoji = b.iter().find(|b| b.action == Action::Emoji).expect("emoji varsayılan olarak atanır");
        assert_eq!(emoji.combo, parse_hypr_combo("SUPER", "period").unwrap());
        assert_eq!(emoji.combo.to_hypr(), "SUPER, period");
        assert_eq!(Action::Emoji.target(), Target::Shell("msg launcher emoji"));
        // Kullanıcının SUPER+. bind'i varsa Nuxut'unki üretilmez.
        let (keep, conflicts) = split_conflicts(b, &[(parse_hypr_combo("SUPER", "PERIOD").unwrap(), 1usize)]);
        assert!(keep.iter().all(|b| b.action != Action::Emoji));
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn action_names_are_unique_and_defaults_parse() {
        for (i, a) in Action::ALL.iter().enumerate() {
            assert_eq!(Action::from_name(a.name()), Some(*a));
            assert!(Action::ALL[i + 1..].iter().all(|b| b.name() != a.name()));
            let d = a.default_combo();
            assert!(d.is_empty() || parse_action_combo(*a, d).is_ok(), "{}", a.name());
            assert_eq!(a.is_base(), *a >= Action::Launcher);
            // Yalnızca dokunma eylemi bırakışta; yalnızca fare eylemleri bindm.
            assert_eq!(a.kind() == BindKind::Release, a.shape() == Shape::Tap, "{}", a.name());
        }
    }

    /// Türkçe Q / AZERTY'de çalışırlık: varsayılan tuşların hiçbiri değiştiricisiz katmanı düzene
    /// göre değişen noktalama keysym'i değil (SUPER+equal sorunu: Türkçe Q'da `=` Shift ister).
    #[test]
    fn defaults_avoid_layout_dependent_punctuation() {
        const LAYOUT_DEPENDENT: [&str; 12] =
            ["equal", "minus", "plus", "comma", "period", "slash", "semicolon", "apostrophe", "grave", "bracketleft", "bracketright", "backslash"];
        // İstisna: emoji SUPER+. (2026-10-13, kullanıcı onayına açık) — `.` ABD ve Türkçe Q'da
        // değiştiricisiz, AZERTY'de Shift ister (o düzende çalışmaz; docs/design/hyprland-base.md §3).
        for a in Action::ALL.iter().filter(|a| **a != Action::Emoji) {
            let d = a.default_combo().to_ascii_lowercase();
            let key = d.rsplit('+').next().unwrap_or("");
            assert!(!LAYOUT_DEPENDENT.contains(&key), "{}: {d}", a.name());
            // Rakamlar keysym olarak değil, tuş koduyla (aile).
            assert!(!(key.len() == 1 && key.as_bytes()[0].is_ascii_digit()), "{}", a.name());
        }
    }
}
