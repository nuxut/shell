//! Ses, parlaklık ve gece ışığı kartlarının satırı: solda ikon düğmesi (sessiz / bilgi),
//! ortada kaydırıcı, sağda değer (yüzde ya da Kelvin). Bileşenlerin bileşimi; çizim ve girdi framework bileşenlerinde.

use std::time::Instant;

use nuxut::ui::{Icon, IconButton, IconButtonVariant, IconName, Label, Orientation, Slider, TextRole, Ui, text_style};
use nuxut::{CursorShape, Point, PointerEvent, Rect, Theme};

/// Satırdaki etkileşim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RowEvent {
    /// İkon düğmesine tıklandı.
    Icon,
    /// Kaydırıcı değeri (0..=1).
    Value(f32),
}

#[derive(Debug)]
pub struct SliderRow {
    button: IconButton,
    slider: Slider,
    percent: Label,
    icon: IconName,
    /// İkon tıklanabilir mi (parlaklıkta yalnızca gösterge).
    interactive: bool,
}

impl SliderRow {
    pub fn new(theme: &Theme, icon: IconName) -> Self {
        Self {
            button: IconButton::new(theme, Icon::named(icon), IconButtonVariant::Standard),
            slider: Slider::new(theme, Orientation::Horizontal, 0.0),
            percent: Label::new("", text_style(theme, TextRole::Label).with_tabular_numbers()),
            icon,
            interactive: true,
        }
    }

    /// İkon yalnızca gösterge: işaretçi olayı almaz.
    pub fn inert(mut self) -> Self {
        self.interactive = false;
        self
    }

    /// Soldaki ikon düğmesinin genişliği (üstteki satırların ikonu aynı sütuna hizalanır).
    pub fn icon_slot(&self, theme: &Theme) -> f32 {
        self.button.preferred_size(theme).width
    }

    pub fn height(&self, theme: &Theme) -> f32 {
        self.slider.preferred_size(theme, 0.0).height.max(self.button.preferred_size(theme).height)
    }

    /// Dış durum. `value` 0..=1 (kaydırıcı), `percent` gösterilen yüzde. Değiştiyse `true`.
    pub fn set(&mut self, icon: IconName, value: f32, percent: u32, enabled: bool, now: Instant) -> bool {
        self.set_with_text(icon, value, &crate::percent::text(percent.into()), enabled, now)
    }

    /// [`Self::set`], sağdaki değer metni hazır verilir (ör. gece ışığında "4000 K").
    pub fn set_with_text(&mut self, icon: IconName, value: f32, text: &str, enabled: bool, now: Instant) -> bool {
        let mut changed = false;
        if self.icon != icon {
            self.icon = icon;
            self.button.set_icon(Icon::named(icon));
            changed = true;
        }
        changed |= self.slider.set_value(value, now);
        changed |= self.percent.set_text(text);
        changed |= self.button.set_enabled(enabled, now);
        self.slider.set_enabled(enabled, now);
        changed
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.button.set_theme(theme, now);
        self.slider.set_theme(theme, now);
        self.percent.set_style(text_style(theme, TextRole::Label).with_tabular_numbers());
    }

    fn place(&mut self, rect: Rect, theme: &Theme) {
        let b = self.button.preferred_size(theme);
        self.button.set_rect(Rect::new(rect.x, rect.center().y - b.height * 0.5, b.width, b.height));
        // Yüzde sütunu "%150" sığacak sabit genişlikte: değer değişince kaydırıcı oynamaz.
        let pct_w = theme.space(12);
        let x = rect.x + b.width + theme.space(2);
        let w = (rect.right() - pct_w - theme.space(2) - x).max(0.0);
        let s = self.slider.preferred_size(theme, w);
        self.slider.set_rect(Rect::new(x, rect.center().y - s.height * 0.5, w, s.height));
    }

    pub fn draw(&mut self, ui: &mut Ui, rect: Rect) {
        let theme = ui.theme;
        self.place(rect, theme);
        self.button.draw(ui);
        self.slider.draw(ui);
        let m = self.percent.measure_in(ui.painter);
        let at = Point::new(rect.right() - m.width, rect.center().y - m.height * 0.5);
        let c = ui.fade(theme.colors.on_surface_variant);
        self.percent.draw(ui.painter, at, c);
    }

    pub fn pointer(&mut self, event: &PointerEvent, rect: Rect, theme: &Theme, now: Instant) -> (bool, Option<RowEvent>) {
        self.place(rect, theme);
        let b = if self.interactive { self.button.pointer(event, now) } else { nuxut::ui::Response::none() };
        let s = self.slider.pointer(event, now);
        let ev = match (b.event, s.event) {
            (Some(_), _) => Some(RowEvent::Icon),
            (None, Some(v)) => Some(RowEvent::Value(v)),
            (None, None) => None,
        };
        (b.repaint | s.repaint, ev)
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.slider.cursor(p).or_else(|| if self.interactive { self.button.cursor(p) } else { None })
    }
}

// Ekran okuyucu (framework ADR-0011).
impl SliderRow {
    /// Satırın düğümleri `tree`'nin şu anki ebeveyni altına: ikon düğmesi (yalnız tıklanabilirse;
    /// ad `icon_label`, ör. "Sesi kapat") ve kaydırıcı (`Role::Slider`, ad `label`, değer metni
    /// sağdaki yazı: "%64", "4000 K"). Kaydırıcının kimliği `key`ten, düğmeninki `(key, "icon")`.
    pub fn access(&self, tree: &mut nuxut::access::AccessTree, key: &str, label: &str, icon_label: &str) {
        if self.interactive {
            self.button.access(tree, (key, "icon")).label(icon_label);
        }
        // 0..=100: Orca sayıyı okur ("64"); birimli metin değer dizgisinde ("%64", "4000 K").
        self.slider.access(tree, key, 0.0..=100.0).label(label).value(self.percent.text());
    }

    /// AT eylemi (`Increment`/`Decrement`/sayısal `SetValue`) bu satırın kaydırıcısına mı:
    /// öyleyse işaretçiyle aynı olay (`RowEvent::Value`). `parent`: [`Self::access`] çağrısındaki
    /// ebeveyn. Tıklama işaretçi yolundan gelir, burada değil.
    pub fn access_action(&mut self, parent: nuxut::access::NodeId, key: &str, request: &nuxut::access::AccessRequest, now: Instant) -> Option<RowEvent> {
        if request.node != nuxut::access::child_id(parent, key) {
            return None;
        }
        self.slider.access_action(request.action, 0.0..=100.0, now).event.map(RowEvent::Value)
    }
}
