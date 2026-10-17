//! `nuxut-system-helper`: bkz. `lib.rs`. Çıkış kodu: 0 başarı, 1 hedef hatası, 2 kullanım/ortam.

use std::process::ExitCode;

use nuxut_system_helper::args::{self, Command};
use nuxut_system_helper::boot_theme::Helper;
use nuxut_system_helper::cmd::Bins;
use nuxut_system_helper::events::Events;
use nuxut_system_helper::greeter::{Command as GreeterCommand, Greeter};
use nuxut_system_helper::ops::Ops;
use nuxut_system_helper::safefs::Root;

#[cfg(all(feature = "test-root", not(debug_assertions)))]
compile_error!("test-root yalnız test derlemesinde: sürüm ikilisi kök yerine dizin kabul etmemeli");

/// Üretim: gerçek kök, root olarak çalışmalı.
#[cfg(not(feature = "test-root"))]
fn context() -> Result<(Root, Bins), String> {
    if !rustix::process::geteuid().is_root() {
        return Err("yönetici yetkisi gerekiyor (Nuxut bu aracı yönetici onayıyla çalıştırır)".into());
    }
    Ok((Root::open("/", 0).map_err(|e| e.to_string())?, Bins::system()))
}

/// Test: kök `NUXUT_HELPER_TEST_ROOT`, sahip çağıran, araçlar `NUXUT_HELPER_TEST_BIN`.
#[cfg(feature = "test-root")]
fn context() -> Result<(Root, Bins), String> {
    let root = std::env::var("NUXUT_HELPER_TEST_ROOT").map_err(|_| "NUXUT_HELPER_TEST_ROOT yok")?;
    let bin = std::env::var("NUXUT_HELPER_TEST_BIN").map_err(|_| "NUXUT_HELPER_TEST_BIN yok")?;
    let uid = rustix::process::getuid().as_raw();
    let r = Root::open(&root, uid).map_err(|e| e.to_string())?;
    Ok((r, Bins::at(bin, vec![("NUXUT_HELPER_TEST_ROOT".into(), root)])))
}

/// Görünümü yayımlanacak kullanıcı: pkexec'in koyduğu `PKEXEC_UID` (sudo ile `SUDO_UID`).
#[cfg(not(feature = "test-root"))]
fn caller() -> Option<u32> {
    ["PKEXEC_UID", "SUDO_UID"].iter().find_map(|k| std::env::var(k).ok()).and_then(|v| v.parse().ok()).filter(|&u| u != 0)
}

/// Test: çağıran test kullanıcısı.
#[cfg(feature = "test-root")]
fn caller() -> Option<u32> {
    Some(rustix::process::getuid().as_raw())
}

fn main() -> ExitCode {
    rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o022));
    let argv: Vec<String> = match std::env::args_os().skip(1).map(|a| a.into_string()).collect() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("nuxut-system-helper: argümanlar UTF-8 değil");
            return ExitCode::from(2);
        }
    };
    let command = match args::parse(&argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nuxut-system-helper: {e}");
            return ExitCode::from(2);
        }
    };
    let (root, bins) = match context() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nuxut-system-helper: {e}");
            return ExitCode::from(2);
        }
    };
    if let Command::Greeter(cmd) = command {
        let ops = Ops::new(root);
        let events = Events::stdout();
        let g = Greeter { ops: &ops, bins: &bins, events: &events, caller: caller() };
        let ok = match cmd {
            GreeterCommand::Status => {
                g.status();
                true
            }
            GreeterCommand::Apply(req) => g.apply(&req),
            GreeterCommand::Revert => g.revert(),
        };
        return if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    let helper = Helper { ops: Ops::new(root), bins, events: Events::stdout() };
    let ok = match command {
        Command::Status => {
            helper.status();
            true
        }
        Command::Apply(req) => helper.apply(&req),
        Command::Revert(targets) => helper.revert(&targets),
        Command::Greeter(_) => false,
    };
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
