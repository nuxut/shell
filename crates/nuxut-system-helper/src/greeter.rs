//! `greeter` alanı: giriş ekranını (greetd + `nuxut-greeter`) etkinleştir, geri al, kullanıcının
//! görünümünü giriş ekranına yayımla (ADR-0008 ikinci alanı; ADR-0010, `docs/design/greeter.md`
//! "Kurulum ve geri alma").
//!
//! ```text
//! nuxut-system-helper greeter status
//! nuxut-system-helper greeter apply [--layout L [--variant V] [--options O]] [--screen-reader] [--no-theme]
//! nuxut-system-helper greeter revert
//! ```
//!
//! - Ön koşul: `greetd` paketi ve `greeter` kullanıcısı, giriş ekranı dosyaları (paket ya da
//!   `install.sh`; hepsi kökün, başkalarınca yazılamaz). Eksikse hiçbir şey yazılmaz.
//! - Önce kayıt (`/var/lib/nuxut/greeter/state.json`): önceki display manager (`tty` ya da birim),
//!   greetd ayarının yedeği. İlk kayıt kazanır: ikinci `apply` özgün durumu ezmez.
//! - Hata = bu turun dosya değişiklikleri geri alınır ([`Tx`]); önceki DM kapatıldıysa yeniden açılır.
//! - Görünüm yalnız **çağıranın** (`PKEXEC_UID`) sahip olduğu düz dosyalardan okunur: son bileşen
//!   bağ olamaz (`O_NOFOLLOW`), açılan dosyanın sahibi çağıran olmalı (ara dizindeki bağ başka bir
//!   yere yönlendirse bile sahibi tutmaz: kökün ya da başkasının dosyası sızmaz), boyut sınırlı,
//!   TOML ayrıştırılarak doğrulanır. Argüman olarak yol ya da içerik alınmaz (ADR-0008 ilke 1):
//!   yardımcı dosyaların yerini çağıranın `passwd` ev dizininden kendisi bulur.

use std::io::Read as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use nuxut_boot_theme::files::ThemeFiles;
use serde::{Deserialize, Serialize};

use crate::cmd::{self, Bins, Tool};
use crate::events::{Event, Events, Outcome, StepState};
use crate::ops::{Ops, OpError, Tx, swap_dir};
use crate::policy::{LOCK_DIR, LOCK_NAME};

pub const STATE_DIR: &str = "var/lib/nuxut/greeter";
pub const RECORD: &str = "state.json";
pub const GREETD_CONFIG: &str = "etc/greetd/config.toml";
pub const DM_LINK: &str = "etc/systemd/system/display-manager.service";
pub const PUBLISHED: &str = "var/lib/nuxut-greeter/config/nuxut";
pub const PASSWD: &str = "etc/passwd";
pub const GREETD_BIN: &str = "usr/bin/greetd";
/// Giriş ekranı dosyaları (kökün olmalı).
pub const REQUIRED: [&str; 4] = [
    "usr/bin/nuxut-greeter",
    "usr/lib/nuxut/nuxut-greeter-session",
    "usr/share/nuxut/greeter/hyprland.lua",
    "usr/lib/tmpfiles.d/nuxut-greeter.conf",
];
pub const TMPFILES_ABS: &str = "/usr/lib/tmpfiles.d/nuxut-greeter.conf";
pub const GREETD_UNIT: &str = "greetd.service";
/// Önceki display manager yoksa (tty'den giriş).
pub const NO_DM: &str = "tty";
/// greetd ayarı: sabit içerik, kullanıcı girdisi yok.
pub const TEMPLATE: &str = include_str!("../../../dist/greeter/greetd-config.toml");

const MAX_TOML: u64 = 256 * 1024;
const MAX_WALLPAPER: u64 = 32 * 1024 * 1024;
const MAX_PASSWD: usize = 1 << 20;
const WALLPAPER_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "jxl", "avif", "svg"];

/// Klavye düzeni (`localectl set-x11-keymap`): her alan `[a-z0-9_,:-]`, en çok 64 bayt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Keymap {
    pub layout: String,
    pub variant: String,
    pub options: String,
}

pub fn valid_xkb(s: &str) -> bool {
    s.len() <= 64 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_,:-".contains(&b))
}

impl Keymap {
    pub fn new(layout: &str, variant: &str, options: &str) -> Result<Self, String> {
        if layout.is_empty() || !valid_xkb(layout) || !valid_xkb(variant) || !valid_xkb(options) {
            return Err("geçersiz klavye düzeni".into()); // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
        }
        Ok(Self { layout: layout.into(), variant: variant.into(), options: options.into() })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub keymap: Option<Keymap>,
    pub screen_reader: bool,
    pub theme: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Status,
    Apply(Request),
    Revert,
}

/// `greeter …` argümanları (alan adı çıkarılmış).
pub fn parse<'a>(mut it: impl Iterator<Item = &'a str>) -> Result<Command, String> {
    let verb = it.next().ok_or("eylem eksik")?;
    let (mut layout, mut variant, mut options) = (None::<&str>, None::<&str>, None::<&str>);
    let (mut screen_reader, mut no_theme) = (false, false);
    while let Some(opt) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{opt} değer bekliyor")); // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
        match opt {
            "--layout" if verb == "apply" && layout.is_none() => layout = Some(value()?),
            "--variant" if verb == "apply" && variant.is_none() => variant = Some(value()?),
            "--options" if verb == "apply" && options.is_none() => options = Some(value()?),
            "--screen-reader" if verb == "apply" && !screen_reader => screen_reader = true,
            "--no-theme" if verb == "apply" && !no_theme => no_theme = true,
            _ => return Err(format!("beklenmeyen argüman: {opt}")), // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
        }
    }
    match verb {
        "status" => Ok(Command::Status),
        "revert" => Ok(Command::Revert),
        "apply" => {
            if layout.is_none() && (variant.is_some() || options.is_some()) {
                return Err("--variant/--options --layout ister".into());
            }
            let keymap = layout.map(|l| Keymap::new(l, variant.unwrap_or(""), options.unwrap_or(""))).transpose()?;
            Ok(Command::Apply(Request { keymap, screen_reader, theme: !no_theme }))
        }
        v => Err(format!("bilinmeyen eylem: {v}")),
    }
}

/// [`parse`]'ın tersi (kullanıcı tarafı pkexec argümanlarını bununla kurar).
pub fn apply_args(req: &Request) -> Vec<String> {
    let mut v = vec!["greeter".to_owned(), "apply".to_owned()];
    if let Some(k) = &req.keymap {
        v.extend(["--layout".to_owned(), k.layout.clone()]);
        if !k.variant.is_empty() {
            v.extend(["--variant".to_owned(), k.variant.clone()]);
        }
        if !k.options.is_empty() {
            v.extend(["--options".to_owned(), k.options.clone()]);
        }
    }
    if req.screen_reader {
        v.push("--screen-reader".into());
    }
    if !req.theme {
        v.push("--no-theme".into());
    }
    v
}

/// Kurulum kaydı.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub version: u32,
    /// `tty` ya da birim adı.
    pub previous_dm: String,
    /// greetd ayarı önceden var mıydı; yedeğin yolu (Nuxut'unkiyle aynıysa yedek yok).
    pub config_existed: bool,
    pub config_backup: Option<String>,
}

/// Durum (kök gerekmez: kullanıcı tarafı da aynı dosyalardan hesaplar).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub greetd_installed: bool,
    pub greeter_installed: bool,
    pub display_manager: Option<String>,
    pub previous: Option<String>,
    pub active: bool,
}

/// Birim adı: `[a-z0-9@._-]+.service`.
pub fn valid_unit(name: &str) -> bool {
    name.len() <= 128
        && name.ends_with(".service")
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"@._-".contains(&b))
}

/// `passwd` satırlarından uid'in ev dizini (mutlak).
pub fn home_of(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 7 && f[2].parse::<u32>().ok() == Some(uid) && f[5].starts_with('/') && !f[5].split('/').any(|c| c == ".."))
            .then(|| f[5].to_owned())
    })
}

fn user_exists(passwd: &str, name: &str) -> bool {
    passwd.lines().any(|l| l.split(':').next() == Some(name))
}

// Linux değerleri (libc'siz; unsafe yasak). O_NOFOLLOW mimariye göre değişir.
#[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
const O_NOFOLLOW: i32 = 0o100000;
#[cfg(not(any(target_arch = "aarch64", target_arch = "arm")))]
const O_NOFOLLOW: i32 = 0o400000;
const O_NONBLOCK: i32 = 0o4000;

/// Kullanıcı dosyası hatası (olay kodu; metin değil).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserFileError {
    Link,
    NotFile,
    Foreign,
    TooLarge,
    Invalid,
    Io,
}

impl UserFileError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Link => "symlink",
            Self::NotFile => "not-file",
            Self::Foreign => "foreign-owner",
            Self::TooLarge => "too-large",
            Self::Invalid => "invalid",
            Self::Io => "io",
        }
    }
}

/// Çağıranın dosyası: son bileşen bağ değil, düz dosya, sahibi `uid`, `max` baytı aşmaz.
/// Yoksa `Ok(None)`.
pub fn read_user_file(path: &Path, uid: u32, max: u64) -> Result<Option<Vec<u8>>, UserFileError> {
    use std::os::unix::fs::OpenOptionsExt as _;
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(UserFileError::Link);
    }
    let file = match std::fs::OpenOptions::new().read(true).custom_flags(O_NOFOLLOW | O_NONBLOCK).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(UserFileError::Io),
    };
    let meta = file.metadata().map_err(|_| UserFileError::Io)?;
    if !meta.is_file() {
        return Err(UserFileError::NotFile);
    }
    if meta.uid() != uid {
        return Err(UserFileError::Foreign);
    }
    if meta.len() > max {
        return Err(UserFileError::TooLarge);
    }
    let mut buf = Vec::new();
    file.take(max + 1).read_to_end(&mut buf).map_err(|_| UserFileError::Io)?;
    if buf.len() as u64 > max {
        return Err(UserFileError::TooLarge);
    }
    Ok(Some(buf))
}

fn read_user_toml(path: &Path, uid: u32) -> Result<Option<toml::Table>, UserFileError> {
    let Some(bytes) = read_user_file(path, uid, MAX_TOML)? else { return Ok(None) };
    let text = String::from_utf8(bytes).map_err(|_| UserFileError::Invalid)?;
    text.parse::<toml::Table>().map(Some).map_err(|_| UserFileError::Invalid)
}

fn toml_bytes(t: &toml::Table) -> Vec<u8> {
    toml::to_string(t).unwrap_or_default().into_bytes()
}

fn err(s: impl Into<String>) -> OpError {
    OpError::Other(s.into())
}

type OpResult<T> = std::result::Result<T, OpError>;

#[derive(Debug)]
pub struct Greeter<'a> {
    pub ops: &'a Ops,
    pub bins: &'a Bins,
    pub events: &'a Events,
    /// Görünümü yayımlanacak kullanıcı (`PKEXEC_UID`); yoksa yayın atlanır.
    pub caller: Option<u32>,
}

impl Greeter<'_> {
    fn step(&self, step: &str, state: StepState) {
        self.events.emit(Event::LoginStep { step: step.to_owned(), state });
    }

    fn run(&self, tool: Tool, args: &[&str]) -> OpResult<()> {
        cmd::run(self.bins, tool, args, self.events).map_err(|e| err(e.to_string()))
    }

    fn record_rel() -> String {
        format!("{STATE_DIR}/{RECORD}")
    }

    fn load_record(&self) -> OpResult<Option<Record>> {
        match self.ops.read(&Self::record_rel(), 64 * 1024)? {
            Some(b) => serde_json::from_slice(&b).map(Some).map_err(|e| err(format!("kayıt bozuk: {e}"))), // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
            None => Ok(None),
        }
    }

    fn current_dm(&self) -> Option<String> {
        let target = std::fs::read_link(self.ops.root.abs(DM_LINK)).ok()?;
        let name = target.file_name()?.to_str()?.to_owned();
        valid_unit(&name).then_some(name)
    }

    /// Dosya kökün (testte beklenen sahibin) ve başkalarınca yazılamaz düz dosya mı.
    fn system_file_ok(&self, rel: &str) -> bool {
        let Ok((dir, name)) = crate::safefs::split(rel) else { return false };
        let owned = self.ops.root.dir(dir).and_then(|d| d.owned_file(name)).unwrap_or(false);
        let mode_ok = std::fs::symlink_metadata(self.ops.root.abs(rel)).is_ok_and(|m| m.is_file() && m.mode() & 0o022 == 0);
        owned && mode_ok
    }

    fn passwd(&self) -> String {
        self.ops.read(PASSWD, MAX_PASSWD).ok().flatten().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
    }

    pub fn status(&self) -> Status {
        let previous = self.load_record().ok().flatten().map(|r| r.previous_dm);
        let display_manager = self.current_dm();
        let st = Status {
            greetd_installed: self.ops.exists(GREETD_BIN),
            greeter_installed: REQUIRED.iter().all(|f| self.ops.exists(f)),
            active: display_manager.as_deref() == Some(GREETD_UNIT) && previous.is_some(),
            display_manager,
            previous,
        };
        self.events.emit(Event::LoginScreen { status: st.clone() });
        st
    }

    fn finish(&self, outcome: Outcome, error: Option<String>) -> bool {
        let ok = error.is_none();
        self.events.emit(Event::LoginResult { outcome, message: error });
        self.events.emit(Event::Done { ok, reboot_hint: ok && outcome != Outcome::Skipped });
        ok
    }

    pub fn apply(&self, req: &Request) -> bool {
        let _lock = match self.ops.root.dir(LOCK_DIR).and_then(|d| d.lock_file(LOCK_NAME)) {
            Ok(l) => l,
            Err(e) => return self.finish(Outcome::Skipped, Some(e.to_string())),
        };
        // Ön koşullar: hiçbir şey yazılmadan.
        if !self.ops.exists(GREETD_BIN) {
            return self.finish(Outcome::Skipped, Some("greetd-missing".into()));
        }
        if !user_exists(&self.passwd(), "greeter") {
            return self.finish(Outcome::Skipped, Some("greeter-user-missing".into()));
        }
        if let Some(f) = REQUIRED.iter().find(|f| !self.system_file_ok(f)) {
            return self.finish(Outcome::Skipped, Some(format!("untrusted-file:/{f}")));
        }
        let mut tx = Tx::default();
        let mut disabled: Option<String> = None;
        match self.apply_inner(req, &mut tx, &mut disabled) {
            Ok(()) => {
                for w in tx.commit(self.ops) {
                    self.events.warn(None, w);
                }
                self.finish(Outcome::Applied, None)
            }
            Err(e) => {
                // Önceki DM kapatıldıysa yeniden aç (greetd açılamadı).
                if let Some(dm) = disabled {
                    let _ = self.run(Tool::Systemctl, &["enable", &dm]);
                }
                let errors = tx.rollback(self.ops);
                let outcome = if errors.is_empty() { Outcome::RolledBack } else { Outcome::Inconsistent };
                self.finish(outcome, Some(e.to_string()))
            }
        }
    }

    fn apply_inner(&self, req: &Request, tx: &mut Tx, disabled: &mut Option<String>) -> OpResult<()> {
        // 1. Kayıt (ilk kayıt kazanır).
        self.step("record", StepState::Start);
        let old_config = self.ops.read(GREETD_CONFIG, MAX_TOML as usize)?;
        let record = match self.load_record()? {
            Some(r) => r,
            None => {
                let previous_dm = self.current_dm().unwrap_or_else(|| NO_DM.into());
                let config_backup = match &old_config {
                    Some(b) if b.as_slice() != TEMPLATE.as_bytes() => self.ops.backup(GREETD_CONFIG)?,
                    _ => None,
                };
                let r = Record { version: 1, previous_dm, config_existed: old_config.is_some(), config_backup };
                self.ops.ensure_dir(STATE_DIR)?;
                self.ops.write(&Self::record_rel(), &serde_json::to_vec_pretty(&r).map_err(|e| err(e.to_string()))?)?;
                r
            }
        };
        self.step("record", StepState::Done);
        // 2. greetd ayarı.
        self.step("config", StepState::Start);
        match old_config {
            Some(b) => tx.on_fail_write(GREETD_CONFIG, b),
            None => tx.on_fail_remove_file(GREETD_CONFIG),
        }
        self.ops.ensure_dir("etc/greetd")?;
        self.ops.write(GREETD_CONFIG, TEMPLATE.as_bytes())?;
        self.run(Tool::SystemdTmpfiles, &["--create", TMPFILES_ABS])?;
        self.step("config", StepState::Done);
        // 3. Görünüm.
        if req.theme {
            match self.caller {
                Some(uid) => {
                    self.step("theme", StepState::Start);
                    self.publish(uid, req.screen_reader, tx)?;
                    self.step("theme", StepState::Done);
                }
                None => self.events.warn(None, "theme-skipped:no-caller"),
            }
        }
        // 4. Display manager: öncekini kapat, sonra greetd'yi aç.
        self.step("display-manager", StepState::Start);
        let prev = record.previous_dm.as_str();
        if prev != NO_DM && prev != GREETD_UNIT && valid_unit(prev) {
            self.run(Tool::Systemctl, &["disable", prev])?;
            *disabled = Some(prev.to_owned());
        }
        self.run(Tool::Systemctl, &["enable", GREETD_UNIT])?;
        self.step("display-manager", StepState::Done);
        // 5. Klavye düzeni (başarısızlık uyarı: giriş ekranı yine açılır, varsayılan düzenle).
        if let Some(k) = &req.keymap {
            self.step("keymap", StepState::Start);
            if let Err(e) = self.run(Tool::Localectl, &["set-x11-keymap", &k.layout, "", &k.variant, &k.options]) {
                self.events.warn(None, format!("keymap: {e}"));
            }
            self.step("keymap", StepState::Done);
        }
        Ok(())
    }

    /// Görünümü yayımla (yeni dizin hazırlanıp yerine konur; hata tx ile geri alınır).
    fn publish(&self, uid: u32, screen_reader: bool, tx: &mut Tx) -> OpResult<()> {
        let home = home_of(&self.passwd(), uid).ok_or_else(|| err("caller-home-missing"))?;
        let user = |rel: &str| PathBuf::from(self.ops.root.abs(&format!("{}/{rel}", home.trim_start_matches('/'))));
        let fail = |what: &str, e: UserFileError| err(format!("user-file:{what}:{}", e.code()));
        let mut files = ThemeFiles::default();
        let published = format!("/{PUBLISHED}");

        let shell = read_user_toml(&user(".config/nuxut/shell.toml"), uid).map_err(|e| fail("shell.toml", e))?;
        let mut wallpaper: Option<String> = None;
        if let Some(path) = shell.as_ref().and_then(|t| t.get("wallpaper")).and_then(|w| w.get("path")).and_then(|v| v.as_str()) {
            if path.starts_with("nuxut:") && path.len() <= 128 {
                wallpaper = Some(path.to_owned());
            } else if path.starts_with('/') && !path.split('/').any(|c| c == "..") {
                let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
                if WALLPAPER_EXTS.contains(&ext.as_str()) {
                    match read_user_file(&PathBuf::from(self.ops.root.abs(path.trim_start_matches('/'))), uid, MAX_WALLPAPER) {
                        Ok(Some(bytes)) => {
                            let name = format!("wallpaper.{ext}");
                            files.push(name.clone(), bytes);
                            wallpaper = Some(format!("{published}/{name}"));
                        }
                        Ok(None) => self.events.warn(None, "wallpaper:missing"),
                        Err(e) => self.events.warn(None, format!("wallpaper:{}", e.code())),
                    }
                } else {
                    self.events.warn(None, "wallpaper:unsupported");
                }
            }
        }
        let mut shell_out = toml::Table::new();
        if let Some(w) = &wallpaper {
            let mut t = toml::Table::new();
            t.insert("path".into(), toml::Value::String(w.clone()));
            shell_out.insert("wallpaper".into(), toml::Value::Table(t));
        }
        files.push("shell.toml", toml_bytes(&shell_out));
        if let Some(t) = read_user_toml(&user(".config/nuxut/framework.toml"), uid).map_err(|e| fail("framework.toml", e))? {
            files.push("framework.toml", toml_bytes(&t));
        }
        if let Some(mut t) = read_user_toml(&user(".config/nuxut/lock.toml"), uid).map_err(|e| fail("lock.toml", e))? {
            // Kilidin kendi duvar kağıdı kullanıcının evinde (greeter okuyamaz): masaüstününki geçer.
            if t.get("wallpaper").and_then(|v| v.as_str()).is_some_and(|s| !s.starts_with("nuxut:")) {
                t.remove("wallpaper");
            }
            files.push("lock.toml", toml_bytes(&t));
        }
        if let Some(mut t) = read_user_toml(&user(".local/state/nuxut/palette.toml"), uid).map_err(|e| fail("palette.toml", e))? {
            if let Some(meta) = t.get_mut("meta").and_then(|m| m.as_table_mut()) {
                match &wallpaper {
                    Some(w) => {
                        meta.insert("image".into(), toml::Value::String(w.clone()));
                    }
                    None => {
                        meta.remove("image");
                    }
                }
            }
            files.push("palette.toml", toml_bytes(&t));
        }
        files.push("greeter.toml", format!("screen_reader = {screen_reader}\n").into_bytes());
        let (parent, _) = crate::safefs::split(PUBLISHED)?;
        self.ops.ensure_dir(parent)?;
        let staged = self.ops.stage_dir(PUBLISHED, &files)?;
        swap_dir(self.ops, tx, PUBLISHED, &staged, false)?;
        Ok(())
    }

    pub fn revert(&self) -> bool {
        let _lock = match self.ops.root.dir(LOCK_DIR).and_then(|d| d.lock_file(LOCK_NAME)) {
            Ok(l) => l,
            Err(e) => return self.finish(Outcome::Skipped, Some(e.to_string())),
        };
        let record = match self.load_record() {
            Ok(Some(r)) => r,
            Ok(None) => return self.finish(Outcome::Skipped, Some("no-record".into())),
            Err(e) => return self.finish(Outcome::Skipped, Some(e.to_string())),
        };
        match self.revert_inner(&record) {
            Ok(()) => self.finish(Outcome::Reverted, None),
            Err(e) => self.finish(Outcome::Inconsistent, Some(e.to_string())),
        }
    }

    fn revert_inner(&self, r: &Record) -> OpResult<()> {
        self.step("display-manager", StepState::Start);
        let prev = r.previous_dm.as_str();
        if prev != GREETD_UNIT {
            self.run(Tool::Systemctl, &["disable", GREETD_UNIT])?;
        }
        if prev != NO_DM && prev != GREETD_UNIT && valid_unit(prev) {
            self.run(Tool::Systemctl, &["enable", prev])?;
        }
        self.step("display-manager", StepState::Done);
        self.step("config", StepState::Start);
        match &r.config_backup {
            Some(b) => self.ops.rename(b, GREETD_CONFIG)?,
            None if !r.config_existed => self.ops.remove_file(GREETD_CONFIG)?,
            None => {}
        }
        self.step("config", StepState::Done);
        self.ops.remove_file(&Self::record_rel())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
