//! Testlerin tek canlılık sınırı (birim testleri `src/lib.rs` üzerinden, entegrasyon testleri
//! `support` üzerinden aynı dosyayı kullanır).
//!
//! Beklenen olay/koşul için üst sınırdır, süre varsayımı DEĞİL: beklemeler koşul gelince hemen
//! döner; bu yalnız takılan testi sonunda düşürür. Kısa sınırlar (3–5 sn, `timeout 5`) yük altında
//! kararsızdı (2026-10-17): testler derleme slice'ında düşük öncelikle ve CPU kotasıyla çalışır,
//! bellek bekçisi baskıda slice'ı ~1 sn'lik art arda dondurur. Donan süreç (test, özel
//! PipeWire/D-Bus, servis thread'i) çözüldüğünde duvar saati son tarihi çoktan geçmiş olur;
//! olay gelmeden beklemeden çıkılırdı. Başarılı koşunun süresi bu değerden etkilenmez.

#![allow(dead_code)]

use std::time::Duration;

pub const LIVENESS: Duration = Duration::from_secs(120);

/// "Takılmadan, çabuk bitti" iddialarının sınırı (kapanış takılı çağrıyı beklemez, servis cevap
/// vermeye devam eder). Koruduğu durumun çok altında (D-Bus çağrısı varsayılan 25 sn zaman aşımı,
/// sonsuz bekleyen thread), slice donmalarının çok üstünde. Eskiden 0.5–2 sn idi; bir donma
/// iddiayı düşürüyordu (2026-10-17).
pub const PROMPT: Duration = Duration::from_secs(10);
