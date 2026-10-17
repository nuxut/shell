use std::os::unix::fs::PermissionsExt as _;
use std::sync::{Arc, Mutex};

use super::*;
use crate::safefs::Root;

/// Sahte araçların yazımı ile çalıştırılması (fork/exec) sıralı: başka bir test thread'i aracı yazarken
/// fork eden test yazma fd'sini exec'e dek miras alır, exec ETXTBSY ile düşer (yükte aralıklı hata).
static EXEC: Mutex<()> = Mutex::new(());

fn exec_lock() -> std::sync::MutexGuard<'static, ()> {
    EXEC.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Geçici kök + sahte systemctl/localectl/systemd-tmpfiles (çağrıları `log`'a yazar; `FAIL`
/// dosyasındaki alt dizgeyi içeren çağrı başarısız olur).
struct Sys {
    _t: tempfile::TempDir,
    root: PathBuf,
    bin: PathBuf,
    log: PathBuf,
    uid: u32,
}

fn write(p: &Path, s: impl AsRef<[u8]>, mode: u32) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
}

impl Sys {
    fn new() -> Self {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        let bin = t.path().join("bin");
        let log = t.path().join("calls.log");
        std::fs::create_dir_all(root.join("run")).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        let uid = rustix::process::getuid().as_raw();
        let serial = exec_lock();
        for tool in ["systemctl", "localectl", "systemd-tmpfiles"] {
            let body = format!(
                "#!/bin/sh\nline=\"{tool} $*\"\necho \"$line\" >> '{}'\nif [ -f '{}' ] && echo \"$line\" | grep -qF \"$(cat '{}')\"; then exit 1; fi\n",
                log.display(),
                t.path().join("FAIL").display(),
                t.path().join("FAIL").display()
            );
            write(&bin.join(tool), body, 0o755);
        }
        drop(serial);
        let s = Self { _t: t, root, bin, log, uid };
        s.file(GREETD_BIN, "x", 0o755);
        for f in REQUIRED {
            s.file(f, "x", 0o755);
        }
        s.file(PASSWD, &format!("root:x:0:0::/root:/bin/sh\ngreeter:x:964:964::/var/lib/greeter:/bin/false\nayse:x:{uid}:{uid}:Ayşe:/home/ayse:/bin/sh\n"), 0o644);
        s
    }

    fn file(&self, rel: &str, text: &str, mode: u32) {
        write(&self.root.join(rel), text, mode);
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }

    fn set_dm(&self, unit: &str) {
        let link = self.root.join(DM_LINK);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(format!("/usr/lib/systemd/system/{unit}"), &link).unwrap();
    }

    fn fail_on(&self, what: &str) {
        std::fs::write(self._t.path().join("FAIL"), what).unwrap();
    }

    fn calls(&self) -> Vec<String> {
        let c = std::fs::read_to_string(&self.log).unwrap_or_default();
        let _ = std::fs::remove_file(&self.log);
        c.lines().map(str::to_owned).collect()
    }

    /// İşlemi çalıştırır: (sonuç, olaylar).
    fn run(&self, f: impl FnOnce(&Greeter) -> bool) -> (bool, Vec<Event>) {
        let ops = Ops::new(Root::open(self.root.to_str().unwrap(), self.uid).unwrap());
        let bins = Bins::at(self.bin.to_str().unwrap(), vec![]);
        let (events, buf): (Events, Arc<Mutex<Vec<Event>>>) = Events::memory();
        let g = Greeter { ops: &ops, bins: &bins, events: &events, caller: Some(self.uid) };
        let ok = {
            let _serial = exec_lock();
            f(&g)
        };
        let ev = buf.lock().unwrap().clone();
        (ok, ev)
    }
}

fn apply_req() -> Request {
    Request { keymap: None, screen_reader: false, theme: false }
}

fn result(ev: &[Event]) -> (Outcome, Option<String>) {
    ev.iter()
        .find_map(|e| match e {
            Event::LoginResult { outcome, message } => Some((*outcome, message.clone())),
            _ => None,
        })
        .unwrap()
}

#[test]
fn apply_from_tty_and_revert() {
    let s = Sys::new();
    let (ok, ev) = s.run(|g| g.apply(&apply_req()));
    assert!(ok, "{ev:?}");
    assert_eq!(result(&ev).0, Outcome::Applied);
    assert_eq!(s.read(GREETD_CONFIG), TEMPLATE);
    let rec: Record = serde_json::from_str(&s.read("var/lib/nuxut/greeter/state.json")).unwrap();
    assert_eq!((rec.previous_dm.as_str(), rec.config_existed, rec.config_backup), ("tty", false, None));
    assert_eq!(s.calls(), ["systemd-tmpfiles --create /usr/lib/tmpfiles.d/nuxut-greeter.conf", "systemctl enable greetd.service"]);

    s.set_dm(GREETD_UNIT);
    let (_, ev) = s.run(|g| {
        g.status();
        true
    });
    assert!(ev.iter().any(|e| matches!(e, Event::LoginScreen { status } if status.active && status.previous.as_deref() == Some("tty"))));

    let (ok, ev) = s.run(|g| g.revert());
    assert!(ok, "{ev:?}");
    assert_eq!(s.calls(), ["systemctl disable greetd.service"], "tty: hiçbir DM etkinleştirilmez");
    assert!(!s.root.join(GREETD_CONFIG).exists(), "önceden yoktu: Nuxut'unki silinir");
    assert!(!s.root.join("var/lib/nuxut/greeter/state.json").exists());
    let (ok, ev) = s.run(|g| g.revert());
    assert!(!ok && result(&ev) == (Outcome::Skipped, Some("no-record".into())));
}

#[test]
fn apply_from_sddm_backs_up_and_restores() {
    let s = Sys::new();
    s.set_dm("sddm.service");
    s.file(GREETD_CONFIG, "[default_session]\ncommand = \"agreety\"\n", 0o644);
    let (ok, ev) = s.run(|g| g.apply(&apply_req()));
    assert!(ok, "{ev:?}");
    assert_eq!(s.calls()[1..], ["systemctl disable sddm.service", "systemctl enable greetd.service"]);
    let rec: Record = serde_json::from_str(&s.read("var/lib/nuxut/greeter/state.json")).unwrap();
    let backup = rec.config_backup.clone().unwrap();
    assert!(s.read(&backup).contains("agreety"));

    // İkinci apply (DM artık greetd): ilk kayıt korunur.
    s.set_dm(GREETD_UNIT);
    assert!(s.run(|g| g.apply(&apply_req())).0);
    let rec2: Record = serde_json::from_str(&s.read("var/lib/nuxut/greeter/state.json")).unwrap();
    assert_eq!(rec, rec2);
    s.calls();

    assert!(s.run(|g| g.revert()).0);
    assert_eq!(s.calls(), ["systemctl disable greetd.service", "systemctl enable sddm.service"]);
    assert!(s.read(GREETD_CONFIG).contains("agreety"), "greetd ayarı geri geldi");
}

#[test]
fn prerequisites_are_enforced_before_any_write() {
    let s = Sys::new();
    std::fs::remove_file(s.root.join(GREETD_BIN)).unwrap();
    let (ok, ev) = s.run(|g| g.apply(&apply_req()));
    assert!(!ok && result(&ev) == (Outcome::Skipped, Some("greetd-missing".into())));

    let s = Sys::new();
    s.file(PASSWD, "root:x:0:0::/root:/bin/sh\n", 0o644);
    let (_, ev) = s.run(|g| g.apply(&apply_req()));
    assert_eq!(result(&ev).1.as_deref(), Some("greeter-user-missing"));

    // Başkalarının yazabildiği giriş ekranı dosyası: kök olarak çalıştırılır, reddedilir.
    let s = Sys::new();
    s.file(REQUIRED[1], "x", 0o777);
    let (_, ev) = s.run(|g| g.apply(&apply_req()));
    assert_eq!(result(&ev).1.as_deref(), Some("untrusted-file:/usr/lib/nuxut/nuxut-greeter-session"));
    assert!(!s.root.join(GREETD_CONFIG).exists() && s.calls().is_empty(), "hiçbir şey yazılmadı/çalışmadı");

    // Bağ da reddedilir.
    let s = Sys::new();
    std::fs::remove_file(s.root.join(REQUIRED[0])).unwrap();
    std::os::unix::fs::symlink("/bin/sh", s.root.join(REQUIRED[0])).unwrap();
    assert!(!s.run(|g| g.apply(&apply_req())).0);
}

#[test]
fn failing_enable_rolls_back_and_reenables_previous() {
    let s = Sys::new();
    s.set_dm("gdm.service");
    s.file(GREETD_CONFIG, "eski\n", 0o644);
    s.fail_on("enable greetd");
    let (ok, ev) = s.run(|g| g.apply(&apply_req()));
    assert!(!ok);
    assert_eq!(result(&ev).0, Outcome::RolledBack);
    let calls = s.calls();
    assert_eq!(calls.last().unwrap(), "systemctl enable gdm.service", "{calls:?}");
    assert_eq!(s.read(GREETD_CONFIG), "eski\n", "ayar geri yazıldı");
    // Kayıt kalır (günlük): sonraki deneme ya da revert kullanır.
    assert!(s.root.join("var/lib/nuxut/greeter/state.json").exists());
}

#[test]
fn keymap_and_args() {
    let s = Sys::new();
    let req = Request { keymap: Some(Keymap::new("tr,us", "", "grp:alt_shift_toggle").unwrap()), screen_reader: false, theme: false };
    assert!(s.run(|g| g.apply(&req)).0);
    assert_eq!(s.calls().last().unwrap(), "localectl set-x11-keymap tr,us   grp:alt_shift_toggle", "düzen, model (boş), değişken (boş), seçenekler");

    let args = apply_args(&req);
    assert_eq!(crate::args::parse(&args), Ok(crate::args::Command::Greeter(Command::Apply(req))));
    let p = |v: &[&str]| crate::args::parse(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(p(&["greeter", "status"]), Ok(crate::args::Command::Greeter(Command::Status)));
    assert_eq!(p(&["greeter", "revert"]), Ok(crate::args::Command::Greeter(Command::Revert)));
    for bad in [
        &["greeter"][..],
        &["greeter", "apply", "--layout", "TR"],
        &["greeter", "apply", "--layout", "tr;x"],
        &["greeter", "apply", "--variant", "x"],
        &["greeter", "apply", "--layout", "us", "--layout", "tr"],
        &["greeter", "revert", "--no-theme"],
        &["greeter", "apply", "--path", "/etc/shadow"],
    ] {
        assert!(p(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn publish_copies_owned_files_and_rewrites_paths() {
    let s = Sys::new();
    let home = s.root.join("home/ayse");
    write(&home.join("Resimler/dag.png"), b"PNGDATA", 0o644);
    write(&home.join(".config/nuxut/shell.toml"), "[bar]\nheight = 40\n[wallpaper]\npath = \"/home/ayse/Resimler/dag.png\"\n", 0o644);
    write(&home.join(".config/nuxut/framework.toml"), "[theme]\nmode = \"dark\"\n", 0o644);
    write(&home.join(".config/nuxut/lock.toml"), "wallpaper = \"/home/ayse/Resimler/dag.png\"\nblur = 12\n", 0o644);
    write(&home.join(".local/state/nuxut/palette.toml"), "[meta]\nimage = \"/home/ayse/Resimler/dag.png\"\n[colors]\nprimary = \"#123456\"\n", 0o644);
    let req = Request { keymap: None, screen_reader: true, theme: true };
    let (ok, ev) = s.run(|g| g.apply(&req));
    assert!(ok, "{ev:?}");
    let out = s.root.join(PUBLISHED);
    assert_eq!(std::fs::read(out.join("wallpaper.png")).unwrap(), b"PNGDATA");
    let wall = format!("/{PUBLISHED}/wallpaper.png");
    let shell: toml::Table = s.read(&format!("{PUBLISHED}/shell.toml")).parse().unwrap();
    assert_eq!(shell["wallpaper"]["path"].as_str(), Some(wall.as_str()));
    assert!(shell.get("bar").is_none(), "yalnız duvar kağıdı");
    let lock: toml::Table = s.read(&format!("{PUBLISHED}/lock.toml")).parse().unwrap();
    assert!(lock.get("wallpaper").is_none() && lock["blur"].as_integer() == Some(12));
    let palette: toml::Table = s.read(&format!("{PUBLISHED}/palette.toml")).parse().unwrap();
    assert_eq!(palette["meta"]["image"].as_str(), Some(wall.as_str()));
    assert_eq!(s.read(&format!("{PUBLISHED}/greeter.toml")), "screen_reader = true\n");

    // Yeniden yayın: eski dosyalar kalmaz.
    write(&home.join(".config/nuxut/shell.toml"), "[wallpaper]\npath = \"nuxut:dalga\"\n", 0o644);
    assert!(s.run(|g| g.apply(&req)).0);
    assert!(!out.join("wallpaper.png").exists());
}

#[test]
fn publish_refuses_links_and_foreign_files() {
    let s = Sys::new();
    let home = s.root.join("home/ayse");
    // Duvar kağıdı kökün gizli bir dosyasına bağ: atlanır (uyarı), yayın sürer.
    s.file("etc/gizli.png", "GIZLI", 0o600);
    std::fs::create_dir_all(&home).unwrap();
    std::os::unix::fs::symlink(s.root.join("etc/gizli.png"), home.join("dag.png")).unwrap();
    write(&home.join(".config/nuxut/shell.toml"), "[wallpaper]\npath = \"/home/ayse/dag.png\"\n", 0o644);
    let req = Request { keymap: None, screen_reader: false, theme: true };
    let (ok, ev) = s.run(|g| g.apply(&req));
    assert!(ok, "{ev:?}");
    assert!(ev.iter().any(|e| matches!(e, Event::Warn { message, .. } if message == "wallpaper:symlink")), "{ev:?}");
    assert!(!s.root.join(PUBLISHED).join("wallpaper.png").exists());

    // Ayar dosyası bağsa uygulama geri alınır (yarım yayın yok).
    let s = Sys::new();
    let home = s.root.join("home/ayse");
    s.file("etc/gizli.toml", "a = 1\n", 0o600);
    std::fs::create_dir_all(home.join(".config/nuxut")).unwrap();
    std::os::unix::fs::symlink(s.root.join("etc/gizli.toml"), home.join(".config/nuxut/framework.toml")).unwrap();
    let (ok, ev) = s.run(|g| g.apply(&req));
    assert!(!ok);
    assert_eq!(result(&ev), (Outcome::RolledBack, Some("user-file:framework.toml:symlink".into())));
    assert!(!s.root.join(GREETD_CONFIG).exists(), "ayar geri alındı");
    assert!(!s.root.join(PUBLISHED).exists());

    // Başka uid'in dosyası: sahip tutmaz.
    let s = Sys::new();
    write(&s.root.join("home/ayse/.config/nuxut/framework.toml"), "[theme]\n", 0o644);
    let ops = Ops::new(Root::open(s.root.to_str().unwrap(), s.uid).unwrap());
    let bins = Bins::at(s.bin.to_str().unwrap(), vec![]);
    let (events, _buf) = Events::memory();
    let g = Greeter { ops: &ops, bins: &bins, events: &events, caller: Some(s.uid) };
    let err = g.publish(s.uid + 1, false, &mut Tx::default()).unwrap_err().to_string();
    assert!(err.contains("caller-home-missing"), "{err}");
    let path = s.root.join("home/ayse/.config/nuxut/framework.toml");
    assert_eq!(read_user_file(&path, s.uid + 1, 1024), Err(UserFileError::Foreign));
    std::fs::write(&path, "x".repeat(300 * 1024)).unwrap();
    assert_eq!(read_user_file(&path, s.uid, MAX_TOML), Err(UserFileError::TooLarge));
}

#[test]
fn helpers() {
    assert!(valid_unit("sddm.service") && valid_unit("lightdm.service") && valid_unit(&format!("getty{}tty1.service", '@')));
    assert!(!valid_unit("../x.service") && !valid_unit("sddm") && !valid_unit("Sddm.service"));
    let passwd = "a:x:1000:1000::/home/a:/bin/sh\nb:x:1001:1001::rel:/bin/sh\nc:x:1002:1002::/home/../etc:/bin/sh\n";
    assert_eq!(home_of(passwd, 1000).as_deref(), Some("/home/a"));
    assert_eq!(home_of(passwd, 1001), None, "göreli ev dizini");
    assert_eq!(home_of(passwd, 1002), None, "'..' içeren ev dizini");
    for bad in ["", "TR", "tr us", "tr;rm", &"a".repeat(65)] {
        assert!(Keymap::new(bad, "", "").is_err(), "{bad}");
    }
}
