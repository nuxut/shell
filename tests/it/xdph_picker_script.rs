//! `dist/xdph/xdph-picker.sh`: xdph.conf'a Nuxut seçici bloğunun eklenmesi/kaldırılması.
//! Her test geçici `XDG_CONFIG_HOME` ile çalışır; kullanıcının dosyasına asla dokunulmaz.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::support::repo::repo_path;

const PICKER: &str = "/home/u/.local/lib/nuxut/nuxut-sharepicker";

struct Env {
    _dir: tempfile::TempDir,
    config: PathBuf,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        Self { config, _dir: dir }
    }

    fn file(&self) -> PathBuf {
        self.config.join("hypr/xdph.conf")
    }

    fn write(&self, content: &str) {
        fs::create_dir_all(self.config.join("hypr")).unwrap();
        fs::write(self.file(), content).unwrap();
    }

    fn read(&self) -> String {
        fs::read_to_string(self.file()).unwrap()
    }

    /// (çıkış kodu, stdout)
    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new("sh")
            .arg(repo_path("dist/xdph/xdph-picker.sh"))
            .args(args)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("HOME", self._dir.path())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.is_empty() || out.status.code() == Some(2), "stderr: {stderr}");
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn backups(&self) -> Vec<String> {
        fs::read_dir(self.config.join("hypr"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("xdph.conf.nuxut-yedek-"))
            .collect()
    }
}

fn block(path: &str) -> String {
    format!("screencopy {{\n    custom_picker_binary = {path}\n}}\n# <<< nuxut-sharepicker <<<\n")
}

#[test]
fn creates_file_when_missing_and_is_idempotent() {
    let e = Env::new();
    let (rc, out) = e.run(&["install", PICKER]);
    assert_eq!(rc, 0, "{out}");
    let c = e.read();
    assert!(c.starts_with("# >>> nuxut-sharepicker"), "{c}");
    assert!(c.ends_with(&block(PICKER)), "{c}");
    assert!(e.backups().is_empty(), "yeni dosyada yedek gerekmez");
    // İkinci çalıştırma: değişiklik yok, yedek yok.
    let (rc, out) = e.run(&["install", PICKER]);
    assert_eq!(rc, 0);
    assert!(out.contains("değişiklik yok"), "{out}");
    assert_eq!(e.read(), c);
    assert!(e.backups().is_empty());
}

#[test]
fn appends_to_user_file_with_backup_and_keeps_content() {
    let e = Env::new();
    let user = "# kullanıcının ayarı\nscreencopy {\n    max_fps = 30\n}\n\n\n";
    e.write(user);
    let (rc, out) = e.run(&["install", PICKER]);
    assert_eq!(rc, 0, "{out}");
    let c = e.read();
    assert!(c.starts_with("# kullanıcının ayarı\nscreencopy {\n    max_fps = 30\n}\n\n# >>> nuxut-sharepicker"), "{c}");
    let backups = e.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read_to_string(e.config.join("hypr").join(&backups[0])).unwrap(), user);
    // Yol değişince yalnızca blok güncellenir; idempotent.
    let (rc, _) = e.run(&["install", "/opt/nuxut/nuxut-sharepicker"]);
    assert_eq!(rc, 0);
    let c2 = e.read();
    assert!(c2.contains("custom_picker_binary = /opt/nuxut/nuxut-sharepicker"));
    assert!(!c2.contains(PICKER));
    assert_eq!(c2.matches("# >>> nuxut-sharepicker").count(), 1);
    assert!(c2.starts_with("# kullanıcının ayarı\nscreencopy {\n    max_fps = 30\n}\n\n# >>>"), "{c2}");
    // Kaldırma: kullanıcının içeriği geri gelir (sondaki boş satırlar hariç).
    let (rc, _) = e.run(&["uninstall"]);
    assert_eq!(rc, 0);
    assert_eq!(e.read(), "# kullanıcının ayarı\nscreencopy {\n    max_fps = 30\n}\n");
    let (rc, out) = e.run(&["uninstall"]);
    assert_eq!((rc, out.contains("değişiklik yok")), (0, true));
}

#[test]
fn uninstall_removes_file_that_only_had_our_block() {
    let e = Env::new();
    assert_eq!(e.run(&["install", PICKER]).0, 0);
    assert_eq!(e.run(&["uninstall"]).0, 0);
    assert!(!e.file().exists());
}

#[test]
fn respects_user_picker() {
    let e = Env::new();
    let user = "screencopy {\n    custom_picker_binary = /usr/bin/benim-secicim\n}\n";
    e.write(user);
    let (rc, out) = e.run(&["install", PICKER]);
    assert_eq!(rc, 3, "{out}");
    assert!(out.contains("sizin seçiciniz"), "{out}");
    assert_eq!(e.read(), user);
    assert!(e.backups().is_empty());
    // Kategori:anahtar yazımı da tanınır; yorum satırı sayılmaz.
    e.write("screencopy:custom_picker_binary = /x\n");
    assert_eq!(e.run(&["install", PICKER]).0, 3);
    e.write("# custom_picker_binary = /x\n");
    assert_eq!(e.run(&["install", PICKER]).0, 0);
}

#[test]
fn refuses_symlink_broken_markers_and_unsafe_path() {
    let e = Env::new();
    fs::create_dir_all(e.config.join("hypr")).unwrap();
    let target = e.config.join("dotfiles-xdph.conf");
    fs::write(&target, "screencopy {\n}\n").unwrap();
    std::os::unix::fs::symlink(&target, e.file()).unwrap();
    let (rc, out) = e.run(&["install", PICKER]);
    assert_eq!(rc, 3, "{out}");
    assert!(out.contains("custom_picker_binary = /home/u"), "elle ekleme yönergesi: {out}");
    assert_eq!(fs::read_to_string(&target).unwrap(), "screencopy {\n}\n");
    assert!(is_symlink(&e.file()));

    let e = Env::new();
    e.write("# >>> nuxut-sharepicker: x >>>\nscreencopy {\n");
    let (rc, out) = e.run(&["install", PICKER]);
    assert_eq!(rc, 3, "{out}");
    assert_eq!(e.read(), "# >>> nuxut-sharepicker: x >>>\nscreencopy {\n");
    assert_eq!(e.run(&["uninstall"]).0, 3);

    let e = Env::new();
    for bad in ["relative/path", "/a b/picker", "/x#y", "/x\ny"] {
        assert_eq!(e.run(&["install", bad]).0, 3, "{bad:?}");
    }
    assert!(!e.file().exists());
}

#[test]
fn plan_writes_nothing() {
    let e = Env::new();
    let (rc, out) = e.run(&["plan", PICKER]);
    assert_eq!(rc, 0);
    assert!(out.starts_with("[deneme]"), "{out}");
    assert!(!e.config.exists() || !e.file().exists());
    e.write("a = 1\n");
    let (rc, out) = e.run(&["plan", PICKER]);
    assert_eq!(rc, 0);
    assert!(out.contains("eklenirdi"), "{out}");
    assert_eq!(e.read(), "a = 1\n");
    assert_eq!(e.run(&[]).0, 2);
}

fn is_symlink(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}
