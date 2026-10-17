//! `install.sh` kayıtsız eski kurulum (2026-10-17): install.conf yokken bütün klonlar aynı aynayı
//! gösteriyorsa terminalde sorulur (evet → kayıt; GitHub'a geç → temiz klonlar; hayır → dur),
//! terminal yoksa açık iletiyle durulur, karışık kaynaklar reddedilir. Senaryo
//! `tests/fixtures/install_legacy_source.sh`'te (yerel bare depolar, git insteadOf; ağa çıkılmaz).

use std::process::Command;

use crate::support::repo::repo_path;

#[test]
fn install_resumes_unrecorded_legacy_source() {
    let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(tmp).unwrap();
    let out = Command::new("sh")
        .arg(repo_path("tests/fixtures/install_legacy_source.sh"))
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
