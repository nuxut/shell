//! Görsel doğrulama için tema dosyalarını bir dizine yazar (sisteme dokunmaz; kök gerekmez).
//!
//! `cargo run -p nuxut-boot-theme --example export_theme -- <çıktı> [ölçek] [sans.ttf] [mono.ttf]`
//!
//! `<çıktı>/grub/` (theme.txt, PNG'ler ve fontlar verilmişse `grub-mkfont` ile PF2'ler) ve
//! `<çıktı>/plymouth/`, `<çıktı>/limine/limine.conf` (örnek girdiler + Nuxut bloğu). GRUB teması
//! QEMU'da `grub-mkstandalone` ile gerçek GRUB'a çizdirilir
//! (docs/design/boot-theme.md "Görsel doğrulama").

use std::path::Path;
use std::process::Command;

use nuxut_boot_theme::{ThemeFiles, ThemeOptions, grub, limine, plymouth};
use nuxut_boot_theme::grub::PF2_RANGES;

fn write(dir: &Path, files: &ThemeFiles) {
    std::fs::create_dir_all(dir).expect("çıktı dizini");
    for (name, bytes) in &files.files {
        std::fs::write(dir.join(name), bytes).expect("dosya yazımı");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = Path::new(args.first().expect("kullanım: export_theme <çıktı> [ölçek] [sans.ttf] [mono.ttf]"));
    let mut o = ThemeOptions::sample();
    o.scale = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let grub_dir = out.join("grub");
    write(&grub_dir, &grub::generate(&o).expect("GRUB teması"));
    write(&out.join("plymouth"), &plymouth::generate(&o).expect("Plymouth teması"));
    // Limine: örnek bir yapılandırmaya Nuxut bloğu (yardımcının yazdığıyla aynı düzenleyici).
    let sample = "timeout: 30\n\n/Arch Linux\n    protocol: linux\n    path: boot():/vmlinuz-linux\n\n/Windows\n    protocol: efi\n    path: boot():/EFI/Microsoft/Boot/bootmgfw.efi\n";
    let limine = limine::apply_block(sample, &o.colors).expect("Limine bloğu");
    std::fs::create_dir_all(out.join("limine")).expect("çıktı dizini");
    std::fs::write(out.join("limine/limine.conf"), limine).expect("dosya yazımı");
    if let (Some(sans), Some(mono)) = (args.get(2), args.get(3)) {
        for spec in grub::fonts(o.scale) {
            let src = if spec.mono { mono } else { sans };
            let size = spec.size.to_string();
            let target = grub_dir.join(&spec.file);
            let ok = Command::new("grub-mkfont")
                .args(["-s", &size, "-n", spec.family, "-r", PF2_RANGES, "-o"])
                .arg(&target)
                .arg(src)
                .status()
                .is_ok_and(|s| s.success());
            assert!(ok, "grub-mkfont başarısız: {}", spec.file);
        }
    }
    println!("{}", out.display());
}
