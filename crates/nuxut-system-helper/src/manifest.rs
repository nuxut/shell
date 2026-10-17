//! Bildirim: yardımcının sistemde neyi değiştirdiğinin kaydı (`/var/lib/nuxut/system-theme/
//! manifest.json`). Her değiştirici adımdan hemen sonra yazılır (günlük): yarıda kesilen bir
//! uygulama da geri alınabilir. Geri alma yalnız buradaki kayıtlara dokunur.

use nuxut_boot_theme::plan::Target;
use serde::{Deserialize, Serialize};

use crate::policy;

pub const VERSION: u32 = 1;
const MAX_MANIFEST: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Entry {
    /// Bizim kurduğumuz tema dizini. `previous`: aynı adda önceden duran (bizim olmayan) dizinin
    /// yedeği; geri almada yerine konur.
    Dir { target: Target, path: String, previous: Option<String> },
    /// Bizim kurduğumuz dosya; `hash`: yazdığımız içerik (değişmişse geri almada silinmez).
    File { target: Target, path: String, hash: String },
    /// Değiştirdiğimiz dosya; `backup`: Nuxut öncesi özgün kopya; `hash`: son yazdığımız içerik.
    Edit { target: Target, path: String, backup: String, hash: String },
}

impl Entry {
    pub fn target(&self) -> Target {
        match self {
            Self::Dir { target, .. } | Self::File { target, .. } | Self::Edit { target, .. } => *target,
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Self::Dir { path, .. } | Self::File { path, .. } | Self::Edit { path, .. } => path,
        }
    }

    /// Kayıttaki her yol beyaz listede mi (bozuk/oynanmış bildirim reddedilir).
    fn valid(&self) -> bool {
        let extra = match self {
            Self::Dir { previous: Some(p), .. } => Some(p.as_str()),
            Self::Edit { backup, .. } => Some(backup.as_str()),
            _ => None,
        };
        policy::allowed(self.path()) && extra.is_none_or(policy::allowed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub entries: Vec<Entry>,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_MANIFEST {
            return Err("bildirim çok büyük".into());
        }
        let m: Self = serde_json::from_slice(bytes).map_err(|e| format!("bildirim okunamadı: {e}"))?;
        if m.version != VERSION {
            return Err(format!("bilinmeyen bildirim sürümü {}", m.version));
        }
        if let Some(bad) = m.entries.iter().find(|e| !e.valid()) {
            return Err(format!("bildirimde izin dışı yol: {}", bad.path()));
        }
        Ok(m)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = serde_json::to_vec_pretty(&Self { version: VERSION, entries: self.entries.clone() }).unwrap_or_default();
        v.push(b'\n');
        v
    }

    pub fn for_target(&self, t: Target) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |e| e.target() == t)
    }

    pub fn find(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path() == path)
    }

    /// Aynı yolun kaydını değiştirir ya da ekler.
    pub fn upsert(&mut self, e: Entry) {
        match self.entries.iter_mut().find(|x| x.path() == e.path()) {
            Some(slot) => *slot = e,
            None => self.entries.push(e),
        }
    }

    pub fn remove_target(&mut self, t: Target) {
        self.entries.retain(|e| e.target() != t);
    }
}

/// İçerik özeti (blake3, onaltılık).
pub fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_validation() {
        let mut m = Manifest { version: VERSION, entries: vec![] };
        m.upsert(Entry::File { target: Target::Grub, path: "etc/default/grub.d/90-nuxut-theme.cfg".into(), hash: hash(b"x") });
        m.upsert(Entry::Edit {
            target: Target::Limine,
            path: "boot/limine.conf".into(),
            backup: "boot/limine.conf.nuxut-20261016120000.bak".into(),
            hash: hash(b"y"),
        });
        let back = Manifest::parse(&m.to_bytes()).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.for_target(Target::Grub).count(), 1);
        let evil = br#"{"version":1,"entries":[{"kind":"file","target":"grub","path":"etc/shadow","hash":"0"}]}"#;
        assert!(Manifest::parse(evil).is_err());
        let evil2 = br#"{"version":1,"entries":[{"kind":"edit","target":"grub","path":"boot/grub/grub.cfg","backup":"etc/shadow","hash":"0"}]}"#;
        assert!(Manifest::parse(evil2).is_err());
        assert!(Manifest::parse(br#"{"version":9,"entries":[]}"#).is_err());
        assert!(Manifest::parse(b"{").is_err());
    }
}
