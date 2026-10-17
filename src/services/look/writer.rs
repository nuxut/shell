//! Birleşik görünümün gerçek yazım arka ucu (ADR-0004 §4-3, §7, §8, §14): plan → `nuxut-apply`
//! (ayar başına tek `Change` → tek snapshot) ve dosya dışı canlı eylemler.
//!
//! Kapılar (kullanıcının ayarına dokunan her yol buradan geçer):
//! - Dosya ve dconf yazımları yalnızca `nuxut_apply::Applier`'dan; `Env::guard` test örneğinin
//!   kullanıcının dosyalarına/dconf'una yazmasını engeller. Test örneğinde dconf parçaları plandan
//!   düşülür (kapı onları zaten reddeder; düşülmeseler aynı ayarın geçici kökteki dosyaları da
//!   yazılamazdı), dosyalar yalnızca test köklerinde yazılır.
//! - Canlı eylemler [`live_gate`]'ten: test örneği ya da kapalı ortam hiçbir sinyal, `setcursor`
//!   ya da oturum ortamı göndermez (§14-3); `setcursor` ayrıca geçerli bir Hyprland oturumu ister.
//!
//! Gerçek gönderici [`SystemLive`] (zbus oturum veri yolu + Hyprland soketi); testlerde kayıtçı.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use nuxut_apply::{
    Applier, Change, Env, FragValue, Filter, Outcome as ApplyOutcome, Position, RestoreMode, RestoreOutcome, RestoredValues, SettingId, Skip,
    Source, TargetKind,
};

use super::backend::{ApplyReport, ChangeOutcome, ExternalTarget, LookBackend, Outcome, TakenOver};
use super::env::{ENV_FILE, LEGACY_ENV_FILE, WATCHED};
use super::plan::{Dirs, Fragment, LiveAction, LookChange, setting};
use super::selection::valid_theme_name;
use crate::services::hypr_config::control::HyprControl;

/// `msg look status` `writer` adı.
pub const WRITER_NAME: &str = "nuxut-apply";

/// Bir ayarın parçaları tek nuxut-apply işlemine sığmıyor (yazılmadı): `MAX_FRAGMENTS` (256) ya
/// da adres bütçesi `MAX_ADDRESS_BYTES` (64 KiB; parça başına yol + anahtar). `look.mode` ~105
/// parçanın çoğu kdeglobals yolunu taşır → yapılandırma dizini yolu ~600 baytı aşarsa sığmaz.
pub const TOO_MANY_FRAGMENTS: &str = "too_many_fragments";

/// İstek tek işleme sığar mı (nuxut-apply'ın istek doğrulamasıyla aynı ölçüt).
pub fn fits_one_transaction(req: &Change) -> bool {
    nuxut_apply::fragments_fit(req.writes.iter().map(|(f, _)| f))
}

/// Dışarıdan yönetilen dosya yüzünden ayarın hiçbir parçası yazılamadı (`nuxut-apply` sonuç adı).
pub const EXTERNALLY_MANAGED: &str = "externally_managed";

/// Bir ayarda devralma + dışarıda bırakma denemesi üst sınırı (look.mode ~16 dosyaya yazar; dosya
/// başına en çok iki deneme: devral, olmazsa çıkar).
const MAX_EXTERNAL_RETRIES: usize = 40;

/// İki yol aynı dosya mı (doğrudan ya da çözülmüş symlink/ata ile)?
fn same_file(a: &Path, b: &Path) -> bool {
    a == b || resolved(a) == resolved(b)
}

/// Var olan yol kanonik; yoksa var olan üst dizini kanonik + ad.
fn resolved(p: &Path) -> Option<std::path::PathBuf> {
    p.canonicalize().ok().or_else(|| Some(p.parent()?.canonicalize().ok()?.join(p.file_name()?)))
}

/// `nuxut-apply`'ın bildirdiği hedef (`Env::display_path` biçimi, çözülmüş yol olabilir) istekteki
/// hangi dosya? Çözülmüş yol birden çok isteğe uyabilir (aynı tema dosyasına iki bağ:
/// `gtk.css` ve `gtk-dark.css`): henüz devralınmamış olan önce.
fn matching_file(req: &Change, target: &str, env: &Env) -> Option<std::path::PathBuf> {
    let shown = |q: &Path| env.display_path(q) == target;
    let matches = || req.writes.iter().filter_map(|(f, _)| f.file()).filter(|p| shown(p) || resolved(p).is_some_and(|r| shown(&r)));
    matches().find(|p| !req.take_over.iter().any(|t| t == p)).or_else(|| matches().next()).map(Path::to_path_buf)
}

/// Devralma taramasında açılan en çok snapshot (en yeniden; her biri küçük bir JSON okuması).
const MAX_TAKEOVER_SNAPSHOTS: usize = 32;

/// Devralma kaydının `~/…` yolu bugün yine bir symlink mi (kullanıcı ya da araç bağı yeniden
/// kurdu: artık Nuxut'un değil)? Göreli/çözülemeyen yol: hayır.
fn still_linked(env: &Env, shown: &str) -> bool {
    let path = match (shown.strip_prefix("~/"), env.home.as_deref()) {
        (Some(rest), Some(home)) => home.join(rest),
        (None, _) if shown.starts_with('/') => std::path::PathBuf::from(shown),
        _ => return false,
    };
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// `SIGUSR1` yalnızca en az bu kadar süredir çalışan kitty'lere: işleyicisini kurmadan önceki
/// (yeni açılan) süreç sinyalle kapanırdı.
const KITTY_MIN_AGE_SECS: u64 = 2;
/// `/proc` taramasında bakılan en çok girdi.
const MAX_PROC_ENTRIES: usize = 8192;

/// `proc` altında `uid`'e ait, adı `kitty` olan ve en az `min_age` tık önce başlamış süreçler.
/// `now`: açılıştan bu yana tık (`/proc/uptime` × `CLK_TCK`). Okunamayan girdi atlanır.
fn kitty_pids(proc: &Path, uid: u32, now: u64, min_age: u64) -> Vec<i32> {
    use std::os::unix::fs::MetadataExt;
    let Ok(rd) = std::fs::read_dir(proc) else { return Vec::new() };
    rd.take(MAX_PROC_ENTRIES)
        .filter_map(Result::ok)
        .filter_map(|e| Some((e.file_name().to_str()?.parse::<i32>().ok()?, e.path())))
        .filter(|(_, dir)| std::fs::read_to_string(dir.join("comm")).is_ok_and(|c| c.trim_end() == "kitty"))
        .filter(|(_, dir)| std::fs::metadata(dir).is_ok_and(|m| m.uid() == uid))
        .filter(|(_, dir)| {
            // stat: "pid (comm) durum …"; başlangıç anı 22. alan (comm'dan sonra 20.).
            let Ok(stat) = std::fs::read_to_string(dir.join("stat")) else { return false };
            let Some((_, rest)) = stat.rsplit_once(')') else { return false };
            rest.split_whitespace().nth(19).and_then(|s| s.parse::<u64>().ok()).is_some_and(|start| now.saturating_sub(start) >= min_age)
        })
        .map(|(pid, _)| pid)
        .collect()
}

/// D-Bus çağrı zaman aşımı (systemd/D-Bus ortamı; yanıtsız servis thread'i bekletmesin).
pub const DBUS_TIMEOUT: Duration = Duration::from_secs(2);

/// Dosya dışı canlı eylemin göndericisi.
pub trait LiveSink {
    fn run(&mut self, action: &LiveAction) -> Result<(), String>;
}

/// Canlı eylem bu ortamda gönderilebilir mi? Test örneği / kapalı ortam: hiçbiri.
pub fn live_gate(env: &Env, action: &LiveAction) -> Result<(), Skip> {
    let kind = match action {
        LiveAction::SessionEnvironment(_) | LiveAction::SessionUnset(_) => TargetKind::Unit,
        _ => TargetKind::Dconf,
    };
    env.guard(kind)?;
    if matches!(action, LiveAction::HyprSetCursor { .. }) && !env.may_control_hyprland() {
        return Err(Skip::NoSession);
    }
    Ok(())
}

/// Plandaki ayarı `nuxut-apply` isteğine çevirir. `dconf = false` (test örneği): dconf parçaları
/// düşülür. Satır parçaları `write_creating` ile: dosya (`gtk.css`) yoksa yalnızca bizim satırımızla
/// yaratılır; geri yüklemede başka içerik kalmamışsa dosya silinir (nuxut-apply). Önceki sürümün
/// başlıklı yarattığı `gtk.css` diskte olağan bir satır parçası olarak işlenir. Geriye parça
/// kalmazsa `None`.
pub fn to_change(change: &LookChange, dconf: bool) -> Option<Change> {
    let id = SettingId::new(change.setting).ok()?;
    let mut out = Change::new(id, change.label.clone(), Source::Shell);
    for (frag, value) in &change.writes {
        let (f, v) = match frag {
            Fragment::ManagedFile { file } => (
                nuxut_apply::Fragment::ManagedFile { file: file.clone() },
                FragValue::File { bytes: value.as_bytes().to_vec(), mode: 0 },
            ),
            Fragment::Line { file, marker } => {
                let f = nuxut_apply::Fragment::Line { file: file.clone(), marker: marker.clone(), at: Position::End };
                out = out.write_creating(f, FragValue::Text(value.clone()));
                continue;
            }
            Fragment::IniKey { file, section, key } => (
                nuxut_apply::Fragment::IniKey { file: file.clone(), section: section.clone(), key: key.clone() },
                FragValue::Text(value.clone()),
            ),
            Fragment::Dconf { key } if dconf => (nuxut_apply::Fragment::Dconf { key: key.clone() }, FragValue::Text(value.clone())),
            Fragment::Dconf { .. } => continue,
            Fragment::Remove(inner) => match inner.as_ref() {
                Fragment::ManagedFile { file } => (nuxut_apply::Fragment::ManagedFile { file: file.clone() }, FragValue::Absent),
                Fragment::Line { file, marker } => (
                    nuxut_apply::Fragment::Line { file: file.clone(), marker: marker.clone(), at: Position::End },
                    FragValue::Absent,
                ),
                _ => continue,
            },
        };
        out = out.write(f, v);
    }
    (!out.writes.is_empty()).then_some(out)
}

/// Göç: isteğin yazdığı `environment.d/99-nuxut-look.conf`'un yanında önceki sürümün
/// `90-nuxut-look.conf`'u Nuxut'un yönetilen dosyası olarak duruyorsa (`is_ours`), aynı işlemde
/// kaldırılır (`Absent`). Eski işleme birleşirse ya da yeni snapshot alınırsa da kapatma iki dosyayı
/// Nuxut öncesine (yok) döndürür. Kullanıcının elle yazdığı aynı adlı dosyaya dokunulmaz.
pub fn with_legacy_env_removal(mut req: Change, change: &LookChange, is_ours: &dyn Fn(&Path) -> bool) -> Change {
    let legacy: Vec<_> = change
        .writes
        .iter()
        .filter_map(|(f, _)| match f {
            Fragment::ManagedFile { file } if file.file_name().is_some_and(|n| n == ENV_FILE) => {
                Some(file.with_file_name(LEGACY_ENV_FILE))
            }
            _ => None,
        })
        .filter(|p| is_ours(p))
        .collect();
    for file in legacy {
        req = req.write(nuxut_apply::Fragment::ManagedFile { file }, FragValue::Absent);
    }
    req
}

/// Dosya Nuxut'un yönetilen dosyası mı (başlık; okunamazsa hayır). Boyut sınırlı okuma.
fn is_managed_file(p: &Path) -> bool {
    nuxut_apply::read_file(p).is_ok_and(|(text, _)| nuxut_apply::edit::managed::is_managed(text.as_bytes()))
}

/// `nuxut-apply` arka ucu. Bloklayan IO (kilit ≤ 5 sn, dconf/systemd alt süreçleri): yalnızca
/// görünüm thread'inde.
pub struct ApplyBackend<'a> {
    app: Applier,
    env: Env,
    live: &'a mut dyn LiveSink,
}

impl std::fmt::Debug for ApplyBackend<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplyBackend").finish_non_exhaustive()
    }
}

impl<'a> ApplyBackend<'a> {
    pub fn new(env: &Env, live: &'a mut dyn LiveSink) -> Self {
        Self { app: Applier::open(env.clone()).with_source(Source::Shell), env: env.clone(), live }
    }
}

impl LookBackend for ApplyBackend<'_> {
    fn apply_change(&mut self, change: &LookChange) -> ChangeOutcome {
        let dconf = self.env.guard(TargetKind::Dconf).is_ok();
        let Some(req) = to_change(change, dconf) else {
            return Outcome::Skipped(Skip::TestInstance.name()).into();
        };
        let req = with_legacy_env_removal(req, change, &is_managed_file);
        // Ayar başına tek işlem: bölünmez (bölünse "önceki hale dön" yarım kalırdı). Tek işlem
        // sınırı (parça sayısı + adres bütçesi) aşılırsa genel `invalid` yerine açık ad.
        if !fits_one_transaction(&req) {
            return Outcome::Failed(TOO_MANY_FRAGMENTS.into()).into();
        }
        // Görünüm yazımları framework.toml/temadan türetilir: araya kimse girmemişse son işleme süre
        // penceresiz birleşir (her duvar kâğıdı/kip değişimi yeni snapshot doğurmaz, "önceki hale
        // dön" Nuxut öncesine döner); ilk dokunuşta ya da dış değişiklikte yeni snapshot alınır.
        //
        // Başka aracın yönettiği dosya (`ExternallyManaged`; docs/design/look-external.md §4):
        // devralınabiliyorsa (symlink — hedefi root'un tema dosyası olsa da —, salt okunur, sabit
        // bağ) varsayılan olarak devralınır (kullanıcı ilkesi 2026-10-16: Nuxut yapabiliyorsa
        // varsayılan Nuxut): özgün biçim aynı snapshot'a girer, "önceki hale dön" / birleşik
        // görünümü kapatmak onu geri koyar. Devralınamayan (nix/guix deposu, başka sahibin düz
        // dosyası, yazılamayan dizin) bütün ayarı düşürmez: o dosyanın parçaları çıkarılıp kalan
        // tek işlemde yeniden denenir (dosya sayısıyla sınırlı) ve rapora `external` olarak girer.
        let mut req = req;
        let mut external = Vec::new();
        for _ in 0..=MAX_EXTERNAL_RETRIES {
            let outcome = match self.app.apply_derived(req.clone()) {
                ApplyOutcome::Applied { .. } => Outcome::Written,
                ApplyOutcome::Noop => Outcome::Unchanged,
                ApplyOutcome::Skipped { reason } => Outcome::Skipped(reason.name()),
                ApplyOutcome::Error { kind } => Outcome::Failed(format!("error({kind})")),
                ApplyOutcome::ExternallyManaged { target, reason } => {
                    let Some(file) = matching_file(&req, &target, &self.env) else {
                        return ChangeOutcome { outcome: Outcome::Failed(EXTERNALLY_MANAGED.into()), external };
                    };
                    if reason.can_take_over(&file) && !req.take_over.contains(&file) {
                        tracing::info!(setting = change.setting, target = %target, reason = reason.name(), "görünüm: dosya devralınıyor");
                        req = req.take_over(file);
                        continue;
                    }
                    external.push(ExternalTarget { setting: change.setting, path: target, reason: reason.name() });
                    req.writes.retain(|(f, _)| f.file().is_none_or(|p| !same_file(p, &file)));
                    req.create_missing.retain(|p| !same_file(p, &file));
                    req.take_over.retain(|p| !same_file(p, &file));
                    if req.writes.is_empty() {
                        return ChangeOutcome { outcome: Outcome::Skipped(EXTERNALLY_MANAGED), external };
                    }
                    continue;
                }
                other => Outcome::Failed(other.name().to_owned()),
            };
            return ChangeOutcome { outcome, external };
        }
        ChangeOutcome { outcome: Outcome::Failed(EXTERNALLY_MANAGED.into()), external }
    }

    fn live(&mut self, action: &LiveAction) -> Outcome {
        run_live(&self.env, self.live, action)
    }

    fn taken_over(&mut self) -> Vec<TakenOver> {
        let Ok(list) = self.app.list(&Filter { setting: None, limit: 200 }) else { return Vec::new() };
        let mut out: Vec<TakenOver> = Vec::new();
        let applied = list.iter().filter(|s| s.state == "applied").filter_map(|s| setting::ALL.into_iter().find(|n| *n == s.setting.as_str()).map(|n| (n, s)));
        for (setting, s) in applied.take(MAX_TAKEOVER_SNAPSHOTS) {
            let Ok(Some(detail)) = self.app.show(&s.id) else { continue };
            for f in detail.fragments.iter().filter(|f| f.kind == "file_form") {
                if out.iter().any(|t| t.path == f.target) || still_linked(&self.env, &f.target) {
                    continue;
                }
                out.push(TakenOver { setting, path: f.target.clone() });
            }
        }
        out
    }

    fn name(&self) -> &'static str {
        WRITER_NAME
    }
}

/// Kapıdan geçirip gönderir.
pub fn run_live(env: &Env, sink: &mut dyn LiveSink, action: &LiveAction) -> Outcome {
    if let Err(skip) = live_gate(env, action) {
        return Outcome::Skipped(skip.name());
    }
    match sink.run(action) {
        Ok(()) => Outcome::Written,
        Err(e) => Outcome::Failed(e),
    }
}

// ---------------------------------------------------------------------------------------------
// Kapatma = geri yükleme (§8-3)

fn setting_id(s: &str) -> Option<SettingId> {
    SettingId::new(s).ok()
}

/// Nuxut'un görünümü diskte etkin mi: bir `look.*` ayarının en yeni işlemi uygulanmış ve bir geri
/// yükleme değil. Depo yoksa / kapı kapalıysa `false`.
pub fn is_active(app: &Applier) -> bool {
    setting::ALL.iter().filter_map(|s| setting_id(s)).any(|id| {
        app.list(&Filter { setting: Some(id), limit: 1 })
            .ok()
            .and_then(|rows| rows.into_iter().next())
            .is_some_and(|row| row.state == "applied" && row.restores.is_none())
    })
}

/// Birleşik görünüm kapalı ama Nuxut'un yazdıkları diskte: her `look.*` ayarının sabitlenmiş
/// (Nuxut öncesi değerleri taşıyan) işlemleri yeniden eskiye **zorla** geri yüklenir — en eskisi
/// en son yazılır, her parça Nuxut öncesi hâline döner (sonradan elle değiştirilmiş yönetilen
/// anahtarlar dahil; ADR-0004 §8-3 "kapatma = ilk snapshot'a dönüş"). Ardından canlı eylemler
/// geri yüklenen değerlerle: KDE sinyalleri, ortam değişkenlerinin silinmesi, geri yüklenen GTK
/// imleci varsa `setcursor` ([`RestoredCursor`]). Etkin değilse `None` (hiçbir şey yazılmaz).
pub fn deactivate(env: &Env, sink: &mut dyn LiveSink, dirs: &Dirs) -> Option<ApplyReport> {
    let app = Applier::open(env.clone()).with_source(Source::Shell);
    if !is_active(&app) {
        return None;
    }
    let mut r = ApplyReport::default();
    let settings_ini = dirs.config.join("gtk-3.0/settings.ini");
    let mut cursor = RestoredCursor::default();
    // Sıra serbest: birden çok ayarın paylaştığı ve Nuxut'un yarattığı dosya (gtk-4.0/settings.ini)
    // yaratıcı bilgisini nuxut-apply'da dosya başına taşır (`file_inherited`); hangi ayar son geri
    // yüklenirse yüklensin, boş kalan dosya silinir.
    for &s in setting::ALL.iter() {
        let Some(id) = setting_id(s) else { continue };
        let rows = match app.list(&Filter { setting: Some(id), limit: nuxut_apply::PER_SETTING_LIMIT * 4 }) {
            Ok(rows) => rows,
            Err(e) => {
                r.errors.push(format!("{s}: {}", crate::services::apply::error_text(&e)));
                continue;
            }
        };
        let mut wrote = false;
        let mut failed = false;
        // Liste yeniden eskiye: en eski sabitlenmiş işlem en son geri yüklenir.
        for row in rows.iter().filter(|row| row.pinned) {
            let (outcome, values) = app.restore_with_values(&row.id, RestoreMode::Force);
            cursor.observe(&values, &settings_ini);
            match outcome {
                RestoreOutcome::Restored { id: Some(_) } | RestoreOutcome::Partial { .. } => wrote = true,
                RestoreOutcome::Restored { id: None } | RestoreOutcome::NotFound => {}
                RestoreOutcome::Skipped { .. } => {}
                other => {
                    failed = true;
                    r.errors.push(format!("{s}: {}", other.name()));
                }
            }
        }
        if wrote {
            r.written.push(s);
        } else if !failed {
            r.unchanged.push(s);
        }
    }
    if !r.written.is_empty() {
        let mut live = vec![
            LiveAction::KdeNotifyChange(0),
            LiveAction::KdeNotifyChange(1),
            LiveAction::KdeNotifyChange(2),
            LiveAction::KdeIconChanged,
            LiveAction::SessionUnset(WATCHED.iter().copied().filter(|k| *k != "GTK_THEME").collect()),
        ];
        if let Some(a) = cursor.action() {
            live.push(a);
        }
        for a in &live {
            match run_live(env, sink, a) {
                Outcome::Written | Outcome::Unchanged => r.live.push(a.name()),
                Outcome::Skipped(_) => {}
                Outcome::Failed(e) => r.errors.push(format!("{}: {}", a.name(), short(&e))),
            }
        }
    }
    r.ok = r.errors.is_empty();
    Some(r)
}

/// Kapatmada geri yüklenen imleç (ADR-0004 §14-5): geri yüklemelerin döndürdüğü değerlerden GTK 3
/// `settings.ini` `gtk-cursor-theme-name`; diske ayrıca bakılmaz. Son geri yükleme kazanır (en eski
/// işlem = Nuxut öncesi en son yazılır); orada anahtar kaldırıldıysa bilinen imleç yoktur (Hyprland
/// imleci bir sonraki oturuma kadar Nuxut'unki kalır).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RestoredCursor(Option<(String, u32)>);

impl RestoredCursor {
    pub fn observe(&mut self, values: &RestoredValues, settings_ini: &Path) {
        let key = |k: &str| nuxut_apply::Fragment::IniKey { file: settings_ini.to_path_buf(), section: "Settings".into(), key: k.into() };
        match values.get(&key("gtk-cursor-theme-name")) {
            Some(Some(theme)) => {
                let theme = theme.trim().to_owned();
                let size = match values.get(&key("gtk-cursor-theme-size")) {
                    Some(Some(v)) => v.trim().parse::<u32>().unwrap_or(nuxut::theme::CURSOR_SIZE_DEFAULT),
                    _ => nuxut::theme::CURSOR_SIZE_DEFAULT,
                };
                let size = size.clamp(nuxut::theme::CURSOR_SIZE_MIN, nuxut::theme::CURSOR_SIZE_MAX);
                self.0 = valid_theme_name(&theme).then_some((theme, size));
            }
            Some(None) => self.0 = None,
            None => {}
        }
    }

    pub fn action(&self) -> Option<LiveAction> {
        self.0.as_ref().map(|(theme, size)| LiveAction::HyprSetCursor { theme: theme.clone(), size: *size })
    }
}


fn short(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// `msg snapshot restore` bir `look.*` ayarını geri yükledi: KDE sinyalleri (ayara bağlı) ve
/// geri yazılan değerlerde GTK3 `settings.ini` imleci varsa `setcursor` ([`RestoredCursor`];
/// `settings_ini` yoksa imleç atlanır). Ortam değişkenleri canlı geri alınmaz (sonraki oturum).
pub fn after_restore(
    env: &Env,
    sink: &mut dyn LiveSink,
    restored: &str,
    values: &RestoredValues,
    settings_ini: Option<&Path>,
) -> Vec<&'static str> {
    let mut actions: Vec<LiveAction> =
        [LiveAction::KdeNotifyChange(0), LiveAction::KdeNotifyChange(1), LiveAction::KdeNotifyChange(2), LiveAction::KdeIconChanged]
            .into_iter()
            .filter(|a| a.settings().contains(&restored))
            .collect();
    if let Some(ini) = settings_ini {
        let mut cursor = RestoredCursor::default();
        cursor.observe(values, ini);
        actions.extend(cursor.action());
    }
    actions.into_iter().filter(|a| matches!(run_live(env, sink, a), Outcome::Written)).map(|a| a.name()).collect()
}

// ---------------------------------------------------------------------------------------------
// Gerçek gönderici

/// zbus oturum veri yolu (KDE sinyalleri, systemd ve D-Bus etkinleştirme ortamı) ve Hyprland
/// soketi (`setcursor`). Bağlantı ilk kullanımda açılır.
pub struct SystemLive<H> {
    hypr: Option<H>,
    bus: Option<zbus::blocking::Connection>,
    /// dconf yazımı (`gtk_reload`): `nuxut-apply`'ın arka ucu, aynı kapı ve zaman aşımı.
    dconf: nuxut_apply::SystemBackend,
}

impl<H> std::fmt::Debug for SystemLive<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemLive").field("hypr", &self.hypr.is_some()).finish_non_exhaustive()
    }
}

impl<H: HyprControl> SystemLive<H> {
    /// `hypr`: yalnızca `Env::may_control_hyprland` doğruyken verilir.
    pub fn new(hypr: Option<H>, env: &Env) -> Self {
        Self { hypr, bus: None, dconf: nuxut_apply::SystemBackend::new(env) }
    }

    fn bus(&mut self) -> Result<&zbus::blocking::Connection, String> {
        if self.bus.is_none() {
            let conn = zbus::blocking::connection::Builder::session()
                .and_then(|b| b.method_timeout(DBUS_TIMEOUT).build())
                .map_err(|e| format!("oturum veri yolu: {}", short(&e.to_string())))?;
            self.bus = Some(conn);
        }
        self.bus.as_ref().ok_or_else(|| "oturum veri yolu yok".into())
    }
}

/// Ortam değeri tek satır ve sınırlı mı (systemd `K=V` dizisine güvenle girer)?
fn env_value_ok(v: &str) -> bool {
    v.len() <= 256 && !v.chars().any(char::is_control)
}

/// `setcursor` komutu (geçersiz ad/boy → `None`).
pub fn setcursor_command(theme: &str, size: u32) -> Option<String> {
    let size_ok = (nuxut::theme::CURSOR_SIZE_MIN..=nuxut::theme::CURSOR_SIZE_MAX).contains(&size);
    (valid_theme_name(theme) && size_ok).then(|| format!("setcursor {theme} {size}"))
}

impl<H: HyprControl> LiveSink for SystemLive<H> {
    fn run(&mut self, action: &LiveAction) -> Result<(), String> {
        let err = |what: &str, e: zbus::Error| format!("{what}: {}", short(&e.to_string()));
        match action {
            LiveAction::KdeNotifyChange(kind) => self
                .bus()?
                .emit_signal(None::<&str>, "/KGlobalSettings", "org.kde.KGlobalSettings", "notifyChange", &(*kind, 0i32))
                .map_err(|e| err("notifyChange", e)),
            LiveAction::KdeIconChanged => self
                .bus()?
                .emit_signal(None::<&str>, "/KIconLoader", "org.kde.KIconLoader", "iconChanged", &(0i32,))
                .map_err(|e| err("iconChanged", e)),
            LiveAction::GtkReload => {
                use nuxut_apply::Backend as _;
                let key = nuxut_apply::Fragment::Dconf { key: format!("{}gtk-theme", super::gtk::DCONF_DIR) };
                let set = |name: &str| FragValue::Text(super::gtk::gvariant_str(name));
                // İkize, sonra asıla: ikisi de değişim sinyali, son değer nuxut-apply'ınkiyle aynı.
                let twin = self.dconf.write(&key, &set(super::gtk_theme::RELOAD_THEME));
                let back = self.dconf.write(&key, &set(super::gtk_theme::THEME));
                back.and(twin).map_err(|e| format!("gtk_reload: {}", short(&crate::services::apply::error_text(&e))))
            }
            LiveAction::IconReload { twin, name } => {
                use nuxut_apply::Backend as _;
                let key = nuxut_apply::Fragment::Dconf { key: format!("{}icon-theme", super::gtk::DCONF_DIR) };
                let set = |name: &str| FragValue::Text(super::gtk::gvariant_str(name));
                // İkize, sonra asıla (GtkReload ile aynı): son değer nuxut-apply'ınkiyle aynı.
                let first = self.dconf.write(&key, &set(twin));
                let back = self.dconf.write(&key, &set(name));
                back.and(first).map_err(|e| format!("icon_reload: {}", short(&crate::services::apply::error_text(&e))))
            }
            LiveAction::KittyReload => {
                let ticks = rustix::param::clock_ticks_per_second();
                let uptime = std::fs::read_to_string("/proc/uptime").ok().and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok());
                let Some(uptime) = uptime else { return Err("kitty_reload: /proc/uptime".into()) };
                let now = (uptime * ticks as f64) as u64;
                let uid = rustix::process::geteuid().as_raw();
                for pid in kitty_pids(Path::new("/proc"), uid, now, KITTY_MIN_AGE_SECS * ticks) {
                    if let Some(pid) = rustix::process::Pid::from_raw(pid) {
                        // Süreç arada çıkmış olabilir; hata önemsiz.
                        let _ = rustix::process::kill_process(pid, rustix::process::Signal::USR1);
                    }
                }
                Ok(())
            }
            LiveAction::HyprSetCursor { theme, size } => {
                let ctl = self.hypr.as_ref().ok_or("no_hyprland_connection")?;
                let cmd = setcursor_command(theme, *size).ok_or("invalid_cursor")?;
                let reply = ctl.request(&cmd)?;
                if reply.trim() == "ok" { Ok(()) } else { Err(format!("setcursor: {}", short(reply.trim()))) }
            }
            LiveAction::SessionEnvironment(vars) => {
                if vars.iter().any(|(_, v)| !env_value_ok(v)) {
                    return Err("invalid_env_value".into());
                }
                let assign: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
                let map: HashMap<&str, &str> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let bus = self.bus()?;
                // İkisi de denenir; ilk hata raporlanır.
                let systemd = bus
                    .call_method(
                        Some("org.freedesktop.systemd1"),
                        "/org/freedesktop/systemd1",
                        Some("org.freedesktop.systemd1.Manager"),
                        "SetEnvironment",
                        &(assign,),
                    )
                    .map(|_| ())
                    .map_err(|e| err("systemd SetEnvironment", e));
                let dbus = bus
                    .call_method(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        Some("org.freedesktop.DBus"),
                        "UpdateActivationEnvironment",
                        &(map,),
                    )
                    .map(|_| ())
                    .map_err(|e| err("UpdateActivationEnvironment", e));
                systemd.and(dbus)
            }
            LiveAction::SessionUnset(names) => self
                .bus()?
                .call_method(
                    Some("org.freedesktop.systemd1"),
                    "/org/freedesktop/systemd1",
                    Some("org.freedesktop.systemd1.Manager"),
                    "UnsetEnvironment",
                    &(names.clone(),),
                )
                .map(|_| ())
                .map_err(|e| err("systemd UnsetEnvironment", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Recording(pub Vec<&'static str>);

    impl LiveSink for Recording {
        fn run(&mut self, a: &LiveAction) -> Result<(), String> {
            self.0.push(a.name());
            Ok(())
        }
    }

    fn user_session() -> Env {
        // Kullanıcı oturumu gibi görünen ortam: YALNIZCA kapı kararını sınar, hiçbir Applier
        // kurulmaz (dconf/systemd'ye ulaşacak yol yok).
        Env { test_instance: false, hyprland: Some("abc_123".into()), ..Env::default() }
    }

    #[test]
    fn live_gate_blocks_test_instance_and_disabled() {
        let t = tempfile::tempdir().unwrap();
        let all = [
            LiveAction::KdeNotifyChange(0),
            LiveAction::KdeIconChanged,
            LiveAction::HyprSetCursor { theme: "Adwaita".into(), size: 24 },
            LiveAction::SessionEnvironment(vec![("XCURSOR_SIZE", "24".into())]),
            LiveAction::SessionUnset(vec!["XCURSOR_SIZE"]),
        ];
        let test = Env::for_test(t.path());
        let off = Env { disabled: true, ..user_session() };
        for a in &all {
            assert_eq!(live_gate(&test, a), Err(Skip::TestInstance), "{a:?}");
            assert_eq!(live_gate(&off, a), Err(Skip::Disabled), "{a:?}");
            assert_eq!(live_gate(&user_session(), a), Ok(()), "{a:?}");
            let mut rec = Recording::default();
            assert!(matches!(run_live(&test, &mut rec, a), Outcome::Skipped("test_instance")));
            assert!(rec.0.is_empty(), "test örneği canlı eylem göndermez");
        }
        // Hyprland oturumu yoksa yalnızca setcursor atlanır.
        let no_hypr = Env { hyprland: None, ..user_session() };
        assert_eq!(live_gate(&no_hypr, &all[2]), Err(Skip::NoSession));
        assert_eq!(live_gate(&no_hypr, &all[0]), Ok(()));
    }

    #[test]
    fn after_restore_sends_restored_cursor() {
        use nuxut_apply::RestoredValue;
        let ini = Path::new("/t/gtk-3.0/settings.ini");
        let frag = |k: &str| nuxut_apply::Fragment::IniKey { file: ini.to_path_buf(), section: "Settings".into(), key: k.into() };
        let values = RestoredValues {
            setting: None,
            values: vec![
                RestoredValue { fragment: frag("gtk-cursor-theme-name"), value: Some("Adwaita".into()) },
                RestoredValue { fragment: frag("gtk-cursor-theme-size"), value: Some("32".into()) },
            ],
        };
        // Kayıtçı gönderici: hiçbir şey gerçekten gönderilmez.
        let mut rec = Recording::default();
        let sent = after_restore(&user_session(), &mut rec, setting::CURSOR, &values, Some(ini));
        assert_eq!(sent, ["hypr_setcursor"]);
        assert_eq!(rec.0, ["hypr_setcursor"]);
        // Değer yoksa ya da yol bilinmiyorsa imleç gönderilmez; KDE sinyalleri ayara bağlı sürer.
        let mut rec = Recording::default();
        assert!(after_restore(&user_session(), &mut rec, setting::CURSOR, &RestoredValues::default(), Some(ini)).is_empty());
        assert!(after_restore(&user_session(), &mut rec, setting::CURSOR, &values, None).is_empty());
        let icons = after_restore(&user_session(), &mut rec, setting::ICONS, &RestoredValues::default(), Some(ini));
        assert!(icons.contains(&"kde_icons") && !icons.contains(&"hypr_setcursor"), "{icons:?}");
        // Test örneği: hiçbiri.
        let t = tempfile::tempdir().unwrap();
        let mut rec = Recording::default();
        assert!(after_restore(&Env::for_test(t.path()), &mut rec, setting::CURSOR, &values, Some(ini)).is_empty());
        assert!(rec.0.is_empty());
    }

    #[test]
    fn legacy_env_file_is_removed_only_when_ours() {
        let c = LookChange {
            setting: setting::UNIFIED,
            label: "x".into(),
            writes: vec![(Fragment::ManagedFile { file: "/t/environment.d/99-nuxut-look.conf".into() }, "# Nuxut — x\n".into())],
        };
        let base = to_change(&c, true).unwrap();
        let kept = with_legacy_env_removal(base.clone(), &c, &|_| false);
        assert_eq!(kept.writes.len(), 1, "bizim değilse dokunulmaz");
        let migrated = with_legacy_env_removal(base, &c, &|p| p == Path::new("/t/environment.d/90-nuxut-look.conf"));
        assert_eq!(migrated.writes.len(), 2);
        let (f, v) = &migrated.writes[1];
        assert_eq!(f.kind_name(), "managed_file");
        assert!(v.is_absent());
        assert!(migrated.validate().is_ok());
    }

    #[test]
    fn setcursor_command_is_validated() {
        assert_eq!(setcursor_command("Bibata-Modern-Classic", 24).as_deref(), Some("setcursor Bibata-Modern-Classic 24"));
        assert_eq!(setcursor_command("a;b", 24), None);
        assert_eq!(setcursor_command("x\ny", 24), None);
        assert_eq!(setcursor_command("Adwaita", 4000), None);
        assert!(!env_value_ok("a\nb") && env_value_ok("") && env_value_ok("kde"));
    }

    #[test]
    fn change_conversion_drops_dconf_only_when_asked() {
        let c = LookChange {
            setting: setting::MODE,
            label: "x".into(),
            writes: vec![
                (Fragment::Dconf { key: "/org/gnome/desktop/interface/color-scheme".into() }, "'prefer-dark'".into()),
                (Fragment::ManagedFile { file: "/t/gtk-4.0/nuxut.css".into() }, "/* Nuxut — x */\n".into()),
                (Fragment::Line { file: "/t/gtk-4.0/gtk.css".into(), marker: "@import url(\"nuxut.css\");".into() }, "@import url(\"nuxut.css\");".into()),
            ],
        };
        let full = to_change(&c, true).unwrap();
        assert_eq!(full.writes.len(), 3);
        assert!(full.validate().is_ok());
        assert_eq!(full.source, Source::Shell);
        // gtk.css her zaman satır parçası (yoksa yalnızca bu satırla yaratılır: write_creating).
        assert_eq!(full.writes[2].0.kind_name(), "line");
        let test = to_change(&c, false).unwrap();
        assert_eq!(test.writes.len(), 2);
        assert!(test.writes.iter().all(|(f, _)| f.kind_name() != "dconf"));
        let only_dconf = LookChange { writes: c.writes[..1].to_vec(), ..c };
        assert!(to_change(&only_dconf, false).is_none());
    }

    #[test]
    fn restored_cursor_last_restore_wins() {
        use nuxut_apply::RestoredValue;
        let ini = Path::new("/t/gtk-3.0/settings.ini");
        let frag = |k: &str| nuxut_apply::Fragment::IniKey { file: ini.to_path_buf(), section: "Settings".into(), key: k.into() };
        let vals = |pairs: &[(&str, Option<&str>)]| RestoredValues {
            setting: None,
            values: pairs.iter().map(|(k, v)| RestoredValue { fragment: frag(k), value: v.map(str::to_owned) }).collect(),
        };
        let mut c = RestoredCursor::default();
        assert_eq!(c.action(), None);
        // Bu geri yüklemede imleç yazılmadı: değişmez.
        c.observe(&vals(&[("gtk-font-name", Some("Inter 10"))]), ini);
        assert_eq!(c.action(), None);
        c.observe(&vals(&[("gtk-cursor-theme-name", Some("Vimix-cursors")), ("gtk-cursor-theme-size", Some("32"))]), ini);
        assert_eq!(c.action(), Some(LiveAction::HyprSetCursor { theme: "Vimix-cursors".into(), size: 32 }));
        // Son (daha eski) geri yükleme kazanır; boy yoksa varsayılan.
        c.observe(&vals(&[("gtk-cursor-theme-name", Some(" Adwaita "))]), ini);
        assert_eq!(c.action(), Some(LiveAction::HyprSetCursor { theme: "Adwaita".into(), size: nuxut::theme::CURSOR_SIZE_DEFAULT }));
        // Boş değer listesi (hiçbir şey yazılmadı): değişmez.
        c.observe(&vals(&[]), ini);
        assert!(c.action().is_some());
        // Nuxut öncesinde anahtar yoktu (kaldırıldı): bilinen imleç yok.
        c.observe(&vals(&[("gtk-cursor-theme-name", None)]), ini);
        assert_eq!(c.action(), None);
        // Geçersiz ad setcursor'a gitmez.
        c.observe(&vals(&[("gtk-cursor-theme-name", Some("a b;c"))]), ini);
        assert_eq!(c.action(), None);
    }

    /// kitty yeniden yükleme hedefleri: yalnızca bu kullanıcının, açılışı tamamlanmış `kitty`
    /// süreçleri (sahte `/proc`).
    #[test]
    fn kitty_pids_filters_name_owner_and_age() {
        let t = tempfile::tempdir().unwrap();
        let uid = rustix::process::geteuid().as_raw();
        let proc = |pid: &str, comm: &str, start: u64| {
            let d = t.path().join(pid);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("comm"), format!("{comm}\n")).unwrap();
            // 22. alan başlangıç; comm boşluk ve parantez içerebilir.
            let fields: Vec<String> = (3..=21).map(|i| i.to_string()).collect();
            std::fs::write(d.join("stat"), format!("{pid} ({comm} x) {} {start} 0 0\n", fields.join(" "))).unwrap();
        };
        proc("100", "kitty", 100);
        proc("101", "kitty", 950); // yeni açılmış
        proc("102", "fish", 100);
        proc("self", "kitty", 100); // sayısal olmayan ad
        std::fs::create_dir_all(t.path().join("103")).unwrap(); // okunamayan girdiler
        let mut pids = kitty_pids(t.path(), uid, 1000, 200);
        pids.sort();
        assert_eq!(pids, [100]);
        assert!(kitty_pids(t.path(), uid.wrapping_add(1), 1000, 200).is_empty(), "başka kullanıcı");
        assert!(kitty_pids(&t.path().join("yok"), uid, 1000, 200).is_empty());
    }
}
