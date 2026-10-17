//! Dosya seçicinin ekran okuyucu ağacı (framework ADR-0011).
//!
//! Pencere kökü başlıktır; araç çubuğu (geri, üst klasör, konum çubuğu, gizli dosyalar, yeni
//! klasör, arama), yerler listesi, dosya listesi (ya da boş durum), ad alanı, filtre, seçim
//! kutuları, durum satırı (canlı bölge) ve Vazgeç/kabul. Diyalog açıkken yalnız diyalog (modal;
//! geri kalanı girdi almaz). Odak gerçek klavye odağını izler: liste odaktaysa odaktaki satır,
//! metin alanı odaktaysa alan.
//!
//! **Güvenlik (2026-10-17, HIGH bulgu):** a11y bus'ı oturumdaki her sürece (Flatpak'te
//! varsayılan izinli) açıktır. Seçiciyi açtıran sandbox'lı uygulama AT `Click` ile satır seçip
//! kabule basarsa kullanıcının seçmediği dosyalara erişim kazanırdı (portal sandbox'ından kaçış).
//! Kural: **erişim veren hiçbir eylem AT'den tetiklenemez.** AT yalnız gezinir ve seçer:
//! `Click` yalnız izin listesindeki düğümlerde — araç çubuğu (yeni klasör hariç: dosya sistemine
//! yazar), yerler, dosya satırları (tek tık seçer; AT tıklamaları çift tık sayılmaz), filtre,
//! seçim kutuları, ad alanını odaklama ve Vazgeç. **Kabul düğmesinde `Click` eylemi yok**,
//! diyalog düğmeleri (üzerine yazma onayı, yeni klasör) AT'ye kapalı; onay yalnız gerçek
//! Wayland girdisiyle (Enter, işaretçi). Orca kullanıcısı odağı taşır, onayı kendi klavyesi
//! verir. AT metin yazamaz (ADR-0011 karar 6; konum ve ad alanı AT için salt okunur).
//!
//! İzinli eylemler gerçek girdinin yolundan gider: `Click` düğümün ortasına işaretçi tıklaması
//! (hareket → bas → bırak → ayrıl), metin alanında `Focus` alanı odaklar. Ağaç içeriği
//! loglanmaz (değişmez 9).

use std::time::{Duration, Instant};

use nuxut::access::{AccessAction, AccessRequest, AccessTree, Action as AxAction, Live, NodeId, Role};
use nuxut::i18n::t;
use nuxut::{Button as MouseButton, Context, Point, PointerEvent, PointerKind, SurfaceId, Theme};
use nuxut_portal_backend::chooser::protocol::Mode;

use super::{ChoiceWidget, FileChooser, Focus, Modal};

/// `tree`'ye `start` indisinden sonra eklenen düğümler (bir bölümün tamamı).
fn added_since(tree: &AccessTree, start: usize) -> impl Iterator<Item = NodeId> + '_ {
    tree.nodes().skip(start).map(|(id, _)| id)
}

/// `start`'tan sonra eklenen düğümlerden `Click` eylemini kaldırır: AT'ye hiç sunulmaz (Orca
/// "düğme" diye okur ama etkinleştirme gerçek girdiyle).
fn strip_click_since(tree: &mut AccessTree, start: usize) {
    let ids: Vec<NodeId> = added_since(tree, start).collect();
    for id in ids {
        if let Some(mut n) = tree.get(id) {
            n.node_mut().remove_action(AxAction::Click);
        }
    }
}

impl FileChooser {
    /// Ağacı kurar (son çizimin yerleşimiyle) ve AT tıklamasına izinli düğümlerin noktalarını
    /// saklar (izin listesi dışındaki düğüme AT tıklaması gitmez).
    pub(super) fn access(&mut self, tree: &mut AccessTree, theme: &Theme) {
        tree.root().label(&self.title());
        self.access_focus.clear();
        let allowed = self.build_access(tree, theme);
        self.access_points = tree
            .nodes()
            .filter(|(id, n)| n.supports_action(AxAction::Click) && allowed.contains(id))
            .filter_map(|(id, n)| n.bounds().map(|b| (id, Point::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))))
            .collect();
    }

    /// Ağacı kurar; AT `Click`'ine izinli düğümleri döndürür (yalnız gezinme ve seçme).
    fn build_access(&mut self, tree: &mut AccessTree, theme: &Theme) -> std::collections::HashSet<NodeId> {
        let mut allowed = std::collections::HashSet::new();
        let Some(w) = self.w.as_ref() else { return allowed };
        let mut focus_targets = Vec::new();
        if self.modal != Modal::None {
            // Diyalog düğmeleri (üzerine yazma onayı, yeni klasör oluşturma) AT'ye kapalı: izinli
            // düğüm yok. Diyalogun alanı (yeni klasör adı) zaten odakta.
            let modal = self.modal.clone();
            let start = tree.len();
            w.dialog.access(tree, "dialog", |tree| match modal {
                Modal::NewFolder => {
                    w.folder_name.access(tree, "folder-name").label(&t!("shell-filechooser-new-folder-placeholder"));
                }
                _ => {
                    w.dialog_body.access(tree, "body", w.dialog.content_rect(theme));
                }
            });
            strip_click_since(tree, start);
            return allowed;
        }

        let save = self.req().mode == Mode::Save;
        let toolbar_bounds = [w.back.rect(), w.up.rect(), w.path.rect(), w.hidden.rect(), w.search.rect()].into_iter().reduce(|a, b| a.union(&b));
        let start = tree.len();
        let toolbar = tree
            .push("toolbar", Role::Toolbar)
            .label(&t!("shell-a11y-filechooser-toolbar"))
            .bounds(toolbar_bounds.unwrap_or_default())
            .id();
        let mut new_folder = None;
        tree.within(toolbar, |tree| {
            w.back.access(tree, "back").label(&t!("shell-a11y-filechooser-back")).enabled(!self.back_stack.is_empty());
            w.up.access(tree, "up").label(&t!("shell-a11y-filechooser-up"));
            w.path
                .access(tree, "path", self.focus == Focus::Location, &t!("shell-a11y-filechooser-parents"))
                .label(&t!("shell-a11y-filechooser-location"));
            w.hidden.access(tree, "hidden").label(&t!("shell-a11y-filechooser-hidden"));
            if self.can_create_folder() {
                new_folder = Some(w.new_folder.access(tree, "new-folder").label(&t!("shell-filechooser-new-folder")).id());
            }
            let search = w.search.input().access(tree, "search").label(&t!("shell-filechooser-search")).id();
            focus_targets.push((search, Focus::Search));
        });
        // Araç çubuğu gezinmedir; yeni klasör dosya sistemine yazar (AT'ye kapalı, eylemsiz).
        allowed.extend(added_since(tree, start).filter(|id| Some(*id) != new_folder));
        if let Some(mut n) = new_folder.and_then(|id| tree.get(id)) {
            n.node_mut().remove_action(AxAction::Click);
        }

        let start = tree.len();
        w.places.access(tree, "places", |i| i).label(&t!("shell-a11y-filechooser-places"));
        if !self.visible.is_empty() {
            let key = |row: usize| self.entry_at(row).map(|e| e.name.clone());
            w.files.access(tree, "files", key).label(&t!("shell-a11y-filechooser-files"));
        } else if !matches!(self.state, super::DirState::Loading) {
            w.empty.access(tree, "empty");
        }
        if save {
            let name = w.name.access(tree, "name").label(w.name_label.text()).id();
            focus_targets.push((name, Focus::Name));
        }
        if let Some(f) = &w.filter {
            f.access(tree, "filter").label(&t!("shell-a11y-filechooser-filter"));
        }
        for (i, (c, label)) in w.choices.iter().zip(&w.choice_labels).enumerate() {
            match c {
                ChoiceWidget::Toggle(chip) => {
                    tree.push(("choice", i), Role::CheckBox).label(chip.label()).toggled(chip.is_selected()).bounds(chip.rect()).clickable();
                }
                ChoiceWidget::List(sel) => {
                    sel.access(tree, ("choice", i)).label(label.text());
                }
            }
        }
        // Yerler, dosya satırları (seçer), ad alanı (odaklar), filtre ve seçim kutuları.
        allowed.extend(added_since(tree, start));
        if !w.status.text().is_empty() {
            tree.push("status", Role::Label).value(w.status.text()).live(Live::Polite);
        }
        allowed.insert(w.cancel.access(tree, "cancel").id());
        // Kabul (Aç/Kaydet/Seç) erişim verir: AT'ye eylemsiz, yalnız okunur.
        let mut accept = w.accept.access(tree, "accept");
        accept.node_mut().remove_action(AxAction::Click);
        self.access_focus = focus_targets;
        allowed
    }

    /// AT eylemi: `Focus` metin alanını odaklar; `Click` yalnız izin listesindeki düğümde, düğümün
    /// ortasına işaretçi tıklaması. Diğer her şey (kabul, diyalog düğmeleri, yeni klasör, metin
    /// yazma) reddedilir.
    pub(super) fn access_action(&mut self, cx: &mut Context<Self>, surface: SurfaceId, request: AccessRequest) {
        if self.done {
            return;
        }
        if request.action == AccessAction::Focus {
            if let Some(&(_, focus)) = self.access_focus.iter().find(|(id, _)| *id == request.node)
                && focus != self.focus
                && self.modal == Modal::None
            {
                self.sync_focus(cx, focus);
            }
            return;
        }
        if request.action != AccessAction::Click {
            return;
        }
        let Some(&at) = self.access_points.get(&request.node) else {
            // Değişmez 9: düğüm içeriği yok, yalnız eylem türü.
            tracing::info!(action = "click", "dosya seçici: AT eylemi reddedildi (erişim veren ya da izin listesi dışı düğüm)");
            return;
        };
        // Çift tık algısı görünümde (framework `List`/`FileGrid`): AT tıklamasından sonra
        // [`ASSISTIVE_ACTIVATION_GUARD`] boyunca işaretçi çift tıkı satırı açmaz/kabul etmez.
        self.assistive_click = Some(Instant::now());
        let left = MouseButton::Left;
        for kind in [PointerKind::Motion, PointerKind::Press(left), PointerKind::Release(left), PointerKind::Leave] {
            if self.done {
                return;
            }
            self.pointer_event(cx, surface, &PointerEvent::new(at, kind));
        }
        self.assistive_click = Some(Instant::now());
    }
}

/// AT tıklamasından sonra işaretçi çift tıkının (satırı açma; çoklu seçimde kabul) yok sayıldığı
/// süre. Framework'ün çift tık aralığından (400 ms) uzun: çift tıkın iki yarısından biri AT ise
/// etkinleştirme olmaz. Gerçek Enter etkilenmez.
pub(super) const ASSISTIVE_ACTIVATION_GUARD: Duration = Duration::from_secs(1);

/// İşaretçi etkinleştirmesi AT tıklamasına bağlı olabilir mi.
pub(super) fn assistive_guard(last_assistive: Option<Instant>, now: Instant) -> bool {
    last_assistive.is_some_and(|t| now.saturating_duration_since(t) < ASSISTIVE_ACTIVATION_GUARD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistive_click_blocks_pointer_activation_for_a_double_click_window() {
        let t = Instant::now();
        assert!(!assistive_guard(None, t), "AT tıklaması yok");
        assert!(assistive_guard(Some(t), t));
        assert!(assistive_guard(Some(t), t + Duration::from_millis(399)), "framework çift tık aralığı içinde");
        assert!(assistive_guard(Some(t), t + Duration::from_millis(999)));
        assert!(!assistive_guard(Some(t), t + ASSISTIVE_ACTIVATION_GUARD));
    }
}
