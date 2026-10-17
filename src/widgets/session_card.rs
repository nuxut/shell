//! Oturum kartı (kenar paneli widget'ı): avatar, ad, açık kalma süresi ve oturum eylemleri.
//! Başlık satırının sağ ucunda küçük Nuxut işareti (`BrandMark`, ikincil rol); ad ve ayrıntı
//! satırı ona değmeden tek satırda kesilir.
//!
//! Eylemler logind'e gider (`Action::Session`). **Her eylem** (kilitle ve askıya al dahil)
//! **yerinde onay** ister: her düğme framework'ün [`ConfirmButton`]'ıdır — ilk basışta bir hapa
//! açılır ("<Eylem> · tekrar bas", ikonun çevresinde `theme.timing.confirm_window`'luk geri
//! sayım halkası), ikinci basış uygular (kuruluştan sonraki `confirm_guard` içindeki basış yok
//! sayılır). Süre dolunca, Escape'te, hap dışına basışta ya da panel kapanınca vazgeçilir. Hap
//! rengi: geri alınamayanlar (oturumu kapat, yeniden başlat, kapat) uyarı (`Destructive`),
//! kilitle ve askıya al vurgu (`Filled`, primary). Hap, basılan düğmenin yerinde açılır (satıra
//! sığdırılır); beklerken diğer düğmeler soluk ve basış görmez, hapın altında kalanlar gizlenir.
//! Halkanın kare/uyanma yönetimi bileşenindir. logind'in izin vermediği eylem devre dışı
//! görünür (bekleyen onayı da düşürür); son hata kartın altında yazılır.

use std::time::Instant;

use chrono::{DateTime, Local};
use nuxut::i18n::t;
use nuxut::theme::ColorRole;
use nuxut::ui::style;
use nuxut::ui::{
    Avatar, AvatarSize, BrandMark, ButtonVariant, ConfirmButton, ConfirmEvent, Icon, IconName, Label, TextRole, Ui,
    text_style,
};
use nuxut::{CursorShape, Key, KeyEvent, KeyState, Point, PointerEvent, Rect, Size, TextSystem};

use super::{Action, PanelWidget, WidgetCx, WidgetKind};
use crate::model::ShellModel;
use crate::services::session::{self, FailureReason, SessionAction};

/// Eylemin ikonu (servis UI'dan bağımsız; eşleme burada).
fn action_icon(a: SessionAction) -> IconName {
    match a {
        SessionAction::Lock => IconName::Lock,
        SessionAction::Logout => IconName::Logout,
        SessionAction::Suspend => IconName::DarkMode,
        SessionAction::Reboot => IconName::Restart,
        SessionAction::PowerOff => IconName::Power,
    }
}

/// Onay bekleyen hapın etiketi.
fn armed_label(a: SessionAction) -> String {
    t!("shell-quick-session-confirm-armed", action = a.label())
}

/// Onay bekleyen hapın değişkeni: geri alınamayan eylem uyarı rengi, diğerleri vurgu.
fn armed_variant(a: SessionAction) -> ButtonVariant {
    if a.is_destructive() { ButtonVariant::Destructive } else { ButtonVariant::Filled }
}

fn action_button(theme: &nuxut::Theme, a: SessionAction) -> ConfirmButton {
    ConfirmButton::new(theme, ButtonVariant::Tonal, Icon::named(action_icon(a)), armed_label(a))
        .with_armed_variant(armed_variant(a))
}

/// Bir eylem onay beklerken diğer (tonal) düğmelerin grup opaklığı. Standartta
/// `style::DISABLED_CONTENT` (%38) aynen; yüksek karşıtlıkta framework `style::disabled_content`
/// tonal kap üstündeki `on_surface` içeriği ≥ 3 tutacak opaklığa çıkarır (t124, devre dışı kararı).
fn inert_alpha(theme: &nuxut::Theme) -> f32 {
    let c = &theme.colors;
    style::disabled_content(theme, c.on_surface, &[style::tonal_container(theme)]).a
}

/// Hata satırı en çok bu kadar satıra sarılır; taşan kısım `…` ile kesilir.
const ERROR_MAX_LINES: usize = 2;

/// Hata metni: kart genişliğinde sarılır, `ERROR_MAX_LINES`'ta üç noktayla kesilir.
fn error_style(theme: &nuxut::Theme) -> nuxut::TextStyle {
    text_style(theme, TextRole::Caption).with_max_lines(ERROR_MAX_LINES)
}

/// Kartta görünen kısa hata ("Kilitle başarısız: İzin verilmedi"); ham D-Bus ayrıntısı logda.
fn error_text(action: SessionAction, reason: FailureReason) -> String {
    t!("shell-quick-session-action-failed", action = action.label(), message = reason.label())
}

/// Başlıktaki marka işaretinin kenarı (mantıksal px; DESIGN "Marka işareti": ≥24).
pub const MARK_SIDE: f32 = 24.0;

/// Ad: tek satır (işarete değmeden kesilir).
fn name_style(theme: &nuxut::Theme) -> nuxut::TextStyle {
    text_style(theme, TextRole::Title).with_max_lines(1)
}

/// Ad altındaki satır: tek satır.
fn detail_style(theme: &nuxut::Theme) -> nuxut::TextStyle {
    text_style(theme, TextRole::Caption).with_max_lines(1)
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

#[derive(Debug)]
pub struct SessionCard {
    avatar: Avatar,
    /// Başlık satırının sağ ucundaki marka işareti.
    mark: BrandMark,
    name: Label,
    detail: Label,
    error: Label,
    has_error: bool,
    /// `SessionAction::ALL` sırasıyla eylem düğmeleri (en çok biri onay bekler).
    actions: Vec<(SessionAction, ConfirmButton)>,
    /// Son çizilen bekleyen eylem (geçişte satır hasarı için).
    drawn_armed: Option<SessionAction>,
    login: String,
}

impl SessionCard {
    pub fn new(cx: &WidgetCx) -> Self {
        let t = cx.theme;
        Self {
            avatar: Avatar::new(t, "", AvatarSize::Large),
            mark: BrandMark::new().with_role(ColorRole::OnSurfaceVariant).with_size(MARK_SIDE),
            name: Label::new("", name_style(t)),
            detail: Label::new("", detail_style(t)),
            error: Label::new("", error_style(t)),
            has_error: false,
            actions: SessionAction::ALL.into_iter().map(|a| (a, action_button(t, a))).collect(),
            drawn_armed: None,
            login: String::new(),
        }
    }

    fn padding(cx: &WidgetCx) -> f32 {
        cx.theme.space(5)
    }

    /// Boştaki (yalnızca ikon) düğmenin kare kenarı: `Button` yüksekliği.
    fn slot(cx: &WidgetCx) -> f32 {
        cx.theme.space(10)
    }

    /// Kart içi yerleşim: (avatar merkezi, metin başlangıcı, eylem satırı).
    fn layout(rect: Rect, cx: &WidgetCx) -> (Point, Point, Rect) {
        let p = Self::padding(cx);
        let side = AvatarSize::Large.side(cx.theme);
        let avatar = Point::new(rect.x + p + side * 0.5, rect.y + p + side * 0.5);
        let text = Point::new(rect.x + p + side + cx.theme.space(4), rect.y + p);
        let row = Rect::new(rect.x + p, rect.y + p + side + cx.theme.space(4), rect.width - 2.0 * p, Self::slot(cx));
        (avatar, text, row)
    }

    /// Başlıktaki işaret: sağ iç boşlukta, avatarın merkez yüksekliğinde (köşeden en az iç
    /// boşluk kadar uzak: panelin sabitleme düğmesi köşe boşluğuna biner, işarete değmez).
    fn mark_rect(rect: Rect, cx: &WidgetCx) -> Rect {
        let (avatar, _, _) = Self::layout(rect, cx);
        let right = rect.right() - Self::padding(cx);
        Rect::new(right - MARK_SIDE, avatar.y - MARK_SIDE * 0.5, MARK_SIDE, MARK_SIDE)
    }

    /// Ad ve ayrıntı satırının en geniş hâli (işaretin solunda `space(3)` boşluk).
    fn text_width(rect: Rect, cx: &WidgetCx) -> f32 {
        let (_, text, _) = Self::layout(rect, cx);
        (Self::mark_rect(rect, cx).x - cx.theme.space(3) - text.x).max(1.0)
    }

    /// Onay bekleyen eylemin sırası (en çok bir tane).
    fn armed(&self, now: Instant) -> Option<usize> {
        self.actions.iter().position(|(_, b)| b.is_armed(now))
    }

    /// Süresi dolan onayları boşa döndürür (yerleşimden önce: içerik ölçüsü değişir).
    fn expire(&mut self, now: Instant) -> bool {
        self.actions.iter_mut().fold(false, |acc, (_, b)| b.expire(now) | acc)
    }

    /// Düğmeler satıra eşit aralıklı kareler; bekleyen eylem kendi yerinde hap (geçerli
    /// içeriğinin boyu, kendi merkezine ortalı, satıra sığdırılmış). Hap dikdörtgenini döndürür.
    fn place(&mut self, text: &mut TextSystem, row: Rect, armed: Option<usize>, cx: &WidgetCx) -> Option<Rect> {
        let n = self.actions.len() as f32;
        let side = Self::slot(cx);
        let gap = ((row.width - n * side) / (n - 1.0).max(1.0)).max(0.0);
        let mut pill = None;
        for (i, (_, b)) in self.actions.iter_mut().enumerate() {
            let slot = Rect::new(row.x + i as f32 * (side + gap), row.y, side, side);
            if armed == Some(i) {
                let size = b.preferred_size(text, cx.theme);
                let w = size.width.min(row.width);
                let x = (slot.center().x - w * 0.5).clamp(row.x, (row.right() - w).max(row.x));
                let r = Rect::new(x, row.center().y - size.height * 0.5, w, size.height);
                b.set_rect(r);
                pill = Some(r);
            } else {
                b.set_rect(slot);
            }
        }
        pill
    }

    /// Ad altındaki satır: kullanıcı adı · açık kalma süresi (boş parçalar atlanır).
    fn refresh_detail(&mut self) -> bool {
        let up = session::uptime().map(session::format_uptime).unwrap_or_default();
        self.detail.set_text(&detail_text(&self.login, &up))
    }
}

fn detail_text(login: &str, uptime: &str) -> String {
    match (login.is_empty(), uptime.is_empty()) {
        (_, true) => login.to_owned(),
        (true, false) => t!("shell-quick-session-uptime", uptime = uptime),
        (false, false) => t!("shell-quick-session-detail", login = login, uptime = uptime),
    }
}

impl PanelWidget for SessionCard {
    fn kind(&self) -> WidgetKind {
        WidgetKind::Session
    }

    fn sync(&mut self, model: &ShellModel, cx: &WidgetCx) -> bool {
        let mut changed = false;
        if self.login != model.user.login {
            self.login.clone_from(&model.user.login);
            self.avatar.set_name(model.user.display_name());
            self.name.set_text(model.user.display_name());
            self.refresh_detail();
            changed = true;
        }
        // İzin kalkarsa bileşen bekleyen onayı da düşürür.
        for (a, b) in &mut self.actions {
            changed |= b.set_enabled(model.session_allowed(*a), cx.now);
        }
        let err = model.session_error.map(|(a, r)| error_text(a, r)).unwrap_or_default();
        self.has_error = !err.is_empty();
        changed |= self.error.set_text(&err);
        changed
    }

    fn set_time(&mut self, _now: &DateTime<Local>, _cx: &WidgetCx) -> bool {
        self.refresh_detail()
    }

    fn set_theme(&mut self, cx: &WidgetCx) {
        let t = cx.theme;
        self.avatar.set_theme(t, cx.now);
        self.mark.set_theme(t, cx.now);
        self.name.set_style(name_style(t));
        self.detail.set_style(detail_style(t));
        self.error.set_style(error_style(t));
        for (_, b) in &mut self.actions {
            b.set_theme(t, cx.now);
        }
    }

    fn height(&mut self, text: &mut TextSystem, width: f32, cx: &WidgetCx) -> Option<f32> {
        let p = Self::padding(cx);
        let side = AvatarSize::Large.side(cx.theme);
        // Hata satırı eylem satırı genişliğinde sarılır (çizimle aynı genişlik).
        self.error.set_max_width(Some((width - 2.0 * p).max(1.0)));
        let err = if self.has_error { cx.theme.space(2) + self.error.measure(text).height } else { 0.0 };
        Some(p + side + cx.theme.space(4) + Self::slot(cx) + err + p)
    }

    fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &WidgetCx) {
        let now = ui.now;
        let colors = &cx.theme.colors;
        let (avatar, text, row) = Self::layout(rect, cx);
        self.avatar.set_center(avatar);
        self.avatar.draw(ui);
        self.mark.set_rect(Self::mark_rect(rect, cx));
        self.mark.draw(ui);

        let width = Some(Self::text_width(rect, cx));
        self.name.set_max_width(width);
        self.detail.set_max_width(width);

        let n = self.name.measure_in(ui.painter);
        self.name.draw(ui.painter, text, ui.fade(colors.on_surface));
        self.detail.draw(ui.painter, Point::new(text.x, text.y + n.height + cx.theme.space(1)), ui.fade(colors.on_surface_variant));

        self.expire(now);
        let armed = self.armed(now);
        let armed_action = armed.map(|i| self.actions[i].0);
        if armed_action != self.drawn_armed {
            // Satır hap ↔ düğmeler arasında değişti: düğmelerin kendi hasar takibi bunu görmez.
            ui.damage(row.expand(cx.theme.space(2)));
            self.drawn_armed = armed_action;
        }
        let pill = self.place(ui.painter.text_system(), row, armed, cx);
        match (armed, pill) {
            (Some(i), Some(pill)) => {
                // Diğer düğmeler soluk (basış görmez); hapın altında kalanlar gizli.
                let gap = cx.theme.space(1);
                ui.with_alpha(inert_alpha(cx.theme), |ui| {
                    for (j, (_, b)) in self.actions.iter_mut().enumerate() {
                        if j != i && !overlaps(b.rect(), pill.expand(gap)) {
                            b.draw(ui);
                        }
                    }
                });
                self.actions[i].1.draw(ui);
            }
            _ => {
                for (_, b) in &mut self.actions {
                    b.draw(ui);
                }
            }
        }
        if self.has_error {
            let y = row.bottom() + cx.theme.space(2);
            self.error.set_max_width(Some(row.width.max(1.0)));
            // TODO(framework): temada `error` rolü yok (framework'ten istendi); şimdilik vurgu rengi.
            self.error.draw(ui.painter, Point::new(row.x, y), ui.fade(colors.primary));
        }
    }

    /// Kullanıcı adı ve ayrıntı (etiket) ve oturum düğmeleri: ad eylemin adı ("Kilitle"); onay
    /// beklerken ad onay etiketi ("Kapat · tekrar bas"), ikinci etkinleştirme onaylar.
    fn access(&self, tree: &mut nuxut::access::AccessTree, rect: Rect, cx: &WidgetCx) {
        crate::widgets::access_card(tree, self.kind(), rect, |tree| {
            let who: Vec<&str> = [self.name.text(), self.detail.text()].into_iter().filter(|s| !s.is_empty()).collect();
            tree.push("user", nuxut::access::Role::Label).value(&who.join(", ")).bounds(rect);
            let armed = self.armed(cx.now);
            for (i, (a, b)) in self.actions.iter().enumerate() {
                let label = if armed == Some(i) { armed_label(*a) } else { a.label() };
                tree.push(*a, nuxut::access::Role::Button)
                    .label(&label)
                    .bounds(b.rect())
                    .enabled(b.is_enabled())
                    .clickable();
            }
            if self.has_error {
                self.error.access(tree, "error", rect);
            }
        });
    }

    fn pointer(&mut self, event: &PointerEvent, rect: Rect, text: &mut TextSystem, cx: &WidgetCx) -> (bool, Option<Action>) {
        let now = cx.now;
        let (_, _, row) = Self::layout(rect, cx);
        let mut repaint = self.expire(now);
        let armed = self.armed(now);
        self.place(text, row, armed, cx);
        if let Some(i) = armed {
            // Bekleme sırasında olayları yalnızca hap görür: kendi dışındaki basış vazgeçirir,
            // diğer düğmeler basışı görmez (bırakmada tıklama üretmez).
            let (action, b) = &mut self.actions[i];
            let r = b.pointer(event, now);
            return match r.event {
                Some(ConfirmEvent::Confirmed) => (true, Some(Action::Session(*action))),
                Some(_) => (true, None),
                None => (repaint | r.repaint, None),
            };
        }
        let mut armed_now = false;
        for (_, b) in &mut self.actions {
            let r = b.pointer(event, now);
            repaint |= r.repaint;
            armed_now |= r.event == Some(ConfirmEvent::Armed);
        }
        if armed_now {
            // İçerik hapa döndü: hap dikdörtgeni hemen (sonraki olay dışarı-basış denetimini
            // doğru dikdörtgenle yapsın).
            let armed = self.armed(now);
            self.place(text, row, armed, cx);
        }
        (repaint | armed_now, None)
    }

    fn key(&mut self, event: &KeyEvent, cx: &WidgetCx) -> Option<(bool, Option<Action>)> {
        // İlk Escape yalnızca bekleyen onaydan vazgeçer (panel açık kalır). Kart odak taşımadığı
        // için Enter/Space bileşene verilmez (yoksa odaksız düğme basılırdı).
        if event.key != Key::Escape || event.state != KeyState::Pressed {
            return None;
        }
        let i = self.armed(cx.now)?;
        let r = self.actions[i].1.key(event, cx.now);
        (r.event == Some(ConfirmEvent::Cancelled)).then_some((true, None))
    }

    fn panel_closed(&mut self, now: Instant) {
        for (_, b) in &mut self.actions {
            b.cancel(now);
        }
    }

    fn cursor(&self, _rect: Rect, p: Point, cx: &WidgetCx) -> Option<CursorShape> {
        // Düğme dikdörtgenleri son yerleşimden (çizim/işaretçi her seferinde günceller).
        match self.armed(cx.now) {
            Some(i) => self.actions[i].1.cursor(p),
            None => self.actions.iter().find_map(|(_, b)| b.cursor(p)),
        }
    }

    fn corner_clearance(&self, cx: &WidgetCx) -> Size {
        let p = Self::padding(cx);
        Size::new(p, p)
    }

    fn is_animating(&mut self, _now: Instant) -> bool {
        // Düğmeler ve geri sayım halkası kare isteğini `ui.request_frame()`/`wake_at` ile yapar.
        false
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nuxut::theme::IconSet;
    use nuxut::ui::{ButtonVariant, IconName, glyph};
    use nuxut::{Button, Key, KeyEvent, KeyState, Modifiers, Point, PointerEvent, PointerKind, Rect, TextSystem};

    use super::{MARK_SIDE, SessionCard, action_icon, armed_label, armed_variant, detail_text};
    use crate::model::ShellModel;
    use crate::services::session::{Capability, FailureReason, SessionAction};
    use crate::widgets::{Action, PanelWidget, WidgetCx, WidgetSettings};

    const CARD: Rect = Rect::new(0.0, 0.0, 400.0, 160.0);

    struct Harness {
        theme: nuxut::Theme,
        settings: WidgetSettings,
        text: TextSystem,
        card: SessionCard,
    }

    impl Harness {
        fn new() -> Self {
            let theme = nuxut::Theme::default();
            let settings = WidgetSettings::default();
            let card = SessionCard::new(&cx(&theme, &settings, Instant::now()));
            Self { theme, settings, text: TextSystem::new(), card }
        }

        /// Eylem düğmesinin (ya da hapın) merkezi; yerleşimi `pointer` günceller.
        fn center(&mut self, a: SessionAction, now: Instant) -> Point {
            let motion = PointerEvent::new(Point::new(-50.0, -50.0), PointerKind::Motion);
            let cx = cx(&self.theme, &self.settings, now);
            self.card.pointer(&motion, CARD, &mut self.text, &cx);
            self.card.actions.iter().find(|(x, _)| *x == a).map(|(_, b)| b.rect().center()).unwrap()
        }

        fn click_at(&mut self, p: Point, now: Instant) -> (bool, Option<Action>) {
            let cx = cx(&self.theme, &self.settings, now);
            let press = PointerEvent::new(p, PointerKind::Press(Button::Left));
            let release = PointerEvent::new(p, PointerKind::Release(Button::Left));
            let (r1, a1) = self.card.pointer(&press, CARD, &mut self.text, &cx);
            let (r2, a2) = self.card.pointer(&release, CARD, &mut self.text, &cx);
            (r1 | r2, a1.or(a2))
        }

        fn click(&mut self, a: SessionAction, now: Instant) -> (bool, Option<Action>) {
            let p = self.center(a, now);
            self.click_at(p, now)
        }

        fn armed(&self, now: Instant) -> Option<SessionAction> {
            self.card.armed(now).map(|i| self.card.actions[i].0)
        }
    }

    fn cx<'a>(theme: &'a nuxut::Theme, settings: &'a WidgetSettings, now: Instant) -> WidgetCx<'a> {
        WidgetCx { theme, settings, vertical: false, item: 26.0, now, bounds: Rect::new(0.0, 0.0, 1920.0, 1080.0) }
    }

    fn escape() -> KeyEvent {
        KeyEvent { key: Key::Escape, state: KeyState::Pressed, text: None, modifiers: Modifiers::default(), xkb: None }
    }

    fn is_session(a: Option<Action>, want: SessionAction) -> bool {
        matches!(a, Some(Action::Session(x)) if x == want)
    }

    /// Her eylemin ikonu her iki ikon setinde (Material, Nerd) glife sahip ve adıyla geri
    /// çözülür (yoksa düğme boş çizilir).
    #[test]
    fn action_icons_resolve() {
        for a in SessionAction::ALL {
            let icon = action_icon(a);
            assert_eq!(icon.as_str().parse::<IconName>().ok(), Some(icon), "{a:?}");
            for set in [IconSet::Material, IconSet::Nerd] {
                assert!(!glyph(set, icon, false).is_empty() && !glyph(set, icon, true).is_empty(), "{a:?} {set:?}");
            }
        }
    }

    #[test]
    fn header_mark_clears_corner_text_and_actions() {
        let theme = nuxut::Theme::default();
        let settings = WidgetSettings::default();
        let c = cx(&theme, &settings, Instant::now());
        let p = SessionCard::padding(&c);
        for card in [CARD, Rect::new(30.0, 50.0, 320.0, 160.0)] {
            let m = SessionCard::mark_rect(card, &c);
            let (avatar, text, row) = SessionCard::layout(card, &c);
            assert!(MARK_SIDE >= 24.0 && m.width == MARK_SIDE);
            // Köşeden en az iç boşluk kadar içeride (sabitleme düğmesi köşe boşluğunda).
            assert!(card.right() - m.right() >= p - 1e-3 && m.y - card.y >= p - 1e-3, "{m:?}");
            assert!((m.center().y - avatar.y).abs() < 1e-3, "avatarla aynı hizada");
            assert!(m.bottom() <= row.y, "eylem satırının üstünde");
            let w = SessionCard::text_width(card, &c);
            assert!(text.x + w + theme.space(3) <= m.x + 1e-3, "metin işarete değmez");
        }
    }

    #[test]
    fn detail_skips_empty_parts() {
        crate::i18n::use_turkish_for_tests();
        assert_eq!(detail_text("ada", "2 sa 5 dk"), "ada · açık 2 sa 5 dk");
        assert_eq!(detail_text("", "2 sa 5 dk"), "açık 2 sa 5 dk");
        assert_eq!(detail_text("ada", ""), "ada");
        assert_eq!(detail_text("", ""), "");
    }

    /// Hata kartta kısa ve anlaşılır (D-Bus adı yok); dar kartta kart genişliğinde sarılır,
    /// en çok iki satırda kesilir, yükseklik buna göre büyür.
    #[test]
    fn error_is_short_and_fits_card_width() {
        crate::i18n::use_turkish_for_tests();
        let mut h = Harness::new();
        let now = Instant::now();
        let cx = cx(&h.theme, &h.settings, now);
        let mut model = ShellModel::default();
        let base = h.card.height(&mut h.text, 400.0, &cx).unwrap();

        model.session_error = Some((SessionAction::Lock, FailureReason::Denied));
        assert!(h.card.sync(&model, &cx));
        assert_eq!(h.card.error.text(), "Kilitle başarısız: İzin verilmedi");
        assert!(!h.card.error.text().contains("org.freedesktop"));
        let with_error = h.card.height(&mut h.text, 400.0, &cx).unwrap();
        assert!(with_error > base, "hata satırı yüksekliğe eklenir");

        // Dar kart + uzun metin: genişliği aşmaz, iki satırda üç noktayla kesilir.
        let p = SessionCard::padding(&cx);
        for width in [400.0, 220.0, 120.0] {
            h.card.error.set_text(&"Kilitle başarısız: çok uzun bir açıklama ".repeat(6));
            let height = h.card.height(&mut h.text, width, &cx).unwrap();
            let layout = h.card.error.layout(&mut h.text);
            assert!(layout.size().width <= width - 2.0 * p + 0.5, "{width}: {:?}", layout.size());
            assert!(layout.line_count() <= super::ERROR_MAX_LINES, "{width}: {} satır", layout.line_count());
            assert!((height - (base + h.theme.space(2) + layout.size().height)).abs() < 0.01, "{width}");
        }

        model.session_error = None;
        assert!(h.card.sync(&model, &cx));
        assert!(h.card.error.text().is_empty());
        assert_eq!(h.card.height(&mut h.text, 400.0, &cx).unwrap(), base);
    }

    #[test]
    fn pill_tone_follows_reversibility() {
        crate::i18n::use_turkish_for_tests();
        assert_eq!(armed_variant(SessionAction::Lock), ButtonVariant::Filled);
        assert_eq!(armed_variant(SessionAction::Suspend), ButtonVariant::Filled);
        for a in [SessionAction::Logout, SessionAction::Reboot, SessionAction::PowerOff] {
            assert_eq!(armed_variant(a), ButtonVariant::Destructive, "{a:?}");
        }
        assert_eq!(armed_label(SessionAction::Reboot), "Yeniden başlat · tekrar bas");
    }

    /// Her eylem ilk basışta uygulanmaz, hapa açılır; koruma süresinden sonra pencere içindeki
    /// ikinci basış (hapın üstünde) uygular.
    #[test]
    fn every_action_confirms_with_second_press() {
        let mut h = Harness::new();
        let t = Instant::now();
        let after_guard = t + h.theme.timing.confirm_guard + Duration::from_millis(50);
        for a in SessionAction::ALL {
            let (repaint, act) = h.click(a, t);
            assert!(repaint && act.is_none(), "{a:?} ilk basışta uygulanmaz");
            assert_eq!(h.armed(t), Some(a));
            // Hap kendi düğmesinin yerinde, satırın içinde ve kareden geniş.
            let pill = h.card.actions.iter().find(|(x, _)| *x == a).unwrap().1.rect();
            assert!(pill.width > h.theme.space(10), "{a:?} hap genişler");
            assert!(pill.x >= CARD.x && pill.right() <= CARD.right(), "{a:?} satıra sığar");
            // Çift tıkın ikinci yarısı (koruma süresi içinde) uygulanmaz.
            assert!(h.click(a, t + Duration::from_millis(100)).1.is_none(), "{a:?} koruma");
            assert!(is_session(h.click(a, after_guard).1, a), "{a:?} ikinci basış uygular");
            assert_eq!(h.armed(after_guard), None, "uygulanınca biter");
        }
    }

    /// Beklerken diğer düğmeler basış görmez: başka eylemin yerine basış (hap dışı) yalnızca
    /// vazgeçirir, o eylemi kurmaz.
    #[test]
    fn other_buttons_inert_while_armed() {
        let mut h = Harness::new();
        let t = Instant::now();
        let other = h.center(SessionAction::PowerOff, t);
        h.click(SessionAction::Lock, t);
        assert_eq!(h.armed(t), Some(SessionAction::Lock));
        let (repaint, act) = h.click_at(other, t + Duration::from_secs(1));
        assert!(repaint && act.is_none());
        assert_eq!(h.armed(t + Duration::from_secs(1)), None, "dışarı basış vazgeçirir, başkasını kurmaz");
    }

    #[test]
    fn escape_cancels_only_while_armed() {
        let mut h = Harness::new();
        let t = Instant::now();
        let c = cx(&h.theme, &h.settings, t);
        assert_eq!(h.card.key(&escape(), &c), None, "bekleyen yokken Escape paneli kapatsın");
        h.click(SessionAction::Reboot, t);
        let c = cx(&h.theme, &h.settings, t);
        assert!(matches!(h.card.key(&escape(), &c), Some((true, None))));
        assert_eq!(h.armed(t), None);
    }

    #[test]
    fn window_expiry_and_panel_close_cancel() {
        let mut h = Harness::new();
        let t = Instant::now();
        let w = h.theme.timing.confirm_window;
        h.click(SessionAction::PowerOff, t);
        assert_eq!(h.armed(t + w - Duration::from_millis(1)), Some(SessionAction::PowerOff));
        assert_eq!(h.armed(t + w), None);
        // Süre dolduktan sonraki basış yeniden onay ister, uygulamaz.
        assert!(h.click(SessionAction::PowerOff, t + w).1.is_none());
        assert_eq!(h.armed(t + w), Some(SessionAction::PowerOff));
        h.card.panel_closed(t + w);
        assert_eq!(h.armed(t + w), None, "panel kapanınca vazgeçilir");
    }

    #[test]
    fn revoked_permission_drops_pending_confirm() {
        let mut h = Harness::new();
        let t = Instant::now();
        h.click(SessionAction::Suspend, t);
        assert_eq!(h.armed(t), Some(SessionAction::Suspend));
        let model = ShellModel { session_caps: vec![(SessionAction::Suspend, Capability::No)], ..ShellModel::default() };
        let c = cx(&h.theme, &h.settings, t);
        assert!(h.card.sync(&model, &c));
        assert_eq!(h.armed(t), None, "izin kalkınca onay düşer");
        // Devre dışı düğme kurulmaz.
        assert!(h.click(SessionAction::Suspend, t).1.is_none());
        assert_eq!(h.armed(t), None);
    }
}

#[cfg(test)]
mod contrast_tests {
    use nuxut::Theme;
    use nuxut::theme::contrast::NON_TEXT_MIN;
    use nuxut::ui::style::{self, DISABLED_CONTENT};

    use super::inert_alpha;

    /// t124: standartta eski %38; yüksek karşıtlıkta grup opaklığıyla soluklaşan tonal düğmenin
    /// içeriği, aynı opaklıktaki kabına karşı (her tema yüzeyinde) ≥ 3.
    #[test]
    fn inert_buttons_standard_unchanged_high_contrast_at_least_three() {
        assert_eq!(inert_alpha(&Theme::default()), DISABLED_CONTENT);
        for mode in ["dark", "light"] {
            let theme = Theme::parse(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"high\"\n")).unwrap().0;
            let a = inert_alpha(&theme);
            let fill = style::tonal_container(&theme).multiply_alpha(a);
            let content = theme.colors.on_surface.multiply_alpha(a);
            let ratio = style::worst_over(content, &style::backdrops(&theme, &[fill]));
            assert!(ratio >= NON_TEXT_MIN, "{mode}: alpha {a} → {ratio}");
        }
    }
}
