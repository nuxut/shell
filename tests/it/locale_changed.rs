//! Dil değişimi (ADR-0010 `LocaleChanged`, `App::locale_changed` → `Screen::relocalize`):
//! kurulmuş widget metnini kendiliğinden değiştirmez; zorla yeniden kurulum yeni katalogdan
//! alır. Kendi test ikilisi: süreç geneli katalog tr ↔ en değiştirilir, kütüphanenin
//! (Türkçeye sabitlenmiş) birim testlerini etkilemez. Bu dosyada tek test var (aynı süreçte
//! paralel testler de katalogu paylaşırdı; şema denetimi aynı testin sonunda).

use std::sync::Arc;
use std::time::Instant;

use nuxut::Rect;
use nuxut::i18n::{Catalog, LanguagePrefs};
use nuxut_shell::config::BarSection;
use nuxut_shell::widgets::notifications_panel::NotificationsPanel;
use nuxut_shell::widgets::{self, BarWidget, PanelWidget, WidgetCx, WidgetKind, WidgetSettings};

fn set_language(tag: &str) {
    nuxut::i18n::set_current(Arc::new(Catalog::load(&[nuxut_shell::i18n::source()], &LanguagePrefs::fixed(&[tag]))));
}

fn title(w: &dyn PanelWidget) -> &str {
    w.as_any().downcast_ref::<NotificationsPanel>().expect("bildirim merkezi").title()
}

fn kinds_panel(ws: &[Box<dyn PanelWidget>]) -> Vec<WidgetKind> {
    ws.iter().map(|w| w.kind()).collect()
}

fn kinds_bar(bar: &[(BarSection, Box<dyn BarWidget>)]) -> Vec<(BarSection, WidgetKind)> {
    bar.iter().map(|(s, w)| (*s, w.kind())).collect()
}

#[test]
fn relocalize_rebuilds_widget_text_from_new_catalog() {
    set_language("tr");
    let theme = nuxut::Theme::default();
    let settings = WidgetSettings::default();
    let cx = WidgetCx {
        theme: &theme,
        settings: &settings,
        vertical: false,
        item: WidgetCx::item_for(&theme, 40.0),
        now: Instant::now(),
        bounds: Rect::new(0.0, 0.0, 1920.0, 1080.0),
    };

    // Çekmece: bildirim merkezi (başlık t!() ile) ve metinsiz bir kart; sıra korunmalı.
    let mut panel: Vec<Box<dyn PanelWidget>> = [WidgetKind::Volume, WidgetKind::Notifications, WidgetKind::Session]
        .into_iter()
        .filter_map(|k| widgets::create_panel(k, &cx))
        .collect();
    let panel_kinds = kinds_panel(&panel);
    assert_eq!(title(panel[1].as_ref()), "Bildirimler");

    set_language("en");
    assert_eq!(title(panel[1].as_ref()), "Bildirimler", "kurulmuş widget kendiliğinden değişmez: zorla yeniden kurulum gerekir");
    widgets::recreate_panel(&mut panel, &cx);
    assert_eq!(title(panel[1].as_ref()), "Notifications");
    assert_eq!(kinds_panel(&panel), panel_kinds, "türler ve sıra korunur");

    set_language("tr");
    widgets::recreate_panel(&mut panel, &cx);
    assert_eq!(title(panel[1].as_ref()), "Bildirimler", "geri dönüş de çalışır");

    // Bar: bölüm ve sıra korunur (açık yaprak vurgusu ve isabet bunlara bağlı).
    let mut bar: Vec<(BarSection, Box<dyn BarWidget>)> = [
        (BarSection::Start, WidgetKind::Workspaces),
        (BarSection::Center, WidgetKind::Clock),
        (BarSection::End, WidgetKind::Notifications),
        (BarSection::End, WidgetKind::Session),
    ]
    .into_iter()
    .filter_map(|(s, k)| widgets::create(k, &cx).map(|w| (s, w)))
    .collect();
    let bar_kinds = kinds_bar(&bar);
    assert_eq!(bar_kinds.len(), 4);
    set_language("en");
    widgets::recreate_bar(&mut bar, &cx);
    assert_eq!(kinds_bar(&bar), bar_kinds);

    schema_labels_follow_catalog_language();
}

/// `nuxut-shell schema` başlıkları, açıklamaları ve seçenek adları etkin dilden gelir: İngilizce
/// katalogla üretilen şemada Türkçeye özgü harf kalmaz. (Aynı test içinde: katalog süreç geneli.)
fn schema_labels_follow_catalog_language() {
    set_language("en");
    let text = nuxut_shell::schema::generate().expect("şema üretilir");
    set_language("tr");
    let turkish: Vec<&str> = text.lines().filter(|l| l.contains(['ç', 'ğ', 'ı', 'İ', 'ö', 'ş', 'ü', 'Ç', 'Ğ', 'Ö', 'Ş', 'Ü'])).collect();
    assert!(turkish.is_empty(), "İngilizce şemada Türkçe metin: {turkish:#?}");
    let tr = nuxut_shell::schema::generate().expect("şema üretilir");
    assert_ne!(tr, text, "dil değişince şema yeniden üretilir, metinler değişir");
}
