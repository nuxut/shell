//! Planın uygulanması: dış yazımların ve canlı eylemlerin **tek kapısı** (ADR-0004 §4-3, §12-1).
//!
//! Yazımlar yalnızca `nuxut-apply` (ADR-0001; atomik, parça düzeyinde snapshot, geri
//! yükleme, test örneği kapısı `Env::guard`) üzerinden yapılır: gerçek arka uç
//! [`super::writer::ApplyBackend`]. [`Disabled`] hiçbir şey yazmaz (görünüm thread'i yokken /
//! ikincil örnekte `msg look status` `writer` adı).

use serde::Serialize;

use super::plan::{LiveAction, LookChange, LookPlan};

/// Bir ayarın (ya da canlı eylemin) sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// En az bir parça değişti.
    Written,
    /// Her şey zaten istenen değerdeydi (`nuxut-apply` `NoOp`); canlı eylem çalışmaz.
    Unchanged,
    /// Bilerek yapılmadı (yazıcı yok, test örneği, araç kurulu değil).
    Skipped(&'static str),
    /// Hata (kullanıcıya gösterilir; yol/değer içermemeli).
    Failed(String),
}

/// Başka bir aracın yönettiği hedef (`nuxut-apply` `ExternallyManaged`; docs/design/look-external.md):
/// Nuxut o dosyaya dokunmaz, ayarın geri kalanı yazılır.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalTarget {
    pub setting: &'static str,
    /// `~/…` biçiminde yol (kullanıcının kendi yapılandırma dosyası).
    pub path: String,
    /// Neden: `symlink_broken` | `store` | `not_owned` | `read_only` | `hard_link` |
    /// `dir_not_writable` | `unknown` (`nuxut_apply::ExternalReason::name`).
    pub reason: &'static str,
}

/// Nuxut'un devraldığı (önceden başka aracın yönettiği) dosya: hâlâ yürürlükte olan bir görünüm
/// snapshot'ında biçim kaydı var ve yol bugün bağ değil. Birleşik görünümü kapatmak ya da
/// "önceki hale dön" özgün biçimi (bağ, izin) geri koyar. docs/design/look-external.md §4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TakenOver {
    pub setting: &'static str,
    /// `~/…` biçiminde yol.
    pub path: String,
}

/// Bir ayarın uygulanma sonucu ve dokunulmayan (dışarıdan yönetilen) hedefleri.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeOutcome {
    pub outcome: Outcome,
    pub external: Vec<ExternalTarget>,
}

impl From<Outcome> for ChangeOutcome {
    fn from(outcome: Outcome) -> Self {
        Self { outcome, external: Vec::new() }
    }
}

/// Dış yazım + canlı eylem arka ucu.
pub trait LookBackend {
    /// Bir ayarın tüm parçaları tek işlemde (bir snapshot).
    fn apply_change(&mut self, change: &LookChange) -> ChangeOutcome;
    /// Dosya dışı canlı eylem (KDE sinyali, `setcursor`, oturum ortamı).
    fn live(&mut self, action: &LiveAction) -> Outcome;
    /// Yürürlükteki devralmalar ([`TakenOver`]); rapor sonunda bir kez sorulur.
    fn taken_over(&mut self) -> Vec<TakenOver> {
        Vec::new()
    }
    /// `msg look status` için ad.
    fn name(&self) -> &'static str;
}

/// Hiçbir şey yazmayan arka uç (kapalı yazım).
#[derive(Debug, Clone, Copy, Default)]
pub struct Disabled;

pub const SKIP_NO_APPLY: &str = "dış yazım kapalı"; // i18n: tanı Outcome::Skipped iç durum ayrıntısı (msg look status)

impl LookBackend for Disabled {
    fn apply_change(&mut self, _: &LookChange) -> ChangeOutcome {
        Outcome::Skipped(SKIP_NO_APPLY).into()
    }

    fn live(&mut self, _: &LiveAction) -> Outcome {
        Outcome::Skipped(SKIP_NO_APPLY)
    }

    fn name(&self) -> &'static str {
        "disabled"
    }
}

/// Uygulama raporu (`msg look status` `last_apply`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ApplyReport {
    /// Hata yok ve hiçbir hedef dışarıda kalmadı.
    pub ok: bool,
    pub written: Vec<&'static str>,
    pub unchanged: Vec<&'static str>,
    pub skipped: Vec<&'static str>,
    /// Kısa hata metinleri (`<ayar>: <neden>`).
    pub errors: Vec<String>,
    /// Çalışan canlı eylemler.
    pub live: Vec<&'static str>,
    /// Başka aracın yönettiği, bu yüzden yazılmayan hedefler (ayarın geri kalanı yazıldı).
    pub external: Vec<ExternalTarget>,
    /// Nuxut'un devraldığı dosyalar (hâlâ yürürlükte; [`TakenOver`]).
    pub taken_over: Vec<TakenOver>,
    /// Yeniden yazım döngüsü bekçisinin durdurduğu ayarlar ([`super::guard`]).
    pub paused: Vec<&'static str>,
    /// Daha yeni bir istek geldiği için tur yarıda bırakıldı ([`apply_with`]): kalan ayarlar ve
    /// canlı eylemler çalışmadı. Görünüm thread'inin iç durumu; durum yanıtına girmez.
    #[serde(skip)]
    pub cancelled: bool,
}

/// Hata metni üst sınırı.
const MAX_ERROR: usize = 200;
/// Rapordaki dış hedef üst sınırı.
pub const MAX_EXTERNAL: usize = 32;

fn short(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(MAX_ERROR).collect()
}

/// Planı sırayla uygular; canlı eylemler yalnızca bağlı oldukları ayar gerçekten yazıldıysa
/// (§7 "aynı değer yeniden yazılmaz, sinyal yayınlanmaz").
pub fn apply(plan: &LookPlan, backend: &mut dyn LookBackend) -> ApplyReport {
    apply_with(plan, backend, &mut || false, &[])
}

/// "Son seçim kazanır" (kullanıcı 2026-10-16: duvar kâğıdı → nötr hızlı seçiminde KDE önce eski
/// renge, çok sonra yenisine geçiyordu): `cancel` her ayardan önce sorulur; doğruysa kalan ayarlar
/// ve **bütün** canlı eylemler atlanır (`cancelled`). Yazılmış ayarlar `written`'da kalır; çağıran
/// onları bir sonraki tamamlanan tura `carried` olarak verir, canlı eylemler (KDE sinyalleri)
/// böylece tek sefer ve en son durumla gönderilir.
pub fn apply_with(
    plan: &LookPlan,
    backend: &mut dyn LookBackend,
    cancel: &mut dyn FnMut() -> bool,
    carried: &[&'static str],
) -> ApplyReport {
    let mut r = ApplyReport::default();
    for change in &plan.changes {
        if cancel() {
            r.cancelled = true;
            return r;
        }
        let done = backend.apply_change(change);
        for e in done.external {
            if r.external.len() < MAX_EXTERNAL {
                r.external.push(e);
            }
        }
        match done.outcome {
            Outcome::Written => r.written.push(change.setting),
            Outcome::Unchanged => r.unchanged.push(change.setting),
            Outcome::Skipped(_) => r.skipped.push(change.setting),
            Outcome::Failed(e) => r.errors.push(format!("{}: {}", change.setting, short(&e))),
        }
    }
    if cancel() {
        r.cancelled = true;
        return r;
    }
    for action in &plan.live {
        if !action.settings().iter().any(|s| r.written.contains(s) || carried.contains(s)) {
            continue;
        }
        match backend.live(action) {
            Outcome::Written | Outcome::Unchanged => r.live.push(action.name()),
            Outcome::Skipped(_) => {}
            Outcome::Failed(e) => r.errors.push(format!("{}: {}", action.name(), short(&e))),
        }
    }
    r.taken_over = backend.taken_over();
    r.taken_over.truncate(MAX_EXTERNAL);
    r.ok = r.errors.is_empty() && r.external.is_empty();
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::look::plan::{Fragment, setting};

    /// Kayıt tutan sahte arka uç: verilen ayarlar "yazılır", diğerleri değişmez.
    #[derive(Debug, Default)]
    struct Recording {
        write: Vec<&'static str>,
        fail: Vec<&'static str>,
        applied: Vec<&'static str>,
        live: Vec<&'static str>,
    }

    impl LookBackend for Recording {
        fn apply_change(&mut self, c: &LookChange) -> ChangeOutcome {
            self.applied.push(c.setting);
            let o = if self.fail.contains(&c.setting) {
                Outcome::Failed("ExternallyManaged\n".into())
            } else if self.write.contains(&c.setting) {
                Outcome::Written
            } else {
                Outcome::Unchanged
            };
            o.into()
        }
        fn live(&mut self, a: &LiveAction) -> Outcome {
            self.live.push(a.name());
            Outcome::Written
        }
        fn name(&self) -> &'static str {
            "recording"
        }
    }

    fn plan() -> LookPlan {
        let change = |s: &'static str| LookChange {
            setting: s,
            label: "x".into(),
            writes: vec![(Fragment::Dconf { key: format!("/k/{s}") }, "'v'".into())],
        };
        LookPlan {
            changes: vec![change(setting::MODE), change(setting::ICONS), change(setting::CURSOR), change(setting::FONTS)],
            live: vec![
                LiveAction::KdeNotifyChange(0),
                LiveAction::KdeNotifyChange(1),
                LiveAction::KdeIconChanged,
                LiveAction::HyprSetCursor { theme: "Adwaita".into(), size: 24 },
                LiveAction::SessionEnvironment(vec![]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn disabled_backend_writes_nothing_and_runs_no_live_action() {
        let r = apply(&plan(), &mut Disabled);
        assert!(r.ok);
        assert!(r.written.is_empty());
        assert_eq!(r.skipped.len(), 4);
        assert!(r.live.is_empty());
    }

    #[test]
    fn live_actions_follow_written_settings() {
        let mut b = Recording { write: vec![setting::ICONS, setting::CURSOR], ..Default::default() };
        let r = apply(&plan(), &mut b);
        assert_eq!(r.written, [setting::ICONS, setting::CURSOR]);
        assert_eq!(r.unchanged, [setting::MODE, setting::FONTS]);
        assert_eq!(b.live, ["kde_icons", "hypr_setcursor", "session_env"]);
        assert!(r.ok);
    }

    #[test]
    fn failures_are_reported_and_sanitized() {
        let mut b = Recording { fail: vec![setting::MODE], write: vec![setting::FONTS], ..Default::default() };
        let r = apply(&plan(), &mut b);
        assert!(!r.ok);
        assert_eq!(r.errors, ["look.mode: ExternallyManaged"]);
        // Diğer ayarlar yine denenir.
        assert_eq!(b.applied.len(), 4);
        assert_eq!(b.live, ["kde_fonts"]);
    }

    /// Yeni istek turun ortasında gelince kalan ayarlar ve canlı eylemler atlanır; sonraki
    /// tamamlanan tur yarıda kalanın yazdıklarının sinyallerini de tek sefer gönderir.
    #[test]
    fn cancelled_round_defers_live_actions_to_the_next_complete_round() {
        let mut b = Recording { write: vec![setting::MODE, setting::ICONS], ..Default::default() };
        let mut asked = 0;
        let r = apply_with(&plan(), &mut b, &mut || { asked += 1; asked > 2 }, &[]);
        assert!(r.cancelled);
        assert_eq!(b.applied, [setting::MODE, setting::ICONS], "üçüncü ayardan önce bırakıldı");
        assert_eq!(r.written, [setting::MODE, setting::ICONS]);
        assert!(b.live.is_empty() && r.live.is_empty(), "yarıda kalan tur sinyal göndermez");

        // Sonraki tur: hepsi zaten yazılı (NoOp) ama taşınan ayarların sinyalleri tek sefer gider.
        let mut b2 = Recording::default();
        let r2 = apply_with(&plan(), &mut b2, &mut || false, &r.written);
        assert!(!r2.cancelled && r2.written.is_empty());
        assert_eq!(b2.live, ["kde_palette", "kde_icons"]);

        // Tamamlanan son ayardan sonra gelen istek de sinyalleri bastırır.
        let mut b3 = Recording { write: vec![setting::MODE], ..Default::default() };
        let mut n = 0;
        let r3 = apply_with(&plan(), &mut b3, &mut || { n += 1; n > 4 }, &[]);
        assert!(r3.cancelled && b3.applied.len() == 4 && b3.live.is_empty());
    }
}
