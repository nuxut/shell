//! Canlı duman testi yardımcısı (`msg snapshot list|show|restore`): geçici bir kökte
//! `bar.size` için tek bir snapshot üretir ve sonucu JSON olarak yazdırır.
//!
//! Yalnızca `nuxut_apply::Env::for_test(<kök>)` kullanır: test örneği kapısı yazımı `<kök>/config`
//! ve `<kök>/state` altına sınırlar (kök gerçek ev dizini dışında olmalı, yoksa `skipped`).
//! Shell aynı köklerle başlatılır: `XDG_CONFIG_HOME=<kök>/config`, `XDG_STATE_HOME=<kök>/state`.
//!
//! ```sh
//! cargo run --example snapshot_seed -- /tmp/.../kök
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use nuxut_apply::{Applier, Change, Env, FragValue, Fragment, KeyPath, SettingId, Source};

fn main() -> ExitCode {
    let Some(root) = std::env::args_os().nth(1).map(PathBuf::from).filter(|p| p.is_absolute()) else {
        eprintln!("kullanım: snapshot_seed <mutlak geçici kök>");
        return ExitCode::from(2);
    };
    let (Ok(setting), Ok(key)) = (SettingId::new("bar.size"), KeyPath::dotted("bar.size")) else {
        return ExitCode::FAILURE;
    };
    let file = root.join("config/nuxut/shell.toml");
    let change = Change::new(setting, "duman testi", Source::Settings)
        .write(Fragment::TomlKey { file, key }, FragValue::Text("44".into()));
    let outcome = Applier::open(Env::for_test(&root)).apply(change);
    println!("{}", serde_json::to_string(&outcome).unwrap_or_default());
    if outcome.name() == "applied" { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
