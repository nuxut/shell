//! Ana Hyprland ayarının devralınması (ADR-0007, kullanıcı kararı 2026-10-16;
//! `docs/design/hyprland-takeover.md`).
//!
//! Kullanıcının `hyprland.conf` / `hyprland.lua`'sı Nuxut'u göstermiyorsa (başında
//! [`MARKER_CONF`]/[`MARKER_LUA`] işareti yoksa) dosya **benzersiz, doğrulanmış** bir yedeğe kopyalanır
//! ve yerine Nuxut'un ana dosyası konur: işaret → temel katman → taşınan `monitor` satırları →
//! Nuxut ayarlar katmanı → en sonda kullanıcının kendi dosyası (`user.conf` / `user.lua`; Nuxut ona
//! hiç yazmaz, yalnızca yoksa boş oluşturur). Yeni dosya yerine konmadan önce Hyprland'in
//! yalnız-doğrulama kipiyle denetlenir ([`Verifier`]); geçmezse eski dosya kalır.
//!
//! Yedekler: `<ad>.nuxut-yedek-YYYY-MM-DD-HHMM[-n]`, `O_EXCL` ile, hiçbirinin üzerine yazılmaz,
//! sınırsız; kopya baytları okunup karşılaştırılır. Geri alma [`restore_backup`]: en yeni yedek
//! geri konur, o anki dosya da önce yedeklenir.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nuxut_apply::TargetKind;
use serde::Serialize;

use super::base::{BaseInput, BaseLayer, generate_base_conf, generate_base_lua};
use super::ensure::{CONF_COMMENT, LUA_COMMENT, display_path};
use super::generate::{GenerateInput, generate_conf};
use super::keybinds::resolve_settings;
use super::lua::{generate_lua, strip_lua_comments};
use super::settings::KeybindsConfig;
use super::parse::{ScanInput, ScanLimits, program_name, scan_chain};
use super::sync::{ConfigKind, SyncEnv};
use crate::services::apply as ext;
use crate::services::clipboard::takeover as clip;

/// Nuxut'un ana dosyasının ilk satırı (devralındığının tek ölçütü; bir `source` satırı yetmez).
pub const MARKER_CONF: &str = "# nuxut: managed main config v1";
pub const MARKER_LUA: &str = "-- nuxut: managed main config v1";
/// Kullanıcının kendi dosyası (ana dosyanın dizininde; en son okunur).
pub const USER_CONF: &str = "user.conf";
pub const USER_LUA: &str = "user.lua";
/// Eski zincirden taşınan donanım/klavye/ortam ayarları (ana dosyanın dizininde; Nuxut ayarlar
/// katmanından ÖNCE okunur: Ayarlar'daki seçim bunların üstüne yazar). Kullanıcı isteği 2026-10-17.
pub const MIGRATED_CONF: &str = "user-migrated.conf";
/// Yedek adının eki: `hyprland.conf.nuxut-yedek-2026-10-16-1430`.
pub const BACKUP_INFIX: &str = ".nuxut-yedek-";
/// Okunan dosya üst sınırı.
const MAX_BYTES: u64 = 1 << 20;

/// Ana dosya Nuxut'unki mi (ilk boş olmayan satır işaret)?
pub fn is_managed_main(text: &str, kind: ConfigKind) -> bool {
    let marker = match kind {
        ConfigKind::Hyprlang => MARKER_CONF,
        ConfigKind::Lua => MARKER_LUA,
    };
    text.lines().map(str::trim).find(|l| !l.is_empty()) == Some(marker)
}

/// Ana ayarın devralma durumu (`msg setup status` `hypr.main`): `managed`, `available`, `missing`,
/// `unknown`. Yalnızca okur (ilk satırlar; sınırlı).
pub fn main_state(env: &SyncEnv) -> &'static str {
    let Some(config) = env.config_path() else { return "unknown" };
    let Some(kind) = ConfigKind::of(&config) else { return "unknown" };
    match read_limited(&config) {
        Ok(None) => "missing",
        Ok(Some(b)) if is_managed_main(&String::from_utf8_lossy(&b), kind) => "managed",
        Ok(Some(_)) => "available",
        Err(_) => "unknown",
    }
}

/// `<ad>.nuxut-yedek-YYYY-MM-DD-HHMM` (n > 1 ise `-n`).
pub fn backup_name(file_name: &str, stamp: &str, n: u32) -> String {
    if n <= 1 { format!("{file_name}{BACKUP_INFIX}{stamp}") } else { format!("{file_name}{BACKUP_INFIX}{stamp}-{n}") }
}

/// Yedek adından sıralama anahtarı (zaman damgası, sıra no); başka ad `None`.
pub fn backup_key(file_name: &str, candidate: &str) -> Option<(String, u32)> {
    let rest = candidate.strip_prefix(file_name)?.strip_prefix(BACKUP_INFIX)?;
    // YYYY-MM-DD-HHMM (15 karakter) [-n]
    let (stamp, tail) = rest.split_at_checked(15)?;
    let digits_ok = stamp.bytes().enumerate().all(|(i, b)| if [4, 7, 10].contains(&i) { b == b'-' } else { b.is_ascii_digit() });
    if !digits_ok {
        return None;
    }
    let n = match tail.strip_prefix('-') {
        None if tail.is_empty() => 1,
        Some(n) if !n.is_empty() && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit()) => n.parse().ok()?,
        _ => return None,
    };
    Some((stamp.to_owned(), n))
}


/// Nuxut'un yarattığı, kullanıcı içeriği taşıyan dosyaların (yedekler, `user-migrated.conf`,
/// `user.conf`, rapor) izni: yalnız sahibi. Ortam satırları belirteç/anahtar içerebilir
/// (güvenlik incelemesi 2026-10-17).
pub const PRIVATE_MODE: u32 = 0o600;

/// Kaynağın izni, en çok [`PRIVATE_MODE`] (grup/herkes okuması hiç verilmez).
pub fn private_mode(source: u32) -> u32 {
    let m = source & PRIVATE_MODE;
    if m & 0o400 == 0 { PRIVATE_MODE } else { m }
}
/// `O_EXCL` ile benzersiz yedek yaratır ([`private_mode`]), baytları yazar, `fsync` eder, geri okuyup karşılaştırır.
/// Hiçbir mevcut dosyanın üzerine yazılmaz. Dönüş: yedeğin yolu.
pub fn create_backup(dir: &Path, file_name: &str, bytes: &[u8], stamp: &str, mode: u32) -> Result<PathBuf, String> {
    // Sınırsız: aynı dakikada `-2`, `-3`… (u32 taşmaz: o kadar dosya yaratılamaz).
    for n in 1..=u32::MAX {
        let path = dir.join(backup_name(file_name, stamp, n));
        let mut f = match OpenOptions::new().write(true).create_new(true).mode(private_mode(mode)).open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("yedek oluşturulamadı: {e}")), // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
        };
        f.write_all(bytes).and_then(|()| f.sync_all()).map_err(|e| format!("yedek yazılamadı: {e}"))?; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
        drop(f);
        let mut back = Vec::new();
        fs::File::open(&path).and_then(|mut f| f.read_to_end(&mut back)).map_err(|e| format!("yedek okunamadı: {e}"))?; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
        if back != bytes {
            // Kendi yarım kopyamız: kalırsa "yedek" sanılırdı.
            let _ = fs::remove_file(&path);
            return Err("yedek içeriği özgün dosyayla aynı değil; devralma yapılmadı".into()); // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
        }
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
        return Ok(path);
    }
    Err("yedek adı bulunamadı".into()) // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
}

/// Doğrulama adayının adı: `.nuxut-aday-<pid>-<ad>` (gizli, pid'li, özgün uzantıyla biter).
pub fn candidate_name(file_name: &str, pid: u32) -> String {
    format!(".nuxut-aday-{pid}-{file_name}")
}

/// Kullanıcının Lua dosyasındaki üst düzey `hl.monitor({ … })` çağrıları (yorumlar atılmış koddan,
/// parantez ve dize duyarlı). En çok 64.
pub fn lua_monitor_calls(text: &str) -> Vec<String> {
    lua_top_calls(text, "hl.monitor")
}

/// Kullanıcının Lua dosyasındaki üst düzey `hl.env(…)` çağrıları (devralmada taşınır; donanım
/// ve oturum değişkenleri). En çok 64.
pub fn lua_env_calls(text: &str) -> Vec<String> {
    lua_top_calls(text, "hl.env")
}

/// `hl.env("AD", …)` çağrısından yalnızca değişken adı (değer hiçbir çıktıya girmez).
pub fn lua_env_name(call: &str) -> Option<String> {
    let rest = call.strip_prefix("hl.env(")?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let name: String = rest[1..].chars().take_while(|c| *c != quote).collect();
    (!name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')).then_some(name)
}

/// Doğrulama hata satırından yalnızca yer (`… at line N`): Hyprland iletisinin geri kalanı
/// satırın değerini (belirteç, anahtar) içerebilir, rapora ve günlüğe girmez.
pub fn redact_verify_line(line: &str) -> String {
    const AT: &str = " at line ";
    match line.find(AT) {
        Some(p) => {
            let digits: String = line[p + AT.len()..].chars().take_while(char::is_ascii_digit).collect();
            let file = line[..p].rsplit('/').next().unwrap_or("");
            format!("Hyprland ayar hatası: {file} satır {digits}") // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
        }
        None => "Hyprland ayar hatası (ayrıntı gizlendi)".to_owned(), // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    }
}

/// Satır başındaki `<ad>(…)` çağrıları (yorumlar atılmış koddan, parantez ve dize duyarlı).
fn lua_top_calls(text: &str, name: &str) -> Vec<String> {
    let (code, _) = strip_lua_comments(text);
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let open = format!("{name}(");
    while let Some(p) = code[i..].find(&open) {
        let start = i + p;
        // Satır başında olmalı (üst düzey; fonksiyon içi çağrılar da satır başında olabilir — kabul).
        let line_start = code[..start].rfind('\n').map_or(0, |n| n + 1);
        let at_line_start = code[line_start..start].trim().is_empty();
        let mut depth = 0i32;
        let mut j = start + name.len();
        let mut quote: Option<u8> = None;
        let mut end = None;
        while j < b.len() {
            let c = b[j];
            match quote {
                Some(q) => {
                    if c == b'\\' {
                        j += 1;
                    } else if c == q {
                        quote = None;
                    }
                }
                None => match c {
                    b'"' | b'\'' => quote = Some(c),
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(j + 1);
                            break;
                        }
                    }
                    _ => {}
                },
            }
            j += 1;
        }
        let Some(end) = end else { break };
        if at_line_start && out.len() < 64 {
            out.push(code[start..end].to_owned());
        }
        i = end;
    }
    out
}

/// Yeni ana dosyanın metni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainParts {
    /// Temel katman satırı (`source = …nuxut-base.conf  # …` / `pcall(require, "nuxut.base")  -- …`).
    pub base_line: String,
    /// Ayarlar katmanı satırları (açıklama + dahil etme).
    pub tail: [String; 2],
    /// Taşınan monitör satırları (hyprlang değerleri ya da Lua çağrıları).
    pub monitors: Vec<String>,
    /// Taşınan ayarlar: hyprlang'da `user-migrated.conf`'un `source` yolu, Lua'da taşınan `hl.env(…)`
    /// çağrıları (satır satır). Nuxut ayarlar katmanından ÖNCE.
    pub migrated: Option<String>,
    /// Yedeğin gösterim yolu (açıklamada).
    pub backup: Option<String>,
    /// Kullanıcı dosyasının gösterim yolu.
    pub user: String,
}

/// Nuxut'un ana `hyprland.conf`'u.
pub fn managed_conf(p: &MainParts) -> String {
    let mut t = format!("{MARKER_CONF}\n");
    t.push_str("# Nuxut — ana Hyprland ayarı; nuxut-shell oluşturdu (ADR-0007). Bu dosyayı elle düzenlemeyin:\n"); // i18n: tanı üretilen dosya yorumu (dosya içeriği)
    if let Some(b) = &p.backup {
        t.push_str(&format!("# önceki dosyanız: {b} (geri almak: nuxut-shell hypr restore-backup).\n")); // i18n: tanı üretilen dosya yorumu (dosya içeriği)
    }
    t.push_str(&format!("# Kendi ayarlarınız: {} — en son okunur, buradaki her şeyi ezer; Nuxut ona yazmaz.\n\n", p.user)); // i18n: tanı üretilen dosya yorumu (dosya içeriği)
    t.push_str(&p.base_line);
    t.push('\n');
    if !p.monitors.is_empty() {
        t.push_str("\n# Önceki ayardan taşınan monitörler\n"); // i18n: tanı üretilen dosya yorumu (dosya içeriği)
        for m in &p.monitors {
            t.push_str(&format!("monitor = {m}\n"));
        }
    }
    if let Some(m) = &p.migrated {
        // i18n: tanı üretilen dosya yorumu (dosya içeriği)
        t.push_str("\n# Önceki ayardan taşınan donanım, klavye ve ortam ayarları (Ayarlar bunların üstüne yazar)\n");
        t.push_str(&format!("source = {m}\n"));
    }
    t.push_str(&format!("\n{}\n{}\n", p.tail[0], p.tail[1]));
    t.push_str(&format!("\nsource = {}\n", p.user));
    t
}

/// Nuxut'un ana `hyprland.lua`'sı.
pub fn managed_lua(p: &MainParts) -> String {
    let mut t = format!("{MARKER_LUA}\n");
    t.push_str("-- Nuxut — ana Hyprland ayarı; nuxut-shell oluşturdu (ADR-0007). Bu dosyayı elle düzenlemeyin:\n"); // i18n: tanı üretilen dosya yorumu (dosya içeriği)
    if let Some(b) = &p.backup {
        t.push_str(&format!("-- önceki dosyanız: {b} (geri almak: nuxut-shell hypr restore-backup).\n")); // i18n: tanı üretilen dosya yorumu (dosya içeriği)
    }
    t.push_str(&format!("-- Kendi ayarlarınız: {} — en son okunur, buradaki her şeyi ezer; Nuxut ona yazmaz.\n\n", p.user)); // i18n: tanı üretilen dosya yorumu (dosya içeriği)
    t.push_str(&p.base_line);
    t.push('\n');
    if !p.monitors.is_empty() {
        t.push_str("\n-- Önceki ayardan taşınan monitörler\n"); // i18n: tanı üretilen dosya yorumu (dosya içeriği)
        for m in &p.monitors {
            t.push_str(m);
            t.push('\n');
        }
    }
    if let Some(m) = &p.migrated {
        // i18n: tanı üretilen dosya yorumu (dosya içeriği)
        t.push_str("\n-- Önceki ayardan taşınan ortam değişkenleri (Ayarlar bunların üstüne yazar)\n");
        t.push_str(m);
        if !m.ends_with('\n') {
            t.push('\n');
        }
    }
    t.push_str(&format!("\n{}\n{}\n", p.tail[0], p.tail[1]));
    // `pcall` değil: kullanıcının kendi dosyasındaki hata görünür kalsın (dosyayı Nuxut yaratır).
    t.push_str("\nrequire(\"user\")\n");
    t
}

/// Yeni dosyanın Hyprland'in yalnız-doğrulama kipiyle denetimi.
pub trait Verifier {
    /// `Ok` = "config ok"; `Err` = hata satırları ya da çalıştırılamadı.
    fn verify(&self, config: &Path) -> Result<(), String>;
}

/// `Hyprland --verify-config -c <dosya>`: compositor, soket, IPC başlatmaz (Hyprland 0.56.2
/// `main.cpp`, `Compositor.cpp`; başvuru §8). Temiz ortamla (`env_clear`), 20 sn sınırlı.
#[derive(Debug, Clone)]
pub struct HyprlandVerifier {
    /// Geçici `XDG_RUNTIME_DIR` (Hyprland günlüğü için; kullanıcınınki verilmez).
    pub runtime_dir: PathBuf,
    /// `~` ve `$XDG_CONFIG_HOME` bunlarla çözülür (devralmanın ortamı; `None` = sürecinki).
    pub home: Option<PathBuf>,
    pub config_home: Option<PathBuf>,
}

/// `--verify-config` çıktısından sonuç (saf; testli).
pub fn parse_verify_output(out: &str) -> Result<(), String> {
    const HEAD: &str = "======== Config parsing result:";
    let Some(p) = out.find(HEAD) else { return Err("Hyprland doğrulama çıktısı tanınmadı".into()) }; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    let body = out[p + HEAD.len()..].trim();
    if body == "config ok" {
        Ok(())
    } else {
        Err(body.lines().filter(|l| !l.trim().is_empty()).take(8).map(redact_verify_line).collect::<Vec<_>>().join("\n"))
    }
}

impl Verifier for HyprlandVerifier {
    fn verify(&self, config: &Path) -> Result<(), String> {
        let _ = fs::create_dir_all(&self.runtime_dir);
        let _ = fs::set_permissions(&self.runtime_dir, fs::Permissions::from_mode(0o700));
        let mut cmd = Command::new("Hyprland");
        cmd.arg("--verify-config").arg("-c").arg(config).env_clear().env("XDG_RUNTIME_DIR", &self.runtime_dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        for k in ["PATH", "LANG"] {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        for (k, own) in [("HOME", &self.home), ("XDG_CONFIG_HOME", &self.config_home)] {
            if let Some(v) = own.as_ref().map(|p| p.as_os_str().to_owned()).or_else(|| std::env::var_os(k)) {
                cmd.env(k, v);
            }
        }
        let mut child = cmd.spawn().map_err(|e| format!("Hyprland çalıştırılamadı (doğrulama yapılamadı): {e}"))?; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Hyprland doğrulaması zaman aşımına uğradı".into()); // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
                }
            }
        }
        let mut out = String::new();
        if let Some(mut s) = child.stdout.take() {
            let _ = s.by_ref().take(256 * 1024).read_to_string(&mut out);
        }
        parse_verify_output(&out)
    }
}

/// `hypr takeover` / `hypr restore-backup` sonucu (JSON).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TakeoverReport {
    /// `taken_over|already_managed|restored|no_backup|inactive|error`
    pub result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_file: Option<String>,
    /// `restore-backup`: geri konan yedek.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored: Option<String>,
    pub monitors: usize,
    /// Taşınan donanım/klavye/ortam ayarlarının dosyası (hyprlang `user-migrated.conf`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migrated_file: Option<String>,
    pub migrated: Migrated,
    pub not_migrated: NotMigrated,
    /// `external_not_migrated`: eski ayardaki harici VM pano köprüsü betiği taşınmadı (Nuxut'un
    /// kendi köprüsü işini yapar, hassas teklifi taşımadan; `docs/design/vm-clipboard.md`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipboard_bridge: Option<&'static str>,
    pub errors: Vec<String>,
}

/// Taşınanların sayısı.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Migrated {
    /// `env`/`envd` (Lua: `hl.env`) satırları.
    pub env: usize,
    /// Taşınan değişkenlerin yalnızca adları (değerler hiçbir çıktıya girmez).
    pub env_names: Vec<String>,
    /// `input`/`cursor`/`xwayland`/`render`/`opengl`/`device` blokları ve yollu atamaları.
    pub settings: usize,
}

/// Taşınmayanların özeti (yedekte kalır; Nuxut'un katmanları karşılar ya da kullanıcı
/// `user.conf`'a kendisi taşır).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NotMigrated {
    /// `exec`/`exec-once` komutlarının yalnızca program adları (argümanlar belirteç içerebilir; en çok 64).
    pub exec: Vec<String>,
    pub binds: usize,
    pub rules: usize,
    pub other: usize,
}

impl TakeoverReport {
    fn error(msg: impl Into<String>) -> Self {
        Self { result: "error", errors: vec![msg.into()], ..Self::default() }
    }

    pub fn ok(&self) -> bool {
        !matches!(self.result, "error" | "inactive")
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{\"result\":\"error\"}".into())
    }
}

/// Son devralmanın raporu (`$XDG_STATE_HOME/nuxut/`): CLI yazar, çalışan kabuk bildirimi bundan
/// kurar (`msg hypr takeover-done`).
pub const REPORT_FILE: &str = "hyprland-main-config.json";
/// Okunan rapor üst sınırı.
const MAX_REPORT_BYTES: u64 = 64 * 1024;

/// Bildirim için rapor özeti (dış girdi: boyut sınırlı, bilinmeyen alanlar yok sayılır).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct ReportSummary {
    pub result: String,
    pub backup: Option<String>,
    pub migrated_file: Option<String>,
    pub migrated: SummaryMigrated,
    pub not_migrated: SummaryNotMigrated,
    pub clipboard_bridge: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct SummaryMigrated {
    pub env: usize,
    pub settings: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct SummaryNotMigrated {
    pub exec: Vec<String>,
    pub binds: usize,
    pub rules: usize,
    pub other: usize,
}

/// Raporu durum dizinine yazar (atomik; yalnız kullanıcı okur).
pub fn save_report(env: &SyncEnv, report: &TakeoverReport) -> Result<PathBuf, String> {
    let dir = env.apply.nuxut_state_dir().ok_or("durum dizini yok")?;
    ext::ensure_private_dir(&env.apply, &dir).map_err(|e| ext::error_text(&e))?;
    let path = dir.join(REPORT_FILE);
    ext::write_own(&env.apply, &path, report.to_json().as_bytes()).map_err(|e| ext::error_text(&e))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(PRIVATE_MODE)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Son raporu okur (yoksa, bozuksa ya da büyükse `None`).
pub fn load_report(env: &SyncEnv) -> Option<ReportSummary> {
    let path = env.apply.nuxut_state_dir()?.join(REPORT_FILE);
    let mut buf = Vec::new();
    fs::File::open(path).ok()?.take(MAX_REPORT_BYTES + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > MAX_REPORT_BYTES {
        return None;
    }
    serde_json::from_slice(&buf).ok()
}

fn read_limited(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::File::open(path) {
        Ok(f) => {
            let mut buf = Vec::new();
            f.take(MAX_BYTES + 1).read_to_end(&mut buf).map_err(|e| format!("okunamadı: {e}"))?; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
            if buf.len() as u64 > MAX_BYTES {
                return Err("ayar dosyası çok büyük".into()); // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
            }
            Ok(Some(buf))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("açılamadı: {e}")), // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    }
}

/// Ortam ve ana dosya yolu (Hyprland oturumu gerekmez; test örneği yalnızca geçici köklerde).
fn main_config(env: &SyncEnv) -> Result<(PathBuf, ConfigKind), Box<TakeoverReport>> {
    match env.apply.guard(TargetKind::File) {
        Ok(()) => {}
        Err(s) => return Err(Box::new(TakeoverReport { result: "inactive", errors: vec![s.name().into()], ..TakeoverReport::default() })),
    }
    let config = env.config_path().ok_or_else(|| Box::new(TakeoverReport::error("Hyprland ayar dosyasının yolu bulunamadı")))?; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    let kind = ConfigKind::of(&config).ok_or_else(|| Box::new(TakeoverReport::error("ayar dosyası .conf ya da .lua değil")))?; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    Ok((config, kind))
}

fn local_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d-%H%M").to_string()
}

/// Varsayılan temel katman: varsayılan kısayollar, kurallar ve varsayılanlar (bulanıklık yok).
fn seed_base(kind: ConfigKind, shell_exe: &str) -> String {
    let (binds, _) = resolve_settings(&KeybindsConfig::default().map);
    let input = BaseInput { shell_exe: shell_exe.to_owned(), binds, layer: Some(BaseLayer { layer_blur: None }) };
    match kind {
        ConfigKind::Hyprlang => generate_base_conf(&input),
        ConfigKind::Lua => generate_base_lua(&input),
    }
}

/// Varsayılan ayarlar katmanı: yalnızca kabuğu oturum başında başlatan kanca (görünüm, hareket,
/// pencere değerleri kabuk açılınca gelir).
fn seed_layer(kind: ConfigKind, shell_exe: &str) -> String {
    let input = GenerateInput {
        launch: true,
        shell_exe: shell_exe.to_owned(),
        look: None,
        motion: None,
        windows: None,
        hide_defaults: false,
        displays: None,
        env: Vec::new(),
        input: None,
        session_path: crate::services::session_path::trusted_bin_dir(),
    };
    match kind {
        ConfigKind::Hyprlang => generate_conf(&input).text,
        ConfigKind::Lua => generate_lua(&input).text,
    }
}

/// Kullanıcının ana ayarını devralır (yoksa oluşturur). `shell_exe`: kurulu kabuk ikilisi (eksik
/// Nuxut katmanları onunla üretilir). `stamp`: yedek zaman damgası (`YYYY-MM-DD-HHMM`; `None` =
/// yerel saat).
pub fn takeover(env: &SyncEnv, verifier: &dyn Verifier, shell_exe: &str, stamp: Option<&str>) -> TakeoverReport {
    let (config, kind) = match main_config(env) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let home = env.apply.home.as_deref();
    let show = |p: &Path| display_path(p, home);
    let Some(dir) = config.parent().map(Path::to_path_buf) else { return TakeoverReport::error("ayar dizini yok") };
    let file_name = config.file_name().and_then(|n| n.to_str()).unwrap_or("hyprland.conf").to_owned();
    let mut report = TakeoverReport { config: Some(show(&config)), ..TakeoverReport::default() };
    let current = match read_limited(&config) {
        Ok(c) => c,
        Err(e) => return TakeoverReport { errors: vec![e], result: "error", ..report },
    };
    let user = dir.join(match kind {
        ConfigKind::Hyprlang => USER_CONF,
        ConfigKind::Lua => USER_LUA,
    });
    report.user_file = Some(show(&user));
    if let Err(e) = ensure_user_file(&user, kind) {
        report.errors.push(e);
    }
    if current.as_deref().is_some_and(|b| is_managed_main(&String::from_utf8_lossy(b), kind)) {
        report.result = "already_managed";
        return report;
    }

    // Nuxut'un dosyaları satırlardan önce var olmalı (hyprlang olmayan `source`'u hata sayar).
    // Yoksa varsayılanlarla üretilir: Hyprland yeni ana dosyayı hemen yeniden yükler ve kabuk hiç
    // çalışmamış olabilir — boş katmanla oturum kısayolsuz kalır, kabuk da sonraki girişte başlamazdı.
    // Kabuk açılınca ikisini kendi ayarıyla yeniden yazar.
    let target = dir.join(kind.generated_name());
    let base = dir.join(kind.base_rel());
    for (path, text) in [(&target, seed_layer(kind, shell_exe)), (&base, seed_base(kind, shell_exe))] {
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = ext::write_derived(&env.apply, path, text.as_bytes()) {
            return TakeoverReport { errors: vec![format!("{} yazılamadı: {}", show(path), ext::error_text(&e))], result: "error", ..report }; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
        }
    }

    // Taşınanlar: monitörler (ana dosyaya), donanım/klavye/ortam ayarları (hyprlang:
    // `user-migrated.conf`, Lua: `hl.env` çağrıları ana dosyaya), taşınmayanların özeti.
    let (monitors, migrated_lines) = match (&current, kind) {
        (Some(_), ConfigKind::Hyprlang) => {
            let r = scan_chain(&ScanInput { root: &config, target: &target, home, watch_keys: &[], skip: std::slice::from_ref(&base), limits: ScanLimits::default() });
            if r.truncated {
                report.errors.push("ayar zinciri tam okunamadı (sınır ya da okunamayan dosya); taşınanlar eksik olabilir".into()); // i18n: tanı CLI/msg uyarı ayrıntısı (çevrilmez)
            }
            let m = r.migration;
            report.migrated = Migrated { env: m.env, env_names: m.env_names.clone(), settings: m.settings };
            if let Some(h) = home {
                let read = |p: &Path| read_limited(p).ok().flatten().filter(|b| b.len() <= clip::MAX_SCRIPT_BYTES).and_then(|b| String::from_utf8(b).ok());
                let bridge = m.exec.iter().any(|cmd| clip::is_clipboard_bridge(cmd) || clip::script_path(cmd, h).and_then(|p| read(&p)).is_some_and(|s| clip::is_clipboard_bridge(&s)));
                report.clipboard_bridge = bridge.then_some("external_not_migrated");
            }
            report.not_migrated = NotMigrated { exec: m.exec.iter().map(|c| program_name(c)).collect(), binds: m.binds, rules: m.rules, other: m.other };
            (r.monitors, (!m.lines.is_empty()).then_some(m.lines))
        }
        (Some(b), ConfigKind::Lua) => {
            let text = String::from_utf8_lossy(b);
            let env = lua_env_calls(&text);
            report.migrated.env = env.len();
            report.migrated.env_names = env.iter().filter_map(|c| lua_env_name(c)).collect();
            (lua_monitor_calls(&text), (!env.is_empty()).then_some(env))
        }
        (None, _) => (Vec::new(), None),
    };
    report.monitors = monitors.len();

    // Yedek (dosya varsa).
    let stamp = stamp.map_or_else(local_stamp, str::to_owned);
    let mode = fs::metadata(&config).map(|m| m.permissions().mode()).unwrap_or(0o644);
    if let Some(bytes) = &current {
        match create_backup(&dir, &file_name, bytes, &stamp, mode) {
            Ok(p) => report.backup = Some(show(&p)),
            Err(e) => return TakeoverReport { errors: vec![e], result: "error", ..report },
        }
    }

    let (Some(tail), Some((_, base_line))) = (kind.include_markers(&show(&target)), kind.base_markers(&show(&base))) else {
        return TakeoverReport { errors: vec!["dahil etme satırı kurulamadı".into()], result: "error", ..report }; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    };
    // Taşınanlar: hyprlang'da ayrı dosya (aday adla doğrulanır), Lua'da ana dosyaya satır satır.
    let migrated_path = dir.join(MIGRATED_CONF);
    let migrated_body = match (kind, &migrated_lines) {
        (ConfigKind::Hyprlang, Some(lines)) => Some(migrated_conf(lines, report.backup.as_deref())),
        _ => None,
    };
    let lua_env = match (kind, migrated_lines) {
        (ConfigKind::Lua, Some(calls)) => Some(calls.join("\n")),
        _ => None,
    };
    let parts_with = |migrated: Option<String>| MainParts {
        base_line: base_line.clone(),
        tail: tail.clone(),
        monitors: monitors.clone(),
        migrated,
        backup: report.backup.clone(),
        user: show(&user),
    };
    let render = |p: &MainParts| match kind {
        ConfigKind::Hyprlang => managed_conf(p),
        ConfigKind::Lua => managed_lua(p),
    };
    let pid = std::process::id();
    let migrated_candidate = dir.join(candidate_name(MIGRATED_CONF, pid));
    let (text, candidate_text) = match (&migrated_body, kind) {
        (Some(_), ConfigKind::Hyprlang) => {
            (render(&parts_with(Some(show(&migrated_path)))), render(&parts_with(Some(migrated_candidate.to_string_lossy().into_owned()))))
        }
        _ => {
            let t = render(&parts_with(lua_env.clone()));
            (t.clone(), t)
        }
    };
    debug_assert!(text.contains(CONF_COMMENT) || text.contains(LUA_COMMENT));

    // Aday aynı dizinde (göreli yollar aynı çözülür), doğrulanır, sonra atomik yerine konur. Ad özgün
    // uzantıyla biter: Hyprland sözdizimini uzantıdan seçer (`.lua` değilse Lua aday hyprlang sanılır).
    let candidate = dir.join(candidate_name(&file_name, pid));
    let write_candidate = |path: &Path, body: &str| {
        let _ = fs::remove_file(path); // yalnızca bizim adımız (pid'li)
        OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).and_then(|mut f| f.write_all(body.as_bytes()).and_then(|()| f.sync_all()))
    };
    let mut wrote = write_candidate(&candidate, &candidate_text);
    if let (Ok(()), Some(body)) = (&wrote, &migrated_body) {
        wrote = write_candidate(&migrated_candidate, body);
    }
    let verified = wrote.map_err(|e| format!("aday dosya yazılamadı: {e}")).and_then(|()| verifier.verify(&candidate)); // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    let _ = fs::remove_file(&candidate);
    let _ = fs::remove_file(&migrated_candidate);
    if let Err(e) = verified {
        return TakeoverReport { errors: vec![format!("yeni ayar Hyprland doğrulamasından geçmedi; eski dosya yerinde: {e}")], result: "error", ..report }; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    }
    let expect = match nuxut_apply::read_bytes(&config) {
        Ok((b, e)) if Some(&b) == current.as_ref() => Some(e),
        Ok(_) => return TakeoverReport { errors: vec!["ayar dosyası bu arada değişti; dokunulmadı".into()], result: "error", ..report }, // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
        Err(nuxut_apply::ApplyError::NotFound(_)) if current.is_none() => Some(nuxut_apply::Expect::Absent),
        Err(e) => return TakeoverReport { errors: vec![ext::error_text(&e)], result: "error", ..report },
    };
    // Taşınanlar ana dosyadan önce (ana dosya onu `source` eder). Önceki bir devralmanın dosyası
    // farklıysa önce benzersiz yedeklenir (kullanıcı düzenlemiş olabilir).
    if let Some(body) = &migrated_body {
        if let Err(e) = write_migrated(&dir, &migrated_path, body, &stamp) {
            return TakeoverReport { errors: vec![e], result: "error", ..report };
        }
        report.migrated_file = Some(show(&migrated_path));
    }
    if let Err(e) = nuxut_apply::write_file(&env.apply, &config, text.as_bytes(), expect.as_ref()) {
        return TakeoverReport { errors: vec![format!("ayar dosyası yazılamadı: {}", ext::error_text(&e))], result: "error", ..report }; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    }
    report.result = "taken_over";
    report
}

/// `user-migrated.conf` metni: açıklama + taşınan parçalar.
pub fn migrated_conf(lines: &[String], backup: Option<&str>) -> String {
    // i18n: tanı üretilen dosya yorumu (dosya içeriği)
    let mut t = String::from("# Önceki Hyprland ayarınızdan taşınan donanım, klavye/fare, imleç ve ortam ayarları (ADR-0007).\n# Nuxut ayarlarından ÖNCE okunur: Ayarlar'da değiştirdiğiniz değer bunun üstüne yazar.\n# Bu dosya sizindir; düzenleyebilirsiniz. Nuxut yalnızca devralma sırasında yazar.\n");
    if let Some(b) = backup {
        t.push_str(&format!("# Kaynak: {b}\n"));
    }
    for l in lines {
        t.push('\n');
        t.push_str(l);
    }
    t
}

/// Taşınanları yazar: dosya varsa ve farklıysa önce benzersiz yedek (`O_EXCL`), sonra atomik
/// yer değiştirme (geçici dosya + `rename`).
fn write_migrated(dir: &Path, path: &Path, body: &str, stamp: &str) -> Result<(), String> {
    match read_limited(path)? {
        Some(old) if old == body.as_bytes() => return Ok(()),
        Some(old) => {
            create_backup(dir, MIGRATED_CONF, &old, stamp, fs::metadata(path).map(|m| m.permissions().mode()).unwrap_or(PRIVATE_MODE))?;
        }
        None => {}
    }
    let tmp = dir.join(candidate_name(&format!("{MIGRATED_CONF}.yeni"), std::process::id()));
    let _ = fs::remove_file(&tmp);
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_MODE)
        .open(&tmp)
        .and_then(|mut f| f.write_all(body.as_bytes()).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("{MIGRATED_CONF} yazılamadı: {e}") // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
        })
}

/// Kullanıcı dosyası yoksa yalnızca açıklamayla oluşturur (`O_EXCL`; varsa dokunulmaz).
fn ensure_user_file(path: &Path, kind: ConfigKind) -> Result<(), String> {
    let text = match kind {
        ConfigKind::Hyprlang => "# Kendi Hyprland ayarlarınız. Bu dosya en son okunur: Nuxut'un her değerini, kuralını ezer;\n# aynı tuşu bağlarsanız Nuxut o kısayolu üretmez. Nuxut bu dosyaya yazmaz.\n", // i18n: tanı üretilen dosya yorumu (dosya içeriği)
        ConfigKind::Lua => "-- Kendi Hyprland ayarlarınız. Bu dosya en son okunur: Nuxut'un her değerini, kuralını ezer;\n-- aynı tuşu bağlarsanız Nuxut o kısayolu üretmez. Nuxut bu dosyaya yazmaz.\n", // i18n: tanı üretilen dosya yorumu (dosya içeriği)
    };
    match OpenOptions::new().write(true).create_new(true).mode(PRIVATE_MODE).open(path) {
        Ok(mut f) => f.write_all(text.as_bytes()).map_err(|e| format!("kullanıcı dosyası yazılamadı: {e}")), // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(format!("kullanıcı dosyası oluşturulamadı: {e}")), // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    }
}

/// En yeni yedeği geri koyar; o anki dosya önce (benzersiz) yedeklenir. Yedek yoksa `no_backup`.
pub fn restore_backup(env: &SyncEnv, stamp: Option<&str>) -> TakeoverReport {
    let (config, _) = match main_config(env) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let home = env.apply.home.as_deref();
    let show = |p: &Path| display_path(p, home);
    let Some(dir) = config.parent().map(Path::to_path_buf) else { return TakeoverReport::error("ayar dizini yok") };
    let file_name = config.file_name().and_then(|n| n.to_str()).unwrap_or("hyprland.conf").to_owned();
    let mut report = TakeoverReport { config: Some(show(&config)), ..TakeoverReport::default() };
    let newest = fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| e.file_name().to_str().and_then(|n| backup_key(&file_name, n).map(|k| (k, e.path()))))
                .max_by(|a, b| a.0.cmp(&b.0))
        })
        .ok()
        .flatten();
    let Some((_, backup)) = newest else {
        report.result = "no_backup";
        return report;
    };
    let bytes = match read_limited(&backup) {
        Ok(Some(b)) => b,
        Ok(None) => return TakeoverReport { errors: vec!["yedek kayboldu".into()], result: "error", ..report },
        Err(e) => return TakeoverReport { errors: vec![e], result: "error", ..report },
    };
    let current = match nuxut_apply::read_bytes(&config) {
        Ok((b, e)) => Some((b, e)),
        Err(nuxut_apply::ApplyError::NotFound(_)) => None,
        Err(e) => return TakeoverReport { errors: vec![ext::error_text(&e)], result: "error", ..report },
    };
    let stamp = stamp.map_or_else(local_stamp, str::to_owned);
    if let Some((cur, _)) = &current {
        let mode = fs::metadata(&config).map(|m| m.permissions().mode()).unwrap_or(0o644);
        match create_backup(&dir, &file_name, cur, &stamp, mode) {
            // Yeni yedek (o anki dosya) geri konanın yerine geçmez: geri konan, ondan önceki en yenidir.
            Ok(p) => report.backup = Some(show(&p)),
            Err(e) => return TakeoverReport { errors: vec![e], result: "error", ..report },
        }
    }
    let expect = current.map_or(nuxut_apply::Expect::Absent, |(_, e)| e);
    if let Err(e) = nuxut_apply::write_file(&env.apply, &config, &bytes, Some(&expect)) {
        return TakeoverReport { errors: vec![format!("geri konamadı: {}", ext::error_text(&e))], result: "error", ..report }; // i18n: tanı CLI/msg hata ayrıntısı (çevrilmez)
    }
    report.result = "restored";
    report.restored = Some(show(&backup));
    report
}

#[cfg(test)]
mod tests;
