//! Beyaz liste: yardımcının yazabileceği, silebileceği ya da yedekleyebileceği yollar.
//!
//! Her yazma/silme/yeniden adlandırma bu denetimden geçer ([`crate::ops`]); listede olmayan yol
//! = hata, işlem yapılmaz. Yollar köke göredir (`etc/default/grub.d/90-nuxut-theme.cfg`).
//! Yedek ve geçici adlar yalnız izinli bir yolun kardeşi olarak, sabit kalıpla kurulabilir.

use nuxut_boot_theme::detect::{ESP_ROOTS, GRUB_DIRS, PLYMOUTH_CONF};
use nuxut_boot_theme::{grub, limine};

/// Bildirim dosyası (yardımcının durum kaydı).
pub const STATE_DIR: &str = "var/lib/nuxut/system-theme";
pub const MANIFEST: &str = "manifest.json";
/// Kilit dosyası (`/run` kökün; tmpfs).
pub const LOCK_DIR: &str = "run";
pub const LOCK_NAME: &str = "nuxut-system-helper.lock";
pub const PLYMOUTH_THEME: &str = "usr/share/plymouth/themes/nuxut";

/// Düzenli (sabit) izinli yollar.
fn fixed() -> Vec<String> {
    let mut v = vec![
        PLYMOUTH_THEME.to_owned(),
        PLYMOUTH_CONF.to_owned(),
        format!("{}/{}", grub::DROPIN_DIR.trim_start_matches('/'), grub::DROPIN_NAME),
        format!("{STATE_DIR}/{MANIFEST}"),
        // Giriş ekranı alanı (crate::greeter).
        crate::greeter::GREETD_CONFIG.to_owned(),
        format!("{}/{}", crate::greeter::STATE_DIR, crate::greeter::RECORD),
        crate::greeter::PUBLISHED.to_owned(),
    ];
    for d in GRUB_DIRS {
        v.push(format!("{d}/{}", grub::THEME_SUBDIR));
        v.push(format!("{d}/grub.cfg"));
        v.push(format!("{d}/grub.cfg.nuxut-new"));
        // grub-mkconfig önce `<çıktı>.new` yazar; yarıda kalırsa o da temizlenir.
        v.push(format!("{d}/grub.cfg.nuxut-new.new"));
    }
    v
}

/// Limine yapılandırma yolu mu (bilinen aday kalıpları).
fn limine_conf(path: &str) -> bool {
    let mut roots: Vec<&str> = vec![""];
    roots.extend(ESP_ROOTS);
    roots.iter().any(|root| {
        let base = if root.is_empty() { String::new() } else { format!("{root}/") };
        let Some(rest) = path.strip_prefix(&base) else { return false };
        if limine::VOLUME_CANDIDATES.contains(&rest) {
            return true;
        }
        // EFI/<alt>/limine.conf (tek düzey, alt dizin adı düz).
        rest.strip_prefix("EFI/")
            .and_then(|r| r.strip_suffix("/limine.conf"))
            .is_some_and(|sub| !sub.is_empty() && !sub.contains('/') && sub != "." && sub != "..")
    })
}

/// initramfs/UKI çıktısı mı: ESP ya da `/boot` altında, `.img`/`.efi` uzantılı, `..` yok.
pub fn initramfs_image(path: &str) -> bool {
    let under = ["boot/", "efi/"].iter().any(|p| path.starts_with(p));
    let ext = path.ends_with(".img") || path.to_ascii_lowercase().ends_with(".efi");
    under && ext && !path.split('/').any(|c| c.is_empty() || c == "." || c == "..")
}

/// Yedek adı kalıbı: `<ad>.nuxut-<14 rakam>[-<n>].bak`.
pub fn backup_of(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".bak")?;
    let (orig, tag) = stem.rsplit_once(".nuxut-")?;
    let digits = tag.split('-').next()?;
    (digits.len() == 14 && digits.bytes().all(|b| b.is_ascii_digit()) && !orig.is_empty()).then_some(orig)
}

/// Geçici kardeş adı kalıbı (tema dizini değişimi): `.<ad>.nuxut-new` / `.<ad>.nuxut-old`.
pub fn temp_of(name: &str) -> Option<&str> {
    let s = name.strip_prefix('.')?;
    s.strip_suffix(".nuxut-new").or_else(|| s.strip_suffix(".nuxut-old"))
}

/// Yol yazılabilir mi (kendisi ya da izinli bir yolun yedeği/geçici kardeşi).
pub fn allowed(path: &str) -> bool {
    if path.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return false;
    }
    let direct = |p: &str| fixed().iter().any(|f| f == p) || limine_conf(p) || initramfs_image(p);
    if direct(path) {
        return true;
    }
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    let sibling = |orig: &str| direct(&if dir.is_empty() { orig.to_owned() } else { format!("{dir}/{orig}") });
    backup_of(name).is_some_and(sibling) || temp_of(name).is_some_and(sibling) || (theme_dir(dir) && nuxut_boot_theme::files::valid_file_name(name))
}

/// Tema dizinlerimiz (ve geçici kardeşleri): içlerine düz dosya yazılabilir.
pub fn theme_dir(dir: &str) -> bool {
    let mut dirs = vec![PLYMOUTH_THEME.to_owned(), crate::greeter::PUBLISHED.to_owned()];
    dirs.extend(GRUB_DIRS.iter().map(|d| format!("{d}/{}", grub::THEME_SUBDIR)));
    let (parent, name) = dir.rsplit_once('/').unwrap_or(("", dir));
    dirs.iter().any(|d| d == dir || temp_of(name).is_some_and(|orig| format!("{parent}/{orig}") == *d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist() {
        for ok in [
            "usr/share/plymouth/themes/nuxut",
            "usr/share/plymouth/themes/.nuxut.nuxut-new",
            "usr/share/plymouth/themes/nuxut.nuxut-20261016120000.bak",
            "etc/plymouth/plymouthd.conf",
            "etc/plymouth/plymouthd.conf.nuxut-20261016120000-2.bak",
            "etc/default/grub.d/90-nuxut-theme.cfg",
            "boot/grub/themes/nuxut",
            "boot/grub/grub.cfg",
            "boot/grub/grub.cfg.nuxut-new",
            "boot/grub2/grub.cfg",
            "boot/limine.conf",
            "boot/EFI/limine/limine.conf",
            "efi/EFI/BOOT/limine.conf",
            "limine.conf",
            "boot/initramfs-linux.img",
            "efi/EFI/Linux/arch-linux.efi",
            "boot/initramfs-linux.img.nuxut-20261016120000.bak",
            "var/lib/nuxut/system-theme/manifest.json",
            "usr/share/plymouth/themes/.nuxut.nuxut-new/watermark.png",
            "boot/grub/themes/nuxut/theme.txt",
        ] {
            assert!(allowed(ok), "{ok}");
        }
        for bad in [
            "etc/passwd",
            "etc/default/grub",
            "etc/default/grub.d/10-user.cfg",
            "boot/grub/themes/starfield",
            "boot/grub/themes/nuxut/../../grub.cfg",
            "boot/EFI/a/b/limine.conf",
            "boot/EFI/../limine.conf",
            "usr/share/plymouth/themes/spinner",
            "etc/shadow.nuxut-20261016120000.bak",
            "etc/plymouth/plymouthd.conf.nuxut-2026.bak",
            "boot/vmlinuz-linux",
            "usr/bin/initramfs.img",
            "/boot/grub/grub.cfg",
            "usr/share/plymouth/themes/spinner/watermark.png",
            "boot/grub/themes/nuxut/sub/x.png",
            "boot/grub/themes/nuxut/X.PNG",
            "",
        ] {
            assert!(!allowed(bad), "{bad}");
        }
    }

    #[test]
    fn backup_pattern() {
        assert_eq!(backup_of("grub.cfg.nuxut-20261016120000.bak"), Some("grub.cfg"));
        assert_eq!(backup_of("grub.cfg.nuxut-20261016120000-3.bak"), Some("grub.cfg"));
        assert_eq!(backup_of("grub.cfg.bak"), None);
        assert_eq!(backup_of(".nuxut-20261016120000.bak"), None);
    }
}
