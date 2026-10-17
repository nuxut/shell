//! `nuxut-shell doctor`: tek komutla durum raporu (docs/design/doctor.md).
//!
//! İki adım: [`gather`] dosya sisteminden, ortamdan, çalışan kabuktan (`msg doctor live`) ve
//! logind'den **yalnız okuyarak** [`Facts`] toplar; [`evaluate`] saf bir işlevle onları
//! bölümlere ve ✓/⚠/✗ denetimlerine çevirir. Rapora kişisel veri girmez: yalnızca yollar
//! (`~` ile), sayılar, sabit durum kimlikleri ve ortam değişkeni adları.

pub mod gather;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use nuxut::i18n::t;
use serde::Serialize;
use serde_json::{Value, json};

/// Rapor biçiminin sürümü (JSON `v`; alanlar yalnızca eklenir).
pub const REPORT_VERSION: u32 = 1;
/// Listelenen en çok öğe (çakışma, dosya…); fazlası sayıyla özetlenir.
const MAX_ITEMS: usize = 12;
/// Boş disk sınırları.
const DISK_FAIL: u64 = 2 << 30;
const DISK_WARN: u64 = 10 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Info,
    Warn,
    Fail,
}

impl Level {
    fn mark(self) -> &'static str {
        match self {
            Level::Ok => "✓",
            Level::Info => "·",
            Level::Warn => "⚠",
            Level::Fail => "✗",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub level: Level,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Makine için ayrıntı (yollar, sayılar, durum kimlikleri).
    #[serde(skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Section {
    pub id: &'static str,
    pub title: String,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub v: u32,
    pub version: String,
    pub sections: Vec<Section>,
}

/// Canlı istek için bekleme (ses özeti ses thread'inden gelir; kabuk meşgulse rapor yine çıkar).
const LIVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Derlemenin kaynak commit'i: install.sh derlerken `NUXUT_BUILD_COMMIT` verir (yalnız onaltılık).
pub fn build_commit() -> Option<&'static str> {
    option_env!("NUXUT_BUILD_COMMIT").filter(|c| !c.is_empty() && c.len() <= 40 && c.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `nuxut-shell doctor [--json]`: raporu yazdırır, çıkış kodunu döner (✗ varsa 1).
pub fn run(json: bool) -> u8 {
    nuxut::i18n::init(&[crate::i18n::source()]);
    let env = gather::Env::from_process();
    let live = match crate::ipc::app_name() {
        Err(_) => gather::LiveReply::NotRunning,
        Ok(app) => match nuxut::ipc::Client::new(&app).and_then(|c| c.with_timeout(LIVE_TIMEOUT).send_args(&["doctor", "live"])) {
            Ok(text) => gather::LiveReply::Text(text),
            Err(nuxut::ipc::IpcError::NotRunning) => gather::LiveReply::NotRunning,
            Err(_) => gather::LiveReply::Failed,
        },
    };
    let theme = nuxut::Theme::load().map(|l| l.value).unwrap_or_default();
    let ty = &theme.typography;
    let families = vec![
        ("ui", ty.ui.family.clone()),
        ("display", ty.display.family.clone()),
        ("document", ty.document.family.clone()),
        ("mono", ty.mono.family.clone()),
        ("icon", ty.icon_family().to_owned()),
    ];
    let mut text = nuxut::TextSystem::new();
    let mut font = |f: &str| text.has_family(f);
    let session = |pid: u32| match crate::services::session::session_resolution(None, pid, crate::services::hyprland::compositor_pid()) {
        crate::services::session::SessionResolution::ByPid => SessionRes::ByPid,
        crate::services::session::SessionResolution::Auto => SessionRes::Auto,
        crate::services::session::SessionResolution::Missing => SessionRes::Missing,
    };
    let systemd_env = gather::systemd_environment;
    let facts =
        gather::gather(&env, live, gather::Probes { session: &session, systemd_env: &systemd_env, font: &mut font, families });
    let report = evaluate(&facts);
    if json {
        println!("{}", report.to_json());
    } else {
        print!("{}", report.render());
    }
    report.exit_code()
}

// ---------------------------------------------------------------------------------------
// Toplanan bilgiler (saf veri; testler elle kurar)

#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// Ev dizini (yolları `~` ile göstermek için).
    pub home: Option<PathBuf>,
    pub install: Install,
    pub live: Live,
    /// Kullanıcının Hyprland ayarında bulunan tema paketi izleri.
    pub packs: Vec<PackHit>,
    /// Canlı bilgi yokken taranan Hyprland ayar dosyası (bilgi için).
    pub hypr_config: Option<PathBuf>,
    pub portal: Portal,
    pub lock: Lock,
    /// Ev dizininin dosya sisteminde boş bayt.
    pub disk_free: Option<u64>,
    pub fonts: Vec<Font>,
    /// Önerilen paketler (pacman yerel veritabanından); `None`: veritabanı yok (Arch dışı).
    pub packages: Option<Packages>,
    /// Nuxut'la aynı işi yapan, çalışan programların adları (ikinci bar, bildirim adı çakışması).
    pub competitors: Vec<&'static str>,
    /// Harici pano araçları: Hyprland `exec` satırları ve çalışan süreçler (içerik/komut yok).
    pub clip_tools: Vec<ClipTool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Packages {
    pub total: usize,
    pub missing: Vec<&'static str>,
    /// İsteğe bağlı paketlerden kurulu olmayanlar (yalnız bilgi).
    pub missing_optional: Vec<&'static str>,
}

/// Kullanıcının etkileşimli kabuğu; rapora yalnız bu sabit kimlik girer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LoginShell {
    Fish,
    Bash,
    Zsh,
    #[default]
    Other,
}

impl LoginShell {
    /// `$SHELL` değerinden (ör. `/usr/bin/fish`): yalnız son bileşenin adına bakılır.
    pub fn from_shell_var(v: Option<&std::ffi::OsStr>) -> Self {
        match v.map(Path::new).and_then(Path::file_name).and_then(|n| n.to_str()) {
            Some("fish") => Self::Fish,
            Some("bash") => Self::Bash,
            Some("zsh") => Self::Zsh,
            _ => Self::Other,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Fish => "fish",
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Other => "other",
        }
    }
}

/// İkili dizini terminal dışındaki ortamların PATH'inde mi (`None`: bilinmiyor). Dosyanın varlığı
/// yetmez: environment.d yalnız systemd kullanıcı yöneticisi başlarken okunur, Hyprland ve onun
/// başlattığı kabuk başka bir ortamdan gelebilir.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionPath {
    /// systemd kullanıcı yöneticisi (`nuxut-shell.service` ve uwsm ile başlayanlar).
    pub systemd: Option<bool>,
    /// Hyprland süreci (`exec-once` ve kısayollarla başlayanlar).
    pub hyprland: Option<bool>,
    /// Çalışan kabuk süreci.
    pub shell: Option<bool>,
    /// `~/.config/environment.d/50-nuxut-path.conf` var.
    pub envd_file: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Install {
    /// Çalışan ikilinin dizini.
    pub bin_dir: Option<PathBuf>,
    pub missing_bins: Vec<&'static str>,
    pub total_bins: usize,
    pub missing_helpers: Vec<&'static str>,
    pub total_helpers: usize,
    /// Terminalin `PATH`'i ikili dizinini içeriyor.
    pub in_path: bool,
    /// Kullanıcının kabuğu (`$SHELL`'in adı; PATH önerisi buna göre).
    pub login_shell: LoginShell,
    /// Oturumun etkili PATH'i (terminal dışı ortamlar).
    pub session_path: SessionPath,
    pub version: String,
    pub commit: Option<String>,
    /// `install.sh` yolu (öneri satırı için).
    pub install_sh: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub enum Live {
    #[default]
    NotRunning,
    /// Kabuk yanıt verdi ama yanıt okunamadı (eski sürüm vb.).
    Unreadable,
    Running(Box<Running>),
}

#[derive(Debug, Clone, Default)]
pub struct Running {
    pub pid: u32,
    pub version: String,
    pub commit: Option<String>,
    pub instance: String,
    pub service: Service,
    /// Aynı ikiliyi çalıştıran süreç sayısı (en az 1).
    pub instances: usize,
    pub session: SessionRes,
    pub hypr: Option<Value>,
    pub look: Option<Value>,
    pub audio: Option<Value>,
    /// Kilit dinleyicisinin canlı durumu (`lock.listening`); eski kabukta yok: `None`.
    pub lock_listening: Option<bool>,
    /// Pano geçmişi ve VM köprüsü (`clipboard`; eski kabukta yok).
    pub clipboard: Option<Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Service {
    /// `nuxut-shell.service` kullanıcı birimi.
    Unit,
    /// Oturum scope'u (`session-N.scope`; Hyprland'in `exec-once`'ı).
    LoginSession,
    /// Başka bir uygulama scope'u (terminalden `setsid` gibi).
    AppScope,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SessionRes {
    /// `GetSessionByPID` kendi oturumunu verdi.
    ByPid,
    /// Yalnız `session/auto` yedeğiyle çözüldü.
    Auto,
    /// Çözülemedi (logind yok ya da oturum yok).
    #[default]
    Missing,
}

/// Harici pano aracının türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    /// X11 ↔ Wayland eşitleyici (ör. `~/hypr-bridge.sh`, `wl-paste --watch xclip`): Nuxut'un VM
    /// köprüsüyle birlikte iki köprü birbirini besler.
    Bridge,
    /// Pano geçmişi yöneticisi (cliphist, copyq…): Nuxut'un geçmişiyle aynı işi yapar.
    Manager,
}

impl ClipKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Bridge => "bridge",
            Self::Manager => "manager",
        }
    }
}

/// Bulunan harici pano aracı: yalnız araç kimliği ve yeri (`dosya:satır`; `None`: çalışan süreç).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipTool {
    pub tool: &'static str,
    pub kind: ClipKind,
    pub at: Option<(PathBuf, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackHit {
    pub pack: &'static str,
    pub file: PathBuf,
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Portal {
    pub missing_files: Vec<PathBuf>,
    pub total_files: usize,
    /// Terminalin `XDG_CURRENT_DESKTOP`'ı `Nuxut` içeriyor mu (`None`: değişken yok).
    pub desktop_has_nuxut: Option<bool>,
    pub portals_conf: ConfState,
    pub portals_conf_path: PathBuf,
    /// `xdph.conf`'ta Nuxut seçici bloğu var mı (`None`: dosya yok).
    pub sharepicker_block: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConfState {
    #[default]
    Missing,
    /// Dosya var ama Nuxut yönetmiyor (ilk satır işareti yok; kullanıcı sahiplenmiş).
    Foreign,
    Managed {
        file_chooser_nuxut: bool,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Lock {
    pub binary: PathBuf,
    pub binary_ok: bool,
    /// `shell.toml [lock] listen_logind` (yoksa `true`).
    pub listen_logind: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    pub role: &'static str,
    pub family: String,
    pub installed: bool,
}

// ---------------------------------------------------------------------------------------
// Değerlendirme

struct Ctx<'a> {
    home: Option<&'a Path>,
}

impl Ctx<'_> {
    /// Yolu ev dizini `~` ile kısaltır.
    fn path(&self, p: &Path) -> String {
        if let Some(rest) = self.home.and_then(|h| p.strip_prefix(h).ok()) {
            if rest.as_os_str().is_empty() {
                return "~".into();
            }
            return format!("~/{}", rest.display());
        }
        p.display().to_string()
    }

    fn path_str(&self, s: &str) -> String {
        self.path(Path::new(s))
    }
}

fn check(id: &'static str, level: Level, text: String) -> Check {
    Check { id, level, text, hint: None, data: Value::Null }
}

impl Check {
    fn hint(mut self, hint: String) -> Self {
        self.hint = Some(hint);
        self
    }

    fn data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }
}

/// Listeyi en çok [`MAX_ITEMS`] öğeyle birleştirir; fazlası "+N".
fn join_limited(items: &[String]) -> String {
    let mut out = items.iter().take(MAX_ITEMS).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > MAX_ITEMS {
        let _ = write!(out, " (+{})", items.len() - MAX_ITEMS);
    }
    out
}

/// JSON'dan kısa, kontrol karaktersiz dize (kabuk yanıtı dış girdi sayılır).
fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(|s| s.chars().filter(|c| !c.is_control()).take(200).collect())
}

pub fn evaluate(f: &Facts) -> Report {
    let cx = Ctx { home: f.home.as_deref() };
    let running = match &f.live {
        Live::Running(r) => Some(r.as_ref()),
        _ => None,
    };
    Report {
        v: REPORT_VERSION,
        version: f.install.version.clone(),
        sections: vec![
            install(&cx, &f.install),
            shell(&f.live, &f.competitors),
            hyprland(&cx, running, &f.packs, f.hypr_config.as_deref()),
            look(&cx, running),
            audio(running),
            portal(&cx, &f.portal, &f.install.install_sh),
            lock(&cx, &f.lock, running),
            clipboard(&cx, running, &f.clip_tools),
            system(f.disk_free, &f.fonts, f.packages.as_ref()),
        ],
    }
}

fn section(id: &'static str, title: String, checks: Vec<Check>) -> Section {
    Section { id, title, checks }
}

/// Etkili PATH: terminal dışındaki ortamlar (systemd kullanıcı yöneticisi, Hyprland, çalışan
/// kabuk). Kabuğun başlattığı komutlar bu ortamlardan gelir; environment.d dosyasının varlığı
/// yetmez.
fn session_path_check(cx: &Ctx, i: &Install, dir: &str) -> Check {
    let sp = i.session_path;
    let envs = [("systemd", sp.systemd), ("Hyprland", sp.hyprland), ("nuxut-shell", sp.shell)];
    let names = |want: bool| envs.iter().filter(|(_, v)| *v == Some(want)).map(|(n, _)| *n).collect::<Vec<_>>();
    let (with, without) = (names(true), names(false));
    let data = json!({"systemd": sp.systemd, "hyprland": sp.hyprland, "shell": sp.shell, "envd_file": sp.envd_file});
    if without.is_empty() {
        let c = if with.is_empty() {
            check("session_path", Level::Info, t!("shell-doctor-session-path-unknown"))
        } else {
            check("session_path", Level::Ok, t!("shell-doctor-session-path-ok", dir = dir.to_owned(), list = with.join(", ")))
        };
        return c.data(data);
    }
    let hint = if sp.systemd == Some(false) && !sp.envd_file {
        t!("shell-doctor-hint-fix-path", cmd = format!("sh {} --fix-path", cx.path(&i.install_sh)))
    } else if sp.systemd == Some(false) {
        // Dosya var ama yönetici onu okumadan başlamış.
        t!("shell-doctor-hint-session-path-relogin")
    } else {
        // systemd tamam; Hyprland ya da kabuk başka bir ortamdan başlamış.
        t!("shell-doctor-hint-session-path-hyprland")
    };
    check("session_path", Level::Warn, t!("shell-doctor-session-path-missing", dir = dir.to_owned(), list = without.join(", ")))
        .hint(hint)
        .data(data)
}

fn install(cx: &Ctx, i: &Install) -> Section {
    let dir = i.bin_dir.as_deref().map(|d| cx.path(d)).unwrap_or_else(|| "?".into());
    let mut checks = Vec::new();
    // PATH ilk satır: kullanıcı en çok bunu yaşıyor ("nuxut-shell: komut bulunamadı").
    checks.push(if i.in_path {
        check("path", Level::Ok, t!("shell-doctor-path-ok", dir = dir.clone()))
    } else {
        let hint = match i.login_shell {
            // fish_add_path evrensel değişken yazar: yeni terminallerde hemen geçerli, oturum kapatmaya gerek yok.
            LoginShell::Fish => t!("shell-doctor-hint-fix-path-fish", cmd = format!("fish_add_path {dir}")),
            LoginShell::Bash | LoginShell::Zsh | LoginShell::Other => {
                t!("shell-doctor-hint-fix-path", cmd = format!("sh {} --fix-path", cx.path(&i.install_sh)))
            }
        };
        check("path", Level::Warn, t!("shell-doctor-path-missing", dir = dir.clone())).hint(hint)
    }
    .data(json!({"dir": dir, "shell": i.login_shell.id()})));
    if i.bin_dir.is_some() {
        checks.push(session_path_check(cx, i, &dir));
    }
    let found = i.total_bins - i.missing_bins.len();
    checks.push(if i.missing_bins.is_empty() {
        check("binaries", Level::Ok, t!("shell-doctor-binaries-ok", dir = dir.clone(), count = found))
    } else {
        check("binaries", Level::Fail, t!("shell-doctor-binaries-missing", dir = dir.clone(), names = i.missing_bins.join(", ")))
            .hint(t!("shell-doctor-hint-reinstall", cmd = format!("sh {}", cx.path(&i.install_sh))))
    }
    .data(json!({"dir": dir, "missing": i.missing_bins})));
    checks.push(if i.missing_helpers.is_empty() {
        check("helpers", Level::Ok, t!("shell-doctor-helpers-ok", count = i.total_helpers))
    } else {
        // Portal ve seçici ikilileri kurulumun zorunlu saydıklarıdır; görsel işçisi uyarı.
        let required = i.missing_helpers.iter().any(|h| gather::REQUIRED_HELPERS.contains(h));
        check("helpers", if required { Level::Fail } else { Level::Warn }, t!("shell-doctor-helpers-missing", names = i.missing_helpers.join(", ")))
            .hint(t!("shell-doctor-hint-reinstall", cmd = format!("sh {}", cx.path(&i.install_sh))))
    }
    .data(json!({"missing": i.missing_helpers})));
    let commit = i.commit.clone().unwrap_or_else(|| t!("shell-doctor-unknown"));
    checks.push(
        check("version", Level::Info, t!("shell-doctor-version", version = i.version.clone(), commit = commit))
            .data(json!({"version": i.version, "commit": i.commit})),
    );
    section("install", t!("shell-doctor-section-install"), checks)
}

fn not_running(id: &'static str) -> Check {
    check(id, Level::Info, t!("shell-doctor-needs-shell"))
}

fn shell(live: &Live, competitors: &[&'static str]) -> Section {
    let mut checks = Vec::new();
    match live {
        Live::NotRunning => checks.push(
            check("running", Level::Fail, t!("shell-doctor-shell-not-running")).hint(t!("shell-doctor-hint-start-shell")),
        ),
        Live::Unreadable => checks.push(
            check("running", Level::Warn, t!("shell-doctor-shell-unreadable")).hint(t!("shell-doctor-hint-restart-shell")),
        ),
        Live::Running(r) => {
            let commit = r.commit.clone().unwrap_or_else(|| t!("shell-doctor-unknown"));
            checks.push(
                check("running", Level::Ok, t!("shell-doctor-shell-running", version = r.version.clone(), commit = commit))
                    .data(json!({"pid": r.pid, "instance": r.instance, "version": r.version, "commit": r.commit})),
            );
            let (level, text, id) = match r.service {
                Service::Unit => (Level::Ok, t!("shell-doctor-service-unit"), "unit"),
                Service::LoginSession => (Level::Ok, t!("shell-doctor-service-session"), "session_scope"),
                Service::AppScope => (Level::Warn, t!("shell-doctor-service-app-scope"), "app_scope"),
                Service::Unknown => (Level::Info, t!("shell-doctor-service-unknown"), "unknown"),
            };
            let mut c = check("service", level, text).data(json!({"kind": id}));
            if r.service == Service::AppScope {
                c = c.hint(t!("shell-doctor-hint-service"));
            }
            checks.push(c);
            checks.push(if r.instances <= 1 {
                check("instances", Level::Ok, t!("shell-doctor-instances-one"))
            } else {
                check("instances", Level::Warn, t!("shell-doctor-instances-many", count = r.instances))
                    .hint(t!("shell-doctor-hint-instances"))
            }
            .data(json!({"count": r.instances})));
            checks.push(match r.session {
                SessionRes::ByPid => check("session", Level::Ok, t!("shell-doctor-session-pid")),
                SessionRes::Auto => {
                    check("session", Level::Warn, t!("shell-doctor-session-auto")).hint(t!("shell-doctor-hint-service"))
                }
                SessionRes::Missing => {
                    check("session", Level::Fail, t!("shell-doctor-session-missing")).hint(t!("shell-doctor-hint-service"))
                }
            });
        }
    }
    checks.push(if competitors.is_empty() {
        check("competitors", Level::Ok, t!("shell-doctor-competitors-none"))
    } else {
        check("competitors", Level::Warn, t!("shell-doctor-competitors", list = competitors.join(", ")))
            .hint(t!("shell-doctor-hint-competitors"))
    }
    .data(json!({"running": competitors})));
    section("shell", t!("shell-doctor-section-shell"), checks)
}

fn hyprland(cx: &Ctx, r: Option<&Running>, packs: &[PackHit], config: Option<&Path>) -> Section {
    let mut checks = Vec::new();
    match r.and_then(|r| r.hypr.as_ref()) {
        None => checks.push(not_running("source")),
        Some(h) => {
            let state = s(h, "source").unwrap_or_default();
            let file = s(h, "config").map(|c| cx.path_str(&c)).unwrap_or_else(|| "?".into());
            let level = match state.as_str() {
                "present" | "added" => Level::Ok,
                "off" | "disabled_by_user" | "removed_by_user" | "undone" => Level::Warn,
                "inactive" | "pending" => Level::Info,
                _ => Level::Fail,
            };
            let mut c = check("source", level, t!("shell-doctor-hypr-source", state = state.clone(), file = file.clone()))
                .data(json!({"state": state, "config": file}));
            if matches!(state.as_str(), "removed_by_user" | "disabled_by_user" | "off" | "undone") {
                c = c.hint(t!("shell-doctor-hint-hypr-source"));
            }
            checks.push(c);
            let conflicts: Vec<String> = h
                .get("conflicts")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|c| {
                            let at = match (s(c, "file"), c.get("line").and_then(Value::as_u64)) {
                                (Some(f), Some(l)) => format!(" ({}:{l})", cx.path_str(&f)),
                                (Some(f), None) => format!(" ({})", cx.path_str(&f)),
                                _ => String::new(),
                            };
                            format!("{} {}{at}", s(c, "combo").unwrap_or_default(), s(c, "action").unwrap_or_default())
                        })
                        .collect()
                })
                .unwrap_or_default();
            checks.push(if conflicts.is_empty() {
                check("conflicts", Level::Ok, t!("shell-doctor-hypr-conflicts-none"))
            } else {
                check("conflicts", Level::Warn, t!("shell-doctor-hypr-conflicts", count = conflicts.len(), list = join_limited(&conflicts)))
                    .hint(t!("shell-doctor-hint-conflicts"))
            }
            .data(json!({"count": conflicts.len()})));
        }
    }
    if packs.is_empty() {
        let file = config.map(|c| cx.path(c)).unwrap_or_else(|| "?".into());
        checks.push(check("theme_pack", Level::Ok, t!("shell-doctor-pack-none", file = file)));
    } else {
        let mut names: Vec<&str> = packs.iter().map(|p| p.pack).collect();
        names.dedup();
        let at: Vec<String> = packs.iter().map(|p| format!("{}:{}", cx.path(&p.file), p.line)).collect();
        checks.push(
            check("theme_pack", Level::Warn, t!("shell-doctor-pack-found", packs = names.join(", "), list = join_limited(&at)))
                .hint(t!("shell-doctor-hint-pack"))
                .data(json!({"packs": names, "at": at})),
        );
    }
    section("hyprland", t!("shell-doctor-section-hyprland"), checks)
}

fn look(cx: &Ctx, r: Option<&Running>) -> Section {
    let mut checks = Vec::new();
    match r.and_then(|r| r.look.as_ref()) {
        None => checks.push(not_running("writer")),
        Some(l) => {
            let writer = s(l, "writer").unwrap_or_default();
            let unified = l.get("unified").and_then(Value::as_bool).unwrap_or(false);
            checks.push(
                check(
                    "writer",
                    if unified || writer == "disabled" { Level::Info } else { Level::Ok },
                    t!("shell-doctor-look-writer", writer = writer.clone(), unified = if unified { "on" } else { "off" }),
                )
                .data(json!({"writer": writer, "unified": unified})),
            );
            // Ortam çatışmaları: yalnız değişken adı ve kaynağı (değer girmez).
            let conflicts: Vec<String> = l
                .get("conflicts")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|c| match (s(c, "name"), s(c, "source")) {
                            (Some(n), Some(src)) => format!("{n} ({src})"),
                            (Some(n), None) => n,
                            (None, _) => s(c, "kind").unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            checks.push(if conflicts.is_empty() {
                check("conflicts", Level::Ok, t!("shell-doctor-look-conflicts-none"))
            } else {
                check("conflicts", Level::Warn, t!("shell-doctor-look-conflicts", list = join_limited(&conflicts)))
                    .hint(t!("shell-doctor-hint-look-conflicts"))
            }
            .data(json!({"count": conflicts.len()})));
            let last = l.get("last_apply");
            let entries: Vec<&Value> =
                last.and_then(|a| a.get("external")).and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
            // Başka kullanıcıya ait dosya (gerçek vaka: root sahipli ~/.config dosyaları): ayrı
            // satır. Çözüm devralma (yedekli, geri alınabilir). Sahiplik komutu ÖNERİLMEZ (2026-10-16):
            // dosya root'a ait bir temaya sembolik bağ olabilir, `chown` bağı izleyip sistem
            // dosyasının sahibini değiştirir.
            let (not_owned, others): (Vec<&Value>, Vec<&Value>) =
                entries.into_iter().partition(|e| s(e, "reason").as_deref() == Some("not_owned"));
            let owned_paths: Vec<String> = not_owned.iter().filter_map(|e| s(e, "path")).map(|p| cx.path_str(&p)).collect();
            if !owned_paths.is_empty() {
                checks.push(
                    check("not_owned", Level::Warn, t!("shell-doctor-look-not-owned", list = join_limited(&owned_paths)))
                        .hint(t!("shell-doctor-hint-look-not-owned"))
                        .data(json!({"paths": owned_paths})),
                );
            }
            let external: Vec<String> = Some(others)
                .map(|a| {
                    a.iter()
                        .map(|e| {
                            format!(
                                "{} → {} ({})",
                                s(e, "setting").unwrap_or_default(),
                                s(e, "path").map(|p| cx.path_str(&p)).unwrap_or_default(),
                                s(e, "reason").unwrap_or_default()
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let paused: Vec<String> = last
                .and_then(|a| a.get("paused"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            checks.push(if external.is_empty() {
                check("external", Level::Ok, t!("shell-doctor-look-external-none"))
            } else {
                check("external", Level::Warn, t!("shell-doctor-look-external", list = join_limited(&external)))
                    .hint(t!("shell-doctor-hint-look-external"))
            }
            .data(json!({"count": external.len()})));
            if !paused.is_empty() {
                checks.push(
                    check("paused", Level::Warn, t!("shell-doctor-look-paused", list = join_limited(&paused)))
                        .hint(t!("shell-doctor-hint-look-external"))
                        .data(json!({"settings": paused})),
                );
            }
        }
    }
    section("look", t!("shell-doctor-section-look"), checks)
}

fn path_check(id: &'static str, kind: &str) -> Check {
    let (level, state) = match kind {
        "route" => (Level::Ok, t!("shell-doctor-audio-route")),
        "props" => (Level::Ok, t!("shell-doctor-audio-props")),
        "await_routes" => (Level::Warn, t!("shell-doctor-audio-await")),
        "card_props" => (Level::Warn, t!("shell-doctor-audio-card-props")),
        _ => (Level::Fail, t!("shell-doctor-audio-none")),
    };
    let text = if id == "sink" {
        t!("shell-doctor-audio-sink", state = state)
    } else {
        t!("shell-doctor-audio-source", state = state)
    };
    let mut c = check(id, level, text).data(json!({"path": kind}));
    if level != Level::Ok {
        c = c.hint(t!("shell-doctor-hint-audio"));
    }
    c
}

fn audio(r: Option<&Running>) -> Section {
    let mut checks = Vec::new();
    match r {
        None => checks.push(not_running("pipewire")),
        Some(r) => match &r.audio {
            None => checks.push(
                check("pipewire", Level::Fail, t!("shell-doctor-audio-no-service")).hint(t!("shell-doctor-hint-restart-shell")),
            ),
            Some(a) => {
                let available = a.get("available").and_then(Value::as_bool).unwrap_or(false);
                let backend = s(a, "backend").unwrap_or_default();
                checks.push(
                    if available {
                        check("pipewire", Level::Ok, t!("shell-doctor-audio-connected", backend = backend.clone()))
                    } else {
                        check("pipewire", Level::Fail, t!("shell-doctor-audio-disconnected"))
                            .hint(t!("shell-doctor-hint-audio"))
                    }
                    .data(json!({"available": available, "backend": backend})),
                );
                if available {
                    checks.push(path_check("sink", &s(a, "sink").unwrap_or_default()));
                    checks.push(path_check("source", &s(a, "source").unwrap_or_default()));
                }
            }
        },
    }
    section("audio", t!("shell-doctor-section-audio"), checks)
}

fn portal(cx: &Ctx, p: &Portal, install_sh: &Path) -> Section {
    let mut checks = Vec::new();
    let missing: Vec<String> = p.missing_files.iter().map(|f| cx.path(f)).collect();
    checks.push(if missing.is_empty() {
        check("files", Level::Ok, t!("shell-doctor-portal-files-ok", count = p.total_files))
    } else {
        check("files", Level::Warn, t!("shell-doctor-portal-files-missing", list = join_limited(&missing)))
            .hint(t!("shell-doctor-hint-reinstall-plain"))
    }
    .data(json!({"missing": missing})));
    checks.push(match p.desktop_has_nuxut {
        Some(true) => check("desktop", Level::Ok, t!("shell-doctor-portal-desktop-ok")),
        Some(false) => check("desktop", Level::Warn, t!("shell-doctor-portal-desktop-other")).hint(t!("shell-doctor-hint-relogin")),
        None => check("desktop", Level::Warn, t!("shell-doctor-portal-desktop-unset")).hint(t!("shell-doctor-hint-relogin")),
    });
    let conf = cx.path(&p.portals_conf_path);
    checks.push(match p.portals_conf {
        ConfState::Missing => check("portals_conf", Level::Warn, t!("shell-doctor-portal-conf-missing", file = conf))
            .hint(t!("shell-doctor-hint-reinstall-plain")),
        ConfState::Foreign => check("portals_conf", Level::Info, t!("shell-doctor-portal-conf-foreign", file = conf.clone()))
            .hint(t!("shell-doctor-hint-portal-foreign", file = conf)),
        ConfState::Managed { file_chooser_nuxut: true } => {
            check("portals_conf", Level::Ok, t!("shell-doctor-portal-conf-ok", file = conf))
        }
        ConfState::Managed { file_chooser_nuxut: false } => {
            check("portals_conf", Level::Info, t!("shell-doctor-portal-conf-no-chooser", file = conf))
                .hint(t!("shell-doctor-hint-portal-chooser", cmd = format!("sh {} --write-portals-conf", cx.path(install_sh))))
        }
    });
    checks.push(match p.sharepicker_block {
        Some(true) => check("sharepicker", Level::Ok, t!("shell-doctor-portal-picker-ok")),
        Some(false) => check("sharepicker", Level::Warn, t!("shell-doctor-portal-picker-missing"))
            .hint(t!("shell-doctor-hint-reinstall-plain")),
        None => check("sharepicker", Level::Warn, t!("shell-doctor-portal-picker-no-file"))
            .hint(t!("shell-doctor-hint-reinstall-plain")),
    });
    section("portal", t!("shell-doctor-section-portal"), checks)
}

fn lock(cx: &Ctx, l: &Lock, r: Option<&Running>) -> Section {
    let mut checks = Vec::new();
    let bin = cx.path(&l.binary);
    checks.push(if l.binary_ok {
        check("binary", Level::Ok, t!("shell-doctor-lock-binary-ok", file = bin.clone()))
    } else {
        check("binary", Level::Fail, t!("shell-doctor-lock-binary-missing", file = bin.clone()))
            .hint(t!("shell-doctor-hint-reinstall-plain"))
    }
    .data(json!({"path": bin})));
    let live = r.and_then(|r| r.lock_listening);
    checks.push(match (l.listen_logind, live) {
        (false, _) => check("listener", Level::Warn, t!("shell-doctor-lock-listener-off")).hint(t!("shell-doctor-hint-lock-listener")),
        (true, Some(true)) => check("listener", Level::Ok, t!("shell-doctor-lock-listener-on")),
        (true, Some(false)) => check("listener", Level::Fail, t!("shell-doctor-lock-listener-dead")).hint(t!("shell-doctor-hint-restart-shell")),
        (true, None) => check("listener", Level::Info, t!("shell-doctor-lock-listener-config")),
    }
    .data(json!({"listen_logind": l.listen_logind, "listening": live})));
    section("lock", t!("shell-doctor-section-lock"), checks)
}

/// Pano: geçmiş izlemesi, VM köprüsü ve harici araçlar (2026-10-16 gerçek vaka: kullanıcının
/// `~/hypr-bridge.sh`'ı Nuxut köprüsüyle birlikte çalışıyordu; docs/design/vm-clipboard.md).
fn clipboard(cx: &Ctx, r: Option<&Running>, tools: &[ClipTool]) -> Section {
    let mut checks = Vec::new();
    let c = r.and_then(|r| r.clipboard.as_ref());
    let flag = |k: &str| c.and_then(|c| c.get(k)).and_then(Value::as_bool);
    let (enabled, data_control, watching) = (flag("enabled"), flag("data_control"), flag("watching"));
    let mode = c.and_then(|c| s(c, "bridge")).unwrap_or_default();
    let (bridge_running, bridge_active) = (flag("bridge_running").unwrap_or(false), flag("bridge_active").unwrap_or(false));
    match (r, c) {
        (None, _) => checks.push(not_running("history")),
        (Some(_), None) => checks.push(check("history", Level::Info, t!("shell-doctor-clipboard-unknown"))),
        (Some(_), Some(_)) => {
            let journal = || t!("shell-doctor-hint-clipboard-journal");
            checks.push(
                match (enabled, data_control, watching) {
                    (_, Some(false), _) => check("history", Level::Warn, t!("shell-doctor-clipboard-no-protocol"))
                        .hint(t!("shell-doctor-hint-clipboard-no-protocol")),
                    (Some(false), ..) => check("history", Level::Info, t!("shell-doctor-clipboard-history-off")),
                    (_, _, Some(true)) => check("history", Level::Ok, t!("shell-doctor-clipboard-history-ok")),
                    _ => check("history", Level::Warn, t!("shell-doctor-clipboard-stopped")).hint(journal()),
                }
                .data(json!({"enabled": enabled, "data_control": data_control, "watching": watching})),
            );
            checks.push(
                match (mode.as_str(), bridge_running, bridge_active) {
                    ("off", ..) => check("bridge", Level::Info, t!("shell-doctor-clipboard-bridge-off")),
                    (_, true, true) => check("bridge", Level::Ok, t!("shell-doctor-clipboard-bridge-active", mode = mode.clone())),
                    (_, true, false) => check("bridge", Level::Info, t!("shell-doctor-clipboard-bridge-waiting")),
                    // data-control yoksa köprü hiç kurulmaz; neden yukarıdaki satırda.
                    _ if data_control == Some(false) => {
                        check("bridge", Level::Info, t!("shell-doctor-clipboard-bridge-down", mode = mode.clone()))
                    }
                    _ => check("bridge", Level::Warn, t!("shell-doctor-clipboard-bridge-down", mode = mode.clone())).hint(journal()),
                }
                .data(json!({"mode": mode, "running": bridge_running, "active": bridge_active})),
            );
        }
    }
    let describe = |kind: ClipKind| -> Vec<String> {
        tools
            .iter()
            .filter(|t| t.kind == kind)
            .map(|t| match &t.at {
                Some((file, line)) => format!("{} ({}:{line})", t.tool, cx.path(file)),
                None => t!("shell-doctor-clipboard-running", tool = t.tool),
            })
            .collect()
    };
    let json_of = |kind: ClipKind| -> Vec<Value> {
        tools
            .iter()
            .filter(|t| t.kind == kind)
            .map(|t| json!({"tool": t.tool, "file": t.at.as_ref().map(|(f, _)| cx.path(f)), "line": t.at.as_ref().map(|(_, l)| l)}))
            .collect()
    };
    let (sync, managers) = (describe(ClipKind::Bridge), describe(ClipKind::Manager));
    if sync.is_empty() && managers.is_empty() {
        checks.push(check("external", Level::Ok, t!("shell-doctor-clipboard-tools-none")));
    }
    if !sync.is_empty() {
        // Nuxut köprüsü kapalıyken harici eşitleyici kullanıcının seçimidir.
        let level = if mode == "off" { Level::Info } else { Level::Warn };
        let mut c = check("sync_tools", level, t!("shell-doctor-clipboard-sync", list = join_limited(&sync)));
        if level == Level::Warn {
            c = c.hint(t!("shell-doctor-hint-clipboard-sync"));
        }
        checks.push(c.data(json!({"tools": json_of(ClipKind::Bridge)})));
    }
    if !managers.is_empty() {
        let level = if enabled == Some(false) { Level::Info } else { Level::Warn };
        let mut c = check("managers", level, t!("shell-doctor-clipboard-managers", list = join_limited(&managers)));
        if level == Level::Warn {
            c = c.hint(t!("shell-doctor-hint-clipboard-managers"));
        }
        checks.push(c.data(json!({"tools": json_of(ClipKind::Manager)})));
    }
    section("clipboard", t!("shell-doctor-section-clipboard"), checks)
}

/// Font ailesinin Arch paketi (öneri için).
fn font_package(family: &str) -> Option<&'static str> {
    match family {
        "Inter" => Some("inter-font"), // font-istisnası: aile adından Arch paketi önerisi (çizim değil)
        "Source Serif 4" => Some("adobe-source-serif-fonts"), // font-istisnası: aile adından Arch paketi önerisi (çizim değil)
        "JetBrainsMono Nerd Font" => Some("ttf-jetbrains-mono-nerd"), // font-istisnası: aile adından Arch paketi önerisi (çizim değil)
        "Material Symbols Rounded" => Some("ttf-material-symbols-variable"), // font-istisnası: aile adından Arch paketi önerisi (çizim değil)
        _ => None,
    }
}

fn packages_check(p: Option<&Packages>) -> Check {
    let Some(p) = p else { return check("packages", Level::Info, t!("shell-doctor-packages-unknown")) };
    let data = json!({"missing": p.missing, "missing_optional": p.missing_optional});
    if p.missing.is_empty() && p.missing_optional.is_empty() {
        return check("packages", Level::Ok, t!("shell-doctor-packages-ok", count = p.total)).data(data);
    }
    if p.missing.is_empty() {
        return check("packages", Level::Info, t!("shell-doctor-packages-optional", list = p.missing_optional.join(", ")))
            .data(data);
    }
    // Önerilenlerin hepsi resmî depoda (fontlar, AUR'daki ikon fontu dahil, `fonts` denetiminde).
    let mut text = t!("shell-doctor-packages-missing", list = p.missing.join(", "));
    if !p.missing_optional.is_empty() {
        let _ = write!(text, " · {}", t!("shell-doctor-packages-optional", list = p.missing_optional.join(", ")));
    }
    check("packages", Level::Warn, text)
        .data(data)
        .hint(t!("shell-doctor-hint-packages", packages = p.missing.join(" ")))
}

fn system(disk_free: Option<u64>, fonts: &[Font], packages: Option<&Packages>) -> Section {
    let mut checks = Vec::new();
    checks.push(match disk_free {
        None => check("disk", Level::Info, t!("shell-doctor-disk-unknown")),
        Some(b) => {
            let gib = format!("{:.1}", b as f64 / (1u64 << 30) as f64);
            let level = if b < DISK_FAIL {
                Level::Fail
            } else if b < DISK_WARN {
                Level::Warn
            } else {
                Level::Ok
            };
            let c = check("disk", level, t!("shell-doctor-disk", gib = gib)).data(json!({"free_bytes": b}));
            if level == Level::Ok { c } else { c.hint(t!("shell-doctor-hint-disk")) }
        }
    });
    let mut missing: Vec<&Font> = fonts.iter().filter(|f| !f.installed).collect();
    missing.dedup_by(|a, b| a.family == b.family);
    checks.push(if missing.is_empty() {
        check("fonts", Level::Ok, t!("shell-doctor-fonts-ok", count = fonts.len()))
    } else {
        let names: Vec<String> = missing.iter().map(|f| f.family.clone()).collect();
        let mut pkgs: Vec<&str> = missing.iter().filter_map(|f| font_package(&f.family)).collect();
        pkgs.dedup();
        let c = check("fonts", Level::Warn, t!("shell-doctor-fonts-missing", list = names.join(", ")));
        if pkgs.is_empty() { c } else { c.hint(t!("shell-doctor-hint-fonts", packages = pkgs.join(" "))) }
    }
    .data(json!({"missing": missing.iter().map(|f| json!({"role": f.role, "family": f.family})).collect::<Vec<_>>()})));
    checks.push(packages_check(packages));
    section("system", t!("shell-doctor-section-system"), checks)
}

// ---------------------------------------------------------------------------------------
// Çıktı

impl Report {
    pub fn worst(&self) -> Level {
        self.sections.iter().flat_map(|s| &s.checks).map(|c| c.level).max().unwrap_or(Level::Ok)
    }

    /// Çıkış kodu: herhangi bir ✗ varsa 1.
    pub fn exit_code(&self) -> u8 {
        u8::from(self.worst() == Level::Fail)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{}", t!("shell-doctor-title", version = self.version.clone()));
        let fix = t!("shell-doctor-fix-label");
        let (mut ok, mut warn, mut fail) = (0, 0, 0);
        for s in &self.sections {
            let _ = writeln!(out, "\n{}", s.title);
            for c in &s.checks {
                match c.level {
                    Level::Ok => ok += 1,
                    Level::Warn => warn += 1,
                    Level::Fail => fail += 1,
                    Level::Info => {}
                }
                let _ = writeln!(out, "  {} {}", c.level.mark(), c.text);
                if let Some(h) = &c.hint {
                    let _ = writeln!(out, "      {fix}: {h}");
                }
            }
        }
        let _ = writeln!(out, "\n{}", t!("shell-doctor-summary", ok = ok, warn = warn, fail = fail));
        out
    }
}

#[cfg(test)]
mod tests;
