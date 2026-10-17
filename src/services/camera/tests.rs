use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::sync::mpsc;

/// Sahte procfs: `pid/comm` ve `pid/fd/N` → hedef.
fn fake_proc(root: &Path, pid: u32, comm: &str, targets: &[&Path]) {
    let fd = root.join(pid.to_string()).join("fd");
    std::fs::create_dir_all(&fd).unwrap();
    std::fs::write(root.join(pid.to_string()).join("comm"), format!("{comm}\n")).unwrap();
    for (i, t) in targets.iter().enumerate() {
        symlink(t, fd.join(i.to_string())).unwrap();
    }
}

/// Sahte `status` (Uid/Gid: gerçek, etkin, saklı, fs).
fn fake_status(root: &Path, pid: u32, uid: u32, gid: u32, groups: &[u32]) {
    let groups: Vec<String> = groups.iter().map(u32::to_string).collect();
    let text = format!("Name:\tx\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nGid:\t{gid}\t{gid}\t{gid}\t{gid}\nGroups:\t{} \n", groups.join(" "));
    std::fs::write(root.join(pid.to_string()).join("status"), text).unwrap();
}

/// Sahte `stat`: ppid ve başlangıç zamanı (alan 4 ve 22); ad boşluk/parantez içerir.
fn fake_stat(root: &Path, pid: u32, ppid: u32, start: u64) {
    let mut fields = vec!["S".to_string(), ppid.to_string()];
    fields.extend((0..17).map(|_| "0".to_string()));
    fields.push(start.to_string());
    std::fs::write(root.join(pid.to_string()).join("stat"), format!("{pid} (a b) c) {}", fields.join(" "))).unwrap();
}

fn lock_fd_dir(root: &Path, pid: u32) {
    std::fs::set_permissions(root.join(pid.to_string()).join("fd"), std::fs::Permissions::from_mode(0o000)).unwrap();
}

fn unlock_fd_dir(root: &Path, pid: u32) {
    std::fs::set_permissions(root.join(pid.to_string()).join("fd"), std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn me() -> (u32, u32) {
    let m = std::fs::metadata("/proc/self").unwrap();
    (m.uid(), m.gid())
}

#[test]
fn node_names() {
    assert!(is_video_node("video0") && is_video_node("video12"));
    assert!(!is_video_node("video") && !is_video_node("video0x") && !is_video_node("videodev") && !is_video_node("media0"));
    assert!(!is_video_node("video0 (deleted)"), "silinmiş düğümün bağlantısı sayılmaz");
}

#[test]
fn scan_finds_holders_by_link_only() {
    let dev = tempfile::tempdir().unwrap();
    let proc = tempfile::tempdir().unwrap();
    let cam = dev.path().join("video0");
    std::fs::write(&cam, b"").unwrap();
    let other = dev.path().join("media0");
    std::fs::write(&other, b"").unwrap();
    fake_proc(proc.path(), 10, "firefox", &[&other, &cam]);
    fake_proc(proc.path(), 11, "firefox", &[&cam]);
    fake_proc(proc.path(), 12, "kitty", &[&other]);
    fake_proc(proc.path(), 13, "obs\u{7}", &[&cam]);
    fake_proc(proc.path(), 14, "nuxut-shell", &[&cam]);
    std::fs::create_dir_all(proc.path().join("self")).unwrap();
    let s = scan(dev.path(), proc.path(), 14);
    assert_eq!(s.users, ["firefox", "obs"]);
    assert_eq!(s.fds, 3, "firefox(10), firefox(11), obs; atlanan kabuk sayılmaz");
    assert_eq!(s.per_device.get(&cam), Some(&3));
    assert_eq!((s.unreadable, s.suspects), (0, 0));
    assert_eq!(scan(dev.path(), &proc.path().join("yok"), 0), Scan { partial: true, ..Scan::default() });
    assert!(s.complete());
    // fd dizini okunamayan, kimliği de okunamayan süreç: adsız, fd'siz, şüpheli (fail-closed).
    fake_proc(proc.path(), 20, "sandbox", &[&cam]);
    lock_fd_dir(proc.path(), 20);
    let s = scan(dev.path(), proc.path(), 14);
    unlock_fd_dir(proc.path(), 20);
    assert_eq!((s.unreadable, s.suspects, s.fds), (1, 1, 3), "okunamayan sahip ne ad ne fd verir");
    assert!(!s.complete(), "şüpheli varken tarama eksik");
}

/// Okunamayan süreç yalnız düğümü açabiliyorsa şüphelidir; root tehdit modeli dışı; düğüm yoksa
/// kimse şüpheli değil.
#[test]
fn suspects_follow_node_permissions() {
    let dev = tempfile::tempdir().unwrap();
    let proc = tempfile::tempdir().unwrap();
    let (uid, gid) = me();
    let cam = dev.path().join("video0");
    std::fs::write(&cam, b"").unwrap();
    std::fs::set_permissions(&cam, std::fs::Permissions::from_mode(0o660)).unwrap();
    let pids = [(30, uid, gid, vec![]), (31, 4242, 4242, vec![]), (32, 4242, 4242, vec![gid]), (33, 0, 0, vec![])];
    for (pid, u, g, groups) in &pids {
        fake_proc(proc.path(), *pid, "x", &[]);
        fake_status(proc.path(), *pid, *u, *g, groups);
        lock_fd_dir(proc.path(), *pid);
    }
    let s = scan(dev.path(), proc.path(), 0);
    let s_none = scan(&dev.path().join("bos"), proc.path(), 0);
    for (pid, ..) in &pids {
        unlock_fd_dir(proc.path(), *pid);
    }
    assert_eq!(s.unreadable, 4);
    assert_eq!(s.suspects, 2, "sahip (30) ve grup üyesi (32); başka uid (31) ve root (33) değil");
    assert_eq!(s_none.suspects, 0, "kamera düğümü yoksa şüpheli yok");
}

#[test]
fn acl_access_algorithm() {
    let creds = |fsuid, fsgid, groups: &[u32]| Creds { ruid: fsuid, fsuid, fsgid, groups: groups.to_vec() };
    let e = |tag, perm, id| AclEntry { tag, perm, id };
    // root:video 0660 + uaccess ACL user:1000:rw, mask rw.
    let node = NodeAccess {
        uid: 0,
        gid: 44,
        mode: 0o20660,
        acl: Some(vec![e(ACL_USER_OBJ, 6, 0), e(ACL_USER, 6, 1000), e(ACL_GROUP_OBJ, 6, 0), e(ACL_MASK, 6, 0), e(ACL_OTHER, 0, 0)]),
    };
    assert!(node.allows(&creds(1000, 1000, &[])), "uaccess ACL");
    assert!(node.allows(&creds(976, 976, &[44])), "video grubu");
    assert!(!node.allows(&creds(976, 976, &[])), "sistem uid'i");
    // Maske adlandırılmış girdiyi kısar.
    let masked = NodeAccess { acl: Some(vec![e(ACL_USER_OBJ, 6, 0), e(ACL_USER, 6, 1000), e(ACL_GROUP_OBJ, 0, 0), e(ACL_MASK, 0, 0), e(ACL_OTHER, 0, 0)]), ..node.clone() };
    assert!(!masked.allows(&creds(1000, 1000, &[])));
    // ACL yok: mod bitleri.
    let plain = NodeAccess { acl: Some(vec![]), ..node.clone() };
    assert!(!plain.allows(&creds(1000, 1000, &[])) && plain.allows(&creds(1000, 1000, &[44])));
    // ACL okunamadı: herkes açabilir sayılır (fail-closed).
    assert!(NodeAccess { acl: None, ..node }.allows(&creds(976, 976, &[])));
    // Ham biçim: sürüm 2 + 8 baytlık girdiler; bozuk boy reddedilir.
    let mut raw = 2u32.to_le_bytes().to_vec();
    raw.extend([2, 0, 6, 0, 0xe8, 3, 0, 0]);
    assert_eq!(parse_acl(&raw), Some(vec![e(ACL_USER, 6, 1000)]));
    assert_eq!(parse_acl(&raw[..7]), None);
    assert_eq!(parse_acl(&[1, 0, 0, 0]), None, "sürüm 1");
}

#[test]
fn creds_and_stat_parsing() {
    let c = Creds::parse("Uid:\t1000\t0\t1000\t7\nGid:\t5\t5\t5\t9\nGroups:\t44 98 \n").unwrap();
    assert_eq!(c, Creds { ruid: 1000, fsuid: 7, fsgid: 9, groups: vec![44, 98] });
    assert!(Creds::parse("Uid:\t1000\n").is_none());
    assert_eq!(parse_stat("12 (we) ird) S 34 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 999 5"), Some((34, 999)));
    assert_eq!(parse_stat("12 (x) S"), None);
}

/// Oturumun `(sd-pam)`'i muaf; adını taklit eden sonradan başlamış çocuk muaf değil.
#[test]
fn session_pam_is_exempt_impostor_is_not() {
    let dev = tempfile::tempdir().unwrap();
    let proc = tempfile::tempdir().unwrap();
    let (uid, gid) = me();
    std::fs::write(dev.path().join("video0"), b"").unwrap();
    // systemd --user (100) → (sd-pam) (101, ilk çocuk), birim (102).
    fake_proc(proc.path(), 100, "systemd", &[]);
    fake_status(proc.path(), 100, uid, gid, &[]);
    std::fs::write(proc.path().join("100/cmdline"), b"/usr/lib/systemd/systemd\0--user\0").unwrap();
    std::fs::create_dir_all(proc.path().join("100/task/100")).unwrap();
    std::fs::write(proc.path().join("100/task/100/children"), "101 102 ").unwrap();
    fake_stat(proc.path(), 100, 1, 939);
    for (pid, start) in [(101, 941), (102, 952)] {
        fake_proc(proc.path(), pid, "(sd-pam)", &[]);
        fake_status(proc.path(), pid, uid, gid, &[]);
        fake_stat(proc.path(), pid, 100, start);
        lock_fd_dir(proc.path(), pid);
    }
    let s = scan(dev.path(), proc.path(), 0);
    assert_eq!((s.unreadable, s.suspects), (2, 1), "yalnız taklitçi şüpheli");
    // Ebeveyn `systemd --user` değilse gerçek (sd-pam) de muaf değil.
    std::fs::write(proc.path().join("100/cmdline"), b"/usr/lib/systemd/systemd\0").unwrap();
    let s = scan(dev.path(), proc.path(), 0);
    for pid in [101, 102] {
        unlock_fd_dir(proc.path(), pid);
    }
    assert_eq!(s.suspects, 2);
}

// --- Tracker (IO'suz karar çekirdeği) ---

fn cam0() -> PathBuf {
    PathBuf::from("/dev/video0")
}

fn scan_of(users: &[&str], per: &[(&Path, u32)], suspects: u32) -> Scan {
    let per_device: HashMap<PathBuf, u32> = per.iter().map(|(p, n)| (p.to_path_buf(), *n)).collect();
    let fds = per_device.values().sum();
    Scan { users: users.iter().map(|u| u.to_string()).collect(), fds, per_device, unreadable: suspects, suspects, partial: false }
}

/// Bir tur: tarama öncesi olaylar uygulanmış; `during` tarama sırasında gelen olaylar.
fn cycle(t: &mut Tracker, scan: &Scan, during: &[(i32, NodeEvent)]) -> CameraState {
    t.begin_scan();
    for (wd, e) in during {
        t.event(*wd, *e);
    }
    t.reconcile(scan);
    t.state(scan)
}

#[test]
fn balance_rules() {
    let mut t = Tracker::default();
    let v1 = PathBuf::from("/dev/video1");
    t.watch(1, &cam0());
    t.watch(2, &v1);
    let base = scan_of(&["obs"], &[(&v1, 2)], 0);
    assert_eq!(cycle(&mut t, &base, &[]), CameraState { users: vec!["obs".into()], unattributed: false });
    assert_eq!((t.nodes[&1].open, t.nodes[&2].open), (0, 2), "başlangıç dengesi taramadan");
    t.event(1, NodeEvent::Open);
    assert!(t.state(&base).unattributed, "taramada sahibi yok");
    t.event(1, NodeEvent::Close);
    t.event(1, NodeEvent::Close);
    assert_eq!(t.nodes[&1].open, 0, "çift kapanış eksiye inmez");
    assert_eq!(t.excess_closes, 1, "kapanış fazlası sayıldı");
    assert!(t.rescan, "kapanış fazlası yeniden tarama ister");
    t.event(2, NodeEvent::Gone);
    assert!(!t.nodes.contains_key(&2), "aygıt çıkarıldı");
    t.event(99, NodeEvent::Open);
    assert!(!t.nodes.contains_key(&99), "izlenmeyen wd yok sayılır");
    assert!(!t.state(&Scan::default()).in_use());
    t.overflow();
    assert!(t.state(&Scan::default()).unattributed, "taşma: fail-closed");
}

/// Okunamayan süreç binlerce kez açıp biri dışında kapatır: denge kırpılmaz, gösterge açık kalır.
#[test]
fn open_flood_cannot_reset_balance() {
    let mut t = Tracker::default();
    t.watch(1, &cam0());
    let empty = scan_of(&[], &[], 1);
    cycle(&mut t, &empty, &[]);
    for _ in 0..10_000 {
        t.event(1, NodeEvent::Open);
    }
    for _ in 0..9_999 {
        t.event(1, NodeEvent::Close);
    }
    assert_eq!(t.nodes[&1].open, 1);
    assert!(cycle(&mut t, &empty, &[]).in_use(), "bir fd hâlâ açık: gösterge açık");
    t.event(1, NodeEvent::Close);
    assert!(!cycle(&mut t, &scan_of(&[], &[], 0), &[]).in_use(), "hepsi kapandı: söner");
}

/// Denge sayacı tavana dayanınca yapışkan işaret: kapanışlar dengeyi düşürse de gösterge açık;
/// yarışlı ya da şüpheli tarama işareti kaldırmaz, yalnız yarışsız tam tarama kaldırır.
#[test]
fn saturated_balance_is_sticky_until_clean_complete_scan() {
    let mut t = Tracker::default();
    t.watch(1, &cam0());
    let clean = scan_of(&[], &[], 0);
    cycle(&mut t, &clean, &[]);
    t.nodes.get_mut(&1).unwrap().open = u64::MAX - 1;
    t.event(1, NodeEvent::Open);
    assert!(!t.nodes[&1].saturated, "tavana kadar sayılır");
    t.event(1, NodeEvent::Open);
    assert!(t.nodes[&1].saturated && t.nodes[&1].open == u64::MAX, "tavanda doyar, sarmaz");
    for _ in 0..3 {
        t.event(1, NodeEvent::Close);
    }
    assert!(t.state(&clean).unattributed, "kapanışlar işareti kaldırmaz");
    let suspect = scan_of(&[], &[], 1);
    assert!(cycle(&mut t, &suspect, &[]).unattributed, "şüpheli tarama kaldırmaz");
    let partial = Scan { partial: true, ..scan_of(&[], &[], 0) };
    assert!(cycle(&mut t, &partial, &[]).unattributed, "yarım tarama kaldırmaz");
    assert!(cycle(&mut t, &clean, &[(1, NodeEvent::Open)]).unattributed, "yarışlı tarama kaldırmaz");
    assert!(t.nodes[&1].saturated);
    let holder = scan_of(&["obs"], &[(&cam0(), 1)], 0);
    let s = cycle(&mut t, &holder, &[]);
    assert!(!t.nodes[&1].saturated, "yarışsız tam tarama kaldırır");
    assert_eq!(t.nodes[&1].open, 1, "denge taramadan kurulur");
    assert_eq!(s, CameraState { users: vec!["obs".into()], unattributed: false });
    t.event(1, NodeEvent::Close);
    assert!(!cycle(&mut t, &clean, &[]).in_use(), "son kapanışla söner");
}

/// Tarama sırasında olay gelen düğümün başlangıç dengesi kurulmaz (yarış); bir sonraki sessiz
/// taramada kurulur ve bu arada gösterge eksik göstermez.
#[test]
fn racy_initial_scan_is_retried() {
    let mut t = Tracker::default();
    t.watch(1, &cam0());
    // Tarama "obs"u görmedi ama tarama sırasında açtı.
    let s1 = scan_of(&[], &[], 0);
    t.begin_scan();
    t.event(1, NodeEvent::Open);
    assert!(t.reconcile(&s1), "yerleşmedi, tekrar taranmalı");
    assert!(t.state(&s1).unattributed, "açma dengede: gösterge açık");
    let s2 = scan_of(&["obs"], &[(&cam0(), 1)], 0);
    assert!(!cycle_pending(&mut t, &s2));
    assert_eq!(t.nodes[&1].open, 1);
    assert!(t.nodes[&1].settled && !t.nodes[&1].uncertain);
}

fn cycle_pending(t: &mut Tracker, s: &Scan) -> bool {
    t.begin_scan();
    t.reconcile(s)
}

/// Kabuk başlamadan açık aygıt (okunabilir sahip) + kapanış + yeni açılış; sonra kabuk başlamadan
/// açık tutan **okunamayan** süreç: belirsizlik + şüpheli gösterge açık; şüpheli gidince tam
/// tarama dengeyi kurar ve söner.
#[test]
fn opened_before_start_then_close_and_reopen() {
    let mut t = Tracker::default();
    t.watch(1, &cam0());
    let held = scan_of(&["obs"], &[(&cam0(), 1)], 0);
    assert!(cycle(&mut t, &held, &[]).in_use());
    t.event(1, NodeEvent::Close);
    let empty = scan_of(&[], &[], 0);
    assert!(!cycle(&mut t, &empty, &[]).in_use(), "kapanınca söner");
    t.event(1, NodeEvent::Open);
    assert!(cycle(&mut t, &held, &[]).in_use(), "yeni açılış");
    t.event(1, NodeEvent::Close);
    assert!(!cycle(&mut t, &empty, &[]).in_use());

    // Okunamayan sahip izlemeden önce açmış: dengede yok, ama şüpheli varken belirsiz.
    let mut t = Tracker::default();
    t.watch(1, &cam0());
    let hidden = scan_of(&[], &[], 1);
    assert!(cycle(&mut t, &hidden, &[]).unattributed, "belirsiz denge + şüpheli");
    assert!(t.nodes[&1].uncertain);
    // Okunabilir biri açıp kapatır; okunamayanın izleme öncesi fd'si kapanır (denge 1 → 0) ama
    // ikinci okunamayan açmıştır: gösterge yine açık (kural 3).
    t.event(1, NodeEvent::Open);
    t.event(1, NodeEvent::Close);
    assert!(cycle(&mut t, &hidden, &[]).unattributed);
    // Şüpheli kalmadı: tam tarama dengeyi kurar, belirsizlik kalkar, söner.
    assert!(!cycle(&mut t, &empty, &[]).in_use());
    assert!(!t.nodes[&1].uncertain);
    // Belirsizlik kalktıktan sonra (kabuktan sonra başlayan) okunamayan süreç göstergeyi yakmaz.
    assert!(!cycle(&mut t, &hidden, &[]).in_use(), "tarayıcı sandbox'ı boşuna yakmaz");
    t.event(1, NodeEvent::Open);
    assert!(cycle(&mut t, &hidden, &[]).unattributed, "açarsa dengede görülür");
}

/// Düşmanca: aç-kapa seliyle kuyruk taşırılıp gösterge söndürülemez. Taşmadan sonra gösterge yalnız
/// yarışsız tam taramada dengeyi taramadan kurar; şüpheli, yarım ya da yarışlı taramada açık kalır.
#[test]
fn overflow_stays_on_until_clean_scan() {
    let v1 = PathBuf::from("/dev/video1");
    let mut t = Tracker::default();
    t.watch(1, &cam0());
    t.watch(2, &v1);
    cycle(&mut t, &scan_of(&[], &[(&v1, 3)], 0), &[]);
    t.overflow();
    let suspect = scan_of(&[], &[], 1);
    assert!(cycle(&mut t, &suspect, &[]).unattributed, "şüpheli varken taşma sürer");
    let partial = Scan { partial: true, ..Scan::default() };
    assert!(cycle(&mut t, &partial, &[]).unattributed, "yarım taramada taşma sürer");
    let clean = scan_of(&["obs"], &[(&cam0(), 1)], 0);
    assert!(cycle(&mut t, &clean, &[(2, NodeEvent::Close)]).unattributed, "yarışlı taramada sürer");
    assert!(cycle(&mut t, &clean, &[]) == CameraState { users: vec!["obs".into()], unattributed: false });
    assert!(!t.overflowed);
    assert_eq!((t.nodes[&1].open, t.nodes[&2].open), (1, 0), "denge taramadan kuruldu");
    // Taşma tarama sırasında olursa o tur hiçbir şey eşitlenmez.
    t.begin_scan();
    t.overflow();
    t.reconcile(&scan_of(&[], &[], 0));
    assert!(t.overflowed && t.nodes[&1].open == 1);
}

/// Aygıt çıkarılıp takılır: eski düğüm düşer, yeni düğümün (yeni inode, yeni wd) dengesi taramayla
/// kurulur.
#[test]
fn replug_rebuilds_from_scan() {
    let mut t = Tracker::default();
    t.watch(1, &cam0());
    t.event(1, NodeEvent::Open);
    cycle(&mut t, &scan_of(&[], &[], 0), &[]);
    t.event(1, NodeEvent::Close);
    t.event(1, NodeEvent::Gone);
    t.watch(7, &cam0());
    let held = scan_of(&["cheese"], &[(&cam0(), 1)], 0);
    assert_eq!(cycle(&mut t, &held, &[]), CameraState { users: vec!["cheese".into()], unattributed: false });
    assert_eq!(t.nodes.keys().copied().collect::<Vec<_>>(), [7]);
    assert_eq!(t.nodes[&7].open, 1);
    // Yeniden adlandırma: aynı wd yeni yol; denge korunur.
    let v5 = PathBuf::from("/dev/video5");
    t.watch(7, &v5);
    assert_eq!((t.nodes[&7].open, &t.nodes[&7].path), (1, &v5));
}

// --- Özellik testi: rastgele olay dizisinde gösterge gerçeği hiç eksik göstermez ---

/// xorshift64* (bağımlılıksız, tohumla tekrarlanabilir).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

#[derive(Debug)]
struct SimNode {
    path: PathBuf,
    /// Takılı (`/dev`'de görünür). Çıkarılan düğümün açık fd'leri kapanana dek inode yaşar.
    live: bool,
    /// Çekirdekteki izleme (IN_IGNORED gelene dek).
    wd: Option<i32>,
}

#[derive(Debug)]
struct SimProc {
    /// fd dizini okunabilir; okunamayanlar aygıtı açabilen şüphelilerdir (root dışı).
    readable: bool,
    /// Açık fd'lerin düğüm indisleri (her biri ayrı dosya açıklaması).
    fds: Vec<usize>,
}

struct Sim {
    rng: Rng,
    nodes: Vec<SimNode>,
    procs: Vec<SimProc>,
    queue: Vec<(i32, NodeEvent)>,
    next_wd: i32,
    tracker: Tracker,
}

impl Sim {
    fn emit(&mut self, node: usize, event: NodeEvent) {
        if let Some(wd) = self.nodes[node].wd {
            self.queue.push((wd, event));
        }
    }

    /// Çıkarılmış düğümün son fd'si kapandı: inode düşer, izleme biter.
    fn maybe_gone(&mut self, node: usize) {
        if !self.nodes[node].live && !self.procs.iter().any(|p| p.fds.contains(&node)) {
            self.emit(node, NodeEvent::Gone);
            self.nodes[node].wd = None;
        }
    }

    fn close(&mut self, p: usize, i: usize) {
        let node = self.procs[p].fds.swap_remove(i);
        self.emit(node, NodeEvent::Close);
        self.maybe_gone(node);
    }

    /// `during`: tarama sırasında (takma yok: `/dev` CREATE olayı zaten bir sonraki turu uyandırır,
    /// o turda düğüm izlenir).
    fn op(&mut self, during: bool) {
        match self.rng.below(100) {
            0..10 if self.procs.len() < 6 => {
                let readable = self.rng.chance(70);
                self.procs.push(SimProc { readable, fds: Vec::new() });
            }
            10..40 => {
                let live: Vec<usize> = (0..self.nodes.len()).filter(|&n| self.nodes[n].live).collect();
                if !live.is_empty() && !self.procs.is_empty() {
                    let p = self.rng.below(self.procs.len());
                    let n = live[self.rng.below(live.len())];
                    self.procs[p].fds.push(n);
                    self.emit(n, NodeEvent::Open);
                }
            }
            40..70 => {
                let holders: Vec<usize> = (0..self.procs.len()).filter(|&p| !self.procs[p].fds.is_empty()).collect();
                if !holders.is_empty() {
                    let p = holders[self.rng.below(holders.len())];
                    let i = self.rng.below(self.procs[p].fds.len());
                    self.close(p, i);
                }
            }
            70..78 if !self.procs.is_empty() => {
                // Süreç çıkar: tüm fd'leri kapanır.
                let p = self.rng.below(self.procs.len());
                while !self.procs[p].fds.is_empty() {
                    self.close(p, 0);
                }
                self.procs.swap_remove(p);
            }
            78..84 if !during && self.nodes.iter().filter(|n| n.live).count() < 3 => {
                // Takma: boş bir videoN yolu (çıkarılmışın yolu yeniden kullanılabilir).
                let path = (0..4).map(|i| PathBuf::from(format!("/dev/video{i}"))).find(|p| !self.nodes.iter().any(|n| n.live && &n.path == p));
                if let Some(path) = path {
                    self.nodes.push(SimNode { path, live: true, wd: None });
                }
            }
            84..88 => {
                let live: Vec<usize> = (0..self.nodes.len()).filter(|&n| self.nodes[n].live).collect();
                if !live.is_empty() {
                    let n = live[self.rng.below(live.len())];
                    self.nodes[n].live = false;
                    self.maybe_gone(n);
                }
            }
            90 if !self.procs.is_empty() && self.rng.chance(40) => {
                // Sayaç seli: eski 4096 sınırının ötesinde aç, birkaçı dışında kapat (kırpılan
                // denge sıfıra inip göstergeyi fd'ler açıkken söndürüyordu).
                let live: Vec<usize> = (0..self.nodes.len()).filter(|&n| self.nodes[n].live).collect();
                if !live.is_empty() {
                    let p = self.rng.below(self.procs.len());
                    let n = live[self.rng.below(live.len())];
                    let burst = 4097 + self.rng.below(600);
                    let start = self.procs[p].fds.len();
                    for _ in 0..burst {
                        self.procs[p].fds.push(n);
                        self.emit(n, NodeEvent::Open);
                    }
                    for _ in 0..burst - 1 - self.rng.below(2) {
                        self.close(p, start);
                    }
                }
            }
            88..90 if !self.queue.is_empty() => {
                // Kuyruk taşması: rastgele olaylar kaybolur, taşma işareti gelir.
                let keep = self.rng.below(self.queue.len());
                self.queue.truncate(keep);
                self.queue.push((-1, NodeEvent::Open));
            }
            _ => {}
        }
    }

    fn deliver(&mut self) {
        for (wd, e) in std::mem::take(&mut self.queue) {
            if wd == -1 {
                self.tracker.overflow();
            } else {
                self.tracker.event(wd, e);
            }
        }
    }

    /// Gerçek tarama: yalnız okunabilir süreçlerin takılı düğümlerdeki fd'leri (silinmiş düğümün
    /// bağlantısı `video0 (deleted)` olur, sayılmaz).
    fn scan(&self) -> Scan {
        let mut out = Scan::default();
        let mut users = BTreeSet::new();
        for (i, p) in self.procs.iter().enumerate() {
            if !p.readable {
                out.unreadable += 1;
                out.suspects += 1;
                continue;
            }
            for &n in p.fds.iter().filter(|&&n| self.nodes[n].live) {
                *out.per_device.entry(self.nodes[n].path.clone()).or_insert(0) += 1;
                out.fds += 1;
                users.insert(format!("p{i}"));
            }
        }
        out.users = users.into_iter().collect();
        out
    }

    fn truth(&self) -> bool {
        self.procs.iter().any(|p| p.fds.iter().any(|&n| self.nodes[n].live))
    }

    /// Servis turu (`run` ile aynı sıra): bekleyen olaylar, izlemeler, tarama, tarama sırasında
    /// olanlar, eşitleme, karar.
    fn cycle(&mut self, ops_during: usize) -> CameraState {
        self.deliver();
        for n in 0..self.nodes.len() {
            if self.nodes[n].live && self.nodes[n].wd.is_none() {
                self.nodes[n].wd = Some(self.next_wd);
                self.tracker.watch(self.next_wd, &self.nodes[n].path);
                self.next_wd += 1;
            }
        }
        let seen: Vec<i32> = self.nodes.iter().filter(|n| n.live).filter_map(|n| n.wd).collect();
        self.tracker.retain(&seen);
        self.tracker.begin_scan();
        let found = self.scan();
        for _ in 0..ops_during {
            self.op(true);
        }
        self.deliver();
        self.tracker.reconcile(&found);
        self.tracker.state(&found)
    }
}

#[test]
fn indicator_never_under_reports_random_sequences() {
    let mut quiet_offs = 0;
    for seed in 1..=400u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        // Başlangıç: kabuk başlamadan açık tutulan aygıtlar (okunabilir ve okunamayan sahipler).
        let mut nodes = vec![SimNode { path: cam0(), live: true, wd: None }];
        if rng.chance(50) {
            nodes.push(SimNode { path: "/dev/video1".into(), live: true, wd: None });
        }
        let mut procs = Vec::new();
        for _ in 0..rng.below(4) {
            let fds = (0..rng.below(3)).map(|_| rng.below(nodes.len())).collect();
            procs.push(SimProc { readable: rng.chance(70), fds });
        }
        let mut sim = Sim { rng, nodes, procs, queue: Vec::new(), next_wd: 1, tracker: Tracker::default() };
        for step in 0..300 {
            for _ in 0..sim.rng.below(4) {
                sim.op(false);
            }
            let during = if sim.rng.chance(30) { sim.rng.below(3) + 1 } else { 0 };
            let state = sim.cycle(during);
            assert!(!sim.truth() || state.in_use(), "seed {seed} adım {step}: kamera kullanımda ama gösterge kapalı: {state:?}");
        }
        // Canlılık: herkes kapatır, okunamayan süreç kalmaz, sessiz tur → gösterge söner (takılı
        // kalmaz).
        while let Some(p) = sim.procs.iter().position(|p| !p.fds.is_empty()) {
            sim.close(p, 0);
        }
        sim.procs.retain(|p| p.readable);
        sim.cycle(0);
        let state = sim.cycle(0);
        assert!(!state.in_use(), "seed {seed}: herkes kapattıktan sonra gösterge takılı kaldı: {state:?} {:?} {:?}", sim.tracker, sim.nodes);
        assert!(!sim.tracker.overflowed);
        quiet_offs += 1;
    }
    assert_eq!(quiet_offs, 400);
}

// --- Uçtan uca (gerçek inotify, sahte procfs) ---

fn idle() -> CameraEvent {
    CameraEvent::State(CameraState::default())
}

fn used(name: &str) -> CameraEvent {
    CameraEvent::State(CameraState { users: vec![name.into()], unattributed: false })
}

fn spawn(dev: &Path, proc: &Path, recheck: Duration) -> (CameraService, mpsc::Receiver<CameraEvent>) {
    let (tx, rx) = mpsc::channel();
    let options = CameraOptions { dev_dir: dev.into(), proc_dir: proc.into(), settle: Duration::from_millis(20), recheck };
    let svc = CameraService::spawn(options, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    (svc, rx)
}

const WAIT: Duration = Duration::from_secs(5);

/// Uçtan uca: düğüm açılınca/kapanınca inotify uyanır, tarama sonucu değişince olay gelir.
#[test]
fn service_reacts_to_open_and_close() {
    let dev = tempfile::tempdir().unwrap();
    let proc = tempfile::tempdir().unwrap();
    let cam = dev.path().join("video0");
    std::fs::write(&cam, b"").unwrap();
    let (svc, rx) = spawn(dev.path(), proc.path(), Duration::from_secs(5));
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), idle());
    // Bir "uygulama" aygıtı açar: sahte procfs'e bağlantı, sonra gerçek open (inotify).
    fake_proc(proc.path(), 77, "zoom", &[&cam]);
    let f = std::fs::File::open(&cam).unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), used("zoom"));
    std::fs::remove_dir_all(proc.path().join("77")).unwrap();
    drop(f);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), idle());
    // Sonradan takılan aygıt da izlenir.
    let cam2 = dev.path().join("video2");
    std::fs::write(&cam2, b"").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    fake_proc(proc.path(), 78, "cheese", &[&cam2]);
    let f = std::fs::File::open(&cam2).unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), used("cheese"));
    drop(f);
    let started = Instant::now();
    drop(svc);
    assert!(started.elapsed() < Duration::from_secs(1), "hızlı durur");
}

/// Düşmanca: sahibi taramada görünmeyen bir açma göstergeyi söndürmez (fail-closed:
/// `unattributed`); düğüm açıkken yeniden adlandırılsa da denge aynı inode'da kalır; kapanınca söner.
#[test]
fn unattributed_and_renamed_opens_fail_closed() {
    let dev = tempfile::tempdir().unwrap();
    let proc = tempfile::tempdir().unwrap();
    let cam = dev.path().join("video0");
    std::fs::write(&cam, b"").unwrap();
    let (svc, rx) = spawn(dev.path(), proc.path(), Duration::from_secs(5));
    let hidden = CameraEvent::State(CameraState { users: vec![], unattributed: true });
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), idle());
    // procfs'te iz yok: açan süreç okunamıyor.
    let f = std::fs::File::open(&cam).unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), hidden);
    // Yeniden adlandırma göstergeyi söndürmez.
    std::fs::rename(&cam, dev.path().join("video5")).unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(400)).is_err(), "durum değişmedi (hâlâ kullanımda)");
    // Kısa yoklama (aç-kapa) birleştirme içinde kalır; sonuç değişmez.
    drop(std::fs::File::open(dev.path().join("video5")).unwrap());
    assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
    drop(f);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), idle());
    drop(svc);
}

/// Kabuk başlamadan açılmış (okunabilen sahipli) kamera açılışta görünür; kapanınca söner; yeniden
/// açılınca yine görünür.
#[test]
fn opened_before_start_is_seen_and_cleared() {
    let dev = tempfile::tempdir().unwrap();
    let proc = tempfile::tempdir().unwrap();
    let cam = dev.path().join("video0");
    std::fs::write(&cam, b"").unwrap();
    fake_proc(proc.path(), 31, "obs", &[&cam]);
    let f = std::fs::File::open(&cam).unwrap();
    let (svc, rx) = spawn(dev.path(), proc.path(), Duration::from_secs(5));
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), used("obs"));
    std::fs::remove_dir_all(proc.path().join("31")).unwrap();
    drop(f);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), idle());
    fake_proc(proc.path(), 32, "cheese", &[&cam]);
    let f = std::fs::File::open(&cam).unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), used("cheese"));
    std::fs::remove_dir_all(proc.path().join("32")).unwrap();
    drop(f);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), idle());
    drop(svc);
}

/// Gösterge açıkken olay kaçsa da (burada: sahip procfs'ten kayboldu, olay yok) doğrulama taraması
/// atıfı günceller; denge > 0 olduğu için gösterge sönmez (bilinmeyen).
#[test]
fn recheck_while_in_use_keeps_fail_closed() {
    let dev = tempfile::tempdir().unwrap();
    let proc = tempfile::tempdir().unwrap();
    let cam = dev.path().join("video0");
    std::fs::write(&cam, b"").unwrap();
    let (svc, rx) = spawn(dev.path(), proc.path(), Duration::from_millis(200));
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), idle());
    fake_proc(proc.path(), 41, "zoom", &[&cam]);
    let f = std::fs::File::open(&cam).unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), used("zoom"));
    std::fs::remove_dir_all(proc.path().join("41")).unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), CameraEvent::State(CameraState { users: vec![], unattributed: true }));
    drop(f);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), idle());
    drop(svc);
}

/// Aygıt çıkarılıp yeniden takılır (gerçek inotify): yeni düğüm izlenir, dengesi taramayla kurulur.
#[test]
fn service_replug() {
    let dev = tempfile::tempdir().unwrap();
    let proc = tempfile::tempdir().unwrap();
    let cam = dev.path().join("video0");
    std::fs::write(&cam, b"").unwrap();
    let (svc, rx) = spawn(dev.path(), proc.path(), Duration::from_secs(5));
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), idle());
    std::fs::remove_file(&cam).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    // Yeniden takıldı ve takılır takılmaz okunabilir bir süreç açtı (izlemeden önce olabilir).
    std::fs::write(&cam, b"").unwrap();
    fake_proc(proc.path(), 51, "cheese", &[&cam]);
    let f = std::fs::File::open(&cam).unwrap();
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), used("cheese"));
    std::fs::remove_dir_all(proc.path().join("51")).unwrap();
    drop(f);
    assert_eq!(rx.recv_timeout(WAIT).unwrap(), idle());
    drop(svc);
}
