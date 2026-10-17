//! `install.sh` ikili listesi ↔ çalışma alanının `[[bin]]`'leri (2026-10-16: `nuxut-lock` ne
//! derleniyor ne kuruluyordu, Kilitle hiçbir şey açmıyordu). Özellik isteyen (test) ikilileri
//! dışında her ikili kurulur ve paketi `cargo build -p` listesinde olur.

use std::collections::BTreeSet;

use crate::support::repo::{manifest_dir, repo_path};

/// `AD=değer` / `AD="değer"` satırının değeri.
fn var(script: &str, name: &str) -> String {
    let prefix = format!("{name}=");
    let raw = script
        .lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("install.sh: {name} yok"));
    raw.split('#').next().unwrap().trim().trim_matches('"').to_owned()
}

/// (paket adı, kurulacak ikililer) — çalışma alanının her üyesi.
fn workspace_bins() -> Vec<(String, Vec<String>)> {
    let root = manifest_dir();
    let read = |p: &std::path::Path| -> toml::Table {
        toml::from_str(&std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
    };
    let ws = read(&root.join("Cargo.toml"));
    let members = ws["workspace"]["members"].as_array().expect("workspace.members");
    members
        .iter()
        .map(|m| {
            let dir = root.join(m.as_str().unwrap());
            let manifest = read(&dir.join("Cargo.toml"));
            let package = manifest["package"]["name"].as_str().unwrap().to_owned();
            let bins: Vec<String> = match manifest.get("bin").and_then(|b| b.as_array()) {
                Some(bins) => bins
                    .iter()
                    .filter(|b| b.get("required-features").is_none())
                    .map(|b| b["name"].as_str().unwrap().to_owned())
                    .collect(),
                None if dir.join("src/main.rs").is_file() => vec![package.clone()],
                None => Vec::new(),
            };
            (package, bins)
        })
        .collect()
}

#[test]
fn install_builds_and_installs_every_workspace_bin() {
    let script = std::fs::read_to_string(repo_path("install.sh")).unwrap();
    let portal = var(&script, "PORTAL_BIN");
    let mut installed: BTreeSet<String> = var(&script, "BINS").split_whitespace().map(str::to_owned).collect();
    installed.extend([portal.clone(), var(&script, "PICKER_BIN"), var(&script, "CHOOSER_BIN"), var(&script, "SYSHELPER_BIN"), var(&script, "GREETER_BIN")]);
    // Kabuk çalışma alanında derlenen paketler (`cd "$SRC/shell" && $CARGO build ... -p <paket>`).
    let built: BTreeSet<String> = script
        .lines()
        .filter(|l| l.contains("cd \"$SRC/shell\"") && l.contains("$CARGO build"))
        .flat_map(|l| l.split(" -p ").skip(1).map(|t| t.split_whitespace().next().unwrap_or("").to_owned()))
        .map(|p| p.trim_matches(|c| c == '"' || c == ')').replace("$PORTAL_BIN", &portal))
        .collect();
    let bins = workspace_bins();
    assert!(bins.iter().any(|(_, b)| b.iter().any(|n| n == "nuxut-lock")), "çalışma alanı okunamadı: {bins:?}");
    for (package, names) in &bins {
        for name in names {
            assert!(installed.contains(name), "{name} ({package}) install.sh'de kurulmuyor (BINS ya da $LIBEXEC ikilileri)");
            assert!(built.contains(package), "{package} install.sh'de derlenmiyor (cargo build -p); derlenenler: {built:?}");
        }
    }
    // Kurulum sonu doğrulaması listeyi kullanıyor (eksik ikili "Tamam" demesin).
    assert!(script.contains("[ -x \"$BIN/$b\" ] || missing="), "install.sh kurulum sonu ikili doğrulaması yok");
}

/// `nuxut-shell doctor`'un denetlediği ikililer kurulumun kurduklarıyla aynı: yeni ikili
/// install.sh'e eklenip doctor'a eklenmezse (ya da tersi) eksik kurulum raporda görünmez.
#[test]
fn doctor_checks_exactly_what_install_installs() {
    use nuxut_shell::doctor::gather::{BINARIES, HELPERS, REQUIRED_HELPERS};
    let script = std::fs::read_to_string(repo_path("install.sh")).unwrap();
    let set = |s: &str| s.split_whitespace().map(str::to_owned).collect::<BTreeSet<String>>();
    let names = |l: &[&str]| l.iter().map(|s| (*s).to_owned()).collect::<BTreeSet<String>>();
    assert_eq!(names(&BINARIES), set(&var(&script, "BINS")), "doctor BINARIES ↔ install.sh BINS");
    let required = set(&[var(&script, "PORTAL_BIN"), var(&script, "PICKER_BIN"), var(&script, "CHOOSER_BIN")].join(" "));
    assert_eq!(names(&REQUIRED_HELPERS), required, "doctor REQUIRED_HELPERS ↔ install.sh PORTAL/PICKER/CHOOSER_BIN");
    let mut helpers = required;
    helpers.extend(set(&var(&script, "HELPERS")));
    assert_eq!(names(&HELPERS), helpers, "doctor HELPERS ↔ install.sh $LIBEXEC ikilileri");
}
