//! Kullanım sıklığı/yakınlığı (frecency): `$XDG_STATE_HOME/nuxut/launcher.toml`
//! (varsayılan `~/.local/state/nuxut/launcher.toml`). Yalnızca yerel; ağ yok.
//!
//! Dosya küçük ve sınırlı: en çok [`MAX_RECORDS`] uygulama (taşınca en düşük puanlılar atılır),
//! okurken [`MAX_FILE_BYTES`]. Yazım atomiktir: aynı dizinde geçici dosya (0600), `fsync`,
//! `rename`. Bozuk dosya yok sayılır (boş kayıtla başlanır, ilk yazımda düzelir).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_RECORDS: usize = 500;
/// Emoji kipinde seçilen en çok emoji kaydı (`[emoji]` tablosu).
pub const MAX_EMOJI_RECORDS: usize = 200;
pub const MAX_FILE_BYTES: u64 = 256 << 10;
const DAY: i64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub count: u32,
    /// Son başlatma (Unix saniye).
    pub last: i64,
}

impl Record {
    /// Frecency: `ln(1 + count)` × yakınlık kovası (gün 1, hafta 0.7, ay 0.5, sonrası 0.25).
    pub fn frecency(&self, now: i64) -> f32 {
        let age = (now - self.last).max(0);
        let recency = match age {
            a if a < DAY => 1.0,
            a if a < 7 * DAY => 0.7,
            a if a < 30 * DAY => 0.5,
            _ => 0.25,
        };
        (1.0 + self.count as f32).ln() * recency
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub apps: HashMap<String, Record>,
    /// Emoji kipinde seçilenler (anahtar emoji dizisi; yalnız gömülü veride olanlar). Boşsa
    /// dosyaya yazılmaz: eski dosyalar ve emoji kullanmayanlar için biçim değişmez.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub emoji: HashMap<String, Record>,
}

impl Usage {
    /// Varsayılan dosya yolu.
    pub fn default_path() -> Option<PathBuf> {
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
        Some(state.join("nuxut/launcher.toml"))
    }

    /// Dosyadan; yoksa/bozuksa boş.
    pub fn load(path: &Path) -> Self {
        use std::io::Read;
        let Ok(file) = std::fs::File::open(path) else { return Self::default() };
        let mut text = String::new();
        if file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text).is_err() || text.len() as u64 > MAX_FILE_BYTES {
            tracing::warn!("başlatıcı kullanım dosyası okunamadı ya da çok büyük; yok sayıldı");
            return Self::default();
        }
        match toml::from_str::<Usage>(&text) {
            Ok(mut usage) => {
                usage.apps.retain(|id, _| valid_id(id));
                usage.emoji.retain(|e, _| super::emoji::find(e).is_some());
                usage.prune();
                usage
            }
            Err(_) => {
                tracing::warn!("başlatıcı kullanım dosyası bozuk; yok sayıldı");
                Self::default()
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<Record> {
        self.apps.get(id).copied()
    }

    pub fn frecency(&self, id: &str, now: i64) -> f32 {
        self.apps.get(id).map_or(0.0, |r| r.frecency(now))
    }

    /// Bir başlatmayı kaydeder.
    pub fn record(&mut self, id: &str, now: i64) {
        if !valid_id(id) {
            return;
        }
        bump(&mut self.apps, id, now);
        self.prune();
    }

    pub fn emoji_frecency(&self, emoji: &str, now: i64) -> f32 {
        self.emoji.get(emoji).map_or(0.0, |r| r.frecency(now))
    }

    /// Emoji kipinde bir seçimi kaydeder (yalnız gömülü veride olan emoji).
    pub fn record_emoji(&mut self, emoji: &str, now: i64) {
        if super::emoji::find(emoji).is_none() {
            return;
        }
        bump(&mut self.emoji, emoji, now);
        self.prune();
    }

    /// Kayıt sayısını sınırlar.
    fn prune(&mut self) {
        prune_map(&mut self.apps, MAX_RECORDS);
        prune_map(&mut self.emoji, MAX_EMOJI_RECORDS);
    }

    /// Atomik yazım (geçici dosya + fsync + rename); dizin yoksa oluşturulur.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let dir = path.parent().ok_or_else(|| std::io::Error::other("üst dizin yok"))?;
        std::fs::create_dir_all(dir)?;
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        let tmp = dir.join(format!(".{}.{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("usage"), std::process::id()));
        let result = (|| {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)?;
            // Dizin girdisi de kalıcı olsun (çökmede eski ya da yeni dosya kalır, yarım değil).
            if let Ok(d) = std::fs::File::open(dir) {
                let _ = d.sync_all();
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

fn bump(map: &mut HashMap<String, Record>, key: &str, now: i64) {
    let r = map.entry(key.to_owned()).or_insert(Record { count: 0, last: now });
    r.count = r.count.saturating_add(1);
    r.last = r.last.max(now);
}

/// En düşük frecency'liler (eşitlikte en eskiler) atılır; `max` kayıt kalır.
fn prune_map(map: &mut HashMap<String, Record>, max: usize) {
    if map.len() <= max {
        return;
    }
    let now = map.values().map(|r| r.last).max().unwrap_or(0);
    let mut ranked: Vec<(String, f32, i64)> = map.iter().map(|(id, r)| (id.clone(), r.frecency(now), r.last)).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.cmp(&a.2)));
    for (id, _, _) in ranked.into_iter().skip(max) {
        map.remove(&id);
    }
}

/// Masaüstü dosya kimliği biçimi (dosyaya keyfi anahtar yazılmasın).
fn valid_id(id: &str) -> bool {
    (1..=255).contains(&id.len())
        && id.ends_with(".desktop")
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+' | b'@'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frecency_orders_by_count_and_recency() {
        let now = 1_000_000_000;
        let fresh = Record { count: 3, last: now - 60 };
        let old = Record { count: 3, last: now - 40 * DAY };
        let many_old = Record { count: 50, last: now - 40 * DAY };
        assert!(fresh.frecency(now) > old.frecency(now));
        assert!(many_old.frecency(now) > old.frecency(now));
        assert_eq!(Record { count: 0, last: now }.frecency(now), 0.0);
    }

    #[test]
    fn record_validates_and_prunes() {
        let mut u = Usage::default();
        u.record("a.desktop", 10);
        u.record("a.desktop", 20);
        u.record("../evil", 20);
        u.record("x\n= 1", 20);
        assert_eq!(u.get("a.desktop"), Some(Record { count: 2, last: 20 }));
        assert_eq!(u.apps.len(), 1);
        for i in 0..MAX_RECORDS + 20 {
            u.record(&format!("app{i}.desktop"), 100 + i as i64);
        }
        assert_eq!(u.apps.len(), MAX_RECORDS);
        assert!(u.get(&format!("app{}.desktop", MAX_RECORDS + 19)).is_some(), "en yeni korunur");
    }

    #[test]
    fn emoji_records_validate_prune_and_roundtrip() {
        let mut u = Usage::default();
        u.record_emoji("🐈", 10);
        u.record_emoji("🐈", 20);
        u.record_emoji("not-an-emoji", 20);
        u.record_emoji("firefox.desktop", 20);
        assert_eq!(u.emoji.len(), 1, "yalnız gömülü verideki emoji");
        assert_eq!(u.emoji["🐈"], Record { count: 2, last: 20 });
        assert!(u.emoji_frecency("🐈", 20) > 0.0);
        assert!(u.apps.is_empty(), "emoji uygulama kaydına karışmaz");

        let all = crate::services::launcher::emoji::all();
        for (i, e) in all.iter().take(MAX_EMOJI_RECORDS + 10).enumerate() {
            u.record_emoji(e.emoji, 100 + i as i64);
        }
        assert_eq!(u.emoji.len(), MAX_EMOJI_RECORDS);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("launcher.toml");
        u.save(&path).unwrap();
        assert_eq!(Usage::load(&path), u);
        // Emoji tablosu olmayan eski dosya okunur; boş tablo yazılmaz.
        std::fs::write(&path, "[apps.\"a.desktop\"]\ncount = 1\nlast = 1\n").unwrap();
        let old = Usage::load(&path);
        assert!(old.emoji.is_empty() && old.apps.len() == 1);
        assert!(!toml::to_string(&old).unwrap().contains("emoji"));
        // Elle eklenmiş geçersiz emoji anahtarı yüklemede atılır.
        std::fs::write(&path, "[emoji.\"x\"]\ncount = 1\nlast = 1\n[emoji.\"🐈\"]\ncount = 1\nlast = 1\n").unwrap();
        assert_eq!(Usage::load(&path).emoji.len(), 1);
    }

    #[test]
    fn save_load_roundtrip_and_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/nuxut/launcher.toml");
        assert_eq!(Usage::load(&path), Usage::default(), "yok → boş");
        let mut u = Usage::default();
        u.record("org.gnome.Nautilus.desktop", 42);
        u.save(&path).unwrap();
        assert_eq!(Usage::load(&path), u);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "geçici dosya kalmaz");

        std::fs::write(&path, "apps = 12\n[[").unwrap();
        assert_eq!(Usage::load(&path), Usage::default());
        std::fs::write(&path, "[apps.\"bad id\"]\ncount = 1\nlast = 1\n[apps.\"ok.desktop\"]\ncount = 2\nlast = 3\n").unwrap();
        assert_eq!(Usage::load(&path).apps.len(), 1);
        std::fs::write(&path, vec![b'#'; MAX_FILE_BYTES as usize + 10]).unwrap();
        assert_eq!(Usage::load(&path), Usage::default());
    }
}
