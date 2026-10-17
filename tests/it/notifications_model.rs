//! `NotificationCenter`: süre dolumu, critical, hover duraklatma, DND, replaces_id, transient,
//! geçmiş/popup sınırları, okunmamış sayısı, gruplama, ayar doğrulama. Zaman sahte.

use std::time::{Duration, Instant, SystemTime};

use nuxut_shell::services::notifications::{
    AppRule, CloseReason, Expiry, Notification, NotificationCenter, NotificationsConfig, NotifyEvent as Ev, NotifyIcon,
    Urgency, markup::plain_body,
};

fn n(id: u32) -> Notification {
    Notification {
        id,
        app_name: "app".into(),
        app_icon: NotifyIcon::None,
        summary: format!("#{id}"),
        body: plain_body("gövde"),
        actions: vec![],
        has_default_action: false,
        urgency: Urgency::Normal,
        expire: Expiry::Default,
        image: NotifyIcon::None,
        category: None,
        desktop_entry: None,
        transient: false,
        resident: false,
        suppress_sound: false,
        received: SystemTime::now(),
        mascot: None,
    }
}

fn crit(id: u32) -> Notification {
    Notification { urgency: Urgency::Critical, ..n(id) }
}

fn transient(id: u32) -> Notification {
    Notification { transient: true, ..n(id) }
}

const S: fn(u64) -> Duration = Duration::from_secs;

fn ids<'a>(it: impl Iterator<Item = &'a Notification>) -> Vec<u32> {
    it.map(|n| n.id).collect()
}

#[test]
fn config_defaults_and_validation() {
    let c = NotificationsConfig::default();
    assert_eq!((c.popup_timeout_ms, c.critical_timeout_ms, c.max_popups, c.history_limit), (5000, None, 4, 100));
    assert!(c.enabled && !c.dnd && c.apps.is_empty());

    let mut bad = NotificationsConfig {
        enabled: true,
        dnd: true,
        popup_timeout_ms: 1,
        critical_timeout_ms: Some(u64::MAX),
        max_popups: 0,
        history_limit: 1_000_000,
        apps: Default::default(),
    };
    let warnings = bad.validate();
    assert_eq!(warnings.len(), 4, "{warnings:?}");
    assert!(warnings.iter().all(|w| w.starts_with("notifications.")), "{warnings:?}");
    assert_eq!(
        (bad.popup_timeout_ms, bad.critical_timeout_ms, bad.max_popups, bad.history_limit),
        (1000, Some(3_600_000), 1, 500)
    );
    let mut high = NotificationsConfig { popup_timeout_ms: 90_000, ..Default::default() };
    assert_eq!(high.validate().len(), 1);
    assert_eq!(high.popup_timeout_ms, 60_000);
    // 0 geçerli: geçmiş tutulmaz.
    let mut zero = NotificationsConfig { history_limit: 0, ..Default::default() };
    assert!(zero.validate().is_empty());
    let mut ok = NotificationsConfig::default();
    assert!(ok.validate().is_empty());
}

#[test]
fn config_from_toml_schema() {
    let toml_text = r#"
        enabled = false
        dnd = true
        popup_timeout_ms = 7000
        critical_timeout_ms = 20000
        max_popups = 3
        history_limit = 50

        [apps."Spotify"]
        popups = false

        [apps."org.example.Chat"]
        history = false
    "#;
    let c: NotificationsConfig = toml::from_str(toml_text).unwrap();
    assert!(!c.enabled && c.dnd);
    assert_eq!((c.popup_timeout_ms, c.critical_timeout_ms, c.max_popups, c.history_limit), (7000, Some(20_000), 3, 50));
    assert_eq!(c.rule("Spotify"), AppRule { popups: false, history: true });
    assert_eq!(c.rule("org.example.Chat"), AppRule { popups: true, history: false });
    // Büyük/küçük harf duyarlı, bilinmeyen → varsayılan.
    assert_eq!(c.rule("spotify"), AppRule::default());
    assert_eq!(AppRule::default(), AppRule { popups: true, history: true });
    // Boş tablo varsayılanlar.
    let d: NotificationsConfig = toml::from_str("").unwrap();
    assert_eq!(d, NotificationsConfig::default());
}

#[test]
fn app_rules_validation_limits() {
    let mut c = NotificationsConfig::default();
    for i in 0..300 {
        c.apps.insert(format!("app{i:03}"), AppRule::default());
    }
    c.apps.insert(String::new(), AppRule::default());
    c.apps.insert("x".repeat(129), AppRule::default());
    c.apps.insert("ş".repeat(128), AppRule::default()); // tam sınırda (karakter, bayt değil)
    let warnings = c.validate();
    assert_eq!(c.apps.len(), 256);
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(warnings.iter().all(|w| w.starts_with("notifications.apps")), "{warnings:?}");
    assert!(c.apps.contains_key("app000"));
}

#[test]
fn app_rules_apply() {
    let mut apps = std::collections::BTreeMap::new();
    apps.insert("sessiz".to_owned(), AppRule { popups: false, history: true });
    apps.insert("anlık".to_owned(), AppRule { popups: true, history: false });
    apps.insert("hiç".to_owned(), AppRule { popups: false, history: false });
    let mut c = NotificationCenter::new(NotificationsConfig { apps, ..Default::default() });
    let t0 = Instant::now();
    let with_app = |id, app: &str| Notification { app_name: app.into(), ..n(id) };

    // popups = false: critical bile popup olmaz, merkeze düşer.
    c.on_event(Ev::Added(Notification { urgency: Urgency::Critical, ..with_app(1, "sessiz") }), t0);
    assert!(!c.is_popup(1));
    assert_eq!(ids(c.history()), vec![1]);
    // history = false: yalnızca popup; bitince kapanır.
    c.on_event(Ev::Added(with_app(2, "anlık")), t0);
    assert!(c.is_popup(2));
    assert_eq!(ids(c.history()), vec![1]);
    assert_eq!(c.unread_count(), 1);
    assert_eq!(c.expire(t0 + S(5)), vec![2]);
    assert_eq!(c.take_closed(), vec![(2, CloseReason::Expired)]);
    // İkisi de kapalı: hemen kapanır.
    c.on_event(Ev::Added(with_app(3, "hiç")), t0);
    assert!(c.get(3).is_none());
    assert_eq!(c.take_closed(), vec![(3, CloseReason::Expired)]);
    // Eşleşme büyük/küçük harf duyarlı.
    c.on_event(Ev::Added(with_app(4, "Sessiz")), t0);
    assert!(c.is_popup(4));
}

#[test]
fn history_limit_zero_keeps_popup_until_it_ends() {
    let mut c = NotificationCenter::new(NotificationsConfig { history_limit: 0, ..Default::default() });
    let t0 = Instant::now();
    c.on_event(Ev::Added(n(1)), t0);
    assert!(c.is_popup(1), "gösterilen popup geçmiş sınırıyla atılmaz");
    assert!(c.take_closed().is_empty());
    assert_eq!(c.expire(t0 + S(5)), vec![1]);
    assert!(c.get(1).is_none());
    assert_eq!(c.take_closed(), vec![(1, CloseReason::Expired)]);

    c.on_event(Ev::Added(n(2)), t0);
    assert!(c.dismiss_popup(2));
    assert_eq!(c.take_closed(), vec![(2, CloseReason::Expired)]);

    // DND açılınca gizlenen popup da kapanır.
    c.on_event(Ev::Added(n(3)), t0);
    c.set_dnd(true);
    assert!(c.get(3).is_none());
    assert_eq!(c.take_closed(), vec![(3, CloseReason::Expired)]);
}

#[test]
fn popup_durations() {
    let c = NotificationsConfig::default();
    assert_eq!(c.popup_duration(&n(1)), Some(S(5)));
    assert_eq!(c.popup_duration(&Notification { urgency: Urgency::Low, ..n(1) }), Some(S(5)));
    assert_eq!(c.popup_duration(&Notification { expire: Expiry::Never, ..n(1) }), None);
    assert_eq!(c.popup_duration(&Notification { expire: Expiry::After(S(2)), ..n(1) }), Some(S(2)));
    assert_eq!(c.popup_duration(&Notification { expire: Expiry::After(S(999_999)), ..n(1) }), Some(S(3600)));
    // Critical: istemci süre verse bile kapanmaz.
    assert_eq!(c.popup_duration(&Notification { expire: Expiry::After(S(2)), ..crit(1) }), None);
    let with_crit = NotificationsConfig { critical_timeout_ms: Some(30_000), ..c };
    assert_eq!(with_crit.popup_duration(&crit(1)), Some(S(30)));
}

#[test]
fn popup_expires_but_stays_in_history() {
    let mut c = NotificationCenter::default();
    let t0 = Instant::now();
    assert!(c.on_event(Ev::Added(n(1)), t0));
    assert_eq!(c.next_deadline(), Some(t0 + S(5)));
    assert!(c.expire(t0 + S(4)).is_empty());
    assert_eq!(c.expire(t0 + S(5)), vec![1]);
    assert_eq!(c.popups().count(), 0);
    assert_eq!(ids(c.history()), vec![1]);
    assert_eq!(c.next_deadline(), None);
    assert!(c.take_closed().is_empty());
}

#[test]
fn critical_never_expires() {
    let mut c = NotificationCenter::default();
    let t0 = Instant::now();
    c.on_event(Ev::Added(crit(1)), t0);
    assert_eq!(c.next_deadline(), None);
    assert!(c.expire(t0 + S(1_000_000)).is_empty());
    assert!(c.is_popup(1));
}

#[test]
fn transient_skips_history_and_closes_on_expiry() {
    let mut c = NotificationCenter::default();
    let t0 = Instant::now();
    c.on_event(Ev::Added(transient(7)), t0);
    assert_eq!(c.history().count(), 0);
    assert_eq!(c.unread_count(), 0);
    assert_eq!(ids(c.popups()), vec![7]);
    assert_eq!(c.expire(t0 + S(5)), vec![7]);
    assert!(c.get(7).is_none());
    assert_eq!(c.take_closed(), vec![(7, CloseReason::Expired)]);
    assert!(c.take_closed().is_empty());
}

#[test]
fn hover_pauses_and_resumes_with_grace() {
    let mut c = NotificationCenter::default();
    let t0 = Instant::now();
    c.on_event(Ev::Added(n(1)), t0);
    assert!(c.set_hovered(1, true, t0 + S(4)));
    assert!(!c.set_hovered(1, true, t0 + S(4)));
    assert_eq!(c.next_deadline(), None);
    assert!(c.expire(t0 + S(100)).is_empty(), "fare üstündeyken süre dolmaz");
    // 1 s kalmıştı; en az 1 s grace → 101'de dolar.
    assert!(c.set_hovered(1, false, t0 + S(100)));
    assert_eq!(c.next_deadline(), Some(t0 + S(101)));
    assert_eq!(c.expire(t0 + S(101)), vec![1]);

    // Uzun kalan süre korunur.
    c.on_event(Ev::Added(Notification { expire: Expiry::After(S(60)), ..n(2) }), t0);
    c.set_hovered(2, true, t0 + S(10));
    c.set_hovered(2, false, t0 + S(500));
    assert!(c.expire(t0 + S(549)).is_empty());
    assert_eq!(c.expire(t0 + S(550)), vec![2]);
    assert!(!c.set_hovered(999, true, t0));

    // Hover'dayken replaces_id ile gelen güncelleme duraklatılmış başlar.
    c.on_event(Ev::Added(n(3)), t0);
    c.set_hovered(3, true, t0);
    c.on_event(Ev::Added(Notification { summary: "yeni".into(), ..n(3) }), t0 + S(1));
    assert!(c.expire(t0 + S(1000)).is_empty());
}

#[test]
fn dnd_suppresses_non_critical_but_records_history() {
    let mut c = NotificationCenter::default();
    let t0 = Instant::now();
    c.on_event(Ev::Added(n(1)), t0);
    c.on_event(Ev::Added(crit(2)), t0);
    c.on_event(Ev::Added(transient(3)), t0);
    c.set_dnd(true);
    assert!(c.dnd());
    assert_eq!(ids(c.popups()), vec![2]);
    assert_eq!(c.take_closed(), vec![(3, CloseReason::Dismissed)]);

    c.on_event(Ev::Added(n(4)), t0);
    assert!(!c.is_popup(4));
    assert_eq!(ids(c.history()), vec![4, 2, 1]);
    c.on_event(Ev::Added(crit(5)), t0);
    assert!(c.is_popup(5));
    // DND'de transient: ne popup ne geçmiş, hemen kapanır.
    c.on_event(Ev::Added(transient(6)), t0);
    assert!(c.get(6).is_none());
    assert_eq!(c.take_closed(), vec![(6, CloseReason::Expired)]);

    c.set_dnd(false);
    c.on_event(Ev::Added(n(7)), t0);
    assert!(c.is_popup(7));

    // Ayardan DND.
    let d = NotificationCenter::new(NotificationsConfig { dnd: true, ..Default::default() });
    assert!(d.dnd());
}

#[test]
fn replaces_id_updates_in_place() {
    let mut c = NotificationCenter::default();
    let t0 = Instant::now();
    c.on_event(Ev::Added(n(1)), t0);
    c.on_event(Ev::Added(n(2)), t0);
    c.expire(t0 + S(5));
    c.mark_all_read();

    c.on_event(Ev::Added(Notification { summary: "güncel".into(), ..n(1) }), t0 + S(10));
    assert_eq!(ids(c.history()), vec![1, 2]);
    assert_eq!(c.get(1).unwrap().summary, "güncel");
    assert!(c.is_popup(1));
    assert_eq!(c.next_deadline(), Some(t0 + S(15)));
    assert_eq!(c.unread_count(), 1);
}

#[test]
fn history_and_popup_limits() {
    let mut c = NotificationCenter::new(NotificationsConfig {
        history_limit: 3,
        max_popups: 2,
        ..NotificationsConfig::default()
    });
    let t0 = Instant::now();
    for id in 1..=4 {
        c.on_event(Ev::Added(n(id)), t0);
    }
    assert_eq!(ids(c.popups()), vec![4, 3]);
    assert_eq!(ids(c.history()), vec![4, 3, 2]);
    assert_eq!(c.take_closed(), vec![(1, CloseReason::Expired)]);

    // Popup limiti critical olmayanı önce gizler.
    let mut c = NotificationCenter::new(NotificationsConfig { max_popups: 1, ..NotificationsConfig::default() });
    c.on_event(Ev::Added(crit(1)), t0);
    c.on_event(Ev::Added(n(2)), t0);
    assert_eq!(ids(c.popups()), vec![1]);
    assert!(c.get(2).is_some());

    // Ayar canlı değişince sınırlar hemen uygulanır.
    let mut c = NotificationCenter::default();
    for id in 1..=5 {
        c.on_event(Ev::Added(n(id)), t0);
    }
    c.take_closed();
    c.set_config(NotificationsConfig { history_limit: 2, ..NotificationsConfig::default() });
    // Gösterilen popup'lar (5,4,3,2) atılmaz; yalnızca popup'ı bitmiş 1 kapanır.
    assert_eq!(ids(c.history()), vec![5, 4, 3, 2]);
    assert_eq!(c.take_closed(), vec![(1, CloseReason::Expired)]);
    // Popup'lar bitince sınır uygulanır.
    c.expire(t0 + S(5));
    assert_eq!(ids(c.history()), vec![5, 4]);
    assert_eq!(c.take_closed().len(), 2);
}

#[test]
fn closed_event_dismiss_remove_clear() {
    let mut c = NotificationCenter::default();
    let t0 = Instant::now();
    for id in 1..=3 {
        c.on_event(Ev::Added(n(id)), t0);
    }
    assert!(c.on_event(Ev::Closed { id: 2, reason: CloseReason::ClosedByCall }, t0));
    assert!(!c.on_event(Ev::Closed { id: 2, reason: CloseReason::ClosedByCall }, t0));
    assert!(c.dismiss_popup(1));
    assert!(!c.dismiss_popup(1));
    assert!(c.get(1).is_some());
    assert_eq!(c.remove(1), Some(1));
    assert_eq!(c.remove(1), None);
    assert_eq!(c.clear_all(), vec![3]);
    assert_eq!(c.history().count(), 0);
}

#[test]
fn unread_and_groups() {
    let mut c = NotificationCenter::default();
    let t0 = Instant::now();
    c.on_event(Ev::Added(Notification { app_name: "Chat".into(), ..n(1) }), t0);
    c.on_event(Ev::Added(Notification { app_name: "Mail".into(), ..n(2) }), t0);
    c.on_event(Ev::Added(Notification { app_name: "chat".into(), ..n(3) }), t0);
    c.on_event(Ev::Added(Notification { app_name: "X".into(), desktop_entry: Some("Mail".into()), ..n(4) }), t0);
    assert_eq!(c.unread_count(), 4);
    c.mark_all_read();
    assert_eq!(c.unread_count(), 0);
    c.on_event(Ev::Added(n(5)), t0);
    assert_eq!(c.unread_count(), 1);

    let shape: Vec<(String, Vec<u32>)> =
        c.history_groups().iter().map(|g| (g.key.clone(), g.notifications.iter().map(|n| n.id).collect())).collect();
    assert_eq!(shape, vec![("app".into(), vec![5]), ("mail".into(), vec![4, 2]), ("chat".into(), vec![3, 1])]);
}

#[test]
fn image_ready_updates_in_place() {
    let mut c = NotificationCenter::default();
    let t0 = Instant::now();
    c.on_event(Ev::Added(n(1)), t0);
    c.on_event(Ev::Added(n(2)), t0);
    c.mark_all_read();
    let deadline = c.next_deadline();
    let pix = NotifyIcon::Pixmap { width: 2, height: 1, rgba: vec![0u8; 8].into() };

    assert!(c.on_event(Ev::ImageReady { id: 1, app_icon: None, image: Some(pix.clone()) }, t0 + S(1)));
    assert_eq!(c.get(1).unwrap().image, pix);
    assert_eq!(c.get(1).unwrap().app_icon, NotifyIcon::None);
    // Sıra, süre ve okunmamış değişmez.
    assert_eq!(ids(c.history()), vec![2, 1]);
    assert_eq!(c.next_deadline(), deadline);
    assert_eq!(c.unread_count(), 0);
    assert_eq!(c.image_bytes(), 8);

    assert!(!c.on_event(Ev::ImageReady { id: 99, app_icon: Some(pix.clone()), image: None }, t0));
    assert!(!c.on_event(Ev::ImageReady { id: 2, app_icon: None, image: None }, t0));
}

#[test]
fn hover_is_exclusive() {
    let mut c = NotificationCenter::default();
    let t0 = Instant::now();
    c.on_event(Ev::Added(n(1)), t0);
    c.on_event(Ev::Added(n(2)), t0);
    assert!(c.set_hovered(1, true, t0 + S(1)));
    assert_eq!(c.next_deadline(), Some(t0 + S(5)), "yalnızca 2 sayıyor");
    // Karttan karta geçiş tek olay: 1 bırakılır (en az grace ile devam), 2 durur.
    assert!(c.set_hovered(2, true, t0 + S(2)));
    assert_eq!(c.next_deadline(), Some(t0 + S(6)), "1: kalan 4 s, t0+2'den");
    assert!(c.expire(t0 + S(100)).contains(&1));
    assert!(c.is_popup(2), "üzerindeki popup dolmaz");
    assert!(c.set_hovered(2, false, t0 + S(100)));
    assert!(!c.set_hovered(99, true, t0));
}

// --- dahili (shell'in kendi) bildirimleri -------------------------------------------------

use nuxut_shell::services::notifications::internal::{INTERNAL_APP_NAME, is_internal_id};
use nuxut_shell::services::notifications::{InternalAction, InternalNotice, body_text};

fn undo_notice() -> InternalNotice {
    InternalNotice::silent(
        "Ayarların yeni sürüme taşındı",
        "Çalışma alanı sayısı",
        vec![InternalAction::new("Geri al", "config undo-migration")],
    )
}

#[test]
fn silent_notice_has_no_popup_but_is_unread_in_center() {
    let t0 = Instant::now();
    let mut c = NotificationCenter::default();
    let id = c.add_internal(undo_notice(), t0);
    assert!(is_internal_id(id));
    assert_eq!(c.popups().count(), 0, "sessiz bildirim popup üretmez");
    assert!(!c.is_popup(id));
    assert_eq!(c.next_deadline(), None, "popup timer'ı kurulmaz");
    let h: Vec<_> = c.history().collect();
    assert_eq!(h.len(), 1, "merkezde görünür");
    assert_eq!(h[0].app_name, INTERNAL_APP_NAME);
    assert_eq!(h[0].summary, "Ayarların yeni sürüme taşındı");
    assert_eq!(body_text(&h[0].body), "Çalışma alanı sayısı");
    assert!(h[0].suppress_sound, "ses yok");
    assert_eq!(h[0].actions.len(), 1);
    assert_eq!(h[0].actions[0].label, "Geri al");
    assert_eq!(c.unread_count(), 1, "okunmamış");
    assert!(c.take_closed().is_empty(), "D-Bus'a kapanış gitmez");
    c.mark_all_read();
    assert_eq!(c.unread_count(), 0);
}

#[test]
fn silent_notice_survives_expiry_and_dnd() {
    let t0 = Instant::now();
    let mut c = NotificationCenter::default();
    c.set_dnd(true);
    let id = c.add_internal(undo_notice(), t0);
    c.expire(t0 + S(3600));
    assert!(c.get(id).is_some());
    assert_eq!(c.popups().count(), 0);
}

#[test]
fn popup_notice_follows_normal_popup_rules() {
    let t0 = Instant::now();
    let mut c = NotificationCenter::default();
    let id = c.add_internal(InternalNotice { popup: true, ..undo_notice() }, t0);
    assert!(c.is_popup(id));
    assert!(!c.get(id).unwrap().suppress_sound);
    assert!(c.next_deadline().is_some());
    c.expire(t0 + S(60));
    assert!(!c.is_popup(id) && c.get(id).is_some(), "süre dolunca merkezde kalır");
    // DND popup'ı bastırır, merkeze yine düşer.
    c.set_dnd(true);
    let id2 = c.add_internal(InternalNotice { popup: true, ..undo_notice() }, t0);
    assert!(!c.is_popup(id2) && c.get(id2).is_some());
}

#[test]
fn action_routes_to_command() {
    let t0 = Instant::now();
    let mut c = NotificationCenter::default();
    let id = c.add_internal(undo_notice(), t0);
    let key = c.get(id).unwrap().actions[0].key.clone();
    assert_eq!(c.internal_command(id, &key), Some("config undo-migration"));
    assert_eq!(c.internal_command(id, "yok"), None);
    // D-Bus bildirimi asla iç komuta yönlenmez (aynı anahtarla bile).
    c.on_event(Ev::Added(n(1)), t0);
    assert_eq!(c.internal_command(1, &key), None);
    // Kaldırılan bildirimin eylemi artık yürümez.
    c.remove(id);
    assert_eq!(c.internal_command(id, &key), None);
}

#[test]
fn internal_notice_is_bounded() {
    let t0 = Instant::now();
    let mut c = NotificationCenter::default();
    let mut actions: Vec<InternalAction> = (0..20).map(|i| InternalAction::new(format!("e{i}"), "ping")).collect();
    actions.insert(0, InternalAction::new("boş", "  "));
    actions.insert(1, InternalAction::new("uzun", "x".repeat(10_000)));
    let notice = InternalNotice::silent("a\nb".repeat(1000), "g".repeat(100_000), actions);
    let id = c.add_internal(notice, t0);
    let n = c.get(id).unwrap();
    assert!(!n.summary.contains('\n'));
    assert!(n.summary.chars().count() <= 256);
    assert!(body_text(&n.body).len() <= 4096);
    assert_eq!(n.actions.len(), 8, "en çok 8 düğme; boş ve uzun komut atılır");
    assert_eq!(n.actions[0].label, "e0");
    // Kimlikler benzersiz; geçmiş sınırı dahili bildirimlere de uygulanır.
    let mut c = NotificationCenter::new(NotificationsConfig { history_limit: 3, ..Default::default() });
    let ids: Vec<u32> = (0..5).map(|_| c.add_internal(undo_notice(), t0)).collect();
    let mut uniq = ids.clone();
    uniq.dedup();
    assert_eq!(uniq.len(), 5);
    assert_eq!(c.history().count(), 3);
    assert!(c.take_closed().is_empty(), "atılan dahili bildirim D-Bus'a kapanış olarak gitmez");
}

#[test]
fn internal_notice_image_is_kept_when_bounded_and_dropped_otherwise() {
    let t0 = Instant::now();
    let mut c = NotificationCenter::default();
    let pixmap = |w: u32, h: u32, len: usize| NotifyIcon::Pixmap { width: w, height: h, rgba: vec![0u8; len].into() };
    // Önizleme: sınır içinde, bayt sayısı tutarlı → modelde.
    let ok = pixmap(256, 144, 256 * 144 * 4);
    let id = c.add_internal(InternalNotice::popup("Kaydedildi", "", Vec::new()).with_image(ok.clone()), t0);
    assert_eq!(c.get(id).unwrap().image, ok);
    // Tema adı serbest.
    let name = NotifyIcon::Name("camera-photo".into());
    let id = c.add_internal(InternalNotice::silent("a", "", Vec::new()).with_image(name.clone()), t0);
    assert_eq!(c.get(id).unwrap().image, name);
    // Sınır dışı kenar, boş, tutarsız bayt → görselsiz.
    for bad in [pixmap(257, 10, 257 * 10 * 4), pixmap(0, 10, 0), pixmap(10, 10, 399)] {
        let id = c.add_internal(InternalNotice::popup("b", "", Vec::new()).with_image(bad), t0);
        assert!(c.get(id).unwrap().image.is_none());
    }
    // Varsayılan: görsel yok.
    assert!(c.add_internal(undo_notice(), t0) > 0);
    assert!(undo_notice().image.is_none());
}

// --- maskot yalnızca dahili bildirimlerde ----------------------------------------------------

use nuxut_shell::services::notifications::NoticeMood;

/// `with_mascot` modele ruh hâlini taşır (piksel değil); verilen içerik görseli atılır (yuva
/// maskotun).
#[test]
fn internal_notice_carries_mascot() {
    let t0 = Instant::now();
    let mut c = NotificationCenter::default();
    let pix = NotifyIcon::Pixmap { width: 1, height: 1, rgba: vec![0u8; 4].into() };
    let id = c.add_internal(
        InternalNotice::popup("Nuxut güncellendi", "Sürüm 0.2.0", vec![]).with_image(pix).with_mascot(NoticeMood::Celebrate),
        t0,
    );
    let got = c.get(id).unwrap();
    assert_eq!(got.mascot, Some(NoticeMood::Celebrate));
    assert_eq!(got.image, NotifyIcon::None, "maskot görsel yuvasını alır");
    assert!(c.is_popup(id));
    // Maskotsuz dahili bildirim: alan boş.
    let plain = c.add_internal(undo_notice(), t0);
    assert_eq!(c.get(plain).unwrap().mascot, None);
}

/// Dış (D-Bus) olay maskot alanını ayarlayamaz: sunucu dışı bir yoldan dolu gelse bile model
/// siler; aynı kimlikle yeniden ekleme de maskot kazandırmaz.
#[test]
fn external_event_cannot_set_mascot() {
    let t0 = Instant::now();
    let mut c = NotificationCenter::default();
    let forged = Notification { app_name: INTERNAL_APP_NAME.into(), mascot: Some(NoticeMood::Celebrate), ..n(5) };
    c.on_event(Ev::Added(forged), t0);
    assert_eq!(c.get(5).unwrap().mascot, None);
    assert!(c.history().all(|x| x.mascot.is_none()));
}
