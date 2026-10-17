//! Kilit sürecinin küçük sistem çağrıları (`unsafe` bu crate'te kalsın diye burada).

use std::io::{self, Write};
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

/// `PR_SET_DUMPABLE=0`: çekirdek dökümü yok; aynı uid'den `ptrace` ve `/proc/<pid>/mem`
/// erişimi kapalı (Yama/ptrace_scope'tan bağımsız).
pub fn set_non_dumpable() -> io::Result<()> {
    // SAFETY: prctl(PR_SET_DUMPABLE, 0) yalnızca bu sürecin bir bayrağını değiştirir; ek
    // argümanlar kullanılmaz (0).
    let rc = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Başlatıcıdan devralınmış `fd`'nin sahipliğini alır. Yalnızca açık ve bu sürece ait
/// olmayan (standart akış olmayan) bir tanımlayıcı kabul edilir.
pub fn take_inherited_fd(fd: RawFd) -> io::Result<OwnedFd> {
    if fd <= 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "standart akış hazır fd'si olamaz",
        ));
    }
    // SAFETY: F_GETFD yalnızca tanımlayıcının açık olduğunu sınar, durumu değiştirmez.
    if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd açık (yukarıda sınandı) ve süreçte başka hiçbir yer ona sahip değil: argv'den
    // gelir ve bu fonksiyon başlangıçta bir kez çağrılır.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `"locked\n"` yazar ve kapatır (düşürerek).
pub fn write_ready(fd: OwnedFd) -> io::Result<()> {
    let mut file = std::fs::File::from(fd);
    file.write_all(b"locked\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::IntoRawFd;

    #[test]
    fn ready_fd_roundtrip_and_standard_streams_rejected() {
        assert!(take_inherited_fd(0).is_err());
        assert!(take_inherited_fd(2).is_err());
        let (mut reader, writer) = std::io::pipe().unwrap();
        let raw = OwnedFd::from(writer).into_raw_fd();
        let fd = take_inherited_fd(raw).unwrap();
        write_ready(fd).unwrap();
        let mut s = String::new();
        reader.read_to_string(&mut s).unwrap();
        // EOF geldi: yazan uç kapatıldı.
        assert_eq!(s, "locked\n");
    }
}
