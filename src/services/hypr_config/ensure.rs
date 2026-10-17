//! Hyprland ayar dosyasının yolu ve `source` satırı kararı (saf: [`plan_source`]). Satırların
//! kendisi ([`CONF_COMMENT`], [`source_line`]) `nuxut-apply` `Fragment::Line` olarak eklenir
//! (`hypr.source` snapshot'ı; yazım `sync.rs`): içerik bayt bayt korunur, sona yalnızca yorum +
//! dahil etme satırı eklenir.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::parse::ChainReport;

/// Hyprland'in komut satırından `-c/--config` yolu (`/proc/<pid>/cmdline` argümanları,
/// program adı hariç). `--config=yol` biçimi de.
pub fn config_path_from_args<I, S>(args: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let a = a.as_ref();
        if a == "-c" || a == "--config" {
            return it.next().map(|p| PathBuf::from(p.as_ref())).filter(|p| !p.as_os_str().is_empty());
        }
        if let Some(rest) = a.to_str().and_then(|s| s.strip_prefix("--config=")) {
            return (!rest.is_empty()).then(|| PathBuf::from(rest));
        }
    }
    None
}

/// `/proc/<pid>/cmdline` içeriğini argümanlara böler (ilk öğe program adı, atlanır).
pub fn split_cmdline(raw: &[u8]) -> Vec<&OsStr> {
    use std::os::unix::ffi::OsStrExt;
    raw.split(|&b| b == 0).skip(1).filter(|a| !a.is_empty()).map(OsStr::from_bytes).collect()
}

/// Varsayılan yol: `$XDG_CONFIG_HOME/hypr/hyprland.conf`, yoksa `$HOME/.config/hypr/…`.
/// `XDG_CONFIG_HOME` mutlak değilse yok sayılır (XDG kuralı).
pub fn default_config_path(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let base = match xdg_config_home.map(Path::new).filter(|p| p.is_absolute()) {
        Some(x) => x.to_path_buf(),
        None => Path::new(home.filter(|h| !h.is_empty())?).join(".config"),
    };
    Some(base.join("hypr").join("hyprland.conf"))
}

/// Shell nasıl başlatılıyor (nuxut.conf'ta `exec-once` yazılsın mı)?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// nuxut.conf `exec-once` içerir.
    ExecOnce,
    /// Kullanıcının zincirinde başka bir `exec-once nuxut-shell` var.
    External,
    /// systemd kullanıcı birimi altında.
    Systemd,
}

impl LaunchMode {
    pub fn name(self) -> &'static str {
        match self {
            LaunchMode::ExecOnce => "exec_once",
            LaunchMode::External => "external",
            LaunchMode::Systemd => "systemd",
        }
    }
}

/// `under_systemd`: süreç ortamında `INVOCATION_ID` ya da `SYSTEMD_EXEC_PID` var.
pub fn launch_mode(under_systemd: bool, report: &ChainReport) -> LaunchMode {
    if under_systemd {
        LaunchMode::Systemd
    } else if report.external_launch.is_some() {
        LaunchMode::External
    } else {
        LaunchMode::ExecOnce
    }
}

/// `source` satırı kararı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePlan {
    /// `[compositor] ensure_source = false`.
    Off,
    /// Ana ayar dosyası yok (oluşturulmaz).
    MissingConfig,
    /// Ana dosya `.conf` değil.
    Unsupported,
    /// Tarama eksik kaldı (sınır/okuma hatası): güvenli tarafta, eklenmez.
    Unknown,
    /// Zaten source ediliyor.
    Present,
    /// Kullanıcı satırı yorumlamış.
    DisabledByUser,
    /// Daha önce eklemiştik, kullanıcı silmiş: tekrar eklenmez.
    RemovedByUser,
    /// Eklenecek.
    Add,
}

impl SourcePlan {
    /// `msg hypr status` adı.
    pub fn name(self) -> &'static str {
        match self {
            SourcePlan::Off => "off",
            SourcePlan::MissingConfig => "missing_config",
            SourcePlan::Unsupported => "unsupported",
            SourcePlan::Unknown => "unknown",
            SourcePlan::Present => "present",
            SourcePlan::DisabledByUser => "disabled_by_user",
            SourcePlan::RemovedByUser => "removed_by_user",
            SourcePlan::Add => "add",
        }
    }
}

/// Karar girdisi.
#[derive(Debug, Clone, Copy)]
pub struct PlanInput<'a> {
    pub enabled: bool,
    pub config_exists: bool,
    pub config_path: &'a Path,
    /// Durum dosyası: daha önce biz ekledik mi?
    pub previously_added: bool,
    pub report: &'a ChainReport,
}

pub fn plan_source(i: &PlanInput<'_>) -> SourcePlan {
    let r = i.report;
    if !i.enabled {
        SourcePlan::Off
    } else if !i.config_exists {
        SourcePlan::MissingConfig
    } else if i.config_path.extension() != Some(OsStr::new("conf")) {
        SourcePlan::Unsupported
    } else if r.source_at.is_some() {
        // Kesik taramada bile bulunduysa kesindir.
        SourcePlan::Present
    } else if r.truncated {
        SourcePlan::Unknown
    } else if r.commented_at.is_some() {
        SourcePlan::DisabledByUser
    } else if i.previously_added {
        SourcePlan::RemovedByUser
    } else {
        SourcePlan::Add
    }
}

/// `source` satırında yazılacak yol: ev dizini altındaysa `~/…`.
pub fn display_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// Dahil etme satırının üstündeki açıklama (hyprlang). Eski sürümlerin eklediği blokla aynı
/// metin: geri al ve snapshot her ikisini de tanır.
pub const CONF_COMMENT: &str =
    "# Nuxut: kabuk ayarları (ekleyen nuxut-shell; kapatmak için shell.toml [compositor] ensure_source = false)"; // i18n: tanı hyprland.conf'a yazılan yorum satırı (dosya içeriği)
/// Lua karşılığı.
pub const LUA_COMMENT: &str =
    "-- Nuxut: kabuk ayarları (ekleyen nuxut-shell; kapatmak için shell.toml [compositor] ensure_source = false)"; // i18n: tanı kullanıcı zincirine yazılan Lua yorum satırı (dosya içeriği)

/// `source = <hedef>` satırı. Hedefte denetim karakteri ya da `#` (hyprlang yorumu) varsa
/// `None`: satır başka bir anahtara dönüşemez.
pub fn source_line(target_display: &str) -> Option<String> {
    let ok = !target_display.is_empty() && !target_display.chars().any(|c| c.is_control() || c == '#');
    ok.then(|| format!("source = {target_display}"))
}

#[cfg(test)]
mod tests {
    use super::super::parse::{Location, ScanInput, ScanLimits, scan_chain};
    use super::*;

    #[test]
    fn config_path_detection() {
        assert_eq!(config_path_from_args(["--i-am-really-stupid", "-c", "/x/h.conf"]), Some("/x/h.conf".into()));
        assert_eq!(config_path_from_args(["--config=/y.conf"]), Some("/y.conf".into()));
        assert_eq!(config_path_from_args(["-c"]), None);
        assert_eq!(config_path_from_args(Vec::<&str>::new()), None);
        let raw = b"Hyprland\0--config\0/a b/h.conf\0";
        assert_eq!(config_path_from_args(split_cmdline(raw)), Some("/a b/h.conf".into()));
        assert_eq!(
            default_config_path(Some(OsStr::new("/xdg")), Some(OsStr::new("/home/u"))),
            Some("/xdg/hypr/hyprland.conf".into())
        );
        assert_eq!(
            default_config_path(Some(OsStr::new("rel")), Some(OsStr::new("/home/u"))),
            Some("/home/u/.config/hypr/hyprland.conf".into())
        );
        assert_eq!(default_config_path(None, None), None);
    }

    fn plan(report: &ChainReport, enabled: bool, exists: bool, prev: bool, path: &str) -> SourcePlan {
        plan_source(&PlanInput { enabled, config_exists: exists, config_path: Path::new(path), previously_added: prev, report })
    }

    #[test]
    fn plan_table() {
        let at = Location { file: 0, line: 1 };
        let empty = ChainReport::default();
        let p = "/h/hyprland.conf";
        assert_eq!(plan(&empty, false, true, false, p), SourcePlan::Off);
        assert_eq!(plan(&empty, true, false, false, p), SourcePlan::MissingConfig);
        assert_eq!(plan(&empty, true, true, false, "/h/hyprland.lua"), SourcePlan::Unsupported);
        assert_eq!(plan(&ChainReport { source_at: Some(at), truncated: true, ..Default::default() }, true, true, false, p), SourcePlan::Present);
        assert_eq!(plan(&ChainReport { truncated: true, ..Default::default() }, true, true, false, p), SourcePlan::Unknown);
        assert_eq!(plan(&ChainReport { commented_at: Some(at), ..Default::default() }, true, true, false, p), SourcePlan::DisabledByUser);
        assert_eq!(plan(&empty, true, true, true, p), SourcePlan::RemovedByUser);
        assert_eq!(plan(&empty, true, true, false, p), SourcePlan::Add);
    }

    #[test]
    fn launch_modes() {
        let ext = ChainReport { external_launch: Some(Location { file: 1, line: 3 }), ..Default::default() };
        assert_eq!(launch_mode(true, &ext), LaunchMode::Systemd);
        assert_eq!(launch_mode(false, &ext), LaunchMode::External);
        assert_eq!(launch_mode(false, &ChainReport::default()), LaunchMode::ExecOnce);
    }

    #[test]
    fn source_line_rules() {
        assert_eq!(source_line("~/.config/hypr/nuxut.conf").as_deref(), Some("source = ~/.config/hypr/nuxut.conf"));
        assert_eq!(source_line("a\nexec = rm"), None);
        assert_eq!(source_line("a # b"), None);
        assert_eq!(source_line(""), None);
        assert_eq!(display_path(Path::new("/home/u/.config/hypr/nuxut.conf"), Some(Path::new("/home/u"))), "~/.config/hypr/nuxut.conf");
        assert_eq!(display_path(Path::new("/etc/x"), Some(Path::new("/home/u"))), "/etc/x");
        // Eklenen satırlar taramada "var" sayılır; açıklama satırı "kullanıcı kapatmış" sayılmaz.
        let tmp = tempfile::tempdir().unwrap();
        let conf = tmp.path().join("hyprland.conf");
        let target = tmp.path().join("nuxut.conf");
        std::fs::write(&target, "").unwrap();
        let line = source_line(&target.display().to_string()).unwrap();
        std::fs::write(&conf, format!("a = 1\n{CONF_COMMENT}\n{line}\n")).unwrap();
        let report = scan_chain(&ScanInput { root: &conf, target: &target, home: None, watch_keys: &[], skip: &[], limits: ScanLimits::default() });
        assert!(report.source_at.is_some() && report.commented_at.is_none());
        std::fs::write(&conf, format!("a = 1\n{CONF_COMMENT}\n")).unwrap();
        let report = scan_chain(&ScanInput { root: &conf, target: &target, home: None, watch_keys: &[], skip: &[], limits: ScanLimits::default() });
        assert!(report.source_at.is_none() && report.commented_at.is_none());
    }
}
