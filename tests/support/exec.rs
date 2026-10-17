//! Testlerin sahte araç betikleri (birim testleri `crate::test_exec`, entegrasyon testleri
//! `support::exec` üzerinden aynı dosya).
//!
//! Betik bu süreçte yazılıp hemen çalıştırılırsa yükte aralıklı `ETXTBSY` ("Text file busy")
//! alınır: yazma sürerken başka bir test thread'i fork ederse çocuk, exec edene dek yazma fd'sinin
//! kopyasını tutar; `O_CLOEXEC` bu pencereyi kapatmaz (2026-10-17: `lock_launcher` testleri
//! 32 thread × 100 turda 7 kez düştü). Çözüm: çalıştırılacak dosyayı bir alt süreç (`cp`) yazar;
//! bu süreçte o dosyaya hiç yazma fd'si açılmaz.

#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// `path`'e `content` içerikli çalıştırılabilir betik yazar (yarışsız; bkz. modül belgesi).
pub fn write_executable(path: &Path, content: &str) {
    let source = path.with_extension("src");
    std::fs::write(&source, content).unwrap();
    let status = Command::new("cp").arg(&source).arg(path).status().unwrap();
    assert!(status.success(), "cp {} başarısız", path.display());
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
