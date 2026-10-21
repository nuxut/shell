//! Eşitleme işi (UI'dan bağımsız): ortam → ayar dosyası (Lua ya da hyprlang, ADR-0003) →
//! tarama → üretim/yazım → dahil etme satırı → canlı uygulama; ayrıca "Geri al". Bloklayan IO;
//! arka plan thread'inde çağrılır. Bütün yazımlar `nuxut-apply` üzerinden (ADR-0001):
//!
//! - Üretilen `nuxut.conf`/`nuxut.lua`: türetilmiş, snapshot yok (`apply::write_derived`). Yolda
//!   Nuxut'a ait olmayan bir dosya varsa yalnızca dahil etme satırı eklenirken ya da satır zaten
//!   varken, `hypr.source` snapshot'ıyla üzerine yazılır (eski `.bak` yedeğinin yerine).
//! - Kullanıcının ana dosyasındaki açıklama + dahil etme satırı: `Fragment::Line` ×2,
//!   `hypr.source` snapshot'ı. "Geri al" da bir işlemdir (satırlar + üretilen dosya `Absent`),
//!   o da `msg snapshot restore` ile geri alınabilir.
//! - "Ben ekledim" işareti (`$XDG_STATE_HOME/nuxut/hypr-source-added`): `apply::write_own`.
//!   Satır sonradan yoksa `removed_by_user` (kendiliğinden yeniden eklenmez); işaret
//!   `ensure_source = false` görülünce silinir, böylece yeniden açılınca satır eklenir.
//!
//! **Güvenlik kapısı ([`SyncEnv::guard`]):** `nuxut_apply::Env::guard(Hyprland)`: Hyprland
//! imzası yoksa hiçbir şey yapılmaz. Test / ikincil örnek (`NUXUT_SHELL_INSTANCE`) yalnızca
//! gerçek ev dizini dışındaki mutlak `XDG_CONFIG_HOME`/`XDG_STATE_HOME` altında dosya yazar,
//! çalışan Hyprland'in süreç bilgisine bakmaz ve Hyprland'e **hiçbir** komut göndermez
//! (kontrol `None`; yalıtılmış iç içe Hyprland istisnası [`SyncEnv::isolated_control`]).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use nuxut::i18n::t;
use nuxut_apply::{Applier, Change, Env, FragValue, Fragment, Outcome, Position, Skip, Source, TargetKind};
use serde::{Deserialize, Serialize};

use super::base::{self, BaseInput, BaseLayer, conf_takeover_section, generate_base_conf, generate_base_lua, ignore_alpha_for};
use super::control::{ApplyAction, HyprControl, apply, plan_apply};
use super::ensure::{
    CONF_COMMENT, LUA_COMMENT, LaunchMode, PlanInput, SourcePlan, config_path_from_args, display_path, launch_mode,
    plan_source, source_line, split_cmdline,
};
use super::generate::{GenerateInput, Generated, LookGroup, MANAGED_KEYS, MotionGroup, WindowsGroup, generate_conf};
use super::input::InputSettings;
use super::keybinds::{Blocker, Combo, KeyPref, MOD_ALT, MOD_CTRL, MOD_OTHER, MOD_SHIFT, MOD_SUPER, UserKey, resolve_settings, split_bindings};
use super::lua::{INCLUDE_LINE, generate_lua, scan_lua};
use super::parse::{ChainReport, Location, ScanInput, ScanLimits, scan_chain};
use super::settings::{CompositorConfig, KeybindOwnerConfig};
use crate::services::apply::{self as ext, HYPR_SOURCE};

/// Üretilen dosya üst sınırı (okuma).
pub const MAX_OWNED_BYTES: u64 = 256 * 1024;

/// Hyprland ayar dili (dosya uzantısından; Hyprland da öyle seçer — ADR-0003 Bulgu 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigKind {
    Lua,
    Hyprlang,
}

impl ConfigKind {
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "lua" => Some(Self::Lua),
            "conf" => Some(Self::Hyprlang),
            _ => None,
        }
    }

    /// Üretilen dosyanın adı (ana dosyanın dizininde).
    pub fn generated_name(self) -> &'static str {
        match self {
            Self::Lua => "nuxut.lua",
            Self::Hyprlang => "nuxut.conf",
        }
    }

    /// `hyprctl status` `configProvider` adı.
    fn provider(self) -> &'static str {
        match self {
            Self::Lua => "lua",
            Self::Hyprlang => "hyprlang",
        }
    }
    /// Dahil etme satırının parçaları: (açıklama, satır) — `Fragment::Line` işaretleri, sona
    /// eklenir. Hyprlang hedef yolu yazdırılamıyorsa `None`.
    pub(crate) fn include_markers(self, target_display: &str) -> Option<[String; 2]> {
        match self {
            Self::Lua => Some([LUA_COMMENT.to_owned(), INCLUDE_LINE.to_owned()]),
            Self::Hyprlang => Some([CONF_COMMENT.to_owned(), source_line(target_display)?]),
        }
    }

    /// Temel katman dosyasının ana dosyanın dizinine göre yolu (ADR-0006).
    pub fn base_rel(self) -> &'static str {
        match self {
            Self::Lua => base::LUA_REL,
            Self::Hyprlang => base::CONF_NAME,
        }
    }

    /// Temel katmanın dahil etme satırı: (kod, satırın tamamı = kod + aynı satırda açıklama) —
    /// dosyanın **başına**, tek parça.
    pub(crate) fn base_markers(self, base_display: &str) -> Option<(String, String)> {
        let (code, note) = match self {
            Self::Lua => (base::LUA_INCLUDE_LINE.to_owned(), base::LUA_NOTE),
            Self::Hyprlang => (source_line(base_display)?, base::CONF_NOTE),
        };
        let line = format!("{code}  {note}");
        Some((code, line))
    }
}

/// Temel katman satırının kullanıcı dosyasındaki hâli (yalnızca ana dosyada aranır: satırı Nuxut
/// oraya, başa koyar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseLine {
    Present,
    /// Yorum işaretiyle kapatılmış (`# source = …nuxut-base.conf`, `-- pcall(require, "nuxut.base")`).
    Commented,
    Absent,
}

/// Satırın kodu `code` mu (ardından yalnızca yorum gelebilir)?
fn is_code_line(line: &str, code: &str) -> bool {
    line.strip_prefix(code).is_some_and(|rest| {
        let rest = rest.trim_start();
        rest.is_empty() || rest.starts_with('#') || rest.starts_with("--")
    })
}

fn base_line_state(text: &str, code: &str) -> BaseLine {
    let mut commented = false;
    for line in text.lines().map(str::trim) {
        if is_code_line(line, code) {
            return BaseLine::Present;
        }
        let uncommented = line.trim_start_matches(['#', '-']).trim_start();
        if uncommented.len() < line.len() && is_code_line(uncommented, code) {
            commented = true;
        }
    }
    if commented { BaseLine::Commented } else { BaseLine::Absent }
}

/// Süreç ortamı (testte elle kurulur). Dosya kapısı ve yollar `nuxut_apply::Env`'den (main'de
/// bir kez kurulan); burada yalnızca Hyprland'e özgü ekler.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncEnv {
    /// Yazım ortamı: ev, `config_home`, `state_home`, test örneği, Hyprland imzası, kapalı.
    pub apply: Env,
    pub runtime_dir: Option<PathBuf>,
    /// Test örneği yalıtılmış (iç içe) bir Hyprland'e komut gönderebilir: açık izin
    /// (`NUXUT_TEST_HYPR_CONTROL=1`) VE `XDG_RUNTIME_DIR` kullanıcının `/run/user/…` dizini değil.
    /// Hyprland soketi `$XDG_RUNTIME_DIR/hypr/<imza>` altında olduğundan kullanıcının oturumuna
    /// yapısal olarak ulaşılamaz.
    pub isolated_control: bool,
    pub under_systemd: bool,
}

/// Test örneğinin iç içe Hyprland'e komut göndermesine izin veren ortam değişkeni.
pub const TEST_CONTROL_ENV: &str = "NUXUT_TEST_HYPR_CONTROL";

/// [`SyncEnv::isolated_control`] kuralı (saf; testli).
fn isolated_runtime(opt_in: bool, runtime_dir: Option<&Path>) -> bool {
    opt_in
        && runtime_dir.is_some_and(|d| {
            d.is_absolute()
                && !d.starts_with("/run/user")
                && !d.components().any(|c| c == std::path::Component::ParentDir)
        })
}

/// Hyprland soketine giden yolun her basamağı (`hypr`, `hypr/<imza>`, `.socket.sock`) gerçek
/// yoluna çözüldüğünde de `/run/user` dışında mı? (`$XDG_RUNTIME_DIR/hypr` bir bağlantıyla
/// kullanıcının oturumuna yönlendirilemesin.) Yol yoksa `false`.
fn isolated_socket_path(opt_in: bool, runtime_dir: Option<&Path>, signature: Option<&str>) -> bool {
    let (Some(dir), Some(sig)) = (runtime_dir, signature) else { return false };
    if sig.is_empty() || !sig.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return false;
    }
    let hypr = dir.join("hypr");
    let inst = hypr.join(sig);
    [hypr, inst.clone(), inst.join(".socket.sock")]
        .iter()
        .all(|p| p.canonicalize().ok().is_some_and(|c| isolated_runtime(opt_in, Some(&c))))
}

impl SyncEnv {
    /// `apply`: main'deki `Env::from_process` (tek kapı). Çalışma dizini ve systemd bilgisi
    /// süreç ortamından.
    pub fn from_process(apply: Env) -> Self {
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map(PathBuf::from);
        let opt_in = std::env::var_os(TEST_CONTROL_ENV).is_some_and(|v| v == "1");
        let test_instance = apply.test_instance;
        Self {
            // Bağlantı üzerinden /run/user'a ulaşan bir dizin de reddedilir.
            isolated_control: test_instance
                && isolated_runtime(opt_in, runtime_dir.as_deref())
                && isolated_runtime(opt_in, runtime_dir.as_deref().and_then(|d| d.canonicalize().ok()).as_deref())
                && isolated_socket_path(opt_in, runtime_dir.as_deref(), apply.hyprland.as_deref()),
            runtime_dir,
            apply,
            under_systemd: std::env::var_os("INVOCATION_ID").is_some() || std::env::var_os("SYSTEMD_EXEC_PID").is_some(),
        }
    }

    fn home(&self) -> Option<&Path> {
        self.apply.home.as_deref()
    }

    /// Eşitleme bu ortamda çalışabilir mi? `Err` = neden (log/durum için).
    pub fn guard(&self) -> Result<(), &'static str> {
        self.apply.guard(TargetKind::Hyprland).map_err(|s| match s {
            Skip::NoSession => "Hyprland oturumu değil (ya da geçersiz imza)", // i18n: tanı msg hypr status nedeni (çevrilmez)
            Skip::TestInstance => {
                "test örneği: XDG_CONFIG_HOME ve XDG_STATE_HOME gerçek ev dizini dışında geçici dizinler olmalı (kullanıcının ayarlarına yazılmaz)" // i18n: tanı msg hypr status nedeni (çevrilmez)
            }
            Skip::Disabled => "bu örnek dosya yazmaz (tek-örnek kilidi alınmadı)", // i18n: tanı msg hypr status nedeni (çevrilmez)
        })
    }

    /// Hyprland'e komut gönderilebilir mi? Test örneği yalnızca yalıtılmış (iç içe) Hyprland'e
    /// ([`Self::isolated_control`]); kullanıcının oturumuna asla.
    pub fn may_control(&self) -> bool {
        let isolated = self.isolated_control && isolated_runtime(true, self.runtime_dir.as_deref());
        (!self.apply.test_instance || isolated) && self.guard().is_ok()
    }

    /// XDG tabanları (kullanıcının; sistem dizinlerine yazılmaz): `config_home` (mutlak
    /// `XDG_CONFIG_HOME`, yoksa `~/.config`) ve test örneği değilse ayrıca `~/.config`.
    fn bases(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(x) = self.apply.config_home.as_ref().filter(|p| p.is_absolute()) {
            out.push(x.clone());
        }
        if !self.apply.test_instance
            && let Some(h) = self.home()
        {
            let c = h.join(".config");
            if !out.contains(&c) {
                out.push(c);
            }
        }
        out
    }

    /// Ana ayar dosyası: çalışan Hyprland'in `-c`'si ya da `HYPRLAND_CONFIG`'i (test örneğinde
    /// bakılmaz), yoksa XDG araması — önce her tabanda `hyprland.lua`, sonra `hyprland.conf`
    /// (Hyprland'in sırası). Hiçbiri yoksa ilk tabanın `hyprland.lua`'sı (Hyprland onu üretir).
    pub fn config_path(&self) -> Option<PathBuf> {
        let test = self.apply.test_instance;
        let explicit = (!test).then(|| self.running_config_path()).flatten();
        let path = match explicit {
            Some(p) => p,
            None => {
                let bases = self.bases();
                let found = ["lua", "conf"]
                    .iter()
                    .flat_map(|ext| bases.iter().map(move |b| b.join("hypr").join(format!("hyprland.{ext}"))))
                    .find(|p| p.is_file());
                found.or_else(|| bases.first().map(|b| b.join("hypr/hyprland.lua")))?
            }
        };
        let path = match path.strip_prefix("~") {
            Ok(rest) => self.home()?.join(rest),
            Err(_) => path,
        };
        if !path.is_absolute() {
            return None;
        }
        if test && !self.apply.config_home.as_deref().is_some_and(|x| path.starts_with(x)) {
            return None;
        }
        Some(path)
    }

    /// `$XDG_RUNTIME_DIR/hypr/<imza>/hyprland.lock` → pid → `/proc/<pid>/cmdline` `-c`, yoksa
    /// `/proc/<pid>/environ` içinden yalnızca `HYPRLAND_CONFIG`.
    fn running_config_path(&self) -> Option<PathBuf> {
        self.guard().ok()?;
        let dir = self.runtime_dir.as_ref()?.join("hypr").join(self.apply.hyprland.as_ref()?);
        let lock = read_limited(&dir.join("hyprland.lock"), 256)?;
        let pid: u32 = String::from_utf8_lossy(&lock).lines().next()?.trim().parse().ok()?;
        if let Some(p) = read_limited(Path::new(&format!("/proc/{pid}/cmdline")), 64 * 1024)
            .and_then(|c| config_path_from_args(split_cmdline(&c)))
        {
            return Some(p);
        }
        let environ = read_limited(Path::new(&format!("/proc/{pid}/environ")), 256 * 1024)?;
        environ_value(&environ, "HYPRLAND_CONFIG").map(PathBuf::from)
    }

    /// "Ben ekledim" işareti dosyası (`$XDG_STATE_HOME/nuxut/hypr-source-added`).
    pub fn state_file(&self) -> Option<PathBuf> {
        Some(self.apply.nuxut_state_dir()?.join("hypr-source-added"))
    }

    /// Temel katman satırının "ben ekledim" işareti (`$XDG_STATE_HOME/nuxut/hypr-base-added`):
    /// satır sonradan yoksa kullanıcı kaldırmış sayılır, kendiliğinden yeniden eklenmez.
    pub fn base_state_file(&self) -> Option<PathBuf> {
        Some(self.apply.nuxut_state_dir()?.join("hypr-base-added"))
    }
}

/// `environ` (NUL ayrımlı) içinden tek bir değişken; diğerleri okunmaz/saklanmaz.
pub fn environ_value(raw: &[u8], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    raw.split(|&b| b == 0)
        .find(|e| e.starts_with(prefix.as_bytes()))
        .map(|e| String::from_utf8_lossy(&e[prefix.len()..]).into_owned())
        .filter(|v| !v.is_empty() && v.len() <= 4096)
}

fn read_limited(path: &Path, max: u64) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(max + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= max).then_some(buf)
}

/// Bir eşitleme isteği (ana thread temadan ve shell.toml'dan kurar; `None` grup eşitlenmez).
#[derive(Debug, Clone, PartialEq)]
pub struct SyncRequest {
    pub compositor: CompositorConfig,
    pub look: Option<LookGroup>,
    pub motion: Option<MotionGroup>,
    pub windows: Option<WindowsGroup>,
    /// Doğrulanmış `[keybinds]` (eşitlenmiyorsa `None`).
    pub keybinds: Option<BTreeMap<String, String>>,
    /// Doğrulanmış `[keybind_owner]`: çakışan tuşta kimin bind'i çalışsın.
    pub key_owner: KeybindOwnerConfig,
    /// Shell masaüstü zeminini (duvar kağıdı) çiziyor: Hyprland varsayılan duvar kağıdı ve logosu
    /// kapatılır (bkz. [`GenerateInput::hide_defaults`]).
    pub hide_defaults: bool,
    pub shell_exe: String,
    /// Oturum ortamı (portal: `XDG_CURRENT_DESKTOP`; `services::portal_env`),
    /// portal kurulu değilse boş.
    pub session_env: Vec<(String, String)>,
    /// Birleşik görünümün imleç ortamı (ADR-0004 §5.4; `XCURSOR_*`/`HYPRCURSOR_*`), kapalıyken boş.
    pub cursor_env: Vec<(String, String)>,
    /// Doğrulanmış `[input]` (`compositor.sync_input` kapalıysa `None`).
    pub input: Option<InputSettings>,
    /// `msg hypr add-source`: kapalı/kullanıcı kapatmış olsa bile bir kez ekle.
    pub force_add: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Override {
    pub key: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictReport {
    pub action: &'static str,
    pub combo: String,
    /// hyprlang: kullanıcının dosyası; Lua: çalışan Hyprland'den (`hyprctl binds`), yer yok.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// `conflicts`: `user_wins` (temel katman, kullanıcınınki bilerek kazanır), `user_choice`
    /// (kullanıcı kendininkini seçti), `read_after|not_loaded|special_mods|unlocated` (Nuxut'unki
    /// isteniyor ama devralınamıyor). `taken_over`: `default` ya da `chosen`.
    pub reason: &'static str,
    /// `conflicts`: "Nuxut'unkini kullan" seçilirse devralınabilir mi. `taken_over`'da her zaman `true`.
    pub can_take: bool,
}

/// `msg hypr status` yanıtı (tasarım notu §6.3). Kullanıcı dosyasının satır metni girmez.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SyncStatus {
    /// `present|added|off|disabled_by_user|removed_by_user|missing_config|unsupported|unknown|error|inactive|undone`
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ConfigKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated: Option<String>,
    /// Temel katman (ADR-0006): `present|added|disabled_by_user|removed_by_user|off`; dosyası `base_file`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Bu turda alınan `hypr.source` snapshot'ının kimliği (ekleme, yabancı dosyaya ilk dokunuş,
    /// geri al); `msg snapshot restore <kimlik>` ile geri alınır.
    pub snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch: Option<&'static str>,
    /// Diskteki ayar dili çalışan Hyprland'inkinden farklı (kullanıcı Lua'ya geçti; sonraki
    /// oturumda geçerli). Canlı uygulama yapılmadı.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pending_switch: bool,
    pub overrides: Vec<Override>,
    pub conflicts: Vec<ConflictReport>,
    /// Devralınan kısayollar: kullanıcının (dosyası değişmeyen) bind'i ayar katmanında `unbind`
    /// edildi, yerine Nuxut'unki bağlandı (`[compositor] take_over_keys`; `combo` kullanıcınınki).
    pub taken_over: Vec<ConflictReport>,
    pub errors: Vec<String>,
    /// Bu turda üretilen dosya yazıldı.
    #[serde(skip)]
    pub wrote: bool,
    /// Bu turda dahil etme satırı eklendi (bildirim için).
    #[serde(skip)]
    pub added: bool,
    /// Bu turu doğuran işin sıra numarası (shell'in "uygulama sürüyor mu?" takibi; JSON'a girmez).
    #[serde(skip)]
    pub seq: u64,
    /// Bu turda üretilen dosya çalışan Hyprland'e canlı uygulandı (`apply` başarılı; değişiklik
    /// yoksa da). `msg input status`'un değer karşılaştırmalı override tespiti yalnızca o zaman.
    #[serde(skip)]
    pub applied: bool,
}

impl SyncStatus {
    pub fn inactive(reason: &str) -> Self {
        Self { source: "inactive", reason: Some(reason.to_owned()), ..Self::default() }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"source\":\"error\"}".into())
    }
}

/// `hyprctl -j binds` öğesi (yalnızca gereken alanlar).
#[derive(Debug, Deserialize)]
struct LiveBind {
    #[serde(default)]
    modmask: u32,
    #[serde(default)]
    key: String,
    #[serde(default)]
    keycode: u32,
    #[serde(default)]
    description: String,
    /// Alt eşlemdeki bind genel kısayolla çakışmaz.
    #[serde(default)]
    submap: String,
}

/// Hyprland değiştirici maskesi (wlr/xkb sırası) → [`Combo`] maskesi.
fn combo_from_live(modmask: u32, key: &str) -> Option<Combo> {
    const SHIFT: u32 = 1;
    const CAPS: u32 = 2;
    const CTRL: u32 = 4;
    const ALT: u32 = 8;
    const MOD2: u32 = 16;
    const MOD3: u32 = 32;
    const SUPER: u32 = 64;
    const MOD5: u32 = 128;
    let mut m = 0;
    for (bit, ours) in [(SHIFT, MOD_SHIFT), (CTRL, MOD_CTRL), (ALT, MOD_ALT), (SUPER, MOD_SUPER)] {
        if modmask & bit != 0 {
            m |= ours;
        }
    }
    if modmask & (CAPS | MOD2 | MOD3 | MOD5) != 0 {
        m |= MOD_OTHER;
    }
    let key = key.trim().to_ascii_lowercase();
    (!key.is_empty() && key.len() <= 32).then_some(Combo { mods: m, key })
}

/// Çalışan Hyprland'deki kullanıcı bind'leri (Nuxut'unkiler — `nuxut:` açıklamalı — hariç).
fn live_user_binds(ctl: &dyn HyprControl) -> Vec<Combo> {
    let Ok(text) = ctl.request("j/binds") else { return Vec::new() };
    let Ok(binds) = serde_json::from_str::<Vec<LiveBind>>(&text) else { return Vec::new() };
    // Lua'da `code:` tuşlu bind'in `key` alanı boş, `keycode` 0 gelir (Hyprland 0.56.2
    // `parseKeyString`): böyle bir bind görülemez (bilinen sınır, docs/design/hyprland-base.md §6).
    binds
        .iter()
        .filter(|b| !b.description.starts_with("nuxut:") && b.submap.is_empty())
        .filter_map(|b| if b.key.is_empty() && b.keycode > 0 { combo_from_live(b.modmask, &format!("code:{}", b.keycode)) } else { combo_from_live(b.modmask, &b.key) })
        .take(1024)
        .collect()
}

/// Çalışan Hyprland'in ayar dili (`hyprctl -j status` `configProvider`).
fn live_provider(ctl: &dyn HyprControl) -> Option<String> {
    #[derive(Deserialize)]
    struct Status {
        #[serde(rename = "configProvider")]
        config_provider: Option<String>,
    }
    let text = ctl.request("j/status").ok()?;
    serde_json::from_str::<Status>(&text).ok()?.config_provider
}

/// Lua arka ucunun canlı uygulama planı: bind/başlatma satırları ya da yazılı `[input]` anahtar
/// kümesi değiştiyse yeniden yükleme, yalnızca değerler değiştiyse `eval`.
pub fn plan_apply_lua(old: Option<&str>, new: &Generated) -> ApplyAction {
    if old == Some(new.text.as_str()) {
        return ApplyAction::Nothing;
    }
    // `[input]` anahtar kümesi değişince de yeniden okuma: `eval` kaldırılan değeri geri almaz.
    let structural = |t: &str| {
        t.lines()
            .filter(|l| l.contains("hl.bind(") || l.contains("hl.on(") || l.starts_with(super::lua::INPUT_KEYS_MARKER))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    match (old, &new.eval) {
        (Some(o), Some(code)) if structural(o) == structural(&new.text) => ApplyAction::Eval(code.clone()),
        _ => ApplyAction::Reload,
    }
}

/// Eşitleyici; her istek [`Syncer::run`], geri alma [`Syncer::undo`].
#[derive(Debug)]
pub struct Syncer {
    env: SyncEnv,
}

/// Bir turun çözülmüş yolları.
struct Paths {
    config: PathBuf,
    kind: ConfigKind,
    target: PathBuf,
    /// Temel katman dosyası (ADR-0006).
    base: PathBuf,
}

/// Üretilen dosya yolunun şu anki hâli.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GenState {
    Absent,
    /// Nuxut başlıklı (metin, sınır içinde).
    Ours(String),
    /// Kullanıcının (başlıksız) ya da okunamayan içerik.
    Foreign,
}

impl Syncer {
    pub fn new(env: SyncEnv) -> Self {
        Self { env }
    }

    fn show(&self, p: &Path) -> String {
        display_path(p, self.env.home())
    }

    fn loc(&self, files: &[PathBuf], at: Location) -> (Option<String>, Option<usize>) {
        (files.get(at.file).map(|f| self.show(f)), Some(at.line))
    }

    fn applier(&self) -> Applier {
        Applier::open(self.env.apply.clone()).with_source(Source::Shell)
    }

    /// Kapı + yol + dil; olmazsa durum.
    fn paths(&self) -> Result<Paths, Box<SyncStatus>> {
        self.env.guard().map_err(|r| Box::new(SyncStatus::inactive(r)))?;
        let config =
            self.env.config_path().ok_or_else(|| Box::new(SyncStatus::inactive("Hyprland ayar dosyasının yolu bulunamadı")))?; // i18n: tanı msg hypr status nedeni (çevrilmez)
        let mut status = SyncStatus { config: Some(self.show(&config)), ..SyncStatus::default() };
        let Some(kind) = ConfigKind::of(&config) else {
            status.source = SourcePlan::Unsupported.name();
            return Err(Box::new(status));
        };
        status.kind = Some(kind);
        if !config.is_file() {
            status.source = SourcePlan::MissingConfig.name();
            return Err(Box::new(status));
        }
        let Some(dir) = config.parent() else {
            status.source = SourcePlan::Unsupported.name();
            return Err(Box::new(status));
        };
        let target = dir.join(kind.generated_name());
        let base = dir.join(kind.base_rel());
        Ok(Paths { config, kind, target, base })
    }

    fn read_generated(target: &Path) -> GenState {
        match read_limited(target, MAX_OWNED_BYTES) {
            None if !target.exists() => GenState::Absent,
            Some(b) if ext::is_managed(&b) => String::from_utf8(b).map_or(GenState::Foreign, GenState::Ours),
            _ => GenState::Foreign,
        }
    }

    /// `ensure_source` kapalıyken "ben ekledim" işareti silinir: yeniden açılınca satır eklenir.
    fn clear_mark(&self) {
        for state in [self.env.state_file(), self.env.base_state_file()].into_iter().flatten() {
            if state.exists()
                && let Err(e) = nuxut_apply::remove_file(&self.env.apply, &state, None)
            {
                tracing::warn!("Hyprland eşitleme durum işareti silinemedi: {}", ext::error_text(&e));
            }
        }
    }

    fn set_mark(&self) -> Result<(), String> {
        self.write_mark(self.env.state_file())
    }

    fn set_base_mark(&self) -> Result<(), String> {
        self.write_mark(self.env.base_state_file())
    }

    fn write_mark(&self, state: Option<PathBuf>) -> Result<(), String> {
        let state = state.ok_or("durum dizini yok")?;
        if let Some(dir) = state.parent() {
            ext::ensure_private_dir(&self.env.apply, dir).map_err(|e| ext::error_text(&e))?;
        }
        ext::write_own(&self.env.apply, &state, b"1\n").map(|_| ()).map_err(|e| ext::error_text(&e))
    }

    /// Bir isteği işler. `ctl`: Hyprland erişimi (yoksa yalnızca dosyalar).
    pub fn run(&self, req: &SyncRequest, ctl: Option<&dyn HyprControl>) -> SyncStatus {
        if let Err(reason) = self.env.guard() {
            return SyncStatus::inactive(reason);
        }
        if !req.compositor.ensure_source && !req.force_add {
            self.clear_mark();
        }
        let has_input = req.input.as_ref().is_some_and(|i| !i.is_empty());
        if !req.compositor.any() && !has_input && !req.force_add {
            return SyncStatus { source: SourcePlan::Off.name(), ..SyncStatus::default() };
        }
        let Paths { config, kind, target, base } = match self.paths() {
            Ok(p) => p,
            Err(s) => return *s,
        };
        let mut status = SyncStatus {
            config: Some(self.show(&config)),
            kind: Some(kind),
            generated: Some(self.show(&target)),
            ..SyncStatus::default()
        };
        if let Some(ctl) = ctl
            && let Some(p) = live_provider(ctl)
            && p != kind.provider()
        {
            status.pending_switch = true;
        }

        // Tarama (dile göre) → karar girdisi ortak biçimde.
        let (chain, external, conf_report, files) = match kind {
            ConfigKind::Hyprlang => {
                let r = scan_chain(&ScanInput {
                    root: &config,
                    target: &target,
                    home: self.env.home(),
                    watch_keys: MANAGED_KEYS,
                    skip: std::slice::from_ref(&base),
                    limits: ScanLimits::default(),
                });
                let ext = r.external_launch.is_some();
                let files = r.files.clone();
                (r.clone(), ext, Some(r), files)
            }
            ConfigKind::Lua => {
                let r = scan_lua(&config, self.env.home(), ScanLimits::default());
                let at = |i: usize| Location { file: i, line: 0 };
                let chain = ChainReport {
                    files: r.files.clone(),
                    source_at: r.included_in.map(at),
                    commented_at: r.commented_in.map(at),
                    external_launch: r.external_launch_in.map(at),
                    truncated: r.truncated,
                    ..ChainReport::default()
                };
                (chain, r.external_launch_in.is_some(), None, r.files)
            }
        };
        let launch = launch_mode(self.env.under_systemd, &chain);
        status.launch = Some(if external { LaunchMode::External.name() } else { launch.name() });

        // Dahil etme satırı kararı (üretilen dosyanın yabancı olup olmadığına göre yazım şekli).
        let state = self.env.state_file();
        let previously_added = state.as_deref().is_some_and(Path::exists);
        let mut plan = plan_source(&PlanInput {
            enabled: req.compositor.ensure_source || req.force_add,
            config_exists: true,
            config_path: &config,
            previously_added: previously_added && !req.force_add,
            report: &chain,
        });
        if plan == SourcePlan::Unsupported {
            // plan_source yalnızca `.conf` bilir; Lua da desteklenir.
            plan = if kind == ConfigKind::Lua { lua_plan(&chain, req, previously_added) } else { plan };
        }
        if req.force_add && plan == SourcePlan::DisabledByUser {
            plan = SourcePlan::Add;
        }
        status.source = plan.name();

        // Kısayollar: kullanıcının bind'leriyle çakışanlar temel katmanda üretilmez; kabuk eylemleri
        // ayar katmanından önce okunan bind'i devralır (`unbind` + Nuxut'unki, ayar katmanında).
        // Lua'da bind'lerin yeri bilinmiyor (çalışan Hyprland'den): devralma yok.
        let mut binds = Vec::new();
        let mut taken = Vec::new();
        if let Some(map) = &req.keybinds {
            let (resolved, _) = resolve_settings(map);
            let user: Vec<UserKey<Option<Location>>> = match &conf_report {
                Some(r) => {
                    // Satır sona eklenecekse bütün bind'ler ondan önce okunur.
                    let order = match plan {
                        SourcePlan::Present => r.bind_order.unwrap_or(0),
                        SourcePlan::Add => r.binds.len(),
                        _ => 0,
                    };
                    let loaded = matches!(plan, SourcePlan::Present | SourcePlan::Add);
                    r.binds
                        .iter()
                        .enumerate()
                        .map(|(i, b)| UserKey {
                            combo: b.combo.clone(),
                            at: Some(b.at),
                            blocker: if !loaded {
                                Some(Blocker::NotLoaded)
                            } else if i >= order {
                                Some(Blocker::ReadAfter)
                            } else if b.combo.mods & MOD_OTHER != 0 {
                                Some(Blocker::SpecialMods)
                            } else {
                                None
                            },
                        })
                        .collect()
                }
                None => ctl
                    .map(live_user_binds)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|combo| UserKey { combo, at: None, blocker: Some(Blocker::Unlocated) })
                    .collect(),
            };
            // Temel katman kapalıysa yalnızca kabuk eylemleri (ses, panel, büyüteç…).
            let resolved = resolved.into_iter().filter(|b| req.compositor.base_layer || !b.action.is_base()).collect();
            let split = split_bindings(resolved, &user, |a| KeyPref::resolve(a, req.key_owner.get(a), req.compositor.take_over_keys));
            binds = split.keep;
            for c in split.conflicts {
                let (file, line) = c.at.map_or((None, None), |at| self.loc(&files, at));
                status.conflicts.push(ConflictReport {
                    action: c.action.name(),
                    combo: c.combo.to_string(),
                    file,
                    line,
                    reason: c.reason.name(),
                    can_take: c.can_take,
                });
            }
            for t in split.taken {
                // Rapor: kullanıcının (değişmeyen) bind'inin ilk yeri; geri alma anahtarı belgede.
                let (file, line) = t.at.first().copied().flatten().map_or((None, None), |at| self.loc(&files, at));
                let combo = t.unbind.first().map(ToString::to_string).unwrap_or_default();
                let reason = if t.chosen { "chosen" } else { "default" };
                status.taken_over.push(ConflictReport { action: t.binding.action.name(), combo, file, line, reason, can_take: true });
                taken.push((t.binding, t.unbind));
            }
        }
        let input = GenerateInput {
            // Başka başlatıcı varsa kanca yazılmaz (çift başlatma yok); systemd altında da kanca
            // yazılır: sonraki oturumu o başlatır (ADR-0003 Karar 5).
            launch: !external,
            shell_exe: req.shell_exe.clone(),
            look: req.look.clone().filter(|_| req.compositor.sync_look),
            motion: req.motion.filter(|_| req.compositor.sync_motion),
            windows: req.windows.filter(|_| req.compositor.sync_windows),
            hide_defaults: req.hide_defaults,
            // Ekranlar'ın dosyası (ADR-0005 §6.2) yalnızca varsa yüklenir.
            displays: config
                .parent()
                .map(|d| d.join(crate::services::displays::render::generated_rel_path(kind)))
                .filter(|p| p.is_file())
                .map(|p| self.show(&p)),
            input: req.input.clone(),
            env: req.session_env.iter().chain(&req.cursor_env).cloned().collect(),
            // Güvenilir kurulum dizini (`<ev>/.local/bin`, sahiplik/izin denetimli); yoksa satır yok.
            session_path: crate::services::session_path::trusted_bin_dir(),
        };
        let mut generated = match kind {
            ConfigKind::Hyprlang => generate_conf(&input),
            ConfigKind::Lua => generate_lua(&input),
        };
        // Devralınan kabuk kısayolları ayar katmanının sonunda (kullanıcının bind'inden sonra okunur).
        generated.text.push_str(&conf_takeover_section(&taken, &req.shell_exe));
        // Temel katman: kısayollar + (açıksa) kurallar ve varsayılanlar. Katman bulanıklığı pencere
        // bulanıklığını izler; eşik gölgenin koyuluğundan (gölgenin arkası bulanıklaşmasın).
        let layer_blur = req.look.as_ref().and_then(|l| l.blur.map(|_| ignore_alpha_for(l.shadow.map(|s| s.color.a))));
        let base_input =
            BaseInput { shell_exe: req.shell_exe.clone(), binds, layer: req.compositor.base_layer.then_some(BaseLayer { layer_blur }) };
        let base_text = match kind {
            ConfigKind::Hyprlang => generate_base_conf(&base_input),
            ConfigKind::Lua => generate_base_lua(&base_input),
        };
        // Override: yalnızca bu turda üretilen anahtar ve yalnızca Hyprland'in Nuxut dosyasından
        // SONRA okuduğu atama (öncekini Nuxut ezer). Satır henüz yoksa sona eklenir: ezen yok.
        if let Some(r) = &conf_report {
            let after = r.source_order.map_or(&[][..], |n| &r.assignments[n.min(r.assignments.len())..]);
            for (key, at) in after {
                if generated.keywords.iter().any(|(k, _)| k == key) {
                    let (file, line) = self.loc(&files, *at);
                    status.overrides.push(Override { key: key.clone(), file: file.unwrap_or_default(), line: line.unwrap_or(0) });
                }
            }
        }

        // Temel katman satırı (ADR-0006): yalnızca Nuxut bağlıyken (dahil etme satırı var ya da
        // ekleniyor), ana dosyanın BAŞINA; kullanıcı kapattıysa/kaldırdıysa kendiliğinden eklenmez.
        let base_markers = kind.base_markers(&self.show(&base));
        let base_wanted = req.compositor.base_layer || req.keybinds.is_some();
        // Satır duruyorsa dosya her zaman güncel tutulur (her şey kapalıysa yalnızca başlık kalır).
        let mut base_plan = if !matches!(plan, SourcePlan::Add | SourcePlan::Present) {
            SourcePlan::Off
        } else {
            let text = read_limited(&config, nuxut_apply::MAX_FILE_BYTES).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            let base_prev = self.env.base_state_file().is_some_and(|p| p.exists());
            match base_markers.as_ref().map(|(code, _)| base_line_state(&text, code)) {
                None => SourcePlan::Unsupported,
                Some(BaseLine::Present) => SourcePlan::Present,
                _ if !base_wanted => SourcePlan::Off,
                Some(BaseLine::Commented) if !req.force_add => SourcePlan::DisabledByUser,
                Some(BaseLine::Absent) if base_prev && !req.force_add => SourcePlan::RemovedByUser,
                Some(_) => SourcePlan::Add,
            }
        };
        let base_old = match Self::read_generated(&base) {
            GenState::Ours(t) => Some(t),
            GenState::Absent => None,
            GenState::Foreign => {
                if matches!(base_plan, SourcePlan::Add | SourcePlan::Present) {
                    status.errors.push(format!("{}: Nuxut'a ait olmayan dosya; temel katman yazılmadı", self.show(&base))); // i18n: tanı msg hypr status hata satırı (çevrilmez)
                    base_plan = SourcePlan::Off;
                }
                None
            }
        };
        // Dosya satırdan önce yazılır (hyprlang olmayan `source`'u hata sayar).
        let mut base_changed = false;
        if matches!(base_plan, SourcePlan::Add | SourcePlan::Present) && base_old.as_deref() != Some(base_text.as_str()) {
            match ext::write_derived(&self.env.apply, &base, base_text.as_bytes()) {
                Ok(_) => {
                    base_changed = true;
                    status.wrote = true;
                }
                Err(e) => {
                    status.errors.push(format!("{} yazılamadı: {}", kind.base_rel(), ext::error_text(&e))); // i18n: tanı msg hypr status hata satırı (çevrilmez)
                    base_plan = SourcePlan::Off;
                }
            }
        }
        status.base = Some(base_plan.name());
        status.base_file = matches!(base_plan, SourcePlan::Add | SourcePlan::Present).then(|| self.show(&base));
        // Başa eklenen tek satır (açıklaması aynı satırda).
        let base_frags: Vec<(Fragment, FragValue)> = match (&base_markers, base_plan) {
            (Some((_, line)), SourcePlan::Add) => {
                vec![(Fragment::Line { file: config.clone(), marker: line.clone(), at: Position::Start }, FragValue::Text(line.clone()))]
            }
            _ => Vec::new(),
        };

        // Üretilen dosya: kendi dosyamız (ya da yok) → snapshot'sız; yabancı içerik yalnızca
        // satır eklenirken ya da satır varken ilk kez (işaret yokken), `hypr.source`
        // snapshot'ıyla. İşaret devralınca da yazılır: kullanıcı o snapshot'ı geri yükleyip
        // kendi dosyasını geri aldıysa sonraki tur onu yeniden ezmez (`ensure_source`
        // kapatılıp açılınca işaret silinir, yeniden devralınır).
        let current = Self::read_generated(&target);
        let old = match &current {
            GenState::Ours(t) => Some(t.clone()),
            _ => None,
        };
        let take_over = current == GenState::Foreign
            && (plan == SourcePlan::Add || (plan == SourcePlan::Present && !previously_added));
        if current == GenState::Foreign && !take_over {
            status.errors.push(format!("{}: Nuxut'a ait olmayan dosya; dokunulmadı", self.show(&target))); // i18n: tanı msg hypr status hata satırı (çevrilmez)
        } else if !take_over && old.as_deref() != Some(generated.text.as_str()) {
            match ext::write_derived(&self.env.apply, &target, generated.text.as_bytes()) {
                Ok(_) => status.wrote = true,
                Err(e) => {
                    status.source = "error";
                    status.errors.push(format!("{} yazılamadı: {}", kind.generated_name(), ext::error_text(&e))); // i18n: tanı msg hypr status hata satırı (çevrilmez)
                    return status;
                }
            }
        }

        let managed = Fragment::ManagedFile { file: target.clone() };
        let managed_value = FragValue::File { bytes: generated.text.clone().into_bytes(), mode: 0o644 };
        let change = match plan {
            SourcePlan::Add => {
                let Some(markers) = kind.include_markers(&self.show(&target)) else {
                    status.source = "error";
                    status.errors.push("dahil etme satırı kurulamadı (dosya yolu yazdırılamıyor)".into()); // i18n: tanı msg hypr status hata satırı (çevrilmez)
                    return status;
                };
                let mut c = Change::new(ext::setting(HYPR_SOURCE), t!("shell-hypr-sync-change-added"), Source::Shell);
                for marker in markers {
                    c = c.write(Fragment::Line { file: config.clone(), marker: marker.clone(), at: Position::End }, FragValue::Text(marker));
                }
                Some(if take_over { c.write(managed, managed_value) } else { c })
            }
            SourcePlan::Present if take_over => Some(
                Change::new(ext::setting(HYPR_SOURCE), t!("shell-hypr-sync-change-takeover"), Source::Shell)
                    .write(managed, managed_value),
            ),
            _ => None,
        };
        // Temel katman satırları aynı işleme (ilk bağlanış) ya da tek başına (sonradan gelen katman).
        let adding_base = !base_frags.is_empty();
        let change = match (change, adding_base) {
            (Some(c), _) => Some(base_frags.into_iter().fold(c, |c, (f, v)| c.write(f, v))),
            (None, true) => Some(
                base_frags
                    .into_iter()
                    .fold(Change::new(ext::setting(HYPR_SOURCE), t!("shell-hypr-sync-change-base-added"), Source::Shell), |c, (f, v)| c.write(f, v)),
            ),
            (None, false) => None,
        };
        let mut base_added = false;
        if let Some(change) = change {
            let adding = plan == SourcePlan::Add;
            match self.applier().apply(change) {
                Outcome::Applied { id } => {
                    status.snapshot = Some(id.as_str().to_owned());
                    status.wrote |= take_over;
                    if adding {
                        status.source = "added";
                        status.added = true;
                    }
                    if (adding || take_over)
                        && let Err(e) = self.set_mark()
                    {
                        tracing::warn!("Hyprland eşitleme durum işareti yazılamadı: {e}");
                    }
                    if adding_base {
                        base_added = true;
                        status.base = Some("added");
                        if let Err(e) = self.set_base_mark() {
                            tracing::warn!("temel katman durum işareti yazılamadı: {e}");
                        }
                    }
                }
                // Satırlar zaten birebir vardı (tarama farklı görmüş olabilir).
                Outcome::Noop => {
                    status.source = if adding { "present" } else { status.source };
                    if adding_base {
                        status.base = Some("present");
                    }
                }
                other => {
                    status.source = "error";
                    status.errors.push(format!("dahil etme satırı eklenemedi: {}", other.name())); // i18n: tanı msg hypr status hata satırı (çevrilmez)
                }
            }
        }

        // Canlı uygulama: yalnızca dosya gerçekten yükleniyorsa ve çalışan Hyprland aynı dili
        // kullanıyorsa. Temel katman değiştiyse (kısayol, kural) yeniden okuma.
        if let Some(ctl) = ctl
            && (status.source == "present" || status.added)
            && !status.pending_switch
        {
            let base_live = matches!(status.base, Some("present" | "added"));
            let action = match (status.added || base_added || (base_changed && base_live), kind) {
                (true, _) => ApplyAction::Reload,
                (false, ConfigKind::Hyprlang) => plan_apply(old.as_deref(), &generated),
                (false, ConfigKind::Lua) => plan_apply_lua(old.as_deref(), &generated),
            };
            match apply(ctl, &action, &[kind.generated_name(), kind.base_rel()]) {
                Ok(errors) => {
                    // Bir `keyword` reddedildiyse uyuşmazlık kullanıcının değil bizim hatamız:
                    // override sayılmasın.
                    status.applied = errors.is_empty();
                    status.errors.extend(errors);
                }
                Err(e) => status.errors.push(format!("Hyprland'e uygulanamadı: {e}")), // i18n: tanı msg hypr status hata satırı (çevrilmez)
            }
        }
        status
    }

    /// "Şimdi uygula" (ADR-0003 Karar 4, karar 2): diskteki ayar dili çalışan
    /// Hyprland'inkinden farklıysa (`pending_switch`) `reload full-reset` gönderir (Hyprland ayar
    /// yolunu yeniden seçer). Geçiş beklemiyorsa ya da Hyprland'e erişim yoksa hiçbir komut
    /// gönderilmez. Dönüş: üretilen dosyaya ait `configerrors` satırları. Çağıran ardından
    /// normal bir eşitleme turu çalıştırır (durum yenilenir).
    pub fn apply_switch(&self, ctl: Option<&dyn HyprControl>) -> Result<Vec<String>, String> {
        let Paths { kind, .. } = self.paths().map_err(|s| s.reason.clone().unwrap_or_else(|| s.source.to_owned()))?;
        let ctl = ctl.ok_or("Hyprland'e komut gönderilemez (test örneği ya da bağlantı yok)")?; // i18n: tanı msg yanıtı (çevrilmez)
        match live_provider(ctl) {
            Some(p) if p != kind.provider() => apply(ctl, &ApplyAction::FullReset, &[kind.generated_name(), kind.base_rel()]),
            Some(_) => Err("geçiş bekleyen ayar yok: çalışan Hyprland zaten bu ayar dilinde".into()), // i18n: tanı msg yanıtı (çevrilmez)
            None => Err("çalışan Hyprland'in ayar dili okunamadı".into()), // i18n: tanı msg yanıtı (çevrilmez)
        }
    }

    /// "Geri al" (ADR-0003 Karar 4): yalnızca Nuxut'un eklediği satırlar ve ürettiği dosya(lar)
    /// kaldırılır — tek `hypr.source` işlemi (snapshot'lı: `msg snapshot restore` ile geri
    /// alınabilir). Durum işareti kalır (bir daha kendiliğinden eklenmez — `removed_by_user`;
    /// yeniden bağlamak: `msg hypr add-source` ya da `ensure_source` kapatıp açmak).
    pub fn undo(&self, ctl: Option<&dyn HyprControl>) -> SyncStatus {
        let Paths { config, kind, target, base } = match self.paths() {
            Ok(p) => p,
            Err(s) => return *s,
        };
        let mut status = SyncStatus {
            source: "undone",
            config: Some(self.show(&config)),
            kind: Some(kind),
            generated: Some(self.show(&target)),
            ..SyncStatus::default()
        };
        // Etkin dosya + (Lua'ya geçilmişse) yanındaki eski hyprland.conf'taki satırlar.
        let mut jobs = vec![(config.clone(), kind, target.clone(), base.clone())];
        if kind == ConfigKind::Lua
            && let Some(dir) = config.parent()
        {
            let conf = dir.join("hyprland.conf");
            if conf.is_file() {
                jobs.push((conf, ConfigKind::Hyprlang, dir.join(ConfigKind::Hyprlang.generated_name()), dir.join(ConfigKind::Hyprlang.base_rel())));
            }
        }
        // Kaynak `Cli`: kullanıcının komutu (bildirim düğmesi / `msg hypr undo`); eklemeyle
        // (`Shell`) 2 sn içinde tek snapshot'ta birleşmez, kendi geri alınabilir kaydı olur.
        let mut change = Change::new(ext::setting(HYPR_SOURCE), t!("shell-hypr-sync-change-undone"), Source::Cli);
        for (file, k, generated, base) in jobs {
            let Some(markers) = k.include_markers(&self.show(&generated)) else { continue };
            if file == config {
                let text = read_limited(&file, nuxut_apply::MAX_FILE_BYTES).map(|b| String::from_utf8_lossy(&b).into_owned());
                let present = text.is_some_and(|t| nuxut_apply::edit::line::get(&t, &markers[1]).is_some());
                if !present {
                    status.errors.push(format!("{}: Nuxut satırı birebir bulunamadı (elle değiştirilmiş olabilir); dokunulmadı", self.show(&file))); // i18n: tanı msg hypr status hata satırı (çevrilmez)
                }
            }
            for marker in markers {
                change = change.write(Fragment::Line { file: file.clone(), marker, at: Position::End }, FragValue::Absent);
            }
            // Temel katman satırları (başta) ve dosyası.
            if let Some((_, marker)) = k.base_markers(&self.show(&base)) {
                change = change.write(Fragment::Line { file: file.clone(), marker, at: Position::Start }, FragValue::Absent);
            }
            for generated in [generated, base] {
                match Self::read_generated(&generated) {
                    GenState::Ours(_) => change = change.write(Fragment::ManagedFile { file: generated }, FragValue::Absent),
                    GenState::Foreign => status.errors.push(format!("{}: Nuxut'a ait olmayan dosya; silinmedi", self.show(&generated))),
                    GenState::Absent => {}
                }
            }
        }
        match self.applier().apply(change) {
            Outcome::Applied { id } => status.snapshot = Some(id.as_str().to_owned()),
            Outcome::Noop => {}
            other => status.errors.push(format!("geri alınamadı: {}", other.name())), // i18n: tanı msg hypr status hata satırı (çevrilmez)
        }
        if let Err(e) = self.set_mark() {
            status.errors.push(format!("durum işareti: {e}")); // i18n: tanı msg hypr status hata satırı (çevrilmez)
        }
        if let Err(e) = self.set_base_mark() {
            status.errors.push(format!("durum işareti: {e}")); // i18n: tanı msg hypr status hata satırı (çevrilmez)
        }
        if let Some(ctl) = ctl {
            match apply(ctl, &ApplyAction::Reload, &[kind.generated_name(), kind.base_rel()]) {
                Ok(errors) => status.errors.extend(errors),
                Err(e) => status.errors.push(format!("Hyprland yeniden yüklenemedi: {e}")), // i18n: tanı msg hypr status hata satırı (çevrilmez)
            }
        }
        status
    }

    /// `msg hypr restore-backup` (ADR-0007): en yeni ana ayar yedeğini geri koyar (o anki dosya önce
    /// yedeklenir), sonra Hyprland yeniden yüklenir.
    pub fn restore_backup(&self, ctl: Option<&dyn HyprControl>) -> super::takeover::TakeoverReport {
        let mut r = super::takeover::restore_backup(&self.env, None);
        if r.result == "restored"
            && let (Ok(Paths { kind, .. }), Some(ctl)) = (self.paths(), ctl)
        {
            match apply(ctl, &ApplyAction::Reload, &[kind.generated_name(), kind.base_rel()]) {
                Ok(errors) => r.errors.extend(errors),
                Err(e) => r.errors.push(format!("Hyprland yeniden yüklenemedi: {e}")), // i18n: tanı msg hata ayrıntısı (çevrilmez)
            }
        }
        r
    }

    /// `hypr.source` snapshot'ı başka yoldan (`msg snapshot restore`) geri yüklendi: Hyprland
    /// yeniden yüklenir (satır eklendi/kaldırıldı). Durum çağıranın sonraki eşitleme turunda.
    pub fn reload_after_restore(&self, ctl: Option<&dyn HyprControl>) -> Vec<String> {
        let (Ok(Paths { kind, .. }), Some(ctl)) = (self.paths(), ctl) else { return Vec::new() };
        match apply(ctl, &ApplyAction::Reload, &[kind.generated_name(), kind.base_rel()]) {
            Ok(errors) => errors,
            Err(e) => vec![format!("Hyprland yeniden yüklenemedi: {e}")], // i18n: tanı msg hypr status hata satırı (çevrilmez)
        }
    }
}

/// Lua için [`plan_source`] karşılığı (uzantı denetimi hariç aynı tablo).
fn lua_plan(chain: &ChainReport, req: &SyncRequest, previously_added: bool) -> SourcePlan {
    if !(req.compositor.ensure_source || req.force_add) {
        SourcePlan::Off
    } else if chain.source_at.is_some() {
        SourcePlan::Present
    } else if chain.truncated {
        SourcePlan::Unknown
    } else if chain.commented_at.is_some() {
        SourcePlan::DisabledByUser
    } else if previously_added && !req.force_add {
        SourcePlan::RemovedByUser
    } else {
        SourcePlan::Add
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use nuxut::Theme;

    use super::*;
    use nuxut_apply::SettingId;
    use crate::services::hypr_config::generate::{BlurLook, start_command};
    use crate::services::hypr_config::settings::KeybindsConfig;

    /// Sahte Hyprland: komutları kaydeder; `binds`/`status` yanıtları verilebilir.
    #[derive(Default)]
    struct Fake {
        sent: RefCell<Vec<String>>,
        binds: String,
        provider: Option<&'static str>,
    }

    impl HyprControl for Fake {
        fn request(&self, command: &str) -> Result<String, String> {
            self.sent.borrow_mut().push(command.to_owned());
            Ok(match command {
                "configerrors" => String::new(),
                "j/binds" => if self.binds.is_empty() { "[]".into() } else { self.binds.clone() },
                "j/status" => self.provider.map_or_else(|| "{}".into(), |p| format!("{{\"configProvider\":\"{p}\"}}")),
                _ => "ok".into(),
            })
        }
    }

    impl Fake {
        /// Sorgular hariç gönderilen yazma komutları.
        fn writes(&self) -> Vec<String> {
            self.sent.borrow().iter().filter(|c| !c.starts_with("j/") && *c != "configerrors").cloned().collect()
        }
    }

    struct Setup {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        hypr: PathBuf,
        env: SyncEnv,
    }

    fn setup(file: Option<(&str, &str)>) -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let xdg = home.join("xdg-config");
        let hypr = xdg.join("hypr");
        fs::create_dir_all(&hypr).unwrap();
        if let Some((name, text)) = file {
            fs::write(hypr.join(name), text).unwrap();
        }
        let env = SyncEnv {
            apply: Env {
                config_home: Some(xdg),
                state_home: Some(home.join("state")),
                hyprland: Some("sahte_imza".into()),
                ..Env::for_test(tmp.path())
            },
            runtime_dir: Some(tmp.path().join("run")),
            isolated_control: false,
            under_systemd: false,
        };
        Setup { _tmp: tmp, home, hypr, env }
    }

    fn request() -> SyncRequest {
        let theme = Theme::default();
        SyncRequest {
            compositor: CompositorConfig::default(),
            look: Some(LookGroup::from_theme(&theme, Some(BlurLook { size: 8, passes: 2 }))),
            motion: Some(MotionGroup::from_theme(&theme)),
            windows: Some(WindowsGroup::from_theme(&theme, 4, 8, 2)),
            keybinds: Some(KeybindsConfig::default().map),
            key_owner: Default::default(),
            hide_defaults: true,
            shell_exe: "/usr/bin/nuxut-shell".into(),
            session_env: Vec::new(),
            cursor_env: Vec::new(),
            input: None,
            force_add: false,
        }
    }

    fn snapshots(s: &Setup) -> Vec<nuxut_apply::SnapshotSummary> {
        Applier::open(s.env.apply.clone()).list(&nuxut_apply::Filter::default()).unwrap()
    }

    #[test]
    fn guard_rules() {
        let s = setup(None);
        assert!(s.env.guard().is_ok());
        assert!(!s.env.may_control(), "test örneği Hyprland'e asla komut göndermez");
        let mut e = s.env.clone();
        e.apply.hyprland = None;
        assert!(e.guard().is_err());
        let mut e = s.env.clone();
        e.apply.hyprland = Some("../x".into());
        assert!(e.guard().is_err());
        let mut e = s.env.clone();
        e.apply.config_home = None;
        assert!(e.guard().is_err(), "test örneği XDG_CONFIG_HOME'suz yazmaz");
        let mut e = s.env.clone();
        e.apply.config_home = e.apply.real_home.as_ref().map(|h| h.join(".config"));
        assert!(e.guard().is_err(), "test örneği kullanıcının (gerçek) ~/.config'ine yazmaz");
        let mut e = s.env.clone();
        e.apply.disabled = true;
        assert!(e.guard().is_err() && !e.may_control(), "kilitsiz örnek");
        let mut e = s.env.clone();
        e.apply.test_instance = false;
        assert!(e.may_control());
        // İç içe Hyprland: açık izin + /run/user dışında çalışma dizini.
        let mut e = s.env.clone();
        e.isolated_control = true;
        assert!(e.may_control());
        e.runtime_dir = Some("/run/user/1000".into());
        assert!(!e.may_control(), "kullanıcının çalışma dizinindeki Hyprland'e asla");
        assert!(!isolated_runtime(false, Some(Path::new("/tmp/x"))));
        assert!(!isolated_runtime(true, Some(Path::new("/run/user/1000"))));
        assert!(!isolated_runtime(true, Some(Path::new("/tmp/../run/user/1000"))));
        assert!(!isolated_runtime(true, Some(Path::new("rel"))));
        assert!(!isolated_runtime(true, None));
        assert!(isolated_runtime(true, Some(Path::new("/tmp/nuxut-test/rt"))));
        // Test örneğinde yol XDG_CONFIG_HOME altında; ~/.config aranmaz.
        assert!(s.env.config_path().unwrap().starts_with(s.env.apply.config_home.as_ref().unwrap()));
        assert_eq!(s.env.bases(), [s.env.apply.config_home.clone().unwrap()]);
        assert!(s.home.is_dir());
    }

    #[test]
    fn socket_path_symlink_cannot_reach_user_session() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = tmp.path().join("rt");
        std::fs::create_dir_all(rt.join("hypr/sig")).unwrap();
        std::fs::write(rt.join("hypr/sig/.socket.sock"), "").unwrap();
        assert!(isolated_socket_path(true, Some(&rt), Some("sig")));
        assert!(!isolated_socket_path(false, Some(&rt), Some("sig")));
        assert!(!isolated_socket_path(true, Some(&rt), Some("yok")), "yol yoksa izin yok");
        assert!(!isolated_socket_path(true, Some(&rt), Some("../x")));
        assert!(!isolated_socket_path(true, None, Some("sig")));
        // `hypr` bağlantısı kullanıcının /run/user dizinine yönlendirirse reddedilir.
        if let Some(user_hypr) = std::fs::read_dir("/run/user")
            .ok()
            .and_then(|mut d| d.find_map(|e| e.ok().map(|e| e.path().join("hypr"))))
            .filter(|p| p.is_dir())
        {
            let rt2 = tmp.path().join("rt2");
            std::fs::create_dir_all(&rt2).unwrap();
            std::os::unix::fs::symlink(&user_hypr, rt2.join("hypr")).unwrap();
            let sig = std::fs::read_dir(&user_hypr).ok().and_then(|mut d| d.find_map(|e| e.ok()?.file_name().into_string().ok()));
            if let Some(sig) = sig {
                assert!(!isolated_socket_path(true, Some(&rt2), Some(&sig)));
            }
        }
    }

    #[test]
    fn lua_wins_over_conf_and_environ_parsing() {
        let s = setup(Some(("hyprland.conf", "")));
        assert!(s.env.config_path().unwrap().ends_with("hyprland.conf"));
        fs::write(s.hypr.join("hyprland.lua"), "").unwrap();
        assert!(s.env.config_path().unwrap().ends_with("hyprland.lua"), "hyprland.lua varsa .conf yok sayılır");
        assert_eq!(ConfigKind::of(Path::new("/a/b.lua")), Some(ConfigKind::Lua));
        assert_eq!(ConfigKind::of(Path::new("/a/b.ini")), None);
        let env = b"PATH=/bin\0HYPRLAND_CONFIG=/x/h.lua\0SECRET=s\0";
        assert_eq!(environ_value(env, "HYPRLAND_CONFIG").as_deref(), Some("/x/h.lua"));
        assert_eq!(environ_value(env, "HYPRLAND"), None);
        // Hiç dosya yoksa: Hyprland'in üreteceği hyprland.lua (yazılmaz).
        let s = setup(None);
        assert!(s.env.config_path().unwrap().ends_with("hypr/hyprland.lua"));
        assert_eq!(Syncer::new(s.env.clone()).run(&request(), None).source, "missing_config");
        assert!(!s.hypr.join("nuxut.lua").exists());
    }

    #[test]
    fn conf_first_run_adds_once_then_keywords() {
        let s = setup(Some(("hyprland.conf", "source = ~/xdg-config/hypr/old.conf\n")));
        fs::write(s.hypr.join("old.conf"), "exec-once = nuxut-shell\ndecoration:rounding = 3\nbind = SUPER, N, exec, foo\n").unwrap();
        let mut req = request();
        req.keybinds.as_mut().unwrap().insert("panel_right".into(), "SUPER+N".into());
        let syncer = Syncer::new(s.env.clone());
        let fake = Fake::default();
        let st = syncer.run(&req, Some(&fake));
        assert_eq!(st.source, "added", "{st:?}");
        assert_eq!(st.kind, Some(ConfigKind::Hyprlang));
        assert_eq!(st.launch, Some("external"));
        // Kabuk eylemi, satır sona eklendiği için ondan önce okunan bind'i devralır.
        assert!(st.conflicts.is_empty(), "{:?}", st.conflicts);
        assert_eq!(st.taken_over.len(), 1);
        assert_eq!((st.taken_over[0].action, st.taken_over[0].line), ("panel_right", Some(3)));
        // old.conf kaynaktan önce okunur (satır sona eklenir): Nuxut onu ezer, override değil.
        assert!(st.overrides.is_empty(), "{:?}", st.overrides);
        let generated = fs::read_to_string(s.hypr.join("nuxut.conf")).unwrap();
        assert!(generated.starts_with("# Nuxut — "));
        assert!(generated.contains("\nunbind = SUPER, n\nbindd = SUPER, n, nuxut:panel_right, exec, "), "{generated}");
        assert!(!fs::read_to_string(s.hypr.join("old.conf")).unwrap().contains("unbind"), "kullanıcının dosyası değişmez");
        // Kapatılınca devralma yok: kullanıcınınki çalışır, Nuxut'unki üretilmez.
        let mut off = req.clone();
        off.compositor.take_over_keys = false;
        let st_off = syncer.run(&off, Some(&Fake::default()));
        assert_eq!((st_off.conflicts.len(), st_off.taken_over.len()), (1, 0));
        assert!(!fs::read_to_string(s.hypr.join("nuxut.conf")).unwrap().contains("unbind"));
        syncer.run(&req, Some(&Fake::default()));
        assert!(!generated.contains("exec-once"), "harici başlatıcı var: çift başlatma yok");
        let main = fs::read_to_string(s.hypr.join("hyprland.conf")).unwrap();
        assert!(main.ends_with("source = ~/xdg-config/hypr/nuxut.conf\n"));
        assert_eq!(fake.writes(), ["reload config-only"]);

        let fake2 = Fake::default();
        let st2 = syncer.run(&req, Some(&fake2));
        assert_eq!(st2.source, "present");
        assert!(!st2.wrote && fake2.writes().is_empty());

        let mut req3 = req.clone();
        req3.windows.as_mut().unwrap().gaps_out = 20;
        let fake3 = Fake::default();
        syncer.run(&req3, Some(&fake3));
        let w = fake3.writes();
        assert!(w[0].starts_with("[[BATCH]]keyword ") && w[0].contains("general:gaps_out 20"));
    }

    #[test]
    fn lua_first_run_includes_with_pcall_and_launch_hook() {
        let s = setup(Some(("hyprland.lua", "hl.config({ general = { gaps_in = 1 } })\n")));
        let syncer = Syncer::new(s.env.clone());
        let fake = Fake {
            binds: r#"[{"modmask":64,"key":"N","description":""},{"modmask":64,"key":"B","description":"nuxut:bar_toggle"}]"#.into(),
            provider: Some("lua"),
            ..Fake::default()
        };
        let mut req = request();
        let kb = req.keybinds.as_mut().unwrap();
        kb.insert("panel_right".into(), "SUPER+N".into());
        kb.insert("bar_toggle".into(), "SUPER+B".into());
        let st = syncer.run(&req, Some(&fake));
        assert_eq!(st.source, "added", "{st:?}");
        assert_eq!(st.kind, Some(ConfigKind::Lua));
        assert_eq!(st.launch, Some("exec_once"));
        assert_eq!(st.conflicts.len(), 1, "yalnızca kullanıcının SUPER+N'si; bizim SUPER+B sayılmaz");
        assert_eq!(st.conflicts[0].action, "panel_right");
        let main = fs::read_to_string(s.hypr.join("hyprland.lua")).unwrap();
        // Temel katman başta, kullanıcının satırları ortada, Nuxut değerleri sonda.
        assert!(main.starts_with(&format!("{}  {}\nhl.config({{ general = {{ gaps_in = 1 }} }})\n", base::LUA_INCLUDE_LINE, base::LUA_NOTE)), "{main}");
        assert!(main.ends_with("\npcall(require, \"nuxut\")\n"));
        assert_eq!(st.base, Some("added"));
        let module = fs::read_to_string(s.hypr.join("nuxut.lua")).unwrap();
        assert!(module.contains(&super::super::lua::lua_str(&start_command("/usr/bin/nuxut-shell"))));
        let base_module = fs::read_to_string(s.hypr.join("nuxut/base.lua")).unwrap();
        assert!(!base_module.contains("panel toggle right"));
        assert!(base_module.contains("msg bar toggle") && !module.contains("hl.bind("));
        assert_eq!(fake.writes(), ["reload config-only"]);

        // Değer değişimi → eval; bind değişimi → reload.
        let mut req2 = req.clone();
        req2.windows.as_mut().unwrap().gaps_out = 30;
        let fake2 = Fake { binds: fake.binds.clone(), provider: Some("lua"), ..Fake::default() };
        syncer.run(&req2, Some(&fake2));
        let w = fake2.writes();
        assert_eq!(w.len(), 1);
        assert!(w[0].starts_with("eval hl.config({") && w[0].contains("gaps_out = 30") && !w[0].contains('\n'), "{w:?}");
        let mut req3 = req2.clone();
        req3.keybinds.as_mut().unwrap().insert("panel_left".into(), "SUPER+L".into());
        let fake3 = Fake { binds: fake.binds.clone(), provider: Some("lua"), ..Fake::default() };
        syncer.run(&req3, Some(&fake3));
        assert_eq!(fake3.writes(), ["reload config-only"]);

        // Çalışan Hyprland hâlâ hyprlang → canlı uygulama yok.
        let mut req4 = req3.clone();
        req4.windows.as_mut().unwrap().gaps_out = 31;
        let fake4 = Fake { provider: Some("hyprlang"), ..Fake::default() };
        let st4 = syncer.run(&req4, Some(&fake4));
        assert!(st4.pending_switch && fake4.writes().is_empty());

        // "Şimdi uygula": yalnızca geçiş bekliyorsa ve yalnızca full-reset.
        let fake5 = Fake { provider: Some("hyprlang"), ..Fake::default() };
        assert!(syncer.apply_switch(Some(&fake5)).is_ok());
        assert_eq!(fake5.writes(), ["reload full-reset"]);
        let fake6 = Fake { provider: Some("lua"), ..Fake::default() };
        assert!(syncer.apply_switch(Some(&fake6)).is_err());
        assert!(fake6.writes().is_empty());
        let fake7 = Fake::default();
        assert!(syncer.apply_switch(Some(&fake7)).is_err() && fake7.writes().is_empty());
        assert!(syncer.apply_switch(None).is_err());
    }

    #[test]
    fn lua_commented_include_and_external_launch() {
        let s = setup(Some((
            "hyprland.lua",
            "-- pcall(require, \"nuxut\")\nhl.on(\"hyprland.start\", function() hl.exec_cmd(\"uwsm app -- nuxut-shell\") end)\n",
        )));
        let st = Syncer::new(s.env.clone()).run(&request(), None);
        assert_eq!(st.source, "disabled_by_user");
        assert_eq!(st.launch, Some("external"));
        let module = fs::read_to_string(s.hypr.join("nuxut.lua")).unwrap();
        assert!(!module.contains("hl.on(\"hyprland.start\""));
    }

    /// Portal oturum ortamı imleç ortamından önce, üretilen modüle girer (docs/design/portal.md).
    #[test]
    fn session_env_precedes_cursor_env_in_module() {
        let s = setup(Some(("hyprland.lua", "hl.config({ general = { gaps_in = 1 } })\n")));
        let req = SyncRequest {
            session_env: vec![("XDG_CURRENT_DESKTOP".into(), "Nuxut:Hyprland".into())],
            cursor_env: vec![("XCURSOR_SIZE".into(), "24".into())],
            ..request()
        };
        Syncer::new(s.env.clone()).run(&req, None);
        let module = fs::read_to_string(s.hypr.join("nuxut.lua")).unwrap();
        assert!(module.contains("  hl.env(\"XDG_CURRENT_DESKTOP\", \"Nuxut:Hyprland\")\n  hl.env(\"XCURSOR_SIZE\", \"24\")\n"), "{module}");
    }

    #[test]
    fn undo_removes_only_our_block_and_file() {
        let original = "hl.config({ general = { gaps_in = 1 } })\n";
        let s = setup(Some(("hyprland.lua", original)));
        let syncer = Syncer::new(s.env.clone());
        assert_eq!(syncer.run(&request(), None).source, "added");
        // Kullanıcı arada başka satır ekledi.
        let main = s.hypr.join("hyprland.lua");
        let mut text = fs::read_to_string(&main).unwrap();
        text.push_str("hl.config({ general = { gaps_out = 3 } })\n");
        fs::write(&main, &text).unwrap();
        let st = syncer.undo(None);
        assert_eq!(st.source, "undone", "{st:?}");
        assert!(st.errors.is_empty(), "{st:?}");
        assert_eq!(fs::read_to_string(&main).unwrap(), format!("{original}hl.config({{ general = {{ gaps_out = 3 }} }})\n"));
        assert!(!s.hypr.join("nuxut.lua").exists());
        // Bir daha kendiliğinden eklenmez.
        assert_eq!(syncer.run(&request(), None).source, "removed_by_user");
        let mut force = request();
        force.force_add = true;
        assert_eq!(syncer.run(&force, None).source, "added");
    }

    #[test]
    fn conf_removed_by_user_and_off_switch_reenables() {
        let s = setup(Some(("hyprland.conf", "a = 1\n")));
        let syncer = Syncer::new(s.env.clone());
        let st = syncer.run(&request(), None);
        assert_eq!(st.source, "added");
        assert!(st.snapshot.is_some());
        let conf = s.hypr.join("hyprland.conf");
        assert_eq!(fs::read_to_string(&conf).unwrap(), format!("{}\na = 1\n{CONF_COMMENT}\nsource = ~/xdg-config/hypr/nuxut.conf\n", conf_base_line()));
        fs::write(&conf, "a = 1\n").unwrap();
        let st = syncer.run(&request(), None);
        assert_eq!((st.source, st.base), ("removed_by_user", Some("off")));
        assert_eq!(fs::read_to_string(&conf).unwrap(), "a = 1\n");
        let mut off = request();
        off.compositor = CompositorConfig { ensure_source: false, sync_look: false, sync_motion: false, sync_windows: false, sync_keybinds: false, sync_input: false, base_layer: false, take_over_keys: true };
        assert_eq!(syncer.run(&off, None).source, "off");
        assert!(!s.env.state_file().unwrap().exists(), "kapatınca işaret silinir");
        // Yeniden açılınca eklenir.
        assert_eq!(syncer.run(&request(), None).source, "added");
        assert!(fs::read_to_string(&conf).unwrap().ends_with("source = ~/xdg-config/hypr/nuxut.conf\n"));
    }

    /// `[input]`: yalnızca yazılı anahtarlar üretilir (kullanıcının `kb_layout = tr`'si kalır);
    /// override yalnızca yazılı anahtarda ve yalnızca Nuxut dosyasından SONRA okunan atamada;
    /// `sync_input = false` iken satır yok; yalnızca girdi eşitlenirken de dosya yazılır.
    #[test]
    fn input_lines_and_overrides_only_for_written_keys() {
        let s = setup(Some((
            "hyprland.conf",
            "input {\n  kb_layout = tr\n  repeat_rate = 70\n}\nsource = ~/xdg-config/hypr/nuxut.conf\nsource = ~/xdg-config/hypr/late.conf\n",
        )));
        fs::write(s.hypr.join("late.conf"), "input:repeat_delay = 300\ninput {\n  kb_options = caps:escape\n}\n").unwrap();
        let (c, w) = crate::config::ShellConfig::parse("[input.keyboard]\nrepeat_rate = 25\nrepeat_delay = 600\n").unwrap();
        assert!(w.is_empty());
        let mut req = request();
        req.input = Some(c.input.settings);
        let syncer = Syncer::new(s.env.clone());
        let st = syncer.run(&req, None);
        assert_eq!(st.source, "present", "{st:?}");
        let got: Vec<_> = st.overrides.iter().map(|o| (o.key.as_str(), o.file.as_str(), o.line)).collect();
        assert_eq!(got, [("input:repeat_delay", "~/xdg-config/hypr/late.conf", 1)]);
        let generated = fs::read_to_string(s.hypr.join("nuxut.conf")).unwrap();
        assert!(generated.contains("\ninput:repeat_rate = 25\ninput:repeat_delay = 600\n"), "{generated}");
        assert!(!generated.contains("kb_layout") && !generated.contains("kb_options"), "yazılmamış: kullanıcının değeri kalır");

        // sync_input kapalı: girdi satırı yok, girdi override'ı yok.
        let mut off = req.clone();
        off.compositor.sync_input = false;
        off.input = None;
        let st = syncer.run(&off, None);
        assert!(st.overrides.is_empty());
        assert!(!fs::read_to_string(s.hypr.join("nuxut.conf")).unwrap().contains("input:"));

        // Diğer bütün gruplar kapalı, yalnızca girdi: dosya yine üretilir (satır kararı `off`:
        // ensure_source kapalı, mevcut satıra dokunulmaz).
        let generated = s.hypr.join("nuxut.conf");
        let mut only = req.clone();
        only.compositor = CompositorConfig { ensure_source: false, sync_look: false, sync_motion: false, sync_windows: false, sync_keybinds: false, sync_input: true, base_layer: false, take_over_keys: true };
        fs::remove_file(&generated).unwrap();
        syncer.run(&only, None);
        assert!(fs::read_to_string(&generated).unwrap().contains("input:repeat_rate = 25\n"));
        // Girdi de boşsa hiçbir şey eşitlenmez: dosya yazılmaz.
        let mut nothing = only.clone();
        nothing.input = Some(InputSettings::default());
        fs::remove_file(&generated).unwrap();
        assert_eq!(syncer.run(&nothing, None).source, "off");
        assert!(!generated.exists());
    }

    /// Yabancı `nuxut.conf`: yalnızca satır eklenirken `hypr.source` snapshot'ıyla üzerine
    /// yazılır (`.bak` yok); snapshot geri yüklenince satır kalkar, kullanıcının dosyası döner ve
    /// sonraki tur onu ezmez (`removed_by_user`).
    #[test]
    fn foreign_generated_file_is_snapshotted_and_restorable() {
        let s = setup(Some(("hyprland.conf", "a = 1\n")));
        let gen_path = s.hypr.join("nuxut.conf");
        fs::write(&gen_path, "kullanıcının kendi dosyası\n").unwrap();
        let syncer = Syncer::new(s.env.clone());
        let st = syncer.run(&request(), None);
        assert_eq!(st.source, "added", "{st:?}");
        assert!(st.wrote);
        assert!(fs::read_to_string(&gen_path).unwrap().starts_with("# Nuxut — "));
        assert!(fs::read_dir(&s.hypr).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".bak")));
        let list = snapshots(&s);
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].setting.as_str(), list[0].fragments), (HYPR_SOURCE, 4), "iki satır + temel katman satırı + dosya");
        assert_eq!(st.snapshot.as_deref(), Some(list[0].id.as_str()));

        // Geri yükleme (msg snapshot restore): satırlar kalkar, yabancı dosya geri gelir.
        let out = ext::snapshot_restore(&s.env.apply, &list[0].id, nuxut_apply::RestoreMode::Safe, None);
        assert!(out.json.starts_with("{\"result\":\"restored\""), "{}", out.json);
        assert_eq!(fs::read_to_string(s.hypr.join("hyprland.conf")).unwrap(), "a = 1\n");
        assert_eq!(fs::read_to_string(&gen_path).unwrap(), "kullanıcının kendi dosyası\n");
        let fake = Fake::default();
        assert!(syncer.reload_after_restore(Some(&fake)).is_empty());
        assert_eq!(fake.writes(), ["reload config-only"]);
        let st = syncer.run(&request(), None);
        assert_eq!(st.source, "removed_by_user");
        assert_eq!(fs::read_to_string(&gen_path).unwrap(), "kullanıcının kendi dosyası\n", "yabancı dosya ezilmez");
        assert!(st.errors.iter().any(|e| e.contains("Nuxut'a ait olmayan")), "{st:?}");
    }

    /// Satır zaten varken yabancı `nuxut.conf` bir kez devralınır (snapshot'lı); kullanıcı o
    /// snapshot'ı geri yüklerse sonraki tur dosyayı yeniden ezmez. `ensure_source` kapatılıp
    /// açılınca yeniden devralınır.
    #[test]
    fn present_line_foreign_file_taken_over_once() {
        let s = setup(Some(("hyprland.conf", "a = 1\nsource = ~/xdg-config/hypr/nuxut.conf\n")));
        let gen_path = s.hypr.join("nuxut.conf");
        fs::write(&gen_path, "kendi dosyam\n").unwrap();
        let syncer = Syncer::new(s.env.clone());
        let st = syncer.run(&request(), None);
        assert_eq!(st.source, "present", "{st:?}");
        assert!(st.wrote && st.snapshot.is_some(), "{st:?}");
        assert!(fs::read_to_string(&gen_path).unwrap().starts_with("# Nuxut — "));
        let id = nuxut_apply::SnapshotId::parse(st.snapshot.as_deref().unwrap()).unwrap();
        let out = ext::snapshot_restore(&s.env.apply, &id, nuxut_apply::RestoreMode::Safe, None);
        assert!(out.json.starts_with("{\"result\":\"restored\""), "{}", out.json);
        assert_eq!(fs::read_to_string(&gen_path).unwrap(), "kendi dosyam\n");
        let st = syncer.run(&request(), None);
        assert_eq!(fs::read_to_string(&gen_path).unwrap(), "kendi dosyam\n", "geri yüklenen dosya ezilmez");
        assert!(st.snapshot.is_none() && st.errors.iter().any(|e| e.contains("Nuxut'a ait olmayan")), "{st:?}");
        let mut off = request();
        off.compositor.ensure_source = false;
        syncer.run(&off, None);
        let st = syncer.run(&request(), None);
        assert!(st.snapshot.is_some(), "{st:?}");
        assert!(fs::read_to_string(&gen_path).unwrap().starts_with("# Nuxut — "));
    }

    /// Geri al bir işlemdir: snapshot'ı geri yüklenince satır ve üretilen dosya geri gelir.
    #[test]
    fn undo_is_a_restorable_snapshot() {
        let s = setup(Some(("hyprland.conf", "a = 1\n")));
        let syncer = Syncer::new(s.env.clone());
        assert_eq!(syncer.run(&request(), None).source, "added");
        let conf = s.hypr.join("hyprland.conf");
        let added = fs::read_to_string(&conf).unwrap();
        let st = syncer.undo(None);
        assert!(st.errors.is_empty(), "{st:?}");
        assert_eq!(fs::read_to_string(&conf).unwrap(), "a = 1\n");
        assert!(!s.hypr.join("nuxut.conf").exists());
        let id = nuxut_apply::SnapshotId::parse(st.snapshot.as_deref().unwrap()).unwrap();
        let out = ext::snapshot_restore(&s.env.apply, &id, nuxut_apply::RestoreMode::Safe, ext::known_setting(HYPR_SOURCE).as_ref());
        assert_eq!(out.setting.as_ref().map(SettingId::as_str), Some(HYPR_SOURCE), "{}", out.json);
        assert_eq!(fs::read_to_string(&conf).unwrap(), added);
        assert!(s.hypr.join("nuxut.conf").is_file());
        assert_eq!(syncer.run(&request(), None).source, "present");
        // Eski sürümün eklediği blok (boş satır + yorum + source) da geri alınır.
        fs::write(&conf, format!("a = 1\n\n{CONF_COMMENT}\nsource = ~/xdg-config/hypr/nuxut.conf\n")).unwrap();
        let st = syncer.undo(None);
        assert!(st.errors.is_empty(), "{st:?}");
        assert_eq!(fs::read_to_string(&conf).unwrap(), "a = 1\n\n");
        // Satır yoksa: hata, dosyaya dokunulmaz.
        let st = syncer.undo(None);
        assert!(st.errors.iter().any(|e| e.contains("birebir bulunamadı")), "{st:?}");
    }

    /// Kullanıcının dosyası dışarıdan yönetiliyorsa (salt okunur) satır eklenmez, snapshot da
    /// kalmaz.
    #[test]
    fn readonly_main_file_is_externally_managed() {
        use std::os::unix::fs::PermissionsExt;
        let s = setup(Some(("hyprland.conf", "a = 1\n")));
        let conf = s.hypr.join("hyprland.conf");
        fs::set_permissions(&conf, fs::Permissions::from_mode(0o444)).unwrap();
        let st = Syncer::new(s.env.clone()).run(&request(), None);
        assert_eq!(st.source, "error");
        assert!(st.errors.iter().any(|e| e.contains("externally_managed")), "{st:?}");
        assert_eq!(fs::read_to_string(&conf).unwrap(), "a = 1\n");
        assert!(snapshots(&s).is_empty());
        assert!(!s.env.state_file().unwrap().exists());
    }

    #[test]
    fn inactive_without_hyprland() {
        let mut s = setup(Some(("hyprland.lua", "")));
        s.env.apply.hyprland = None;
        let fake = Fake::default();
        let st = Syncer::new(s.env.clone()).run(&request(), Some(&fake));
        assert_eq!(st.source, "inactive");
        assert!(fake.sent.borrow().is_empty());
        assert!(!s.hypr.join("nuxut.lua").exists());
    }

    #[test]
    fn live_modmask_mapping() {
        let c = combo_from_live(64 | 1, "N").unwrap();
        assert_eq!(c.to_string(), "SUPER+SHIFT+n");
        assert_eq!(combo_from_live(4 | 8, "x").unwrap().mods, MOD_CTRL | MOD_ALT);
    }

    /// Test kökündeki hyprlang temel katman satırı.
    fn conf_base_line() -> String {
        format!("source = ~/xdg-config/hypr/{}  {}", base::CONF_NAME, base::CONF_NOTE)
    }

    /// Temel katman (ADR-0006): satır BAŞA eklenir, dosya Nuxut'un; kullanıcının aynı tuştaki
    /// bind'i varsa Nuxut'unki üretilmez (çift tetikleme yok), kullanıcının dosyası okunmaya devam
    /// eder; kurallar ve varsayılanlar temel dosyada, Nuxut değerleri dosyasında bind yok.
    #[test]
    fn base_layer_first_with_user_binds_winning() {
        let s = setup(Some(("hyprland.conf", "$mod = SUPER\nbind = $mod, Q, exec, kitty\ngeneral:gaps_in = 3\n")));
        let syncer = Syncer::new(s.env.clone());
        let fake = Fake::default();
        let st = syncer.run(&request(), Some(&fake));
        assert_eq!((st.source, st.base), ("added", Some("added")), "{st:?}");
        assert_eq!(st.base_file.as_deref(), Some("~/xdg-config/hypr/nuxut-base.conf"));
        let conf = fs::read_to_string(s.hypr.join("hyprland.conf")).unwrap();
        assert!(conf.starts_with(&format!("{}\n$mod = SUPER\n", conf_base_line())), "{conf}");
        assert!(conf.ends_with("source = ~/xdg-config/hypr/nuxut.conf\n"));
        let base = fs::read_to_string(s.hypr.join(base::CONF_NAME)).unwrap();
        assert!(base.starts_with("# Nuxut — "));
        assert!(!base.contains("nuxut:close_window,"), "kullanıcının SUPER+Q'su kazanır");
        assert!(base.contains("nuxut:close_window_alt, killactive"));
        assert!(base.contains("windowrule = match:modal true, float on, center on\n"));
        let c = st.conflicts.iter().find(|c| c.action == "close_window").expect("çakışma raporlanır");
        assert_eq!((c.file.as_deref(), c.line), (Some("~/xdg-config/hypr/hyprland.conf"), Some(2)), "tarama satır eklenmeden önce");
        let tail = fs::read_to_string(s.hypr.join("nuxut.conf")).unwrap();
        assert!(!tail.contains("\nbind") && !tail.contains("windowrule"));
        assert_eq!(fake.writes(), ["reload config-only"]);
        // Değişiklik yoksa hiçbir şey gönderilmez; temel katman değişirse (kısayol) yeniden okuma.
        let fake2 = Fake::default();
        let st2 = syncer.run(&request(), Some(&fake2));
        assert_eq!((st2.source, st2.base), ("present", Some("present")));
        assert!(fake2.writes().is_empty());
        let mut req = request();
        req.keybinds.as_mut().unwrap().insert("lock".into(), String::new());
        let fake3 = Fake::default();
        syncer.run(&req, Some(&fake3));
        assert_eq!(fake3.writes(), ["reload config-only"]);
        assert!(!fs::read_to_string(s.hypr.join(base::CONF_NAME)).unwrap().contains("nuxut:lock"));
        // Kendi temel dosyamızın bind'leri bir sonraki taramada kullanıcınınki sanılmaz.
        let st4 = syncer.run(&request(), None);
        assert!(st4.conflicts.iter().all(|c| c.action == "close_window"), "{:?}", st4.conflicts);
    }

    /// Satır varken (end4 benzeri zincir): ayar katmanından önceki ses/medya/emoji bind'leri devralınır,
    /// sonraki Print kullanıcıda kalır (Nuxut'unki yeniden bağlanırdı: çift tetikleme); `CAPS`'lı
    /// bind başka tuş birleşimidir, dokunulmaz.
    #[test]
    fn takeover_only_before_settings_layer() {
        let s = setup(Some((
            "hyprland.conf",
            "source = ~/xdg-config/hypr/keys.conf\nsource = ~/xdg-config/hypr/nuxut.conf\nbind = , Print, exec, grim\n",
        )));
        fs::write(
            s.hypr.join("keys.conf"),
            "bindl = , XF86AudioPlay, exec, playerctl play-pause\nbindle = , XF86AudioRaiseVolume, exec, wpctl set-volume @DEFAULT_SINK@ 5%+\nbind = SUPER, period, exec, emoji\nbind = CAPS, XF86AudioMute, exec, x\n",
        )
        .unwrap();
        let syncer = Syncer::new(s.env.clone());
        let st = syncer.run(&request(), Some(&Fake::default()));
        assert_eq!(st.source, "present", "{st:?}");
        let mut taken: Vec<_> = st.taken_over.iter().map(|t| t.action).collect();
        taken.sort_unstable();
        assert_eq!(taken, ["emoji", "media_play", "volume_up"]);
        let play = st.taken_over.iter().find(|t| t.action == "media_play").unwrap();
        assert_eq!((play.file.as_deref(), play.line), (Some("~/xdg-config/hypr/keys.conf"), Some(1)));
        let conflicts: Vec<_> = st.conflicts.iter().map(|c| c.action).collect();
        assert_eq!(conflicts, ["shot_region"]);
        let tail = fs::read_to_string(s.hypr.join("nuxut.conf")).unwrap();
        assert!(tail.contains("unbind = , xf86audioplay\nbindld = , xf86audioplay, nuxut:media_play, exec, playerctl play-pause\n"), "{tail}");
        assert!(!tail.contains("print") && !tail.contains("xf86audiomute"), "{tail}");
        let base = fs::read_to_string(s.hypr.join(base::CONF_NAME)).unwrap();
        assert!(!base.contains("nuxut:media_play") && !base.contains("nuxut:shot_region"), "devralınan temel katmanda değil");
        assert!(base.contains("nuxut:volume_mute"), "CAPS'lı bind farklı tuş: Nuxut'unki temel katmanda");
        let json = st.to_json();
        assert!(json.contains("\"taken_over\":[{"), "{json}");
    }

    /// end4 benzeri zinciri (`tests/fixtures/hyprland/end4-like`) test dizinine kopyalar.
    fn end4_like(s: &Setup) {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hyprland/end4-like");
        for rel in ["hyprland.conf", "hyprland/env.conf", "hyprland/keybinds.conf", "custom/keybinds.conf"] {
            let to = s.hypr.join(rel);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::copy(src.join(rel), &to).unwrap();
        }
    }

    /// Gerçek makinedeki "37 kısayol çakışması" (2026-10-17): end4 düzeninde kabuk eylemlerinin
    /// hepsi devralınır; kalan 37'si temel katman eylemi, kullanıcınınki bilerek kazanır
    /// (`user_wins`, uyarı değil) ve her biri tek tıkla Nuxut'a geçirilebilir.
    #[test]
    fn end4_like_chain_conflicts_are_intended() {
        let s = setup(None);
        end4_like(&s);
        let syncer = Syncer::new(s.env.clone());
        let st = syncer.run(&request(), Some(&Fake::default()));
        assert_eq!(st.source, "present", "{st:?}");
        let mut taken: Vec<_> = st.taken_over.iter().map(|t| t.action).collect();
        taken.sort_unstable();
        assert_eq!(
            taken,
            [
                "brightness_down", "brightness_up", "emoji", "media_next", "media_pause", "media_play", "media_prev", "mic_mute",
                "shot_region", "volume_down", "volume_mute", "volume_up"
            ]
        );
        assert!(st.taken_over.iter().all(|t| t.reason == "default"));
        assert_eq!(st.conflicts.len(), 37, "{:?}", st.conflicts.iter().map(|c| c.action).collect::<Vec<_>>());
        assert!(st.conflicts.iter().all(|c| c.reason == "user_wins" && c.can_take), "{:?}", st.conflicts);
        let q = st.conflicts.iter().find(|c| c.action == "close_window").unwrap();
        assert_eq!((q.combo.as_str(), q.file.as_deref()), ("SUPER+q", Some("~/xdg-config/hypr/hyprland/keybinds.conf")));
        assert!(st.to_json().contains("\"reason\":\"user_wins\",\"can_take\":true"), "{}", st.to_json());

        // "Nuxut'unkini kullan" (pencere kapatma) ve "Benimkini koru" (ses kapat): biri devralınır,
        // öteki kullanıcının seçimi olarak raporlanır. Kullanıcının dosyası değişmez.
        let before = fs::read_to_string(s.hypr.join("hyprland/keybinds.conf")).unwrap();
        let mut req = request();
        req.key_owner.map.insert("close_window".into(), "nuxut".into());
        req.key_owner.map.insert("volume_mute".into(), "user".into());
        let st = syncer.run(&req, Some(&Fake::default()));
        let q = st.taken_over.iter().find(|t| t.action == "close_window").unwrap();
        assert_eq!(q.reason, "chosen");
        let m = st.conflicts.iter().find(|c| c.action == "volume_mute").unwrap();
        assert_eq!((m.reason, m.can_take), ("user_choice", true));
        assert_eq!(st.conflicts.len(), 37);
        let tail = fs::read_to_string(s.hypr.join("nuxut.conf")).unwrap();
        assert!(tail.contains("unbind = SUPER, q\nbindd = SUPER, q, nuxut:close_window, killactive"), "{tail}");
        assert!(!tail.contains("nuxut:volume_mute"), "{tail}");
        assert_eq!(fs::read_to_string(s.hypr.join("hyprland/keybinds.conf")).unwrap(), before);

        // Nuxut satırından SONRA okunan bind: Nuxut'unki istense de devralınamaz → uyarı (neden + yer).
        let main = s.hypr.join("hyprland.conf");
        let text = fs::read_to_string(&main).unwrap() + "source = custom/late.conf\n";
        fs::write(&main, text).unwrap();
        fs::write(s.hypr.join("custom/late.conf"), "bind = Super+Shift, V, exec, cliphist\n").unwrap();
        let st = syncer.run(&req, Some(&Fake::default()));
        let v = st.conflicts.iter().find(|c| c.action == "clipboard").unwrap();
        assert_eq!((v.reason, v.can_take, v.file.as_deref(), v.line), ("read_after", false, Some("~/xdg-config/hypr/custom/late.conf"), Some(1)));
    }

    /// Eski kurulum (sondaki satır var, temel katman yok): katman tek başına, snapshot'lı eklenir.
    /// Kullanıcı yorumlarsa ya da silerse kendiliğinden geri gelmez; geri al ikisini de kaldırır.
    #[test]
    fn base_layer_added_to_existing_install_and_respects_user() {
        let s = setup(Some(("hyprland.lua", "hl.config({ general = { gaps_in = 1 } })\n-- Nuxut\npcall(require, \"nuxut\")\n")));
        let syncer = Syncer::new(s.env.clone());
        let main = s.hypr.join("hyprland.lua");
        let st = syncer.run(&request(), None);
        assert_eq!((st.source, st.base), ("present", Some("added")), "{st:?}");
        assert!(st.snapshot.is_some());
        let base_line = format!("{}  {}", base::LUA_INCLUDE_LINE, base::LUA_NOTE);
        assert!(fs::read_to_string(&main).unwrap().starts_with(&format!("{base_line}\n")));
        assert!(s.hypr.join(base::LUA_REL).is_file());
        // Yorumlanmış satır: kapatılmış, yeniden eklenmez.
        let text = fs::read_to_string(&main).unwrap().replacen(&base_line, &format!("-- {base_line}"), 1);
        fs::write(&main, &text).unwrap();
        assert_eq!(syncer.run(&request(), None).base, Some("disabled_by_user"));
        assert_eq!(fs::read_to_string(&main).unwrap(), text);
        // Silinmiş satır: işaret var → kaldırılmış.
        let text = text.replacen(&format!("-- {base_line}\n"), "", 1);
        fs::write(&main, &text).unwrap();
        assert_eq!(syncer.run(&request(), None).base, Some("removed_by_user"));
        // Zorla ekleme (msg hypr add-source) geri getirir; geri al ikisini de kaldırır.
        let mut force = request();
        force.force_add = true;
        assert_eq!(syncer.run(&force, None).base, Some("added"));
        let st = syncer.undo(None);
        assert!(st.errors.is_empty(), "{st:?}");
        assert_eq!(fs::read_to_string(&main).unwrap(), "hl.config({ general = { gaps_in = 1 } })\n-- Nuxut\n");
        assert!(!s.hypr.join(base::LUA_REL).exists());
    }

    /// `base_layer = false`: temel dosyada yalnızca kabuk eylemlerinin kısayolları (kural yok).
    #[test]
    fn base_layer_off_keeps_shell_binds_only() {
        let s = setup(Some(("hyprland.conf", "a = 1\n")));
        let mut req = request();
        req.compositor.base_layer = false;
        let st = Syncer::new(s.env.clone()).run(&req, None);
        assert_eq!(st.base, Some("added"), "kabuk kısayolları için satır yine gerekli");
        let base = fs::read_to_string(s.hypr.join(base::CONF_NAME)).unwrap();
        assert!(base.contains("nuxut:volume_up") && base.contains("nuxut:magnifier_in"));
        assert!(!base.contains("nuxut:launcher") && !base.contains("windowrule") && !base.contains("gesture"));
    }

    #[test]
    fn base_line_detection() {
        let code = "pcall(require, \"nuxut.base\")";
        assert_eq!(base_line_state(&format!("{code}  -- not\nx\n"), code), BaseLine::Present);
        assert_eq!(base_line_state(&format!("  {code}\n"), code), BaseLine::Present);
        assert_eq!(base_line_state(&format!("-- {code}\n"), code), BaseLine::Commented);
        assert_eq!(base_line_state(&format!("{code}x\n"), code), BaseLine::Absent);
        assert_eq!(base_line_state("pcall(require, \"nuxut\")\n", code), BaseLine::Absent);
        let conf = "source = ~/.config/hypr/nuxut-base.conf";
        assert_eq!(base_line_state(&format!("# {conf}\n"), conf), BaseLine::Commented);
        assert_eq!(base_line_state(&format!("{conf} # ok\n"), conf), BaseLine::Present);
    }

    /// `hyprctl -j binds`: alt eşlemdeki bind genel kısayolla çakışmaz; Lua `code:` bind'i (anahtar
    /// boş, kod 0) görülemez; keycode'lu hyprlang bind'i `code:` olarak okunur.
    #[test]
    fn live_binds_skip_submaps_and_read_keycodes() {
        let fake = Fake {
            binds: r#"[{"modmask":64,"key":"Q","submap":"resize"},{"modmask":64,"key":"","keycode":10},{"modmask":64,"key":"","keycode":0},{"modmask":64,"key":"E","description":"nuxut:file_manager"}]"#.into(),
            ..Fake::default()
        };
        let got: Vec<String> = live_user_binds(&fake).iter().map(ToString::to_string).collect();
        assert_eq!(got, ["SUPER+code:10"]);
    }
}
