//! Ajan ↔ diyalog sözleşmesi ve `BeginAuthentication` argümanlarının dönüşümü (saf, testli).
//!
//! Ajan diyalog sürecinin stdin'ine **tek satır JSON** ([`DialogRequest`]) yazar; diyalog
//! stdout'a **tek satır sonuç** ([`DialogOutcome::as_line`]) yazar. Sözleşmede parola alanı
//! yoktur ve olamaz: `deny_unknown_fields` ile fazladan alan (ör. `password`) isteği bozar.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zbus::zvariant::OwnedValue;

use crate::text;

/// Diyaloğun okuduğu istek satırının üst sınırı.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// Gösterilecek kimlik sayısı sınırı (polkitd grubu kullanıcılara açar; büyük grup olabilir).
pub const MAX_IDENTITIES: usize = 64;
/// Çerez uzunluğu sınırı (polkitd `<seri>-<sayaç>-<rastgele>` üretir; ~60 bayt).
pub const MAX_COOKIE_BYTES: usize = 256;
/// Eylem kimliği uzunluğu sınırı (polkit eylem kimlikleri kısa, ters DNS ASCII).
pub const MAX_ACTION_ID_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialogRequest {
    /// polkitd çerezi; yalnızca yardımcıya yazılır, gösterilmez.
    pub cookie: String,
    pub action_id: String,
    /// polkitd'nin (ajanın kayıt locale'iyle) çevirdiği mesaj.
    pub message: String,
    /// polkitd'nin verdiği ikon adı. Bilgi amaçlı: diyalog tema ikon dosyası çözmez (parolayı
    /// tutan süreçte görsel çözücü yok), anlamsal kilit ikonu çizer.
    pub icon_name: String,
    /// Seçilebilir kimlikler (`unix-user` uid'leri, polkitd sırasıyla).
    pub identities: Vec<u32>,
    /// İsteyen süreç, varsa ([`Requester`]).
    pub requester: Option<Requester>,
    /// Etkileşimsiz geçen bu süre sonra diyalog iptal olur.
    pub timeout_s: u64,
}

impl DialogRequest {
    /// Diyalog tarafında doğrulama: boş/aşırı alanlar reddedilir.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.cookie.is_empty()
            || self.cookie.len() > MAX_COOKIE_BYTES
            || self.cookie.bytes().any(|b| b.is_ascii_control() || b == b' ')
        {
            return Err("geçersiz çerez"); // i18n: tanı log ve polkitd hata yanıtı
        }
        if self.identities.is_empty() || self.identities.len() > MAX_IDENTITIES {
            return Err("kimlik listesi geçersiz"); // i18n: tanı log ve polkitd hata yanıtı
        }
        if self.action_id.len() > MAX_ACTION_ID_BYTES {
            return Err("eylem kimliği çok uzun"); // i18n: tanı log ve polkitd hata yanıtı
        }
        if self.timeout_s == 0 {
            return Err("zaman aşımı 0"); // i18n: tanı log ve polkitd hata yanıtı
        }
        Ok(())
    }

    /// stdin'den okunan tek satırı çözer (sınır ve doğrulama dahil).
    pub fn from_line(line: &[u8]) -> Result<Self, String> {
        if line.len() > MAX_REQUEST_BYTES {
            return Err("istek çok büyük".into()); // i18n: tanı diyalog logu
        }
        let req: Self = serde_json::from_slice(line).map_err(|e| format!("istek çözülemedi: {e}"))?; // i18n: tanı diyalog logu
        req.validate()?;
        Ok(req)
    }

    pub fn to_line(&self) -> String {
        // Yalnızca dizgi/sayı alanları: serileştirme başarısız olamaz.
        let mut s = serde_json::to_string(self).unwrap_or_default();
        s.push('\n');
        s
    }
}

/// Diyaloğun sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogOutcome {
    /// Yardımcı `SUCCESS` yazdı (polkitd yanıtı aldı).
    Success,
    /// Kullanıcı vazgeçti, zaman aşımı ya da iptal.
    Cancelled,
    /// Hata (yardımcıya ulaşılamadı, protokol ihlali…).
    Failed,
}

impl DialogOutcome {
    pub fn as_line(self) -> &'static str {
        match self {
            Self::Success => "success\n",
            Self::Cancelled => "cancelled\n",
            Self::Failed => "failed\n",
        }
    }

    /// Diyaloğun stdout'u. Tam olarak bilinen tek satır değilse (boş, fazlası, bozuk) `Failed`:
    /// diyalog çökerse ya da öldürülürse istek başarısızdır (fail-closed).
    pub fn parse(output: &[u8]) -> Self {
        match output {
            b"success\n" => Self::Success,
            b"cancelled\n" => Self::Cancelled,
            _ => Self::Failed,
        }
    }
}

/// `a(sa{sv})` kimliklerinden `unix-user` uid'leri. polkitd grupları kullanıcılara açarak
/// gönderir (`polkitbackendinteractiveauthority.c`: "expand groups/netgroups to users"); başka
/// tür gelirse atlanır. Tekrarlar atılır, sıra korunur, en çok [`MAX_IDENTITIES`].
pub fn user_identities(identities: &[(String, HashMap<String, OwnedValue>)]) -> Vec<u32> {
    let mut out = Vec::new();
    for (kind, props) in identities {
        if kind != "unix-user" {
            tracing::debug!(kind, "desteklenmeyen kimlik türü atlandı");
            continue;
        }
        let Some(uid) = props.get("uid").and_then(|v| u32::try_from(v).ok()) else {
            continue;
        };
        if !out.contains(&uid) {
            out.push(uid);
        }
        if out.len() == MAX_IDENTITIES {
            break;
        }
    }
    out
}

/// Varsayılan kimlik: çağıran kullanıcı listedeyse o, değilse ilki.
pub fn default_identity(identities: &[u32], own_uid: u32) -> usize {
    identities.iter().position(|&u| u == own_uid).unwrap_or(0)
}

/// İsteyen süreç ve bilginin ne kadar güvenilir olduğu. Yollar ham gider; gösterim (kaçış,
/// `~`, ortadan kırpma) diyalogda [`text::display_path`] ile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requester {
    /// Doğrulanmış sistem yürütülebilir dosyası (`/proc/<pid>/exe`, [`exe_verified`]): kök
    /// sahipli, grup/diğer yazamaz, tüm üst dizinleri de öyle, kullanıcı yazılabilir köklerin
    /// dışında, yalnızca düz ASCII ve yol hâlâ çalışan inode'u gösteriyor; süreç istekten önce
    /// başlamış ve okuma boyunca aynı ([`ProcRead::stable`]). Yorumlayıcıyla çalışan betikte
    /// yorumlayıcının yoludur. İstekten önce yapılmış exec'i ayırt edemez (bkz. polkit.md).
    Exe(String),
    /// Yol biliniyor ama doğrulanamadı (ev dizini, /tmp…, kullanıcıya ait ya da yazılabilir,
    /// silinmiş/değiştirilmiş, ASCII dışı/karışabilir karakter). "Doğrulanmamış" etiketiyle.
    UnverifiedExe(String),
    /// Sürecin adı (`/proc/<pid>/comm`). Süreç bunu `prctl(PR_SET_NAME)` ile **kendisi seçer**;
    /// yalnızca `exe` okunamadığında (ör. başka kullanıcının süreci) yedek olarak verilir ve
    /// diyalogda "doğrulanmamış" etiketiyle gösterilir.
    Comm(String),
}

/// Bir dosyanın stat özeti.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub dev: u64,
    pub ino: u64,
    pub uid: u32,
    pub mode: u32,
}

impl FileStat {
    fn from_meta(m: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: m.dev(),
            ino: m.ino(),
            uid: m.uid(),
            mode: m.mode(),
        }
    }

    /// Kök sahipli ve grup/diğer yazamaz.
    fn root_only(&self) -> bool {
        self.uid == 0 && self.mode & 0o022 == 0
    }

    fn is_dir(&self) -> bool {
        self.mode & 0o170000 == 0o040000
    }

    fn is_file(&self) -> bool {
        self.mode & 0o170000 == 0o100000
    }
}

/// `/proc/<pid>/exe` okuması: bağlantı hedefi (UTF-8 değilse kayıplı) ve sürecin **gerçekten
/// çalıştırdığı** inode'un stat'ı (bağlantı üzerinden; yol üzerinden değil — yarışsız).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExeInfo {
    pub path: String,
    /// Yol geçerli UTF-8 miydi (değilse doğrulanmaz).
    pub lossless: bool,
    pub stat: Option<FileStat>,
}

/// Kullanıcının yazabildiği kökler (ev dizini ayrıca).
pub const USER_WRITABLE_ROOTS: &[&str] = &["/tmp", "/var/tmp", "/run/user", "/dev/shm"];

fn under(path: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    !root.is_empty() && (path == root || path.strip_prefix(root).is_some_and(|r| r.starts_with('/')))
}

/// Yol doğrulanmış bir sistem yürütülebilir dosyası mı (bkz. [`Requester::Exe`]).
/// `stat_path` yolu sembolik bağlantı izlemeden stat eder (test için dışarıdan).
pub fn exe_verified(exe: &ExeInfo, home: Option<&str>, stat_path: impl Fn(&str) -> Option<FileStat>) -> bool {
    let p = exe.path.as_str();
    if !exe.lossless || !p.starts_with('/') || !p.chars().all(text::is_plain_ascii) {
        return false;
    }
    // Kanonik olmayan yol (readlink kanonik verir; değilse güvenme).
    if p.len() > 1 && p[1..].split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return false;
    }
    if home.is_some_and(|h| under(p, h)) || USER_WRITABLE_ROOTS.iter().any(|r| under(p, r)) {
        return false;
    }
    let Some(running) = exe.stat else {
        return false;
    };
    if !running.is_file() || !running.root_only() {
        return false;
    }
    // Yol hâlâ çalışan inode'u göstermeli (silinmiş "(deleted)" ya da değiştirilmiş değil).
    match stat_path(p) {
        Some(s) if s.dev == running.dev && s.ino == running.ino => {}
        _ => return false,
    }
    // Üst dizinlerin hepsi kök sahipli ve yalnız kökün yazabildiği gerçek dizinler.
    let mut dir = p;
    while let Some(i) = dir.rfind('/') {
        dir = if i == 0 { "/" } else { &dir[..i] };
        match stat_path(dir) {
            Some(s) if s.is_dir() && s.root_only() => {}
            _ => return false,
        }
        if dir == "/" {
            break;
        }
    }
    true
}

/// Bir PID'in istek anına bağlı okuması ([`read_proc`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcRead {
    pub exe: Option<ExeInfo>,
    pub comm: Option<String>,
    /// Okumanın tamamı istekten önce başlamış **aynı** sürece ait ve okuma boyunca `exe`
    /// değişmedi ([`same_process`]). Değilse yol hiçbir koşulda doğrulanmış sayılmaz.
    pub stable: bool,
}

/// İsteyen süreç: `polkit.subject-pid` (yetkiyi isteyen uygulama), yoksa `polkit.caller-pid`
/// (polkit'e soran servis). Her PID için önce `exe` yolu (doğrulanmış ya da değil), okunamazsa
/// `comm` adı. `read` istek anından SONRA başlamış süreçte (PID yeniden kullanımı) `None`
/// döner: o PID atlanır. Okuyucular dışarıdan verilir (test).
pub fn requester(
    details: &HashMap<String, String>,
    home: Option<&str>,
    read: impl Fn(u32) -> Option<ProcRead>,
    stat_path: impl Fn(&str) -> Option<FileStat>,
) -> Option<Requester> {
    ["polkit.subject-pid", "polkit.caller-pid"]
        .iter()
        .filter_map(|k| details.get(*k))
        .filter_map(|v| v.trim().parse::<u32>().ok())
        .filter(|pid| *pid > 0)
        .find_map(|pid| {
            let r = read(pid)?;
            r.exe
                .filter(|e| !e.path.is_empty() && e.path.len() <= MAX_PATH_BYTES)
                .map(|e| {
                    if r.stable && exe_verified(&e, home, &stat_path) {
                        Requester::Exe(e.path)
                    } else {
                        Requester::UnverifiedExe(e.path)
                    }
                })
                .or_else(|| r.comm.filter(|c| !c.is_empty()).map(Requester::Comm))
        })
}

/// Okumanın istek anındaki süreçle tutarlılığı (saf). `arrival`: isteğin geldiği an,
/// `start_*`: okumadan önce/sonra `/proc/<pid>/stat` başlangıç zamanı (ikisi de saat tıkı,
/// CLOCK_BOOTTIME), `exe_*`: başlangıç zamanı okumalarının arasında iki kez okunan `exe`,
/// `alive`: okumadan önce açılan pidfd'nin süreci okuma sonunda hâlâ yaşıyor (yaşıyorsa PID
/// arada yeniden kullanılamaz).
pub fn same_process(
    arrival: u64,
    start_before: u64,
    start_after: u64,
    exe_before: Option<&ExeInfo>,
    exe_after: Option<&ExeInfo>,
    alive: bool,
) -> bool {
    alive && start_before == start_after && start_before <= arrival && exe_before == exe_after
}

/// Şimdiki CLOCK_BOOTTIME, `/proc/<pid>/stat` başlangıç zamanıyla aynı birimde (saat tıkı).
/// İstek geldiği anda alınır; etiket bu ana aittir.
pub fn boot_ticks_now() -> u64 {
    let ts = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
    let hz = rustix::param::clock_ticks_per_second().max(1);
    let ns = u128::try_from(ts.tv_sec).unwrap_or(0) * 1_000_000_000 + u128::try_from(ts.tv_nsec).unwrap_or(0);
    u64::try_from(ns * u128::from(hz) / 1_000_000_000).unwrap_or(u64::MAX)
}

/// `/proc/<pid>/stat` satırının 22. alanı (başlangıç zamanı, açılıştan beri saat tıkı). `comm`
/// boşluk ve `)` içerebilir: alanlar son `)`'den sonra 3. alandan başlar.
pub fn parse_start_time(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_ascii_whitespace().nth(22 - 3)?.parse().ok()
}

fn read_start_time(pid: u32) -> Option<u64> {
    parse_start_time(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// PID'i istek anına bağlı okur. `arrival`tan sonra başlamış süreçte (PID yeniden kullanıldı:
/// asıl istekçi çıkmış) ya da süreç yoksa `None`. Sıra: pidfd aç → başlangıç zamanı → exe →
/// comm → exe → başlangıç zamanı → pidfd hâlâ yaşıyor mu. Tutarsızlık (süreç arada öldü,
/// başlangıç zamanı ya da exe değişti — ör. okuma sırasında exec) → `stable = false`.
pub fn read_proc(pid: u32, arrival: u64) -> Option<ProcRead> {
    let pidfd = rustix::process::Pid::from_raw(i32::try_from(pid).ok()?)
        .and_then(|p| rustix::process::pidfd_open(p, rustix::process::PidfdFlags::empty()).ok());
    let start_before = read_start_time(pid)?;
    if start_before > arrival {
        tracing::warn!(pid, "isteyen PID istekten sonra başlamış süreç (yeniden kullanım); atlandı");
        return None;
    }
    let exe_before = read_proc_exe(pid);
    let comm = read_proc_comm(pid);
    let exe_after = read_proc_exe(pid);
    let start_after = read_start_time(pid).unwrap_or(u64::MAX);
    // pidfd süreç çıkınca okunabilir olur (POLLIN); açılamadıysa yaşadığı kanıtlanamaz.
    let alive = pidfd.is_some_and(|fd| {
        let mut fds = [rustix::event::PollFd::new(&fd, rustix::event::PollFlags::IN)];
        let zero = rustix::time::Timespec { tv_sec: 0, tv_nsec: 0 };
        matches!(rustix::event::poll(&mut fds, Some(&zero)), Ok(0))
    });
    let stable = same_process(arrival, start_before, start_after, exe_before.as_ref(), exe_after.as_ref(), alive);
    if !stable {
        tracing::warn!(pid, "isteyen süreç okuma sırasında değişti; doğrulanmamış sayılır");
    }
    Some(ProcRead { exe: exe_before, comm, stable })
}

/// Sözleşmede yol üst sınırı (PATH_MAX).
pub const MAX_PATH_BYTES: usize = 4096;

/// `/proc/<pid>/exe`: hedef yol ve çalışan inode'un stat'ı. Başka kullanıcının sürecinde
/// (ptrace erişimi yok) ya da çekirdek iş parçacığında `None`.
pub fn read_proc_exe(pid: u32) -> Option<ExeInfo> {
    let link = format!("/proc/{pid}/exe");
    let target = std::fs::read_link(&link).ok()?;
    // metadata bağlantıyı izler: sürecin eşlediği inode (silinmiş olsa da).
    let stat = std::fs::metadata(&link).ok().map(|m| FileStat::from_meta(&m));
    let (path, lossless) = match target.to_str() {
        Some(s) => (s.to_owned(), true),
        None => (target.to_string_lossy().into_owned(), false),
    };
    Some(ExeInfo { path, lossless, stat })
}

/// Yolu sembolik bağlantı izlemeden stat eder.
pub fn stat_path(path: &str) -> Option<FileStat> {
    std::fs::symlink_metadata(path).ok().map(|m| FileStat::from_meta(&m))
}

/// Bu sürecin kullanıcısının ev dizini (`getpwuid_r`, yoksa `$HOME`); `~` gösterimi ve
/// kullanıcı yazılabilir kök denetimi için.
pub fn own_home() -> Option<String> {
    let uid = rustix::process::getuid().as_raw();
    nuxut_pam::lookup_uid(uid)
        .ok()
        .and_then(|u| u.home.to_str().map(str::to_owned))
        .or_else(|| std::env::var("HOME").ok())
        .filter(|h| h.starts_with('/') && h.len() > 1)
}

/// `/proc/<pid>/comm` (en çok 64 bayt; çekirdek 15 karakterle sınırlar).
pub fn read_proc_comm(pid: u32) -> Option<String> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(64);
    std::fs::File::open(format!("/proc/{pid}/comm"))
        .ok()?
        .take(64)
        .read_to_end(&mut buf)
        .ok()?;
    Some(String::from_utf8_lossy(&buf).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn req() -> DialogRequest {
        DialogRequest {
            cookie: "3-abc-1-def".into(),
            action_id: "org.freedesktop.timedate1.set-time".into(),
            message: "Saati değiştirmek için kimlik doğrulama gerekiyor".into(),
            icon_name: String::new(),
            identities: vec![1000, 0],
            requester: Some(Requester::Exe("/usr/bin/timedatectl".into())),
            timeout_s: 300,
        }
    }

    #[test]
    fn json_roundtrip_and_no_password_field() {
        let line = req().to_line();
        assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'));
        assert_eq!(DialogRequest::from_line(line.trim_end().as_bytes()).unwrap(), req());
        let lower = line.to_lowercase();
        for k in ["password", "parola", "secret", "response"] {
            assert!(!lower.contains(k), "{k}");
        }
        // Sözleşmeye eklenmiş bir parola alanı isteği bozar.
        let mut v: serde_json::Value = serde_json::from_str(&line).unwrap();
        v["password"] = "x".into();
        assert!(DialogRequest::from_line(v.to_string().as_bytes()).is_err());
    }

    #[test]
    fn request_validation() {
        let mut r = req();
        r.identities.clear();
        assert!(r.validate().is_err());
        let mut r = req();
        r.cookie = "a b".into();
        assert!(r.validate().is_err());
        let mut r = req();
        r.cookie = "a\nSUCCESS".into();
        assert!(r.validate().is_err());
        let mut r = req();
        r.timeout_s = 0;
        assert!(r.validate().is_err());
        assert!(DialogRequest::from_line(&vec![b' '; MAX_REQUEST_BYTES + 1]).is_err());
        assert!(DialogRequest::from_line(b"{").is_err());
    }

    #[test]
    fn outcome_lines_fail_closed() {
        for o in [DialogOutcome::Success, DialogOutcome::Cancelled, DialogOutcome::Failed] {
            assert_eq!(DialogOutcome::parse(o.as_line().as_bytes()), o);
        }
        for bad in [&b""[..], b"success", b"success\nsuccess\n", b"SUCCESS\n", b"ok\n"] {
            assert_eq!(DialogOutcome::parse(bad), DialogOutcome::Failed, "{bad:?}");
        }
    }

    fn ident(kind: &str, key: &str, v: u32) -> (String, HashMap<String, OwnedValue>) {
        let mut m = HashMap::new();
        m.insert(key.to_owned(), OwnedValue::try_from(Value::U32(v)).unwrap());
        (kind.to_owned(), m)
    }

    #[test]
    fn identities_and_default() {
        let ids = vec![
            ident("unix-group", "gid", 10),
            ident("unix-user", "uid", 0),
            ident("unix-user", "uid", 1000),
            ident("unix-user", "uid", 1000),
            ident("unix-user", "gid", 5),
            ("unix-user".into(), {
                let mut m = HashMap::new();
                m.insert("uid".into(), OwnedValue::from(zbus::zvariant::Str::from("1000")));
                m
            }),
        ];
        let uids = user_identities(&ids);
        assert_eq!(uids, [0, 1000]);
        assert_eq!(default_identity(&uids, 1000), 1);
        assert_eq!(default_identity(&uids, 1001), 0);
        let many: Vec<_> = (0..100).map(|u| ident("unix-user", "uid", u)).collect();
        assert_eq!(user_identities(&many).len(), MAX_IDENTITIES);
    }

    /// Tutarlı (istekten önce başlamış, okuma boyunca aynı) süreç okuması.
    fn read_all(
        d: &HashMap<String, String>,
        home: Option<&str>,
        read_exe: impl Fn(u32) -> Option<ExeInfo>,
        stat: impl Fn(&str) -> Option<FileStat>,
        read_comm: impl Fn(u32) -> Option<String>,
    ) -> Option<Requester> {
        let read = |pid| {
            let (exe, comm) = (read_exe(pid), read_comm(pid));
            (exe.is_some() || comm.is_some()).then_some(ProcRead { exe, comm, stable: true })
        };
        requester(d, home, read, stat)
    }

    #[test]
    fn requester_prefers_subject_then_caller() {
        let mut d = HashMap::new();
        d.insert("polkit.caller-pid".to_owned(), "20".to_owned());
        d.insert("polkit.subject-pid".to_owned(), "10".to_owned());
        let no_exe = |_: u32| None;
        let no_stat = |_: &str| None;
        let comm = |pid: u32| match pid {
            10 => Some("app\u{202E}gpj".to_owned()),
            20 => Some("udisksd".to_owned()),
            _ => None,
        };
        let c = |s: &str| Some(Requester::Comm(s.to_owned()));
        // comm ham gider; diyalog kaçışlı gösterir (yön karakteri görünür kalır).
        assert_eq!(read_all(&d, None, no_exe, no_stat, comm), c("app\u{202E}gpj"));
        d.insert("polkit.subject-pid".to_owned(), "99".to_owned());
        assert_eq!(read_all(&d, None, no_exe, no_stat, comm), c("udisksd"));
        d.insert("polkit.caller-pid".to_owned(), "x".to_owned());
        assert_eq!(read_all(&d, None, no_exe, no_stat, comm), None);
        let mut d = HashMap::new();
        d.insert("polkit.caller-pid".to_owned(), "0".to_owned());
        assert_eq!(read_all(&d, None, no_exe, no_stat, |_| Some("init".into())), None);
        assert_eq!(read_all(&HashMap::new(), None, no_exe, no_stat, |_| Some("x".into())), None);
    }

    const ROOT_DIR: FileStat = FileStat { dev: 1, ino: 2, uid: 0, mode: 0o040755 };
    const ROOT_BIN: FileStat = FileStat { dev: 1, ino: 77, uid: 0, mode: 0o100755 };

    fn exe(path: &str, stat: Option<FileStat>) -> ExeInfo {
        ExeInfo { path: path.into(), lossless: true, stat }
    }

    /// Sahte dosya sistemi: `files` dışındaki her yol kök sahipli dizin.
    fn fs(files: Vec<(&'static str, FileStat)>) -> impl Fn(&str) -> Option<FileStat> {
        move |p: &str| files.iter().find(|(f, _)| *f == p).map(|(_, s)| *s).or(Some(ROOT_DIR))
    }

    /// Taklit yolları asla doğrulanmış sistem yolu olmaz; gerçek kök sahipli yol olur.
    #[test]
    fn exe_verification_rejects_spoofs() {
        let home = Some("/home/u");
        let sys = fs(vec![("/usr/bin/pkexec", ROOT_BIN)]);
        assert!(exe_verified(&exe("/usr/bin/pkexec", Some(ROOT_BIN)), home, &sys));
        // Ev altında "usr/bin" taklidi (dosya kök sahipli olsa bile).
        let spoof = "/home/u/.cache/x/usr/bin/gnome-software";
        assert!(!exe_verified(&exe(spoof, Some(ROOT_BIN)), home, fs(vec![])));
        // Diğer kullanıcı yazılabilir kökler.
        for p in ["/tmp/usr/bin/x", "/var/tmp/x", "/run/user/1000/x", "/dev/shm/x"] {
            assert!(!exe_verified(&exe(p, Some(ROOT_BIN)), home, fs(vec![])), "{p}");
        }
        // Dosya kullanıcıya ait ya da grup/diğer yazılabilir.
        let mine = FileStat { uid: 1000, ..ROOT_BIN };
        assert!(!exe_verified(&exe("/opt/x/bin/app", Some(mine)), home, fs(vec![])));
        let gw = FileStat { mode: 0o100775, ..ROOT_BIN };
        assert!(!exe_verified(&exe("/opt/x/bin/app", Some(gw)), home, fs(vec![])));
        // Üst dizin kullanıcıya ait ya da yazılabilir (ör. /opt/x kullanıcıya verilmiş).
        let user_dir = FileStat { uid: 1000, ..ROOT_DIR };
        let p = exe("/opt/x/bin/app", Some(ROOT_BIN));
        assert!(!exe_verified(&p, home, fs(vec![("/opt/x/bin/app", ROOT_BIN), ("/opt/x", user_dir)])));
        let sticky = FileStat { mode: 0o041777, ..ROOT_DIR };
        assert!(!exe_verified(&p, home, fs(vec![("/opt/x/bin/app", ROOT_BIN), ("/opt", sticky)])));
        assert!(exe_verified(&p, home, fs(vec![("/opt/x/bin/app", ROOT_BIN)])));
        // Yol artık çalışan inode'u göstermiyor (silinmiş/değiştirilmiş).
        let other = FileStat { ino: 78, ..ROOT_BIN };
        assert!(!exe_verified(&p, home, fs(vec![("/opt/x/bin/app", other)])));
        assert!(!exe_verified(&exe("/usr/bin/x (deleted)", Some(ROOT_BIN)), home, |_| None));
        // Homoglyph / ASCII dışı / boşluk / kayıplı yol / kanonik olmayan yol / stat yok.
        let ok = fs(vec![]);
        assert!(!exe_verified(&exe("/usr/bin/gnom\u{435}-software", Some(ROOT_BIN)), home, &ok));
        assert!(!exe_verified(&exe("/usr/bin/a b", Some(ROOT_BIN)), home, &ok));
        assert!(!exe_verified(&ExeInfo { lossless: false, ..exe("/usr/bin/x", Some(ROOT_BIN)) }, home, &ok));
        assert!(!exe_verified(&exe("/usr/bin/../bin/x", Some(ROOT_BIN)), home, &ok));
        assert!(!exe_verified(&exe("/usr/bin/x", None), home, &ok));
        assert!(!exe_verified(&exe("usr/bin/x", Some(ROOT_BIN)), home, &ok));
    }

    #[test]
    fn requester_classifies_exe() {
        let mut d = HashMap::new();
        d.insert("polkit.subject-pid".to_owned(), "10".to_owned());
        d.insert("polkit.caller-pid".to_owned(), "20".to_owned());
        let comm = |pid: u32| match pid {
            10 => Some("Yazılım Güncelleme".to_owned()),
            20 => Some("udisksd".to_owned()),
            _ => None,
        };
        let home = Some("/home/u");
        let spoof = "/home/u/.cache/x/usr/bin/gnome-software";
        let r = read_all(&d, home, |_| Some(exe(spoof, Some(ROOT_BIN))), fs(vec![]), comm);
        assert_eq!(r, Some(Requester::UnverifiedExe(spoof.into())));
        let r = read_all(
            &d,
            home,
            |_| Some(exe("/usr/bin/pkexec", Some(ROOT_BIN))),
            fs(vec![("/usr/bin/pkexec", ROOT_BIN)]),
            comm,
        );
        assert_eq!(r, Some(Requester::Exe("/usr/bin/pkexec".into())));
        // exe okunamıyor (başka kullanıcı): comm, doğrulanmamış.
        let r = read_all(&d, home, |_| None, fs(vec![]), comm);
        assert_eq!(r, Some(Requester::Comm("Yazılım Güncelleme".into())));
        // Sözleşme JSON'u gidip gelir.
        let mut req = DialogRequest {
            cookie: "c".into(),
            action_id: "a".into(),
            message: String::new(),
            icon_name: String::new(),
            identities: vec![0],
            requester: Some(Requester::Exe("/usr/bin/x".into())),
            timeout_s: 1,
        };
        for r in [
            Requester::Exe("/usr/bin/x".into()),
            Requester::UnverifiedExe("/home/u/x".into()),
            Requester::Comm("x".into()),
        ] {
            req.requester = Some(r);
            assert_eq!(DialogRequest::from_line(req.to_line().trim_end().as_bytes()).unwrap(), req);
        }
    }

    /// TOCTOU: PID yeniden kullanımı ve okuma sırasında exec doğrulanmış etiket vermez.
    #[test]
    fn unstable_or_reused_pid_never_verified() {
        let bin = exe("/usr/bin/pkexec", Some(ROOT_BIN));
        let other = exe("/usr/bin/gnome-software", Some(FileStat { ino: 78, ..ROOT_BIN }));
        // Tutarlı okuma.
        assert!(same_process(100, 50, 50, Some(&bin), Some(&bin), true));
        assert!(same_process(100, 100, 100, Some(&bin), Some(&bin), true));
        // Başlangıç zamanı istekten sonra (PID yeniden kullanıldı).
        assert!(!same_process(100, 101, 101, Some(&bin), Some(&bin), true));
        // Başlangıç zamanı okuma sırasında değişti (süreç çıktı, PID yeniden kullanıldı).
        assert!(!same_process(100, 50, 60, Some(&bin), Some(&bin), true));
        // Okuma sırasında exec.
        assert!(!same_process(100, 50, 50, Some(&bin), Some(&other), true));
        assert!(!same_process(100, 50, 50, Some(&bin), None, true));
        // pidfd'nin süreci öldü ya da pidfd açılamadı.
        assert!(!same_process(100, 50, 50, Some(&bin), Some(&bin), false));

        let mut d = HashMap::new();
        d.insert("polkit.subject-pid".to_owned(), "10".to_owned());
        d.insert("polkit.caller-pid".to_owned(), "20".to_owned());
        let sys = fs(vec![("/usr/bin/pkexec", ROOT_BIN)]);
        let read = |stable| {
            let bin = bin.clone();
            move |pid: u32| (pid == 10).then(|| ProcRead { exe: Some(bin.clone()), comm: None, stable })
        };
        assert_eq!(requester(&d, None, read(true), &sys), Some(Requester::Exe("/usr/bin/pkexec".into())));
        // Kararsız okuma: sistem yolu bile doğrulanmamış konum.
        assert_eq!(
            requester(&d, None, read(false), &sys),
            Some(Requester::UnverifiedExe("/usr/bin/pkexec".into()))
        );
        // Özne PID'i istekten sonra başlamış (okuyucu None): atlanır, çağıran PID'e geçilir.
        let caller = |pid: u32| {
            (pid == 20).then(|| ProcRead { exe: None, comm: Some("udisksd".into()), stable: true })
        };
        assert_eq!(requester(&d, None, caller, &sys), Some(Requester::Comm("udisksd".into())));
    }

    #[test]
    fn stat_start_time_parsing() {
        // Alan 22 = 21 (başlangıç zamanı); comm boşluk ve ')' içerebilir.
        let tail = "S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 424242 20 21";
        assert_eq!(parse_start_time(&format!("77 (a) b) c) {tail}")), Some(424_242));
        assert_eq!(parse_start_time(&format!("77 (sleep) {tail}")), Some(424_242));
        assert_eq!(parse_start_time("77 (x) S 1 2"), None);
        assert_eq!(parse_start_time("bozuk"), None);
    }

    /// Gerçek /proc: istekten önce başlamış süreç tutarlı okunur; istekten sonra başlamış
    /// süreç (PID yeniden kullanımı benzetimi) atlanır.
    #[test]
    fn real_proc_start_time_vs_arrival() {
        let me = std::process::id();
        let r = read_proc(me, boot_ticks_now()).unwrap();
        assert!(r.stable && r.exe.is_some() && r.comm.is_some(), "{r:?}");
        let before_child = boot_ticks_now();
        // Saat tıkı (≈10 ms) ilerlesin: çocuk kesinlikle "istekten sonra" başlar.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut child = std::process::Command::new("/usr/bin/sleep").arg("5").spawn().unwrap();
        let reused = read_proc(child.id(), before_child);
        // exec bitene dek bekle (okuma sırasında exec kararsız sayılır).
        let start = std::time::Instant::now();
        while read_proc_exe(child.id()).is_none_or(|i| i.path != "/usr/bin/sleep")
            && start.elapsed() < std::time::Duration::from_secs(3)
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let now = read_proc(child.id(), boot_ticks_now());
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(reused, None);
        assert!(now.is_some_and(|r| r.stable));
    }

    /// Gerçek /proc: başka uid'in süreci (PID 1, root) için exe okunamaz (ptrace erişimi yok) →
    /// en fazla "doğrulanmamış ad", asla doğrulanmış yol. Root olarak çalışırken anlamsız; atlanır.
    #[test]
    fn other_uid_process_never_verified() {
        if rustix::process::getuid().is_root() {
            return;
        }
        assert_eq!(read_proc_exe(1), None);
        let mut d = HashMap::new();
        d.insert("polkit.subject-pid".to_owned(), "1".to_owned());
        let r = requester(&d, own_home().as_deref(), |pid| read_proc(pid, boot_ticks_now()), stat_path);
        assert!(matches!(r, Some(Requester::Comm(_)) | None), "{r:?}");
    }

    /// Gerçek /proc: ev altındaki test ikilisi doğrulanmaz; kök sahipli /usr/bin aracı doğrulanır.
    #[test]
    fn real_proc_exe_classification() {
        let me = std::process::id();
        assert!(read_proc_comm(me).is_some_and(|s| !s.is_empty()));
        let info = read_proc_exe(me).unwrap();
        assert_eq!(Some(info.path.as_str()), std::env::current_exe().unwrap().to_str());
        let home = own_home();
        if std::env::current_exe().unwrap().starts_with(home.as_deref().unwrap_or("/home")) {
            assert!(!exe_verified(&info, home.as_deref(), stat_path));
        }
        let mut child = std::process::Command::new("/usr/bin/sleep").arg("5").spawn().unwrap();
        // spawn exec bitmeden dönebilir: exe hedefi değişene dek bekle.
        let start = std::time::Instant::now();
        let mut info = read_proc_exe(child.id());
        while info.as_ref().is_none_or(|i| i.path != "/usr/bin/sleep")
            && start.elapsed() < std::time::Duration::from_secs(3)
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
            info = read_proc_exe(child.id());
        }
        let _ = child.kill();
        let _ = child.wait();
        let info = info.unwrap();
        assert_eq!(info.path, "/usr/bin/sleep");
        assert!(exe_verified(&info, home.as_deref(), stat_path), "{info:?}");
    }
}
