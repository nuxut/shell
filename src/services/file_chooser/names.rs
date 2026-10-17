//! Dosya adı kuralları: kaydetme adının doğrulanması, `SaveFiles` için benzersiz ad, konum
//! alanındaki yolun çözümü ve arama eşleşmesi.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use nuxut_portal_backend::chooser::protocol::{MAX_NAME_BYTES, valid_name};

/// Benzersiz ad ararken en büyük sayı (`ad (9999).uzantı`).
pub const MAX_UNIQUE: u32 = 9999;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    Dot,
    Slash,
    TooLong,
}

/// Kaydetme adını doğrular (baştaki/sondaki boşluklar çağıranda kırpılmaz: dosya adında geçerli).
pub fn check_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        Err(NameError::Empty)
    } else if name == "." || name == ".." {
        Err(NameError::Dot)
    } else if name.contains('/') || name.contains('\0') {
        Err(NameError::Slash)
    } else if name.len() > MAX_NAME_BYTES {
        Err(NameError::TooLong)
    } else {
        Ok(())
    }
}

/// Adı gövde ve uzantıya böler: son noktadan (baştaki nokta gizli dosya işaretidir, uzantı değil).
pub fn split_ext(name: &[u8]) -> (&[u8], &[u8]) {
    match name.iter().rposition(|&b| b == b'.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, &[]),
    }
}

/// Kaydetme alanında başta seçilecek bayt aralığı: uzantıdan önceki gövde.
pub fn stem_range(name: &str) -> std::ops::Range<usize> {
    0..split_ext(name.as_bytes()).0.len()
}

/// `dir` içinde `name` yoksa o; varsa `gövde (n)uzantı`, n = 2..=[`MAX_UNIQUE`] (ad sınırı
/// aşılırsa gövde kısaltılır). `taken`: aynı istekte daha önce verilmiş adlar da sayılır.
pub fn unique_name(dir: &Path, name: &OsStr, taken: &[OsString], exists: &dyn Fn(&Path) -> bool) -> OsString {
    let free = |n: &OsStr| !exists(&dir.join(n)) && !taken.iter().any(|t| t == n);
    if free(name) {
        return name.to_owned();
    }
    let (stem, ext) = split_ext(name.as_bytes());
    for n in 2..=MAX_UNIQUE {
        let suffix = format!(" ({n})");
        let room = MAX_NAME_BYTES.saturating_sub(suffix.len() + ext.len());
        let mut stem = stem[..stem.len().min(room)].to_vec();
        // UTF-8 karakterinin ortasında kesme (sonda yarım karakter: `error_len` yok).
        while std::str::from_utf8(&stem).is_err_and(|e| e.error_len().is_none()) {
            stem.pop();
        }
        let mut candidate = stem;
        candidate.extend_from_slice(suffix.as_bytes());
        candidate.extend_from_slice(ext);
        let candidate = OsString::from_vec(candidate);
        if valid_name(candidate.as_bytes()) && free(&candidate) {
            return candidate;
        }
    }
    name.to_owned()
}

/// `SaveFiles` hedefleri: her kodlu ad çözülür ve [`valid_name`] ile denetlenir (boş, `/`, NUL,
/// `.`/`..`, 255 bayttan uzun reddedilir; UTF-8 olmayan bayt geçerli), çakışmada [`unique_name`].
/// Biri bile geçersizse ya da benzersiz ad bulunamazsa `None` (kısmi yazma yok). Son güvence: her yolun üstü tam olarak
/// `target`.
pub fn save_files_targets(target: &Path, files: &[String], exists: &dyn Fn(&Path) -> bool) -> Option<Vec<PathBuf>> {
    let mut taken: Vec<OsString> = Vec::new();
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let raw = nuxut_portal_backend::chooser::protocol::decode_bytes(f)?;
        if !valid_name(&raw) {
            return None;
        }
        let unique = unique_name(target, OsStr::from_bytes(&raw), &taken, exists);
        // Geçersiz ya da (benzersiz ad tükenince) yine çakışan ad: var olan dosyanın üzerine sessizce
        // yazılmaz, aynı istekte iki URI aynı dosyaya gitmez.
        if !valid_name(unique.as_bytes()) || taken.contains(&unique) || exists(&target.join(&unique)) {
            return None;
        }
        let path = target.join(&unique);
        if path.parent() != Some(target) {
            return None;
        }
        out.push(path);
        taken.push(unique);
    }
    Some(out)
}

/// Konum alanındaki metni yola çevirir: `~` ve `~/…` ev dizinine, göreli yol `base`'e göre;
/// `.`/`..` bileşenleri sözcüksel olarak çözülür (bağ izlenmez).
pub fn resolve_location(input: &str, base: &Path, home: &Path) -> Option<PathBuf> {
    let input = input.trim_end_matches('\n');
    if input.is_empty() || input.contains('\0') {
        return None;
    }
    let raw = if input == "~" {
        home.to_path_buf()
    } else if let Some(rest) = input.strip_prefix("~/") {
        home.join(rest)
    } else if input.starts_with('/') {
        PathBuf::from(input)
    } else {
        base.join(input)
    };
    let mut out = PathBuf::from("/");
    for c in raw.components() {
        match c {
            std::path::Component::Normal(n) => out.push(n),
            std::path::Component::ParentDir => {
                out.pop();
            }
            _ => {}
        }
    }
    Some(out)
}

/// Arama: harf duyarsız alt dize. Boş sorgu her şeyle eşleşir.
pub fn search_matches(query: &str, name: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(&query.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_rules() {
        assert_eq!(check_name("a.txt"), Ok(()));
        assert_eq!(check_name(""), Err(NameError::Empty));
        assert_eq!(check_name(".."), Err(NameError::Dot));
        assert_eq!(check_name("a/b"), Err(NameError::Slash));
        assert_eq!(check_name(&"ç".repeat(128)), Err(NameError::TooLong));
        assert_eq!(stem_range("rapor.son.pdf"), 0..9);
        assert_eq!(stem_range(".bashrc"), 0..7);
        assert_eq!(stem_range("Makefile"), 0..8);
    }

    #[test]
    fn unique_names() {
        let dir = Path::new("/d");
        let existing = ["/d/rapor.pdf", "/d/rapor (2).pdf", "/d/.gizli"];
        let exists = |p: &Path| existing.iter().any(|e| Path::new(e) == p);
        assert_eq!(unique_name(dir, OsStr::new("yeni.pdf"), &[], &exists), "yeni.pdf");
        assert_eq!(unique_name(dir, OsStr::new("rapor.pdf"), &[], &exists), "rapor (3).pdf");
        assert_eq!(unique_name(dir, OsStr::new(".gizli"), &[], &exists), ".gizli (2)");
        assert_eq!(unique_name(dir, OsStr::new("a"), &["a".into()], &exists), "a (2)");
        let long = format!("{}.txt", "ş".repeat(125));
        let exists_all = |p: &Path| p.file_name().is_some_and(|n| n.len() > 200);
        let got = unique_name(dir, OsStr::new(&long), &[], &exists_all);
        assert!(got.len() <= MAX_NAME_BYTES);
        assert!(got.to_str().is_some(), "UTF-8 korunur");
    }

    #[test]
    fn save_files_rejects_hostile_names() {
        use nuxut_portal_backend::chooser::protocol::encode_bytes;
        let dir = Path::new("/home/u/Belgeler");
        let none = |_: &Path| false;
        let enc = |b: &[u8]| encode_bytes(b);
        let ok = save_files_targets(dir, &[enc(b"a.txt"), enc(b"r\xe9.txt")], &none).unwrap();
        assert!(ok.iter().all(|p| p.parent() == Some(dir)));
        let long = vec![b'a'; 256];
        for bad in [&b"../x"[..], b"/abs", b"a/b", b".", b"..", b"", &long] {
            assert!(save_files_targets(dir, &[enc(b"iyi.txt"), enc(bad)], &none).is_none(), "{bad:?}");
        }
        // NUL kodlu ad çözülemez.
        assert!(save_files_targets(dir, &["a%00b".to_owned()], &none).is_none());
        // Ham (kodsuz) yol karakterleri de geçmez.
        assert!(save_files_targets(dir, &["../x".to_owned()], &none).is_none());
        // Yüzde kodlu `/` ve `..` de çözüldükten sonra reddedilir.
        for bad in ["..%2Fx", "%2Fetc%2Fpasswd", "%2E%2E", "a%2Fb"] {
            assert!(save_files_targets(dir, &[bad.to_owned()], &none).is_none(), "{bad}");
        }
        // Aynı ad iki kez: ikincisi benzersizleşir, hiçbiri klasör dışına çıkmaz.
        let two = save_files_targets(dir, &[enc(b"a.txt"), enc(b"a.txt")], &none).unwrap();
        assert_eq!(two, vec![dir.join("a.txt"), dir.join("a (2).txt")]);
        // 255 baytlık ad çakışınca gövde kısaltılır, sınır aşılmaz.
        let max = vec![b'b'; 255];
        let taken = |p: &Path| p.file_name().is_some_and(|n| n.as_bytes() == max.as_slice());
        let got = save_files_targets(dir, &[enc(&max)], &taken).unwrap();
        assert!(got[0].file_name().unwrap().len() <= 255 && got[0].parent() == Some(dir));
        // Benzersiz ad bulunamıyorsa (hepsi dolu) istek reddedilir; var olanın üzerine sessizce yazılmaz.
        let all = |_: &Path| true;
        assert!(save_files_targets(dir, &[enc(b"a.txt")], &all).is_none());
    }

    #[test]
    fn locations() {
        let base = Path::new("/home/u/Belgeler");
        let home = Path::new("/home/u");
        assert_eq!(resolve_location("~", base, home).unwrap(), home);
        assert_eq!(resolve_location("~/İndirilenler", base, home).unwrap(), Path::new("/home/u/İndirilenler"));
        assert_eq!(resolve_location("../x/./y", base, home).unwrap(), Path::new("/home/u/x/y"));
        assert_eq!(resolve_location("/../../etc", base, home).unwrap(), Path::new("/etc"));
        assert!(resolve_location("", base, home).is_none());
    }

    #[test]
    fn search() {
        assert!(search_matches("RAP", "rapor.pdf"));
        assert!(search_matches("", "x"));
        assert!(!search_matches("zz", "rapor"));
    }
}
