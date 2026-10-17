//! Nuxut TOML ayar dosyalarının kendini onarması (`docs/design/config-migrations.md`).
//!
//! Saf çekirdek: metin girer, onarılmış metin ve yapılanların listesi çıkar; dosya sistemi yok
//! (yedek yazımı ayrı: [`backup`]). Onarım yorumları ve düzeni korur, dokunulmayan her bayt aynı
//! kalır. Her anlamsal adım bir **eşitlik kâhiniyle** doğrulanır: çağıranın yükleyicisi
//! ([`Loaded`]) değişiklikten önce ve sonra aynı ayarı vermiyorsa adım uygulanmaz. Böylece
//! "temizlik" asla davranış değiştirmez; davranış değiştiren tek adımlar açıkça adlandırılmıştır
//! (geçersiz değer → varsayılan, eski ad → yeni ad, bozuk satır → yorum).
//!
//! Sıra: [`syntax`] (bozuk TOML) → geçersiz değerler → bilinmeyen anahtarlar (yeniden adlandırma,
//! eski, yazım hatası, kullanılmayan) → aralık dışı sayılar → varsayılana eşit satırlar → boşalan
//! tablo başlıkları. İdempotenttir: onarılmış metin ikinci kez onarılınca değişmez.

pub mod backup;
mod heal;
mod syntax;
mod text;
mod trace;
mod typo;

use std::ops::Range;

pub use heal::{Rules, heal, repair_syntax};
pub use trace::{Kind, Schema};

/// Onarımın yaptığı bir şey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// Anahtar yolu (`bar.size`); bozuk satırda boş olabilir.
    pub key: String,
    pub kind: ActionKind,
    /// 1 tabanlı satır (o adımdaki metinde; yalnızca gösterim için).
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    /// Okunamayan satır yoruma alındı ya da başlık onarıldı.
    Broken,
    /// Tekrar eden anahtar: önceki yoruma alındı, son yazılan geçerli.
    Duplicate,
    /// Tekrar eden tablo başlığı: ikinci bölüm birinciye taşındı.
    MergedTable,
    /// Eski adla yazılmış anahtar yeni adına taşındı.
    Moved { to: String },
    /// Değer okunamadı ya da yok sayılıyordu: satır yoruma alındı, varsayılan geçerli.
    Invalid,
    /// Sayı sınır dışıydı: zaten kullanılan sınır değeri yazıldı, eski değer yorumda.
    Clamped { to: String },
    /// Artık kullanılmayan (kaldırılmış) anahtar silindi.
    Retired,
    /// Hiçbir sürümün tanımadığı, etkisiz anahtar silindi.
    Unused,
    /// Varsayılana eşit değer silindi (dosya yalnızca kullanıcının farklarını tutar).
    Redundant,
    /// Yazım hatası olabilir: değişiklik yok, öneri.
    Suggest { candidate: String },
}

/// Bildirimde gruplama.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// Bozuk satır, tekrar.
    Repaired,
    /// Geçersiz ya da sınır dışı değer.
    Reset,
    /// Eski, kullanılmayan ya da taşınan satır.
    Cleaned,
    /// Varsayılana eşit satır (sessiz).
    Quiet,
    /// Yazım hatası önerisi (metin değişmez).
    Suggestion,
}

impl ActionKind {
    pub fn class(&self) -> Class {
        match self {
            Self::Broken | Self::Duplicate | Self::MergedTable => Class::Repaired,
            Self::Invalid | Self::Clamped { .. } => Class::Reset,
            Self::Moved { .. } | Self::Retired | Self::Unused => Class::Cleaned,
            Self::Redundant => Class::Quiet,
            Self::Suggest { .. } => Class::Suggestion,
        }
    }

    /// Bu adım metni değiştirir mi?
    pub fn edits(&self) -> bool {
        !matches!(self, Self::Suggest { .. })
    }

    /// Kullanıcı "Geri al" dediğinde bu anahtar sonraki onarımlarda korunur mu? Bozuk satır ve
    /// geçersiz değer korunamaz (dosya okunamaz ya da ayar çalışmaz); öneri zaten bir şey
    /// değiştirmedi (yok saymak ayrı eylemdir). Diğerleri zararsızdır.
    pub fn keepable(&self) -> bool {
        matches!(self, Self::Moved { .. } | Self::Retired | Self::Unused | Self::Redundant)
    }
}

/// Onarım sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Healed {
    pub text: String,
    pub actions: Vec<Action>,
}

impl Healed {
    /// Metni değiştiren adım var mı?
    pub fn edited(&self) -> bool {
        self.actions.iter().any(|a| a.kind.edits())
    }

    /// Sınıf başına sayı.
    pub fn count(&self, class: Class) -> usize {
        self.actions.iter().filter(|a| a.kind.class() == class).count()
    }
}

/// Dosyaya yazılan yorum işaretleri (çağıranın dilinde). İşaret satırı: `# {etiket} ({damga})`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labels {
    /// Tarih damgası (`2026-10-16`).
    pub stamp: String,
    pub broken: String,
    pub duplicate: String,
    pub invalid: String,
    pub clamped: String,
}

impl Labels {
    pub(crate) fn marker(&self, label: &str) -> String {
        format!("{label} ({})", self.stamp)
    }
}

/// Çağıranın yükleyicisinin sonucu: ayrıştırma + doğrulama.
#[derive(Debug, Clone, PartialEq)]
pub enum Loaded<C> {
    Ok {
        config: C,
        /// Türün tanımadığı anahtarların yolları (harita anahtarları olduğu gibi; dizi öğesi `[i]`).
        unknown: Vec<Vec<String>>,
    },
    /// Değer okunamadı (tür hatası vb.); `span` metindeki yeri.
    Rejected { span: Option<Range<usize>>, message: String },
}

/// `T`'yi TOML'dan okur; bilinmeyen anahtarları toplar (serde_ignored), hata konumunu verir.
pub fn load_serde<T: serde::de::DeserializeOwned>(text: &str) -> Loaded<T> {
    let de = match toml::Deserializer::parse(text) {
        Ok(de) => de,
        Err(e) => return Loaded::Rejected { span: e.span(), message: e.message().to_owned() },
    };
    let mut unknown = Vec::new();
    match serde_ignored::deserialize(de, |path| unknown.push(path_segments(&path))) {
        Ok(config) => Loaded::Ok { config, unknown },
        Err(e) => Loaded::Rejected { span: e.span(), message: e.message().to_owned() },
    }
}

fn path_segments(path: &serde_ignored::Path<'_>) -> Vec<String> {
    use serde_ignored::Path;
    let mut out = match path {
        Path::Root => return Vec::new(),
        Path::Seq { parent, .. }
        | Path::Map { parent, .. }
        | Path::Some { parent }
        | Path::NewtypeStruct { parent }
        | Path::NewtypeVariant { parent } => path_segments(parent),
    };
    match path {
        Path::Seq { index, .. } => out.push(format!("[{index}]")),
        Path::Map { key, .. } => out.push(key.clone()),
        _ => {}
    }
    out
}
