//! Onay bekleyen değişikliğin kaydı ve hyprlang ölü adam anahtarı (ADR-0005 §5.3).
//!
//! `$XDG_RUNTIME_DIR/nuxut/displays-pending-<örnek>.json`: jeton, son an, ayar dili ve **önceki**
//! (kalıcı) yerleşim. Shell çökerse ya da yeniden başlarsa açılışta bu kayıttan geri dönülür.
//! hyprlang'da Hyprland zamanlayıcısı olmadığından `nuxut-shell display-guard <jeton> <son an>
//! <örnek>` Hyprland'in çocuğu olarak başlatılır: süre dolunca kayıt hâlâ aynı jetonla duruyorsa
//! `reload` (diskteki kalıcı yapılandırma = geri dönüş) gönderir ve kaydı siler. Onay sırası:
//! önce kalıcı yazım, sonra kaydın silinmesi.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Kayıt üst sınırı (önceki yerleşim `apply` JSON'u kadar).
pub const MAX_PENDING_BYTES: u64 = 16 * 1024;
/// Bekçinin en uzun bekleyişi (onay süresi en çok 60 sn + pay).
pub const MAX_GUARD_WAIT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingRecord {
    pub v: u32,
    pub token: String,
    /// Unix ms; bu andan sonra geri dönülür.
    pub deadline_unix_ms: u64,
    /// `lua|hyprlang`.
    pub provider: String,
    /// Önceki yerleşim (`displays apply` biçiminde JSON metni).
    pub previous: String,
}

impl PendingRecord {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Option<Self> {
        if text.len() as u64 > MAX_PENDING_BYTES {
            return None;
        }
        serde_json::from_str::<Self>(text).ok().filter(|r| valid_token(&r.token))
    }
}

/// Jeton: 1..=32 onaltılık karakter.
pub fn valid_token(t: &str) -> bool {
    !t.is_empty() && t.len() <= 32 && t.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Örnek (IPC uygulama) adı: `[a-z0-9_-]`, ≤ 64.
pub fn valid_app(a: &str) -> bool {
    !a.is_empty() && a.len() <= 64 && a.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
}

/// Kayıt dosyasının yolu.
pub fn pending_path(runtime_dir: &Path, app: &str) -> Option<PathBuf> {
    (runtime_dir.is_absolute() && valid_app(app)).then(|| runtime_dir.join("nuxut").join(format!("displays-pending-{app}.json")))
}

pub fn read_pending(path: &Path) -> Option<PendingRecord> {
    use std::io::Read;
    let mut buf = String::new();
    std::fs::File::open(path).ok()?.take(MAX_PENDING_BYTES + 1).read_to_string(&mut buf).ok()?;
    PendingRecord::parse(&buf)
}

/// `nuxut-shell display-guard` argümanları.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardArgs {
    pub token: String,
    pub deadline_unix_ms: u64,
    pub app: String,
}

impl GuardArgs {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        match args {
            [token, deadline, app] if valid_token(token) && valid_app(app) => Ok(Self {
                token: token.clone(),
                deadline_unix_ms: deadline.parse().map_err(|_| "geçersiz son an".to_owned())?,
                app: app.clone(),
            }),
            _ => Err("kullanım: display-guard <jeton> <son an (unix ms)> <örnek>".into()),
        }
    }
}

/// Süre dolduğunda yapılacak iş (saf karar): kayıt hâlâ aynı jetonla duruyorsa geri dön.
pub fn should_revert(record: Option<&PendingRecord>, token: &str) -> bool {
    record.is_some_and(|r| r.token == token)
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Bekçiyi çalıştırır (bloklar). Hyprland'e yalnızca `reload` gönderir, yalnızca kayıt
/// hâlâ aynı jetonla duruyorsa.
pub fn run_guard(args: &GuardArgs) -> Result<(), String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).ok_or("XDG_RUNTIME_DIR yok")?;
    let path = pending_path(&runtime, &args.app).ok_or("geçersiz örnek adı")?;
    let wait = Duration::from_millis(args.deadline_unix_ms.saturating_sub(unix_ms())).min(MAX_GUARD_WAIT);
    std::thread::sleep(wait);
    if !should_revert(read_pending(&path).as_ref(), &args.token) {
        return Ok(());
    }
    let paths = crate::services::hyprland::HyprlandPaths::from_env().map_err(|e| e.to_string())?;
    let client = crate::services::hyprland::HyprlandClient::new(paths.socket1);
    client.request("reload").map_err(|e| e.to_string())?;
    // Aradaki onay yarışına karşı: yalnızca hâlâ aynı jetonsa sil.
    if should_revert(read_pending(&path).as_ref(), &args.token) {
        let _ = std::fs::remove_file(&path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_roundtrip_and_limits() {
        let r = PendingRecord { v: 1, token: "ab12".into(), deadline_unix_ms: 5, provider: "lua".into(), previous: "{}".into() };
        assert_eq!(PendingRecord::parse(&r.to_json()), Some(r.clone()));
        assert!(PendingRecord::parse(&r.to_json().replace("ab12", "zz")).is_none());
        assert!(PendingRecord::parse(&" ".repeat(MAX_PENDING_BYTES as usize + 1)).is_none());
        assert!(should_revert(Some(&r), "ab12"));
        assert!(!should_revert(Some(&r), "ab13"));
        assert!(!should_revert(None, "ab12"));
    }

    #[test]
    fn paths_and_args() {
        assert_eq!(
            pending_path(Path::new("/run/user/1000"), "nuxut-shell").unwrap(),
            Path::new("/run/user/1000/nuxut/displays-pending-nuxut-shell.json")
        );
        assert!(pending_path(Path::new("rel"), "nuxut-shell").is_none());
        assert!(pending_path(Path::new("/r"), "../x").is_none());
        let a = |v: &[&str]| GuardArgs::parse(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&["ab", "10", "nuxut-shell"]).unwrap().deadline_unix_ms, 10);
        assert!(a(&["ab", "x", "nuxut-shell"]).is_err());
        assert!(a(&["ab;", "1", "nuxut-shell"]).is_err());
        assert!(a(&["ab", "1"]).is_err());
    }

    #[test]
    fn guard_without_record_sends_nothing() {
        // Kayıt yok → Hyprland'e bağlanılmadan çıkılır (ortamdaki imzaya bakılmaz).
        let tmp = tempfile::tempdir().unwrap();
        let path = pending_path(tmp.path(), "t").unwrap();
        assert!(!should_revert(read_pending(&path).as_ref(), "ab"));
    }
}
