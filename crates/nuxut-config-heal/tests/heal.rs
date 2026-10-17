//! Onarım hattı: örnek bir ayar türüyle uçtan uca.

use std::collections::{BTreeMap, BTreeSet};

use nuxut_config_heal::{ActionKind, Class, Healed, Labels, Loaded, Rules, Schema, heal, load_serde, repair_syntax};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
struct Config {
    config_version: Option<toml::Value>,
    bar: Bar,
    frame: Frame,
    outputs: BTreeMap<String, Output>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
struct Bar {
    size: f32,
    edge: Edge,
    start: Vec<String>,
    mode: String,
}

impl Default for Bar {
    fn default() -> Self {
        Self { size: 30.0, edge: Edge::Top, start: vec!["clock".into()], mode: "auto".into() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Edge {
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
struct Frame {
    thickness: f32,
    enabled: bool,
}

impl Default for Frame {
    fn default() -> Self {
        Self { thickness: 6.0, enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
struct Output {
    fit: String,
}

/// Yükleyici: ayrıştır + doğrula (aralık kırpma, bilinmeyen kipte varsayılan).
fn load(text: &str) -> Loaded<Config> {
    match load_serde::<Config>(text) {
        Loaded::Ok { mut config, unknown } => {
            config.config_version = None;
            config.frame.thickness = config.frame.thickness.clamp(0.0, 64.0);
            if !["auto", "always"].contains(&config.bar.mode.as_str()) {
                config.bar.mode = "auto".into();
            }
            Loaded::Ok { config, unknown }
        }
        other => other,
    }
}

fn rules() -> Rules {
    Rules {
        schema: Schema::of::<Config>(),
        renames: vec![("bar.left".into(), "bar.start".into()), ("clock".into(), "bar.clock".into())],
        retired: vec!["frame.inner_radius".into()],
        protected: vec!["config_version".into()],
        defaults: [("bar.mode".to_owned(), "\"auto\"".to_owned())].into(),
        limits: [("frame.thickness".to_owned(), (0.0, 64.0))].into(),
    }
}

fn labels() -> Labels {
    Labels {
        stamp: "2026-10-16".into(),
        broken: "okunamadı".into(),
        duplicate: "tekrar".into(),
        invalid: "geçersiz".into(),
        clamped: "sınır dışı".into(),
    }
}

fn run(text: &str) -> Healed {
    run_keep(text, &BTreeSet::new())
}

fn run_keep(text: &str, keep: &BTreeSet<String>) -> Healed {
    let h = heal(text, &rules(), keep, &labels(), &load);
    // İdempotent: ikinci onarım hiçbir şeyi değiştirmez.
    let again = heal(&h.text, &rules(), keep, &labels(), &load);
    assert!(!again.edited(), "ikinci onarım değiştirdi:\n{}\n{:?}", h.text, again.actions);
    assert_eq!(again.text, h.text);
    h
}

fn kinds(h: &Healed) -> Vec<(String, ActionKind)> {
    h.actions.iter().map(|a| (a.key.clone(), a.kind.clone())).collect()
}

#[test]
fn clean_file_is_untouched() {
    let text = "config_version = 2\n# benim barım\n[bar]\nsize = 40 # büyük\nedge = \"bottom\"\n";
    let h = run(text);
    assert_eq!(h.text, text);
    assert!(h.actions.is_empty());
}

#[test]
fn unknown_retired_and_unused_keys_are_removed_comments_stay() {
    let text = "\
config_version = 2
[bar]
size = 40
# eski bir şey
nonsense_key = 1 # not
[frame]
inner_radius = 4
[my_notes]
a = 1
";
    let h = run(text);
    assert_eq!(
        h.text,
        "\
config_version = 2
[bar]
size = 40
# eski bir şey
# not
"
    );
    assert_eq!(
        kinds(&h),
        [
            ("my_notes".into(), ActionKind::Unused),
            ("bar.nonsense_key".into(), ActionKind::Unused),
            ("frame.inner_radius".into(), ActionKind::Retired),
            ("frame".into(), ActionKind::Redundant),
        ]
    );
    // Boşalan `[frame]` başlığı da gider; `[bar]` yorum ve değer tuttuğu için kalır.
}

#[test]
fn typo_is_suggested_not_removed() {
    let text = "[bar]\nsiez = 40\n";
    let h = run(text);
    assert_eq!(h.text, text);
    assert_eq!(kinds(&h), [("bar.siez".into(), ActionKind::Suggest { candidate: "size".into() })]);
    assert_eq!(h.count(Class::Suggestion), 1);
    assert!(!h.edited());
    // Aday zaten yazılıysa öneri yok: etkisiz satır silinir.
    let h = run("[bar]\nsize = 40\nsiez = 41\n");
    assert_eq!(h.text, "[bar]\nsize = 40\n");
}

#[test]
fn renames_move_old_names() {
    let h = run("[bar]\n# sol\nleft = [\"a\"] # x\nsize = 40\n");
    assert_eq!(h.text, "[bar]\n# sol\nstart = [\"a\"] # x\nsize = 40\n");
    assert_eq!(kinds(&h), [("bar.left".into(), ActionKind::Moved { to: "bar.start".into() })]);
    // Yeni ad zaten varsa eski satır etkisizdi: silinir.
    let h = run("[bar]\nleft = [\"a\"]\nstart = [\"b\"]\n");
    assert_eq!(h.text, "[bar]\nstart = [\"b\"]\n");
    assert_eq!(kinds(&h), [("bar.left".into(), ActionKind::Retired)]);
}

#[test]
fn invalid_value_falls_back_to_default_and_is_kept_as_comment() {
    let text = "[bar]\nsize = 40\nedge = \"middle\"\n[frame]\nenabled = \"yes\"\n";
    let h = run(text);
    assert_eq!(
        h.text,
        "[bar]\nsize = 40\n# geçersiz (2026-10-16)\n# edge = \"middle\"\n[frame]\n# geçersiz (2026-10-16)\n# enabled = \"yes\"\n"
    );
    assert_eq!(kinds(&h), [("bar.edge".into(), ActionKind::Invalid), ("frame.enabled".into(), ActionKind::Invalid)]);
    // Ayar yüklenebiliyor ve diğer değerler korunuyor.
    let Loaded::Ok { config, .. } = load(&h.text) else { panic!() };
    assert_eq!(config.bar.size, 40.0);
}

#[test]
fn value_ignored_by_validation_is_invalid_and_default_equal_is_redundant() {
    let h = run("[bar]\nmode = \"sometimes\"\nsize = 30\n");
    assert_eq!(h.text, "[bar]\n# geçersiz (2026-10-16)\n# mode = \"sometimes\"\n");
    assert_eq!(kinds(&h), [("bar.mode".into(), ActionKind::Invalid), ("bar.size".into(), ActionKind::Redundant)]);
}

#[test]
fn redundant_lines_with_comments_are_kept_and_empty_tables_go() {
    let h = run("[bar]\n# hep 30 olsun\nsize = 30\n[frame]\nthickness = 6.0\nenabled = true\n\n[outputs.\"DP-1\"]\nfit = \"cover\"\n");
    assert_eq!(h.text, "[bar]\n# hep 30 olsun\nsize = 30\n\n[outputs.\"DP-1\"]\nfit = \"cover\"\n");
    // İki varsayılana eşit satır + boşalan `[frame]` başlığı.
    assert_eq!(h.count(Class::Quiet), 3);
}

#[test]
fn out_of_range_number_is_clamped_old_value_commented() {
    let h = run("[frame]\nthickness = 500 # kalın\n");
    assert_eq!(h.text, "[frame]\n# sınır dışı (2026-10-16): thickness = 500 # kalın\nthickness = 64 # kalın\n");
    assert_eq!(kinds(&h), [("frame.thickness".into(), ActionKind::Clamped { to: "64".into() })]);
}

#[test]
fn keep_set_protects_undone_keys() {
    let keep: BTreeSet<String> = ["my_notes".to_owned(), "bar.size".to_owned()].into();
    let text = "[bar]\nsize = 30\n[my_notes]\na = 1\n";
    let h = run_keep(text, &keep);
    assert_eq!(h.text, text);
}

#[test]
fn duplicate_key_last_wins() {
    let h = run("[bar]\nsize = 40\nedge = \"bottom\"\nsize = 50\n");
    assert_eq!(h.text, "[bar]\n# tekrar (2026-10-16)\n# size = 40\nedge = \"bottom\"\nsize = 50\n");
    assert_eq!(kinds(&h), [("bar.size".into(), ActionKind::Duplicate)]);
}

#[test]
fn duplicate_table_is_merged() {
    let h = run("[bar]\nsize = 40\n\n[frame]\nthickness = 3.0\n\n[bar]\nedge = \"bottom\"\nsize = 50\n");
    assert_eq!(
        h.text,
        "[bar]\n# tekrar (2026-10-16)\n# size = 40\n\nedge = \"bottom\"\nsize = 50\n[frame]\nthickness = 3.0\n"
    );
    assert_eq!(kinds(&h), [("bar".into(), ActionKind::MergedTable), ("bar.size".into(), ActionKind::Duplicate)]);
}

#[test]
fn broken_lines_are_commented_rest_survives() {
    let h = run("[bar\nsize = 40\n[frame]\nthickness = = 3\nenabled = false\n");
    assert_eq!(
        h.text,
        "[bar]\nsize = 40\n[frame]\n# okunamadı (2026-10-16)\n# thickness = = 3\nenabled = false\n"
    );
    assert_eq!(h.count(Class::Repaired), 2);
}

#[test]
fn unfixable_header_takes_its_section_along() {
    let h = run("[frame]\nthickness = 3.0\n[ba r]\nsize = 40\n[outputs.x]\nfit = \"c\"\n");
    assert_eq!(
        h.text,
        "[frame]\nthickness = 3.0\n# okunamadı (2026-10-16)\n# [ba r]\n# size = 40\n[outputs.x]\nfit = \"c\"\n"
    );
}

#[test]
fn unclosed_array_is_commented_from_its_start() {
    let h = run("[bar]\nstart = [\"a\",\n  \"b\",\nsize = 40\n[frame]\nthickness = 3.0\n");
    assert_eq!(
        h.text,
        "[bar]\n# okunamadı (2026-10-16)\n# start = [\"a\",\n#   \"b\",\nsize = 40\n[frame]\nthickness = 3.0\n"
    );
    let h = run("[bar]\nsize = 40\nstart = [\"a\",\n");
    assert_eq!(h.text, "[bar]\nsize = 40\n# okunamadı (2026-10-16)\n# start = [\"a\",\n");
}

#[test]
fn garbage_never_panics_and_terminates() {
    for text in [
        "",
        "\n\n",
        "[",
        "]]]]",
        "a = \"\"\"\nnever closed",
        "= = =\n[[[\n",
        "[bar]\n[bar]\n[bar]\n",
        "x = {a = 1, a = 2}\n",
        "[[outputs]]\nfit = 1\n[[outputs]]\n",
        "bar = 1\n[bar]\nsize = 2\n",
        "\u{feff}[bar]\nsize = 40\n",
        "[bar]\nsize = 40\r\nedge = \"x\"\r\n",
    ] {
        let h = heal(text, &rules(), &BTreeSet::new(), &labels(), &load);
        let _ = heal(&h.text, &rules(), &BTreeSet::new(), &labels(), &load);
        let _ = repair_syntax(text, &labels());
    }
}

#[test]
fn schema_children_power_suggestions_in_maps() {
    let h = run("[outputs.\"DP-1\"]\nfti = \"cover\"\n");
    assert_eq!(kinds(&h), [("outputs.DP-1.fti".into(), ActionKind::Suggest { candidate: "fit".into() })]);
}
