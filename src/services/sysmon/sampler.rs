//! Örnekleyici: `/proc` ve `/sys`'ten ölçüm alır, önceki sayaçlarla farkı hesaplar.
//!
//! Ucuz yol: sık okunan dosyalar (`/proc/stat`, `meminfo`, `net/dev`, `diskstats`, sensörler)
//! bir kez açılır, her ölçümde aynı tanıtıcıdan 0 konumundan (`pread`) yeniden okunur; tampon
//! yeniden kullanılır. Pahalı ölçümler (süreç listesi, tüm sensörler, bağlama noktaları, frekans)
//! yalnızca ayrıntı kipinde (panel açık). Aygıt listeleri (fiziksel ağ arayüzleri, diskler,
//! sensörler, GPU'lar) belirli aralıkla yeniden keşfedilir (takılan aygıt yakalanır).
//!
//! Her okuma sınırlıdır ([`MAX_READ`]); okunamayan dosya o ölçümü `None` yapar, hiçbir hata
//! paniğe ya da servisin durmasına yol açmaz. Kökler (`/proc`, `/sys`) testte geçici dizindir.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::parse::{self, CpuTimes};
use super::{Usage, Gpu, Process, Rates, Snapshot, Temperature};

/// Tek dosyadan okunacak en çok bayt (`/proc/stat` çok çekirdekte ve `intr` satırıyla büyür).
pub const MAX_READ: usize = 1 << 20;
/// sysfs öznitelikleri tek sayfadır.
const MAX_ATTR: usize = 4096;
/// Okunacak en çok çekirdek satırı.
pub const MAX_CPUS: usize = 1024;
/// Keşif sınırları.
const MAX_HWMON: usize = 64;
const MAX_TEMPS_PER_HWMON: usize = 32;
pub const MAX_TEMPS: usize = 24;
pub const MAX_GPUS: usize = 8;
pub const MAX_MOUNTS: usize = 8;
const MAX_PROC_ENTRIES: usize = 65_536;
/// Aygıt listelerinin yeniden keşif aralığı.
const REDISCOVER: Duration = Duration::from_secs(30);
/// Kök dosya sisteminin doluluğu (bar/ipucu) bu aralıkla.
const ROOT_USAGE_EVERY: Duration = Duration::from_secs(15);

/// Okunan kökler.
#[derive(Debug, Clone)]
pub struct Roots {
    pub proc: PathBuf,
    pub sys: PathBuf,
}

impl Roots {
    pub fn system() -> Self {
        Self { proc: "/proc".into(), sys: "/sys".into() }
    }
}

/// Açık tutulan, 0 konumundan yeniden okunan dosya.
#[derive(Debug)]
struct Reopen {
    path: PathBuf,
    file: Option<File>,
}

impl Reopen {
    fn new(path: PathBuf) -> Self {
        let file = File::open(&path).ok();
        Self { path, file }
    }

    /// Metni `buf`'a okur (en çok `max`). Tanıtıcı geçersizleştiyse bir kez yeniden açar.
    fn read<'a>(&mut self, buf: &'a mut Vec<u8>, max: usize) -> Option<&'a str> {
        if self.file.is_none() {
            self.file = File::open(&self.path).ok();
        }
        match read_at_zero(self.file.as_ref()?, buf, max) {
            Some(n) => Some(utf8_prefix(&buf[..n])),
            None => {
                self.file = None;
                None
            }
        }
    }
}

/// Dosyayı 0 konumundan sonuna (ya da `max`'a) kadar `buf`'a okur. Okunan bayt sayısı.
fn read_at_zero(file: &File, buf: &mut Vec<u8>, max: usize) -> Option<usize> {
    let max = max.max(1);
    if buf.len() < 4096.min(max) {
        buf.resize(4096.min(max), 0);
    }
    let mut n = 0;
    loop {
        if n == buf.len() {
            if buf.len() >= max {
                break;
            }
            let grown = (buf.len() * 2).min(max);
            buf.resize(grown, 0);
        }
        match file.read_at(&mut buf[n..], n as u64) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(n)
}

/// Geçerli UTF-8 önek (kırpılmış çok baytlı karakter düşer).
fn utf8_prefix(b: &[u8]) -> &str {
    match std::str::from_utf8(b) {
        Ok(s) => s,
        Err(e) => std::str::from_utf8(&b[..e.valid_up_to()]).unwrap_or_default(),
    }
}

/// Küçük bir dosyayı yolundan bir kez okur (keşif ve ayrıntı ölçümleri).
fn read_small(path: &Path, max: usize) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut buf = Vec::new();
    let n = read_at_zero(&file, &mut buf, max)?;
    Some(utf8_prefix(&buf[..n]).to_owned())
}

/// Bir sıcaklık girdisi.
#[derive(Debug)]
struct TempSensor {
    label: String,
    input: Reopen,
    /// CPU sıcaklığı adayıysa önceliği (küçük = iyi).
    cpu_priority: Option<u8>,
}

/// Bilinen sürücülerde CPU sensörü önceliği: (sürücü, etiket) → öncelik.
fn cpu_priority(driver: &str, label: &str) -> Option<u8> {
    match driver {
        "k10temp" | "zenpower" => Some(match label {
            "Tctl" => 0,
            "Tdie" => 1,
            _ => 4,
        }),
        "coretemp" => Some(if label.starts_with("Package") { 0 } else { 5 }),
        "cpu_thermal" | "cpu-thermal" => Some(2),
        "x86_pkg_temp" => Some(3),
        "acpitz" => Some(8),
        _ => None,
    }
}

/// Sürücü adı / etiket: tek satır, yazdırılabilir, kısa.
fn clean_name(s: &str, max: usize) -> String {
    let s: String = s.trim().chars().filter(|c| !c.is_control()).take(max).collect();
    s
}

/// `dir` altındaki `prefix<N>` adlı girdiler, N'ye göre sıralı (en çok `limit`).
fn numbered_entries(dir: &Path, prefix: &str, limit: usize) -> Vec<(u32, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(u32, PathBuf)> = rd
        .flatten()
        .take(4096)
        .filter_map(|e| {
            let name = e.file_name();
            let n = name.to_str()?.strip_prefix(prefix)?.parse::<u32>().ok()?;
            Some((n, e.path()))
        })
        .collect();
    out.sort_unstable_by_key(|(n, _)| *n);
    out.truncate(limit);
    out
}

/// hwmon sıcaklıkları; hiç yoksa thermal bölgeleri. `drivetemp` atlanır (okuma diski uyandırır).
fn discover_temps(sys: &Path) -> Vec<TempSensor> {
    let mut out = Vec::new();
    for (_, dir) in numbered_entries(&sys.join("class/hwmon"), "hwmon", MAX_HWMON) {
        let driver = read_small(&dir.join("name"), 64).map(|s| clean_name(&s, 24)).unwrap_or_default();
        if driver.is_empty() || driver == "drivetemp" {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut inputs: Vec<(u32, PathBuf)> = rd
            .flatten()
            .take(512)
            .filter_map(|e| {
                let name = e.file_name();
                let n = name.to_str()?.strip_prefix("temp")?.strip_suffix("_input")?.parse::<u32>().ok()?;
                Some((n, e.path()))
            })
            .collect();
        inputs.sort_unstable_by_key(|(n, _)| *n);
        for (n, path) in inputs.into_iter().take(MAX_TEMPS_PER_HWMON) {
            let label = read_small(&dir.join(format!("temp{n}_label")), 64).map(|s| clean_name(&s, 24)).unwrap_or_default();
            let priority = cpu_priority(&driver, &label);
            let shown = if label.is_empty() { driver.clone() } else { format!("{driver} {label}") };
            out.push(TempSensor { label: shown, input: Reopen::new(path), cpu_priority: priority });
        }
    }
    if out.is_empty() {
        for (_, dir) in numbered_entries(&sys.join("class/thermal"), "thermal_zone", MAX_HWMON) {
            let kind = read_small(&dir.join("type"), 64).map(|s| clean_name(&s, 24)).unwrap_or_default();
            if kind.is_empty() {
                continue;
            }
            let priority = cpu_priority(&kind, "");
            out.push(TempSensor { label: kind, input: Reopen::new(dir.join("temp")), cpu_priority: priority });
        }
    }
    // CPU adayı önce (en iyi öncelik), sonra keşif sırası; liste sınırlı.
    out.sort_by_key(|s| s.cpu_priority.unwrap_or(u8::MAX));
    out.truncate(MAX_TEMPS);
    out
}

/// Bir GPU'nun açık dosyaları.
#[derive(Debug)]
struct GpuDev {
    driver: String,
    busy: Option<Reopen>,
    vram_used: Option<Reopen>,
    vram_total: Option<u64>,
    /// `power/runtime_status`: "suspended" iken diğer dosyalar okunmaz (okuma aygıtı uyandırır).
    runtime: Option<Reopen>,
}

/// `/sys/class/drm/cardN` (bağlayıcılar `cardN-…` değil). Yalnızca kullanım ya da VRAM verenler
/// (amdgpu); intel/nvidia sysfs'te vermez → listelenmez.
fn discover_gpus(sys: &Path) -> Vec<GpuDev> {
    let mut out = Vec::new();
    for (_, card) in numbered_entries(&sys.join("class/drm"), "card", 64) {
        if out.len() >= MAX_GPUS {
            break;
        }
        let dev = card.join("device");
        let exists = |name: &str| dev.join(name).exists();
        let (has_busy, has_vram) = (exists("gpu_busy_percent"), exists("mem_info_vram_used"));
        if !has_busy && !has_vram {
            continue;
        }
        let driver = std::fs::read_link(dev.join("driver"))
            .ok()
            .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(|n| clean_name(n, 24)))
            .unwrap_or_else(|| "GPU".into());
        out.push(GpuDev {
            driver,
            busy: has_busy.then(|| Reopen::new(dev.join("gpu_busy_percent"))),
            vram_used: has_vram.then(|| Reopen::new(dev.join("mem_info_vram_used"))),
            vram_total: read_small(&dev.join("mem_info_vram_total"), 64).and_then(|s| parse::parse_u64(&s)).filter(|&t| t > 0),
            runtime: exists("power/runtime_status").then(|| Reopen::new(dev.join("power/runtime_status"))),
        });
    }
    out
}

/// Ağ: fiziksel arayüzler (`/sys/class/net/<ad>/device` olanlar; VPN/köprü/sanal arayüzler
/// trafiği iki kez saydırır). Hiç yoksa `lo` dışındakiler.
fn physical_ifaces(sys: &Path) -> HashSet<String> {
    let Ok(rd) = std::fs::read_dir(sys.join("class/net")) else { return HashSet::new() };
    rd.flatten()
        .take(512)
        .filter(|e| e.path().join("device").exists())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect()
}

/// Diskler: `/sys/block/<ad>/device` olan bütün diskler (bölüm, zram, loop, dm hariç; G/Ç iki
/// kez sayılmaz).
fn whole_disks(sys: &Path) -> HashSet<String> {
    let Ok(rd) = std::fs::read_dir(sys.join("block")) else { return HashSet::new() };
    rd.flatten()
        .take(512)
        .filter(|e| e.path().join("device").exists())
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect()
}

fn statvfs_usage(path: &str) -> Option<(u64, u64)> {
    let s = rustix::fs::statvfs(path).ok()?;
    let frsize = s.f_frsize.max(1);
    let total = s.f_blocks.checked_mul(frsize)?;
    if total == 0 {
        return None;
    }
    let free = s.f_bfree.min(s.f_blocks).saturating_mul(frsize);
    Some((total.saturating_sub(free), total))
}

/// Yerel blok aygıtı bağlama noktaları (aygıt başına bir, `/` önce; en çok [`MAX_MOUNTS`]).
fn local_mounts(mounts: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for line in mounts.lines().take(4096) {
        let Some((dev, mount, fs)) = parse::parse_mount_line(line) else { continue };
        let network_fuse = fs == "fuse" || fs.starts_with("fuse.");
        if !dev.starts_with("/dev/") || matches!(fs, "squashfs" | "iso9660" | "udf") || network_fuse {
            continue;
        }
        if seen.insert(dev.to_owned()) {
            out.push(mount);
        }
    }
    out.sort_by_key(|m| m != "/");
    out.truncate(MAX_MOUNTS);
    out
}

/// Bir sürecin önceki tık sayısı.
#[derive(Debug, Default)]
struct ProcTable {
    prev: HashMap<u32, u64>,
    at: Option<Instant>,
}

/// Örnekleyicinin durumu.
#[derive(Debug)]
pub struct Sampler {
    roots: Roots,
    buf: Vec<u8>,
    stat: Reopen,
    meminfo: Reopen,
    net_dev: Reopen,
    diskstats: Reopen,
    loadavg: Reopen,
    cpu_prev: Vec<CpuTimes>,
    net_prev: Option<(u64, u64, Instant)>,
    disk_prev: Option<(u64, u64, Instant)>,
    temps: Vec<TempSensor>,
    gpus: Vec<GpuDev>,
    ifaces: HashSet<String>,
    disks: HashSet<String>,
    discovered: Option<Instant>,
    root_usage: Option<(Usage, Instant)>,
    procs: ProcTable,
    clk_tck: u64,
    page_size: u64,
}

impl Sampler {
    pub fn new(roots: Roots) -> Self {
        let p = |name: &str| Reopen::new(roots.proc.join(name));
        Self {
            stat: p("stat"),
            meminfo: p("meminfo"),
            net_dev: p("net/dev"),
            diskstats: p("diskstats"),
            loadavg: p("loadavg"),
            roots,
            buf: Vec::new(),
            cpu_prev: Vec::new(),
            net_prev: None,
            disk_prev: None,
            temps: Vec::new(),
            gpus: Vec::new(),
            ifaces: HashSet::new(),
            disks: HashSet::new(),
            discovered: None,
            root_usage: None,
            procs: ProcTable::default(),
            clk_tck: rustix::param::clock_ticks_per_second().max(1),
            page_size: rustix::param::page_size() as u64,
        }
    }

    /// Süreç tablosunu sıfırlar (ayrıntı kipi yeniden açıldı: eski tıklar bayat).
    pub fn reset_processes(&mut self) {
        self.procs = ProcTable::default();
    }

    fn rediscover(&mut self, now: Instant) {
        if self.discovered.is_some_and(|t| now.saturating_duration_since(t) < REDISCOVER) {
            return;
        }
        self.discovered = Some(now);
        self.temps = discover_temps(&self.roots.sys);
        self.gpus = discover_gpus(&self.roots.sys);
        self.ifaces = physical_ifaces(&self.roots.sys);
        self.disks = whole_disks(&self.roots.sys);
    }

    /// Bir ölçüm. `detail`: panel açık (süreçler, tüm sensörler, bağlama noktaları, yük, frekans).
    /// `processes`: en çok kaç süreç.
    pub fn sample(&mut self, detail: bool, processes: usize) -> Snapshot {
        let now = Instant::now();
        self.rediscover(now);
        let mut s = Snapshot { detail, ..Snapshot::default() };
        self.sample_cpu(&mut s);
        s.memory = self.meminfo.read(&mut self.buf, 64 * 1024).and_then(parse::parse_meminfo);
        self.sample_net(&mut s, now);
        self.sample_disk_io(&mut s, now);
        self.sample_temps(&mut s, detail);
        self.sample_gpus(&mut s);
        self.sample_root(&mut s, now);
        if detail {
            // Çekirdek sayısı son `/proc/stat` okumasından (ilk ölçümde de bilinir).
            let ncpu = self.cpu_prev.len().saturating_sub(1);
            s.load = self.loadavg.read(&mut self.buf, 4096).and_then(parse::parse_loadavg);
            s.freq_mhz = self.cpu_freq(ncpu);
            s.mounts = self.mounts();
            s.processes = self.processes(now, processes, ncpu.max(1));
        }
        s
    }

    fn sample_cpu(&mut self, s: &mut Snapshot) {
        let Some(text) = self.stat.read(&mut self.buf, MAX_READ) else { return };
        let cur = parse::parse_cpu_stat(text, MAX_CPUS);
        if let (Some(p), Some(c)) = (self.cpu_prev.first(), cur.first()) {
            s.cpu = parse::cpu_percent(*p, *c);
            // Çekirdek sayısı değiştiyse (hotplug) bu ölçümde çekirdek yok.
            if self.cpu_prev.len() == cur.len() {
                s.cores = self.cpu_prev[1..].iter().zip(&cur[1..]).map(|(p, c)| parse::cpu_percent(*p, *c).unwrap_or(0.0)).collect();
            }
        }
        self.cpu_prev = cur;
    }

    fn sample_net(&mut self, s: &mut Snapshot, now: Instant) {
        let Some(text) = self.net_dev.read(&mut self.buf, 256 * 1024) else { return };
        let rows = parse::parse_net_dev(text);
        let any_physical = rows.iter().any(|(n, _, _)| self.ifaces.contains(n));
        let (rx, tx) = rows
            .iter()
            .filter(|(n, _, _)| if any_physical { self.ifaces.contains(n) } else { n != "lo" })
            .fold((0u64, 0u64), |(a, b), (_, r, t)| (a.saturating_add(*r), b.saturating_add(*t)));
        if let Some((prx, ptx, at)) = self.net_prev {
            let dt = now.saturating_duration_since(at).as_secs_f64();
            if dt > 0.0 {
                s.net = Some(Rates { down: parse::rate(prx, rx, dt), up: parse::rate(ptx, tx, dt) });
            }
        }
        self.net_prev = Some((rx, tx, now));
    }

    fn sample_disk_io(&mut self, s: &mut Snapshot, now: Instant) {
        let Some(text) = self.diskstats.read(&mut self.buf, 256 * 1024) else { return };
        let (r, w) = parse::parse_diskstats(text)
            .iter()
            .filter(|(n, _, _)| self.disks.contains(n))
            .fold((0u64, 0u64), |(a, b), (_, r, w)| (a.saturating_add(*r), b.saturating_add(*w)));
        let (r, w) = (r.saturating_mul(512), w.saturating_mul(512));
        if let Some((pr, pw, at)) = self.disk_prev {
            let dt = now.saturating_duration_since(at).as_secs_f64();
            if dt > 0.0 {
                s.disk_io = Some(Rates { down: parse::rate(pr, r, dt), up: parse::rate(pw, w, dt) });
            }
        }
        self.disk_prev = Some((r, w, now));
    }

    /// Ayrıntısız: yalnızca en iyi CPU sensörü. Ayrıntıda hepsi.
    fn sample_temps(&mut self, s: &mut Snapshot, detail: bool) {
        let mut cpu: Option<(u8, f32)> = None;
        for t in &mut self.temps {
            if !detail && t.cpu_priority.is_none() {
                continue;
            }
            let Some(c) = t.input.read(&mut self.buf, MAX_ATTR).and_then(parse::parse_millidegrees) else { continue };
            if let Some(p) = t.cpu_priority
                && cpu.is_none_or(|(best, _)| p < best)
            {
                cpu = Some((p, c));
            }
            if detail {
                s.temps.push(Temperature { label: t.label.clone(), celsius: c });
            }
            // Liste CPU önceliğine göre sıralı: ayrıntısızda ilk okunan en iyisidir.
            if !detail && cpu.is_some() {
                break;
            }
        }
        s.cpu_temp = cpu.map(|(_, c)| c);
    }

    fn sample_gpus(&mut self, s: &mut Snapshot) {
        for g in &mut self.gpus {
            let suspended =
                g.runtime.as_mut().and_then(|r| r.read(&mut self.buf, MAX_ATTR)).is_some_and(|t| t.trim() == "suspended");
            let mut out = Gpu { driver: g.driver.clone(), suspended, busy: None, vram: None };
            if !suspended {
                out.busy = g
                    .busy
                    .as_mut()
                    .and_then(|f| f.read(&mut self.buf, MAX_ATTR))
                    .and_then(parse::parse_u64)
                    .map(|v| v.min(100) as f32);
                let used = g.vram_used.as_mut().and_then(|f| f.read(&mut self.buf, MAX_ATTR)).and_then(parse::parse_u64);
                out.vram = used.zip(g.vram_total).map(|(u, t)| Usage { used: u.min(t), total: t });
            }
            s.gpus.push(out);
        }
    }

    fn sample_root(&mut self, s: &mut Snapshot, now: Instant) {
        if let Some((u, at)) = self.root_usage
            && now.saturating_duration_since(at) < ROOT_USAGE_EVERY
        {
            s.root = Some(u);
            return;
        }
        // Kökler testte geçici dizin: `/` yerine sys kökünün üstü değil, gerçek `/` ölçülür.
        let usage = statvfs_usage("/").map(|(used, total)| Usage { used, total });
        self.root_usage = usage.map(|u| (u, now));
        s.root = usage;
    }

    fn cpu_freq(&mut self, cores: usize) -> Option<u32> {
        let base = self.roots.sys.join("devices/system/cpu");
        let (mut sum, mut n) = (0u64, 0u64);
        for i in 0..cores.min(MAX_CPUS) {
            let path = base.join(format!("cpu{i}/cpufreq/scaling_cur_freq"));
            if let Some(khz) = read_small(&path, 64).and_then(|t| parse::parse_u64(&t)) {
                sum = sum.saturating_add(khz);
                n += 1;
            }
        }
        (n > 0).then(|| u32::try_from(sum / n / 1000).unwrap_or(u32::MAX))
    }

    fn mounts(&mut self) -> Vec<(String, Usage)> {
        let Some(text) = read_small(&self.roots.proc.join("self/mounts"), 256 * 1024) else { return Vec::new() };
        local_mounts(&text)
            .into_iter()
            .filter_map(|m| statvfs_usage(&m).map(|(used, total)| (m, Usage { used, total })))
            .collect()
    }

    /// En çok CPU kullanan `limit` süreç (eşitlikte bellek). CPU payı tüm makineye göre (%).
    fn processes(&mut self, now: Instant, limit: usize, ncpu: usize) -> Vec<Process> {
        if limit == 0 {
            return Vec::new();
        }
        let dt = self.procs.at.map(|t| now.saturating_duration_since(t).as_secs_f64()).filter(|d| *d > 0.0);
        let Ok(rd) = std::fs::read_dir(&self.roots.proc) else { return Vec::new() };
        let mut next = HashMap::with_capacity(self.procs.prev.len());
        let mut all = Vec::new();
        for e in rd.flatten().take(MAX_PROC_ENTRIES) {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            let Some(st) = read_small(&e.path().join("stat"), 1024).and_then(|t| parse::parse_proc_stat(&t)) else { continue };
            let cpu = match (dt, self.procs.prev.get(&pid)) {
                (Some(dt), Some(&prev)) => {
                    let secs = st.ticks.saturating_sub(prev) as f64 / self.clk_tck as f64;
                    Some((secs / dt / ncpu as f64 * 100.0).clamp(0.0, 100.0) as f32)
                }
                _ => None,
            };
            next.insert(pid, st.ticks);
            all.push(Process { pid, name: st.name, cpu, rss: st.rss_pages.saturating_mul(self.page_size) });
        }
        self.procs = ProcTable { prev: next, at: Some(now) };
        sort_processes(&mut all);
        all.truncate(limit);
        all
    }
}

/// CPU'ya göre azalan (bilinmeyen en sonda), eşitlikte bellek, sonra pid.
pub fn sort_processes(v: &mut [Process]) {
    v.sort_by(|a, b| {
        let ca = a.cpu.unwrap_or(-1.0);
        let cb = b.cpu.unwrap_or(-1.0);
        cb.total_cmp(&ca).then(b.rss.cmp(&a.rss)).then(a.pid.cmp(&b.pid))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn fake_roots() -> (tempfile::TempDir, Roots) {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots { proc: dir.path().join("proc"), sys: dir.path().join("sys") };
        (dir, roots)
    }

    #[test]
    fn samples_from_fake_tree() {
        let (_d, roots) = fake_roots();
        let (p, s) = (roots.proc.clone(), roots.sys.clone());
        write(&p, "stat", "cpu  100 0 100 800 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0\ncpu1 50 0 50 400 0 0 0 0\n");
        write(&p, "meminfo", "MemTotal: 1000 kB\nMemAvailable: 250 kB\n");
        write(&p, "net/dev", "h\nh\nlo: 1000 0 0 0 0 0 0 0 1000\neth0: 100 0 0 0 0 0 0 0 50\nwg0: 100 0 0 0 0 0 0 0 50\n");
        write(&p, "diskstats", " 8 0 sda 0 0 10 0 0 0 20 0\n 8 1 sda1 0 0 10 0 0 0 20 0\n");
        write(&p, "loadavg", "1.00 0.50 0.25 1/2 3\n");
        write(&p, "42/stat", "42 (busy) R 1 1 1 0 -1 0 0 0 0 0 100 0 0 0 0 0 1 0 0 0 10\n");
        write(&p, "43/stat", "43 (idle) S 1 1 1 0 -1 0 0 0 0 0 5 0 0 0 0 0 1 0 0 0 99\n");
        write(&p, "self/mounts", "/dev/vda2 / ext4 rw 0 0\nproc /proc proc rw 0 0\n");
        write(&s, "class/net/eth0/device/x", "");
        write(&s, "class/net/wg0/mtu", "1420");
        write(&s, "block/sda/device/x", "");
        write(&s, "class/hwmon/hwmon0/name", "k10temp\n");
        write(&s, "class/hwmon/hwmon0/temp1_input", "55000\n");
        write(&s, "class/hwmon/hwmon0/temp1_label", "Tctl\n");
        write(&s, "class/hwmon/hwmon1/name", "nvme\n");
        write(&s, "class/hwmon/hwmon1/temp1_input", "40000\n");
        write(&s, "class/hwmon/hwmon2/name", "drivetemp\n");
        write(&s, "class/hwmon/hwmon2/temp1_input", "30000\n");
        write(&s, "class/drm/card0/device/gpu_busy_percent", "37\n");
        write(&s, "class/drm/card0/device/mem_info_vram_used", "1024\n");
        write(&s, "class/drm/card0/device/mem_info_vram_total", "4096\n");
        write(&s, "class/drm/card0-DP-1/status", "connected\n");
        write(&s, "class/drm/card1/device/power/runtime_status", "suspended\n");
        write(&s, "class/drm/card1/device/gpu_busy_percent", "99\n");
        write(&s, "devices/system/cpu/cpu0/cpufreq/scaling_cur_freq", "2000000\n");
        write(&s, "devices/system/cpu/cpu1/cpufreq/scaling_cur_freq", "3000000\n");

        let mut sm = Sampler::new(roots);
        let first = sm.sample(true, 5);
        assert_eq!(first.cpu, None, "ilk ölçüm taban");
        assert_eq!(first.memory.unwrap().used, 750 * 1024);
        assert_eq!(first.cpu_temp, Some(55.0));
        assert_eq!(first.temps.len(), 2, "drivetemp atlanır: {:?}", first.temps);
        assert_eq!(first.gpus.len(), 2);
        assert_eq!(first.gpus[0].busy, Some(37.0));
        assert_eq!(first.gpus[0].vram, Some(Usage { used: 1024, total: 4096 }));
        assert!(first.gpus[1].suspended && first.gpus[1].busy.is_none(), "uyuyan GPU okunmaz");
        assert_eq!(first.load, Some([1.0, 0.5, 0.25]));
        assert_eq!(first.freq_mhz, Some(2500));
        assert_eq!(first.processes.len(), 2);
        assert!(first.processes.iter().all(|p| p.cpu.is_none()));

        write(&p, "stat", "cpu  200 0 200 900 0 0 0 0\ncpu0 150 0 150 400 0 0 0 0\ncpu1 50 0 50 500 0 0 0 0\n");
        write(&p, "net/dev", "h\nh\nlo: 9000 0 0 0 0 0 0 0 9000\neth0: 300 0 0 0 0 0 0 0 150\nwg0: 900 0 0 0 0 0 0 0 900\n");
        write(&p, "diskstats", " 8 0 sda 0 0 20 0 0 0 40 0\n 8 1 sda1 0 0 20 0 0 0 40 0\n");
        write(&p, "42/stat", "42 (busy) R 1 1 1 0 -1 0 0 0 0 0 150 0 0 0 0 0 1 0 0 0 10\n");
        std::thread::sleep(Duration::from_millis(20));
        let second = sm.sample(true, 5);
        assert!((second.cpu.unwrap() - 66.666).abs() < 0.1, "{:?}", second.cpu);
        assert_eq!(second.cores, vec![100.0, 0.0]);
        let net = second.net.unwrap();
        assert!(net.down > 0.0 && net.up > 0.0);
        // Yalnızca fiziksel eth0: 200 bayt / dt; wg0 ve lo sayılmaz.
        assert!(net.down < 200.0 / 0.019, "{net:?}");
        assert!(second.disk_io.unwrap().down > 0.0);
        assert_eq!(second.processes[0].name, "busy");
        assert!(second.processes[0].cpu.unwrap() > 0.0);
        assert_eq!(second.processes[1].cpu, Some(0.0));
        // Ayrıntısız: süreç/sensör listesi yok, CPU sıcaklığı var.
        let plain = sm.sample(false, 5);
        assert!(plain.processes.is_empty() && plain.temps.is_empty() && plain.mounts.is_empty());
        assert_eq!(plain.cpu_temp, Some(55.0));
    }

    #[test]
    fn missing_tree_is_empty_not_panic() {
        let (_d, roots) = fake_roots();
        let mut sm = Sampler::new(roots);
        for detail in [false, true] {
            let s = sm.sample(detail, 5);
            assert!(s.cpu.is_none() && s.memory.is_none() && s.gpus.is_empty() && s.temps.is_empty());
        }
    }

    #[test]
    fn hostile_files_are_bounded() {
        let (_d, roots) = fake_roots();
        // Dev /proc/stat: okuma MAX_READ'de durur, panik yok.
        let big = format!("cpu  1 1 1 1\n{}", "x".repeat(MAX_READ * 2));
        write(&roots.proc, "stat", &big);
        write(&roots.proc, "meminfo", "\u{0}\u{ff}garbage");
        write(&roots.sys, "class/hwmon/hwmon0/name", &"\u{1b}k10temp".repeat(100));
        write(&roots.sys, "class/hwmon/hwmon0/temp1_input", "nope");
        let mut sm = Sampler::new(roots);
        let s = sm.sample(true, 5);
        assert!(s.memory.is_none() && s.cpu_temp.is_none());
        assert!(s.temps.is_empty());
    }

    #[test]
    fn mounts_are_local_and_deduplicated() {
        let m = local_mounts(
            "/dev/vda2 /home ext4 rw 0 0\n/dev/vda2 /var/lib/x ext4 rw 0 0\nproc /proc proc rw 0 0\n/dev/vda1 / ext4 rw 0 0\n\
             /dev/loop0 /snap/a squashfs ro 0 0\n/dev/fuse /net fuse.sshfs rw 0 0\n/dev/sdb1 /mnt/win fuseblk rw 0 0\n",
        );
        assert_eq!(m, ["/", "/home", "/mnt/win"]);
    }

    #[test]
    fn process_order() {
        let p = |pid, cpu, rss| Process { pid, name: String::new(), cpu, rss };
        let mut v = vec![p(1, None, 5), p(2, Some(1.0), 1), p(3, Some(1.0), 9), p(4, Some(50.0), 0)];
        sort_processes(&mut v);
        assert_eq!(v.iter().map(|p| p.pid).collect::<Vec<_>>(), [4, 3, 2, 1]);
    }
}
