//! `~/.config/nuxut/displays.toml`: onaylanmış yerleşimlerin bağlı monitör kümesi başına
//! kaydı (ADR-0005 §6.1, §8; K5 otomatik küme hafızası, K7 ayrı dosya). Saf ayrıştırma ve
//! yazım metni; dosya IO'su çağıranın (`app/displays.rs`, `nuxut-apply` ile).
//!
//! Bilinmeyen anahtar yok sayılır; sınır dışı değer o küme kaydını geçersiz kılar (uyarı);
//! dosya ≤ 256 KiB; en çok 32 küme (en eski kullanılan düşer).

use serde::{Deserialize, Serialize};

use super::model::{DisplayConfig, DisplayLayout, MAX_DISPLAYS, MAX_FIELD, Mode, Transform, Vrr, scale_from_f64};

pub const FILE_NAME: &str = "displays.toml";
pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_SETS: usize = 32;
pub const VERSION: u32 = 1;
pub const HEADER: &str = "# Nuxut — ekran yerleşimleri (Ayarlar → Ekranlar yazar; elle düzenlemeyin).\n"; // i18n: tanı kalıcı dosyanın başlık yorumu (dosya içeriği)

#[derive(Debug, Serialize, Deserialize, Default)]
struct FileRepr {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    set: Vec<SetRepr>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SetRepr {
    key: Vec<String>,
    #[serde(default)]
    primary: String,
    /// Son kullanım sırası (büyük = yeni).
    #[serde(default)]
    used: u64,
    #[serde(default)]
    display: Vec<DisplayRepr>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DisplayRepr {
    id: String,
    enabled: bool,
    mode: String,
    position: [i32; 2],
    scale: f64,
    transform: u8,
    vrr: String,
    #[serde(default)]
    mirror_of: String,
}

/// Bir küme kaydı.
#[derive(Debug, Clone, PartialEq)]
pub struct SetRecord {
    /// Sıralı eşleme anahtarları (`Hardware::set_key`).
    pub key: Vec<String>,
    pub layout: DisplayLayout,
    pub used: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Store {
    pub sets: Vec<SetRecord>,
}

fn key_ok(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= MAX_FIELD * 4 + 3 && !s.chars().any(char::is_control)
}

impl Store {
    /// Metni okur; (kayıtlar, uyarılar). Bozuk dosya boş depo + uyarı.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        if text.len() > MAX_FILE_BYTES {
            return (Self::default(), vec![format!("{FILE_NAME} çok büyük; yok sayıldı")]); // i18n: tanı depo uyarısı (log + msg)
        }
        let repr: FileRepr = match toml::from_str(text) {
            Ok(r) => r,
            Err(e) => return (Self::default(), vec![format!("{FILE_NAME} okunamadı: {}", e.message())]), // i18n: tanı depo uyarısı (log + msg)
        };
        if repr.version > VERSION {
            warnings.push(format!("{FILE_NAME} sürümü {} bilinmiyor; okunabilen alanlar kullanılıyor", repr.version));
        }
        let mut sets = Vec::new();
        for (i, s) in repr.set.into_iter().enumerate() {
            match Self::record(s) {
                Ok(r) if !sets.iter().any(|o: &SetRecord| o.key == r.key) => sets.push(r),
                Ok(_) => warnings.push(format!("{FILE_NAME}: set[{i}] yinelenen küme; atlandı")),
                Err(e) => warnings.push(format!("{FILE_NAME}: set[{i}] geçersiz ({e}); atlandı")),
            }
            if sets.len() == MAX_SETS {
                break;
            }
        }
        (Self { sets }, warnings)
    }

    fn record(s: SetRepr) -> Result<SetRecord, String> {
        if s.key.is_empty() || s.key.len() > MAX_DISPLAYS || !s.key.iter().all(|k| key_ok(k)) {
            return Err("key".into());
        }
        if s.display.len() != s.key.len() {
            return Err("display sayısı key ile uyuşmuyor".into());
        }
        let mut key = s.key;
        key.sort();
        let mut displays = Vec::new();
        for d in s.display {
            if !key.contains(&d.id) {
                return Err(format!("display {} kümede yok", d.id.chars().take(32).collect::<String>()));
            }
            let mode = Mode::parse_toml(&d.mode).ok_or("mode")?;
            let scale_120 = scale_from_f64(d.scale).filter(|s| (60..=480).contains(s)).ok_or("scale")?;
            let transform = Transform::new(d.transform).ok_or("transform")?;
            let vrr = Vrr::from_name(&d.vrr).ok_or("vrr")?;
            if d.position.iter().any(|v| v.unsigned_abs() > super::model::MAX_COORD.unsigned_abs()) {
                return Err("position".into());
            }
            let mirror_of = (!d.mirror_of.is_empty()).then_some(d.mirror_of);
            if mirror_of.as_ref().is_some_and(|m| !key.contains(m)) {
                return Err("mirror_of".into());
            }
            displays.push(DisplayConfig {
                key: d.id,
                enabled: d.enabled,
                mode,
                pos: (d.position[0], d.position[1]),
                scale_120,
                transform,
                vrr,
                mirror_of,
            });
        }
        let primary = (!s.primary.is_empty()).then_some(s.primary).filter(|p| key.contains(p));
        Ok(SetRecord { key, layout: DisplayLayout { displays, primary }, used: s.used })
    }

    pub fn get(&self, key: &[String]) -> Option<&SetRecord> {
        self.sets.iter().find(|s| s.key == key)
    }

    /// Kaydı ekler/günceller; en çok [`MAX_SETS`] (en az yakın kullanılan düşer).
    pub fn put(&mut self, key: Vec<String>, layout: DisplayLayout) {
        let used = self.sets.iter().map(|s| s.used).max().unwrap_or(0) + 1;
        self.sets.retain(|s| s.key != key);
        self.sets.push(SetRecord { key, layout, used });
        while self.sets.len() > MAX_SETS {
            if let Some(i) = self.sets.iter().enumerate().min_by_key(|(_, s)| s.used).map(|(i, _)| i) {
                self.sets.remove(i);
            }
        }
    }

    /// Kümeyi en son kullanılan yapar (hotplug'da uygulandı).
    pub fn touch(&mut self, key: &[String]) {
        let used = self.sets.iter().map(|s| s.used).max().unwrap_or(0) + 1;
        if let Some(s) = self.sets.iter_mut().find(|s| s.key == key) {
            s.used = used;
        }
    }

    pub fn to_toml(&self) -> String {
        let repr = FileRepr {
            version: VERSION,
            set: self
                .sets
                .iter()
                .map(|s| SetRepr {
                    key: s.key.clone(),
                    primary: s.layout.primary.clone().unwrap_or_default(),
                    used: s.used,
                    display: s
                        .layout
                        .displays
                        .iter()
                        .map(|d| DisplayRepr {
                            id: d.key.clone(),
                            enabled: d.enabled,
                            mode: d.mode.rule(),
                            position: [d.pos.0, d.pos.1],
                            scale: f64::from(d.scale_120) / 120.0,
                            transform: d.transform.get(),
                            vrr: d.vrr.name().to_owned(),
                            mirror_of: d.mirror_of.clone().unwrap_or_default(),
                        })
                        .collect(),
                })
                .collect(),
        };
        let body = toml::to_string(&repr).unwrap_or_default();
        format!("{HEADER}{body}")
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::Hardware;
    use super::super::model::fixtures::DUAL;
    use super::*;

    fn layout() -> (Vec<String>, DisplayLayout) {
        let hw = Hardware::parse(DUAL).unwrap();
        let mut l = hw.current_layout();
        l.displays[0].scale_120 = 160;
        l.displays[1].mirror_of = Some(l.displays[0].key.clone());
        l.primary = Some(l.displays[0].key.clone());
        (hw.set_key(), l)
    }

    #[test]
    fn roundtrip() {
        let (key, l) = layout();
        let mut s = Store::default();
        s.put(key.clone(), l.clone());
        let text = s.to_toml();
        assert!(text.starts_with(HEADER));
        let (back, w) = Store::parse(&text);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back.get(&key).unwrap().layout, l);
        assert_eq!(back, s);
    }

    #[test]
    fn invalid_records_are_skipped_with_warning() {
        let (key, l) = layout();
        let mut s = Store::default();
        s.put(key, l);
        let good = s.to_toml();
        let bad = good.replace("scale = 1.3333333333333333", "scale = 9.0");
        assert_ne!(bad, good, "{good}");
        let (back, w) = Store::parse(&bad);
        assert!(back.sets.is_empty());
        assert_eq!(w.len(), 1);
        let (back, w) = Store::parse("version = 1\n[[set]]\nkey = []\n");
        assert!(back.sets.is_empty() && w.len() == 1);
        let (back, w) = Store::parse("bu toml değil = = =");
        assert!(back.sets.is_empty() && w.len() == 1);
        // Bilinmeyen anahtar sorun değil.
        let (back, w) = Store::parse(&format!("gelecek = true\n{good}"));
        assert_eq!(back.sets.len(), 1);
        assert!(w.is_empty());
        // i32::MIN konum: panic yok, kayıt geçersiz.
        let min = good.replacen("position = [0, 0]", "position = [-2147483648, 0]", 1);
        assert_ne!(min, good, "{good}");
        let (back, w) = Store::parse(&min);
        assert!(back.sets.is_empty() && w.len() == 1);
        let (back, w) = Store::parse(&"#".repeat(MAX_FILE_BYTES + 1));
        assert!(back.sets.is_empty() && w.len() == 1);
    }

    #[test]
    fn lru_cap() {
        let (_, l) = layout();
        let mut s = Store::default();
        for i in 0..(MAX_SETS + 5) {
            s.put(vec![format!("k{i}")], l.clone());
        }
        assert_eq!(s.sets.len(), MAX_SETS);
        assert!(s.get(&["k0".to_owned()]).is_none());
        s.touch(&["k5".to_owned()]);
        s.put(vec!["new".to_owned()], l.clone());
        assert!(s.get(&["k5".to_owned()]).is_some(), "dokunulan kalır");
        assert!(s.get(&["k6".to_owned()]).is_none());
    }
}
