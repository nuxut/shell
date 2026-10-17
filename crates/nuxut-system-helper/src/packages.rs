//! `packages` alanı: Nuxut'un önerdiği resmî depo paketlerini kurar (kullanıcı kararı 2026-10-17:
//! kök yetkisi gereken düzeltme bir düğmeyle yapılır, kullanıcıya `sudo pacman` yazdırılmaz).
//!
//! ```text
//! nuxut-system-helper packages install <ad>…
//! ```
//!
//! - Ad yalnız [`ALLOWED`] listesinden: Nuxut'un görünüm kataloğundaki resmî depo paketleri
//!   (kabuğun `look::catalog` testi her depo paketinin burada olduğunu denetler). Liste dışı ad,
//!   yineleme ya da [`MAX_NAMES`]'ten fazlası hata; hiçbir şey çalışmaz.
//! - `/usr/bin/pacman -S --needed --noconfirm -- <adlar>`: mutlak yol, temiz ortam, süre sınırı,
//!   çıktı satır satır olay ([`crate::cmd`]). AUR paketi kurulmaz (kök olarak derlenmez).
//! - Yedek ve geri alma yok: kurulum pacman'ın kendi işlemidir; kaldırmak paket yöneticisiyle.

use crate::cmd::{self, Bins, Tool};
use crate::events::{Event, Events, Outcome};

/// Kurulabilen paketler (Arch resmî depoları).
pub const ALLOWED: [&str; 9] = [
    "papirus-icon-theme",
    "breeze-icons",
    "vimix-cursors",
    "capitaine-cursors",
    "adw-gtk-theme",
    "xdg-desktop-portal-gtk",
    "plasma-integration",
    "qt6ct",
    "breeze",
];

pub const MAX_NAMES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Install(Vec<String>),
}

pub fn parse<'a>(mut it: impl Iterator<Item = &'a str>) -> Result<Command, String> {
    match it.next() {
        Some("install") => {}
        _ => return Err("kullanım: nuxut-system-helper packages install <ad>…".into()), // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
    }
    let mut names: Vec<String> = Vec::new();
    for n in it {
        if !ALLOWED.contains(&n) {
            return Err(format!("izin verilmeyen paket: {n}"));
        }
        if names.iter().any(|x| x == n) {
            return Err(format!("yinelenen paket: {n}"));
        }
        names.push(n.to_owned());
    }
    if names.is_empty() || names.len() > MAX_NAMES {
        return Err("paket sayısı 1–16 olmalı".into()); // i18n: tanı kök yardımcı girdi/kayıt hatası (CLI, çevrilmez)
    }
    Ok(Command::Install(names))
}

/// [`parse`]'ın tersi (kullanıcı tarafı pkexec argümanlarını bununla kurar).
pub fn install_args(names: &[&str]) -> Vec<String> {
    let mut v = vec!["packages".to_owned(), "install".to_owned()];
    v.extend(names.iter().map(|n| (*n).to_owned()));
    v
}

#[derive(Debug)]
pub struct Packages<'a> {
    pub bins: &'a Bins,
    pub events: &'a Events,
}

impl Packages<'_> {
    /// Kurar; sonuç `packages-result` olayı. `true` = kuruldu (ya da zaten kuruluydu).
    pub fn install(&self, names: &[String]) -> bool {
        let mut args: Vec<&str> = vec!["-S", "--needed", "--noconfirm", "--"];
        args.extend(names.iter().map(String::as_str));
        let res = cmd::run(self.bins, Tool::Pacman, &args, self.events);
        let (outcome, message) = match &res {
            Ok(()) => (Outcome::Applied, None),
            Err(e) => (Outcome::RolledBack, Some(e.to_string())),
        };
        self.events.emit(Event::PackagesResult { outcome, names: names.to_vec(), message });
        res.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_unique_names() {
        assert_eq!(parse(["install", "qt6ct", "breeze"].into_iter()), Ok(Command::Install(vec!["qt6ct".into(), "breeze".into()])));
        for bad in [
            &["install"][..],
            &["install", "qt6ct", "qt6ct"],
            &["install", "linux"],
            &["install", "--overwrite=*"],
            &["install", "-Syu"],
            &["remove", "qt6ct"],
            &[],
        ] {
            assert!(parse(bad.iter().copied()).is_err(), "{bad:?}");
        }
        let args = install_args(&["qt6ct"]);
        assert_eq!(parse(args[1..].iter().map(String::as_str)), Ok(Command::Install(vec!["qt6ct".into()])));
        for n in ALLOWED {
            assert!(n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !n.starts_with('-'), "{n}");
        }
    }
}
