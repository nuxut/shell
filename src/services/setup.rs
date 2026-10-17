//! İlk açılış / Hoş geldin durumu (ADR-0001 §10, "Uygulama").
//!
//! - Dosya: `$XDG_STATE_HOME/nuxut/setup.toml` (yoksa `~/.local/state/nuxut/setup.toml`).
//!   **Tek yazan shell'dir**: Ayarlar sihirbazı bitirince/atlayınca `nuxut-shell msg setup
//!   done|skip <sürüm>` çağırır; shell açılışta dosya yoksa `nuxut-settings --welcome`'ı bir kez
//!   başlatır ve kendisi dosya yazmaz.
//! - İçerik (shell'e özel, donuk v1):
//!   ```toml
//!   state = "done"            # ya da "skipped"
//!   version = 1               # sihirbaz sürümü, 1..=65535
//!   at = "2026-10-11T09:30:00Z"
//!   ```
//!   Bilinmeyen anahtarlar yok sayılır (ileri uyum); `at` bilgi amaçlıdır.
//! - Okuma [`MAX_SETUP_BYTES`] ile sınırlı; bozuk dosya panik değil, [`SetupRead::Corrupt`].
//! - Yazım `nuxut-apply` ile (`services::apply::write_own`: atomik, symlink korunur, ortam
//!   kapısı); dizin yoksa `0700` oluşturulur. Snapshot yok (Nuxut'un kendi durum dosyası).
//! - Sürüm düşürülmez: daha küçük sürümle gelen `done/skip` daha büyük kayıtlı sürümü ezmez
//!   (no-op, yanıt yine `ok`).
//!
//! UI'dan bağımsızdır; IO yalnızca küçük bir dosya (ana thread'de çağrılması kabul edilebilir).

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

use nuxut_apply::{Env, TargetKind};
use serde::{Deserialize, Serialize};

use nuxut::i18n::t;
use crate::services::apply;

/// setup.toml okuma üst sınırı (geçerli dosya ~80 bayt).
pub const MAX_SETUP_BYTES: u64 = 4096;
/// Dosya adı (`<state>/nuxut/` altında).
pub const SETUP_FILE: &str = "setup.toml";
/// Ayarlar ikilisinin adı.
pub const SETTINGS_BIN: &str = "nuxut-settings";
/// Hoş geldin penceresini açan argüman.
pub const WELCOME_ARG: &str = "--welcome";

/// `shell.toml [setup]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SetupConfig {
    /// İlk açılışta (setup.toml yokken) Hoş geldin penceresini aç.
    pub welcome: bool,
}

impl Default for SetupConfig {
    fn default() -> Self {
        Self { welcome: true }
    }
}

impl SetupConfig {
    pub fn schema(out: &mut Vec<crate::schema::SettingSchema>) {
        out.push(crate::schema::SettingSchema::bool("setup.welcome", t!("shell-setup-welcome-label"), Self::default().welcome));
    }
}

/// Sihirbazın sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SetupState {
    /// Kullanıcı adımları tamamladı.
    Done,
    /// Kullanıcı pencereyi kapattı/atladı (Ayarlar'dan yeniden açılabilir).
    Skipped,
}

impl SetupState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Skipped => "skipped",
        }
    }
}

/// Diskteki kayıt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupRecord {
    pub state: SetupState,
    pub version: u16,
    /// RFC 3339 (UTC). Bilgi amaçlı; eksikse kayıt yine geçerli.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// setup.toml'un okunmuş hâli.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupRead {
    /// Dosya yok: ilk açılış.
    Missing,
    Present(SetupRecord),
    /// Okunamadı/ayrıştırılamadı (neden log içindir; dosya içeriği girmez).
    Corrupt(String),
}

impl SetupRead {
    /// İlk açılış mı? Yalnızca dosya **yoksa**. Bozuk dosya ilk açılış sayılmaz: kullanıcı
    /// sihirbazı zaten görmüş ve bir şey yazılmış demektir; bozulmanın nedeni (disk, el
    /// düzenlemesi, ileri sürüm biçimi) ne olursa olsun her açılışta pencereyi yeniden açmak
    /// kullanıcıyı rahatsız eder ve yazım hatası kalıcıysa döngüye girer. Uyarı loglanır;
    /// Ayarlar'dan sihirbaz yeniden açılıp `setup done/skip` ile dosya onarılabilir.
    pub fn first_run(&self) -> bool {
        matches!(self, Self::Missing)
    }

    pub fn record(&self) -> Option<&SetupRecord> {
        match self {
            Self::Present(r) => Some(r),
            _ => None,
        }
    }
}

/// `<state_home>/nuxut/setup.toml` (`Env::state_home`: mutlak `$XDG_STATE_HOME`, yoksa
/// `$HOME/.local/state`).
pub fn setup_path(env: &Env) -> Option<PathBuf> {
    Some(env.nuxut_state_dir()?.join(SETUP_FILE))
}

/// Metni ayrıştırır (saf).
pub fn parse(text: &str) -> Result<SetupRecord, String> {
    #[derive(Deserialize)]
    struct Raw {
        state: SetupState,
        version: i64,
        at: Option<String>,
    }
    let raw: Raw = toml::from_str(text).map_err(|e| format!("ayrıştırılamadı: {}", e.message()))?;
    let version = u16::try_from(raw.version).ok().filter(|v| *v >= 1).ok_or("version 1..=65535 olmalı")?;
    if let Some(at) = &raw.at
        && chrono::DateTime::parse_from_rfc3339(at).is_err()
    {
        return Err("at RFC 3339 değil".into());
    }
    Ok(SetupRecord { state: raw.state, version, at: raw.at })
}

/// Kaydı dosya metnine çevirir (saf).
pub fn render(record: &SetupRecord) -> String {
    // i18n: tanı makine dosyasının başlık yorumu
    let mut out = String::from("# Nuxut ilk açılış durumu; shell yazar (nuxut-shell msg setup …).\n");
    out.push_str(&toml::to_string(record).unwrap_or_default());
    out
}

/// Dosyayı sınırla okur.
pub fn read(path: &Path) -> SetupRead {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SetupRead::Missing,
        Err(e) => return SetupRead::Corrupt(format!("açılamadı ({:?})", e.kind())),
    };
    match file.metadata() {
        Ok(m) if !m.is_file() => return SetupRead::Corrupt("normal dosya değil".into()),
        Ok(m) if m.len() > MAX_SETUP_BYTES => return SetupRead::Corrupt(format!("{MAX_SETUP_BYTES} bayttan büyük")),
        Ok(_) => {}
        Err(e) => return SetupRead::Corrupt(format!("okunamadı ({:?})", e.kind())),
    }
    let mut bytes = Vec::new();
    // Ölçümle okuma arasında dosya büyüyebilir: okuma da sınırlı.
    if let Err(e) = file.take(MAX_SETUP_BYTES + 1).read_to_end(&mut bytes) {
        return SetupRead::Corrupt(format!("okunamadı ({:?})", e.kind()));
    }
    if bytes.len() as u64 > MAX_SETUP_BYTES {
        return SetupRead::Corrupt(format!("{MAX_SETUP_BYTES} bayttan büyük"));
    }
    let Ok(text) = String::from_utf8(bytes) else { return SetupRead::Corrupt("UTF-8 değil".into()) };
    match parse(&text) {
        Ok(r) => SetupRead::Present(r),
        Err(e) => SetupRead::Corrupt(e),
    }
}

/// `done/skip` yazılmalı mı? Daha büyük kayıtlı sürüm düşürülmez; aynı sürüm ve aynı durum
/// yeniden yazılmaz (`at` korunur). Aynı sürümde durum değişebilir (atlandı → bitti).
/// Bozuk/eksik dosya her zaman yazılır (onarım).
pub fn should_write(current: &SetupRead, state: SetupState, version: u16) -> bool {
    match current.record() {
        None => true,
        Some(r) => version > r.version || (version == r.version && state != r.state),
    }
}

/// Yazım sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    Written,
    /// Sürüm düşürmeme ya da aynı kayıt: dosyaya dokunulmadı.
    Unchanged,
}

/// `setup done|skip <version>`: gerekiyorsa kaydı `nuxut-apply` ile atomik yazar (ortam kapısı:
/// test örneği yalnızca kendi geçici `XDG_STATE_HOME`'una, kapalı ortam hiçbir yere). `now`:
/// RFC 3339 zaman damgası. Hata: ADR-0001 §10.3 adı (`skipped(test_instance)`, …).
pub fn record(env: &Env, path: &Path, state: SetupState, version: u16, now: &str) -> Result<WriteOutcome, String> {
    if version == 0 {
        return Err("invalid".into());
    }
    // Kapı okumadan önce: reddedilecek örnek dosyayı da açmaz.
    env.guard(TargetKind::File).map_err(|s| apply::error_text(&s.into()))?;
    let current = read(path);
    if let SetupRead::Corrupt(reason) = &current {
        tracing::warn!(reason = %reason, "setup.toml bozuk; yeni kayıtla değiştiriliyor");
    }
    if !should_write(&current, state, version) {
        return Ok(WriteOutcome::Unchanged);
    }
    let text = render(&SetupRecord { state, version, at: Some(now.to_owned()) });
    if let Some(dir) = path.parent() {
        apply::ensure_private_dir(env, dir).map_err(|e| apply::error_text(&e))?;
    }
    apply::write_own(env, path, text.as_bytes()).map_err(|e| apply::error_text(&e))?;
    Ok(WriteOutcome::Written)
}

/// Şimdiki zaman, RFC 3339 UTC saniye çözünürlüğü.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Bu süreç setup.toml'a yazabilir mi, nereden okur? Kural tek yerde: `nuxut-apply`
/// ortam kapısı ([`Env::guard`]).
/// - Tek-örnek kilidini almış varsayılan örnek kullanıcının durum dizinine yazar.
/// - `NUXUT_SHELL_INSTANCE`'lı test örneği yalnızca gerçek ev dizini dışındaki mutlak
///   `XDG_STATE_HOME`/`XDG_CONFIG_HOME` altına yazar (bağlantı çözülerek); aksi hâlde yazmaz
///   (okuma serbest; yalnızca durum bildirir).
/// - Kilidi alamamış örnek yazmaz (`Env::disabled`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupAccess {
    /// Okunan (ve izin varsa yazılan) dosya; durum dizini çözülemezse `None`.
    pub path: Option<PathBuf>,
    env: Env,
}

impl Default for SetupAccess {
    fn default() -> Self {
        Self { path: None, env: Env { disabled: true, ..Env::default() } }
    }
}

impl SetupAccess {
    pub fn new(env: &Env) -> Self {
        Self { path: setup_path(env), env: env.clone() }
    }

    /// Yazım izni; `Err`: ADR-0001 §10.3 adı (`skipped(test_instance)` …).
    pub fn writable(&self) -> Result<(), String> {
        self.env.guard(TargetKind::File).map_err(|s| apply::error_text(&s.into()))?;
        self.path.as_ref().map(|_| ()).ok_or_else(|| "skipped(no_session)".to_owned())
    }

    /// Test/ikincil örnek mi?
    pub fn test_instance(&self) -> bool {
        self.env.test_instance
    }

    /// `setup done|skip`: kaydı yazar (kapı [`record`]'da).
    pub fn record(&self, state: SetupState, version: u16, now: &str) -> Result<WriteOutcome, String> {
        self.writable()?;
        let path = self.path.as_deref().ok_or("skipped(no_session)")?;
        record(&self.env, path, state, version, now)
    }

    /// Şimdiki dosya durumu (yol yoksa "yok").
    pub fn read(&self) -> SetupRead {
        self.path.as_deref().map_or(SetupRead::Missing, read)
    }
}

/// `msg setup status` yanıtı (ADR-0001 §10.2 + `setup_state`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupStatus {
    pub first_run: bool,
    pub setup_version: Option<u16>,
    pub setup_state: Option<&'static str>,
    pub hypr: HyprBrief,
    pub migration: MigrationBrief,
}

/// `hypr` alanı: `msg hypr status`'un özeti.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HyprBrief {
    /// Hyprland oturumu (geçerli `HYPRLAND_INSTANCE_SIGNATURE`).
    pub session: bool,
    /// `SyncStatus::source` olduğu gibi (§10.2 kümesi + `pending|inactive|unknown|error|undone`).
    pub source: &'static str,
    /// `exec_once|external|systemd`; eşitleme turu henüz çalışmadıysa `null`.
    pub launch: Option<&'static str>,
    /// Ana ayarın durumu (ADR-0007): `managed` (Nuxut'un), `available` (devralınabilir), `missing`
    /// (ana dosya yok), `unknown` (okunamadı). Hoş geldin devralma adımını yalnız `available`'da gösterir.
    pub main: &'static str,
}

/// `migration` alanı.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationBrief {
    /// Geri alınabilir son shell.toml göçünün kimliği (opak); yoksa `null`.
    pub pending_notice: Option<String>,
}

impl SetupStatus {
    pub fn new(read: &SetupRead, hypr: HyprBrief, migration: MigrationBrief) -> Self {
        let record = read.record();
        Self {
            first_run: read.first_run(),
            setup_version: record.map(|r| r.version),
            setup_state: record.map(|r| r.state.name()),
            hypr,
            migration,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"first_run\":false}".into())
    }
}

/// Geçerli bir Hyprland imzası var mı (oturum)?
pub fn hypr_session(signature: Option<&OsStr>) -> bool {
    signature.and_then(OsStr::to_str).is_some_and(|s| {
        !s.is_empty() && s.len() <= 255 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    })
}

/// Açılışta Hoş geldin başlatılsın mı (saf)? Dosya yok, ayar açık, varsayılan (yazabilen) örnek
/// ve Hyprland oturumu.
pub fn should_offer_welcome(read: &SetupRead, enabled: bool, access: &SetupAccess, session: bool) -> bool {
    enabled && session && !access.test_instance() && access.writable().is_ok() && read.first_run()
}

/// `nuxut-settings`: önce shell'in kendi dizini (kurulum birlikte), sonra `PATH`.
pub fn find_settings(exe_dir: Option<&Path>, path: &[PathBuf]) -> Option<PathBuf> {
    use crate::services::launcher::catalog::find_executable;
    exe_dir
        .and_then(|d| find_executable(&d.join(SETTINGS_BIN).to_string_lossy(), &[]))
        .or_else(|| find_executable(SETTINGS_BIN, path))
}

/// Oturum başına tek başlatma işareti: `$XDG_RUNTIME_DIR/nuxut/welcome-<imza>`. Shell oturum
/// içinde yeniden başlarsa (çökme, `Restart=on-failure`) pencere ikinci kez açılmaz; yeni
/// oturumda (runtime dizini temiz, imza farklı) dosya hâlâ yoksa yeniden sunulur. `true`:
/// işaret bu çağrıda oluşturuldu (başlat). İşaret yazılamazsa yine başlatılır (en kötü
/// ihtimalle yeniden başlatmada bir kez daha açılır).
pub fn claim_session_marker(runtime_dir: Option<&Path>, signature: &str) -> bool {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let Some(dir) = runtime_dir.filter(|d| d.is_absolute()).map(|d| d.join("nuxut")) else { return true };
    if std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).is_err() {
        return true;
    }
    match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(dir.join(format!("welcome-{signature}"))) {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(_) => true,
    }
}

/// `nuxut-settings --welcome`'ı ayrık başlatır; beklemez. Ayrıntı: [`spawn_settings`].
pub fn spawn_welcome(settings: PathBuf, setsid: Option<PathBuf>, home: Option<PathBuf>) -> std::io::Result<()> {
    spawn_settings(settings, WELCOME_ARG.into(), setsid, home)
}

/// `nuxut-settings <arg>`'ı ayrık başlatır; beklemez. `setsid -f` varsa onunla (Ayarlar
/// shell'in oturumundan/süreç grubundan kopar, shell yeniden başlasa da yaşar), yoksa ayrı süreç
/// grubunda ve toplayıcı thread'le. Shell'in `NUXUT_*` değişkenleri (örnek adı vb.) geçmez.
/// Ara zincirin (`setsid -f`, ms'ler) beklenmesi de ana thread'i tutmasın diye bir thread'de.
pub fn spawn_settings(settings: PathBuf, arg: String, setsid: Option<PathBuf>, home: Option<PathBuf>) -> std::io::Result<()> {
    std::thread::Builder::new().name("nuxut-settings-spawn".into()).spawn(move || {
        use std::os::unix::process::CommandExt;
        let mut cmd = match &setsid {
            Some(s) => {
                let mut c = std::process::Command::new(s);
                c.arg("-f").arg(&settings);
                c
            }
            None => std::process::Command::new(&settings),
        };
        cmd.arg(&arg)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        if let Some(h) = home.filter(|h| h.is_dir()) {
            cmd.current_dir(h);
        }
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("NUXUT_") {
                cmd.env_remove(key);
            }
        }
        match cmd.spawn() {
            Ok(mut child) => match child.wait() {
                Ok(s) if !s.success() => tracing::warn!(status = ?s.code(), "Ayarlar başlatıcısı hatayla çıktı ({SETTINGS_BIN} {arg})"),
                _ => tracing::info!("Ayarlar başlatıldı ({SETTINGS_BIN} {arg})"),
            },
            Err(e) => tracing::warn!("Ayarlar başlatılamadı ({SETTINGS_BIN} {arg}): {:?}", e.kind()),
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(s: &str) -> Option<&OsStr> {
        Some(OsStr::new(s))
    }

    #[test]
    fn path_is_under_env_state_home() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::for_test(t.path());
        assert_eq!(setup_path(&env), Some(t.path().join("state/nuxut/setup.toml")));
        assert_eq!(setup_path(&Env { state_home: None, ..env.clone() }), None);
        assert_eq!(setup_path(&Env { state_home: Some("rel".into()), ..env }), None);
    }

    #[test]
    fn parse_and_render_roundtrip() {
        let r = SetupRecord { state: SetupState::Skipped, version: 3, at: Some("2026-10-11T09:30:00Z".into()) };
        let text = render(&r);
        assert_eq!(parse(&text), Ok(r));
        assert!(text.contains("state = \"skipped\"") && text.contains("version = 3"), "{text}");
        // `at` isteğe bağlı; bilinmeyen anahtar yok sayılır (ileri uyum).
        assert_eq!(
            parse("state = \"done\"\nversion = 65535\nfuture = 1\n"),
            Ok(SetupRecord { state: SetupState::Done, version: 65535, at: None })
        );
    }

    #[test]
    fn parse_rejects_bad_values() {
        for bad in [
            "",
            "state = \"done\"",
            "version = 1",
            "state = \"maybe\"\nversion = 1",
            "state = \"done\"\nversion = 0",
            "state = \"done\"\nversion = 65536",
            "state = \"done\"\nversion = -1",
            "state = \"done\"\nversion = \"1\"",
            "state = \"done\"\nversion = 1\nat = \"dün\"",
            "state = \"done\"\nversion = 1\nat = 5",
            "not toml [",
        ] {
            assert!(parse(bad).is_err(), "kabul edilmemeli: {bad:?}");
        }
    }

    #[test]
    fn missing_present_corrupt_and_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nuxut/setup.toml");
        assert_eq!(read(&path), SetupRead::Missing);
        assert!(read(&path).first_run());

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "state = \"done\"\nversion = 2\n").unwrap();
        let r = read(&path);
        assert_eq!(r.record().map(|r| (r.state, r.version)), Some((SetupState::Done, 2)));
        assert!(!r.first_run());

        std::fs::write(&path, "garbage =").unwrap();
        let r = read(&path);
        assert!(matches!(r, SetupRead::Corrupt(_)), "{r:?}");
        assert!(!r.first_run(), "bozuk dosya Hoş geldin'i yeniden tetiklemez");

        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(matches!(read(&path), SetupRead::Corrupt(_)));

        // Sınır: tam MAX bayt geçerli (yorumla doldurulmuş), bir fazlası bozuk.
        let base = "state = \"done\"\nversion = 1\n#";
        let fill = |n: usize| format!("{base}{}", "x".repeat(n - base.len()));
        std::fs::write(&path, fill(MAX_SETUP_BYTES as usize)).unwrap();
        assert!(matches!(read(&path), SetupRead::Present(_)));
        std::fs::write(&path, fill(MAX_SETUP_BYTES as usize + 1)).unwrap();
        assert!(matches!(read(&path), SetupRead::Corrupt(_)));

        // Dizin: normal dosya değil.
        let d = dir.path().join("asdir.toml");
        std::fs::create_dir(&d).unwrap();
        assert!(matches!(read(&d), SetupRead::Corrupt(_)));
    }

    #[test]
    fn record_creates_dir_writes_and_never_downgrades() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::for_test(t.path());
        let path = setup_path(&env).unwrap();
        let at = "2026-10-11T09:30:00Z";
        assert_eq!(record(&env, &path, SetupState::Skipped, 2, at), Ok(WriteOutcome::Written));
        let first = std::fs::read_to_string(&path).unwrap();
        assert_eq!(parse(&first).unwrap(), SetupRecord { state: SetupState::Skipped, version: 2, at: Some(at.into()) });
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);

        // Daha küçük sürüm düşürmez; aynı kayıt yeniden yazılmaz (`at` korunur).
        assert_eq!(record(&env, &path, SetupState::Done, 1, "2027-01-01T00:00:00Z"), Ok(WriteOutcome::Unchanged));
        assert_eq!(record(&env, &path, SetupState::Skipped, 2, "2027-01-01T00:00:00Z"), Ok(WriteOutcome::Unchanged));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);

        // Aynı sürümde atlandı → bitti; daha büyük sürüm yazılır.
        assert_eq!(record(&env, &path, SetupState::Done, 2, at), Ok(WriteOutcome::Written));
        assert_eq!(read(&path).record().map(|r| (r.state, r.version)), Some((SetupState::Done, 2)));
        assert_eq!(record(&env, &path, SetupState::Skipped, 7, at), Ok(WriteOutcome::Written));
        assert_eq!(read(&path).record().map(|r| (r.state, r.version)), Some((SetupState::Skipped, 7)));

        // Bozuk dosya onarılır; geçici dosya kalmaz.
        std::fs::write(&path, "bozuk [").unwrap();
        assert_eq!(record(&env, &path, SetupState::Done, 1, at), Ok(WriteOutcome::Written));
        assert_eq!(read(&path).record().map(|r| r.version), Some(1));
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("setup.toml")]);

        assert!(record(&env, &path, SetupState::Done, 0, at).is_err());
    }

    #[test]
    fn should_write_rules() {
        let present = |state, version| SetupRead::Present(SetupRecord { state, version, at: None });
        assert!(should_write(&SetupRead::Missing, SetupState::Done, 1));
        assert!(should_write(&SetupRead::Corrupt("x".into()), SetupState::Done, 1));
        assert!(!should_write(&present(SetupState::Done, 5), SetupState::Done, 4));
        assert!(!should_write(&present(SetupState::Done, 5), SetupState::Skipped, 4));
        assert!(!should_write(&present(SetupState::Done, 5), SetupState::Done, 5));
        assert!(should_write(&present(SetupState::Skipped, 5), SetupState::Done, 5));
        assert!(should_write(&present(SetupState::Done, 5), SetupState::Skipped, 6));
    }

    #[test]
    fn now_is_rfc3339() {
        assert!(chrono::DateTime::parse_from_rfc3339(&now_rfc3339()).is_ok());
    }

    #[test]
    fn access_rules() {
        let t = tempfile::tempdir().unwrap();
        let test = SetupAccess::new(&Env::for_test(t.path()));
        assert_eq!(test.writable(), Ok(()));
        assert!(test.test_instance());
        assert_eq!(test.path, Some(t.path().join("state/nuxut/setup.toml")));
        assert_eq!(test.record(SetupState::Done, 1, "2026-10-11T09:30:00Z"), Ok(WriteOutcome::Written));

        // Kapalı (kilitsiz) örnek, varsayılan ve kökleri kullanıcının evinde olan test örneği
        // yazmaz; reddedilen yazım dosya açmaz.
        assert_eq!(SetupAccess::default().writable(), Err("skipped(disabled)".into()));
        let off = SetupAccess::new(&Env { disabled: true, ..Env::for_test(t.path()) });
        assert_eq!(off.record(SetupState::Done, 9, "2026-10-11T09:30:00Z"), Err("skipped(disabled)".into()));
        if let Some(home) = nuxut_apply::Env::from_process(Some("t")).real_home {
            let inside = SetupAccess::new(&Env::for_test(&home.join("nuxut-setup-test-yok")));
            assert_eq!(inside.writable(), Err("skipped(test_instance)".into()));
            assert!(!home.join("nuxut-setup-test-yok").exists());
        }
        // Bağlantıyla gerçek eve çözülen kök de reddedilir (framework kapısı).
        if let Some(home) = nuxut_apply::Env::from_process(Some("t")).real_home {
            let link = t.path().join("link");
            std::os::unix::fs::symlink(&home, &link).unwrap();
            let env = Env { state_home: Some(link.join(".local/state")), ..Env::for_test(t.path()) };
            assert!(SetupAccess::new(&env).writable().is_err());
        }
    }

    #[test]
    fn welcome_offer_rules() {
        let t = tempfile::tempdir().unwrap();
        let user = SetupAccess::new(&Env { test_instance: false, ..Env::for_test(t.path()) });
        let test = SetupAccess::new(&Env::for_test(t.path()));
        let done = SetupRead::Present(SetupRecord { state: SetupState::Done, version: 1, at: None });
        assert!(should_offer_welcome(&SetupRead::Missing, true, &user, true));
        assert!(!should_offer_welcome(&SetupRead::Missing, false, &user, true), "ayar kapalı");
        assert!(!should_offer_welcome(&SetupRead::Missing, true, &user, false), "Hyprland oturumu değil");
        assert!(!should_offer_welcome(&SetupRead::Missing, true, &test, true), "test örneği");
        assert!(!should_offer_welcome(&done, true, &user, true));
        assert!(!should_offer_welcome(&SetupRead::Corrupt("x".into()), true, &user, true));
        assert!(!should_offer_welcome(&SetupRead::Missing, true, &SetupAccess::default(), true), "kilitsiz örnek");
    }

    #[test]
    fn status_json_shape() {
        let hypr = HyprBrief { session: true, source: "present", launch: Some("exec_once"), main: "available" };
        let none = MigrationBrief { pending_notice: None };
        let s = SetupStatus::new(&SetupRead::Missing, hypr.clone(), none.clone());
        assert_eq!(
            s.to_json(),
            r#"{"first_run":true,"setup_version":null,"setup_state":null,"hypr":{"session":true,"source":"present","launch":"exec_once","main":"available"},"migration":{"pending_notice":null}}"#
        );
        let done = SetupRead::Present(SetupRecord { state: SetupState::Skipped, version: 4, at: None });
        let v: serde_json::Value =
            serde_json::from_str(&SetupStatus::new(&done, hypr.clone(), MigrationBrief { pending_notice: Some("m".into()) }).to_json())
                .unwrap();
        assert_eq!(v["first_run"], false);
        assert_eq!(v["setup_version"], 4);
        assert_eq!(v["setup_state"], "skipped");
        assert_eq!(v["migration"]["pending_notice"], "m");
        let corrupt: serde_json::Value =
            serde_json::from_str(&SetupStatus::new(&SetupRead::Corrupt("x".into()), hypr, none).to_json()).unwrap();
        assert_eq!(corrupt["first_run"], false);
        assert!(corrupt["setup_state"].is_null() && corrupt["setup_version"].is_null());
    }

    #[test]
    fn session_signature() {
        assert!(hypr_session(os("abc_123-x")));
        assert!(!hypr_session(None));
        assert!(!hypr_session(os("")));
        assert!(!hypr_session(os("a/b")));
    }

    #[test]
    fn finds_settings_next_to_shell_then_path() {
        use std::os::unix::fs::PermissionsExt;
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let mk = |d: &Path| {
            let p = d.join(SETTINGS_BIN);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        assert_eq!(find_settings(Some(a.path()), &[b.path().to_path_buf()]), None);
        let in_path = mk(b.path());
        assert_eq!(find_settings(Some(a.path()), &[b.path().to_path_buf()]), Some(in_path));
        let beside = mk(a.path());
        assert_eq!(find_settings(Some(a.path()), &[b.path().to_path_buf()]), Some(beside));
    }

    #[test]
    fn session_marker_once_per_signature() {
        let rt = tempfile::tempdir().unwrap();
        assert!(claim_session_marker(Some(rt.path()), "sigA"));
        assert!(!claim_session_marker(Some(rt.path()), "sigA"), "aynı oturumda ikinci kez yok");
        assert!(claim_session_marker(Some(rt.path()), "sigB"));
        assert!(claim_session_marker(None, "sigA"), "runtime dizini yoksa başlatılır");
    }
}
