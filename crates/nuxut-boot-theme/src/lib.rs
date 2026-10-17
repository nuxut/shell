//! Nuxut sistem görünümü: Plymouth (açılış/LUKS), GRUB ve Limine temaları.
//!
//! Bu crate yalnız **saf** iş yapar: tema dosyalarını bellekte üretir, sistem yapılandırma
//! metinlerini ayrıştırır/düzenler (yan etkisiz fonksiyonlar), önizleme çizer. Diske yazan,
//! komut çalıştıran ve yetki isteyen kısım ayrı, küçük kök yardımcıdadır
//! (`crates/nuxut-system-helper`, ADR-0008); kullanıcı tarafı (`nuxut-shell system-theme`)
//! yalnız algılar, planı gösterir ve önizler. Tasarım: `docs/design/boot-theme.md`.

pub mod art;
pub mod colors;
pub mod detect;
pub mod files;
pub mod fontsel;
pub mod grub;
pub mod limine;
pub mod pf2;
pub mod plan;
pub mod plymouth;
pub mod preview;
pub mod raster;

pub use colors::{BootColors, Rgb};
pub use files::{Lang, ThemeFiles, ThemeOptions};
