//! Workspace göstergesinin saf mantığı: sayfa, yuva durumları, önceki/sonraki sayfa
//! göstergeleri, kaydırma hedefi ve dokunmatik yüzey birikimi. Tasarım:
//! `docs/design/workspaces.md`.

use std::time::{Duration, Instant};

use nuxut::Point;

use crate::services::hyprland::{HyprSnapshot, WorkspaceRef, is_special_workspace};
use crate::widgets::registry::WorkspaceScroll;

/// Bir sayfadaki tek yuva.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlotState {
    pub id: i64,
    /// İçinde pencere var.
    pub occupied: bool,
    /// Bu monitörün aktif workspace'i.
    pub active: bool,
    /// Başka bir monitörde görünür (o monitörün aktif workspace'i).
    pub visible_elsewhere: bool,
    /// Temsilci pencerenin sınıfı (boş = yok).
    pub app_class: String,
}

/// Bir monitörün bar'ında gösterilecek sayfa.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspacePage {
    /// 0'dan sayfa numarası.
    pub page: i64,
    /// Her zaman `count` yuva: `page·count+1 ..= page·count+count`.
    pub slots: Vec<SlotState>,
    /// Önceki sayfalarda kayda değer workspace varsa en büyüğü (`<` göstergesinin hedefi).
    pub prev: Option<i64>,
    /// Sonraki sayfalarda kayda değer workspace varsa en küçüğü (`>` göstergesinin hedefi).
    pub next: Option<i64>,
    /// Bu monitörde aktif adlı workspace (`name:foo`): sayfa noktalarına girmez, ayrılmış yerde
    /// gösterilir.
    pub named: Option<NamedWorkspace>,
    /// Bu monitörde açık special workspace'in adı (`special:` öneki atılmış).
    pub special: Option<String>,
}

/// Aktif adlı workspace.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NamedWorkspace {
    pub id: i64,
    pub name: String,
    /// Temsilci pencerenin sınıfı (boş = yok).
    pub app_class: String,
}

/// Numarasız, special olmayan aktif workspace → adlı workspace.
fn named_workspace(snapshot: &HyprSnapshot, ws: &WorkspaceRef) -> Option<NamedWorkspace> {
    if ws.id >= 1 || ws.name.is_empty() || is_special_workspace(ws.id, &ws.name) {
        return None;
    }
    let name = ws.name.strip_prefix("name:").unwrap_or(&ws.name).to_owned();
    let app_class = snapshot.workspace(ws.id).map(|w| w.app_class.clone()).unwrap_or_default();
    Some(NamedWorkspace { id: ws.id, name, app_class })
}

/// `special:magic` → `magic`, `special` → `special`.
fn special_name(name: &str) -> String {
    match name.strip_prefix("special:") {
        Some(n) if !n.is_empty() => n.to_owned(),
        _ => "special".to_owned(),
    }
}

impl WorkspacePage {
    /// Bu monitörün aktif workspace'inin yuva sırası (sayfada değilse `None`: hap gizli).
    pub fn active_index(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.active)
    }
}

/// Numaralı workspace'in sayfası; numarasızda (adlı, special, 0) `None`.
pub fn page_of(id: i64, count: u32) -> Option<i64> {
    (id >= 1).then(|| (id - 1) / i64::from(count.max(1)))
}

/// Kayda değer (penceresi olan ya da bir monitörde görünen) numaralı workspace'ler, artan.
pub fn notable_ids(snapshot: &HyprSnapshot) -> Vec<i64> {
    let occupied = snapshot.workspaces.iter().filter(|w| !w.is_special() && w.windows > 0).map(|w| w.id);
    let visible = snapshot.monitors.iter().filter(|m| !m.disabled).map(|m| m.active_workspace.id);
    let mut ids: Vec<i64> = occupied.chain(visible).filter(|&id| id >= 1).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// `monitor`'ün sayfası. Aktif workspace numarasızsa ya da monitör bilinmiyorsa `fallback_page`.
pub fn workspace_page(snapshot: &HyprSnapshot, monitor: &str, count: u32, fallback_page: i64) -> WorkspacePage {
    let count = count.max(1);
    let n = i64::from(count);
    let mon = snapshot.monitor(monitor);
    let active = mon.map(|m| m.active_workspace.id);
    let page = active.and_then(|id| page_of(id, count)).unwrap_or(fallback_page.max(0));
    let first = page.saturating_mul(n).saturating_add(1);
    let last = first.saturating_add(n - 1);

    let slots = (first..=last)
        .map(|id| {
            let ws = snapshot.workspace(id);
            SlotState {
                id,
                occupied: ws.is_some_and(|w| w.windows > 0),
                active: active == Some(id),
                visible_elsewhere: snapshot
                    .monitors
                    .iter()
                    .any(|m| m.name != monitor && !m.disabled && m.active_workspace.id == id),
                app_class: ws.map(|w| w.app_class.clone()).unwrap_or_default(),
            }
        })
        .collect();

    let notable = notable_ids(snapshot);
    WorkspacePage {
        page,
        slots,
        prev: notable.iter().rev().copied().find(|&id| id < first),
        next: notable.iter().copied().find(|&id| id > last),
        named: mon.and_then(|m| named_workspace(snapshot, &m.active_workspace)),
        special: mon.and_then(|m| m.special()).map(|s| special_name(&s.name)),
    }
}

/// Kaydırma hedefi. `steps` > 0 sonraki, < 0 önceki. Hareket yoksa `None`.
/// - `All`: `from + steps`, alt sınır 1.
/// - `Occupied`: `notable` içinde `from`'dan itibaren `|steps|` adım (uçta durur).
/// - `from` numarasızsa (adlı workspace) `All` için 1'den, `Occupied` için ilk kayda değerden başlanmaz: `None`.
pub fn scroll_target(mode: WorkspaceScroll, notable: &[i64], from: i64, steps: i32) -> Option<i64> {
    if steps == 0 || from < 1 {
        return None;
    }
    let target = match mode {
        WorkspaceScroll::Off => return None,
        WorkspaceScroll::All => from.saturating_add(i64::from(steps)).max(1),
        WorkspaceScroll::Occupied => {
            let mut cur = from;
            for _ in 0..steps.unsigned_abs() {
                let next = if steps > 0 {
                    notable.iter().copied().find(|&id| id > cur)
                } else {
                    notable.iter().rev().copied().find(|&id| id < cur)
                };
                match next {
                    Some(id) => cur = id,
                    None => break,
                }
            }
            cur
        }
    };
    (target != from).then_some(target)
}

/// Kaydırmayla workspace geçişinin tek yolu (widget ve bar uç eylemi `workspace`): hedef
/// [`scroll_target`] ile hesaplanır; gönderilmiş ama Hyprland'den henüz dönmemiş hedef
/// [`PENDING_TTL`] boyunca sonraki adımın başlangıcıdır (hızlı kaydırmada bayat aktif id'den
/// aynı hedefi iki kez göndermemek için).
#[derive(Debug, Clone, Default)]
pub struct WorkspaceScroller {
    pending: Option<(i64, Instant)>,
}

impl WorkspaceScroller {
    /// `active`: monitörün (modeldeki) aktif workspace'i. `steps` > 0 sonraki. Döner: gönderilecek
    /// hedef.
    pub fn step(
        &mut self,
        mode: WorkspaceScroll,
        notable: &[i64],
        active: Option<i64>,
        steps: i32,
        now: Instant,
    ) -> Option<i64> {
        self.observe(active, now);
        let from = self.pending.map(|(id, _)| id).or(active)?;
        let target = scroll_target(mode, notable, from, steps)?;
        self.pending = Some((target, now));
        Some(target)
    }

    /// Model güncellendi: hedefe varıldıysa ya da süresi geçtiyse bekleyen düşer.
    pub fn observe(&mut self, active: Option<i64>, now: Instant) {
        if self.pending.is_some_and(|(id, at)| Some(id) == active || now.saturating_duration_since(at) > PENDING_TTL) {
            self.pending = None;
        }
    }

    #[cfg(test)]
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

/// Dokunmatik yüzeyde bir adım için gereken kaydırma (mantıksal piksel).
pub const TOUCHPAD_STEP_PX: f32 = 48.0;
/// Bu kadar sessizlikten sonra birikim sıfırlanır.
pub const SCROLL_IDLE: Duration = Duration::from_millis(400);
/// Gönderilen hedef bu süre boyunca sonraki adımın başlangıcıdır (Hyprland olayı gelene dek).
pub const PENDING_TTL: Duration = Duration::from_millis(600);

/// Tekerlek çentiği ve dokunmatik yüzey pikselini workspace adımına çevirir.
#[derive(Debug, Clone, Default)]
pub struct ScrollAccumulator {
    acc: f32,
    last: Option<Instant>,
}

impl ScrollAccumulator {
    /// Döner: adım sayısı (+ sonraki / − önceki). Baskın eksen kullanılır; aşağı ve sağ ileri.
    /// `steps` (tekerlek çentiği) varsa doğrudan, yoksa `delta` pikselleri birikir.
    pub fn feed(&mut self, delta: Point, steps: Point, now: Instant) -> i32 {
        if self.last.is_some_and(|t| now.saturating_duration_since(t) > SCROLL_IDLE) {
            self.acc = 0.0;
        }
        self.last = Some(now);
        let dominant = |p: Point| if p.y.abs() >= p.x.abs() { p.y } else { p.x };
        // Çentik bir tam adımdır; yüksek çözünürlüklü tekerleğin kesirli çentikleri birikir.
        let notches = dominant(steps);
        let d = if notches != 0.0 { notches * TOUCHPAD_STEP_PX } else { dominant(delta) };
        if !d.is_finite() || d == 0.0 {
            return 0;
        }
        if d.signum() != self.acc.signum() && self.acc != 0.0 {
            self.acc = 0.0; // yön değişti
        }
        self.acc += d;
        let n = (self.acc / TOUCHPAD_STEP_PX).trunc();
        self.acc -= n * TOUCHPAD_STEP_PX;
        n.clamp(-10.0, 10.0) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::hyprland::{Monitor, Workspace, WorkspaceRef};

    fn ws(id: i64, windows: u32) -> Workspace {
        Workspace { id, name: id.to_string(), windows, ..Default::default() }
    }

    fn mon(name: &str, active: i64) -> Monitor {
        Monitor { name: name.into(), active_workspace: WorkspaceRef { id: active, name: active.to_string() }, ..Default::default() }
    }

    fn snap(monitors: Vec<Monitor>, workspaces: Vec<Workspace>) -> HyprSnapshot {
        HyprSnapshot { monitors, workspaces, active_window: None }
    }

    fn ids(p: &WorkspacePage) -> Vec<i64> {
        p.slots.iter().map(|s| s.id).collect()
    }

    #[test]
    fn page_of_numbers() {
        assert_eq!(page_of(1, 10), Some(0));
        assert_eq!(page_of(10, 10), Some(0));
        assert_eq!(page_of(11, 10), Some(1));
        assert_eq!(page_of(6, 5), Some(1));
        assert_eq!(page_of(0, 10), None);
        assert_eq!(page_of(-1337, 10), None, "adlı workspace");
        assert_eq!(page_of(5, 0), Some(4), "count 0 → 1 kabul edilir");
        assert_eq!(page_of(i64::MAX, 30), Some((i64::MAX - 1) / 30), "taşma yok");
    }

    #[test]
    fn always_count_slots_on_active_page() {
        let s = snap(vec![mon("A", 2)], vec![ws(1, 1), ws(2, 0)]);
        let p = workspace_page(&s, "A", 10, 0);
        assert_eq!(ids(&p), (1..=10).collect::<Vec<_>>());
        assert_eq!(p.active_index(), Some(1));
        assert!(p.slots[0].occupied && !p.slots[1].occupied && !p.slots[5].occupied);
        assert_eq!((p.prev, p.next), (None, None));

        let p = workspace_page(&s, "A", 5, 0);
        assert_eq!(p.slots.len(), 5);
    }

    #[test]
    fn eleventh_workspace_is_page_two_with_prev_indicator() {
        let s = snap(vec![mon("A", 11)], vec![ws(3, 2), ws(11, 1), ws(25, 1)]);
        let p = workspace_page(&s, "A", 10, 0);
        assert_eq!(p.page, 1);
        assert_eq!(ids(&p), (11..=20).collect::<Vec<_>>());
        assert_eq!(p.active_index(), Some(0));
        assert_eq!(p.prev, Some(3), "önceki sayfadaki dolu");
        assert_eq!(p.next, Some(25), "sonraki sayfadaki en yakın dolu");
    }

    #[test]
    fn indicators_count_visible_on_other_monitor_but_not_empty_persistent() {
        // 5 boş ama B'de görünür; 7 kalıcı-boş (görünmez) sayılmaz.
        let s = snap(vec![mon("A", 12), mon("B", 5)], vec![ws(5, 0), ws(7, 0), ws(12, 1)]);
        let p = workspace_page(&s, "A", 10, 0);
        assert_eq!(p.prev, Some(5));
        let b = workspace_page(&s, "B", 10, 0);
        assert_eq!(b.next, Some(12), "A'nın görünür workspace'i B için sonraki sayfada");
        assert!(b.slots[4].active && !b.slots[4].visible_elsewhere);
        let a1 = workspace_page(&snap(vec![mon("A", 2), mon("B", 5)], vec![]), "A", 10, 0);
        assert!(a1.slots[4].visible_elsewhere && !a1.slots[4].active);
        assert!(!a1.slots[1].visible_elsewhere, "kendi aktifi 'başka monitörde' değil");
    }

    #[test]
    fn disabled_monitor_does_not_count() {
        let mut b = mon("B", 15);
        b.disabled = true;
        let p = workspace_page(&snap(vec![mon("A", 1), b], vec![]), "A", 10, 0);
        assert_eq!(p.next, None);
        assert!(p.slots.iter().all(|s| !s.visible_elsewhere));
    }

    #[test]
    fn named_special_and_unknown_monitor_keep_fallback_page() {
        let named = Workspace { id: -1337, name: "notes".into(), windows: 3, ..Default::default() };
        let special = Workspace { id: -98, name: "special:magic".into(), windows: 2, ..Default::default() };
        let mut m = mon("A", -1337);
        m.active_workspace.name = "notes".into();
        let s = snap(vec![m], vec![ws(2, 1), named, special]);
        let p = workspace_page(&s, "A", 10, 1);
        assert_eq!(p.page, 1, "adlı workspace aktif: son sayfa korunur");
        assert_eq!(p.active_index(), None, "hap gizli");
        assert_eq!(p.prev, Some(2));
        assert_eq!(notable_ids(&s), [2], "adlı ve special sayılmaz");

        let p = workspace_page(&s, "HDMI-A-9", 10, 0);
        assert_eq!(p.page, 0);
        assert_eq!(p.active_index(), None);
        assert_eq!(workspace_page(&s, "HDMI-A-9", 10, -4).page, 0, "negatif yedek sayfa 0");
    }

    #[test]
    fn named_and_special_are_reported_outside_page() {
        let mut named = Workspace { id: -1337, name: "notes".into(), windows: 1, ..Default::default() };
        named.app_class = "obsidian".into();
        let mut m = mon("A", -1337);
        m.active_workspace.name = "notes".into();
        m.special_workspace = WorkspaceRef { id: -98, name: "special:magic".into() };
        let s = snap(vec![m, mon("B", 3)], vec![named]);
        let p = workspace_page(&s, "A", 10, 0);
        assert_eq!(p.named, Some(NamedWorkspace { id: -1337, name: "notes".into(), app_class: "obsidian".into() }));
        assert_eq!(p.special.as_deref(), Some("magic"));
        assert_eq!(p.slots.len(), 10, "noktalar etkilenmez");
        let b = workspace_page(&s, "B", 10, 0);
        assert_eq!((b.named, b.special), (None, None), "başka monitörün adlı/special'ı sayılmaz");

        let mut m = mon("A", 2);
        m.special_workspace = WorkspaceRef { id: -99, name: "special".into() };
        let p = workspace_page(&snap(vec![m], vec![]), "A", 10, 0);
        assert_eq!((p.named, p.special.as_deref()), (None, Some("special")));
        // Special workspace aktif workspace olarak bildirilirse adlı sayılmaz.
        let mut m = mon("A", -98);
        m.active_workspace.name = "special:x".into();
        assert_eq!(workspace_page(&snap(vec![m], vec![]), "A", 10, 0).named, None);
    }

    #[test]
    fn app_class_flows_into_slots() {
        let mut w = ws(3, 1);
        w.app_class = "firefox".into();
        let p = workspace_page(&snap(vec![mon("A", 1)], vec![w]), "A", 10, 0);
        assert_eq!(p.slots[2].app_class, "firefox");
        assert_eq!(p.slots[0].app_class, "");
    }

    #[test]
    fn scroll_targets() {
        use WorkspaceScroll::*;
        let notable = [1, 4, 12];
        assert_eq!(scroll_target(All, &notable, 4, 1), Some(5));
        assert_eq!(scroll_target(All, &notable, 4, -2), Some(2));
        assert_eq!(scroll_target(All, &notable, 1, -1), None, "1'in altı yok");
        assert_eq!(scroll_target(All, &notable, 2, -5), Some(1));
        assert_eq!(scroll_target(Occupied, &notable, 4, 1), Some(12));
        assert_eq!(scroll_target(Occupied, &notable, 5, -1), Some(4), "aktif kayda değer olmasa da");
        assert_eq!(scroll_target(Occupied, &notable, 1, 5), Some(12), "uçta durur");
        assert_eq!(scroll_target(Occupied, &notable, 12, 1), None);
        assert_eq!(scroll_target(Off, &notable, 4, 1), None);
        assert_eq!(scroll_target(All, &notable, -1337, 1), None, "adlı workspace'ten kaydırma yok");
        assert_eq!(scroll_target(All, &notable, 4, 0), None);
        assert_eq!(scroll_target(All, &notable, i64::MAX, 1), None, "taşma yok");
    }

    #[test]
    fn scroller_chains_pending_until_reached_or_expired() {
        use WorkspaceScroll::*;
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut s = WorkspaceScroller::default();
        assert_eq!(s.step(All, &[], Some(3), 1, ms(0)), Some(4));
        assert_eq!(s.step(All, &[], Some(3), 1, ms(50)), Some(5), "Hyprland dönmeden: bekleyenden devam");
        s.observe(Some(5), ms(80));
        assert!(!s.is_pending(), "hedefe varıldı");
        assert_eq!(s.step(Occupied, &[1, 5, 9], Some(5), -1, ms(100)), Some(1));
        assert_eq!(s.step(All, &[], Some(5), 1, ms(100 + 601)), Some(6), "süresi geçen bekleyen düşer");
        assert_eq!(s.step(All, &[], None, 1, ms(5000)), None, "monitör bilinmiyor");
        assert_eq!(s.step(Off, &[], Some(2), 1, ms(6000)), None);
    }

    #[test]
    fn wheel_notches_step_directly() {
        let t = Instant::now();
        let mut a = ScrollAccumulator::default();
        assert_eq!(a.feed(Point::new(0.0, 15.0), Point::new(0.0, 1.0), t), 1, "aşağı = sonraki");
        assert_eq!(a.feed(Point::new(0.0, -30.0), Point::new(0.0, -2.0), t), -2);
        assert_eq!(a.feed(Point::new(0.0, 0.0), Point::new(1.0, 0.0), t), 1, "yatay tekerlek: sağ = sonraki");
        assert_eq!(a.feed(Point::ZERO, Point::new(0.0, f32::NAN), t), 0);
        assert_eq!(a.feed(Point::ZERO, Point::new(0.0, 1e6), t), 10, "sınırlı");
        let mut a = ScrollAccumulator::default();
        let quarter = |a: &mut ScrollAccumulator| a.feed(Point::new(0.0, 4.0), Point::new(0.0, 0.25), t);
        assert_eq!([quarter(&mut a), quarter(&mut a), quarter(&mut a), quarter(&mut a)], [0, 0, 0, 1], "kesirli çentik birikir");
    }

    #[test]
    fn touchpad_accumulates_and_resets() {
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut a = ScrollAccumulator::default();
        assert_eq!(a.feed(Point::new(0.0, 20.0), Point::ZERO, ms(0)), 0);
        assert_eq!(a.feed(Point::new(0.0, 20.0), Point::ZERO, ms(16)), 0);
        assert_eq!(a.feed(Point::new(0.0, 20.0), Point::ZERO, ms(32)), 1, "48 px = bir adım");
        assert_eq!(a.feed(Point::new(0.0, 100.0), Point::ZERO, ms(48)), 2, "kalan 12 + 100");
        // Yön değişimi birikimi sıfırlar.
        assert_eq!(a.feed(Point::new(0.0, -40.0), Point::ZERO, ms(64)), 0);
        assert_eq!(a.feed(Point::new(0.0, -10.0), Point::ZERO, ms(80)), -1);
        // Sessizlik birikimi sıfırlar.
        assert_eq!(a.feed(Point::new(0.0, 40.0), Point::ZERO, ms(100)), 0);
        assert_eq!(a.feed(Point::new(0.0, 40.0), Point::ZERO, ms(1000)), 0, "400 ms sonra baştan");
        assert_eq!(a.feed(Point::new(-30.0, 3.0), Point::ZERO, ms(1010)), 0, "yatay baskın, yön değişti");
        assert_eq!(a.feed(Point::new(-20.0, 0.0), Point::ZERO, ms(1020)), -1);
        assert_eq!(a.feed(Point::new(f32::INFINITY, 0.0), Point::ZERO, ms(1030)), 0);
    }
}
