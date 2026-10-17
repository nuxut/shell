//! DDC/CI (VESA MCCS) ile harici monitör parlaklığı: `/dev/i2c-N` üzerinden doğrudan.
//!
//! Neden `ddcutil` değil: CLI her çağrıda tüm bus'ları tarar (yüzlerce ms, alt süreç), C
//! kütüphanesi (libddcutil) ağır ve global durum tutar. Bize gereken iki mesaj (VCP oku/yaz,
//! özellik 0x10 = parlaklık); çerçeveleme burada saf fonksiyonlardır ve test edilir. I2C
//! `ioctl(I2C_SLAVE)` için `i2cdev` (rust-embedded) kullanılır.
//!
//! Zamanlama (MCCS 2.2): yazmadan sonra okumadan önce 40 ms, set sonrası 50 ms beklenir.
//! Monitörler yavaş ve bazen bozuk cevap verir: sağlama toplamı hatasında 3 deneme.

use std::path::Path;
use std::time::Duration;

use i2cdev::core::I2CDevice;
use i2cdev::linux::LinuxI2CDevice;

/// DDC/CI cihaz adresi (7 bit).
pub const DDC_ADDR: u16 = 0x37;
/// Parlaklık VCP kodu.
pub const VCP_BRIGHTNESS: u8 = 0x10;
/// Hedef adres (yazma) 0x6E, kaynak (host) 0x51, cevaptaki sanal host adresi 0x50.
const DEST: u8 = 0x6E;
const HOST: u8 = 0x51;
const HOST_VIRTUAL: u8 = 0x50;
const GET_VCP: u8 = 0x01;
const GET_VCP_REPLY: u8 = 0x02;
const SET_VCP: u8 = 0x03;
const READ_DELAY: Duration = Duration::from_millis(40);
const WRITE_DELAY: Duration = Duration::from_millis(50);
const ATTEMPTS: u32 = 3;

fn checksum(init: u8, bytes: &[u8]) -> u8 {
    bytes.iter().fold(init, |acc, b| acc ^ b)
}

/// "VCP özelliği oku" isteği.
pub fn get_vcp_request(code: u8) -> [u8; 5] {
    let mut m = [HOST, 0x82, GET_VCP, code, 0];
    m[4] = checksum(DEST, &m[..4]);
    m
}

/// "VCP özelliği yaz" isteği.
pub fn set_vcp_request(code: u8, value: u16) -> [u8; 7] {
    let [hi, lo] = value.to_be_bytes();
    let mut m = [HOST, 0x84, SET_VCP, code, hi, lo, 0];
    m[6] = checksum(DEST, &m[..6]);
    m
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DdcError {
    #[error("geçersiz DDC cevabı")]
    BadReply,
    #[error("sağlama toplamı hatalı")]
    Checksum,
    #[error("monitör bu özelliği desteklemiyor")]
    Unsupported,
    #[error("I2C: {0}")]
    Io(String),
}

/// Okunan VCP değeri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcpValue {
    pub current: u16,
    pub max: u16,
}

/// "VCP oku" cevabını çözer. `reply` i2c'den okunan ham bayttır (ilk bayt kaynak adresi 0x6E).
pub fn parse_vcp_reply(reply: &[u8], code: u8) -> Result<VcpValue, DdcError> {
    if reply.len() < 11 || reply[0] != DEST {
        return Err(DdcError::BadReply);
    }
    let len = usize::from(reply[1] & 0x7F);
    if reply[1] & 0x80 == 0 || len != 8 {
        return Err(DdcError::BadReply);
    }
    // Sağlama: sanal host adresi ^ cevabın ilk 10 baytı (adres dahil).
    if checksum(HOST_VIRTUAL, &reply[..10]) != reply[10] {
        return Err(DdcError::Checksum);
    }
    if reply[2] != GET_VCP_REPLY || reply[4] != code {
        return Err(DdcError::BadReply);
    }
    if reply[3] != 0 {
        return Err(DdcError::Unsupported);
    }
    let max = u16::from_be_bytes([reply[6], reply[7]]);
    let current = u16::from_be_bytes([reply[8], reply[9]]);
    if max == 0 {
        return Err(DdcError::BadReply);
    }
    Ok(VcpValue { current: current.min(max), max })
}

/// Açık bir DDC/CI hattı.
#[derive(Debug)]
pub struct DdcBus {
    dev: LinuxI2CDevice,
}

impl DdcBus {
    pub fn open(path: &Path) -> Result<Self, DdcError> {
        LinuxI2CDevice::new(path, DDC_ADDR).map(|dev| Self { dev }).map_err(|e| DdcError::Io(e.to_string()))
    }

    pub fn get_vcp(&mut self, code: u8) -> Result<VcpValue, DdcError> {
        let mut last = DdcError::BadReply;
        for _ in 0..ATTEMPTS {
            self.dev.write(&get_vcp_request(code)).map_err(|e| DdcError::Io(e.to_string()))?;
            std::thread::sleep(READ_DELAY);
            let mut buf = [0u8; 11];
            self.dev.read(&mut buf).map_err(|e| DdcError::Io(e.to_string()))?;
            match parse_vcp_reply(&buf, code) {
                Ok(v) => return Ok(v),
                Err(DdcError::Unsupported) => return Err(DdcError::Unsupported),
                Err(e) => last = e,
            }
            std::thread::sleep(READ_DELAY);
        }
        Err(last)
    }

    pub fn set_vcp(&mut self, code: u8, value: u16) -> Result<(), DdcError> {
        self.dev.write(&set_vcp_request(code, value)).map_err(|e| DdcError::Io(e.to_string()))?;
        std::thread::sleep(WRITE_DELAY);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_match_mccs() {
        // ddcutil'in "getvcp 10" isteği: 51 82 01 10 ac
        assert_eq!(get_vcp_request(0x10), [0x51, 0x82, 0x01, 0x10, 0xAC]);
        let set = set_vcp_request(0x10, 70);
        assert_eq!(&set[..6], &[0x51, 0x84, 0x03, 0x10, 0x00, 70]);
        assert_eq!(set[6], 0x6E ^ 0x51 ^ 0x84 ^ 0x03 ^ 0x10 ^ 70);
    }

    fn reply(current: u16, max: u16, result: u8) -> [u8; 11] {
        let [mh, ml] = max.to_be_bytes();
        let [ch, cl] = current.to_be_bytes();
        let mut r = [0x6E, 0x88, 0x02, result, 0x10, 0x00, mh, ml, ch, cl, 0];
        r[10] = checksum(0x50, &r[..10]);
        r
    }

    #[test]
    fn reply_parses_and_validates() {
        assert_eq!(parse_vcp_reply(&reply(40, 100, 0), 0x10), Ok(VcpValue { current: 40, max: 100 }));
        assert_eq!(parse_vcp_reply(&reply(40, 100, 1), 0x10), Err(DdcError::Unsupported));
        let mut bad = reply(40, 100, 0);
        bad[9] ^= 1;
        assert_eq!(parse_vcp_reply(&bad, 0x10), Err(DdcError::Checksum));
        assert_eq!(parse_vcp_reply(&reply(40, 100, 0), 0x12), Err(DdcError::BadReply));
        assert_eq!(parse_vcp_reply(&[0x6E, 0x80], 0x10), Err(DdcError::BadReply));
        // current > max sınırlanır; max 0 reddedilir.
        assert_eq!(parse_vcp_reply(&reply(500, 100, 0), 0x10).unwrap().current, 100);
        assert_eq!(parse_vcp_reply(&reply(0, 0, 0), 0x10), Err(DdcError::BadReply));
    }
}
