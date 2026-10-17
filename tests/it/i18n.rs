//! i18n kapıları (ADR-0010 §4): katalog (tr/en tam, yorum, değişken, [max], önek) ve gömülü
//! kullanıcı metni bekçisi (`i18n-allow.txt` geçici listesi aşama B'de küçülür).
//!
//! Yollar çalışma zamanında çözülür (`support::repo`): bayat ikili başka bir worktree'nin
//! ağacını tarayıp sessizce geçmesin.

use crate::support::repo;

#[test]
fn i18n_catalog() {
    let dir = repo::repo_path("locales");
    nuxut::i18n::check::catalog_embedded(&nuxut::i18n::embed!("shell"), dir, "shell").assert_ok();
}

#[test]
fn i18n_source_guard() {
    let root = repo::manifest_dir();
    // Bekçi `src/**/*.rs` tarar; ağaç boş/yanlışsa sessiz yeşil vermesin.
    repo::assert_enough_sources(&root, repo::count_rust_files(&root.join("src")), repo::MIN_SHELL_SOURCES);
    nuxut::i18n::check::source_guard(&root).assert_ok();
}
