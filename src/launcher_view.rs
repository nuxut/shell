//! Uygulama başlatıcısı paneli: arama alanı + sanal sonuç listesi (`ResultList`) + masaüstü
//! eylemleri alt sayfası (`PageStack`, Menu ile aynı morf/kayma dili).
//!
//! Panelin kendisi bar kenarından büyüyen bir [`crate::sheet::Sheet`]'tir (`Screen`). Bu modül
//! yalnızca içeriği bilir: katalog ve kullanım kayıtları modelden (`Arc`, servis thread'inden
//! gelir), arama UI thread'inde senkron (`Searcher`, tuş başına < 2 ms).
//!
//! Klavye: yazmak arar; ↑ ↓ PgUp PgDn Home End seçer; Enter başlatır; → (imleç metnin
//! sonundayken) eylemler sayfasını açar, ← / Backspace / Esc geri döner; Esc önce metni
//! temizler, boşsa paneli kapatır.
//!
//! İkonlar: görünür olabilecek satırların ikon adları [`LauncherView::take_icon_requests`] ile
//! App'e verilir (nuxut-image Loader, arka planda tema araması + çözme), sonuç
//! [`LauncherView::icon_loaded`] ile gelir; doku çizimde yüklenir. Önbellek sınırlı.
//!
//! Pano kipi ([`LauncherMode::Clipboard`], `msg launcher clipboard`): aynı panel ve satırlar
//! pano geçmişini listeler (metnin ilk satırı ya da görsel küçük resmi; alt metinde göreli zaman
//! ve boyut). Arama metin öğelerini süzer. Enter/tık öğeyi panoya koyar ve paneli kapatır
//! (otomatik yapıştırma yok); Delete (imleç metnin sonundayken) seçili öğeyi siler; arama
//! alanının yanındaki "Geçmişi temizle" iki adımlı onay ister (framework `ConfirmButton`).
//! Küçük resimler ikonlarla aynı önbellekte, [`THUMB_PREFIX`]'li anahtarla; çözme App'te
//! (görsel yalıtımı, `Untrusted`).
//!
//! Boş liste: maskot Fiyu (96) + metin, liste alanında ortalı; ruh hâli [`empty_mood`]
//! (`docs/design/mascot-placement.md`); framework `EmptyState`. Maskot gizliyse ya da sığmıyorsa
//! yer kaplamaz: metin liste alanında kendi başına ortalı.
//!
//! Emoji kipi ([`LauncherMode::Emoji`], `msg launcher emoji`; `docs/design/emoji.md`): liste
//! alanında framework `GlyphGrid` + seçili emojinin ad satırı ([`EmojiView`]). Gezinme ve Enter
//! ızgaraya, yazmak arama alanına. Enter/tık emojiyi panoya koyar, sık kullanılanlara kaydeder
//! ve paneli kapatır (odaktaki pencereye yazma yok).
//!
//! Gizlilik: arama metni, uygulama adları, komutlar ve pano içeriği loglanmaz.

mod access;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use nuxut::render::{Texture, TextureOptions};
use nuxut::i18n::t;
use nuxut::ui::{
    ButtonVariant, ConfirmButton, ConfirmEvent, EmptyState, Icon, IconName, Mood, PageHeader, PageStack, ResultEvent, ResultItem, ResultList,
    SearchField, TextInputEvent, Ui,
};
use nuxut::{CursorShape, Key, KeyEvent, KeyState, Painter, Point, PointerEvent, Rect, Size, TextSystem, Theme};

use crate::services::clipboard::{ClipboardCommand, ClipboardSnapshot, ItemId};
use crate::emoji_view::EmojiView;
use crate::services::launcher::{Catalog, DesktopEntry, Hit, LauncherConfig, Searcher, Usage, calc};
use crate::widgets::CopyText;

/// Boş olmayan sorguda gösterilen en çok sonuç (vurgu yalnızca bunlar için hesaplanır).
pub const MAX_RESULTS: usize = 100;
/// Bir seferde istenen ikon sayısı (seçimin çevresi).
const ICON_WINDOW: usize = 24;
/// Tutulan en çok ikon (çözülmüş görüntü ya da doku).
const MAX_ICONS: usize = 384;
/// İkon mantıksal boyutu ve çözme ölçeği.
pub const ICON_SIZE: u32 = 32;
pub const ICON_SCALE: u32 = 2;

/// Panelden çıkan istekler (`Screen` → `App`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LauncherOutcome {
    /// Uygulamayı (ya da eylemini) başlat ve paneli kapat.
    Launch { entry: Arc<DesktopEntry>, action: Option<String> },
    /// Paneli kapat.
    Close,
    /// Arama alanından kopyala/kes: metin panoya.
    Copy(CopyText),
    /// Arama alanında Ctrl+V: pano okunur, sonuç [`LauncherView::paste`] ile gelir.
    RequestPaste,
    /// Pano kipi: öğeyi panoya koy (paneli kapatır), sil ya da geçmişi temizle.
    Clipboard(ClipboardCommand),
    /// Emoji kipi: emoji panoya (paneli kapatır, sık kullanılanlara kaydedilir).
    Emoji(&'static str),
}

/// Başlatıcının içeriği.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LauncherMode {
    #[default]
    Apps,
    Clipboard,
    /// Emoji seçici (`msg launcher emoji`).
    Emoji,
}

/// Pano küçük resmi anahtarlarının öneki (ikon adlarıyla çakışmaz: kontrol karakteri).
pub const THUMB_PREFIX: &str = "\u{1}clip/";
/// Küçük resmin en uzun kenarı (fiziksel piksel).
pub const THUMB_SIDE: u32 = 96;

fn thumb_key(id: ItemId) -> String {
    format!("{THUMB_PREFIX}{id}")
}

fn thumb_id(key: &str) -> Option<ItemId> {
    key.strip_prefix(THUMB_PREFIX)?.parse().ok()
}

/// Çözülecek küçük resim (App görsel yükleyicisine verir; sonuç [`LauncherView::icon_loaded`]).
#[derive(Clone)]
pub struct ThumbRequest {
    pub key: String,
    pub data: Arc<[u8]>,
}

impl std::fmt::Debug for ThumbRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ThumbRequest({} bayt)", self.data.len())
    }
}

/// Metin aramasında bakılan en çok bayt (öğe başına).
const CLIP_SEARCH_BYTES: usize = 16 << 10;

/// Büyük/küçük harf duyarsız alt dizgi (`needle` küçük harfe çevrilmiş). Ayırmasız.
fn contains_ci(hay: &str, needle: &[char]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut end = hay.len().min(CLIP_SEARCH_BYTES);
    while !hay.is_char_boundary(end) {
        end -= 1;
    }
    let hay = &hay[..end];
    hay.char_indices().any(|(i, _)| {
        let mut rest = hay[i..].chars().flat_map(char::to_lowercase);
        needle.iter().all(|n| rest.next() == Some(*n))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Results,
    Actions,
}

/// Sonuç listesindeki bir satırın anlamı.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Row {
    Calc(f64),
    App(u32),
    Clip(ItemId),
}

#[derive(Debug)]
enum IconSlot {
    Pending,
    Missing,
    Image(nuxut_image::Image),
    Ready(Texture),
}

#[derive(Debug)]
pub struct LauncherView {
    search: SearchField,
    list: ResultList,
    actions: ResultList,
    header: PageHeader,
    pages: PageStack<Page>,
    /// Boş liste (framework `EmptyState`): maskot, ruh hâli [`empty_mood`]; gizliyse ya da
    /// sığmıyorsa yalnız metin.
    empty: EmptyState,
    /// Son verilen boş liste ruh hâli.
    empty_mood: Mood,
    searcher: Searcher,
    /// Emoji kipinin ızgarası ve ad satırı.
    emoji: EmojiView,
    catalog: Arc<Catalog>,
    usage: Arc<Usage>,
    rows: Vec<Row>,
    /// Liste satırı → ikon adı (dokusu gelince `set_item_icon`).
    row_icons: Vec<Option<String>>,
    /// Eylem sayfasındaki girdi (katalog indeksi).
    action_entry: Option<usize>,
    icons: HashMap<String, IconSlot>,
    icon_requests: Vec<String>,
    /// Doku bekleyen adlar (görüntü geldi, çizimde yüklenecek).
    icons_arrived: HashSet<String>,
    config: LauncherConfig,
    rect: Rect,
    decimal: char,
    open: bool,
    /// Kapandı ama yaprak henüz çökmedi: sorgu ve sonuçlar kapanış animasyonu bitince
    /// ([`Self::release`]) bırakılır.
    release_pending: bool,
    mode: LauncherMode,
    /// Kapalıyken istenen kip: bir sonraki açılışta uygulanır (yoksa uygulamalar).
    next_mode: Option<LauncherMode>,
    clipboard: Arc<ClipboardSnapshot>,
    /// Pano kipinde arama alanının yanındaki "Geçmişi temizle".
    clear: ConfirmButton,
    thumb_requests: Vec<ThumbRequest>,
}

/// Boş sonuç listesinin maskot ruh hâli (docs/design/mascot-placement.md): uygulama araması,
/// emoji araması ve pano süzgecinde eşleşme yok → `NoResults`; pano geçmişi boş ya da kapalı →
/// `Empty`; pano izleyicisi yok (protokol desteklenmiyor) → `Error`.
fn empty_mood(mode: LauncherMode, clipboard: &ClipboardSnapshot) -> Mood {
    match mode {
        LauncherMode::Apps | LauncherMode::Emoji => Mood::NoResults,
        LauncherMode::Clipboard if !clipboard.enabled => Mood::Empty,
        LauncherMode::Clipboard if !clipboard.available => Mood::Error,
        LauncherMode::Clipboard if clipboard.items.is_empty() => Mood::Empty,
        LauncherMode::Clipboard => Mood::NoResults,
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl LauncherView {
    pub fn new(theme: &Theme, config: &LauncherConfig) -> Self {
        // Ondalık ayırıcı yerel ayardan (LC_NUMERIC); boşsa nokta.
        let decimal = nuxut::locale::Locale::numeric_from_env().decimal_separator().chars().next().unwrap_or('.');
        Self {
            search: SearchField::new(theme, "Uygulama ara…"),
            list: ResultList::new(theme),
            actions: ResultList::new(theme),
            header: PageHeader::new(theme, ""),
            pages: PageStack::new(theme, Page::Results, Size::ZERO),
            empty: EmptyState::new(theme, t!("shell-launcher-no-results")).with_mascot(theme, Mood::NoResults),
            empty_mood: Mood::NoResults,
            searcher: Searcher::new(),
            emoji: EmojiView::new(theme),
            catalog: Arc::default(),
            usage: Arc::default(),
            rows: Vec::new(),
            row_icons: Vec::new(),
            action_entry: None,
            icons: HashMap::new(),
            icon_requests: Vec::new(),
            icons_arrived: HashSet::new(),
            config: config.clone(),
            rect: Rect::ZERO,
            decimal,
            open: false,
            release_pending: false,
            mode: LauncherMode::Apps,
            next_mode: None,
            clipboard: Arc::default(),
            clear: ConfirmButton::new(theme, ButtonVariant::Text, Icon::named(IconName::Delete), t!("shell-launcher-clipboard-clear-armed"))
                .with_label(theme, t!("shell-launcher-clipboard-clear")),
            thumb_requests: Vec::new(),
        }
    }

    /// Açıkken içerik kipi; kapalıysa `None`.
    pub fn mode(&self) -> Option<LauncherMode> {
        self.open.then_some(self.mode)
    }

    /// Kipi ister: açıksa hemen geçer (arama sıfırlanır), kapalıysa bir sonraki açılışta.
    /// Yeniden çizim gerekiyorsa `true`.
    pub fn request_mode(&mut self, theme: &Theme, mode: LauncherMode, now: Instant) -> bool {
        if !self.open {
            self.next_mode = Some(mode);
            return false;
        }
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        self.reset(theme, now);
        true
    }

    fn placeholder(mode: LauncherMode) -> String {
        match mode {
            LauncherMode::Apps => t!("shell-launcher-search-placeholder"),
            LauncherMode::Clipboard => t!("shell-launcher-clipboard-placeholder"),
            LauncherMode::Emoji => t!("shell-launcher-emoji-placeholder"),
        }
    }

    /// Arama, alt sayfa ve onay durumu sıfırlanır; sonuçlar kurulur.
    fn reset(&mut self, theme: &Theme, now: Instant) {
        self.search.set_placeholder(&Self::placeholder(self.mode));
        self.search.input_mut().set_text("");
        self.search.set_focused(true, now);
        while self.pages.depth() > 0 {
            self.pages.pop(self.content_size(theme), now);
        }
        self.action_entry = None;
        self.clear.cancel(now);
        self.emoji.forget_position();
        self.refresh(theme, now);
    }

    /// Pano geçmişi değişti. Pano kipinde açıksa liste yenilenir; yeniden çizim gerekiyorsa `true`.
    pub fn set_clipboard(&mut self, theme: &Theme, snapshot: &Arc<ClipboardSnapshot>, now: Instant) -> bool {
        if Arc::ptr_eq(&self.clipboard, snapshot) {
            return false;
        }
        self.clipboard = snapshot.clone();
        // Artık olmayan öğelerin küçük resimleri bırakılır (önbellek sınırı uygulamalarla ortak).
        let alive = |k: &String| thumb_id(k).is_none_or(|id| snapshot.get(id).is_some());
        self.icons.retain(|k, _| alive(k));
        self.icons_arrived.retain(|k| alive(k));
        self.thumb_requests.retain(|r| alive(&r.key));
        if !(self.open && self.mode == LauncherMode::Clipboard) {
            return false;
        }
        let selected = self.list.selected();
        self.refresh(theme, now);
        if let Some(i) = selected.filter(|&i| i < self.list.len()) {
            self.list.set_selected(theme, i, now);
        }
        true
    }

    /// İstenen küçük resimler (App yalıtılmış çözücüye verir).
    pub fn take_thumb_requests(&mut self) -> Vec<ThumbRequest> {
        std::mem::take(&mut self.thumb_requests)
    }

    pub fn set_config(&mut self, config: &LauncherConfig) {
        self.config = config.clone();
    }

    /// İçerik boyutu (iç boşluk hariç): arama alanı + aralık + `rows` satır.
    pub fn content_size(&self, theme: &Theme) -> Size {
        let field = self.search.preferred_size(theme, self.config.width).height;
        let h = field + theme.space(3) + self.config.rows as f32 * ResultList::row_height(theme);
        Size::new(self.config.width, h)
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.search.set_theme(theme, now);
        self.list.set_theme(theme, now);
        self.actions.set_theme(theme, now);
        self.header.set_theme(theme, now);
        self.pages.set_theme(theme, now);
        self.empty.set_theme(theme, now);
        self.clear.set_theme(theme, now);
        self.emoji.set_theme(theme, now);
    }

    /// Model verisi (katalog/kullanım) değişti. Açıkken sonuçlar yenilenir; yeniden çizim
    /// gerekiyorsa `true`.
    pub fn set_data(&mut self, theme: &Theme, catalog: &Arc<Catalog>, usage: &Arc<Usage>, now: Instant) -> bool {
        let changed = !Arc::ptr_eq(&self.catalog, catalog) || !Arc::ptr_eq(&self.usage, usage);
        if !changed {
            return false;
        }
        let catalog_changed = !Arc::ptr_eq(&self.catalog, catalog);
        let usage_changed = !Arc::ptr_eq(&self.usage, usage);
        self.catalog = catalog.clone();
        self.usage = usage.clone();
        if catalog_changed && self.pages.depth() > 0 {
            // Eylem sayfasındaki girdi kayboldu/değişti olabilir: sonuçlara dön.
            self.pages.pop(self.content_size(theme), now);
            self.action_entry = None;
        }
        // Emoji kipi uygulama kataloğuna bakmaz: yalnız kullanım (sık kullanılanlar) değişince
        // yenilenir; katalog taraması (arka planda uygulama kurulması) ızgaranın seçimini ve
        // kaydırmasını sıfırlamaz (inceleme şartı, 2026-10-14).
        let refresh = match self.mode {
            LauncherMode::Apps => true,
            LauncherMode::Emoji => usage_changed,
            LauncherMode::Clipboard => false,
        };
        if self.open && refresh {
            self.refresh(theme, now);
            return true;
        }
        false
    }

    /// Panel açıldı: istenen kip (yoksa uygulamalar), arama sıfırlanır, alan odaklanır.
    pub fn opened(&mut self, theme: &Theme, now: Instant) {
        self.open = true;
        self.release_pending = false;
        self.mode = self.next_mode.take().unwrap_or_default();
        self.reset(theme, now);
    }

    /// Panel kapanmaya başladı: odak ve onay bekleyen düğme bırakılır; içerik kapanış
    /// animasyonu boyunca olduğu gibi kalır. Sorgu ve sonuçlar yaprak çöktüğünde
    /// [`Self::release`] ile temizlenir. Eskiden burada temizleniyordu: kapanan yaprakta sonuç
    /// listesinin üstüne "Sonuç yok" boş durumu biniyordu (2026-10-17, kare denetimi).
    pub fn closed(&mut self, now: Instant) {
        self.open = false;
        self.next_mode = None;
        self.search.set_focused(false, now);
        self.clear.cancel(now);
        self.release_pending = true;
    }

    /// Yaprak tamamen çöktü: sorgu bellekte tutulmaz, sonuçlar ve bekleyen ikon istekleri
    /// bırakılır. Kapanmamışsa ya da zaten bırakılmışsa bir şey yapmaz.
    pub fn release(&mut self) {
        if self.open || !std::mem::take(&mut self.release_pending) {
            return;
        }
        self.search.input_mut().set_text("");
        self.rows.clear();
        self.row_icons.clear();
        self.icon_requests.clear();
        self.thumb_requests.clear();
    }

    /// Sorguyu yeniden çalıştırır ve listeyi kurar.
    fn refresh(&mut self, theme: &Theme, now: Instant) {
        match self.mode {
            LauncherMode::Clipboard => return self.refresh_clipboard(theme, now),
            LauncherMode::Emoji => return self.refresh_emoji(theme, now),
            LauncherMode::Apps => {}
        }
        self.empty.set_title(&t!("shell-launcher-no-results"));
        self.set_empty_mood(empty_mood(self.mode, &self.clipboard));
        let query = self.search.text().to_owned();
        let calc = self.config.calculator.then(|| calc::evaluate(&query)).flatten();
        let hits: Vec<Hit> = self.searcher.search(&self.catalog, &self.usage, &query, now_unix()).to_vec();
        let limit = if query.trim().is_empty() { hits.len() } else { hits.len().min(MAX_RESULTS) };
        let mut items = Vec::with_capacity(limit + 1);
        self.rows.clear();
        self.row_icons.clear();
        if let Some(v) = calc {
            let text = calc::format(v, self.decimal);
            let hint = nuxut::i18n::current().attr("shell-launcher-calc-result", "description", None).map(|s| s.into_owned()).unwrap_or_default();
            items.push(ResultItem::new(t!("shell-launcher-calc-result", result = text.as_str())).subtitle(hint).icon(Icon::named(IconName::Code)));
            self.rows.push(Row::Calc(v));
            self.row_icons.push(None);
        }
        let highlight = !query.trim().is_empty();
        for hit in &hits[..limit] {
            let i = hit.index as usize;
            let Some(app) = self.catalog.apps.get(i) else { continue };
            let e = &app.entry;
            let mut item = ResultItem::new(e.name.clone());
            if let Some(sub) = e.generic_name.as_ref().or(e.comment.as_ref()) {
                item = item.subtitle(sub.clone());
            }
            if highlight {
                item = item.match_chars(self.searcher.name_highlight(&self.catalog, i));
            }
            item = item.icon(self.icon_for(e.icon.as_deref(), IconName::Apps));
            items.push(item);
            self.rows.push(Row::App(hit.index));
            self.row_icons.push(e.icon.clone());
        }
        self.list.set_items(theme, items, now);
        self.request_icons_around(0);
    }

    /// Pano kipinin listesi: en yeni başta; sorgu metin öğelerini süzer.
    fn refresh_clipboard(&mut self, theme: &Theme, now: Instant) {
        let snapshot = self.clipboard.clone();
        let needle: Vec<char> = self.search.text().trim().chars().flat_map(char::to_lowercase).collect();
        let wall = SystemTime::now();
        let mut items = Vec::new();
        self.rows.clear();
        self.row_icons.clear();
        if snapshot.enabled && snapshot.available {
            for item in &snapshot.items {
                if !needle.is_empty() && !item.content.as_text().is_some_and(|t| contains_ci(t, &needle)) {
                    continue;
                }
                let meta = t!(
                    "shell-launcher-clipboard-meta",
                    age = crate::widgets::relative_time(item.copied_at, wall),
                    size = crate::services::sysmon::format::bytes(item.content.len() as u64)
                );
                let (row, key) = if item.is_image() {
                    let key = thumb_key(item.id);
                    let icon = self.icon_for(Some(&key), IconName::Image);
                    (ResultItem::new(t!("shell-launcher-clipboard-image")).icon(icon), Some(key))
                } else {
                    (ResultItem::new(item.preview()).icon(Icon::named(IconName::Paste)), None)
                };
                items.push(row.subtitle(meta));
                self.rows.push(Row::Clip(item.id));
                self.row_icons.push(key);
            }
        }
        let empty = if !snapshot.enabled {
            t!("shell-launcher-clipboard-disabled")
        } else if !snapshot.available {
            t!("shell-launcher-clipboard-unavailable")
        } else if snapshot.items.is_empty() {
            t!("shell-launcher-clipboard-empty")
        } else {
            t!("shell-launcher-no-results")
        };
        self.empty.set_title(&empty);
        self.set_empty_mood(empty_mood(self.mode, &snapshot));
        self.clear.set_enabled(!snapshot.items.is_empty(), now);
        self.list.set_items(theme, items, now);
        self.request_icons_around(0);
    }

    /// Emoji kipi: liste satırı yok, içerik [`EmojiView`] ızgarasında (aynı alan).
    fn refresh_emoji(&mut self, theme: &Theme, now: Instant) {
        self.empty.set_title(&t!("shell-launcher-no-results"));
        self.set_empty_mood(empty_mood(self.mode, &self.clipboard));
        self.rows.clear();
        self.row_icons.clear();
        self.list.set_items(theme, Vec::new(), now);
        let query = self.search.text().to_owned();
        self.emoji.refresh(theme, &self.usage, &query, now);
    }

    /// Önbellekteki doku ya da yer tutucu.
    fn icon_for(&self, name: Option<&str>, fallback: IconName) -> Icon {
        match name.and_then(|n| self.icons.get(n)) {
            Some(IconSlot::Ready(t)) => Icon::image(t.clone()),
            _ => Icon::named(fallback),
        }
    }

    /// `center` çevresindeki satırların eksik ikonlarını (ve pano küçük resimlerini) ister.
    fn request_icons_around(&mut self, center: usize) {
        let start = center.saturating_sub(ICON_WINDOW / 2);
        let end = (start + ICON_WINDOW).min(self.row_icons.len());
        for name in self.row_icons[start..end].iter().flatten() {
            if self.icons.contains_key(name) || self.icons.len() >= MAX_ICONS {
                continue;
            }
            match thumb_id(name) {
                Some(id) => {
                    let Some(item) = self.clipboard.get(id) else { continue };
                    self.thumb_requests.push(ThumbRequest { key: name.clone(), data: item.content.blob().shared() });
                }
                None => self.icon_requests.push(name.clone()),
            }
            self.icons.insert(name.clone(), IconSlot::Pending);
        }
    }

    /// İstenen ikon adları (App bunları yükleyiciye verir).
    pub fn take_icon_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.icon_requests)
    }

    /// Yükleyiciden sonuç. Görünür bir satırı etkiliyorsa `true` (kare iste).
    pub fn icon_loaded(&mut self, name: &str, image: Option<nuxut_image::Image>) -> bool {
        let Some(slot) = self.icons.get_mut(name) else { return false };
        *slot = match image {
            Some(img) => {
                self.icons_arrived.insert(name.to_owned());
                IconSlot::Image(img)
            }
            None => IconSlot::Missing,
        };
        self.open && self.row_icons.iter().flatten().any(|n| n == name)
    }

    /// Gelen görüntüleri dokuya yükler ve satırlara verir (yalnızca çizimde: painter gerekir).
    fn upload_icons(&mut self, painter: &mut Painter) {
        if self.icons_arrived.is_empty() {
            return;
        }
        for name in std::mem::take(&mut self.icons_arrived) {
            let Some(slot) = self.icons.get_mut(&name) else { continue };
            let IconSlot::Image(img) = std::mem::replace(slot, IconSlot::Missing) else { continue };
            let (w, h) = img.size();
            let data = nuxut::render::ImageData { width: w, height: h, pixels: img.into_pixels().into() };
            let texture = painter.upload_texture(data, TextureOptions::default());
            *slot = IconSlot::Ready(texture.clone());
            for (i, n) in self.row_icons.iter().enumerate() {
                if n.as_deref() == Some(name.as_str()) {
                    self.list.set_item_icon(i, Some(Icon::image(texture.clone())));
                }
            }
        }
    }

    fn entry_at(&self, row: usize) -> Option<Arc<DesktopEntry>> {
        match self.rows.get(row)? {
            Row::App(i) => self.catalog.apps.get(*i as usize).map(|a| a.entry.clone()),
            Row::Calc(_) | Row::Clip(_) => None,
        }
    }

    /// Seçili pano öğesi.
    fn selected_clip(&self) -> Option<ItemId> {
        match self.rows.get(self.list.selected()?)? {
            Row::Clip(id) => Some(*id),
            _ => None,
        }
    }

    fn activate(&mut self, theme: &Theme, row: usize, now: Instant) -> Option<LauncherOutcome> {
        match *self.rows.get(row)? {
            Row::Calc(v) => {
                // Pano henüz yok: sonuç arama alanına yazılır (zincirleme hesap).
                let text = calc::format(v, self.decimal);
                self.search.input_mut().set_text(&text);
                self.refresh(theme, now);
                None
            }
            Row::App(_) => self.entry_at(row).map(|entry| LauncherOutcome::Launch { entry, action: None }),
            Row::Clip(id) => Some(LauncherOutcome::Clipboard(ClipboardCommand::Pick(id))),
        }
    }

    /// Eylemler sayfasını açar (girdinin eylemi yoksa hiçbir şey).
    fn open_actions(&mut self, theme: &Theme, row: usize, now: Instant) -> bool {
        let Some(entry) = self.entry_at(row) else { return false };
        if entry.actions.is_empty() {
            return false;
        }
        let Row::App(index) = self.rows[row] else { return false };
        self.action_entry = Some(index as usize);
        self.header.set_title(&entry.name);
        let items = self.action_items(&entry);
        self.actions.set_items(theme, items, now);
        self.actions.set_selected(theme, 0, now);
        self.pages.push(Page::Actions, self.content_size(theme), now);
        true
    }

    /// Eylemler sayfasının satırları: "Aç" ve girdinin eylemleri.
    fn action_items(&self, entry: &DesktopEntry) -> Vec<ResultItem> {
        let open_icon = self.icon_for(entry.icon.as_deref(), IconName::Apps);
        let mut items = vec![ResultItem::new(t!("shell-launcher-action-open")).icon(open_icon)];
        for a in &entry.actions {
            items.push(ResultItem::new(a.name.clone()).icon(self.icon_for(a.icon.as_deref().or(entry.icon.as_deref()), IconName::Apps)));
        }
        items
    }

    /// Dil değişti: arama alanı ve boş liste metni kurucudaki kaynaktan yeniden alınır
    /// (yazılan sorgu ve odak korunur); açıksa sonuçlar ve eylemler sayfası yeni metinlerle
    /// yeniden kurulur, seçim yerinde kalır.
    pub fn relocalize(&mut self, theme: &Theme, now: Instant) {
        let fresh = Self::new(theme, &self.config);
        let query = self.search.text().to_owned();
        let focused = self.search.input().is_focused();
        self.search = fresh.search;
        self.search.set_placeholder(&Self::placeholder(self.mode));
        self.search.input_mut().set_text(&query);
        self.search.set_focused(focused, now);
        self.empty = fresh.empty;
        self.empty.set_mood(self.empty_mood);
        self.clear = fresh.clear;
        self.clear.set_enabled(!self.clipboard.items.is_empty(), now);
        if !self.open {
            return;
        }
        let selected = self.list.selected();
        self.refresh(theme, now);
        if let Some(i) = selected.filter(|&i| i < self.list.len()) {
            self.list.set_selected(theme, i, now);
        }
        let entry = self.action_entry.and_then(|i| self.catalog.apps.get(i)).map(|a| a.entry.clone());
        if let Some(entry) = entry {
            let selected = self.actions.selected();
            let items = self.action_items(&entry);
            self.actions.set_items(theme, items, now);
            self.actions.set_selected(theme, selected.unwrap_or(0).min(self.actions.len().saturating_sub(1)), now);
        }
    }

    fn close_actions(&mut self, theme: &Theme, now: Instant) {
        self.pages.pop(self.content_size(theme), now);
        self.action_entry = None;
    }

    fn launch_action(&self, i: usize) -> Option<LauncherOutcome> {
        let entry = self.catalog.apps.get(self.action_entry?)?.entry.clone();
        let action = match i {
            0 => None,
            n => Some(entry.actions.get(n - 1)?.id.clone()),
        };
        Some(LauncherOutcome::Launch { entry, action })
    }

    /// Klavye. Döner: (yeniden çizim, çıktı).
    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> (bool, Option<LauncherOutcome>) {
        if *self.pages.current() == Page::Actions {
            if event.state != KeyState::Released && event.key == Key::Escape {
                self.close_actions(theme, now);
                return (true, None);
            }
            if self.header.key(event, now).event.is_some() {
                self.close_actions(theme, now);
                return (true, None);
            }
            let r = self.actions.key(theme, event, now);
            return match r.event {
                Some(ResultEvent::Activated(i)) => (true, self.launch_action(i)),
                _ => (r.repaint, None),
            };
        }

        let pressed = event.state != KeyState::Released;
        let caret_at_end = {
            let (caret, anchor) = self.search.input().caret();
            caret == anchor && caret == self.search.text().len()
        };
        if self.mode == LauncherMode::Clipboard {
            // Onay bekleyen "Geçmişi temizle": Esc yalnızca onu bırakır.
            if pressed && event.key == Key::Escape && self.clear.cancel(now) {
                return (true, None);
            }
            // Delete: ileri silinecek metin yoksa seçili öğeyi siler.
            if event.key == Key::Delete && caret_at_end {
                let outcome = pressed.then(|| self.selected_clip()).flatten();
                return (outcome.is_some(), outcome.map(|id| LauncherOutcome::Clipboard(ClipboardCommand::Delete(id))));
            }
        }
        // Emoji kipi: gezinme ve Enter ızgaraya (Ctrl/Shift'li ok, arama alanında seçim için
        // alana kalır); harfler, Space, Backspace arama alanına.
        if self.mode == LauncherMode::Emoji
            && EmojiView::wants_key(event)
            && !(event.modifiers.ctrl || event.modifiers.shift)
        {
            let (repaint, picked) = self.emoji.key(event, now);
            return (repaint, picked.map(LauncherOutcome::Emoji));
        }
        let to_list = matches!(event.key, Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Enter)
            || (event.key == Key::Right && {
                let (caret, anchor) = self.search.input().caret();
                caret == anchor && caret == self.search.text().len()
            });
        if to_list {
            let r = self.list.key(theme, event, now);
            return match r.event {
                Some(ResultEvent::Activated(i)) => (true, self.activate(theme, i, now)),
                Some(ResultEvent::Actions(i)) => (self.open_actions(theme, i, now) | r.repaint, None),
                Some(ResultEvent::Selected(i)) => {
                    self.request_icons_around(i);
                    (true, None)
                }
                None => (r.repaint, None),
            };
        }
        let r = self.search.key(event, now);
        match r.event {
            Some(TextInputEvent::Changed) => {
                self.refresh(theme, now);
                (true, None)
            }
            Some(TextInputEvent::Cancel) => (true, Some(LauncherOutcome::Close)),
            Some(TextInputEvent::Submit) => (true, self.list.selected().and_then(|i| self.activate(theme, i, now))),
            Some(TextInputEvent::Copy(s)) => (r.repaint, Some(LauncherOutcome::Copy(CopyText::new(s)))),
            Some(TextInputEvent::Cut(s)) => {
                self.refresh(theme, now);
                (true, Some(LauncherOutcome::Copy(CopyText::new(s))))
            }
            Some(TextInputEvent::PasteRequested) => (r.repaint, Some(LauncherOutcome::RequestPaste)),
            _ => (r.repaint || pressed && event.key == Key::Escape, None),
        }
    }

    /// Panodan gelen metni arama alanına yapıştırır. Yeniden çizim gerekiyorsa `true`.
    pub fn paste(&mut self, theme: &Theme, text: &str, now: Instant) -> bool {
        if !self.open || *self.pages.current() != Page::Results {
            return false;
        }
        if self.search.input_mut().paste(text).event == Some(TextInputEvent::Changed) {
            self.refresh(theme, now);
            return true;
        }
        false
    }

    /// İşaretçi. Döner: (yeniden çizim, çıktı).
    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, event: &PointerEvent, now: Instant) -> (bool, Option<LauncherOutcome>) {
        if *self.pages.current() == Page::Actions {
            if self.header.pointer(event, now).event.is_some() {
                self.close_actions(theme, now);
                return (true, None);
            }
            let r = self.actions.pointer(theme, event, now);
            return match r.event {
                Some(ResultEvent::Activated(i)) => (true, self.launch_action(i)),
                _ => (r.repaint, None),
            };
        }
        let mut repaint = false;
        if self.mode == LauncherMode::Clipboard {
            let r = self.clear.pointer(event, now);
            repaint |= r.repaint;
            match r.event {
                Some(ConfirmEvent::Confirmed) => return (true, Some(LauncherOutcome::Clipboard(ClipboardCommand::Clear))),
                Some(ConfirmEvent::Armed) => return (true, None),
                Some(ConfirmEvent::Cancelled) | None => {}
            }
        }
        let r = self.search.pointer(text, theme, event, now);
        repaint |= r.repaint;
        if r.event == Some(TextInputEvent::Changed) {
            self.refresh(theme, now);
            return (true, None);
        }
        if self.mode == LauncherMode::Emoji {
            let (r, picked) = self.emoji.pointer(theme, event, now);
            return (repaint | r, picked.map(LauncherOutcome::Emoji));
        }
        let r = self.list.pointer(theme, event, now);
        repaint |= r.repaint;
        match r.event {
            Some(ResultEvent::Activated(i)) => (true, self.activate(theme, i, now)),
            Some(ResultEvent::Selected(i)) => {
                self.request_icons_around(i);
                (true, None)
            }
            _ => (repaint, None),
        }
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if *self.pages.current() == Page::Actions {
            return self.header.cursor(p).or_else(|| self.actions.cursor(p));
        }
        let clear = (self.mode == LauncherMode::Clipboard).then(|| self.clear.cursor(p)).flatten();
        let results = |p| if self.mode == LauncherMode::Emoji { self.emoji.cursor(p) } else { self.list.cursor(p) };
        clear.or_else(|| self.search.cursor(p)).or_else(|| results(p))
    }

    /// Sonuç sayfasının yerleşimi (`origin`: sayfanın sol üstü; geçişte kayar). `side`: arama
    /// alanının sağında düğmeye ayrılan (boyut; pano kipinde "Geçmişi temizle").
    fn layout_results(
        search: &mut SearchField,
        list: &mut ResultList,
        side: Option<(&mut ConfirmButton, Size)>,
        theme: &Theme,
        origin: Point,
        size: Size,
        now: Instant,
    ) {
        let field_h = search.preferred_size(theme, size.width).height;
        let reserved = side.as_ref().map_or(0.0, |(_, s)| s.width + theme.space(2));
        let field = Rect::new(origin.x, origin.y, (size.width - reserved).max(0.0), field_h);
        if search.rect() != field {
            search.set_rect(theme, field);
        }
        if let Some((button, s)) = side {
            button.set_rect(Rect::new(origin.x + size.width - s.width, origin.y + (field_h - s.height) * 0.5, s.width, s.height));
        }
        let y = origin.y + field_h + theme.space(3);
        let r = Rect::new(origin.x, y, size.width, (origin.y + size.height - y).max(0.0));
        if list.rect() != r {
            list.set_rect(theme, r, now);
        }
    }

    /// Eylem sayfasının yerleşimi: "‹ Ad" başlığı + eylemler.
    fn layout_actions(header: &mut PageHeader, actions: &mut ResultList, theme: &Theme, origin: Point, size: Size, now: Instant) {
        let hh = theme.space(12); // PageHeader yüksekliği
        header.set_rect(Rect::new(origin.x, origin.y, size.width, hh));
        let y = origin.y + hh + theme.space(2);
        let r = Rect::new(origin.x, y, size.width, (origin.y + size.height - y).max(0.0));
        if actions.rect() != r {
            actions.set_rect(theme, r, now);
        }
    }

    /// Boş listenin ruh hâli (maskot gizliyken de tutulur; geri açılınca doğru yüz).
    fn set_empty_mood(&mut self, mood: Mood) {
        self.empty_mood = mood;
        self.empty.set_mood(mood);
    }

    /// İçeriği `area`'ya çizer (çağıran kırpar ve soldurur).
    pub fn draw(&mut self, ui: &mut Ui, area: Rect) {
        let now = ui.now;
        let theme = ui.theme;
        // Pano kipinde arama alanının yanındaki düğme (iki durumun en genişi: etiket değişince
        // alan kaymaz).
        let clipboard = self.mode == LauncherMode::Clipboard;
        let side = clipboard.then(|| self.clear.max_size(ui.painter.text_system(), theme));
        if area != self.rect {
            // Olay işleyicileri son yerleşimi kullanır: sayfa yerinde dururkenki konum.
            self.rect = area;
            let side = side.map(|s| (&mut self.clear, s));
            Self::layout_results(&mut self.search, &mut self.list, side, theme, area.origin(), area.size(), now);
            Self::layout_actions(&mut self.header, &mut self.actions, theme, area.origin(), area.size(), now);
        }
        self.upload_icons(ui.painter);
        let size = area.size();
        let emoji_mode = self.mode == LauncherMode::Emoji;
        let (search, list, header, actions, empty_state, clear, emoji, usage) = (
            &mut self.search,
            &mut self.list,
            &mut self.header,
            &mut self.actions,
            &mut self.empty,
            &mut self.clear,
            &mut self.emoji,
            &self.usage,
        );
        let rows_empty = self.rows.is_empty();
        self.pages.draw(ui, area, |ui, page, at| match page {
            Page::Results => {
                Self::layout_results(search, list, side.map(|s| (&mut *clear, s)), theme, at, size, now);
                search.draw(ui);
                if clipboard {
                    clear.draw(ui);
                }
                // Emoji kipinde liste alanı ızgara + ad satırıdır.
                let empty = if emoji_mode {
                    emoji.draw(ui, list.rect(), usage);
                    emoji.is_empty()
                } else {
                    list.draw(ui);
                    rows_empty
                };
                if empty {
                    // Liste alanının ortasında maskot + metin (framework `EmptyState`).
                    empty_state.set_rect(list.rect());
                    empty_state.draw(ui);
                }
            }
            Page::Actions => {
                Self::layout_actions(header, actions, theme, at, size, now);
                header.draw(ui);
                actions.draw(ui);
            }
        });
        // Geçiş bitince yerleşim duruş konumuna döner (isabet testi için).
        let side = side.map(|s| (&mut self.clear, s));
        Self::layout_results(&mut self.search, &mut self.list, side, theme, area.origin(), size, now);
        Self::layout_actions(&mut self.header, &mut self.actions, theme, area.origin(), size, now);
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        let emoji = self.mode == LauncherMode::Emoji && self.emoji.is_animating(now);
        self.pages.is_animating(now) | emoji
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relocalize_keeps_query_and_focus() {
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = LauncherView::new(&theme, &LauncherConfig::default());
        v.opened(&theme, now);
        v.paste(&theme, "fire", now);
        v.relocalize(&theme, now);
        assert_eq!(v.search.text(), "fire", "yazılan sorgu dil değişiminde kaybolmaz");
        assert!(v.search.input().is_focused(), "odak korunur");
        assert!(v.open);

        // Kapalı başlatıcı kapalı kalır; boş sorgu boş kalır.
        v.closed(now);
        v.release();
        v.relocalize(&theme, now);
        assert!(!v.open && v.search.text().is_empty() && !v.search.input().is_focused());
    }

    /// Kapanış kare regresyonu (2026-10-17): kapanma başlarken sorgu ve sonuçlar yerinde kalır
    /// (kapanan yaprakta "Sonuç yok" boş durumu listenin üstüne binmez); yaprak çöktüğünde
    /// (`release`) temizlenir. Çökmeden yeniden açılırsa temizlik açılışa bırakılır.
    #[test]
    fn query_and_results_survive_the_closing_animation() {
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = LauncherView::new(&theme, &LauncherConfig::default());
        v.opened(&theme, now);
        v.paste(&theme, "1+1", now);
        let rows = v.rows.clone();
        assert!(!rows.is_empty(), "hesap makinesi satırı");

        v.closed(now);
        assert!(!v.open && !v.search.input().is_focused());
        assert_eq!(v.search.text(), "1+1", "kapanış boyunca sorgu yerinde");
        assert_eq!(v.rows, rows, "kapanış boyunca sonuçlar yerinde");

        v.release();
        assert!(v.search.text().is_empty() && v.rows.is_empty(), "çökünce bırakılır");

        // Kapanırken yeniden açılış: release bir şey yapmaz, açılış sıfırlar.
        v.opened(&theme, now);
        v.paste(&theme, "2+2", now);
        v.closed(now);
        v.opened(&theme, now);
        v.release();
        assert!(v.open && v.search.text().is_empty(), "açılış sorguyu sıfırladı, release açıkken dokunmadı");
    }

    fn key(k: Key) -> KeyEvent {
        KeyEvent { key: k, state: KeyState::Pressed, text: None, modifiers: nuxut::Modifiers::default(), xkb: None }
    }

    fn snapshot(texts: &[&str], image: bool) -> Arc<ClipboardSnapshot> {
        use crate::services::clipboard::history::{Content, Item};
        use crate::services::clipboard::filter::ImageMime;
        use zeroize::Zeroizing;
        let mut items: Vec<Arc<Item>> = texts
            .iter()
            .enumerate()
            .map(|(i, s)| Arc::new(Item::new(i as u64 + 1, Content::text(Zeroizing::new(s.as_bytes().to_vec())).unwrap(), SystemTime::now())))
            .collect();
        if image {
            let png = Content::image(ImageMime::Png, Zeroizing::new(b"\x89PNG\r\n\x1a\nxx".to_vec())).unwrap();
            items.push(Arc::new(Item::new(100, png, SystemTime::now())));
        }
        Arc::new(ClipboardSnapshot { enabled: true, available: true, items })
    }

    #[test]
    fn clipboard_mode_lists_filters_and_picks() {
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = LauncherView::new(&theme, &LauncherConfig::default());
        v.set_clipboard(&theme, &snapshot(&["Merhaba dünya", "ikinci öğe"], true), now);
        assert!(!v.request_mode(&theme, LauncherMode::Clipboard, now), "kapalıyken bir sonraki açılışa");
        v.opened(&theme, now);
        assert_eq!(v.mode(), Some(LauncherMode::Clipboard));
        assert_eq!(v.rows, [Row::Clip(1), Row::Clip(2), Row::Clip(100)]);
        // Görsel satırı küçük resim ister (baytlar paylaşılır).
        let thumbs = v.take_thumb_requests();
        assert_eq!(thumbs.len(), 1);
        assert_eq!(thumbs[0].key, thumb_key(100));
        assert_eq!(&thumbs[0].data[..8], b"\x89PNG\r\n\x1a\n");
        assert!(v.take_icon_requests().is_empty(), "küçük resim ikon temasına gitmez");

        // Arama metin öğelerini süzer (büyük/küçük harf duyarsız), görseller düşer.
        v.paste(&theme, "DÜNYA", now);
        assert_eq!(v.rows, [Row::Clip(1)]);
        let (_, out) = v.key(&theme, &key(Key::Enter), now);
        assert_eq!(out, Some(LauncherOutcome::Clipboard(ClipboardCommand::Pick(1))));

        // Delete: imleç sonda → seçili öğe silinir.
        let (redraw, out) = v.key(&theme, &key(Key::Delete), now);
        assert!(redraw);
        assert_eq!(out, Some(LauncherOutcome::Clipboard(ClipboardCommand::Delete(1))));

        // Kapanınca kip unutulur: sonraki açılış uygulamalar.
        v.closed(now);
        v.opened(&theme, now);
        assert_eq!(v.mode(), Some(LauncherMode::Apps));
        // Açıkken kip değişimi hemen.
        assert!(v.request_mode(&theme, LauncherMode::Clipboard, now));
        assert_eq!(v.mode(), Some(LauncherMode::Clipboard));
    }

    #[test]
    fn clipboard_mode_empty_states_and_eviction() {
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = LauncherView::new(&theme, &LauncherConfig::default());
        v.request_mode(&theme, LauncherMode::Clipboard, now);
        v.opened(&theme, now);
        assert!(v.rows.is_empty(), "izleyici yok: liste boş");
        let unavailable = Arc::new(ClipboardSnapshot { enabled: true, available: false, items: snapshot(&["x"], false).items.clone() });
        v.set_clipboard(&theme, &unavailable, now);
        assert!(v.rows.is_empty(), "protokol yoksa öğe gösterilmez");
        let off = Arc::new(ClipboardSnapshot { enabled: false, available: true, items: Vec::new() });
        v.set_clipboard(&theme, &off, now);
        assert!(v.rows.is_empty());

        // Küçük resmi istenen öğe geçmişten çıkınca önbellekten de çıkar.
        v.set_clipboard(&theme, &snapshot(&[], true), now);
        assert_eq!(v.take_thumb_requests().len(), 1);
        assert!(v.icons.contains_key(&thumb_key(100)));
        v.set_clipboard(&theme, &snapshot(&["yalnız metin"], false), now);
        assert!(!v.icons.contains_key(&thumb_key(100)));
        assert_eq!(v.rows, [Row::Clip(1)]);
    }

    #[test]
    fn emoji_mode_searches_and_picks() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = LauncherView::new(&theme, &LauncherConfig::default());
        assert!(!v.request_mode(&theme, LauncherMode::Emoji, now));
        v.opened(&theme, now);
        assert_eq!(v.mode(), Some(LauncherMode::Emoji));
        assert!(v.rows.is_empty(), "emoji kipinde liste satırı yok: ızgara");
        assert_eq!(v.emoji.len(), crate::services::launcher::emoji::all().len(), "boş sorgu: tümü (bölümlü ızgara)");
        assert_eq!(v.search.input().placeholder(), t!("shell-launcher-emoji-placeholder"));
        assert!(v.take_icon_requests().is_empty(), "emoji satırı ikon teması istemez");

        // Türkçe arayüzde İngilizce de bulunur; Enter emojiyi verir.
        v.paste(&theme, "kedi", now);
        assert_eq!(v.emoji.emoji_at(0), Some("🐈"));
        // Ctrl+→ arama alanına kalır (seçim değişmez).
        let mut ctrl_right = key(Key::Right);
        ctrl_right.modifiers.ctrl = true;
        v.key(&theme, &ctrl_right, now);
        assert_eq!(v.emoji.selected_emoji(), Some("🐈"));
        let (_, out) = v.key(&theme, &key(Key::Enter), now);
        assert_eq!(out, Some(LauncherOutcome::Emoji("🐈")));

        // Kullanım değişince (servisten) liste yenilenir: sık kullanılan başa.
        v.closed(now);
        v.request_mode(&theme, LauncherMode::Emoji, now);
        v.opened(&theme, now);
        let mut usage = Usage::default();
        usage.record_emoji("🌹", now_unix());
        assert!(v.set_data(&theme, &Arc::default(), &Arc::new(usage), now));
        assert_eq!(v.emoji.emoji_at(0), Some("🌹"));

        // Hesap makinesi emoji kipinde çalışmaz; kapanınca kip unutulur.
        v.paste(&theme, "1+1", now);
        assert!(!v.rows.iter().any(|r| matches!(r, Row::Calc(_))));
        v.closed(now);
        v.opened(&theme, now);
        assert_eq!(v.mode(), Some(LauncherMode::Apps));
    }

    /// inceleme şartı: emoji kipinde katalog, kullanım ve dil değişimi seçimi sıfırlamaz.
    #[test]
    fn emoji_mode_ignores_catalog_changes() {
        crate::i18n::use_turkish_for_tests();
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = LauncherView::new(&theme, &LauncherConfig::default());
        v.request_mode(&theme, LauncherMode::Emoji, now);
        v.opened(&theme, now);
        v.key(&theme, &key(Key::Right), now);
        let moved = v.emoji.selected_emoji();
        assert!(moved.is_some() && moved != Some("😀"), "seçim ilerledi");

        // Yeni katalog (aynı kullanım): yenileme yok, seçim yerinde.
        let usage = v.usage.clone();
        assert!(!v.set_data(&theme, &Arc::new(Catalog::default()), &usage, now));
        assert_eq!(v.emoji.selected_emoji(), moved);

        // Kullanım değişti: yenilenir (sık kullanılanlar bölümü).
        let mut u = Usage::default();
        u.record_emoji("🌹", now_unix());
        assert!(v.set_data(&theme, &v.catalog.clone(), &Arc::new(u), now));
        assert_eq!(v.emoji.emoji_at(0), Some("🌹"));
        assert_eq!(v.emoji.selected_emoji(), moved, "yenilenen listede seçim aynı emojide");

        // Dil değişimi (relocalize): sorgu ve seçim yerinde.
        v.relocalize(&theme, now);
        assert_eq!(v.emoji.selected_emoji(), moved);

        // Yeniden açılış başa döner.
        v.closed(now);
        v.request_mode(&theme, LauncherMode::Emoji, now);
        v.opened(&theme, now);
        assert_eq!(v.emoji.grid().selected(), Some(0));

        // Uygulama kipinde katalog değişimi yine yeniler.
        v.request_mode(&theme, LauncherMode::Apps, now);
        let usage = v.usage.clone();
        assert!(v.set_data(&theme, &Arc::new(Catalog::default()), &usage, now));
    }

    #[test]
    fn case_insensitive_contains() {
        let n = |s: &str| s.chars().flat_map(char::to_lowercase).collect::<Vec<_>>();
        assert!(contains_ci("Merhaba DÜNYA", &n("dünya")));
        assert!(contains_ci("abc", &n("")));
        assert!(!contains_ci("abc", &n("abcd")));
        assert!(contains_ci("xxİyy", &n("i̇")));
        // Arama ilk 16 KiB ile sınırlı (karakter sınırında kesilir).
        let long = format!("{}ğ{}", "a".repeat(CLIP_SEARCH_BYTES - 1), "hedef");
        assert!(!contains_ci(&long, &n("hedef")));
        assert_eq!(thumb_id(&thumb_key(42)), Some(42));
        assert_eq!(thumb_id("firefox"), None);
    }

    fn render(v: &mut LauncherView, theme: &Theme, area: Rect) -> Vec<Rect> {
        let mut text = TextSystem::new();
        let mut rec = nuxut::render::Recorder::new();
        rec.record(&mut text, (800, 800), 1.0, |p| {
            let mut ui = Ui::new(p, theme, Instant::now());
            v.draw(&mut ui, area);
        });
        rec.draws().into_iter().filter(|d| d.kind == nuxut::render::DrawKind::Image).map(|d| d.rect).collect()
    }

    /// Boş listenin ruh hâli her durumda yerleşim kuralına uyar.
    #[test]
    fn empty_list_mascot_mood_per_state() {
        let theme = Theme::default();
        let now = Instant::now();
        let mut v = LauncherView::new(&theme, &LauncherConfig::default());
        v.opened(&theme, now);
        v.paste(&theme, "zzqqxx-yok", now);
        assert!(v.rows.is_empty());
        assert_eq!(v.empty_mood, Mood::NoResults, "uygulama araması: sonuç yok");

        v.request_mode(&theme, LauncherMode::Clipboard, now);
        let cases = [
            (ClipboardSnapshot { enabled: false, available: true, items: Vec::new() }, Mood::Empty),
            (ClipboardSnapshot { enabled: true, available: false, items: Vec::new() }, Mood::Error),
            (ClipboardSnapshot { enabled: true, available: true, items: Vec::new() }, Mood::Empty),
        ];
        for (snap, mood) in cases {
            v.set_clipboard(&theme, &Arc::new(snap), now);
            assert!(v.rows.is_empty());
            assert_eq!(v.empty_mood, mood);
        }
        // Dolu geçmiş, süzgeçte eşleşme yok.
        v.set_clipboard(&theme, &snapshot(&["bir", "iki"], false), now);
        v.paste(&theme, "üç", now);
        assert!(v.rows.is_empty());
        assert_eq!(v.empty_mood, Mood::NoResults);
    }

    /// Boş listede maskot (96) liste alanında ortalı; sonuç varken çizilmez; gizlenince
    /// (`[appearance] mascot = false`) ölçü 0, görsel yok; geri açılınca aynı yerde.
    #[test]
    fn empty_list_draws_mascot_and_hides_cleanly() {
        let shown = Theme::default();
        let mut hidden = shown.clone();
        hidden.appearance.mascot = false;
        let now = Instant::now();
        let area = Rect::new(20.0, 20.0, 560.0, v_height(&shown));
        let mut v = LauncherView::new(&shown, &LauncherConfig::default());
        v.request_mode(&shown, LauncherMode::Clipboard, now);
        v.opened(&shown, now);
        v.set_clipboard(&shown, &snapshot(&[], false), now);
        assert!(v.rows.is_empty());
        let imgs = render(&mut v, &shown, area);
        assert_eq!(imgs.len(), 1, "boş geçmiş: tek görsel, maskot");
        let list = v.list.rect();
        assert_eq!(imgs[0].size(), Size::new(96.0, 96.0));
        assert!((imgs[0].center().x - list.center().x).abs() <= 1.0);
        assert!(imgs[0].y >= list.y && imgs[0].bottom() <= list.bottom());

        v.set_theme(&hidden, now);
        assert!(render(&mut v, &hidden, area).is_empty(), "gizli maskot çizilmez");
        v.set_theme(&shown, now);
        assert_eq!(render(&mut v, &shown, area), imgs);

        // Öğe gelince liste dolu: maskot yok (metin öğesinde görsel yok).
        v.set_clipboard(&shown, &snapshot(&["metin"], false), now);
        assert!(!v.rows.is_empty());
        assert!(render(&mut v, &shown, area).is_empty());
    }

    fn v_height(theme: &Theme) -> f32 {
        LauncherView::new(theme, &LauncherConfig::default()).content_size(theme).height
    }
}
