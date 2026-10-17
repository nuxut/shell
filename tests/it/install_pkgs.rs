//! `install.sh` paket seçimi (2026-10-17): eksikler `pacman -T` ile (provides dahil) bulunur;
//! provides etmeyen -git/-bin varyantı kuruluysa paket atlanır; eksik yoksa `pacman -S` ve
//! `sudo` hiç çağrılmaz. Senaryo `tests/fixtures/install_pkgs.sh`'te (sahte pacman/sudo).

use std::process::Command;

use crate::support::repo::repo_path;

#[test]
fn install_installs_only_unsatisfied_packages() {
    let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(tmp).unwrap();
    let out = Command::new("sh")
        .arg(repo_path("tests/fixtures/install_pkgs.sh"))
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

/// `--dry-run --hyprland-takeover` planı devralmayı gösterir: deneme kolu erken çıktığı için
/// satır deneme bloğunun içinde olmalı (2026-10-17: görünmüyordu).
#[test]
fn dry_run_plan_shows_hyprland_takeover() {
    let script = std::fs::read_to_string(repo_path("install.sh")).unwrap();
    let start = script.find("# --- Deneme").expect("deneme bloğu");
    let end = start + script[start..].find("# --- Kaldırma").expect("kaldırma bloğu");
    let block = &script[start..end];
    assert!(block.contains("HYPRLAND_TAKEOVER\" = 1") && block.contains("[deneme] Hyprland ana ayarı devralınırdı"));
}
