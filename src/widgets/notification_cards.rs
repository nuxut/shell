//! Bildirim kartları için ortak parçalar (merkez paneli ve popup yığını):
//!
//! - [`CardSet`]: modeldeki bildirimlerle eşlenen `NotificationCard`'lar. Kartlar kimlikle
//!   korunur; yalnızca içerik değişince (replaces_id, gelen görsel) yeniden kurulur. Kart kurmak
//!   doku yüklemesi gerektirdiği için [`CardSet::prepare`] çizimde (Painter varken) çağrılır.
//! - Saf yardımcılar: göreli zaman, kategori görünümü, içerik eşleme, kart olayı → komut.
//!
//! Görseller sunucuda çözülmüş küçük düz RGBA'dır (`NotifyIcon::Pixmap`); burada önçarpılıp
//! (`premultiplied`) bir kez GPU'ya yüklenir. Doku kartla yaşar: kart kalkınca düşer.
//!
//! Maskot (yalnız dahili bildirim, `Notification::mascot`; yerleşim
//! `docs/design/mascot-placement.md`): framework kartının maskot yuvası
//! (`NotificationCard::with_mascot`, fw 1c9174a) — metnin solunda, karosuz,
//! `NOTIFICATION_MASCOT_SIZE` (64) kenarlı; kartın kaydırma/kapanma solmasıyla birlikte solar.
//! Kart maskotu `[appearance] mascot`'tan bağımsız taşır; gizliyken yuvayı kart kendisi kapatır
//! (metin sola yaslanır, kart kısalır). Tema değişiminde kart yeniden kurulmaz: `set_theme` ölçüyü
//! düşürür, sonraki karede `measure` (`prepare`) + `set_rect` (`draw`) yeni yüksekliği dizer.

use std::hash::{Hash, Hasher};
use std::time::{Instant, SystemTime};

use nuxut::render::{ImageData, Texture, TextureOptions};
use nuxut::ui::{
    BodySpan as CardSpan, Icon, IconName, Mood, NotificationCard, NotificationContent, NotificationEvent,
    NotificationStack, StackSlot,
};
use nuxut::i18n::t;
use nuxut::{Button, Color, CursorShape, Painter, Point, PointerEvent, PointerKind, Rect, TextSystem, Theme};

use super::NotifyCommand;
use crate::services::notifications::{NoticeMood, Notification, NotifyIcon, Urgency, body_text};

/// Takvim hatırlatıcısı kategorisi (takvim servisi sözleşmesi).
pub const CATEGORY_CALENDAR: &str = "x-nuxut.calendar.reminder";
/// Hesap uyarıları kategorisi.
pub const CATEGORY_ACCOUNTS: &str = "x-nuxut.accounts";

/// "şimdi", "5 dk", "3 sa", "dün", "4 gün" — kart başlığındaki göreli zaman.
pub fn relative_time(received: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(received).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..60 => t!("shell-notifications-time-now"),
        60..3600 => t!("shell-notifications-time-minutes", count = secs / 60),
        3600..86_400 => t!("shell-notifications-time-hours", count = secs / 3600),
        86_400..172_800 => t!("shell-notifications-time-yesterday"),
        _ => t!("shell-notifications-time-days", count = secs / 86_400),
    }
}

/// Uygulama simgesi yoksa (ya da tema adı bulunamadıysa) kategoriden seçilen semantik ikon.
pub fn fallback_icon(n: &Notification) -> IconName {
    match n.category.as_deref() {
        Some(CATEGORY_CALENDAR) => IconName::Event,
        Some(CATEGORY_ACCOUNTS) => IconName::AccountCircle,
        Some(c) if c.starts_with("email") => IconName::Mail,
        Some(c) if c.starts_with("im") => IconName::Chat,
        Some(c) if c.starts_with("device") => IconName::Desktop,
        Some(c) if c.starts_with("network") => IconName::Wifi,
        Some(c) if c.starts_with("transfer") => IconName::Download,
        _ => IconName::Notifications,
    }
}

/// Kartın sol kenarındaki vurgu şeridi (temadan): takvim ve critical birincil renk, hesap
/// uyarıları çizgi rengi; diğerleri şeritsiz.
pub fn accent(n: &Notification, theme: &Theme) -> Option<Color> {
    match n.category.as_deref() {
        Some(CATEGORY_CALENDAR) => Some(theme.colors.primary),
        Some(CATEGORY_ACCOUNTS) => Some(theme.colors.outline),
        _ if n.urgency == Urgency::Critical => Some(theme.colors.primary),
        _ => None,
    }
}

/// İçeriği değiştiren alanların özeti: değişince kart yeniden kurulur.
pub fn fingerprint(n: &Notification) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    n.app_name.hash(&mut h);
    n.summary.hash(&mut h);
    body_text(&n.body).hash(&mut h);
    for a in &n.actions {
        a.label.hash(&mut h);
    }
    n.category.hash(&mut h);
    n.urgency.hash(&mut h);
    n.received.hash(&mut h);
    n.mascot.hash(&mut h);
    for icon in [&n.app_icon, &n.image] {
        match icon {
            NotifyIcon::None => 0u8.hash(&mut h),
            NotifyIcon::Name(name) => name.hash(&mut h),
            // Pikseller paylaşımlı Arc: işaretçi ve boyut yeter (içerik taranmaz).
            NotifyIcon::Pixmap { width, height, rgba } => (width, height, rgba.as_ptr() as usize).hash(&mut h),
        }
    }
    h.finish()
}

/// Kartın gövdesi: model parçaları → kart parçaları (bağlantı = doğrulanmış normal URL).
pub fn card_body(n: &Notification) -> Vec<CardSpan> {
    n.body
        .iter()
        .map(|s| CardSpan {
            text: s.text.clone(),
            bold: s.style.bold,
            italic: s.style.italic,
            underline: s.style.underline,
            link: s.link.as_ref().map(|l| l.url.clone()),
        })
        .collect()
}

/// Kart içeriği. `app_icon`/`image`: yüklenmiş dokular (varsa).
pub fn card_content(
    n: &Notification,
    app_icon: Option<&Texture>,
    image: Option<&Texture>,
    now: SystemTime,
) -> NotificationContent {
    NotificationContent {
        app_name: n.app_name.clone(),
        app_icon: Some(match app_icon {
            Some(t) => Icon::image(t.clone()),
            None => Icon::named(fallback_icon(n)),
        }),
        time: relative_time(n.received, now),
        title: n.summary.clone(),
        body: card_body(n),
        image: image.cloned(),
        actions: n.actions.iter().take(nuxut::ui::MAX_NOTIFICATION_ACTIONS).map(|a| a.label.clone()).collect(),
    }
}

/// Kart olayını komuta çevirir. `swiped`: kapanış kaydırma hareketiyle mi (X düğmesi değil);
/// `popup`: popup yığınında mı (kaydırma orada yalnızca popup'ı gizler).
pub fn command_for(n: &Notification, event: NotificationEvent, swiped: bool, popup: bool) -> Option<NotifyCommand> {
    match event {
        NotificationEvent::Activated => {
            n.has_default_action.then(|| NotifyCommand::Invoke { id: n.id, key: "default".to_owned() })
        }
        NotificationEvent::Action(i) => {
            n.actions.get(i).map(|a| NotifyCommand::Invoke { id: n.id, key: a.key.clone() })
        }
        NotificationEvent::Link(url) => Some(NotifyCommand::OpenLink(url)),
        NotificationEvent::Dismissed if swiped && popup => Some(NotifyCommand::HidePopup(n.id)),
        NotificationEvent::Dismissed => Some(NotifyCommand::Dismiss(n.id)),
    }
}

/// Kartın maskot ruh hâli: yalnız maskot isteyen dahili bildirim (`Notification::mascot`; dış
/// bildirimde model alanı hep `None`). `[appearance] mascot` burada değil, kartta uygulanır.
pub fn card_mascot(n: &Notification) -> Option<Mood> {
    n.mascot.map(|mood| match mood {
        NoticeMood::Celebrate => Mood::Celebrate,
    })
}

/// Kartı kurar: içerik + (varsa) framework maskot yuvası.
fn build_card(theme: &Theme, n: &Notification, app_icon: Option<&Texture>, image: Option<&Texture>, now: SystemTime) -> NotificationCard {
    let card = NotificationCard::new(theme, card_content(n, app_icon, image, now));
    match card_mascot(n) {
        Some(mood) => card.with_mascot(theme, mood),
        None => card,
    }
}

/// Düz RGBA pikselleri önçarpıp doku olarak yükler. Pixmap değilse `None`.
pub fn upload_icon(painter: &mut Painter, icon: &NotifyIcon) -> Option<Texture> {
    let NotifyIcon::Pixmap { width, height, rgba } = icon else { return None };
    let image = nuxut_image::Image::from_straight(*width, *height, rgba.to_vec()).ok()?;
    let data = ImageData { width: *width, height: *height, pixels: image.into_pixels().into() };
    Some(painter.upload_texture(data, TextureOptions { mipmaps: false }))
}

// ---------------------------------------------------------------------------------------
// Kart kümesi

#[derive(Debug)]
pub struct CardEntry {
    pub notification: Notification,
    fingerprint: u64,
    pub card: Option<NotificationCard>,
    app_icon: Option<Texture>,
    image: Option<Texture>,
    /// Son ölçüm: (genişlik, yükseklik).
    measured: Option<(f32, f32)>,
    /// Modelden kalktı: kart uçarak çıkıyor; bitince silinir.
    pub leaving: bool,
    /// Çıkan kartın yığından çıkarıldığı andaki yeri: kart orada uçar, kalanlar ve kap aynı
    /// anda yerleşir (çıkışın bitmesini beklemez).
    frozen: Option<StackSlot>,
    /// Sol tuş basıldığı nokta (kaydırma mı tık mı ayırmak için).
    press: Option<Point>,
}

impl CardEntry {
    pub fn id(&self) -> u32 {
        self.notification.id
    }

    pub fn height(&self) -> f32 {
        self.measured.map_or(0.0, |(_, h)| h)
    }
}

/// Kimlikle korunan kart listesi (sıra modeldeki gibi). `NotificationStack` konumları canlandırır.
#[derive(Debug)]
pub struct CardSet {
    pub entries: Vec<CardEntry>,
    pub stack: NotificationStack<u32>,
    /// `true`: modelden kalkan kart uçarak çıkar (popup); `false`: hemen silinir (merkez).
    animate_removal: bool,
}

impl CardSet {
    pub fn new(theme: &Theme, animate_removal: bool) -> Self {
        Self { entries: Vec::new(), stack: NotificationStack::new(theme), animate_removal }
    }

    /// Modeldeki sırayla eşler. Görünür bir şey değiştiyse `true`.
    pub fn sync<'a>(&mut self, items: impl IntoIterator<Item = &'a Notification>, now: Instant) -> bool {
        let mut changed = false;
        let mut next: Vec<CardEntry> = Vec::new();
        for n in items {
            let fp = fingerprint(n);
            match self.entries.iter().position(|e| e.id() == n.id && !e.leaving) {
                Some(i) => {
                    let mut e = self.entries.remove(i);
                    if e.fingerprint != fp {
                        // İçerik değişti (replaces_id, gelen görsel): yeniden kurulacak.
                        e.fingerprint = fp;
                        e.card = None;
                        e.measured = None;
                        e.app_icon = None;
                        e.image = None;
                        changed = true;
                    }
                    e.notification = n.clone();
                    next.push(e);
                }
                None => {
                    next.push(CardEntry {
                        notification: n.clone(),
                        fingerprint: fp,
                        card: None,
                        app_icon: None,
                        image: None,
                        measured: None,
                        leaving: false,
                        frozen: None,
                        press: None,
                    });
                    changed = true;
                }
            }
        }
        // Kalkanlar: popup'ta yerinde (yaklaşık eski sırasında) uçarak çıkar, merkezde hemen
        // silinir.
        for (i, mut e) in std::mem::take(&mut self.entries).into_iter().enumerate() {
            if self.animate_removal && e.card.is_some() {
                if !e.leaving {
                    e.leaving = true;
                    changed = true;
                    if let Some(card) = &mut e.card {
                        card.dismiss(now);
                    }
                }
                next.insert(i.min(next.len()), e);
            } else {
                changed = true;
            }
        }
        self.entries = next;
        changed
    }

    /// Eksik kartları kurar (doku yükleyerek) ve `width`'te ölçer. Çizimde çağrılır.
    pub fn prepare(&mut self, painter: &mut Painter, theme: &Theme, width: f32) {
        let now = SystemTime::now();
        for e in &mut self.entries {
            if e.card.is_none() {
                // Dokular yalnızca yoksa yüklenir (içerik değişiminde `sync` onları da düşürür;
                // dil değişiminde kalır).
                if e.app_icon.is_none() {
                    e.app_icon = upload_icon(painter, &e.notification.app_icon);
                }
                if e.image.is_none() {
                    e.image = upload_icon(painter, &e.notification.image);
                }
                e.card = Some(build_card(theme, &e.notification, e.app_icon.as_ref(), e.image.as_ref(), now));
                e.measured = None;
            }
        }
        self.measure(painter.text_system(), theme, width);
    }

    /// Kurulu kartları `width`'te ölçer (ölçüsü düşmüş ya da genişliği değişmiş olanlar).
    fn measure(&mut self, text: &mut TextSystem, theme: &Theme, width: f32) {
        for e in &mut self.entries {
            if e.measured.is_none_or(|(w, _)| w != width)
                && let Some(card) = &mut e.card
            {
                let h = card.measure(text, theme, width);
                e.measured = Some((width, h));
            }
        }
    }

    /// Ölçülmüş, kalan kartlarla yığın düzeni; toplam (hedef) yükseklik. Çıkan kartlar düzene
    /// girmez: son yerlerinde dondurulur, boşluk ve toplam yükseklik kaldırılma kararıyla
    /// hemen kapanır.
    pub fn layout(&mut self, gap: f32, now: Instant) -> f32 {
        for e in &mut self.entries {
            if e.leaving && e.frozen.is_none() {
                e.frozen = self.stack.slot(&e.notification.id, now);
            }
        }
        let items: Vec<(u32, f32)> = self
            .entries
            .iter()
            .filter(|e| !e.leaving && e.measured.is_some())
            .map(|e| (e.id(), e.height()))
            .collect();
        self.stack.layout(&items, gap, now)
    }

    /// Kartın bu karedeki yığın yeri (çıkan kart için dondurulmuş yeri).
    pub fn slot(&self, e: &CardEntry, now: Instant) -> Option<StackSlot> {
        e.frozen.or_else(|| self.stack.slot(&e.id(), now))
    }


    /// Ekran okuyucu (ADR-0011): `clip` içinde görünen, çıkmayan kartlar `tree`'nin şu anki
    /// ebeveyni altına (framework `NotificationCard::access`; anahtar bildirim kimliği). Açılır
    /// kartlarda `live`: Orca odaksız kartı duyursun. Ekranda görünmeyen kart ağaçta yok.
    pub fn access(&self, tree: &mut nuxut::access::AccessTree, clip: Rect, live: bool) {
        let close = t!("shell-a11y-notification-close");
        for e in self.entries.iter().filter(|e| !e.leaving) {
            let Some(card) = &e.card else { continue };
            let seen = card.rect().intersect(&clip);
            if card.is_dismissed() || seen.width <= 0.0 || seen.height <= 0.0 {
                continue;
            }
            let node = card.access(tree, e.id(), &close);
            if live {
                // Orca canlı bölgede adı okur (açıklamayı değil): açılır kartta ad başlık + gövde
                // (Orca canlı testinde yalnız başlık duyuruluyordu, 2026-10-17). Sınırlı önizleme.
                let n = &e.notification;
                let body = nuxut::access::preview(&body_text(&n.body), 160);
                let title = if n.summary.is_empty() { n.app_name.as_str() } else { n.summary.as_str() };
                let spoken = if body.is_empty() { title.to_owned() } else { format!("{title}: {body}") };
                node.label(&spoken).live(nuxut::access::Live::Polite);
            }
        }
    }

    /// AT tıklaması bir kartın düğümüne mi (kart, eylem düğmesi, kapat): öyleyse işaretçiyle aynı
    /// komut. `parent`: [`Self::access`] çağrısındaki ebeveyn.
    pub fn access_action(
        &mut self,
        parent: nuxut::access::NodeId,
        request: &nuxut::access::AccessRequest,
        now: Instant,
        popup: bool,
    ) -> Option<NotifyCommand> {
        self.entries.iter_mut().filter(|e| !e.leaving).find_map(|e| {
            let id = nuxut::access::child_id(parent, e.notification.id);
            let card = e.card.as_mut()?;
            let event = card.access_action(id, *request, now)?;
            command_for(&e.notification, event, false, popup)
        })
    }
    /// Uçarak çıkışı biten kartları siler. Bir şey silindiyse `true`.
    pub fn reap(&mut self, now: Instant) -> bool {
        let before = self.entries.len();
        self.entries.retain_mut(|e| !(e.leaving && e.card.as_mut().is_none_or(|c| c.is_gone(now))));
        before != self.entries.len()
    }

    /// Dakikalık zaman metni güncellemesi. Değiştiyse `true`.
    pub fn refresh_time(&mut self, now: SystemTime) -> bool {
        let mut changed = false;
        for e in &mut self.entries {
            if let Some(card) = &mut e.card {
                let t = relative_time(e.notification.received, now);
                if card.content().time != t {
                    card.set_time(&t);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Dil değişti: kalan kartlar bir sonraki karede yeni katalogla yeniden kurulup ölçülür
    /// (göreli zaman, eylem etiketleri). Yığın yerleri kimlikle korunur; uçarak çıkan kartlar
    /// olduğu gibi biter.
    pub fn relocalize(&mut self) {
        for e in self.entries.iter_mut().filter(|e| !e.leaving) {
            e.card = None;
            e.measured = None;
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.stack.set_theme(theme, now);
        for e in &mut self.entries {
            // Ölçüler temaya bağlı (boşluk, yazı, `[appearance] mascot` → maskot yuvası açılır/
            // kapanır, kart uzar/kısalır): kart korunur, sonraki karede `prepare` yeniden ölçer,
            // `draw` yeni dikdörtgeni verir.
            if let Some(card) = &mut e.card {
                card.set_theme(theme, now);
            }
            e.measured = None;
        }
    }

    /// Kart işaretçi olayı → komut. `popup`: popup yığını mı. Döner: (yeniden çiz, komut).
    pub fn pointer(
        &mut self,
        theme: &Theme,
        event: &PointerEvent,
        now: Instant,
        popup: bool,
    ) -> (bool, Option<NotifyCommand>) {
        let mut repaint = false;
        let mut command = None;
        for e in &mut self.entries {
            let Some(card) = &mut e.card else { continue };
            if e.leaving {
                continue;
            }
            match event.kind {
                PointerKind::Press(Button::Left) if card.rect().contains(event.position) => {
                    e.press = Some(event.position);
                }
                _ => {}
            }
            let r = card.pointer(theme, event, now);
            repaint |= r.repaint;
            if let Some(ev) = r.event {
                // Basıldığı yerden belirgin yatay uzaklık: kaydırma (X düğmesi değil).
                let swiped = e.press.is_some_and(|p| (event.position.x - p.x).abs() > theme.space(2));
                command = command_for(&e.notification, ev, swiped, popup);
                e.press = None;
                break;
            }
            if matches!(event.kind, PointerKind::Release(_) | PointerKind::Leave) {
                e.press = None;
            }
        }
        (repaint, command)
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.entries.iter().filter_map(|e| e.card.as_ref()).find_map(|c| c.cursor(p))
    }

    /// `p`'nin üzerindeki (çıkmayan) kartın kimliği.
    pub fn card_at(&self, p: Point) -> Option<u32> {
        self.entries
            .iter()
            .filter(|e| !e.leaving)
            .find(|e| e.card.as_ref().is_some_and(|c| c.rect().contains(p)))
            .map(CardEntry::id)
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        self.stack.is_animating(now) | self.entries.iter().any(|e| e.leaving)
    }

    /// Kartları ve vurgu şeritlerini çizer; `place(girdi) -> Some((dikdörtgen, opaklık))` kartın
    /// bu karedeki yeri (`None` = çizilmez, ör. görünüm dışı).
    pub fn draw(&mut self, ui: &mut nuxut::ui::Ui, mut place: impl FnMut(&CardEntry) -> Option<(Rect, f32)>) {
        let theme = ui.theme;
        for e in &mut self.entries {
            let Some((rect, alpha)) = place(e) else { continue };
            let accent = accent(&e.notification, theme);
            let Some(card) = &mut e.card else { continue };
            card.set_rect(ui.painter.text_system(), theme, rect);
            ui.with_alpha(alpha, |ui| {
                card.draw(ui);
                if let Some(color) = accent
                    && !card.is_dismissed()
                {
                    let w = theme.space(1);
                    let strip = Rect::new(rect.x, rect.y + theme.space(4), w, (rect.height - theme.space(8)).max(0.0));
                    ui.painter.rect(strip, &nuxut::RectStyle::filled(ui.fade(color), nuxut::Corners::uniform(w * 0.5)));
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::services::notifications::markup::{BodyLink, BodySpan, TextStyle};
    use crate::services::notifications::{Expiry, NotifyAction};

    fn n(id: u32) -> Notification {
        Notification {
            id,
            app_name: "app".into(),
            app_icon: NotifyIcon::None,
            summary: "başlık".into(),
            body: vec![BodySpan {
                text: "bağ".into(),
                style: TextStyle { bold: true, ..TextStyle::default() },
                link: Some(BodyLink { url: "https://example.org/".into(), host: "example.org".into() }),
            }],
            actions: vec![NotifyAction { key: "ok".into(), label: "Tamam".into() }],
            has_default_action: true,
            urgency: Urgency::Normal,
            expire: Expiry::Default,
            image: NotifyIcon::None,
            category: None,
            desktop_entry: None,
            transient: false,
            resident: false,
            suppress_sound: false,
            received: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000),
            mascot: None,
        }
    }

    /// Yedek ikonların hepsi her iki ikon setinde (Material, Nerd) glife sahip ve adıyla geri
    /// çözülür (yoksa boş çizilir).
    #[test]
    fn fallback_symbols_resolve() {
        use nuxut::theme::IconSet;
        use nuxut::ui::glyph;
        for category in [None, Some("calendar"), Some("email"), Some("im"), Some("device"), Some("network"), Some("transfer")] {
            let mut x = n(1);
            x.category = category.map(str::to_owned);
            let icon = fallback_icon(&x);
            assert_eq!(icon.as_str().parse::<IconName>().ok(), Some(icon), "{category:?}");
            for set in [IconSet::Material, IconSet::Nerd] {
                assert!(!glyph(set, icon, false).is_empty(), "{category:?} {set:?}");
            }
        }
    }

    /// Painter'sız: kartı kurup ölçülmüş say (yalnızca yerleşim testi için).
    fn built(set: &mut CardSet, theme: &Theme, h: f32) {
        for e in &mut set.entries {
            if e.card.is_none() {
                e.card = Some(build_card(theme, &e.notification, None, None, SystemTime::UNIX_EPOCH));
                e.measured = Some((300.0, h));
            }
        }
    }

    #[test]
    fn leaving_card_frees_space_at_removal() {
        let theme = Theme::default();
        let now = Instant::now();
        let mut set = CardSet::new(&theme, true);
        set.sync([n(1), n(2)].iter(), now);
        built(&mut set, &theme, 100.0);
        assert_eq!(set.layout(10.0, now), 210.0);
        let before = set.slot(&set.entries[0], now).unwrap();

        // 1 kalktı: kart uçarak çıkar ama toplam yükseklik hemen yalnızca kalan kart kadardır.
        set.sync([n(2)].iter(), now);
        assert!(set.entries.iter().any(|e| e.id() == 1 && e.leaving));
        assert_eq!(set.layout(10.0, now), 100.0);
        let leaving = set.entries.iter().find(|e| e.id() == 1).unwrap();
        assert_eq!(set.slot(leaving, now), Some(before), "çıkan kart yerinde donar");

        // Son kart da kalktı: yığın yüksekliği 0 (kap kaldırılma kararıyla çöker).
        set.sync(std::iter::empty(), now);
        assert_eq!(set.layout(10.0, now), 0.0);
        assert_eq!(set.entries.len(), 2, "kartlar çıkış animasyonu bitene kadar çizilir");
    }

    #[test]
    fn relative_times() {
        crate::i18n::use_turkish_for_tests();
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let at = |s| relative_time(t, t + Duration::from_secs(s));
        assert_eq!(at(0), "şimdi");
        assert_eq!(at(59), "şimdi");
        assert_eq!(at(60), "1 dk");
        assert_eq!(at(3599), "59 dk");
        assert_eq!(at(7200), "2 sa");
        assert_eq!(at(90_000), "dün");
        assert_eq!(at(5 * 86_400), "5 gün");
        // Saat geri giderse "şimdi".
        assert_eq!(relative_time(t + Duration::from_secs(10), t), "şimdi");
    }

    #[test]
    fn category_look() {
        let theme = Theme::default();
        let mut cal = n(1);
        cal.category = Some(CATEGORY_CALENDAR.into());
        assert_eq!(fallback_icon(&cal), IconName::Event);
        assert_eq!(accent(&cal, &theme), Some(theme.colors.primary));
        let mut acc = n(2);
        acc.category = Some(CATEGORY_ACCOUNTS.into());
        assert_eq!(fallback_icon(&acc), IconName::AccountCircle);
        assert_eq!(accent(&acc, &theme), Some(theme.colors.outline));
        let crit = Notification { urgency: Urgency::Critical, ..n(3) };
        assert_eq!(accent(&crit, &theme), Some(theme.colors.primary));
        assert_eq!(accent(&n(4), &theme), None);
        assert_eq!(fallback_icon(&n(4)), IconName::Notifications);
        let mut mail = n(5);
        mail.category = Some("email.arrived".into());
        assert_eq!(fallback_icon(&mail), IconName::Mail);
    }

    #[test]
    fn content_mapping() {
        crate::i18n::use_turkish_for_tests();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_120);
        let c = card_content(&n(1), None, None, now);
        assert_eq!((c.app_name.as_str(), c.title.as_str(), c.time.as_str()), ("app", "başlık", "2 dk"));
        assert_eq!(c.app_icon, Some(Icon::named(IconName::Notifications)));
        assert_eq!(c.actions, ["Tamam"]);
        assert_eq!(c.body.len(), 1);
        assert!(c.body[0].bold);
        assert_eq!(c.body[0].link.as_deref(), Some("https://example.org/"));
        assert!(c.image.is_none());
    }

    #[test]
    fn event_to_command() {
        let x = n(7);
        assert_eq!(
            command_for(&x, NotificationEvent::Activated, false, false),
            Some(NotifyCommand::Invoke { id: 7, key: "default".into() })
        );
        let no_default = Notification { has_default_action: false, ..n(7) };
        assert_eq!(command_for(&no_default, NotificationEvent::Activated, false, true), None);
        assert_eq!(
            command_for(&x, NotificationEvent::Action(0), false, true),
            Some(NotifyCommand::Invoke { id: 7, key: "ok".into() })
        );
        assert_eq!(command_for(&x, NotificationEvent::Action(5), false, true), None);
        assert_eq!(
            command_for(&x, NotificationEvent::Link("https://example.org/".into()), false, true),
            Some(NotifyCommand::OpenLink("https://example.org/".into()))
        );
        // X: her yerde kapatır; kaydırma popup'ta yalnızca gizler, merkezde kapatır.
        assert_eq!(command_for(&x, NotificationEvent::Dismissed, false, true), Some(NotifyCommand::Dismiss(7)));
        assert_eq!(command_for(&x, NotificationEvent::Dismissed, true, true), Some(NotifyCommand::HidePopup(7)));
        assert_eq!(command_for(&x, NotificationEvent::Dismissed, true, false), Some(NotifyCommand::Dismiss(7)));
    }

    #[test]
    fn fingerprint_tracks_content_not_identity() {
        let a = n(1);
        assert_eq!(fingerprint(&a), fingerprint(&a.clone()));
        let b = Notification { summary: "değişti".into(), ..n(1) };
        assert_ne!(fingerprint(&a), fingerprint(&b));
        let pix = NotifyIcon::Pixmap { width: 1, height: 1, rgba: vec![0u8; 4].into() };
        let c = Notification { image: pix, ..n(1) };
        assert_ne!(fingerprint(&a), fingerprint(&c));
    }

    #[test]
    fn mascot_only_for_internal_request() {
        let theme = Theme::default();
        assert_eq!(card_mascot(&n(1)), None, "istemeyen bildirimde maskot yok");
        let cel = Notification { mascot: Some(NoticeMood::Celebrate), ..n(1) };
        assert_eq!(card_mascot(&cel), Some(Mood::Celebrate));
        assert_ne!(fingerprint(&cel), fingerprint(&n(1)));
        let t = SystemTime::UNIX_EPOCH;
        assert_eq!(build_card(&theme, &n(1), None, None, t).mascot(), None, "dış/sıradan kartta yuva yok");
        assert_eq!(build_card(&theme, &cel, None, None, t).mascot(), Some(Mood::Celebrate));
    }

    /// Kartları ölçüp yerleştirir (`prepare` + `draw` ile aynı sıra: `measure`, sonra `set_rect`).
    fn place_all(set: &mut CardSet, text: &mut TextSystem, theme: &Theme) -> Vec<f32> {
        set.measure(text, theme, 360.0);
        let mut y = 0.0;
        let mut heights = Vec::new();
        for e in &mut set.entries {
            let h = e.height();
            if let Some(card) = &mut e.card {
                card.set_rect(text, theme, Rect::new(0.0, y, 360.0, h));
            }
            y += h;
            heights.push(h);
        }
        heights
    }

    /// `[appearance] mascot` canlı: kart yeniden kurulmaz; framework yuvası kalkar/açılır, kart
    /// yeniden ölçülüp kısalır/uzar. Maskotsuz kart hiç etkilenmez, yuvası hiç olmaz.
    #[test]
    fn mascot_toggle_relayouts_without_rebuild() {
        let mut theme = Theme::default();
        theme.appearance.mascot = true;
        let now = Instant::now();
        let mut text = TextSystem::new();
        let mut set = CardSet::new(&theme, false);
        let cel = Notification { mascot: Some(NoticeMood::Celebrate), ..n(1) };
        set.sync([&cel, &n(2)], now);
        built(&mut set, &theme, 0.0);
        for e in &mut set.entries {
            e.measured = None;
        }
        let shown = place_all(&mut set, &mut text, &theme);
        let slot = |set: &CardSet, i: usize, theme: &Theme| set.entries[i].card.as_ref().unwrap().mascot_rect(theme, now);
        let m = slot(&set, 0, &theme).expect("açıkken maskot yuvası");
        assert_eq!((m.width, m.height), (nuxut::ui::NOTIFICATION_MASCOT_SIZE, nuxut::ui::NOTIFICATION_MASCOT_SIZE));
        assert!(m.x < set.entries[0].card.as_ref().unwrap().rect().center().x, "yuva solda");
        assert_eq!(slot(&set, 1, &theme), None);

        theme.appearance.mascot = false;
        set.set_theme(&theme, now);
        assert!(set.entries.iter().all(|e| e.card.is_some() && e.measured.is_none()), "kart korunur, ölçü düşer");
        let hidden = place_all(&mut set, &mut text, &theme);
        assert_eq!(slot(&set, 0, &theme), None, "gizliyken yuva yok");
        assert_eq!(set.entries[0].card.as_ref().unwrap().mascot(), Some(Mood::Celebrate), "istek korunur");
        assert!(hidden[0] < shown[0], "maskotsuz kart kısalır: {hidden:?} < {shown:?}");
        assert_eq!(hidden[1], shown[1]);

        theme.appearance.mascot = true;
        set.set_theme(&theme, now);
        assert_eq!(place_all(&mut set, &mut text, &theme), shown);
        assert!(slot(&set, 0, &theme).is_some());
    }

    #[test]
    fn card_set_keeps_cards_by_id() {
        let theme = Theme::default();
        let now = Instant::now();
        let mut set = CardSet::new(&theme, false);
        assert!(set.sync([&n(1), &n(2)], now));
        assert_eq!(set.entries.iter().map(CardEntry::id).collect::<Vec<_>>(), [1, 2]);
        // Aynı içerik: değişiklik yok.
        assert!(!set.sync([&n(1), &n(2)], now));
        // Sıra değişimi ve silme.
        assert!(set.sync([&n(2)], now));
        assert_eq!(set.entries.iter().map(CardEntry::id).collect::<Vec<_>>(), [2]);
    }
}
