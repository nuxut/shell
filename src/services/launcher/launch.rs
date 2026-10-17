//! Uygulama başlatma: `Exec` → argv (kabuk yok), terminal, DBusActivatable, xdg-activation
//! token'ı, süreç ayırma.
//!
//! **Süreç modeli.** Başlatılan uygulama shell'in alt süreci olarak kalmaz:
//! `systemd-run --user --scope --unit=app-nuxut-<kimlik>-<rastgele>.scope -- setsid -f <argv>`.
//! `systemd-run` kendi cgroup kapsamını (scope) açıp `setsid`'e dönüşür; `setsid -f` yeni
//! oturumda çatallanıp hemen çıkar, uygulama alt-süreç toplayıcıya (systemd --user) geçer.
//! Doğrudan çocuğumuz yalnızca bu kısa ömürlü zincirdir; çıkış kodu beklenir — kapsam
//! açılamazsa (`systemd-run` hatası) yalnızca `setsid -f` ile yeniden denenir. Böylece shell
//! yeniden başlasa/çökse de uygulamalar kendi cgroup'unda yaşar (systemd XDG uygulama birimleri
//! adlandırması). `systemd-run` yoksa (systemd'siz kullanıcı oturumu) yalnızca `setsid -f`;
//! o da yoksa doğrudan, ayrı süreç grubunda başlatılır ve bir toplayıcı thread bekler.
//!
//! Ortam: shell'in `XDG_ACTIVATION_TOKEN`/`DESKTOP_STARTUP_ID` ve tüm `NUXUT_*`
//! değişkenleri temizlenir; token varsa ilk ikisi ona ayarlanır. Çalışma dizini `Path=` (varsa ve
//! dizinse), yoksa `$HOME`. stdin/stdout/stderr `/dev/null`.
//!
//! **DBusActivatable**: `org.freedesktop.Application.Activate`/`ActivateAction` (platform_data:
//! `activation-token`, `desktop-startup-id`); başarısızsa `Exec` varsa ona düşülür.
//!
//! Loglarda uygulama adı, kimliği ve argv yazılmaz (yalnızca hata türü).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::catalog::find_executable;
use super::desktop::DesktopEntry;
use super::exec::{self, ExecError, FieldValues};

/// Kullanıcı ayarları (`[launcher]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOptions {
    /// `Terminal=true` uygulamaların önüne konan argv (`["kitty", "-e"]`); boş → otomatik.
    pub terminal: Vec<String>,
    /// Uygulamayı kendi systemd kapsamında başlat.
    pub systemd_scope: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self { terminal: Vec::new(), systemd_scope: true }
    }
}

/// Otomatik terminal adayları (sırayla, `$PATH`'te ilk bulunan): program ve komuttan önceki
/// argümanları.
const TERMINALS: &[&[&str]] = &[
    &["xdg-terminal-exec"],
    &["kitty"],
    &["foot"],
    &["alacritty", "-e"],
    &["wezterm", "start", "--"],
    &["ghostty", "-e"],
    &["konsole", "-e"],
    &["gnome-terminal", "--"],
    &["xterm", "-e"],
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchError {
    #[error("Exec yok")]
    NoExec,
    #[error("Exec geçersiz: {0}")]
    Exec(#[from] ExecError),
    #[error("program bulunamadı")]
    NotFound,
    #[error("terminal bulunamadı")]
    NoTerminal,
    #[error("eylem yok")]
    UnknownAction,
    #[error("süreç başlatılamadı: {0:?}")]
    Spawn(std::io::ErrorKind),
}

/// Ortamdan bir kez çözülen yardımcı programlar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tools {
    pub setsid: Option<PathBuf>,
    pub systemd_run: Option<PathBuf>,
    pub path: Vec<PathBuf>,
    pub home: Option<PathBuf>,
}

impl Tools {
    pub fn from_env() -> Self {
        let path: Vec<PathBuf> =
            std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        // systemd kullanıcı yöneticisi çalışıyor mu? (özel soketi)
        let user_manager = std::env::var_os("XDG_RUNTIME_DIR")
            .map(|d| PathBuf::from(d).join("systemd/private"))
            .is_some_and(|p| p.exists());
        Self {
            setsid: find_executable("setsid", &path),
            systemd_run: user_manager.then(|| find_executable("systemd-run", &path)).flatten(),
            home: std::env::var_os("HOME").map(PathBuf::from),
            path,
        }
    }
}

/// Başlatma planı (saf; test edilebilir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Uygulamanın kendi argv'si (terminal dahil, sarmalayıcısız).
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// Girdi (ve eylem) için argv ve çalışma dizini.
pub fn plan(entry: &DesktopEntry, action: Option<&str>, options: &LaunchOptions, tools: &Tools) -> Result<Plan, LaunchError> {
    let (exec_line, icon) = match action {
        None => (entry.exec.as_deref(), entry.icon.as_deref()),
        Some(id) => {
            let a = entry.actions.iter().find(|a| a.id == id).ok_or(LaunchError::UnknownAction)?;
            (a.exec.as_deref(), a.icon.as_deref().or(entry.icon.as_deref()))
        }
    };
    let exec_line = exec_line.ok_or(LaunchError::NoExec)?;
    let location = entry.path.to_string_lossy();
    let mut argv = exec::expand(exec_line, &FieldValues { icon, name: &entry.name, location: &location })?;
    if find_executable(&argv[0], &tools.path).is_none() {
        return Err(LaunchError::NotFound);
    }
    if entry.terminal {
        let prefix = terminal_prefix(options, tools).ok_or(LaunchError::NoTerminal)?;
        argv.splice(0..0, prefix);
    }
    let cwd = entry
        .working_dir
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir())
        .or_else(|| tools.home.clone());
    Ok(Plan { argv, cwd })
}

fn terminal_prefix(options: &LaunchOptions, tools: &Tools) -> Option<Vec<String>> {
    if let Some(program) = options.terminal.first() {
        return find_executable(program, &tools.path).map(|_| options.terminal.clone());
    }
    TERMINALS
        .iter()
        .find(|t| find_executable(t[0], &tools.path).is_some())
        .map(|t| t.iter().map(|s| (*s).to_owned()).collect())
}

/// systemd birim adı kaçışı (`systemd-escape`): `[A-Za-z0-9:_.]` dışı → `\xNN`; baştaki `.` da.
/// XDG uygulama birimlerinde `-` ayırıcı olduğundan kimlikteki `-` de kaçışlanır.
pub fn systemd_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, b) in s.bytes().enumerate() {
        if b.is_ascii_alphanumeric() || b == b':' || b == b'_' || (b == b'.' && i > 0) {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\x{b:02x}"));
        }
    }
    out
}

/// `app-nuxut-<kimlik>-<rastgele>.scope`.
pub fn scope_unit(id: &str, nonce: u64) -> String {
    let app = id.strip_suffix(".desktop").unwrap_or(id);
    let mut escaped = systemd_escape(app);
    escaped.truncate(200); // birim adı sınırı 255
    format!("app-nuxut-{escaped}-{nonce:x}.scope")
}

/// Sarmalanmış argv: `[systemd-run … --] [setsid -f] argv`.
pub fn wrap(argv: &[String], tools: &Tools, scope: Option<&str>) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len() + 10);
    if let (Some(run), Some(unit)) = (&tools.systemd_run, scope) {
        out.extend([
            run.to_string_lossy().into_owned(),
            "--user".into(),
            "--scope".into(),
            "--quiet".into(),
            "--collect".into(),
            format!("--unit={unit}"),
            "--".into(),
        ]);
    }
    if let Some(setsid) = &tools.setsid {
        out.extend([setsid.to_string_lossy().into_owned(), "-f".into()]);
    }
    out.extend_from_slice(argv);
    out
}

fn nonce() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (t.as_nanos() as u64) ^ ((std::process::id() as u64) << 32)
}

/// Planı başlatır (bloklar: yalnızca ara zincirin çıkışını bekler).
pub fn spawn(entry: &DesktopEntry, plan: &Plan, options: &LaunchOptions, tools: &Tools, token: Option<&str>) -> Result<(), LaunchError> {
    let scope = (options.systemd_scope && tools.systemd_run.is_some()).then(|| scope_unit(&entry.id, nonce()));
    let argv = wrap(&plan.argv, tools, scope.as_deref());
    match run(&argv, plan.cwd.as_deref(), token, tools.setsid.is_none()) {
        Ok(true) => Ok(()),
        Ok(false) if scope.is_some() => {
            tracing::warn!("systemd kapsamı açılamadı; kapsamsız başlatılıyor");
            let argv = wrap(&plan.argv, tools, None);
            match run(&argv, plan.cwd.as_deref(), token, tools.setsid.is_none())? {
                true => Ok(()),
                false => Err(LaunchError::Spawn(std::io::ErrorKind::Other)),
            }
        }
        Ok(false) => Err(LaunchError::Spawn(std::io::ErrorKind::Other)),
        Err(e) => Err(e),
    }
}

/// Alt sürece geçmeyen ortam değişkenleri: shell'in kendi `NUXUT_*` değişkenleri (örnek adı,
/// test/ayar kancaları) ve shell'e verilmiş eski etkinleştirme belirteçleri.
fn is_scrubbed_env(key: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let k = key.as_bytes();
    k.starts_with(b"NUXUT_") || k == b"XDG_ACTIVATION_TOKEN" || k == b"DESKTOP_STARTUP_ID"
}

/// Komutu çalıştırır. `long_lived`: doğrudan uygulamanın kendisi (ayırıcı yok) — beklenmez,
/// toplayıcı thread'e bırakılır. Dönüş: ara zincir başarıyla çıktı mı.
fn run(argv: &[String], cwd: Option<&Path>, token: Option<&str>, long_lived: bool) -> Result<bool, LaunchError> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    for (key, _) in std::env::vars_os().filter(|(k, _)| is_scrubbed_env(k)) {
        cmd.env_remove(key);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    if let Some(t) = token {
        cmd.env("XDG_ACTIVATION_TOKEN", t).env("DESKTOP_STARTUP_ID", t);
    }
    let mut child = cmd.spawn().map_err(|e| LaunchError::Spawn(e.kind()))?;
    if long_lived {
        let _ = std::thread::Builder::new().name("nuxut-launch-reap".into()).spawn(move || {
            let _ = child.wait();
        });
        return Ok(true);
    }
    match child.wait() {
        Ok(status) => Ok(status.success()),
        // SIGCHLD yok sayılıyorsa çocuk kendiliğinden toplanır: başarı varsay.
        Err(e) if e.raw_os_error() == Some(10 /* ECHILD */) => Ok(true),
        Err(e) => Err(LaunchError::Spawn(e.kind())),
    }
}

/// `org.freedesktop.Application` nesne yolu: `/` + kimlik, `.` → `/`, `-` → `_`.
pub fn dbus_object_path(app_id: &str) -> String {
    let mut path = String::from("/");
    path.extend(app_id.chars().map(|c| match c {
        '.' => '/',
        '-' => '_',
        c => c,
    }));
    path
}

/// DBusActivatable uygulamayı etkinleştirir. `connection` tembel açılır ve yeniden kullanılır.
pub fn dbus_activate(
    connection: &mut Option<zbus::blocking::Connection>,
    entry: &DesktopEntry,
    action: Option<&str>,
    token: Option<&str>,
) -> Result<(), zbus::Error> {
    use zbus::zvariant::{OwnedValue, Value};
    let app_id = entry.id.strip_suffix(".desktop").unwrap_or(&entry.id);
    let name = zbus::names::WellKnownName::try_from(app_id)?;
    let path = zbus::zvariant::ObjectPath::try_from(dbus_object_path(app_id))?;
    if connection.is_none() {
        let conn = zbus::blocking::connection::Builder::session()?
            .method_timeout(std::time::Duration::from_secs(5))
            .build()?;
        *connection = Some(conn);
    }
    let conn = connection.as_ref().expect("bağlantı yukarıda kuruldu");
    let mut platform: HashMap<&str, OwnedValue> = HashMap::new();
    if let Some(t) = token {
        platform.insert("activation-token", Value::from(t).try_into()?);
        platform.insert("desktop-startup-id", Value::from(t).try_into()?);
    }
    let iface = "org.freedesktop.Application";
    match action {
        None => conn.call_method(Some(name), &path, Some(iface), "Activate", &(platform,))?,
        Some(a) => {
            let params: Vec<OwnedValue> = Vec::new();
            conn.call_method(Some(name), &path, Some(iface), "ActivateAction", &(a, params, platform))?
        }
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::services::launcher::desktop::{self, Locale};

    #[test]
    fn shell_env_is_scrubbed() {
        use std::ffi::OsStr;
        for k in ["NUXUT_SHELL_INSTANCE", "NUXUT_TEST_NAME", "NUXUT_", "XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"] {
            assert!(is_scrubbed_env(OsStr::new(k)), "{k}");
        }
        for k in ["PATH", "WAYLAND_DISPLAY", "XNUXUT_X", "nuxut_shell", "NUXUT"] {
            assert!(!is_scrubbed_env(OsStr::new(k)), "{k}");
        }
    }

    fn entry(body: &str) -> DesktopEntry {
        let text = format!("[Desktop Entry]\nType=Application\nName=Uyg\nIcon=uyg\n{body}");
        desktop::parse("org.example.My-App.desktop", Path::new("/apps/my.desktop"), &text, &Locale::default()).unwrap()
    }

    fn tools(dir: &Path, bins: &[&str]) -> Tools {
        for b in bins {
            let p = dir.join(b);
            std::fs::write(&p, "").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Tools { setsid: None, systemd_run: None, path: vec![dir.to_path_buf()], home: Some(dir.to_path_buf()) }
    }

    #[test]
    fn plan_expands_exec_and_actions() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tools(tmp.path(), &["app", "kitty"]);
        let e = entry(
            "Exec=app %U --icon-arg %i --name %c\nActions=new;\n[Desktop Action new]\nName=New\nExec=app --new %f\nIcon=new-icon",
        );
        let opts = LaunchOptions::default();
        let p = plan(&e, None, &opts, &t).unwrap();
        assert_eq!(p.argv, ["app", "--icon-arg", "--icon", "uyg", "--name", "Uyg"]);
        assert_eq!(p.cwd.as_deref(), Some(tmp.path()), "Path yoksa HOME");
        assert_eq!(plan(&e, Some("new"), &opts, &t).unwrap().argv, ["app", "--new"]);
        assert_eq!(plan(&e, Some("yok"), &opts, &t), Err(LaunchError::UnknownAction));
        let missing = entry("Exec=not-installed");
        assert_eq!(plan(&missing, None, &opts, &t), Err(LaunchError::NotFound));
        let dbus_only = entry("DBusActivatable=true");
        assert_eq!(plan(&dbus_only, None, &opts, &t), Err(LaunchError::NoExec));
    }

    #[test]
    fn terminal_and_working_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tools(tmp.path(), &["htop", "foot", "alacritty"]);
        let work = tmp.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let e = entry(&format!("Exec=htop\nTerminal=true\nPath={}", work.display()));
        let p = plan(&e, None, &LaunchOptions::default(), &t).unwrap();
        assert_eq!(p.argv, ["foot", "htop"], "otomatik: listede ilk bulunan");
        assert_eq!(p.cwd.as_deref(), Some(work.as_path()));
        let opts = LaunchOptions { terminal: vec!["alacritty".into(), "-e".into()], ..Default::default() };
        assert_eq!(plan(&e, None, &opts, &t).unwrap().argv, ["alacritty", "-e", "htop"]);
        let opts = LaunchOptions { terminal: vec!["yok-terminal".into()], ..Default::default() };
        assert_eq!(plan(&e, None, &opts, &t), Err(LaunchError::NoTerminal));
        let bad_dir = entry("Exec=htop\nPath=relative/dir");
        assert_eq!(plan(&bad_dir, None, &LaunchOptions::default(), &t).unwrap().cwd.as_deref(), Some(tmp.path()));
    }

    #[test]
    fn wrapping_and_unit_names() {
        assert_eq!(systemd_escape("org.example.My-App"), "org.example.My\\x2dApp");
        assert_eq!(systemd_escape(".x y"), "\\x2ex\\x20y");
        assert_eq!(scope_unit("org.example.My-App.desktop", 0xab), "app-nuxut-org.example.My\\x2dApp-ab.scope");
        let argv = vec!["app".to_owned(), "--x".to_owned()];
        let t = Tools {
            setsid: Some("/usr/bin/setsid".into()),
            systemd_run: Some("/usr/bin/systemd-run".into()),
            ..Default::default()
        };
        assert_eq!(wrap(&argv, &t, Some("u.scope")), [
            "/usr/bin/systemd-run",
            "--user",
            "--scope",
            "--quiet",
            "--collect",
            "--unit=u.scope",
            "--",
            "/usr/bin/setsid",
            "-f",
            "app",
            "--x"
        ]);
        assert_eq!(wrap(&argv, &t, None), ["/usr/bin/setsid", "-f", "app", "--x"]);
        assert_eq!(wrap(&argv, &Tools::default(), Some("u.scope")), ["app", "--x"]);
        assert_eq!(dbus_object_path("org.gnome.Nautilus"), "/org/gnome/Nautilus");
        assert_eq!(dbus_object_path("org.example.My-App"), "/org/example/My_App");
    }

    /// Gerçek başlatma: `setsid -f` ile ayrılan süreç bir işaret dosyası yazar; ortam temizlenir,
    /// token aktarılır, çalışma dizini uygulanır. (systemd kapsamı burada kullanılmaz.)
    #[test]
    fn spawn_detaches_and_sets_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let path: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH").unwrap()).collect();
        let Some(env_bin) = find_executable("env", &path) else { return };
        let marker = tmp.path().join("out");
        // `env` + `sh` yalnızca testte işaret dosyası yazmak için (ürün kodu kabuk çağırmaz).
        // ETXTBSY yarışsız yazım (`tests/support/exec.rs`).
        let script = tmp.path().join("probe");
        crate::test_exec::write_executable(
            &script,
            &format!(
                "#!/bin/sh\necho \"$XDG_ACTIVATION_TOKEN|$DESKTOP_STARTUP_ID|${{NUXUT_SHELL_INSTANCE:-yok}}|$(pwd)\" > {}\n",
                marker.display()
            ),
        );
        let tools = Tools { setsid: find_executable("setsid", &path), systemd_run: None, path, home: None };
        let e = entry(&format!("Exec={} {}\nPath={}", env_bin.display(), script.display(), tmp.path().display()));
        let opts = LaunchOptions { systemd_scope: false, ..Default::default() };
        let p = plan(&e, None, &opts, &tools).unwrap();
        spawn(&e, &p, &opts, &tools, Some("tok123")).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut out = String::new();
        while !out.ends_with('\n') && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            out = std::fs::read_to_string(&marker).unwrap_or_default();
        }
        let canon = tmp.path().canonicalize().unwrap();
        assert_eq!(out.trim(), format!("tok123|tok123|yok|{}", canon.display()));
    }
}
