//! Ölçümlerin görünen metinleri (ondalık ayırıcı ve yüzde kalıbı etkin sayı locale'inden,
//! [`crate::percent`]). Bar, ipucu ve panel aynı biçimi kullanır; görünen metin değişmedikçe
//! yeniden çizim yapılmaz (karşılaştırma bu metinlerle).

use super::{Rates, Snapshot, Usage};

use nuxut::i18n::t;

/// tr "%23", en "23%" (0..=100'e sınırlı, yuvarlanmış). NaN → 0.
pub fn percent(v: f32) -> String {
    let v = if v.is_finite() { v.clamp(0.0, 100.0) } else { 0.0 };
    crate::percent::text(f64::from(v))
}

/// Tam `digits` kesir basamağı, etkin sayı locale'inin ondalık ayırıcısıyla (tr "5,1", en "5.1"),
/// gruplamasız. Locale katalogdan ([`nuxut::i18n::current`]; iş parçacığı önbellekli, ucuz).
pub fn fixed(v: f64, digits: usize) -> String {
    let mut o = nuxut::locale::NumberOptions::fixed(digits);
    o.grouping = false;
    nuxut::i18n::current().numeric().format_number_with(v, &o)
}

/// Bir ondalık; 100 ve üstünde ondalıksız.
fn decimal(v: f64) -> String {
    if v >= 99.95 { fixed(v, 0) } else { fixed(v, 1) }
}

const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

/// (değer, birim indeksi): 1024'lük basamak.
fn scale(v: f64) -> (f64, usize) {
    let mut v = if v.is_finite() { v.max(0.0) } else { 0.0 };
    let mut i = 0;
    while v >= 1024.0 && i + 1 < UNITS.len() {
        v /= 1024.0;
        i += 1;
    }
    (v, i)
}

/// "5,1 GiB", "512 B".
pub fn bytes(b: u64) -> String {
    let (v, i) = scale(b as f64);
    if i == 0 { format!("{b} B") } else { format!("{} {}", decimal(v), UNITS[i]) }
}

/// "5,1 / 15,6 GiB" (iki değer aynı birimde, toplamın birimiyle).
pub fn usage(u: Usage) -> String {
    let (t, i) = scale(u.total as f64);
    if i == 0 {
        return format!("{} / {} B", u.used, u.total);
    }
    let used = u.used as f64 / 1024f64.powi(i as i32);
    format!("{} / {} {}", decimal(used), decimal(t), UNITS[i])
}

/// "1,2 MiB/s".
pub fn rate(bps: f64) -> String {
    let (v, i) = scale(bps);
    if i == 0 { format!("{} B/s", v.round() as u64) } else { format!("{} {}/s", decimal(v), UNITS[i]) }
}

/// Bar için kısa hız: "0K", "12K", "1,2M", "120M" (en az KiB; genişlik sabit kalsın diye kısa).
pub fn rate_compact(bps: f64) -> String {
    let (mut v, mut i) = scale(bps);
    if i == 0 {
        v /= 1024.0;
        i = 1;
    }
    let unit = &UNITS[i][..1];
    if v >= 10.0 || i == 1 { format!("{}{unit}", v.round() as u64) } else { format!("{}{unit}", decimal(v)) }
}

/// "52 °C".
pub fn celsius(c: f32) -> String {
    format!("{} °C", c.round() as i32)
}

/// Bar için: "52°".
pub fn celsius_compact(c: f32) -> String {
    format!("{}°", c.round() as i32)
}

/// "↓ 1,2 MiB/s  ↑ 30,0 KiB/s".
pub fn rates(r: Rates) -> String {
    format!("↓ {}  ↑ {}", rate(r.down), rate(r.up))
}

/// İpucu (en çok 4 satır; framework ipucu sınırı): CPU (+ sıcaklık), bellek, GPU (varsa), ağ + disk.
pub fn tooltip(s: &Snapshot) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(4);
    let mut cpu = match s.cpu {
        Some(c) => t!("shell-sysmon-tooltip-cpu", value = percent(c)),
        None => t!("shell-sysmon-tooltip-cpu-unavailable"),
    };
    if let Some(t) = s.cpu_temp {
        cpu.push_str(&format!(" · {}", celsius(t)));
    }
    lines.push(cpu);
    if let Some(m) = s.memory {
        let u = Usage { used: m.used, total: m.total };
        lines.push(t!("shell-sysmon-tooltip-memory", usage = usage(u), percent_text = percent(u.percent())));
    }
    if let Some(g) = s.gpus.iter().find(|g| !g.suspended && (g.busy.is_some() || g.vram.is_some())) {
        let mut line = String::from("GPU");
        if let Some(b) = g.busy {
            line.push_str(&format!(" {}", percent(b)));
        }
        if let Some(v) = g.vram {
            line.push_str(&format!(" · VRAM {}", usage(v)));
        }
        lines.push(line);
    }
    let mut io = Vec::new();
    if let Some(n) = s.net {
        io.push(format!("↓ {} ↑ {}", rate(n.down), rate(n.up)));
    }
    if let Some(r) = s.root {
        io.push(t!("shell-sysmon-tooltip-disk", percent_text = percent(r.percent())));
    }
    if !io.is_empty() {
        lines.push(io.join(" · "));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::sysmon::{Gpu, Memory};

    #[test]
    fn numbers() {
        crate::i18n::use_turkish_for_tests();
        crate::percent::tr_catalog();
        assert_eq!(percent(23.4), "%23");
        assert_eq!(percent(150.0), "%100");
        assert_eq!(percent(f32::NAN), "%0");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(5_476_083_302), "5,1 GiB");
        assert_eq!(bytes(200 * 1024 * 1024), "200 MiB");
        assert!(bytes(u64::MAX).ends_with(" PiB"), "en büyük birimde kalır");
        assert_eq!(usage(Usage { used: 5 << 30, total: 16 << 30 }), "5,0 / 16,0 GiB");
        assert_eq!(usage(Usage { used: 3, total: 9 }), "3 / 9 B");
        assert_eq!(rate(0.0), "0 B/s");
        assert_eq!(rate(1.5 * 1024.0 * 1024.0), "1,5 MiB/s");
        assert_eq!(rate(f64::NAN), "0 B/s");
        assert_eq!(rate_compact(0.0), "0K");
        assert_eq!(rate_compact(12.0 * 1024.0), "12K");
        assert_eq!(rate_compact(1.2 * 1024.0 * 1024.0), "1,2M");
        assert_eq!(rate_compact(120.0 * 1024.0 * 1024.0), "120M");
        assert_eq!(celsius(52.4), "52 °C");
        assert_eq!(celsius_compact(-3.6), "-4°");
    }

    #[test]
    fn tooltip_lines() {
        crate::i18n::use_turkish_for_tests();
        crate::percent::tr_catalog();
        let s = Snapshot {
            cpu: Some(12.3),
            cpu_temp: Some(51.0),
            memory: Some(Memory { total: 16 << 30, used: 4 << 30, swap_total: 0, swap_used: 0 }),
            gpus: vec![Gpu { driver: "amdgpu".into(), suspended: false, busy: Some(5.0), vram: Some(Usage { used: 1 << 30, total: 8 << 30 }) }],
            net: Some(Rates { down: 2048.0, up: 0.0 }),
            root: Some(Usage { used: 1, total: 2 }),
            ..Snapshot::default()
        };
        let t = tooltip(&s);
        assert_eq!(
            t,
            "İşlemci %12 · 51 °C\nBellek 4,0 / 16,0 GiB (%25)\nGPU %5 · VRAM 1,0 / 8,0 GiB\n↓ 2,0 KiB/s ↑ 0 B/s · Disk %50"
        );
        assert!(t.lines().count() <= nuxut::ui::TOOLTIP_MAX_LINES);
        assert_eq!(tooltip(&Snapshot::default()), "İşlemci —");
    }
}
