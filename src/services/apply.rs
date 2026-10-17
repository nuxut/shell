//! Shell ile `nuxut-apply` (framework, shell ADR-0001) arasındaki ince yapıştırıcı.
//!
//! Shell'in kendi dosyalarına ve kullanıcının dosyalarına yazdığı her şey framework'ün tek atomik
//! yazıcısından (`nuxut_apply::write_file`) ya da işlem katmanından (`Applier`) geçer; ortam
//! kapısı (`Env::guard`: test örneği kullanıcının dosyalarına asla yazmaz) orada. Burada yalnızca
//! shell'e özgü kararlar var:
//!
//! - Snapshot ayar kimliklerinin kapalı listesi (ADR-0001 §9.3): [`known_setting`]. Shell'in
//!   kendi aldıkları: [`HYPR_SOURCE`], [`MIGRATION_SHELL`].
//! - Nuxut'un kendi küçük dosyaları (setup.toml, Hyprland durum işareti): [`write_own`] — dosya
//!   yoksa yaratır, varsa okunduğu hâl üzerinden değiştirir; snapshot yok.
//! - Türetilmiş dosyalar (displays.toml, `displays.lua`/`.conf`): [`write_derived`] — yalnızca
//!   yoksa ya da Nuxut başlığı taşıyorsa yazar; yabancı içerik `externally_managed`.
//! - Yönetilen dosya tanıma: [`is_managed`] (framework `edit::managed`; Lua `--` yorumu için
//!   framework talebi açık, bkz. işlev belgesi).
//! - `msg snapshot list|show|restore` gövdeleri: [`snapshot_list`], [`snapshot_show`],
//!   [`snapshot_restore`] (tek satır JSON; değer/içerik geçmez).
//!
//! Üretimde `Env::from_process(instance)` bir kez (main), testlerde `Env::for_test(root)`.

use std::path::Path;
use std::sync::OnceLock;

use nuxut_apply::{
    Applier, ApplyError, Env, Expect, Filter, RestoreMode, RestoreOutcome, RestoredValues, SettingId, SnapshotId, Source,
    TargetKind,
    Written, edit::managed,
};

/// Kullanıcının Hyprland ana dosyasındaki `source`/`require` satırı (ve ilk dokunuşta yabancı
/// `nuxut.conf`/`nuxut.lua` içeriği).
pub const HYPR_SOURCE: &str = "hypr.source";
/// shell.toml sürüm göçü (ADR-0002).
pub const MIGRATION_SHELL: &str = "migration.shell";
/// Kullanıcının kendi VM pano köprüsü betiğini başlatan Hyprland satırının devralınması
/// (`services::clipboard::takeover`).
pub const CLIPBOARD_BRIDGE: &str = "hypr.clipboard_bridge";
/// Birleşik görünüm kimlikleri (ADR-0004; Ayarlar ve `look` yazar, shell burada yalnızca tanır).
pub const LOOK_SETTINGS: [&str; 7] = [
    "look.unified",
    "look.mode",
    "look.icons",
    "look.cursor",
    "look.fonts",
    "look.motion",
    "look.qt_backend",
];
/// Varsayılan uygulamalar (sözleşme defaults-v1; `services::default_apps`): rol başına bir kimlik.
pub const DEFAULTS_SETTINGS: [&str; 9] = [
    "defaults.browser",
    "defaults.email",
    "defaults.file_manager",
    "defaults.terminal",
    "defaults.text_editor",
    "defaults.image",
    "defaults.video",
    "defaults.music",
    "defaults.pdf",
];

/// `msg snapshot list --limit` üst sınırı (framework `Filter` ile aynı).
pub const MAX_LIST_LIMIT: usize = 200;

/// Snapshot'ı alınmayan kendi dosyası: varsa okunan hâl hâlâ geçerliyse değiştirir, yoksa
/// yaratır (dizinler `0755`). İçerik aynıysa dokunmaz (`Unchanged`). Kapı, symlink koruması,
/// boyut ve yarış denetimi framework'te.
pub fn write_own(env: &Env, path: &Path, bytes: &[u8]) -> Result<Written, ApplyError> {
    match nuxut_apply::write_file(env, path, bytes, None) {
        Err(ApplyError::NotFound(_)) => nuxut_apply::write_file(env, path, bytes, Some(&Expect::Absent)),
        other => other,
    }
}

/// Türetilmiş (tümü Nuxut'un) dosya: yoksa yaratır, Nuxut başlığı ([`is_managed`]) taşıyorsa
/// okunduğu hâl üzerinden değiştirir. Başlıksız (kullanıcının) içerik **yazılmaz**:
/// `ExternallyManaged` (eski `.bak` yedeğinin yerine; ADR-0001 §11-3).
pub fn write_derived(env: &Env, path: &Path, bytes: &[u8]) -> Result<Written, ApplyError> {
    match nuxut_apply::read_bytes(path) {
        Ok((current, expect)) => {
            if !is_managed(&current) {
                return Err(ApplyError::ExternallyManaged(path.to_path_buf()));
            }
            nuxut_apply::write_file(env, path, bytes, Some(&expect))
        }
        Err(ApplyError::NotFound(_)) => nuxut_apply::write_file(env, path, bytes, Some(&Expect::Absent)),
        Err(e) => Err(e),
    }
}

/// Türetilmiş dosyayı siler: yalnızca Nuxut başlığı taşıyorsa (yoksa `Unchanged`). Yabancı
/// içerik silinmez (`ExternallyManaged`).
pub fn remove_derived(env: &Env, path: &Path) -> Result<Written, ApplyError> {
    match nuxut_apply::read_bytes(path) {
        Ok((current, expect)) if is_managed(&current) => nuxut_apply::remove_file(env, path, Some(&expect)),
        Ok(_) => Err(ApplyError::ExternallyManaged(path.to_path_buf())),
        Err(ApplyError::NotFound(_)) => Ok(Written::Unchanged),
        Err(e) => Err(e),
    }
}

/// Kapı geçerse `dir`'i (yoksa) `0700` yaratır: durum dizini yalnızca kullanıcının
/// (`write_file` yeni dizinleri `0755` açar; framework talebi: dizin kipi seçeneği).
pub fn ensure_private_dir(env: &Env, dir: &Path) -> Result<(), ApplyError> {
    use std::os::unix::fs::DirBuilderExt;
    env.guard(TargetKind::File)?;
    if !dir.is_absolute() || dir.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(ApplyError::Invalid("dizin yolu mutlak olmalı"));
    }
    // Test örneğinde yalnızca kendi kökleri altında (kapının kök kuralıyla aynı yer).
    if env.test_instance {
        let inside = [env.config_home.as_deref(), env.state_home.as_deref()]
            .into_iter()
            .flatten()
            .any(|root| dir.starts_with(root));
        if !inside {
            return Err(ApplyError::Skipped(nuxut_apply::Skip::TestInstance));
        }
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(ApplyError::from)
}

/// İlk satır bir Nuxut başlığı mı (`# Nuxut — `, `/* `, `// ` ve Lua `-- `)?
///
/// Framework `edit::managed::is_managed` Lua'nın `--` yorumunu tanımıyor (framework talebi:
/// `CommentStyle::DoubleDash`); o gelene kadar Lua başlığı framework'ün `HEADER_MARK`'ı ile
/// burada tanınır. Başlık metninin kendisi framework'ün.
pub fn is_managed(bytes: &[u8]) -> bool {
    if managed::is_managed(bytes) {
        return true;
    }
    let first = bytes.split(|&b| b == b'\n').next().unwrap_or_default();
    std::str::from_utf8(first)
        .ok()
        .and_then(|l| l.trim_start().strip_prefix("-- "))
        .is_some_and(|rest| rest.starts_with(managed::HEADER_MARK))
}

/// Kullanıcıya/IPC'ye gösterilecek kısa hata (yol ve değer içermez; ADR-0001 §10.3 adları).
pub fn error_text(e: &ApplyError) -> String {
    match e {
        ApplyError::Skipped(s) => format!("skipped({})", s.name()),
        other => other.kind_name().to_owned(),
    }
}

/// Sabit kimlikten `SettingId` (kimlikler bu modülde, geçerli oldukları testli).
pub fn setting(id: &'static str) -> SettingId {
    SettingId::new(id).unwrap_or_else(|_| unreachable!("sabit ayar kimliği geçersiz: {id}"))
}

/// shell.toml şemasının ayar kimlikleri (`nuxut-shell schema`; bileşen ayarları
/// `bar.widget.<ad>.<anahtar>`). Bir kez hesaplanır.
fn schema_ids() -> &'static [String] {
    static IDS: OnceLock<Vec<String>> = OnceLock::new();
    IDS.get_or_init(|| {
        let mut out = Vec::new();
        crate::config::ShellConfig::schema(&mut out);
        let mut ids: Vec<String> = out.into_iter().map(|s| s.key).collect();
        for kind in crate::widgets::WidgetKind::ALL {
            let mut settings = Vec::new();
            kind.settings_schema(&mut settings);
            ids.extend(settings.into_iter().map(|s| format!("bar.widget.{}.{}", kind.name(), s.key)));
        }
        ids
    })
}

/// Kapalı listedeki snapshot ayar kimliği mi (ADR-0001 §9.3)? Geçerliyse `SettingId`.
/// Liste: `look.*` (6), `defaults.*` (9), `hypr.source`, `migration.shell` ve shell.toml şema
/// kimlikleri.
/// `hypr.<grup>` ve `displays` kimlik değildir.
pub fn known_setting(id: &str) -> Option<SettingId> {
    let known = LOOK_SETTINGS.contains(&id) || DEFAULTS_SETTINGS.contains(&id) || [HYPR_SOURCE, MIGRATION_SHELL, CLIPBOARD_BRIDGE].contains(&id) || schema_ids().iter().any(|s| s == id);
    known.then(|| SettingId::new(id).ok()).flatten()
}

fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{\"result\":\"error\",\"kind\":\"io\"}".into())
}

/// `snapshot list`: yeniden eskiye, tek satır JSON dizi (değer yok). Hata: §10.3 adı.
pub fn snapshot_list(env: &Env, setting: Option<SettingId>, limit: usize) -> Result<String, String> {
    let filter = Filter { setting, limit: limit.clamp(1, MAX_LIST_LIMIT) };
    match Applier::open(env.clone()).list(&filter) {
        Ok(rows) => Ok(json(&rows)),
        // Depo hiç yoksa (henüz snapshot alınmamış) boş liste.
        Err(ApplyError::NotFound(_)) => Ok("[]".into()),
        Err(e) => Err(error_text(&e)),
    }
}

/// `snapshot show <id>`: tek snapshot (parça türü, hedef `~` ile, önce/sonra var mı).
pub fn snapshot_show(env: &Env, id: &SnapshotId) -> Result<String, String> {
    match Applier::open(env.clone()).show(id) {
        Ok(Some(detail)) => Ok(json(&detail)),
        Ok(None) | Err(ApplyError::NotFound(_)) => Err("not_found".into()),
        Err(e) => Err(error_text(&e)),
    }
}

/// `snapshot restore` sonucu: yanıt JSON'u ve (bir şey yazıldıysa) geri yüklenen ayar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub json: String,
    /// Yazım oldu (`restored` ya da `partial`): shell'in canlı uygulaması gereken ayar.
    pub setting: Option<SettingId>,
    /// Gerçekten geri yazılan değerler (canlı eşitleme için, ör. imleç; ADR-0004 §14-5). `Debug`
    /// içerik göstermez; JSON'a girmez.
    pub values: RestoredValues,
}

/// `snapshot restore <id> [--force] [--setting <ayar>]`: üç yollu geri yükleme (ADR-0001 §7).
/// `expect_setting` verilmişse snapshot o ayara ait değilse hiçbir şey yazılmaz
/// (`{"result":"not_found"}`). Bloklayan IO (kilit ≤ 5 sn): ana thread'de çağrılmaz.
pub fn snapshot_restore(env: &Env, id: &SnapshotId, mode: RestoreMode, expect_setting: Option<&SettingId>) -> Restored {
    let app = Applier::open(env.clone()).with_source(Source::Cli);
    let setting = match app.show(id) {
        Ok(Some(d)) => d.setting,
        Ok(None) | Err(ApplyError::NotFound(_)) => return Restored { json: json(&RestoreOutcome::NotFound), setting: None, values: RestoredValues::default() },
        Err(ApplyError::Busy) => return Restored { json: json(&RestoreOutcome::Busy), setting: None, values: RestoredValues::default() },
        Err(ApplyError::Skipped(reason)) => return Restored { json: json(&RestoreOutcome::Skipped { reason }), setting: None, values: RestoredValues::default() },
        Err(e) => return Restored { json: json(&RestoreOutcome::Error { kind: e.kind_name() }), setting: None, values: RestoredValues::default() },
    };
    if expect_setting.is_some_and(|s| *s != setting) {
        return Restored { json: json(&RestoreOutcome::NotFound), setting: None, values: RestoredValues::default() };
    }
    let (out, values) = app.restore_with_values(id, mode);
    let wrote = matches!(out, RestoreOutcome::Restored { id: Some(_) } | RestoreOutcome::Partial { .. });
    Restored { json: json(&out), setting: wrote.then_some(setting), values }
}

#[cfg(test)]
mod tests {
    use nuxut_apply::{Change, FragValue, Fragment, KeyPath};

    use super::*;

    #[test]
    fn setting_ids_are_valid_and_closed() {
        let mut look: Vec<&str> = crate::services::look::plan::setting::ALL.to_vec();
        let mut known = LOOK_SETTINGS.to_vec();
        look.sort_unstable();
        known.sort_unstable();
        assert_eq!(look, known, "görünüm planının her ayarı tanınır (snapshot geri alma)");
        for id in [HYPR_SOURCE, MIGRATION_SHELL, CLIPBOARD_BRIDGE].into_iter().chain(LOOK_SETTINGS).chain(DEFAULTS_SETTINGS) {
            assert!(SettingId::new(id).is_ok(), "{id}");
            assert!(known_setting(id).is_some(), "{id}");
        }
        for id in ["bar.size", "compositor.sync_look", "bar.widget.workspaces.count"] {
            assert!(known_setting(id).is_some(), "şema kimliği: {id}");
        }
        for id in ["hypr.look", "displays", "defaults", "defaults.nope", "look", "bar", "bar.nope", "", "Bar.size", "x".repeat(300).as_str()] {
            assert!(known_setting(id).is_none(), "kapalı liste dışında: {id}");
        }
    }

    #[test]
    fn managed_header_detection_includes_lua() {
        assert!(is_managed("# Nuxut — x\nrest".as_bytes()));
        assert!(is_managed("-- Nuxut — nuxut-shell tarafından üretildi\n".as_bytes()));
        assert!(!is_managed(b"-- kullanici\n"));
        assert!(!is_managed("a\n-- Nuxut — x".as_bytes()));
        assert!(!is_managed(b""));
    }

    #[test]
    fn write_own_creates_then_replaces_and_respects_gate() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::for_test(t.path());
        let f = t.path().join("state/nuxut/x.toml");
        assert_eq!(write_own(&env, &f, b"a\n"), Ok(Written::Written));
        assert_eq!(write_own(&env, &f, b"a\n"), Ok(Written::Unchanged));
        assert_eq!(write_own(&env, &f, b"b\n"), Ok(Written::Written));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "b\n");
        // Kök dışı (test örneği kapısı): yazılmaz, dosya açılmaz.
        let outside = t.path().join("elsewhere/x.toml");
        assert!(matches!(write_own(&env, &outside, b"x"), Err(ApplyError::Skipped(_))));
        assert!(!outside.exists());
        // Kapalı ortam (Shell::new varsayılanı) hiçbir şey yazmaz.
        let off = Env { disabled: true, ..Env::for_test(t.path()) };
        assert!(matches!(write_own(&off, &f, b"c\n"), Err(ApplyError::Skipped(_))));
        assert_eq!(error_text(&ApplyError::Skipped(nuxut_apply::Skip::TestInstance)), "skipped(test_instance)");
        assert_eq!(error_text(&ApplyError::Busy), "busy");
    }

    #[test]
    fn derived_files_never_overwrite_foreign_content() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::for_test(t.path());
        let f = t.path().join("config/hypr/nuxut/displays.lua");
        assert_eq!(write_derived(&env, &f, "-- Nuxut — a\n".as_bytes()), Ok(Written::Written));
        assert_eq!(write_derived(&env, &f, "-- Nuxut — b\n".as_bytes()), Ok(Written::Written));
        std::fs::write(&f, "kullanıcı\n").unwrap();
        assert!(matches!(write_derived(&env, &f, "-- Nuxut — c\n".as_bytes()), Err(ApplyError::ExternallyManaged(_))));
        assert!(matches!(remove_derived(&env, &f), Err(ApplyError::ExternallyManaged(_))));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "kullanıcı\n");
        std::fs::write(&f, "# Nuxut — d\n").unwrap();
        assert_eq!(remove_derived(&env, &f), Ok(Written::Written));
        assert_eq!(remove_derived(&env, &f), Ok(Written::Unchanged));
        assert!(!f.exists());
    }

    #[test]
    fn private_dir_is_0700_and_gated() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let env = Env::for_test(t.path());
        let d = t.path().join("state/nuxut");
        ensure_private_dir(&env, &d).unwrap();
        assert_eq!(std::fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
        let outside = t.path().join("x/y");
        assert!(matches!(ensure_private_dir(&env, &outside), Err(ApplyError::Skipped(_))));
        assert!(!outside.exists());
        let off = Env { disabled: true, ..env };
        assert!(ensure_private_dir(&off, &t.path().join("state/z")).is_err());
        assert!(!t.path().join("state/z").exists());
    }

    /// `snapshot list/show/restore` gövdeleri: JSON biçimi, kapalı liste süzgeci, `--setting`
    /// uyuşmazlığı, değer sızmaz.
    #[test]
    fn snapshot_commands_roundtrip() {
        let t = tempfile::tempdir().unwrap();
        let env = Env::for_test(t.path());
        assert_eq!(snapshot_list(&env, None, 50), Ok("[]".into()));
        let file = t.path().join("config/nuxut/shell.toml");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "[bar]\nsize = 30\n").unwrap();
        let change = Change::new(setting(MIGRATION_SHELL), "göç", Source::Migration)
            .write(Fragment::TomlKey { file: file.clone(), key: KeyPath::dotted("bar.size").unwrap() }, FragValue::Text("44".into()));
        let id = match Applier::open(env.clone()).apply(change) {
            nuxut_apply::Outcome::Applied { id } => id,
            other => panic!("{other:?}"),
        };
        let list: serde_json::Value = serde_json::from_str(&snapshot_list(&env, None, 50).unwrap()).unwrap();
        assert_eq!(list[0]["id"], id.as_str());
        assert_eq!(list[0]["setting"], MIGRATION_SHELL);
        assert!(list[0]["at"].is_string() && list[0]["fragments"] == 1);
        let filtered = snapshot_list(&env, known_setting(HYPR_SOURCE), 50).unwrap();
        assert_eq!(filtered, "[]");
        let show = snapshot_show(&env, &id).unwrap();
        // Değerler sızmaz: önce/sonra yalnızca durum adı. (Ham "44"/"30" araması kimlikteki zaman
        // damgasına denk gelip kararsızdı.)
        let shown: serde_json::Value = serde_json::from_str(&show).unwrap();
        let frag = &shown["fragments"][0];
        assert!(frag["kind"] == "toml_key" && frag["before"] == "set" && frag["after"] == "set", "{show}");
        assert!(!show.contains("size = ") && !show.contains("\"44\"") && !show.contains("\"30\""), "{show}");
        let missing = SnapshotId::parse("20200101T000000Z-0000").unwrap();
        assert_eq!(snapshot_show(&env, &missing), Err("not_found".into()));

        // Yanlış ayarla geri yükleme hiçbir şey yazmaz.
        let wrong = snapshot_restore(&env, &id, RestoreMode::Safe, known_setting(HYPR_SOURCE).as_ref());
        assert_eq!((wrong.json.as_str(), wrong.setting), ("{\"result\":\"not_found\"}", None));
        assert!(std::fs::read_to_string(&file).unwrap().contains("44"));
        // Kullanıcı sonradan değiştirdi: güvenli kip çakışma, zorla geri yükler.
        std::fs::write(&file, "[bar]\nsize = 50\n").unwrap();
        let c = snapshot_restore(&env, &id, RestoreMode::Safe, None);
        assert!(c.json.starts_with("{\"result\":\"conflict\"") && c.setting.is_none(), "{}", c.json);
        let r = snapshot_restore(&env, &id, RestoreMode::Force, known_setting(MIGRATION_SHELL).as_ref());
        assert!(r.json.starts_with("{\"result\":\"restored\""), "{}", r.json);
        assert_eq!(r.setting, known_setting(MIGRATION_SHELL));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[bar]\nsize = 30\n");
        let nf = snapshot_restore(&env, &missing, RestoreMode::Safe, None);
        assert_eq!(nf.json, "{\"result\":\"not_found\"}");

        // Kapalı ortam: hata adı.
        let off = Env { disabled: true, ..env };
        assert_eq!(snapshot_list(&off, None, 5), Err("skipped(disabled)".into()));
        assert_eq!(snapshot_restore(&off, &id, RestoreMode::Safe, None).json, "{\"result\":\"skipped\",\"reason\":\"disabled\"}");
    }
}
