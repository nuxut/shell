//! Sistem izleyicisinin saf ayrıştırıcıları: `/proc` ve `/sys` metinlerinden sayılar.
//!
//! Hepsi güvenilmeyen (bozuk, kırpılmış, düşmanca) metne karşı dayanıklıdır: hiçbir koşulda
//! panik yok, taşan sayı `None`/atlanır, liste uzunlukları sınırlı. Okuma (IO) burada değil:
//! `sampler.rs` sınırlı tamponla okur, buraya metni verir.

/// Bir CPU satırının (ya da toplam satırın) sayaçları (jiffies).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuTimes {
    /// user + nice + system + irq + softirq + steal (guest user'ın içinde, tekrar sayılmaz).
    pub busy: u64,
    /// busy + idle + iowait.
    pub total: u64,
}

/// `/proc/stat`'ın `cpu` satırları: ilk eleman toplam (`cpu `), sonra `cpuN` sırasıyla.
/// Toplam satırı yoksa boş. En çok `max_cpus` çekirdek satırı okunur.
pub fn parse_cpu_stat(text: &str, max_cpus: usize) -> Vec<CpuTimes> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let Some(rest) = line.strip_prefix("cpu") else { break };
        // İlk satır toplam olmalı ("cpu  ..."); çekirdek satırı "cpuN ...".
        let is_total = rest.starts_with(' ');
        if (i == 0) != is_total {
            break;
        }
        if !is_total && !rest.starts_with(|c: char| c.is_ascii_digit()) {
            break;
        }
        if out.len() > max_cpus {
            break;
        }
        let mut fields = rest.split_ascii_whitespace();
        if !is_total {
            fields.next(); // çekirdek numarası
        }
        let vals: Vec<u64> = fields.take(8).map_while(|f| f.parse().ok()).collect();
        if vals.len() < 4 {
            break;
        }
        let get = |k: usize| vals.get(k).copied().unwrap_or(0);
        // user nice system idle iowait irq softirq steal
        let busy = [0, 1, 2, 5, 6, 7].iter().fold(0u64, |a, &k| a.saturating_add(get(k)));
        let idle = get(3).saturating_add(get(4));
        out.push(CpuTimes { busy, total: busy.saturating_add(idle) });
    }
    out
}

/// İki ölçüm arasındaki kullanım yüzdesi (0..=100). Sayaç geri gittiyse ya da zaman ilerlemediyse `None`.
pub fn cpu_percent(prev: CpuTimes, cur: CpuTimes) -> Option<f32> {
    let dt = cur.total.checked_sub(prev.total)?;
    if dt == 0 {
        return None;
    }
    let db = cur.busy.saturating_sub(prev.busy).min(dt);
    Some((db as f64 / dt as f64 * 100.0) as f32)
}

/// Bellek ve takas (bayt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Memory {
    pub total: u64,
    pub used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

/// `/proc/meminfo`. `MemTotal` yoksa ya da 0 ise `None`. `MemAvailable` yoksa (eski çekirdek)
/// `MemFree + Buffers + Cached` kullanılır.
pub fn parse_meminfo(text: &str) -> Option<Memory> {
    let (mut total, mut avail, mut free, mut buffers, mut cached, mut swap_total, mut swap_free) =
        (None, None, None, None, None, None, None);
    for line in text.lines().take(256) {
        let Some((key, rest)) = line.split_once(':') else { continue };
        let slot = match key {
            "MemTotal" => &mut total,
            "MemAvailable" => &mut avail,
            "MemFree" => &mut free,
            "Buffers" => &mut buffers,
            "Cached" => &mut cached,
            "SwapTotal" => &mut swap_total,
            "SwapFree" => &mut swap_free,
            _ => continue,
        };
        let mut it = rest.split_ascii_whitespace();
        let Some(v) = it.next().and_then(|v| v.parse::<u64>().ok()) else { continue };
        // Birim yoksa bayt, "kB" ise KiB (çekirdek hep kB yazar).
        let mul = if it.next() == Some("kB") { 1024 } else { 1 };
        *slot = v.checked_mul(mul);
    }
    let total = total.filter(|&t| t > 0)?;
    let avail = avail.unwrap_or_else(|| free.unwrap_or(0).saturating_add(buffers.unwrap_or(0)).saturating_add(cached.unwrap_or(0)));
    let swap_total = swap_total.unwrap_or(0);
    Some(Memory {
        total,
        used: total.saturating_sub(avail.min(total)),
        swap_total,
        swap_used: swap_total.saturating_sub(swap_free.unwrap_or(swap_total).min(swap_total)),
    })
}

/// Arayüz adının sınırı (çekirdek `IFNAMSIZ` 16; daha uzunu bozuk sayılır).
pub const MAX_IFACE_NAME: usize = 15;
/// Okunacak en çok arayüz / disk satırı.
pub const MAX_DEVICES: usize = 256;

/// `/proc/net/dev`: (arayüz, alınan bayt, gönderilen bayt).
pub fn parse_net_dev(text: &str) -> Vec<(String, u64, u64)> {
    let mut out = Vec::new();
    for line in text.lines().skip(2) {
        if out.len() >= MAX_DEVICES {
            break;
        }
        let Some((name, rest)) = line.split_once(':') else { continue };
        let name = name.trim();
        if name.is_empty() || name.len() > MAX_IFACE_NAME {
            continue;
        }
        let f: Vec<u64> = rest.split_ascii_whitespace().take(9).map_while(|v| v.parse().ok()).collect();
        if f.len() < 9 {
            continue;
        }
        out.push((name.to_owned(), f[0], f[8]));
    }
    out
}

/// Disk adının sınırı (`DISK_NAME_LEN` 32).
pub const MAX_DISK_NAME: usize = 32;

/// `/proc/diskstats`: (aygıt, okunan sektör, yazılan sektör). Sektör her zaman 512 bayttır.
pub fn parse_diskstats(text: &str) -> Vec<(String, u64, u64)> {
    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() >= MAX_DEVICES {
            break;
        }
        let f: Vec<&str> = line.split_ascii_whitespace().take(10).collect();
        if f.len() < 10 || f[2].len() > MAX_DISK_NAME {
            continue;
        }
        let (Ok(read), Ok(write)) = (f[5].parse::<u64>(), f[9].parse::<u64>()) else { continue };
        out.push((f[2].to_owned(), read, write));
    }
    out
}

/// `/proc/loadavg`'ın ilk üç alanı.
pub fn parse_loadavg(text: &str) -> Option<[f32; 3]> {
    let mut it = text.split_ascii_whitespace().map(|v| v.parse::<f32>().ok().filter(|x| x.is_finite() && *x >= 0.0));
    Some([it.next()??, it.next()??, it.next()??])
}

/// Bir sürecin `/proc/<pid>/stat`'ından gerekenler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcStat {
    /// `comm` (en çok 15 bayt; kontrol karakterleri temizlenmiş).
    pub name: String,
    /// utime + stime (saat tıkı).
    pub ticks: u64,
    /// Yerleşik bellek (sayfa).
    pub rss_pages: u64,
}

/// `comm`'un çekirdekteki sınırı (`TASK_COMM_LEN` - 1); fazlası kırpılır.
pub const MAX_COMM: usize = 15;

/// `/proc/<pid>/stat`. Ad parantez içindedir ve kendisi `)` ya da boşluk içerebilir: son `)`
/// adın sonudur.
pub fn parse_proc_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    if close < open {
        return None;
    }
    let raw = &text[open + 1..close];
    let mut name: String = raw.chars().filter(|c| !c.is_control()).collect();
    if name.len() > MAX_COMM {
        let mut end = MAX_COMM;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    // Ad sonrası alanlar: 0 = durum (alan 3); utime alan 14 → 11, stime 15 → 12, rss 24 → 21.
    let f: Vec<&str> = text[close + 1..].split_ascii_whitespace().take(22).collect();
    let num = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok());
    let ticks = num(11)?.checked_add(num(12)?)?;
    // rss negatif olamaz; çekirdek işlemcilerinde 0.
    let rss_pages = f.get(21).and_then(|v| v.parse::<i64>().ok()).map_or(0, |v| v.max(0) as u64);
    Some(ProcStat { name, ticks, rss_pages })
}

/// hwmon/thermal `temp*_input` / `temp` değeri (mili °C) → °C. Makul aralık dışı (sensör hatası,
/// "-273" gibi yer tutucular) `None`.
pub fn parse_millidegrees(text: &str) -> Option<f32> {
    let v: i64 = text.trim().parse().ok()?;
    let c = v as f64 / 1000.0;
    (-40.0..=200.0).contains(&c).then_some(c as f32)
}

/// sysfs'teki tek tam sayılık dosya (`gpu_busy_percent`, `mem_info_vram_used`, `scaling_cur_freq`).
pub fn parse_u64(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

/// `/proc/self/mounts` satırı: (aygıt, bağlama noktası, dosya sistemi). Bağlama noktasındaki
/// sekizlik kaçışlar (`\040` = boşluk) çözülür.
pub fn parse_mount_line(line: &str) -> Option<(&str, String, &str)> {
    let mut f = line.split_ascii_whitespace();
    let dev = f.next()?;
    let mount = unescape_octal(f.next()?)?;
    let fs = f.next()?;
    Some((dev, mount, fs))
}

fn unescape_octal(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            let v = (b[i + 1] - b'0') as u32 * 64 + (b[i + 2] - b'0') as u32 * 8 + (b[i + 3] - b'0') as u32;
            out.push(u8::try_from(v).ok()?);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Sayaç farkından saniye başına hız. Sayaç geri gittiyse (arayüz yeniden kuruldu) 0.
pub fn rate(prev: u64, cur: u64, secs: f64) -> f64 {
    if secs <= 0.0 || !secs.is_finite() {
        return 0.0;
    }
    cur.saturating_sub(prev) as f64 / secs
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  100 10 50 800 40 5 5 0 0 0\n\
                        cpu0 50 5 25 400 20 2 3 0 0 0\n\
                        cpu1 50 5 25 400 20 3 2 0 0 0\n\
                        intr 1234 5 6\nctxt 99\n";

    #[test]
    fn cpu_stat_total_and_cores() {
        let v = parse_cpu_stat(STAT, 64);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], CpuTimes { busy: 170, total: 1010 });
        assert_eq!(v[1].busy, 85);
        let cur = CpuTimes { busy: 270, total: 1210 };
        assert_eq!(cpu_percent(v[0], cur), Some(50.0));
        assert_eq!(cpu_percent(v[0], v[0]), None, "zaman ilerlemedi");
        assert_eq!(cpu_percent(cur, v[0]), None, "sayaç geri gitti");
        // busy farkı total farkını aşamaz.
        assert_eq!(cpu_percent(CpuTimes { busy: 0, total: 0 }, CpuTimes { busy: 50, total: 10 }), Some(100.0));
    }

    #[test]
    fn cpu_stat_hostile() {
        assert!(parse_cpu_stat("", 8).is_empty());
        assert!(parse_cpu_stat("cpu0 1 2 3 4\n", 8).is_empty(), "toplam satırı yok");
        assert!(parse_cpu_stat("cpu  a b c d\n", 8).is_empty());
        assert_eq!(parse_cpu_stat("cpu  1 2 3\n", 8).len(), 0, "eksik alan");
        let huge = format!("cpu  {m} {m} {m} {m} {m} {m} {m} {m}\n", m = u64::MAX);
        let v = parse_cpu_stat(&huge, 8);
        assert_eq!(v[0].total, u64::MAX, "taşma doyar");
        // Çekirdek sınırı.
        let mut many = String::from("cpu  1 1 1 1\n");
        for i in 0..100 {
            many.push_str(&format!("cpu{i} 1 1 1 1\n"));
        }
        assert_eq!(parse_cpu_stat(&many, 8).len(), 9);
        // Kırpılmış son satır yalnızca kendisini düşürür.
        assert_eq!(parse_cpu_stat("cpu  1 2 3 4\ncpu0 1 2", 8).len(), 1);
        assert_eq!(parse_cpu_stat("cpu  1 2 3 4\ncpux 1 2 3 4\n", 8).len(), 1);
    }

    #[test]
    fn meminfo() {
        let m = parse_meminfo(
            "MemTotal:       16000 kB\nMemFree: 1000 kB\nMemAvailable:    6000 kB\nSwapTotal: 2000 kB\nSwapFree: 1500 kB\n",
        )
        .unwrap();
        assert_eq!(m, Memory { total: 16000 * 1024, used: 10000 * 1024, swap_total: 2000 * 1024, swap_used: 500 * 1024 });
        // MemAvailable yok: free + buffers + cached.
        let m = parse_meminfo("MemTotal: 100 kB\nMemFree: 10 kB\nBuffers: 5 kB\nCached: 25 kB\n").unwrap();
        assert_eq!(m.used, 60 * 1024);
        assert_eq!(m.swap_total, 0);
    }

    #[test]
    fn meminfo_hostile() {
        assert_eq!(parse_meminfo(""), None);
        assert_eq!(parse_meminfo("MemTotal: 0 kB\n"), None);
        assert_eq!(parse_meminfo("MemTotal: x kB\n"), None);
        let big = format!("MemTotal: {} kB\nMemAvailable: {} kB\n", u64::MAX, u64::MAX);
        assert_eq!(parse_meminfo(&big), None, "taşan değer yok sayılır");
        // Mevcut > toplam: kullanılan 0, eksiye düşmez.
        let m = parse_meminfo("MemTotal: 10 kB\nMemAvailable: 99 kB\nSwapTotal: 1 kB\nSwapFree: 9 kB\n").unwrap();
        assert_eq!((m.used, m.swap_used), (0, 0));
    }

    #[test]
    fn net_dev() {
        let t = "Inter-|   Receive |  Transmit\n face |bytes packets|bytes\n    lo:  100 1 0 0 0 0 0 0 200 1 0 0 0 0 0 0\n\
                 enp1s0: 3294649 12642 0 6 0 0 0 0 7969756 14782 0 0 0 0 0 0\nbad\nx: 1 2\n";
        assert_eq!(parse_net_dev(t), vec![("lo".into(), 100, 200), ("enp1s0".into(), 3_294_649, 7_969_756)]);
        let long = format!("h\nh\n{}: 1 1 1 1 1 1 1 1 1\n", "a".repeat(40));
        assert!(parse_net_dev(&long).is_empty());
        let many: String = std::iter::repeat_n("e: 1 1 1 1 1 1 1 1 1\n", 1000).collect();
        assert_eq!(parse_net_dev(&format!("h\nh\n{many}")).len(), MAX_DEVICES);
    }

    #[test]
    fn diskstats() {
        let t = " 254       0 vda 75883 11351 12940393 95063 48182 3919 6306808 70202 0 32812\n 1 2 short\n";
        assert_eq!(parse_diskstats(t), vec![("vda".into(), 12_940_393, 6_306_808)]);
        assert!(parse_diskstats("1 2 x a b c d e f g h\n").is_empty());
    }

    #[test]
    fn loadavg() {
        assert_eq!(parse_loadavg("0.52 0.41 0.30 1/234 5678\n"), Some([0.52, 0.41, 0.30]));
        assert_eq!(parse_loadavg("0.5 nan 1"), None);
        assert_eq!(parse_loadavg("0.5"), None);
        assert_eq!(parse_loadavg("-1 0 0"), None);
    }

    #[test]
    fn proc_stat() {
        let t = "1234 (my (weird) proc) S 1 1234 1234 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 1 0 1000 123456 789 18446744073709551615";
        let p = parse_proc_stat(t).unwrap();
        assert_eq!(p, ProcStat { name: "my (weird) proc".into(), ticks: 300, rss_pages: 789 });
        assert_eq!(parse_proc_stat("1 (x) S"), None, "kırpılmış");
        assert_eq!(parse_proc_stat(")("), None);
        assert_eq!(parse_proc_stat(""), None);
        // Kontrol karakterleri ve uzun ad.
        let t = "1 (a\u{1b}[31mbçççççççççççççç) S 0 0 0 0 0 0 0 0 0 0 1 2 0 0 0 0 0 0 0 0 -5";
        let p = parse_proc_stat(t).unwrap();
        assert!(p.name.len() <= MAX_COMM && !p.name.chars().any(char::is_control), "{:?}", p.name);
        assert_eq!(p.rss_pages, 0, "negatif rss 0");
        // Taşan tık toplamı.
        let t = format!("1 (x) S 0 0 0 0 0 0 0 0 0 0 {m} {m} 0 0 0 0 0 0 0 0 1", m = u64::MAX);
        assert_eq!(parse_proc_stat(&t), None);
    }

    #[test]
    fn small_values() {
        assert_eq!(parse_millidegrees("52000\n"), Some(52.0));
        assert_eq!(parse_millidegrees("-273000"), None);
        assert_eq!(parse_millidegrees("999999999"), None);
        assert_eq!(parse_millidegrees("x"), None);
        assert_eq!(parse_u64(" 42\n"), Some(42));
        assert_eq!(parse_u64("-1"), None);
        assert_eq!(rate(100, 300, 2.0), 100.0);
        assert_eq!(rate(300, 100, 2.0), 0.0, "sayaç sıfırlandı");
        assert_eq!(rate(0, 100, 0.0), 0.0);
        assert_eq!(rate(0, 100, f64::NAN), 0.0);
    }

    #[test]
    fn mounts() {
        assert_eq!(parse_mount_line("/dev/vda2 / ext4 rw 0 0"), Some(("/dev/vda2", "/".into(), "ext4")));
        assert_eq!(parse_mount_line("/dev/sdb1 /media/my\\040disk vfat rw 0 0").unwrap().1, "/media/my disk");
        assert_eq!(parse_mount_line("/dev/x /a\\777b ext4"), None, "bayt aralığı dışı");
        assert_eq!(parse_mount_line("/dev/x /a\\04 ext4").unwrap().1, "/a\\04", "yarım kaçış olduğu gibi");
        assert_eq!(parse_mount_line("/dev/x"), None);
    }
}
