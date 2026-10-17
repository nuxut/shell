//! `nuxut-shell packages install [--json] [--dry-run] <ad>…`: Nuxut'un önerdiği resmî depo
//! paketlerini kök yardımcının `packages` alanıyla kurar (pkexec, Nuxut polkit penceresi; kullanıcı
//! kararı 2026-10-17: kök gerektiren düzeltme bir düğmeyle, kullanıcıya `sudo pacman` yazdırılmaz).
//! Ayarlar'ın "Kur" düğmesi bunu çalıştırır. Adlar yardımcının sabit listesinden
//! (`nuxut_system_helper::packages::ALLOWED`); yardımcı aynı denetimi kendisi de yapar.

use std::process::ExitCode;

use nuxut_system_helper::packages::{self, Command};

use crate::services::system_theme::{ensure_helper, run_helper};

const USAGE: &str = "kullanım: nuxut-shell packages install [--json] [--dry-run] <ad>…";

pub fn run_cli(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut dry = false;
    let mut words: Vec<&str> = Vec::new();
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            "--dry-run" => dry = true,
            w => words.push(w),
        }
    }
    let names = match packages::parse(words.iter().copied()) {
        Ok(Command::Install(n)) => n,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let argv = packages::install_args(&names.iter().map(String::as_str).collect::<Vec<_>>());
    if dry {
        println!("yönetici onayıyla çalışır: {} {}", nuxut_system_helper::INSTALL_PATH, argv.join(" "));
        return ExitCode::SUCCESS;
    }
    if let Err(code) = ensure_helper() {
        return code;
    }
    run_helper(&argv, json)
}
