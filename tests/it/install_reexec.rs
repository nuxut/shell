//! `install.sh` kendini güncelleme: kaynak güncellemesi betiği değiştirince aynı argümanlarla bir
//! kez yeniden exec. Senaryo `tests/fixtures/install_reexec.sh`'te (geçici HOME, yerel bare
//! "origin", sahte cargo; kullanıcının dosyalarına ve ağa ulaşılmaz).

use std::process::Command;

use crate::support::repo::repo_path;

#[test]
fn install_reexecs_once_with_original_arguments() {
    let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(tmp).unwrap();
    let out = Command::new("sh")
        .arg(repo_path("tests/fixtures/install_reexec.sh"))
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
