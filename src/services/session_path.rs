//! Oturum PATH'inin kendini onarması (kullanıcı isteği 2026-10-17).
//!
//! Kalıcı çözüm Nuxut'un Hyprland katmanındadır (`nuxut.conf`/`nuxut.lua`,
//! [`crate::services::hypr_config::generate::GenerateInput::session_path`]); katman bu oturumda
//! yüklenmemişse (ilk kurulumdan sonraki ilk oturum, dahil etme satırı yok) kabuk açılışta
//! kurulum dizinini çalışan Hyprland'in ortamına IPC ile ekler — `install.sh` ile aynı yöntem:
//! Lua ayarında `eval` (Hyprland'in kendi PATH'i okunur, varsa eklenmez), hyprlang'da `keyword
//! envd` (`$PATH` Hyprland'in ortamından açılır; katmanın oturumda-bir-kez işareti de konur ki
//! ayar yeniden okununca dizin ikinci kez eklenmesin). Her iki yol systemd kullanıcı yöneticisine
//! ve D-Bus etkinleştirme ortamına da aktarır. Hyprland'in ve kabuğun başlattığı yeni süreçler
//! (yeni terminaller) dizini hemen görür; açık terminaller görmez — doctor bunu söyler.
//!
//! Güvenlik (güvenlik incelemesi 2026-10-17, güvenilmeyen arama yolu):
//! - Dizin PATH'in SONUNA eklenir: `nuxut-*` adları benzersizdir; başa eklenseydi kullanıcının
//!   yazabildiği dizindeki bir dosya `ls`, `sudo` gibi sistem komutlarının önüne geçerdi.
//! - Yalnız güvenilir dizin: parola veritabanındaki ev dizininin `.local/bin`'i (ortam
//!   değişkeni değil) ve kabuk oradan çalışıyorsa. Dizin sembolik bağ değil, sahibi kullanıcı,
//!   grup/başkaları yazamaz; üst dizinleri kullanıcıya ya da root'a ait ve grup/başkaları
//!   yazamaz ([`dir_trusted`]). Denetim geçmezse hiçbir şey eklenmez.
//! - Hyprland'e giden değer yalnız yalın yol ([`safe_path_value`]): virgül, `$`, `:`, satır sonu,
//!   tırnak yok — `keyword`/`eval` enjeksiyonu yok.
//!
//! `/proc/<pid>/environ` sürecin başlangıç ortamıdır, sonradan eklenen PATH orada görünmez:
//! uygulanınca `$XDG_RUNTIME_DIR/nuxut/session-path-<imza>` işareti yazılır (tmpfs, oturumla
//! silinir). Kabuk yeniden başlarsa ikinci kez uygulamaz; doctor işareti okur.
//!
//! Yalnızca asıl örnek, kullanıcının Hyprland'inde çalışır (test örneği hiçbir şey göndermez).
//! Bloklayan IO ayrı thread'de.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use crate::services::hypr_config::control::HyprControl;
use crate::services::hypr_config::generate::SESSION_PATH_MARKER;
use crate::services::magnifier::{PROVIDER_COMMAND, Provider, parse_provider};

/// İşaret dosyasının adı öneki (`<önek><imza>`).
const MARKER_PREFIX: &str = "session-path-";

/// Oturum PATH'ine hiç eklenmeyen sistem dizinleri (zaten PATH'tedir).
const SYSTEM_BIN_DIRS: [&str; 5] = ["/usr/bin", "/bin", "/usr/local/bin", "/usr/sbin", "/sbin"];

/// `/etc/passwd` okuma sınırı.
const MAX_PASSWD: u64 = 1 << 20;

/// Hyprland'e (`keyword`, `eval`, `nuxut.conf`/`nuxut.lua`) ve environment.d'ye yazılabilecek
/// dizin: mutlak, yalnız `[A-Za-z0-9/._+-]` (virgül, `$`, `:`, boşluk, tırnak, satır sonu yok),
/// `.`/`..` bileşeni yok, en çok 512 bayt, sistem dizini değil.
pub fn safe_path_value(dir: &str) -> bool {
    dir.starts_with('/')
        && dir.len() <= 512
        && dir.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._+-".contains(&b))
        && Path::new(dir).components().all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
        && !dir.split('/').any(|s| s == "." || s == "..")
        && !SYSTEM_BIN_DIRS.contains(&dir.trim_end_matches('/'))
}

/// Parola veritabanı metninden `uid`'in ev dizini (mutlak değilse `None`).
pub fn passwd_home(text: &str, uid: u32) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        (f.len() >= 7 && f[2].parse::<u32>().ok() == Some(uid)).then(|| PathBuf::from(f[5])).filter(|h| h.is_absolute())
    })
}

/// Güven denetimi: `dir` sembolik bağ olmayan bir dizin, sahibi `uid`, grup/başkaları yazamaz;
/// `root`'a kadar her üst dizin `uid`'e ya da root'a ait ve grup/başkaları yazamaz (`install.sh`
/// `path_trusted` ile aynı kural). Gerçek kullanımda `root` = `/`; testler geçici kök verir.
pub fn dir_trusted(dir: &Path, uid: u32, root: &Path) -> bool {
    let private = |m: &std::fs::Metadata| m.mode() & 0o022 == 0;
    let Ok(meta) = std::fs::symlink_metadata(dir) else { return false };
    if meta.file_type().is_symlink() || !meta.is_dir() || meta.uid() != uid || !private(&meta) {
        return false;
    }
    let mut cur = dir.parent();
    while let Some(p) = cur {
        let Ok(m) = std::fs::metadata(p) else { return false };
        if !(m.uid() == uid || m.uid() == 0) || !private(&m) {
            return false;
        }
        if p == root {
            return true;
        }
        cur = p.parent();
    }
    root == Path::new("/")
}

/// Oturum PATH'ine eklenecek güvenilir dizin: `<ev>/.local/bin`, yalnız kabuk oradan çalışıyorsa
/// (`exe_dir` yalnız karşılaştırılır, kaynak değildir) ve [`dir_trusted`] + [`safe_path_value`]
/// geçerse.
pub fn trusted_bin_dir_in(home: &Path, uid: u32, exe_dir: Option<&Path>, root: &Path) -> Option<String> {
    let dir = home.join(".local/bin");
    let s = dir.to_str()?.to_owned();
    let same = exe_dir.is_some_and(|e| e == dir || dir.canonicalize().is_ok_and(|c| c == e));
    (same && safe_path_value(&s) && dir_trusted(&dir, uid, root)).then_some(s)
}

/// [`trusted_bin_dir_in`] gerçek değerlerle: ev dizini `/etc/passwd`'den (`$HOME` değil), uid
/// süreçten, kabuğun dizini `current_exe`'den (yalnız karşılaştırma).
pub fn trusted_bin_dir() -> Option<String> {
    let uid = rustix::process::getuid().as_raw();
    let mut text = String::new();
    std::fs::File::open("/etc/passwd").ok()?.take(MAX_PASSWD).read_to_string(&mut text).ok()?;
    let home = passwd_home(&text, uid)?;
    trusted_bin_dir_in(&home, uid, crate::self_exe::exe_dir().as_deref(), Path::new("/"))
}

/// Uygulandığını kaydeden işaret: `$XDG_RUNTIME_DIR/nuxut/session-path-<imza>`. Yalnızca güvenli
/// imza (Hyprland örnek adı biçimi) ve mutlak çalışma dizini.
pub fn marker_path(runtime_dir: &Path, signature: &str) -> Option<PathBuf> {
    let ok = !signature.is_empty()
        && signature.len() <= 128
        && signature.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    (ok && runtime_dir.is_absolute()).then(|| runtime_dir.join("nuxut").join(format!("{MARKER_PREFIX}{signature}")))
}

/// Onarımın sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Hyprland zaten bu dizinle başlamış.
    Present,
    /// Bu Hyprland örneğinde daha önce uygulanmış (işaret var).
    AlreadyApplied,
    /// Şimdi uygulandı.
    Applied,
    /// Hyprland reddetti ya da yanıt vermedi (kısaltılmış neden).
    Failed(String),
}

impl Outcome {
    /// Dizin artık oturumun PATH'inde mi?
    pub fn in_session(&self) -> bool {
        !matches!(self, Self::Failed(_))
    }
}

/// Hyprland'e gidecek komutlar; dizin PATH'in SONUNA eklenir. [`safe_path_value`] geçmezse `None`
/// (Lua dizgesine ve hyprlang değerine kaçış gerekmeyen biçim, `install.sh` ile aynı sınır).
pub fn commands(provider: Provider, dir: &str) -> Option<Vec<String>> {
    if !safe_path_value(dir) {
        return None;
    }
    Some(match provider {
        Provider::Lua => vec![format!(
            "eval local p = os.getenv(\"PATH\") or \"\"; if not (\":\" .. p .. \":\"):find(\":{dir}:\", 1, true) then hl.env(\"PATH\", p .. \":{dir}\", true) end"
        )],
        Provider::Hyprlang => vec![format!("keyword envd PATH,$PATH:{dir}"), format!("keyword env {SESSION_PATH_MARKER},1")],
    })
}

/// Onarım adımı (yan etkisiz karar + IPC). `hypr_path`: Hyprland'in başlangıç PATH'i (okunamazsa
/// `None`, yine uygulanır: Lua yolu kendisi denetler, hyprlang'da işaret tekrarı önler).
pub fn heal(ctl: &dyn HyprControl, dir: &str, hypr_path: Option<&OsStr>, marker: Option<&Path>) -> Outcome {
    if crate::self_exe::dir_in_path(Path::new(dir), hypr_path) {
        return Outcome::Present;
    }
    if marker.is_some_and(Path::exists) {
        return Outcome::AlreadyApplied;
    }
    if !safe_path_value(dir) {
        return Outcome::Failed("dizin yolu güvenli değil".into()); // i18n: tanı log'u (çevrilmez)
    }
    let provider = match ctl.request(PROVIDER_COMMAND) {
        Ok(text) => parse_provider(&text).unwrap_or(Provider::Hyprlang),
        Err(e) => return Outcome::Failed(short(&e)),
    };
    let cmds = commands(provider, dir).unwrap_or_default();
    // İlk komut PATH'i koyar ve yanıtı "ok" olmalı; hyprlang'daki işaret en iyi çabayla.
    for (i, cmd) in cmds.iter().enumerate() {
        match ctl.request(cmd) {
            Ok(reply) if reply.trim() == "ok" => {}
            Ok(_) | Err(_) if i > 0 => {}
            Ok(reply) => return Outcome::Failed(short(&reply)),
            Err(e) => return Outcome::Failed(short(&e)),
        }
    }
    if let Some(m) = marker {
        if let Some(parent) = m.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(m, b"1\n") {
            tracing::debug!("oturum PATH işareti yazılamadı ({}): {e}", m.display());
        }
    }
    Outcome::Applied
}

fn short(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// Kabuğun başlattığı uygulamalara verilecek PATH (dizin oturuma eklendiyse ve kabuğun kendi
/// başlangıç PATH'inde yoksa). Başlatıcıdan açılan yeni terminal de dizini görür.
static CHILD_PATH: OnceLock<OsString> = OnceLock::new();

/// [`CHILD_PATH`]; ayarlanmadıysa `None` (alt süreç kabuğun ortamını olduğu gibi alır).
pub fn child_path() -> Option<&'static OsStr> {
    CHILD_PATH.get().map(OsString::as_os_str)
}

/// `dir`'i `path`'in SONUNA ekler (zaten varsa `None`): sistem komutları önce bulunur.
pub fn append(dir: &Path, path: Option<&OsStr>) -> Option<OsString> {
    if crate::self_exe::dir_in_path(dir, path) {
        return None;
    }
    let head = path.map(std::env::split_paths).into_iter().flatten();
    std::env::join_paths(head.chain(std::iter::once(dir.to_path_buf()))).ok()
}

/// Açılışta bir kez: ayrı thread'de onarır, sonucu log'a yazar. `allowed`: asıl örnek ve test
/// örneği değil (kullanıcının oturumu). Güvenilir dizin yoksa ([`trusted_bin_dir`]) hiçbir şey
/// gönderilmez.
pub fn start(allowed: bool) {
    if !allowed {
        crate::self_exe::warn_if_not_in_path();
        return;
    }
    let _ = std::thread::Builder::new().name("nuxut-session-path".into()).spawn(|| {
        match trusted_bin_dir() {
            Some(dir) => run(&dir),
            None => crate::self_exe::warn_if_not_in_path(),
        }
    });
}

fn run(dir: &str) {
    let Ok(paths) = crate::services::hyprland::HyprlandPaths::from_env() else { return };
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap_or_default();
    let marker = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .and_then(|r| marker_path(Path::new(&r), &signature));
    let hypr_path = crate::services::hyprland::compositor_pid().and_then(|pid| crate::self_exe::environ_path(Path::new("/proc"), pid));
    let client = crate::services::hyprland::HyprlandClient::new(paths.socket1).with_timeout(std::time::Duration::from_secs(2));
    let outcome = heal(&client, dir, hypr_path.as_deref(), marker.as_deref());
    match &outcome {
        Outcome::Present => {}
        Outcome::AlreadyApplied => tracing::debug!(dir, "kurulum dizini bu oturumun PATH'ine daha önce eklenmiş"),
        Outcome::Applied => tracing::info!(dir, "kurulum dizini bu oturumun PATH'ine eklendi; yeni açılan terminallerde etkin"),
        Outcome::Failed(why) => tracing::warn!(dir, "kurulum dizini oturumun PATH'ine eklenemedi: {why}"),
    }
    if outcome.in_session()
        && let Some(p) = append(Path::new(dir), std::env::var_os("PATH").as_deref())
    {
        let _ = CHILD_PATH.set(p);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// Sahte Hyprland: `j/status` ayar dilini verir, gönderilenleri kaydeder.
    struct Fake {
        provider: &'static str,
        reply: &'static str,
        down: bool,
        sent: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(provider: &'static str) -> Self {
            Self { provider, reply: "ok", down: false, sent: RefCell::default() }
        }
        fn writes(&self) -> Vec<String> {
            self.sent.borrow().iter().filter(|c| *c != PROVIDER_COMMAND).cloned().collect()
        }
    }

    impl HyprControl for Fake {
        fn request(&self, command: &str) -> Result<String, String> {
            if self.down {
                return Err("bağlanılamadı".into());
            }
            self.sent.borrow_mut().push(command.to_owned());
            Ok(if command == PROVIDER_COMMAND { format!("{{\"configProvider\": \"{}\"}}", self.provider) } else { self.reply.into() })
        }
    }

    const DIR: &str = "/home/u/.local/bin";

    #[test]
    fn lua_eval_checks_hyprland_path_itself() {
        let fake = Fake::new("lua");
        assert_eq!(heal(&fake, DIR, Some(OsStr::new("/usr/bin")), None), Outcome::Applied);
        let w = fake.writes();
        assert_eq!(w.len(), 1);
        assert!(w[0].starts_with("eval local p = os.getenv(\"PATH\")"), "{}", w[0]);
        assert!(w[0].contains(":find(\":/home/u/.local/bin:\", 1, true)"), "{}", w[0]);
        // Sona eklenir: sistem komutları önce bulunur.
        assert!(w[0].contains("hl.env(\"PATH\", p .. \":/home/u/.local/bin\", true)"), "{}", w[0]);
        assert!(!w[0].contains(['\n', '\r', '\0']));
    }

    #[test]
    fn hyprlang_uses_envd_and_layer_marker() {
        let fake = Fake::new("hyprlang");
        assert_eq!(heal(&fake, DIR, None, None), Outcome::Applied);
        assert_eq!(fake.writes(), ["keyword envd PATH,$PATH:/home/u/.local/bin", "keyword env NUXUT_PATH_ADDED,1"]);
    }

    #[test]
    fn present_or_marked_sends_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = Fake::new("lua");
        let path = std::env::join_paths(["/usr/bin", DIR]).unwrap();
        assert_eq!(heal(&fake, DIR, Some(&path), None), Outcome::Present);
        let marker = marker_path(tmp.path(), "abc_123").unwrap();
        // İlk tur uygular ve işaret yazar; ikinci tur (kabuk yeniden başladı) göndermez.
        assert_eq!(heal(&fake, DIR, None, Some(&marker)), Outcome::Applied);
        assert!(marker.is_file());
        let sent = fake.sent.borrow().len();
        assert_eq!(heal(&fake, DIR, None, Some(&marker)), Outcome::AlreadyApplied);
        assert_eq!(fake.sent.borrow().len(), sent);
    }

    #[test]
    fn failures_do_not_mark() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = marker_path(tmp.path(), "sig").unwrap();
        let mut down = Fake::new("lua");
        down.down = true;
        assert!(matches!(heal(&down, DIR, None, Some(&marker)), Outcome::Failed(_)));
        let mut refused = Fake::new("hyprlang");
        refused.reply = "invalid\nkeyword";
        assert_eq!(heal(&refused, DIR, None, Some(&marker)), Outcome::Failed("invalidkeyword".into()));
        assert_eq!(refused.writes().len(), 1, "PATH reddedilince işaret gönderilmez");
        assert!(!marker.exists());
        // Güvensiz yol hiçbir şey göndermez.
        let fake = Fake::new("lua");
        assert!(matches!(heal(&fake, "/home/u/a b\";os.exit()", None, None), Outcome::Failed(_)));
        assert!(fake.writes().is_empty());
    }

    /// Gerçek istemci, sahte Hyprland soketi: istek başına bir bağlantı (socket1 gibi).
    #[test]
    fn real_client_against_fake_socket() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;
        let tmp = tempfile::tempdir().unwrap();
        let sock = tmp.path().join(".socket.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let server = std::thread::spawn(move || {
            let mut got = Vec::new();
            for _ in 0..2 {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let reply = if req == PROVIDER_COMMAND { "{\"configProvider\": \"lua\"}" } else { "ok" };
                s.write_all(reply.as_bytes()).unwrap();
                got.push(req);
            }
            got
        });
        let client = crate::services::hyprland::HyprlandClient::new(sock).with_timeout(std::time::Duration::from_secs(5));
        let marker = marker_path(tmp.path(), "sig").unwrap();
        assert_eq!(heal(&client, DIR, Some(OsStr::new("/usr/bin")), Some(&marker)), Outcome::Applied);
        let got = server.join().unwrap();
        assert_eq!(got[0], PROVIDER_COMMAND);
        assert!(got[1].starts_with("eval ") && got[1].contains(DIR), "{}", got[1]);
        assert!(marker.is_file());
    }

    /// Hyprland değerine (keyword/eval) virgül, `$`, `:`, satır sonu, tırnak, `..` girmez.
    #[test]
    fn commands_reject_unsafe_dirs() {
        for bad in [
            "relative/bin",
            "/a;dispatch exec x",
            "/a,b",
            "/a$HOME",
            "/a:b",
            "/a\nkeyword x",
            "/a\"b",
            "/a b",
            "/a/../b",
            "/a/./b",
            "/usr/bin",
            "/bin",
            "",
        ] {
            assert!(commands(Provider::Lua, bad).is_none(), "{bad:?}");
            assert!(commands(Provider::Hyprlang, bad).is_none(), "{bad:?}");
            assert!(!safe_path_value(bad), "{bad:?}");
        }
        assert!(commands(Provider::Lua, "/opt/nuxut-1.0/bin").is_some());
        assert!(!safe_path_value(&format!("/{}", "a".repeat(600))));
    }

    #[test]
    fn passwd_home_by_uid() {
        let text = "root:x:0:0::/root:/bin/bash\nu:x:1000:1000:U:/home/u:/bin/fish\nbad:x:1001:1001::rel:/bin/sh\n";
        assert_eq!(passwd_home(text, 1000), Some(PathBuf::from("/home/u")));
        assert_eq!(passwd_home(text, 0), Some(PathBuf::from("/root")));
        assert_eq!(passwd_home(text, 1001), None, "göreli ev dizini");
        assert_eq!(passwd_home(text, 42), None);
    }

    /// Güven: sembolik bağ, başkasına ait, grup/başkaları yazabilen dizin ya da üst dizin reddedilir.
    #[test]
    fn trust_rules() {
        use std::os::unix::fs::PermissionsExt;
        let set = |p: &Path, mode: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        set(root, 0o700);
        let uid = rustix::process::getuid().as_raw();
        let home = root.join("home");
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        set(&home, 0o700);
        set(&home.join(".local"), 0o700);
        set(&bin, 0o755);
        let canon = |p: &Path| p.canonicalize().unwrap();
        let ok = |exe: &Path| trusted_bin_dir_in(&home, uid, Some(exe), root);
        assert_eq!(ok(&bin), bin.to_str().map(str::to_owned));
        assert!(ok(&canon(&bin)).is_some());
        // Kabuk başka yerden çalışıyorsa eklenmez (current_exe kaynak değil).
        assert!(ok(&root.join("elsewhere")).is_none());
        assert!(trusted_bin_dir_in(&home, uid, None, root).is_none());
        // Başka kullanıcı.
        assert!(trusted_bin_dir_in(&home, uid + 1, Some(&bin), root).is_none());
        // Grup/başkaları yazabilir: dizin ya da üst dizin.
        set(&bin, 0o775);
        assert!(ok(&bin).is_none());
        set(&bin, 0o757);
        assert!(ok(&bin).is_none());
        set(&bin, 0o755);
        set(&home.join(".local"), 0o770);
        assert!(ok(&bin).is_none(), "üst dizin grup yazılabilir");
        set(&home.join(".local"), 0o700);
        assert!(ok(&bin).is_some());
        // Sembolik bağ.
        let real = root.join("real");
        std::fs::create_dir(&real).unwrap();
        set(&real, 0o755);
        std::fs::remove_dir(&bin).unwrap();
        std::os::unix::fs::symlink(&real, &bin).unwrap();
        assert!(!dir_trusted(&bin, uid, root));
        assert!(ok(&bin).is_none());
        assert!(ok(&canon(&bin)).is_none());
        // Yok.
        std::fs::remove_file(&bin).unwrap();
        assert!(ok(&bin).is_none());
    }

    #[test]
    fn marker_path_validates_signature() {
        let r = Path::new("/run/user/1000");
        assert_eq!(marker_path(r, "a_b-1"), Some(PathBuf::from("/run/user/1000/nuxut/session-path-a_b-1")));
        assert!(marker_path(r, "../x").is_none());
        assert!(marker_path(r, "").is_none());
        assert!(marker_path(Path::new("rel"), "a").is_none());
    }

    #[test]
    fn append_only_when_missing() {
        let p = append(Path::new(DIR), Some(OsStr::new("/usr/bin"))).unwrap();
        assert_eq!(p, OsString::from("/usr/bin:/home/u/.local/bin"), "sona");
        assert!(append(Path::new(DIR), Some(&p)).is_none());
        assert_eq!(append(Path::new(DIR), None).unwrap(), OsString::from(DIR));
    }
}
