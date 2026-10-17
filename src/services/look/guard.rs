//! Görünüm turlarının bekçisi (2026-10-16 hatası: look.icons/look.fonts saniyede bir yeniden
//! yazılıyordu; docs/design/look-external.md §1). Saf durum makinesi; `nuxut-look` thread'i kullanır.
//!
//! - **Aynı plan yeniden uygulanmaz:** tetik (kurulu tema izleyicisi, tema yeniden yüklemesi)
//!   geldiğinde planı önceki uygulanan planla aynıysa tur atlanır. Nuxut'un kendi yazımı ya da
//!   ilgisiz bir dosya olayı böylece hiçbir yazım/sinyal doğurmaz. Dışarıdan geri alınan değer
//!   girdi değişene (ya da `nuxut-shell look apply`) kadar yeniden dayatılmaz: başka bir araçla
//!   kavga edilmez.
//! - **Sınırlı yineleme:** bir ayar [`LOOP_WINDOW`] içinde [`LOOP_MAX_WRITES`] kezden fazla **aynı
//!   içerikle** yeniden yazılırsa (başka araç sürekli geri alıyor ya da karşılaştırma eşitsizliği)
//!   [`LOOP_PAUSE`] boyunca durdurulur; durdurma başlarken tek uyarı. Yeni içerik (kullanıcının hızlı
//!   seçimleri) hiç sayılmaz.
//! - **Uyarılar durum değişince bir kez:** aynı hata/dış hedef kümesi her turda yeniden loglanmaz.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::backend::ApplyReport;
use super::plan::{LookChange, LookPlan};

/// Yazım sayılan pencere.
pub const LOOP_WINDOW: Duration = Duration::from_secs(60);
/// Pencerede izin verilen en çok aynı içerikli yeniden yazım (yeni içerik, yani kullanıcının
/// seçimleri, hiç sayılmaz).
pub const LOOP_MAX_WRITES: usize = 10;
/// Durdurulan ayarın bekleme süresi.
pub const LOOP_PAUSE: Duration = Duration::from_secs(300);

#[derive(Debug, Default)]
pub struct ApplyGuard {
    /// Son uygulanan (filtresiz) plan ve birleşik görünüm durumu.
    last: Option<(bool, LookPlan)>,
    /// Ayarın son gerçekten yazılan içeriği.
    last_written: HashMap<&'static str, LookChange>,
    writes: HashMap<&'static str, VecDeque<Instant>>,
    paused: HashMap<&'static str, Instant>,
    /// Son loglanan sorun kümesi (hatalar + dış hedefler).
    problems: Option<(Vec<String>, Vec<String>)>,
}

impl ApplyGuard {
    /// Bu plan uygulanmalı mı? Önceki uygulanan planla aynıysa `false` (tur atlanır).
    pub fn should_apply(&self, unified: bool, plan: &LookPlan) -> bool {
        self.last.as_ref().is_none_or(|(u, p)| *u != unified || p != plan)
    }

    /// Tur yarıda bırakıldı: disk artık son uygulanan planla eşit değil, bir sonraki plan (önceki
    /// planla aynı olsa bile — A → B yarıda → A) yeniden uygulanmalı.
    pub fn forget(&mut self) {
        self.last = None;
    }

    /// Uygulanacak plan: durdurulmuş ayarlar çıkarılır (süresi dolanlar serbest kalır).
    pub fn filter(&mut self, plan: &LookPlan, now: Instant) -> (LookPlan, Vec<&'static str>) {
        self.paused.retain(|_, until| *until > now);
        let mut out = plan.clone();
        out.changes.retain(|c| !self.paused.contains_key(c.setting));
        let paused = plan.changes.iter().map(|c| c.setting).filter(|s| self.paused.contains_key(s)).collect();
        (out, paused)
    }

    /// Turun sonucu: planı hatırlar, gerçek yazımları sayar. Yeni durdurulan ayarlar döner
    /// (çağıran bir kez uyarır).
    pub fn record(&mut self, unified: bool, plan: &LookPlan, report: Option<&ApplyReport>, now: Instant) -> Vec<&'static str> {
        self.last = Some((unified, plan.clone()));
        let mut newly = Vec::new();
        for &s in report.map_or(&[][..], |r| &r.written[..]) {
            // Kullanıcının art arda seçimleri (her biri yeni içerik) döngü değildir — 2026-10-16:
            // duvar kâğıtları arasında hızlı gezinmek look.mode'u 5 dk durduruyor, KDE eski renkte
            // kalıyordu. Yalnızca aynı içeriğin yeniden yazılması sayılır (başka araç geri alıyor
            // ya da karşılaştırma eşitsizliği).
            let change = plan.changes.iter().find(|c| c.setting == s);
            let same = change.is_some() && self.last_written.get(s) == change;
            if let Some(c) = change {
                self.last_written.insert(s, c.clone());
            }
            if !same {
                continue;
            }
            let q = self.writes.entry(s).or_default();
            q.push_back(now);
            while q.front().is_some_and(|t| now.duration_since(*t) > LOOP_WINDOW) {
                q.pop_front();
            }
            if q.len() > LOOP_MAX_WRITES {
                q.clear();
                self.paused.insert(s, now + LOOP_PAUSE);
                newly.push(s);
            }
        }
        newly
    }

    /// Raporun sorunları (hatalar, dış hedefler) öncekinden farklı mı? Farklıysa yenisi hatırlanır;
    /// çağıran yalnızca `true` iken uyarı loglar.
    pub fn problems_changed(&mut self, report: &ApplyReport) -> bool {
        let now = (report.errors.clone(), report.external.iter().map(|e| format!("{} {}", e.setting, e.path)).collect());
        if self.problems.as_ref() == Some(&now) {
            return false;
        }
        self.problems = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::look::backend::ExternalTarget;
    use crate::services::look::plan::{Fragment, LookChange, setting};

    fn plan(value: &str) -> LookPlan {
        let change = |s: &'static str| LookChange {
            setting: s,
            label: "x".into(),
            writes: vec![(Fragment::Dconf { key: format!("/k/{s}") }, value.into())],
        };
        LookPlan { changes: vec![change(setting::ICONS), change(setting::FONTS)], ..Default::default() }
    }

    fn written(s: &[&'static str]) -> ApplyReport {
        ApplyReport { written: s.to_vec(), ok: true, ..Default::default() }
    }

    /// Aynı plan ikinci kez uygulanmaz; plan ya da birleşik görünüm değişince uygulanır.
    #[test]
    fn same_plan_is_not_reapplied() {
        let mut g = ApplyGuard::default();
        let t = Instant::now();
        assert!(g.should_apply(true, &plan("a")), "ilk tur her zaman");
        g.record(true, &plan("a"), Some(&written(&[setting::ICONS])), t);
        assert!(!g.should_apply(true, &plan("a")), "tetik geldi ama girdi aynı: tur yok");
        assert!(g.should_apply(true, &plan("b")));
        assert!(g.should_apply(false, &plan("a")));
    }

    /// İki ayarın ayrı değerli planı (ICONS sabit tutulup FONTS değiştirilerek yeni tur zorlanır).
    fn plan2(icons: &str, fonts: &str) -> LookPlan {
        let change = |s: &'static str, v: &str| LookChange {
            setting: s,
            label: "x".into(),
            writes: vec![(Fragment::Dconf { key: format!("/k/{s}") }, v.into())],
        };
        LookPlan { changes: vec![change(setting::ICONS, icons), change(setting::FONTS, fonts)], ..Default::default() }
    }

    /// Aynı içerik sürekli yeniden yazılıyor (başka araç geri alıyor): pencerede sınırı aşan ayar
    /// durdurulur (tek kez bildirilir), diğerleri sürer; süre dolunca serbest.
    #[test]
    fn rewriting_same_content_is_paused_once() {
        let mut g = ApplyGuard::default();
        let t0 = Instant::now();
        let mut paused_at = None;
        for i in 0..=LOOP_MAX_WRITES + 1 {
            let now = t0 + Duration::from_secs(i as u64);
            let p = plan2("same", &i.to_string());
            assert!(g.should_apply(true, &p));
            let (filtered, _) = g.filter(&p, now);
            assert_eq!(filtered.changes.len(), 2);
            let newly = g.record(true, &p, Some(&written(&[setting::ICONS, setting::FONTS])), now);
            if !newly.is_empty() {
                assert_eq!(newly, [setting::ICONS], "FONTS her turda yeni içerik: döngü değil");
                paused_at = Some(now);
            }
        }
        let at = paused_at.expect("sınır aşıldı");
        let (filtered, paused) = g.filter(&plan2("same", "x"), at + Duration::from_secs(1));
        assert_eq!(paused, [setting::ICONS]);
        assert_eq!(filtered.changes.iter().map(|c| c.setting).collect::<Vec<_>>(), [setting::FONTS]);
        // Durdurulmuşken yazım sayılmaz, tekrar bildirilmez.
        assert!(g.record(true, &plan2("same", "y"), Some(&written(&[setting::FONTS])), at + Duration::from_secs(2)).is_empty());
        let (filtered, paused) = g.filter(&plan2("same", "z"), at + LOOP_PAUSE + Duration::from_secs(1));
        assert!(paused.is_empty() && filtered.changes.len() == 2, "süre doldu");
    }

    /// Kullanıcının hızlı seçimleri (her tur yeni içerik, iki seçenek arasında gidip gelmek dahil)
    /// hiçbir zaman durdurulmaz.
    #[test]
    fn rapid_user_choices_are_never_paused() {
        let mut g = ApplyGuard::default();
        let t0 = Instant::now();
        for i in 0..(LOOP_MAX_WRITES * 5) {
            let now = t0 + Duration::from_millis(200 * i as u64);
            let p = plan(if i % 2 == 0 { "a" } else { "b" });
            assert!(g.should_apply(true, &p));
            assert!(g.filter(&p, now).1.is_empty());
            assert!(g.record(true, &p, Some(&written(&[setting::ICONS, setting::FONTS])), now).is_empty(), "tur {i}");
        }
    }

    /// Yarıda bırakılan turdan sonra önceki planla aynı plan da yeniden uygulanır (A → B yarıda → A).
    #[test]
    fn forget_reapplies_previous_plan() {
        let mut g = ApplyGuard::default();
        let t = Instant::now();
        g.record(true, &plan("a"), Some(&written(&[setting::ICONS])), t);
        assert!(!g.should_apply(true, &plan("a")));
        g.forget();
        assert!(g.should_apply(true, &plan("a")));
    }

    /// Seyrek yazımlar (pencere dışı) durdurmaz.
    #[test]
    fn slow_changes_are_not_paused() {
        let mut g = ApplyGuard::default();
        let t0 = Instant::now();
        for i in 0..(LOOP_MAX_WRITES * 3) {
            let now = t0 + LOOP_WINDOW / 4 * i as u32;
            assert!(g.record(true, &plan("same"), Some(&written(&[setting::ICONS])), now).is_empty());
        }
    }

    /// Aynı sorun kümesi yeniden loglanmaz; değişince bir kez.
    #[test]
    fn problems_logged_on_change_only() {
        let mut g = ApplyGuard::default();
        let ext = ExternalTarget { setting: setting::MODE, path: "~/.config/gtk-3.0/gtk.css".into(), reason: "read_only" };
        let r = ApplyReport { external: vec![ext], ..Default::default() };
        assert!(g.problems_changed(&r));
        assert!(!g.problems_changed(&r));
        assert!(!g.problems_changed(&r.clone()));
        assert!(g.problems_changed(&ApplyReport::default()), "düzeldi: bir kez");
        assert!(!g.problems_changed(&ApplyReport::default()));
    }
}
