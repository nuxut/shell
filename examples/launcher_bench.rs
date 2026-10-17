//! Başlatıcı aramasının tuş başına süresi: 5000 sentetik uygulama, artan önekli sorgular.
//! Hedef: tuş başına < 2 ms (`cargo run --release --example launcher_bench`).
//! İsteğe bağlı: gerçek sistem kataloğu da ölçülür (`--system`). Emoji kipinin ilk ayrıştırması
//! ve araması da ölçülür.

use std::path::Path;
use std::time::{Duration, Instant};

use nuxut_shell::services::launcher::desktop::{self, Locale};
use nuxut_shell::services::launcher::{Catalog, ScanConfig, Scanner, Searcher, Usage};

const WORDS: &[&str] = &[
    "fire", "fox", "term", "code", "files", "editor", "music", "player", "settings", "image", "viewer", "office",
    "writer", "calc", "mail", "chat", "studio", "graph", "network", "monitor", "disk", "font", "camera", "video",
    "ayarlar", "dosya", "yönetici", "görüntü", "müzik", "çizim",
];

fn synthetic(n: usize) -> Catalog {
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let entries = (0..n).map(|i| {
        let mut word = || WORDS[(next() % WORDS.len() as u64) as usize];
        let name = format!("{} {} {i}", word(), word());
        let text = format!(
            "[Desktop Entry]\nType=Application\nName={name}\nGenericName={} {}\nKeywords={};{};{};\nExec=app-{i} %U\n",
            word(),
            word(),
            word(),
            word(),
            word()
        );
        desktop::parse(&format!("org.example.app{i}.desktop"), Path::new("/x"), &text, &Locale::default()).unwrap()
    });
    Catalog::from_entries(entries, 1)
}

fn measure(label: &str, catalog: &Catalog) {
    let mut usage = Usage::default();
    for (i, app) in catalog.apps.iter().enumerate().step_by(7) {
        for _ in 0..(i % 5) {
            usage.record(&app.entry.id, 1_000_000);
        }
    }
    let mut searcher = Searcher::new();
    let queries = ["f", "fi", "fir", "fire", "firef", "", "t", "te", "ter", "term", "s", "se", "set", "sett", "ç", "çi", "çiz"];
    // Isınma.
    for q in queries {
        searcher.search(catalog, &usage, q, 1_000_000);
    }
    let mut worst = Duration::ZERO;
    let mut total = Duration::ZERO;
    let rounds = 20;
    for _ in 0..rounds {
        for q in queries {
            let t = Instant::now();
            let n = searcher.search(catalog, &usage, q, 1_000_000).len();
            std::hint::black_box(n);
            let e = t.elapsed();
            worst = worst.max(e);
            total += e;
        }
    }
    let avg = total / (rounds * queries.len() as u32);
    println!("{label}: {} uygulama, tuş başına ort. {:.3} ms, en kötü {:.3} ms", catalog.len(), avg.as_secs_f64() * 1e3, worst.as_secs_f64() * 1e3);
}

fn main() {
    measure("sentetik", &synthetic(5000));
    measure_emoji();
    if std::env::args().any(|a| a == "--system") {
        let t = Instant::now();
        let catalog = Scanner::new().scan(&ScanConfig::from_env());
        println!("sistem taraması: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        measure("sistem", &catalog);
    }
}

/// Emoji kipi: gömülü verinin ilk ayrıştırması ve tuş başına arama (iki dil).
fn measure_emoji() {
    use nuxut_shell::services::launcher::emoji::{self, EmojiSearcher, Lang};
    let t = Instant::now();
    let n = emoji::all().len();
    println!("emoji verisi: {n} emoji, ilk ayrıştırma {:.2} ms", t.elapsed().as_secs_f64() * 1e3);
    let usage = Usage::default();
    let mut searcher = EmojiSearcher::new();
    let queries: Vec<String> = ["kırmızı kalp", "red heart", "türkiye", "smiling face"]
        .iter()
        .flat_map(|w| (1..=w.chars().count()).map(move |k| w.chars().take(k).collect::<String>()))
        .collect();
    let (mut total, mut worst) = (Duration::ZERO, Duration::ZERO);
    let rounds = 20u32;
    for _ in 0..rounds {
        for q in &queries {
            let t = Instant::now();
            std::hint::black_box(searcher.search(&usage, q, Lang::Tr, 1_000_000).len());
            let e = t.elapsed();
            worst = worst.max(e);
            total += e;
        }
    }
    let avg = total / (rounds * queries.len() as u32);
    println!("emoji araması: tuş başına ort. {:.3} ms, en kötü {:.3} ms", avg.as_secs_f64() * 1e3, worst.as_secs_f64() * 1e3);
}
