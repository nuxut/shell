use std::time::Duration;

use nuxut::Theme;

use super::*;
use crate::services::hyprland::{Monitor, Workspace, WorkspaceRef};
use crate::widgets::WidgetSettings;

fn cx<'a>(theme: &'a Theme, settings: &'a WidgetSettings) -> WidgetCx<'a> {
    WidgetCx {
        theme,
        settings,
        vertical: false,
        item: 26.0,
        now: Instant::now(),
        bounds: nuxut::Rect::new(0.0, 0.0, 1920.0, 1080.0),
    }
}

fn ws(id: i64, windows: u32) -> Workspace {
    Workspace { id, name: id.to_string(), windows, ..Default::default() }
}

fn set_active(model: &mut ShellModel, id: i64) {
    model.hypr.monitors =
        vec![Monitor { name: "M".into(), active_workspace: WorkspaceRef { id, name: id.to_string() }, ..Default::default() }];
}

#[test]
fn slot_at_interpolates_and_extrapolates() {
    let slots = [Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(14.0, 0.0, 10.0, 10.0), Rect::new(28.0, 0.0, 10.0, 10.0)];
    assert_eq!(slot_at(&slots, 1.0), slots[1]);
    assert_eq!(slot_at(&slots, 0.5).x, 7.0);
    assert_eq!(slot_at(&slots, 2.25).x, 31.5, "yay taşması uçtan dışarı");
    assert_eq!(slot_at(&slots, -0.5).x, -7.0);
    assert_eq!(slot_at(&slots[..1], 3.0), slots[0]);
    assert_eq!(slot_at(&[], 0.0), Rect::ZERO);
}

#[test]
fn overlap_ratio_is_area_fraction() {
    let a = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert_eq!(overlap_ratio(a, Rect::new(5.0, 0.0, 10.0, 10.0)), 0.5);
    assert_eq!(overlap_ratio(a, Rect::new(0.0, 5.0, 10.0, 10.0)), 0.5, "dikey kayma");
    assert_eq!(overlap_ratio(a, Rect::new(20.0, 0.0, 10.0, 10.0)), 0.0);
    assert_eq!(overlap_ratio(a, a), 1.0);
}

#[test]
fn content_per_style() {
    let full = SlotState { id: 3, occupied: true, app_class: "firefox".into(), ..Default::default() };
    let unknown = SlotState { id: 4, occupied: true, app_class: "mystery".into(), ..Default::default() };
    let empty = SlotState { id: 5, ..Default::default() };
    use WorkspaceStyle::*;
    assert_eq!(slot_content(Icons, &full, true), Content::Glyph('\u{f0239}'));
    assert_eq!(slot_content(Icons, &full, false), Content::Number, "Nerd Font yok: numara");
    assert_eq!(slot_content(Icons, &unknown, true), Content::Number, "bilinmeyen sınıf: numara");
    assert_eq!(slot_content(Icons, &empty, true), Content::Dot { big: false });
    assert_eq!(slot_content(Numbers, &empty, true), Content::Number);
    assert_eq!(slot_content(Dots, &full, true), Content::Dot { big: true });
    assert_eq!(slot_content(Dots, &empty, true), Content::Dot { big: false });
}

/// Regresyon: `Screen::draw` bar'ın hareketini (`is_animating`) widget'lar çizilmeden önce
/// sorar. Hap hedefi `draw` içinde kurulursa geçiş karesi "durgun" görünür, sonraki kare
/// istenmez. Hedef modelle (`sync`) kurulmalı.
#[test]
fn active_change_animates_before_any_draw() {
    let theme = Theme::default();
    let settings = WidgetSettings::default();
    let mut cx = cx(&theme, &settings);
    let mut model = ShellModel::default();
    model.hypr.workspaces = (1..=3).map(|id| ws(id, 1)).collect();
    set_active(&mut model, 1);
    let mut w = Workspaces::new(&cx);
    assert!(w.sync(&model, "M", &cx));
    assert_eq!(w.slots.len(), 10, "her zaman sayfa dolu");
    cx.now += Duration::from_secs(1);
    assert!(!w.is_animating(cx.now), "ilk yerleşim animasyonsuz");

    for (id, gap_ms) in [(2, 0), (3, 30), (2, 2000), (1, 2000), (7, 2000)] {
        cx.now += Duration::from_millis(gap_ms);
        set_active(&mut model, id);
        assert!(w.sync(&model, "M", &cx), "{id}: değişiklik bildirilir");
        assert!(w.is_animating(cx.now), "{id}: çizimden önce hareket görünür");
        assert_eq!(w.pill.target(), (id - 1) as f32);
    }

    // İlgisiz değişiklik süren hareketi bozmaz.
    model.hypr.workspaces[2].app_class = "firefox".into();
    assert!(w.sync(&model, "M", &cx));
    assert!(w.is_animating(cx.now), "ilgisiz değişiklik hareketi kesmez");
}

#[test]
fn page_change_jumps_and_shows_arrows() {
    let theme = Theme::default();
    let settings = WidgetSettings::default();
    let mut cx = cx(&theme, &settings);
    let mut model = ShellModel::default();
    model.hypr.workspaces = vec![ws(2, 1), ws(11, 1)];
    set_active(&mut model, 10);
    let mut w = Workspaces::new(&cx);
    w.sync(&model, "M", &cx);
    assert_eq!((w.prev.target, w.next.target), (None, Some(11)));
    cx.now += Duration::from_secs(2);

    set_active(&mut model, 11);
    w.sync(&model, "M", &cx);
    assert_eq!(w.slots[0].state.id, 11);
    assert_eq!(w.pill.target(), 0.0);
    assert!(w.pill.is_settled(cx.now), "sayfa değişince hap kaymaz, atlar");
    assert_eq!((w.prev.target, w.next.target), (Some(2), None), "10 artık boş ve görünmez; önceki dolu 2");
    assert!(w.is_animating(cx.now), "ok görünürlüğü solar");

    // Adlı workspace aktif: sayfa korunur, hap gizlenir.
    model.hypr.monitors[0].active_workspace = WorkspaceRef { id: -1337, name: "notes".into() };
    w.sync(&model, "M", &cx);
    assert_eq!(w.slots[0].state.id, 11);
    assert!(!w.pill_visible);
}

#[test]
fn arrows_reserve_space_and_slots_never_shift() {
    let theme = Theme::default();
    let settings = WidgetSettings::default();
    let mut cx = cx(&theme, &settings);
    let mut model = ShellModel::default();
    set_active(&mut model, 1);
    let mut w = Workspaces::new(&cx);
    w.sync(&model, "M", &cx);
    let rect = Rect::new(10.0, 0.0, 400.0, 34.0);
    let before = w.layout(rect, &cx);
    model.hypr.workspaces = vec![ws(15, 1)];
    w.sync(&model, "M", &cx);
    assert!(w.next.target.is_some());
    assert_eq!(w.layout(rect, &cx), before, "ok görünse de yuvalar yerinde");

    // Yatay: ok (16) + boşluk (4) sonra yuvalar.
    assert_eq!(before.prev, Rect::new(10.0, 4.0, 16.0, 26.0));
    assert_eq!(before.slots[1], Rect::new(10.0 + 20.0 + 30.0, 4.0, 26.0, 26.0));
    assert_eq!(before.next.x, before.slots[9].right() + 4.0);
    assert_eq!(w.click(rect, Point::new(before.slots[1].center().x, 17.0), &cx), Some(Action::SwitchWorkspace(2)));
    assert_eq!(w.click(rect, before.next.center(), &cx), Some(Action::SwitchWorkspace(15)));
    assert_eq!(w.click(rect, before.prev.center(), &cx), None, "görünmez ok tıklanmaz");

    cx.vertical = true;
    let v = w.layout(Rect::new(0.0, 100.0, 34.0, 400.0), &cx);
    assert_eq!(v.prev, Rect::new(4.0, 100.0, 26.0, 16.0));
    assert_eq!(v.slots[2], Rect::new(4.0, 100.0 + 20.0 + 60.0, 26.0, 26.0));
}

#[test]
fn scroll_switches_and_chains_pending_target() {
    let theme = Theme::default();
    let settings = WidgetSettings::default();
    let mut cx = cx(&theme, &settings);
    let mut model = ShellModel::default();
    set_active(&mut model, 3);
    let mut w = Workspaces::new(&cx);
    w.sync(&model, "M", &cx);
    let mut text = TextSystem::new();
    let rect = Rect::new(0.0, 0.0, 400.0, 34.0);
    w.last_rect = Some(rect);
    let wheel = |dy: f32| PointerEvent::new(Point::new(100.0, 17.0), PointerKind::Scroll { delta: Point::new(0.0, dy * 15.0), steps: Point::new(0.0, dy) });

    assert_eq!(w.overlay_pointer(&wheel(1.0), &mut text, &cx), Some((false, Some(Action::SwitchWorkspace(4)))));
    // Hyprland henüz bildirmedi: sonraki çentik bekleyen hedeften devam eder.
    cx.now += Duration::from_millis(50);
    assert_eq!(w.overlay_pointer(&wheel(1.0), &mut text, &cx), Some((false, Some(Action::SwitchWorkspace(5)))));
    // Bildirim geldi; bekleyen düşer.
    set_active(&mut model, 5);
    w.sync(&model, "M", &cx);
    assert!(!w.scroller.is_pending());
    cx.now += Duration::from_millis(50);
    assert_eq!(w.overlay_pointer(&wheel(-1.0), &mut text, &cx), Some((false, Some(Action::SwitchWorkspace(4)))));
    // Bekleyen eskidi (Hyprland reddetti): aktiften devam.
    cx.now += Duration::from_secs(2);
    assert_eq!(w.overlay_pointer(&wheel(-1.0), &mut text, &cx), Some((false, Some(Action::SwitchWorkspace(4)))));

    // Dışarıdaki kaydırma ve kaydırma olmayan olaylar tüketilmez.
    let outside = PointerEvent::new(Point::new(900.0, 17.0), PointerKind::Scroll { delta: Point::ZERO, steps: Point::new(0.0, 1.0) });
    assert_eq!(w.overlay_pointer(&outside, &mut text, &cx), None);
    assert_eq!(w.overlay_pointer(&PointerEvent::new(Point::new(100.0, 17.0), PointerKind::Motion), &mut text, &cx), None);

    // `scroll = "off"`.
    let mut off = WidgetSettings::default();
    off.workspaces.scroll = WorkspaceScroll::Off;
    let cx_off = WidgetCx { settings: &off, ..cx };
    assert_eq!(w.overlay_pointer(&wheel(1.0), &mut text, &cx_off), None);
}

/// Bar uç eylemi önceliği (`EdgeBinding::route`, `Screen::pointer` 0. adım): uç widget'ı
/// workspaces iken varsayılan `scroll = "widget"` kaydırmayı widget'a bırakır ve widget geçiş
/// yapar; `widget` dışında bir eylem atanmışsa olay widget'a hiç ulaşmaz.
#[test]
fn edge_actions_priority_over_workspaces_widget() {
    use crate::bar_edge::{EdgeBinding, EdgeRoute, ScrollAction};
    let theme = Theme::default();
    let settings = WidgetSettings::default();
    let cx = cx(&theme, &settings);
    let mut model = ShellModel::default();
    set_active(&mut model, 3);
    let mut w = Workspaces::new(&cx);
    w.sync(&model, "M", &cx);
    w.last_rect = Some(Rect::new(0.0, 0.0, 400.0, 34.0));
    let mut text = TextSystem::new();
    let wheel = PointerEvent::new(Point::new(100.0, 17.0), PointerKind::Scroll { delta: Point::new(0.0, 15.0), steps: Point::new(0.0, 1.0) });

    // Varsayılan: uç eylemi yok, widget geçiş yapar.
    assert_eq!(EdgeBinding::default().route(&wheel.kind), None);
    assert_eq!(w.overlay_pointer(&wheel, &mut text, &cx), Some((false, Some(Action::SwitchWorkspace(4)))));
    // Atanmış eylem uç eylemine gider (Screen olayı widget'a iletmez).
    for action in [ScrollAction::Volume, ScrollAction::Workspace, ScrollAction::None] {
        let b = EdgeBinding { scroll: action, ..Default::default() };
        assert!(matches!(b.route(&wheel.kind), Some(EdgeRoute::Scroll(_) | EdgeRoute::Done(None))), "{action:?}");
    }
    // Tık: varsayılan widget'ın (yuva → workspace).
    let release = PointerKind::Release(nuxut::Button::Left);
    assert_eq!(EdgeBinding::default().route(&release), None);
}

#[test]
fn extra_slot_text_and_tooltip() {
    crate::i18n::use_turkish_for_tests();
    assert_eq!(extra_tooltip(Some("notes"), None), "notes");
    assert_eq!(extra_tooltip(None, Some("magic")), "Özel çalışma alanı: magic");
    assert_eq!(extra_tooltip(Some("notes"), Some("magic")), "notes · Özel çalışma alanı: magic");
    assert_eq!(extra_tooltip(None, None), "");
    assert_eq!(named_label("notes", "obsidian", true), (app_glyph("obsidian").unwrap().to_string(), true));
    assert_eq!(named_label("ödev", "mystery", true), ("Ö".into(), false), "glif yoksa baş harf, büyük");
    assert_eq!(named_label("notes", "obsidian", false), ("N".into(), false), "Nerd Font yoksa baş harf");
    assert_eq!(named_label("", "", true), (String::new(), false));
}

#[test]
fn named_and_special_use_reserved_slot_with_tooltip() {
    crate::i18n::use_turkish_for_tests();
    let theme = Theme::default();
    let settings = WidgetSettings::default();
    let mut cx = cx(&theme, &settings);
    let mut model = ShellModel::default();
    set_active(&mut model, 2);
    let mut w = Workspaces::new(&cx);
    w.sync(&model, "M", &cx);
    let rect = Rect::new(0.0, 0.0, 420.0, 34.0);
    let before = w.layout(rect, &cx);
    assert_eq!(before.extra.x, before.next.right() + 4.0, "yuva okun dışında, hep ayrılı");
    assert_eq!(w.tooltip(rect, before.extra.center(), &cx), None, "boşken ipucu yok");

    // Adlı workspace aktif + special açık.
    cx.now += Duration::from_secs(1);
    model.hypr.monitors[0].active_workspace = WorkspaceRef { id: -1337, name: "notes".into() };
    model.hypr.monitors[0].special_workspace = WorkspaceRef { id: -98, name: "special:magic".into() };
    assert!(w.sync(&model, "M", &cx));
    assert!(w.is_animating(cx.now), "belirme solarak");
    assert_eq!(w.layout(rect, &cx), before, "noktalar ve oklar kaymaz");
    // Ipucu metni ve çapası: yalnızca hover'daki ek yuva (çapa = yuvanın rect'i, widget rect'i değil).
    let tip = w.tooltip(rect, before.extra.center(), &cx).expect("ek yuvada ipucu");
    assert_eq!(tip.text, "notes · Özel çalışma alanı: magic");
    assert_eq!(tip.anchor, Some(before.extra), "balon yalnızca hover'daki yuvanın altına");
    assert!(before.extra.x > before.slots[0].x && before.extra.height == before.slots[0].height, "çapa ayrı bir yuva");
    assert!(w.tooltip(rect, before.slots[0].center(), &cx).is_none(), "nokta yuvasında ipucu yok");
    assert_eq!(w.extra.label.text(), "N");
    assert!(!w.pill_visible);
}

// --- Yüksek karşıtlık (framework t124): yuva renkleri framework yardımcılarından ---

fn high_theme(mode: &str) -> Theme {
    Theme::parse(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"high\"\n")).unwrap().0
}

fn slot(occupied: bool, active: bool, elsewhere: bool) -> SlotState {
    SlotState { id: 1, occupied, active, visible_elsewhere: elsewhere, app_class: String::new() }
}

#[test]
fn slot_colors_standard_mode_is_unchanged() {
    let theme = Theme::default();
    let c = &theme.colors;
    for text in [false, true] {
        assert_eq!(
            slot_colors(&theme, &slot(true, false, true), text),
            (Some(c.primary.multiply_alpha(ELSEWHERE_ALPHA)), c.primary)
        );
        assert_eq!(slot_colors(&theme, &slot(true, false, false), text), (None, c.on_surface));
        assert_eq!(slot_colors(&theme, &slot(false, false, false), text), (None, c.on_surface_variant.multiply_alpha(EMPTY_ALPHA)));
    }
    assert_eq!(tonal_disk(&theme, 0.05), c.primary.multiply_alpha(0.05));
}

#[test]
fn slot_colors_meet_thresholds_in_high_contrast() {
    use nuxut::theme::contrast::{NON_TEXT_MIN, TEXT_MIN};
    for mode in ["dark", "light"] {
        let theme = high_theme(mode);
        let c = &theme.colors;
        let surfaces = style::backdrops(&theme, &[]);
        // Boş yuva: metin ≥ 4.5, nokta ≥ 3; soluk %55 bu eşiği standart değerle tutmaz.
        let (_, text) = slot_colors(&theme, &slot(false, false, false), true);
        assert!(style::worst_over(text, &surfaces) >= TEXT_MIN, "{mode}: boş yuva metni");
        let (_, dot) = slot_colors(&theme, &slot(false, false, false), false);
        assert!(style::worst_over(dot, &surfaces) >= NON_TEXT_MIN, "{mode}: boş yuva noktası");
        // Tonal disk üstündeki primary içerik ≥ 4.5 (disk bindirilmiş zemine karşı).
        let (disk, fg) = slot_colors(&theme, &slot(true, false, true), true);
        let disk = disk.unwrap();
        assert_eq!(disk, style::state_layer(&theme, c.primary, &[], ELSEWHERE_ALPHA));
        assert!(style::worst_over(fg, &style::backdrops(&theme, &[disk])) >= TEXT_MIN, "{mode}: disk üstü içerik");
    }
}

/// Aktif hapın üstündeki yuvanın hover/basılı katmanı (zemin `primary`) yüksek karşıtlıkta
/// `on_primary` içeriği ≥ 4.5 bırakır; standartta katman eski `on_surface × opaklık`.
#[test]
fn hover_over_active_pill_keeps_on_primary_legible() {
    use nuxut::theme::contrast::TEXT_MIN;
    let std_theme = Theme::default();
    let c = &std_theme.colors;
    assert_eq!(
        style::state_layer_under(&std_theme, c.on_surface, 0.12, &[c.primary], &[c.on_primary]),
        c.on_surface.multiply_alpha(0.12)
    );
    for mode in ["dark", "light"] {
        let theme = high_theme(mode);
        let c = &theme.colors;
        for opacity in [0.08, 0.12, 0.16, 1.0] {
            let layer = style::state_layer_under(&theme, c.on_surface, opacity, &[c.primary], &[c.on_primary]);
            let ratio = style::worst_over(c.on_primary, &style::backdrops(&theme, &[c.primary, layer]));
            assert!(ratio >= TEXT_MIN, "{mode} opaklık {opacity}: {ratio}");
        }
    }
}
