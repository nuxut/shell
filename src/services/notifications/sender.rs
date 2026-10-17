//! `Notify` çağıranının kimliği: görsel yalıtımının kaynak anahtarı (ADR-0007) buna bağlanır.
//!
//! İstemcinin bildirdiği `app_name` yalnızca görüntü içindir; anahtar olsaydı bir uygulama başka
//! bir uygulamanın adıyla bozuk görseller gönderip onun bildirim görsellerini susturtabilirdi.
//! Kimlik D-Bus'ın doğruladığı bilgiden türetilir: gönderenin benzersiz adı →
//! `GetConnectionUnixProcessID` → `/proc/<pid>/exe` + birleşik cgroup yolu. Aynı ikilinin aynı
//! cgroup'taki süreçleri (her seferinde yeni süreç açan `notify-send` gibi) ortak bir kaynaktır;
//! susturma ve hız sınırı yeni süreç açarak aşılmaz.
//!
//! Kapalı başarısızlık (fail-closed): kimlik çözülemezse (PID verilmedi, gönderen çoktan çıktı,
//! `/proc` okunamadı, eşler arası bağlantıda gönderen yok) istek ortak [`ANON_IDENTITY`] kovasına
//! düşer. Benzersiz ad yedek olsaydı saldırgan her istekte bağlantıyı hemen kapatan yeni bir süreç
//! açıp (her biri yeni ad) susturma, negatif önbellek ve hız sınırından kaçardı; ortak kovada tüm
//! çözülemeyenler birlikte susturulur. Bedeli: kimliği çözülemeyen meşru gönderen o kovayı
//! paylaşır (oturum bus'ında hepsi aynı kullanıcı; kova kullanıcı düzeyindedir).
//!
//! PID yeniden kullanımı: PID sorgusuyla `/proc` okuması arasında gönderen ölüp PID başka bir
//! sürece geçerse kimlik o sürecinki olur. Bunu hedefli kullanmak (kurbanın tam o PID'i almasını
//! sağlamak) pratikte mümkün değil; sonuç en kötü durumda yanlış kovaya sayılan birkaç görsel.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use zbus::names::{BusName, UniqueName};

use crate::services::image_sandbox::{KeyDomain, source_key};

/// Önbellekteki en fazla gönderen; dolunca boşaltılır (benzersiz adlar bus ömrünce yeniden
/// kullanılmaz, eski girdiler yalnızca yer kaplar).
const MAX_CACHED: usize = 256;
/// `/proc/<pid>/cgroup` okuma sınırı.
const MAX_CGROUP_BYTES: u64 = 4096;

/// Benzersiz ad → kaynak anahtarı. Bağlantının ilk görselli bildiriminde bir D-Bus gidiş-dönüşü,
/// sonrası önbellekten.
#[derive(Debug, Default)]
pub struct SenderKeys {
    cache: Mutex<HashMap<String, u64>>,
}

impl SenderKeys {
    /// Gönderenin kaynak anahtarı ([`KeyDomain::NotifySender`]). Gönderen yoksa ya da kimliği
    /// çözülemezse ortak kova ([`anon_key`]; fail-closed). Ortak kova da önbelleğe girer.
    pub async fn key(&self, conn: &zbus::Connection, sender: Option<&UniqueName<'_>>) -> u64 {
        let Some(sender) = sender else { return anon_key() };
        if let Some(&key) = self.lock().get(sender.as_str()) {
            return key;
        }
        let pid = match zbus::fdo::DBusProxy::new(conn).await {
            Ok(proxy) => proxy.get_connection_unix_process_id(BusName::Unique(sender.clone())).await.ok(),
            Err(_) => None,
        };
        let key = pid
            .and_then(|pid| process_identity(Path::new("/proc"), pid))
            .map_or_else(anon_key, |identity| source_key(KeyDomain::NotifySender, &identity));
        let mut cache = self.lock();
        if cache.len() >= MAX_CACHED {
            cache.clear();
        }
        cache.insert(sender.to_string(), key);
        key
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, u64>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Çözülemeyen gönderenlerin ortak kimliği. Gerçek kimlik çalıştırılabilir yolla (`/` ile)
/// başladığından onunla çakışmaz.
pub const ANON_IDENTITY: &str = "anon";

/// Ortak kovanın anahtarı ([`ANON_IDENTITY`]).
pub fn anon_key() -> u64 {
    source_key(KeyDomain::NotifySender, ANON_IDENTITY)
}

/// `<proc_root>/<pid>` sürecinin kimliği: çalıştırılabilir yol + cgroup v2 yolu (`0::` satırı;
/// yoksa boş). Çalıştırılabilir okunamazsa `None`.
fn process_identity(proc_root: &Path, pid: u32) -> Option<String> {
    let dir = proc_root.join(pid.to_string());
    let exe = std::fs::read_link(dir.join("exe")).ok()?;
    let mut cgroup = String::new();
    if let Ok(file) = std::fs::File::open(dir.join("cgroup")) {
        // Geçersiz UTF-8 ya da okuma hatası: cgroup'suz kimlik.
        if file.take(MAX_CGROUP_BYTES).read_to_string(&mut cgroup).is_err() {
            cgroup.clear();
        }
    }
    let unified = cgroup.lines().find_map(|l| l.strip_prefix("0::")).unwrap_or("");
    Some(format!("{}\0{unified}", exe.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_proc(exe: &str, cgroup: Option<&str>) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("42");
        std::fs::create_dir(&dir).unwrap();
        std::os::unix::fs::symlink(exe, dir.join("exe")).unwrap();
        if let Some(cgroup) = cgroup {
            std::fs::write(dir.join("cgroup"), cgroup).unwrap();
        }
        root
    }

    #[test]
    fn identity_is_exe_and_unified_cgroup() {
        let root = fake_proc("/usr/bin/foo", Some("1:name=systemd:/eski\n0::/user.slice/app-foo.scope\n"));
        assert_eq!(process_identity(root.path(), 42).unwrap(), "/usr/bin/foo\0/user.slice/app-foo.scope");
        // cgroup yoksa yalnızca çalıştırılabilir.
        let root = fake_proc("/usr/bin/foo", None);
        assert_eq!(process_identity(root.path(), 42).unwrap(), "/usr/bin/foo\0");
        // Süreç yok: None (çağıran benzersiz ada düşer).
        assert_eq!(process_identity(root.path(), 7), None);
    }

    #[test]
    fn same_exe_different_cgroup_is_different_identity() {
        let a = fake_proc("/usr/bin/python3", Some("0::/user.slice/app-a.scope\n"));
        let b = fake_proc("/usr/bin/python3", Some("0::/user.slice/app-b.scope\n"));
        assert_ne!(process_identity(a.path(), 42), process_identity(b.path(), 42));
    }

    #[test]
    fn oversized_cgroup_is_bounded() {
        let big = format!("0::/{}\n", "x".repeat(MAX_CGROUP_BYTES as usize * 4));
        let root = fake_proc("/usr/bin/foo", Some(&big));
        let id = process_identity(root.path(), 42).unwrap();
        assert!(id.len() <= "/usr/bin/foo\0".len() + MAX_CGROUP_BYTES as usize);
    }

    #[test]
    fn own_process_resolves() {
        let id = process_identity(Path::new("/proc"), std::process::id()).unwrap();
        let exe = std::env::current_exe().unwrap();
        assert!(id.starts_with(&format!("{}\0", exe.display())), "{id:?}");
    }

    #[test]
    fn anon_bucket_is_shared_and_distinct_from_real_identities() {
        assert_eq!(anon_key(), anon_key());
        let real = process_identity(Path::new("/proc"), std::process::id()).unwrap();
        assert_ne!(source_key(KeyDomain::NotifySender, &real), anon_key());
        assert!(real.starts_with('/'), "gerçek kimlik mutlak yolla başlar; ANON_IDENTITY ile çakışmaz");
    }
}
