//! `dist/takeover/conflicting-services.sh`: çakışan bar/bildirim servislerinin devralınması ve
//! geri getirilmesi (docs/design/service-takeover.md). Senaryo `tests/fixtures/service_takeover.sh`'te
//! (geçici HOME/XDG, sahte systemctl/pgrep/pkill; kullanıcının birimlerine ve süreçlerine ulaşılmaz).

use std::process::Command;

use crate::support::repo::repo_path;

#[test]
fn takeover_records_disables_and_restores() {
    let out = Command::new("sh")
        .arg(repo_path("tests/fixtures/service_takeover.sh"))
        .arg(repo_path("dist/takeover/conflicting-services.sh"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("HEPSİ TAMAM"),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
