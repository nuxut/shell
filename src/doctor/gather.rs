//! [`Facts`] toplama: dosya sistemi, ortam, çalışan kabuğun `msg doctor live` yanıtı. **Yalnız
//! okur**; alt süreç başlatmaz. Dış girdiler (kullanıcı dosyaları, kabuk yanıtı) boyut sınırlı.
//! Ortam ve yan etkili yoklamalar (canlı yanıt, logind, fontlar) çağırandan gelir: testler sahte
//! ev dizini, sahte `/proc` kökü ve sabit yanıtlarla çalışır.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{ClipKind, ClipTool, ConfState, Facts, Font, Install, Live, Lock, PackHit, Packages, Portal, Running, Service, SessionPath, SessionRes};

/// Kurulumun zorunlu ikilileri (çalışan ikilinin dizininde): `install.sh` `BINS` ile aynı
/// (2026-10-16 gerçek vaka: `nuxut-lock` kurulmamıştı; eşleme `tests/it/install_bins.rs`'te).
pub const BINARIES: [&str; 5] = ["nuxut-shell", "nuxut-lock", "nuxut-polkit", "nuxut-settings", "nuxut-calendard"];
/// Yardımcı ikililer (`<önek>/lib/nuxut`): `install.sh` `HELPERS` + portal/seçici ikilileri.
pub const HELPERS: [&str; 4] = ["nuxut-image-worker", "xdg-desktop-portal-nuxut", "nuxut-sharepicker", "nuxut-filechooser"];
/// Kurulumun zorunlu saydığı yardımcılar (`install.sh` `PORTAL_BIN PICKER_BIN CHOOSER_BIN`);
/// görsel işçisi eksikse yalnız dosya duvar kağıtları ve tema önizlemeleri çözülmez: uyarı.
pub const REQUIRED_HELPERS: [&str; 3] = ["xdg-desktop-portal-nuxut", "nuxut-sharepicker", "nuxut-filechooser"];
/// Okunan kullanıcı dosyalarının üst sınırı.
const MAX_FILE: u64 = 1 << 20;
/// Tema paketi taramasında izlenen en çok `source` dosyası (bir düzey).
const MAX_SOURCES: usize = 32;
/// Tema paketi izleri: (küçük harf iz, paket adı).
const PACK_MARKERS: [(&str, &str); 8] = [
    ("illogical-impulse", "end4"),
    ("dots-hyprland", "end4"),
    ("end-4", "end4"),
    ("caelestia", "caelestia"),
    ("ml4w", "ml4w"),
    (".config/hyde", "HyDE"),
    ("jakoolit", "JaKooLit"),
    ("omarchy", "omarchy"),
];
/// `nuxut-portals.conf` ilk satır işareti (portal crate'i `PORTALS_CONF_PREFIX` ile aynı).
const PORTALS_CONF_PREFIX: &str = "# nuxut-managed: portals";
const FILE_CHOOSER_KEY: &str = "org.freedesktop.impl.portal.FileChooser";
/// Önerilen paketler (install.sh `PKG_LOOK`; ekran paylaşımı için xdph). Fontlar `fonts`
/// denetiminde, aileden.
pub const PACKAGES: [&str; 5] =
    ["papirus-icon-theme", "vimix-cursors", "xdg-desktop-portal", "xdg-desktop-portal-gtk", "xdg-desktop-portal-hyprland"];
/// İsteğe bağlı paketler (yalnız bilgi): video duvar kağıdı, harici monitör parlaklığı.
pub const OPTIONAL_PACKAGES: [&str; 2] = ["mpv", "ddcutil"];
/// Nuxut'la aynı işi yapan programlar (`/proc/<pid>/comm`; install.sh "Kontroller" listesi).
const COMPETITORS: [&str; 8] = ["dunst", "mako", "swaync", "waybar", "quickshell", "qs", "ags", "eww"];
/// Hyprland `exec` satırlarında aranan harici pano araçları: (küçük harf iz, araç, tür). Sıra
/// önemli: eşitleyici izleri önce (`wl-paste --watch xclip` bir köprüdür, geçmiş değil).
const CLIP_MARKERS: [(&str, &str, ClipKind); 12] = [
    ("hypr-bridge", "hypr-bridge", ClipKind::Bridge),
    ("xclip", "xclip", ClipKind::Bridge),
    ("xsel", "xsel", ClipKind::Bridge),
    ("clipnotify", "clipnotify", ClipKind::Bridge),
    ("clipsync", "clipsync", ClipKind::Bridge),
    ("cliphist", "cliphist", ClipKind::Manager),
    ("clipman", "clipman", ClipKind::Manager),
    ("copyq", "copyq", ClipKind::Manager),
    ("greenclip", "greenclip", ClipKind::Manager),
    ("clipse", "clipse", ClipKind::Manager),
    ("wl-clip-persist", "wl-clip-persist", ClipKind::Manager),
    ("clipvault", "clipvault", ClipKind::Manager),
];
/// Sürekli çalışan harici pano araçlarının süreç adları (`/proc/<pid>/comm`, ≤ 15 bayt).
/// `wl-paste --watch` ayrıca komut satırından sınıflanır (bkz. [`clipboard_processes`]).
const CLIP_PROCESSES: [(&str, &str, ClipKind); 10] = [
    ("hypr-bridge.sh", "hypr-bridge", ClipKind::Bridge),
    ("hypr-bridge", "hypr-bridge", ClipKind::Bridge),
    ("clipnotify", "clipnotify", ClipKind::Bridge),
    ("clipsync", "clipsync", ClipKind::Bridge),
    ("copyq", "copyq", ClipKind::Manager),
    ("greenclip", "greenclip", ClipKind::Manager),
    ("clipse", "clipse", ClipKind::Manager),
    ("wl-clip-persist", "wl-clip-persist", ClipKind::Manager),
    ("clipman", "clipman", ClipKind::Manager),
    ("clipvault", "clipvault", ClipKind::Manager),
];
/// pacman veritabanında okunan en çok kayıt.
const MAX_DB_ENTRIES: usize = 20_000;
/// `xdph.conf` blok başlangıcı (`dist/xdph/xdph-picker.sh` `BEGIN_PREFIX`).
const SHAREPICKER_BEGIN: &str = "# >>> nuxut-sharepicker";
/// install.sh --fix-path dosyası (`$XDG_CONFIG_HOME` altında).
pub const ENVD_PATH_FILE: &str = "environment.d/50-nuxut-path.conf";

/// Toplamanın girdileri (süreç ortamı ya da test).
#[derive(Debug, Clone)]
pub struct Env {
    pub home: Option<PathBuf>,
    pub config_home: PathBuf,
    pub data_home: PathBuf,
    /// `doctor`'u çalıştıran ikili.
    pub exe: Option<PathBuf>,
    /// Terminalin `PATH`'i.
    pub path: Option<OsString>,
    /// `$SHELL` (yalnız adı kullanılır: PATH önerisi fish/bash'e göre).
    pub shell: Option<OsString>,
    pub current_desktop: Option<String>,
    pub proc_root: PathBuf,
    /// pacman yerel veritabanı (`/var/lib/pacman/local`; yoksa paketler denetlenmez).
    pub pacman_db: PathBuf,
    /// Kaynak dizini (`NUXUT_SRC`, yoksa `~/.local/src/nuxut`): öneri satırları için.
    pub src: PathBuf,
    pub version: String,
    pub commit: Option<String>,
}

impl Env {
    pub fn from_process() -> Self {
        let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
        let base = |var: &str, rel: &str| {
            std::env::var_os(var)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|h| h.join(rel)))
                .unwrap_or_default()
        };
        let src = std::env::var_os("NUXUT_SRC")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".local/src/nuxut")))
            .unwrap_or_default();
        Self {
            config_home: base("XDG_CONFIG_HOME", ".config"),
            data_home: base("XDG_DATA_HOME", ".local/share"),
            exe: std::env::current_exe().ok().map(|e| e.canonicalize().unwrap_or(e)),
            path: std::env::var_os("PATH"),
            shell: std::env::var_os("SHELL"),
            current_desktop: std::env::var("XDG_CURRENT_DESKTOP").ok(),
            proc_root: PathBuf::from("/proc"),
            pacman_db: PathBuf::from("/var/lib/pacman/local"),
            src,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            commit: super::build_commit().map(str::to_owned),
            home,
        }
    }
}

/// Çalışan kabuğun `msg doctor live` yanıtı.
#[derive(Debug, Clone)]
pub enum LiveReply {
    NotRunning,
    /// Kabuk var ama yanıt gelmedi (zaman aşımı, bağlantı hatası).
    Failed,
    Text(String),
}

/// Yan etkili yoklamalar (testlerde sahte).
pub struct Probes<'a> {
    /// Kabuk PID'inin logind oturumu nasıl çözülüyor.
    pub session: &'a dyn Fn(u32) -> SessionRes,
    /// systemd kullanıcı yöneticisinin ortamı (`Manager.Environment`, salt okur); yoksa `None`.
    pub systemd_env: &'a dyn Fn() -> Option<Vec<String>>,
    /// Font ailesi kurulu mu.
    pub font: &'a mut dyn FnMut(&str) -> bool,
    /// Temanın font aileleri (rol, aile).
    pub families: Vec<(&'static str, String)>,
}

impl std::fmt::Debug for Probes<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probes").field("families", &self.families).finish_non_exhaustive()
    }
}

pub fn gather(env: &Env, live: LiveReply, probes: Probes<'_>) -> Facts {
    let bin_dir = env.exe.as_deref().and_then(Path::parent).map(Path::to_path_buf);
    let mut install = install(env, bin_dir.as_deref());
    let live = parse_live(env, live, probes.session);
    install.session_path = session_path(env, bin_dir.as_deref(), &live, (probes.systemd_env)());
    let live_config = match &live {
        Live::Running(r) => r.hypr.as_ref().and_then(|h| h.get("config")).and_then(Value::as_str).map(PathBuf::from),
        _ => None,
    };
    let hypr_config = live_config.or_else(|| default_hypr_config(&env.config_home));
    let scan = hypr_config.as_deref().map(|c| scan_hypr(c, env.home.as_deref())).unwrap_or_default();
    let mut clip_tools = scan.clip;
    clip_tools.extend(clipboard_processes(&env.proc_root));
    let libexec = bin_dir.as_deref().and_then(Path::parent).map(|p| p.join("lib/nuxut")).unwrap_or_default();
    let Probes { font, families, .. } = probes;
    Facts {
        home: env.home.clone(),
        install,
        live,
        packs: scan.packs,
        hypr_config,
        portal: portal(env, &libexec),
        lock: lock(env, bin_dir.as_deref()),
        disk_free: env.home.as_deref().and_then(disk_free),
        fonts: families.into_iter().map(|(role, family)| Font { role, installed: font(&family), family }).collect(),
        packages: packages(&env.pacman_db),
        competitors: competitors(&env.proc_root),
        clip_tools,
    }
}

fn is_executable(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn install(env: &Env, bin_dir: Option<&Path>) -> Install {
    let missing = |dir: Option<&Path>, names: &[&'static str]| -> Vec<&'static str> {
        names.iter().copied().filter(|n| !dir.is_some_and(|d| is_executable(&d.join(n)))).collect()
    };
    let libexec = bin_dir.and_then(Path::parent).map(|p| p.join("lib/nuxut"));
    Install {
        bin_dir: bin_dir.map(Path::to_path_buf),
        missing_bins: missing(bin_dir, &BINARIES),
        total_bins: BINARIES.len(),
        missing_helpers: missing(libexec.as_deref(), &HELPERS),
        total_helpers: HELPERS.len(),
        in_path: bin_dir.is_some_and(|d| crate::self_exe::dir_in_path(d, env.path.as_deref())),
        login_shell: super::LoginShell::from_shell_var(env.shell.as_deref()),
        session_path: SessionPath::default(),
        version: env.version.clone(),
        commit: env.commit.clone(),
        install_sh: env.src.join("shell/install.sh"),
    }
}

/// Dosyanın ilk [`MAX_FILE`] baytı (UTF-8 kayıplı); okunamazsa `None`.
fn read_limited(p: &Path) -> Option<String> {
    let f = std::fs::File::open(p).ok()?;
    let mut buf = Vec::new();
    f.take(MAX_FILE).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn parse_live(env: &Env, live: LiveReply, session: &dyn Fn(u32) -> SessionRes) -> Live {
    let text = match live {
        LiveReply::NotRunning => return Live::NotRunning,
        LiveReply::Failed => return Live::Unreadable,
        LiveReply::Text(t) => t,
    };
    if text.len() > MAX_FILE as usize {
        return Live::Unreadable;
    }
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return Live::Unreadable };
    let Some(pid) = v.get("pid").and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()).filter(|&p| p > 0) else {
        return Live::Unreadable;
    };
    let str_of = |k: &str| v.get(k).and_then(Value::as_str).map(|s| s.chars().filter(|c| !c.is_control()).take(64).collect::<String>());
    let obj = |k: &str| v.get(k).filter(|x| x.is_object()).cloned();
    Live::Running(Box::new(Running {
        pid,
        version: str_of("version").unwrap_or_default(),
        commit: str_of("commit"),
        instance: str_of("instance").unwrap_or_default(),
        service: service(&env.proc_root, pid),
        instances: instances(&env.proc_root, pid, std::process::id()),
        session: session(pid),
        hypr: obj("hypr"),
        look: obj("look"),
        audio: obj("audio"),
        lock_listening: v.get("lock").and_then(|l| l.get("listening")).and_then(Value::as_bool),
        clipboard: obj("clipboard"),
    }))
}

/// Kabuğun cgroup'undan nasıl başlatıldığı (`0::/…/<birim>` satırının son öğesi).
pub fn service(proc_root: &Path, pid: u32) -> Service {
    let Some(text) = read_limited(&proc_root.join(pid.to_string()).join("cgroup")) else { return Service::Unknown };
    let Some(unit) = text.lines().find_map(|l| l.strip_prefix("0::")).and_then(|p| p.trim().rsplit('/').next()) else {
        return Service::Unknown;
    };
    if unit == "nuxut-shell.service" {
        Service::Unit
    } else if unit.starts_with("session-") && unit.ends_with(".scope") {
        Service::LoginSession
    } else if unit.ends_with(".scope") || unit.ends_with(".service") {
        Service::AppScope
    } else {
        Service::Unknown
    }
}

/// Kabukla aynı ikiliyi çalıştıran süreç sayısı (okunabilen `exe` bağları; en az 1).
pub fn instances(proc_root: &Path, pid: u32, client: u32) -> usize {
    let Ok(exe) = std::fs::read_link(proc_root.join(pid.to_string()).join("exe")) else { return 1 };
    let Ok(dir) = std::fs::read_dir(proc_root) else { return 1 };
    let n = dir
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())))
        // Raporu üreten istemci (`nuxut-shell doctor`) aynı ikilidir: sayılmaz.
        .filter(|e| e.file_name().to_str() != Some(client.to_string().as_str()))
        .filter(|e| std::fs::read_link(e.path().join("exe")).is_ok_and(|p| p == exe))
        .count();
    n.max(1)
}

fn default_hypr_config(config_home: &Path) -> Option<PathBuf> {
    let dir = config_home.join("hypr");
    ["hyprland.lua", "hyprland.conf"].into_iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// `~`, `$HOME` ve `$XDG_CONFIG_HOME` açılımı (Hyprland `source` yolları); glob ve diğer
/// değişkenler desteklenmez (atlanır).
fn expand(raw: &str, base: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let p = if let Some(rest) = raw.strip_prefix("~/") {
        home?.join(rest)
    } else if let Some(rest) = raw.strip_prefix("$HOME/") {
        home?.join(rest)
    } else {
        // `./x` → `x`: raporda `…/hypr/./x` görünmesin.
        PathBuf::from(raw.strip_prefix("./").unwrap_or(raw))
    };
    if p.to_str().is_some_and(|s| s.contains(['*', '?', '$'])) {
        return None;
    }
    Some(if p.is_absolute() { p } else { base.join(p) })
}

/// Hyprland ayarı taramasının bulguları.
#[derive(Debug, Default)]
pub struct HyprScan {
    pub packs: Vec<PackHit>,
    /// `exec`/`exec-once` satırlarındaki harici pano araçları (yorum satırları sayılmaz).
    pub clip: Vec<ClipTool>,
}

/// Kullanıcının Hyprland ayarında (ve bir düzey `source` dosyalarında) tema paketi izleri ve
/// harici pano araçları: yalnız paket/araç adı ve `dosya:satır` (satır metni dışarı çıkmaz).
pub fn scan_hypr(config: &Path, home: Option<&Path>) -> HyprScan {
    let mut scan = HyprScan::default();
    let mut sources = Vec::new();
    scan_file(config, home, &mut scan, Some(&mut sources));
    sources.truncate(MAX_SOURCES);
    for s in sources {
        scan_file(&s, home, &mut scan, None);
    }
    scan.packs.dedup();
    scan.clip.dedup();
    scan
}

/// Satır sürekli çalışan bir `exec` mı: hyprlang `exec-once = …` / `exec = …` ya da Lua
/// `hl.exec_once(…)`. Yorumlar ve kısayol satırları (`bind = …, exec, …`: tek seferlik) sayılmaz.
fn is_exec_line(lower: &str) -> bool {
    let t = lower.trim_start();
    if t.starts_with('#') || t.starts_with("--") {
        return false;
    }
    t.starts_with("exec") || t.contains("exec_once(")
}

fn scan_file(file: &Path, home: Option<&Path>, scan: &mut HyprScan, mut sources: Option<&mut Vec<PathBuf>>) {
    let Ok(f) = std::fs::File::open(file) else { return };
    let base = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let reader = BufReader::new(f.take(MAX_FILE));
    for (i, line) in reader.split(b'\n').enumerate() {
        let Ok(line) = line else { break };
        let orig = String::from_utf8_lossy(&line);
        let lower = orig.to_lowercase();
        if let Some((_, pack)) = PACK_MARKERS.iter().find(|(m, _)| lower.contains(m)) {
            scan.packs.push(PackHit { pack, file: file.to_path_buf(), line: i + 1 });
        }
        if is_exec_line(&lower)
            && let Some((_, tool, kind)) = CLIP_MARKERS.iter().find(|(m, ..)| lower.contains(m))
        {
            scan.clip.push(ClipTool { tool, kind: *kind, at: Some((file.to_path_buf(), i + 1)) });
        }
        if let Some(src) = sources.as_deref_mut() {
            // `source = <yol>` (anahtar ASCII, büyük/küçük harf duyarsız; yol özgün satırdan).
            let t = orig.trim_start();
            let rest = t
                .get(..6)
                .filter(|k| k.eq_ignore_ascii_case("source"))
                .and_then(|_| t.get(6..))
                .map(str::trim_start)
                .and_then(|r| r.strip_prefix('='));
            if let Some(rest) = rest {
                let raw = rest.split('#').next().unwrap_or("").trim();
                if let Some(p) = expand(raw, &base, home).filter(|p| p.is_file()) {
                    src.push(p);
                }
            }
        }
    }
}

/// Çalışan harici pano araçları (`/proc/<pid>/comm`). `wl-paste` yalnız `--watch` ile sürekli
/// çalışır: komut satırındaki araç izinden sınıflanır (`xclip` → eşitleyici, `cliphist` →
/// geçmiş; iz yoksa geçmiş sayılır). Komut satırı rapora girmez, yalnız araç kimliği.
pub fn clipboard_processes(proc_root: &Path) -> Vec<ClipTool> {
    let Ok(dir) = std::fs::read_dir(proc_root) else { return Vec::new() };
    let mut found: Vec<ClipTool> = Vec::new();
    for e in dir.filter_map(Result::ok) {
        if !e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())) {
            continue;
        }
        let Ok(comm) = std::fs::read(e.path().join("comm")) else { continue };
        let comm = String::from_utf8_lossy(&comm[..comm.len().min(64)]).trim_end().to_owned();
        let hit = if comm == "wl-paste" {
            let Ok(f) = std::fs::File::open(e.path().join("cmdline")) else { continue };
            let mut buf = Vec::new();
            if f.take(4096).read_to_end(&mut buf).is_err() {
                continue;
            }
            let args: Vec<String> = buf.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).to_lowercase()).collect();
            if !args.iter().any(|a| a == "--watch" || a == "-w") {
                continue;
            }
            let rest = args.join(" ");
            let (tool, kind) = CLIP_MARKERS
                .iter()
                .find(|(m, ..)| rest.contains(m))
                .map_or(("wl-paste --watch", ClipKind::Manager), |(_, t, k)| (*t, *k));
            ClipTool { tool, kind, at: None }
        } else if let Some((_, tool, kind)) = CLIP_PROCESSES.iter().find(|(c, ..)| *c == comm) {
            ClipTool { tool, kind: *kind, at: None }
        } else {
            continue;
        };
        if !found.contains(&hit) {
            found.push(hit);
        }
    }
    found.sort_unstable_by_key(|c| c.tool);
    found
}

fn portal(env: &Env, libexec: &Path) -> Portal {
    let files = [
        env.data_home.join("xdg-desktop-portal/portals/nuxut.portal"),
        env.data_home.join("dbus-1/services/org.freedesktop.impl.portal.desktop.nuxut.service"),
        env.config_home.join("systemd/user/xdg-desktop-portal-nuxut.service"),
        libexec.join("xdg-desktop-portal-nuxut"),
    ];
    let total_files = files.len();
    let missing_files = files.into_iter().filter(|f| !f.is_file()).collect();
    let conf_path = env.config_home.join("xdg-desktop-portal/nuxut-portals.conf");
    let portals_conf = match read_limited(&conf_path) {
        None => ConfState::Missing,
        Some(t) if !t.starts_with(PORTALS_CONF_PREFIX) => ConfState::Foreign,
        Some(t) => ConfState::Managed {
            file_chooser_nuxut: t.lines().any(|l| {
                l.split_once('=').is_some_and(|(k, v)| k.trim() == FILE_CHOOSER_KEY && v.split(';').any(|x| x.trim() == "nuxut"))
            }),
        },
    };
    Portal {
        missing_files,
        total_files,
        desktop_has_nuxut: env.current_desktop.as_deref().map(|d| d.split(':').any(|x| x == "Nuxut")),
        portals_conf,
        portals_conf_path: conf_path,
        sharepicker_block: read_limited(&env.config_home.join("hypr/xdph.conf"))
            .map(|t| t.lines().any(|l| l.trim_start().starts_with(SHAREPICKER_BEGIN))),
    }
}

fn lock(env: &Env, bin_dir: Option<&Path>) -> Lock {
    let binary = bin_dir.map(|d| d.join("nuxut-lock")).unwrap_or_else(|| PathBuf::from("nuxut-lock"));
    let listen_logind = read_limited(&env.config_home.join("nuxut/shell.toml"))
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| t.get("lock")?.get("listen_logind")?.as_bool())
        .unwrap_or(true);
    Lock { binary_ok: is_executable(&binary), binary, listen_logind }
}

/// pacman yerel veritabanından önerilen paketler. Kayıt dizini `<ad>-<sürüm>-<yayım>`; ad son
/// iki `-` öncesi. Veritabanı yoksa `None`.
pub fn packages(db: &Path) -> Option<Packages> {
    let dir = std::fs::read_dir(db).ok()?;
    let mut installed = std::collections::HashSet::new();
    for e in dir.filter_map(Result::ok).take(MAX_DB_ENTRIES) {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let mut parts = name.rsplitn(3, '-');
        if let (Some(_), Some(_), Some(pkg)) = (parts.next(), parts.next(), parts.next()) {
            installed.insert(pkg.to_owned());
        }
    }
    let missing = |list: &[&'static str]| list.iter().copied().filter(|p| !installed.contains(*p)).collect();
    Some(Packages { total: PACKAGES.len(), missing: missing(&PACKAGES), missing_optional: missing(&OPTIONAL_PACKAGES) })
}

/// Çalışan, Nuxut'la aynı işi yapan programlar (yalnız [`COMPETITORS`] adları; tekrar yok).
pub fn competitors(proc_root: &Path) -> Vec<&'static str> {
    let Ok(dir) = std::fs::read_dir(proc_root) else { return Vec::new() };
    let mut found = Vec::new();
    for e in dir.filter_map(Result::ok) {
        if !e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())) {
            continue;
        }
        let Ok(comm) = std::fs::read(e.path().join("comm")) else { continue };
        let comm = String::from_utf8_lossy(&comm[..comm.len().min(64)]);
        if let Some(name) = COMPETITORS.iter().find(|c| **c == comm.trim_end())
            && !found.contains(name)
        {
            found.push(*name);
        }
    }
    found.sort_unstable();
    found
}

fn disk_free(path: &Path) -> Option<u64> {
    let s = rustix::fs::statvfs(path).ok()?;
    Some(s.f_bavail.saturating_mul(s.f_frsize))
}

/// Sürecin başlangıç ortamındaki `PATH` (`/proc/<pid>/environ`; boyut sınırlı). Okunamazsa
/// (başka kullanıcının süreci, süreç yok) `None`. Değer rapora girmez, yalnız denetlenir.
fn environ_path(proc_root: &Path, pid: u32) -> Option<OsString> {
    use std::os::unix::ffi::OsStrExt;
    let f = std::fs::File::open(proc_root.join(pid.to_string()).join("environ")).ok()?;
    let mut buf = Vec::new();
    f.take(MAX_FILE).read_to_end(&mut buf).ok()?;
    let entry = buf.split(|b| *b == 0).find(|e| e.starts_with(b"PATH="))?;
    Some(std::ffi::OsStr::from_bytes(&entry[5..]).to_owned())
}

/// Kullanıcının Hyprland sürecinin PID'i (ortamı okunabilen ilk `Hyprland`).
fn hyprland_pid(proc_root: &Path) -> Option<u32> {
    let dir = std::fs::read_dir(proc_root).ok()?;
    dir.filter_map(Result::ok).find_map(|e| {
        let pid = e.file_name().to_str()?.parse::<u32>().ok()?;
        let comm = std::fs::read(e.path().join("comm")).ok()?;
        (String::from_utf8_lossy(&comm[..comm.len().min(64)]).trim_end() == "Hyprland"
            && std::fs::File::open(e.path().join("environ")).is_ok())
        .then_some(pid)
    })
}

/// Etkili PATH: terminal dışındaki ortamlar (systemd kullanıcı yöneticisi, Hyprland, çalışan
/// kabuk) ikili dizinini içeriyor mu. Bilinmeyen `None`.
fn session_path(env: &Env, bin_dir: Option<&Path>, live: &Live, systemd: Option<Vec<String>>) -> SessionPath {
    let Some(dir) = bin_dir else { return SessionPath::default() };
    let has = |p: Option<OsString>| p.map(|p| crate::self_exe::dir_in_path(dir, Some(&p)));
    let systemd_path = systemd.and_then(|vars| {
        vars.into_iter().take(4096).find_map(|v| v.strip_prefix("PATH=").map(OsString::from))
    });
    let shell = match live {
        Live::Running(r) => has(environ_path(&env.proc_root, r.pid)),
        _ => None,
    };
    SessionPath {
        systemd: has(systemd_path),
        hyprland: has(hyprland_pid(&env.proc_root).and_then(|pid| environ_path(&env.proc_root, pid))),
        shell,
        envd_file: env.config_home.join(ENVD_PATH_FILE).is_file(),
    }
}

/// systemd kullanıcı yöneticisinin ortamı: oturum bus'ında `Manager.Environment` özelliği
/// (salt okur, 2 sn sınırlı). Bus ya da yönetici yoksa `None`.
pub fn systemd_environment() -> Option<Vec<String>> {
    let conn = zbus::blocking::connection::Builder::session().ok()?.method_timeout(std::time::Duration::from_secs(2)).build().ok()?;
    let proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .ok()?;
    proxy.get_property::<Vec<String>>("Environment").ok()
}
