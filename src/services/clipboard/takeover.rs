//! Kullanıcının kendi VM pano köprüsü betiğinin devralınması (saf kısım; tasarım
//! `docs/design/vm-clipboard.md` "Kullanıcı betiğinin devralınması").
//!
//! Nuxut köprüsü çalışırken ayrıca `wl-paste --watch xclip` + `xclip -o` yoklaması yapan bir
//! betik çalışırsa iki köprü birbirini besler. Daha önemlisi bu tür betikler hassas teklifi
//! (`x-kde-passwordManagerHint`, wl-clipboard'un `CLIPBOARD_STATE=sensitive`'i) yok sayar:
//! KeePassXC'den kopyalanan parola X11 üzerinden ana makineye gider. Nuxut köprüsü bunu taşımaz.
//!
//! Burada yalnız karar var (IO çağıranın): Hyprland ayar satırlarından pano köprüsü başlatan
//! `exec-once`/`exec` satırlarını bulur. Satır, çalıştırdığı komut ya da betik (≤ 64 KiB,
//! yorum satırları hariç) hem `wl-paste` hem `xclip`/`xsel` içeriyorsa köprü sayılır.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use nuxut::i18n::t;
use nuxut_apply::{Applier, Change, FragValue, Fragment, Outcome, Position, RestoreMode, RestoreOutcome, SnapshotId, Source};

use crate::services::apply::{self as ext, CLIPBOARD_BRIDGE};
use crate::services::hypr_config::parse::{ScanInput, ScanLimits, scan_chain};
use crate::services::hypr_config::sync::{ConfigKind, SyncEnv};

/// Okunacak betiğin en çok boyutu (bayt).
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;
/// Taranan ayar dosyasının en çok boyutu (bayt).
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
/// Satırların yerine konan blok kimliği (`# >>> nuxut:clipboard-bridge`).
const BLOCK_ID: &str = "clipboard-bridge";
/// "Bir kez yapıldı" işareti (`$XDG_STATE_HOME/nuxut/`): kullanıcı geri aldıysa yeniden yapılmaz.
const MARK: &str = "clipboard-bridge-takeover";

/// Devralma sonucu (log için; içerik taşımaz).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeoverResult {
    /// Bu kadar satır yorum satırına çevrildi.
    Disabled(usize),
    /// Daha önce yapıldı (ya da kullanıcı geri aldı): dokunulmaz.
    AlreadyDone,
    NotFound,
    Skipped(&'static str),
    Failed(String),
}

fn read_limited(path: &Path, max: u64) -> Option<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(max + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= max).then(|| String::from_utf8(buf).ok()).flatten()
}

fn mark_path(sync: &SyncEnv) -> Option<PathBuf> {
    Some(sync.apply.nuxut_state_dir()?.join(MARK))
}

/// Çalışma anı devralması: Hyprland ayar zincirindeki (hyprlang) pano köprüsü satırlarını tek
/// geri alınabilir işlemle (snapshot, `hypr.clipboard_bridge`) yorumlanmış bloğa çevirir. Çağıran
/// bunu yalnızca Nuxut köprüsü gerçekten bağlıyken ve thread'de çağırır. Çalışan sürece dokunulmaz.
pub fn take_over(sync: &SyncEnv) -> TakeoverResult {
    if let Err(r) = sync.guard() {
        return TakeoverResult::Skipped(r);
    }
    let Some(mark) = mark_path(sync) else { return TakeoverResult::Skipped("durum dizini yok") };
    if mark.exists() {
        return TakeoverResult::AlreadyDone;
    }
    let (Some(config), Some(home)) = (sync.config_path(), sync.apply.home.clone()) else {
        return TakeoverResult::Skipped("Hyprland ayar dosyası bulunamadı"); // i18n: tanı log nedeni (çevrilmez)
    };
    let Some(kind @ ConfigKind::Hyprlang) = ConfigKind::of(&config) else {
        return TakeoverResult::Skipped("Lua ayarı: devralma yalnız hyprlang'de"); // i18n: tanı log nedeni (çevrilmez)
    };
    let target = config.with_file_name(kind.generated_name());
    let report = scan_chain(&ScanInput { root: &config, target: &target, skip: &[], home: Some(&home), watch_keys: &[], limits: ScanLimits::default() });
    let mut change = Change::new(ext::setting(CLIPBOARD_BRIDGE), t!("shell-clipboard-bridge-takeover-change"), Source::Shell);
    let mut count = 0;
    for file in &report.files {
        let Some(text) = read_limited(file, MAX_CONFIG_BYTES) else { continue };
        let lines = find_bridge_lines(&text, &home, |p| read_limited(p, MAX_SCRIPT_BYTES as u64));
        if lines.is_empty() {
            continue;
        }
        count += lines.len();
        let block: String = lines.iter().map(|l| disabled_block(l)).collect();
        for line in lines {
            change = change.write(Fragment::Line { file: file.clone(), marker: line, at: Position::End }, FragValue::Absent);
        }
        change = change.write(Fragment::Block { file: file.clone(), id: BLOCK_ID.into() }, FragValue::Text(block));
    }
    if count == 0 {
        return TakeoverResult::NotFound;
    }
    match Applier::open(sync.apply.clone()).with_source(Source::Shell).apply(change) {
        Outcome::Applied { id } => {
            // İşaret devralma snapshot'ının kimliğini taşır (geri alma onu geri yükler).
            if let Err(e) = ext::write_own(&sync.apply, &mark, format!("{}\n", id.as_str()).as_bytes()) {
                tracing::warn!("pano köprüsü devralma işareti yazılamadı: {}", ext::error_text(&e));
            }
            TakeoverResult::Disabled(count)
        }
        Outcome::Noop => TakeoverResult::NotFound,
        other => TakeoverResult::Failed(other.name().to_owned()),
    }
}

/// `msg hypr undo`: devralma snapshot'ını (kimliği işaret dosyasında) geri yükler — satırlar
/// geri gelir, blok kalkar. İşaret kalır: kendiliğinden yeniden devralınmaz. Devralma yoksa ya da
/// zaten geri alındıysa `Ok(false)`.
pub fn undo(sync: &SyncEnv) -> Result<bool, String> {
    let Some(text) = mark_path(sync).and_then(|m| read_limited(&m, 64)) else { return Ok(false) };
    let Ok(id) = SnapshotId::parse(text.trim()) else { return Ok(false) };
    let app = Applier::open(sync.apply.clone()).with_source(Source::Cli);
    match app.show(&id) {
        Ok(Some(d)) if d.restored_by.is_none() => {}
        Ok(_) => return Ok(false),
        Err(e) => return Err(ext::error_text(&e)),
    }
    match app.restore(&id, RestoreMode::Safe) {
        RestoreOutcome::Restored { .. } => Ok(true),
        other => Err(other.name().to_owned()),
    }
}

/// Hyprlang `exec-once = <komut>` / `exec = <komut>` satırının komutu (yorum satırı değil).
pub fn exec_command(line: &str) -> Option<&str> {
    let t = line.trim();
    if t.starts_with('#') {
        return None;
    }
    let (key, value) = t.split_once('=')?;
    matches!(key.trim(), "exec-once" | "exec").then(|| value.trim()).filter(|v| !v.is_empty())
}

/// Komutun çalıştırdığı betik dosyası (`~/x.sh`, `$HOME/x.sh`, mutlak yol; `sh`/`bash` önekli de).
pub fn script_path(command: &str, home: &Path) -> Option<PathBuf> {
    let mut words = command.split_whitespace();
    let mut first = words.next()?;
    if matches!(first, "sh" | "bash" | "/bin/sh" | "/bin/bash" | "/usr/bin/bash" | "/usr/bin/sh") {
        first = words.next()?;
    }
    let first = first.trim_matches(|c| c == '"' || c == '\'');
    if let Some(rest) = first.strip_prefix("~/").or_else(|| first.strip_prefix("$HOME/")) {
        return Some(home.join(rest));
    }
    first.starts_with('/').then(|| PathBuf::from(first))
}

/// Metin (komut ya da betik) bir Wayland ↔ X11 pano köprüsü mü: yorum dışı satırlarda hem
/// `wl-paste` hem `xclip`/`xsel`.
pub fn is_clipboard_bridge(text: &str) -> bool {
    let code = || text.lines().map(str::trim_start).filter(|l| !l.starts_with('#'));
    code().any(|l| l.contains("wl-paste")) && code().any(|l| l.contains("xclip") || l.contains("xsel"))
}

/// Ayar metnindeki pano köprüsü satırları (birebir, sırayla). `read_script`: betik içeriği
/// (yoksa, büyükse ya da okunamazsa `None`).
pub fn find_bridge_lines(config: &str, home: &Path, read_script: impl Fn(&Path) -> Option<String>) -> Vec<String> {
    config
        .lines()
        .filter(|line| {
            let Some(cmd) = exec_command(line) else { return false };
            is_clipboard_bridge(cmd) || script_path(cmd, home).and_then(|p| read_script(&p)).is_some_and(|s| is_clipboard_bridge(&s))
        })
        .map(str::to_owned)
        .collect()
}

/// Devre dışı bırakılan satırın yerine konan blok içeriği (yorumlanmış satır + gerekçe).
pub fn disabled_block(line: &str) -> String {
    let note = "# Nuxut VM pano köprüsü bu satırın işini yapıyor (hassas/parola teklifini taşımadan); iki köprü birbirini besler. Geri almak: nuxut-shell msg hypr undo"; // i18n: tanı hyprland.conf'a yazılan yorum satırı (dosya içeriği)
    format!("{note}\n# {}\n", line.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/u";
    const USER_SCRIPT: &str = "#!/bin/bash\nwl-paste --type text --watch xclip -selection clipboard &\nwhile true; do\n  c=$(xclip -o -selection clipboard)\n  echo -n \"$c\" | wl-copy\n  sleep 0.1\ndone\n";

    fn read(p: &Path) -> Option<String> {
        (p == Path::new("/home/u/hypr-bridge.sh")).then(|| USER_SCRIPT.to_owned())
    }

    #[test]
    fn exec_lines() {
        assert_eq!(exec_command("exec-once = ~/hypr-bridge.sh"), Some("~/hypr-bridge.sh"));
        assert_eq!(exec_command("  exec=foo bar "), Some("foo bar"));
        assert_eq!(exec_command("# exec-once = ~/hypr-bridge.sh"), None);
        assert_eq!(exec_command("bind = SUPER, V, exec, wl-paste"), None);
        assert_eq!(exec_command("exec-once ="), None);
    }

    #[test]
    fn script_paths() {
        let h = Path::new(HOME);
        assert_eq!(script_path("~/hypr-bridge.sh", h), Some(PathBuf::from("/home/u/hypr-bridge.sh")));
        assert_eq!(script_path("bash $HOME/b.sh --x", h), Some(PathBuf::from("/home/u/b.sh")));
        assert_eq!(script_path("/opt/b.sh", h), Some(PathBuf::from("/opt/b.sh")));
        assert_eq!(script_path("spice-vdagent -x", h), None);
    }

    #[test]
    fn finds_user_bridge_and_inline_bridges_only() {
        let conf = "exec-once = xhost +local:\n\
                    exec-once = sleep 1 && DISPLAY=:1 spice-vdagent -x\n\
                    exec-once = ~/hypr-bridge.sh\n\
                    # exec-once = ~/hypr-bridge.sh\n\
                    exec-once = wl-paste --watch xsel -bi\n\
                    exec-once = wl-paste --watch cliphist store\n\
                    bind = SUPER, V, exec, wl-paste | xclip\n";
        let found = find_bridge_lines(conf, Path::new(HOME), read);
        assert_eq!(found, ["exec-once = ~/hypr-bridge.sh", "exec-once = wl-paste --watch xsel -bi"]);
    }

    #[test]
    fn commented_tools_do_not_count() {
        assert!(is_clipboard_bridge(USER_SCRIPT));
        assert!(!is_clipboard_bridge("wl-paste --watch cliphist store\n# xclip eskiden\n"));
        assert!(!is_clipboard_bridge("xclip -o\n"));
    }

    /// Geçici ev dizininde gerçek devralma + `msg hypr undo` karşılığı (snapshot'lı, tek sefer).
    #[test]
    fn take_over_then_undo_restores_and_never_repeats() {
        use std::fs;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let xdg = home.join("xdg-config");
        let hypr = xdg.join("hypr");
        fs::create_dir_all(&hypr).unwrap();
        fs::write(home.join("hypr-bridge.sh"), USER_SCRIPT).unwrap();
        let original = "a = 1\nexec-once = sleep 1 && DISPLAY=:1 spice-vdagent -x\nexec-once = ~/hypr-bridge.sh\nexec-once = foo\n";
        let conf = hypr.join("hyprland.conf");
        fs::write(&conf, original).unwrap();
        let sync = SyncEnv {
            apply: nuxut_apply::Env {
                home: Some(home.clone()),
                config_home: Some(xdg),
                state_home: Some(home.join("state")),
                hyprland: Some("sahte_imza".into()),
                ..nuxut_apply::Env::for_test(tmp.path())
            },
            runtime_dir: Some(tmp.path().join("run")),
            isolated_control: false,
            under_systemd: false,
        };
        assert_eq!(take_over(&sync), TakeoverResult::Disabled(1));
        let after = fs::read_to_string(&conf).unwrap();
        assert!(!after.lines().any(|l| l.trim() == "exec-once = ~/hypr-bridge.sh"), "{after}");
        assert!(after.contains("# exec-once = ~/hypr-bridge.sh"), "{after}");
        assert!(after.contains("spice-vdagent -x") && after.contains("exec-once = foo"), "diğer satırlar kalır");
        assert!(home.join("hypr-bridge.sh").is_file(), "betik silinmez");
        assert_eq!(take_over(&sync), TakeoverResult::AlreadyDone);
        // Geri al: satır geri gelir, blok kalkar; ikinci kez geri alınacak bir şey yok.
        assert_eq!(undo(&sync), Ok(true));
        let restored = fs::read_to_string(&conf).unwrap();
        assert!(restored.lines().any(|l| l.trim() == "exec-once = ~/hypr-bridge.sh"), "{restored}");
        assert!(!restored.contains("nuxut:clipboard-bridge"), "{restored}");
        assert_eq!(undo(&sync), Ok(false));
        // Kullanıcı geri aldıysa kendiliğinden yeniden devralınmaz.
        assert_eq!(take_over(&sync), TakeoverResult::AlreadyDone);
        assert_eq!(fs::read_to_string(&conf).unwrap(), restored);
    }

    #[test]
    fn block_keeps_original_line_commented() {
        let b = disabled_block("exec-once = ~/hypr-bridge.sh");
        assert!(b.ends_with("# exec-once = ~/hypr-bridge.sh\n"));
        assert!(b.contains("msg hypr undo"));
        assert!(b.lines().all(|l| l.starts_with('#')), "blokta çalışan satır yok");
    }
}
