//! `install.sh` kaynak güveni (güvenlik incelemesi 2026-10-16: untrusted-source-selection,
//! path-trust-bypass, supply-chain): yabancı origin, http/git/göreli uzak adres, göreli `--src`,
//! başkalarının yazabildiği dizin, borudan çalışmada çalışma dizini, sembolik bağ ve `..` ile
//! kaçış, güvenilmeyen önek, seçenek biçimli dal adı, `--locked`, AUR yardımcısı yok (kurulu Nuxut
//! paketi yalnız `pacman -Qi` ile algılanır), sabitleme, git taşımaları.
//! Senaryo `tests/fixtures/install_source_trust.sh`'te (geçici HOME, yerel bare depolar, sahte
//! cargo; ağa ve kullanıcının dosyalarına ulaşılmaz).

use std::process::Command;

use crate::support::repo::repo_path;

#[test]
fn install_refuses_untrusted_sources() {
    let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(tmp).unwrap();
    let out = Command::new("sh")
        .arg(repo_path("tests/fixtures/install_source_trust.sh"))
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
