//! Şifre tamponu: sabit kapasite, `mlock`, düşerken ve temizlenirken sıfırlanır.
//!
//! Kapasite baştan ayrılır ve tampon asla büyümez: `Vec`'in yeniden tahsisi eski içeriği
//! serbest bırakılmış bellekte (sıfırlanmadan) bırakırdı. `Clone` yoktur; `Debug` içeriği ve
//! uzunluğu göstermez.

use zeroize::Zeroize;

/// Şifrenin bayt üst sınırı. Linux-PAM yanıtları `PAM_MAX_RESP_SIZE` (512) ile sınırlar.
pub const MAX_SECRET_BYTES: usize = 512;

/// [`SecretBuf`]'a eklenemeyen girdi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretError {
    /// [`MAX_SECRET_BYTES`] aşılırdı; tampon değişmedi.
    Full,
    /// NUL baytı C dizgisinde şifreyi keserdi; tampon değişmedi.
    Nul,
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Full => "şifre çok uzun",
            Self::Nul => "şifrede NUL baytı olamaz",
        })
    }
}

impl std::error::Error for SecretError {}

/// Sıfırlanan şifre tamponu.
pub struct SecretBuf {
    bytes: Vec<u8>,
    mlocked: bool,
}

impl SecretBuf {
    /// Boş tampon; [`MAX_SECRET_BYTES`] kapasite ayrılır ve (mümkünse) belleğe kilitlenir.
    pub fn new() -> Self {
        let bytes = Vec::with_capacity(MAX_SECRET_BYTES);
        let mlocked = mlock(&bytes);
        Self { bytes, mlocked }
    }

    /// Baytları sona ekler. Sınır aşılırsa ya da NUL varsa hiçbir şey eklenmez.
    pub fn push_bytes(&mut self, more: &[u8]) -> Result<(), SecretError> {
        if more.contains(&0) {
            return Err(SecretError::Nul);
        }
        if self.bytes.len() + more.len() > MAX_SECRET_BYTES {
            return Err(SecretError::Full);
        }
        let capacity = self.bytes.capacity();
        self.bytes.extend_from_slice(more);
        debug_assert_eq!(
            self.bytes.capacity(),
            capacity,
            "SecretBuf yeniden tahsis etmemeli"
        );
        Ok(())
    }

    pub fn push_str(&mut self, s: &str) -> Result<(), SecretError> {
        self.push_bytes(s.as_bytes())
    }

    /// Son UTF-8 karakterini siler; boşaltılan baytlar sıfırlanır.
    pub fn pop_char(&mut self) {
        let Some(start) = self.bytes.iter().rposition(|b| b & 0xC0 != 0x80) else {
            self.clear();
            return;
        };
        self.bytes[start..].zeroize();
        self.bytes.truncate(start);
    }

    /// Tüm kapasiteyi sıfırlar ve boşaltır.
    pub fn clear(&mut self) {
        // `Vec::zeroize` içeriği ve boş kapasiteyi sıfırlar, sonra uzunluğu 0 yapar.
        self.bytes.zeroize();
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Maske noktaları için karakter sayısı (geçersiz UTF-8 baytları da birer sayılır).
    pub fn char_count(&self) -> usize {
        self.bytes.iter().filter(|b| *b & 0xC0 != 0x80).count()
    }

    /// Tampon belleğe kilitlendi mi (swap'e yazılmaz). `RLIMIT_MEMLOCK` yetmezse `false`.
    pub fn is_mlocked(&self) -> bool {
        self.mlocked
    }

    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.bytes.capacity()
    }
}

impl Default for SecretBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SecretBuf {
    fn drop(&mut self) {
        self.bytes.zeroize();
        if self.mlocked {
            munlock(&self.bytes);
        }
    }
}

impl std::fmt::Debug for SecretBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBuf(<gizli>)")
    }
}

fn mlock(bytes: &Vec<u8>) -> bool {
    if bytes.capacity() == 0 {
        return false;
    }
    // SAFETY: işaretçi ve uzunluk Vec'in kendi tahsisini tanımlar; tampon asla büyümediği için
    // tahsis `munlock`'a (Drop) kadar yerinde kalır. mlock belleği okumaz/yazmaz.
    unsafe { libc::mlock(bytes.as_ptr().cast(), bytes.capacity()) == 0 }
}

fn munlock(bytes: &Vec<u8>) {
    // SAFETY: `mlock` ile aynı, hâlâ canlı tahsis.
    unsafe {
        libc::munlock(bytes.as_ptr().cast(), bytes.capacity());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_clear() {
        let mut s = SecretBuf::new();
        s.push_str("aş日").unwrap();
        assert_eq!(s.as_bytes(), "aş日".as_bytes());
        assert_eq!(s.char_count(), 3);
        s.pop_char();
        assert_eq!(s.as_bytes(), "aş".as_bytes());
        s.pop_char();
        s.pop_char();
        assert!(s.is_empty());
        s.pop_char();
        assert!(s.is_empty());
        s.push_str("x").unwrap();
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn never_reallocates_and_rejects_overflow() {
        let mut s = SecretBuf::new();
        let cap = s.capacity();
        assert!(cap >= MAX_SECRET_BYTES);
        let ptr = s.as_bytes().as_ptr();
        for _ in 0..MAX_SECRET_BYTES {
            s.push_str("a").unwrap();
        }
        assert_eq!(s.push_str("a"), Err(SecretError::Full));
        assert_eq!(s.len(), MAX_SECRET_BYTES);
        assert_eq!(s.capacity(), cap);
        assert_eq!(s.as_bytes().as_ptr(), ptr);
    }

    #[test]
    fn rejects_nul_without_change() {
        let mut s = SecretBuf::new();
        s.push_str("ab").unwrap();
        assert_eq!(s.push_bytes(b"c\0d"), Err(SecretError::Nul));
        assert_eq!(s.as_bytes(), b"ab");
    }

    #[test]
    fn pop_zeroes_the_freed_bytes() {
        let mut s = SecretBuf::new();
        s.push_str("ağ").unwrap();
        s.pop_char();
        // Boşaltılan alan sıfır: spare capacity'yi güvenli API ile oku.
        let spare = s.bytes.spare_capacity_mut();
        // SAFETY: (test) pop_char'ın sıfırladığı iki bayt daha önce başlatılmıştı.
        let freed: Vec<u8> = spare[..2]
            .iter()
            .map(|b| unsafe { b.assume_init() })
            .collect();
        assert_eq!(freed, [0, 0]);
    }

    #[test]
    fn clear_zeroes_capacity() {
        let mut s = SecretBuf::new();
        s.push_str("gizli").unwrap();
        s.clear();
        let spare = s.bytes.spare_capacity_mut();
        // SAFETY: (test) ilk 5 bayt yazılmış ve clear ile sıfırlanmıştı.
        let freed: Vec<u8> = spare[..5]
            .iter()
            .map(|b| unsafe { b.assume_init() })
            .collect();
        assert_eq!(freed, [0; 5]);
    }

    #[test]
    fn debug_hides_content_and_length() {
        let mut s = SecretBuf::new();
        s.push_str("cokgizli").unwrap();
        let d = format!("{s:?}");
        assert_eq!(d, "SecretBuf(<gizli>)");
        assert!(!d.contains("cokgizli") && !d.contains('8'));
    }
}
