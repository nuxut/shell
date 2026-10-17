//! Yakalama: `grim` alt süreci (wlr-screencopy / ext-image-copy-capture compositor'da), çıktısı
//! PPM (P6) olarak bellekte çözülür — PNG kodlama/çözme turu yok, en hızlı yol.
//!
//! Komut satırı sabittir (shell'e verilmez); çıkış adı IPC'de zaten doğrulanmıştır ama burada
//! da `-` ile başlayamaz (seçenek sanılmasın). Okuma boyut sınırlıdır.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use super::geometry::{LogicalRect, RgbaImage};

/// grim'in en uzun süresi: compositor yanıt vermezse oturum sonsuza dek "meşgul" kalmasın.
pub const GRIM_TIMEOUT: Duration = Duration::from_secs(10);

/// grim çıktısının en büyük boyu (16384² RGB ≈ 768 MiB'ın çok altında makul bir tavan:
/// 8K × 2 çıkış bile ~200 MiB).
pub const MAX_PPM_BYTES: u64 = 512 << 20;
/// Kabul edilen en büyük kenar.
pub const MAX_SIDE: u32 = 16384;

/// Ne yakalanacak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Tek çıkış (bağlayıcı adı).
    Output(String),
    /// Mantıksal düzen dikdörtgeni (birden çok çıkışa taşabilir; grim birleştirir).
    Region(LogicalRect),
}

/// Yakalama hatası.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("grim bulunamadı (grim paketi kurulu mu?)")]
    Missing,
    #[error("grim başlatılamadı: {0}")]
    Spawn(std::io::Error),
    #[error("grim başarısız oldu: {0}")]
    Failed(String),
    #[error("geçersiz çıkış adı")]
    BadOutput,
    #[error("görüntü çözülemedi: {0}")]
    Decode(&'static str),
}

/// grim argümanları (program adı hariç). Sonuç stdout'a PPM.
pub fn grim_args(source: &Source) -> Result<Vec<String>, CaptureError> {
    let mut args = vec!["-t".to_owned(), "ppm".to_owned()];
    match source {
        Source::Output(name) => {
            if name.is_empty() || name.starts_with('-') || name.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err(CaptureError::BadOutput);
            }
            args.extend(["-o".to_owned(), name.clone()]);
        }
        Source::Region(r) => {
            if r.is_empty() {
                return Err(CaptureError::Decode("boş bölge"));
            }
            args.extend(["-g".to_owned(), r.to_grim()]);
        }
    }
    args.push("-".to_owned());
    Ok(args)
}

/// grim'i çalıştırır ve çıktıyı çözer (bloklar: arka plan thread'inde).
pub fn capture(source: &Source) -> Result<RgbaImage, CaptureError> {
    let args = grim_args(source)?;
    let mut child = Command::new("grim")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { CaptureError::Missing } else { CaptureError::Spawn(e) })?;
    let mut out = Vec::new();
    let mut err = String::new();
    let (read, timed_out) = with_deadline(&mut child, GRIM_TIMEOUT, |child| {
        let read = child.stdout.take().map(|s| s.take(MAX_PPM_BYTES + 1).read_to_end(&mut out));
        if out.len() as u64 > MAX_PPM_BYTES || matches!(read, Some(Err(_))) {
            // Okuma bıraktı: yazmakta takılı kalmasın.
            let _ = child.kill();
        }
        if let Some(mut e) = child.stderr.take() {
            let _ = (&mut e).take(4096).read_to_string(&mut err);
        }
        read
    });
    let status = child.wait().map_err(CaptureError::Spawn)?;
    if timed_out {
        return Err(CaptureError::Failed(format!("{} sn içinde bitmedi", GRIM_TIMEOUT.as_secs())));
    }
    if let Some(Err(e)) = read {
        return Err(CaptureError::Spawn(e));
    }
    if out.len() as u64 > MAX_PPM_BYTES {
        return Err(CaptureError::Decode("çıktı çok büyük"));
    }
    if !status.success() {
        let line = err.lines().next().unwrap_or("").chars().filter(|c| !c.is_control()).take(200).collect::<String>();
        return Err(CaptureError::Failed(if line.is_empty() { status.to_string() } else { line }));
    }
    parse_ppm(&out)
}

/// `work` (çocuğun borularını tüketir, çocuğu beklemez) `timeout` içinde bitmezse çocuk
/// SIGKILL'le öldürülür. Pid yalnızca bu çocuğundur: `work` bitip bekçi durana dek kimse onu
/// beklemez (reap yok), numara başka sürece geçemez. Dönen ikinci değer: süre aşıldı mı.
pub fn with_deadline<T>(child: &mut Child, timeout: Duration, work: impl FnOnce(&mut Child) -> T) -> (T, bool) {
    let pid = rustix::process::Pid::from_child(child);
    let (done, wait) = mpsc::channel::<()>();
    let watchdog = std::thread::Builder::new().name("nuxut-shot-deadline".into()).spawn(move || {
        let fired = wait.recv_timeout(timeout) == Err(RecvTimeoutError::Timeout);
        if fired {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
        }
        fired
    });
    let out = work(child);
    drop(done);
    let fired = watchdog.ok().and_then(|w| w.join().ok()).unwrap_or(false);
    (out, fired)
}

/// Çocuk çıkana dek bekler ama onu toplamaz (`WNOWAIT`): [`with_deadline`] içinde çıkışı
/// beklemek için; toplama sonra `Child::wait` ile.
pub fn wait_exit_no_reap(child: &Child) {
    let pid = rustix::process::Pid::from_child(child);
    let opts = rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOWAIT;
    while let Err(rustix::io::Errno::INTR) = rustix::process::waitid(rustix::process::WaitId::Pid(pid), opts) {}
}

/// Binary PPM (P6, maxval 255) → opak RGBA.
pub fn parse_ppm(data: &[u8]) -> Result<RgbaImage, CaptureError> {
    let mut pos = 0usize;
    let mut fields = [0u32; 3];
    if data.get(..2) != Some(b"P6") {
        return Err(CaptureError::Decode("P6 değil"));
    }
    pos += 2;
    for field in &mut fields {
        // Boşluk ve `#` yorumları atlanır.
        loop {
            match data.get(pos) {
                Some(b) if b.is_ascii_whitespace() => pos += 1,
                Some(b'#') => {
                    while data.get(pos).is_some_and(|&b| b != b'\n') {
                        pos += 1;
                    }
                }
                _ => break,
            }
        }
        let start = pos;
        while data.get(pos).is_some_and(u8::is_ascii_digit) {
            pos += 1;
        }
        if pos == start || pos - start > 9 {
            return Err(CaptureError::Decode("başlık"));
        }
        *field = std::str::from_utf8(&data[start..pos]).ok().and_then(|s| s.parse().ok()).ok_or(CaptureError::Decode("başlık"))?;
    }
    // Başlık tek bir boşlukla biter.
    if !data.get(pos).is_some_and(u8::is_ascii_whitespace) {
        return Err(CaptureError::Decode("başlık sonu"));
    }
    pos += 1;
    let [w, h, max] = fields;
    if max != 255 {
        return Err(CaptureError::Decode("yalnızca 8 bit"));
    }
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(CaptureError::Decode("boyut"));
    }
    let n = w as usize * h as usize;
    let body = data.get(pos..pos + n * 3).ok_or(CaptureError::Decode("eksik veri"))?;
    let mut rgba = Vec::with_capacity(n * 4);
    for px in body.chunks_exact(3) {
        rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    RgbaImage::new(w, h, rgba).ok_or(CaptureError::Decode("boyut"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grim_command_lines() {
        assert_eq!(grim_args(&Source::Output("DP-1".into())).unwrap(), ["-t", "ppm", "-o", "DP-1", "-"]);
        assert_eq!(grim_args(&Source::Region(LogicalRect::new(-10, 20, 300, 200))).unwrap(), ["-t", "ppm", "-g", "-10,20 300x200", "-"]);
        assert!(matches!(grim_args(&Source::Output("-c".into())), Err(CaptureError::BadOutput)));
        assert!(matches!(grim_args(&Source::Output("a b".into())), Err(CaptureError::BadOutput)));
        assert!(matches!(grim_args(&Source::Output(String::new())), Err(CaptureError::BadOutput)));
        assert!(grim_args(&Source::Region(LogicalRect::new(0, 0, 0, 5))).is_err());
    }

    #[test]
    fn deadline_kills_hung_child_only_after_timeout() {
        let spawn = |secs: &str| Command::new("sleep").arg(secs).stdin(Stdio::null()).spawn().unwrap();
        // Takılan çocuk süre sonunda öldürülür; bekleme süreyi aşmaz.
        let mut child = spawn("30");
        let started = std::time::Instant::now();
        let ((), fired) = with_deadline(&mut child, Duration::from_millis(100), |c| wait_exit_no_reap(c));
        assert!(fired);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!child.wait().unwrap().success());
        // Zamanında biten çocuğa dokunulmaz.
        let mut child = spawn("0");
        let ((), fired) = with_deadline(&mut child, Duration::from_secs(10), |c| wait_exit_no_reap(c));
        assert!(!fired);
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn ppm_parses_with_comments() {
        let mut data = b"P6\n# grim\n2 1\n255\n".to_vec();
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let img = parse_ppm(&data).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.pixel(0, 0), [1, 2, 3, 255]);
        assert_eq!(img.pixel(1, 0), [4, 5, 6, 255]);
        // Gövde ilk baytı boşluk karakteri olabilir (yalnızca bir ayırıcı tüketilir).
        let mut data = b"P6 1 1 255 ".to_vec();
        data.extend_from_slice(&[b' ', 10, 20]);
        assert_eq!(parse_ppm(&data).unwrap().pixel(0, 0), [b' ', 10, 20, 255]);
    }

    #[test]
    fn ppm_rejects_bad_input() {
        assert!(parse_ppm(b"P5\n1 1\n255\n\0").is_err());
        assert!(parse_ppm(b"P6\n1 1\n65535\n\0\0\0\0\0\0").is_err());
        assert!(parse_ppm(b"P6\n2 2\n255\n\0\0\0").is_err(), "eksik veri");
        assert!(parse_ppm(b"P6\n0 2\n255\n").is_err());
        assert!(parse_ppm(b"P6\n99999 2\n255\n").is_err());
        assert!(parse_ppm(b"P6\n1").is_err());
        assert!(parse_ppm(b"P6\n1 1\n255").is_err(), "başlık sonu yok");
        assert!(parse_ppm(b"").is_err());
    }
}
