//! `install.sh` PATH denetimi (2026-10-17): kurulum dizini oturumda yoksa çalışan Hyprland
//! oturumuna IPC ile eklenir (Lua: `hyprctl eval`, hyprlang: `hyprctl keyword envd`); Hyprland
//! dışında yeniden giriş iletisi. Senaryo `tests/fixtures/install_path.sh`'te (sahte hyprctl).

use std::process::Command;

use crate::support::repo::repo_path;

#[test]
fn install_adds_bin_to_running_session_path() {
    let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(tmp).unwrap();
    let out = Command::new("sh")
        .arg(repo_path("tests/fixtures/install_path.sh"))
        .arg(repo_path("install.sh"))
        .env("NUXUT_TEST_TMP", tmp)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("HEPSİ TAMAM"),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
